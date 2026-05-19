pub mod fetcher;
pub mod lockfile;
pub mod malware_scanner;
pub mod manifest;
pub mod resolver;
pub mod semver;
pub mod signature_verifier;

pub use lockfile::Lockfile;
pub use malware_scanner::{MalwareScanner, ScanResult, Threat, ThreatLevel};
pub use manifest::{PackageInfo, PackageManifest, PackagePermissions};
pub use resolver::{DependencyGraph, DependencyNode, Resolver};
pub use semver::{Semver, SemverRange};
pub use signature_verifier::{HashAlgorithm, SignatureInfo, SignatureVerifier, VerificationStatus};

use std::collections::HashMap;
use std::path::PathBuf;

// ── Registry ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Registry {
    Npm,
    Jsr,
    Yarn,
    Custom(String),
}

impl Registry {
    /// Deriva el registro desde el host especificado en --allow-net.
    /// El host que el usuario autoriza explícitamente define el registro.
    pub fn from_allowed_host(host: &str) -> Self {
        let h = host
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        // Strip path component and port — keep only hostname
        let h = h.split('/').next().unwrap_or(h);
        let h = h.split(':').next().unwrap_or(h);
        // Exact-match or public-suffix match against known registries.
        // Using .contains() would let "evil.npmjs.org.attacker.com" match — use eq or suffix.
        if h == "jsr.io" || h.ends_with(".jsr.io") || h == "npm.jsr.io" {
            Registry::Jsr
        } else if h == "registry.yarnpkg.com" || h.ends_with(".yarnpkg.com") {
            Registry::Yarn
        } else if h == "registry.npmjs.org"
            || h == "registry.npmjs.com"
            || h.ends_with(".npmjs.org")
            || h.ends_with(".npmjs.com")
        {
            Registry::Npm
        } else {
            Registry::Custom(format!("https://{}", h))
        }
    }

    pub fn base_url(&self) -> &str {
        match self {
            Registry::Npm => "https://registry.npmjs.org",
            Registry::Jsr => "https://jsr.io/api",
            Registry::Yarn => "https://registry.yarnpkg.com",
            Registry::Custom(url) => url.as_str(),
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Registry::Npm => "registry.npmjs.org",
            Registry::Jsr => "jsr.io",
            Registry::Yarn => "registry.yarnpkg.com",
            Registry::Custom(url) => url.as_str(),
        }
    }
}

// ── PackageManager ────────────────────────────────────────────────────────────

pub struct PackageManager {
    resolver: Resolver,
    #[allow(dead_code)]
    cache_dir: PathBuf,
}

impl PackageManager {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            resolver: Resolver::new("https://registry.npmjs.org"),
            cache_dir,
        }
    }

    pub fn install(
        &mut self,
        deps: &HashMap<String, String>,
        project_name: &str,
        project_version: &str,
    ) -> anyhow::Result<Lockfile> {
        let graph = self.resolver.resolve(deps);
        let lockfile = Lockfile::generate(&graph, project_name, project_version);
        tracing::info!("Resolved {} dependencies", graph.nodes().len());
        Ok(lockfile)
    }

    pub fn load_lockfile(path: &PathBuf) -> anyhow::Result<Lockfile> {
        Lockfile::load(path)
    }

    pub fn save_lockfile(&self, lockfile: &Lockfile, path: &PathBuf) -> anyhow::Result<()> {
        lockfile.save(path)
    }
}

// ── Registry lookups ──────────────────────────────────────────────────────────

struct VersionMeta {
    tarball: String,
    integrity: Option<String>,
}

struct RegistryInfo {
    versions: Vec<String>,
    latest: Option<String>,
    version_meta: HashMap<String, VersionMeta>,
}

async fn lookup_npm_compat(
    client: &reqwest::Client,
    base_url: &str,
    pkg_name: &str,
) -> anyhow::Result<RegistryInfo> {
    let url = format!("{}/{}", base_url, pkg_name);
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!("Package '{}' not found in registry", pkg_name);
    }
    if !resp.status().is_success() {
        anyhow::bail!("Registry returned HTTP {}", resp.status());
    }

    let data: serde_json::Value = resp.json().await?;
    let latest = data["dist-tags"]["latest"].as_str().map(|s| s.to_string());

    let mut versions = Vec::new();
    let mut version_meta = HashMap::new();

    if let Some(obj) = data["versions"].as_object() {
        for (ver, meta) in obj {
            versions.push(ver.clone());
            let tarball = meta["dist"]["tarball"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}/{}/-/{}-{}.tgz", base_url, pkg_name, pkg_name, ver));
            let integrity = meta["dist"]["integrity"].as_str().map(|s| s.to_string());
            version_meta.insert(ver.clone(), VersionMeta { tarball, integrity });
        }
    }

    Ok(RegistryInfo {
        versions,
        latest,
        version_meta,
    })
}

async fn lookup_jsr(client: &reqwest::Client, pkg_name: &str) -> anyhow::Result<RegistryInfo> {
    if !pkg_name.starts_with('@') || !pkg_name.contains('/') {
        anyhow::bail!(
            "JSR only supports scoped packages (e.g. @scope/name). '{}' is not a valid JSR package name",
            pkg_name
        );
    }
    let trimmed = pkg_name.trim_start_matches('@');
    let (scope, name) = trimmed.split_once('/').unwrap();
    // JSR exposes npm-compatible packages at npm.jsr.io under @jsr/scope__name
    let npm_name = format!("@jsr/{}__{}", scope, name);
    lookup_npm_compat(client, "https://npm.jsr.io", &npm_name)
        .await
        .map_err(|e| anyhow::anyhow!("Package '{}' not found on jsr.io: {}", pkg_name, e))
}

async fn lookup_registry(registry: &Registry, pkg_name: &str) -> anyhow::Result<RegistryInfo> {
    let client = reqwest::Client::new();
    match registry {
        Registry::Jsr => lookup_jsr(&client, pkg_name).await,
        Registry::Npm | Registry::Yarn | Registry::Custom(_) => {
            lookup_npm_compat(&client, registry.base_url(), pkg_name).await
        }
    }
}

// ── Version utilities ─────────────────────────────────────────────────────────

fn parse_semver_tuple(v: &str) -> Option<(u64, u64, u64)> {
    let clean = v
        .trim_start_matches('^')
        .trim_start_matches('~')
        .trim_start_matches('v');
    let base = clean.split(['-', '+']).next()?;
    let parts: Vec<&str> = base.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let major = parts[0].parse().ok()?;
    let minor = parts[1].parse().ok()?;
    let patch = parts[2].parse().ok()?;
    Some((major, minor, patch))
}

fn semver_score(t: (u64, u64, u64)) -> u64 {
    t.0 * 1_000_000 + t.1 * 1_000 + t.2
}

fn find_nearby_versions(requested: &str, available: &[String], count: usize) -> Vec<String> {
    let req_score = parse_semver_tuple(requested)
        .map(semver_score)
        .unwrap_or(u64::MAX);

    let mut scored: Vec<(u64, &String)> = available
        .iter()
        .filter_map(|v| {
            let score = semver_score(parse_semver_tuple(v)?);
            let dist = score.abs_diff(req_score);
            Some((dist, v))
        })
        .collect();

    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(a.1)));
    scored
        .iter()
        .take(count)
        .map(|(_, v)| (*v).clone())
        .collect()
}

// ── Package spec parsing ──────────────────────────────────────────────────────

fn is_valid_package_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 214 {
        return false;
    }
    if name.starts_with('@') {
        let parts: Vec<&str> = name.split('/').collect();
        return parts.len() >= 2 && parts[0].len() >= 2 && parts.iter().all(|p| !p.is_empty());
    }
    if name.chars().any(|c| c.is_uppercase()) {
        return false;
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() > 1 && (parts[0].is_empty() || parts[parts.len() - 1].is_empty()) {
        return false;
    }
    !name.contains("::") && !name.contains("..")
}

fn normalize_version(version: &str) -> String {
    let v = version
        .trim_start_matches('^')
        .trim_start_matches('*')
        .to_string();
    if v.is_empty() { "*".to_string() } else { v }
}

fn parse_package_spec(input: &str) -> anyhow::Result<(String, Option<String>)> {
    // Scoped packages: @scope/name or @scope/name@version
    if let Some(after_at) = input.strip_prefix('@') {
        if let Some(slash_pos) = after_at.find('/') {
            let after_slash = &after_at[slash_pos + 1..];
            if let Some(ver_at_pos) = after_slash.find('@') {
                let name_end = 1 + slash_pos + 1 + ver_at_pos;
                let name = &input[..name_end];
                let version = &input[name_end + 1..];
                if !is_valid_package_name(name) {
                    anyhow::bail!("Invalid scoped package name: '{}'", name);
                }
                if version.is_empty() {
                    anyhow::bail!("Empty version specified for '{}'", name);
                }
                return Ok((name.to_string(), Some(version.to_string())));
            } else {
                if !is_valid_package_name(input) {
                    anyhow::bail!("Invalid package name: '{}'", input);
                }
                return Ok((input.to_string(), None));
            }
        }
        anyhow::bail!(
            "Invalid scoped package format: '{}'. Expected @scope/name",
            input
        );
    }

    // Regular packages: name or name@version
    let (name, version) = match input.split_once('@') {
        None => (input, None),
        Some((n, v)) => (n, Some(v)),
    };

    if !is_valid_package_name(name) {
        anyhow::bail!(
            "Invalid package name: '{}'. Names must be lowercase and cannot contain '::', '..', or start/end with '/'",
            name
        );
    }
    if let Some(v) = version
        && v.contains(':')
    {
        anyhow::bail!(
            "Invalid version '{}'. Use name@version format (not name:version)",
            v
        );
    }

    Ok((name.to_string(), version.map(|s| s.to_string())))
}

// ── Integrity verification ────────────────────────────────────────────────────

fn verify_integrity(data: &[u8], integrity: &str) -> anyhow::Result<()> {
    use base64::Engine;
    use sha2::Digest;

    if let Some(expected_b64) = integrity.strip_prefix("sha512-") {
        let mut hasher = sha2::Sha512::new();
        hasher.update(data);
        let computed = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
        if computed != expected_b64 {
            anyhow::bail!(
                "Integrity check failed (sha512): tarball hash does not match registry metadata"
            );
        }
        return Ok(());
    }
    if let Some(expected_b64) = integrity.strip_prefix("sha256-") {
        let mut hasher = sha2::Sha256::new();
        hasher.update(data);
        let computed = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
        if computed != expected_b64 {
            anyhow::bail!(
                "Integrity check failed (sha256): tarball hash does not match registry metadata"
            );
        }
        return Ok(());
    }
    // Unknown format — log and continue
    tracing::warn!(
        "Unknown integrity format '{}', skipping verification",
        integrity
    );
    Ok(())
}

// ── Download + extract ────────────────────────────────────────────────────────

async fn download_tarball(url: &str) -> anyhow::Result<Vec<u8>> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Network error downloading tarball: {}", e))?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Failed to download tarball from {}: HTTP {}",
            url,
            resp.status()
        );
    }

    Ok(resp.bytes().await?.to_vec())
}

fn extract_tarball(data: &[u8], dest: &PathBuf) -> anyhow::Result<()> {
    let decoder = flate2::read::GzDecoder::new(data);
    let mut archive = tar::Archive::new(decoder);

    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::create_dir_all(dest)?;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        // npm tarballs have a leading "package/" directory — skip it
        let cleaned: PathBuf = path.iter().skip(1).collect();
        if cleaned.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&cleaned);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&out)?;
    }
    Ok(())
}

// ── Public API ────────────────────────────────────────────────────────────────

pub async fn install_package(name: &str, allow_net: Option<&[String]>) -> anyhow::Result<()> {
    install_with_transitive(name, false, allow_net).await
}

pub async fn reinstall_package(name: &str, allow_net: Option<&[String]>) -> anyhow::Result<()> {
    install_with_transitive(name, true, allow_net).await
}

/// BFS install: installs `root` and all of its transitive dependencies.
async fn install_with_transitive(
    root: &str,
    force: bool,
    allow_net: Option<&[String]>,
) -> anyhow::Result<()> {
    use std::collections::{HashSet, VecDeque};

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(root.to_string());

    while let Some(pkg) = queue.pop_front() {
        if visited.contains(&pkg) {
            continue;
        }
        visited.insert(pkg.clone());

        // Skip if already extracted (and not forced), but still read its deps
        let node_modules_dest = PathBuf::from("node_modules").join(&pkg);
        let already_ok = !force && {
            let pj = node_modules_dest.join("package.json");
            pj.exists()
        };

        if already_ok && pkg != root {
            // Already installed — just enqueue its deps
        } else {
            install_package_impl(&pkg, force && pkg == root, allow_net).await?;
        }

        // Read transitive deps from the installed package.json
        let pkg_json = node_modules_dest.join("package.json");
        if let Ok(content) = std::fs::read_to_string(&pkg_json)
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(deps_obj) = val["dependencies"].as_object()
        {
            for dep_name in deps_obj.keys() {
                if !visited.contains(dep_name.as_str()) {
                    queue.push_back(dep_name.clone());
                }
            }
        }
    }

    Ok(())
}

pub fn audit_packages() -> anyhow::Result<bool> {
    let lockfile_path = PathBuf::from("3va-lock.json");
    let node_modules = PathBuf::from("node_modules");

    if !node_modules.exists() {
        eprintln!(
            "✗ node_modules/ not found. Run '3va install <package> --allow-net=<host>' first."
        );
        anyhow::bail!("node_modules not found");
    }

    let installed: Vec<(String, String, Option<String>)> = if lockfile_path.exists() {
        let lock = Lockfile::load(&lockfile_path)?;
        lock.dependencies
            .iter()
            .map(|(name, dep)| (name.clone(), dep.version.clone(), dep.registry.clone()))
            .collect()
    } else {
        // Fall back to scanning all directories in node_modules/
        let mut pkgs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&node_modules) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    // Handle scoped packages (@scope/name)
                    if name.starts_with('@') {
                        if let Ok(sub) = std::fs::read_dir(&path) {
                            for sub_entry in sub.flatten() {
                                let sub_name =
                                    format!("{}/{}", name, sub_entry.file_name().to_string_lossy());
                                pkgs.push((sub_name, "unknown".to_string(), None));
                            }
                        }
                    } else {
                        pkgs.push((name, "unknown".to_string(), None));
                    }
                }
            }
        }
        pkgs
    };

    if installed.is_empty() {
        println!("No packages to audit.");
        return Ok(true);
    }

    println!();
    println!(
        "Auditing {} package(s) in node_modules/ ...",
        installed.len()
    );
    println!();

    let scanner = MalwareScanner::new();
    let mut total_threats = 0usize;
    let mut packages_with_issues = 0usize;
    let mut any_critical = false;

    for (pkg_name, version, registry) in &installed {
        let pkg_dir = node_modules.join(pkg_name);
        if !pkg_dir.exists() {
            println!(
                "  {:<35} (not extracted)",
                format!("{}@{}", pkg_name, version)
            );
            continue;
        }

        let results = scanner.scan_directory(&pkg_dir);
        let all_threats: Vec<_> = results.iter().flat_map(|r| r.threats.iter()).collect();
        let threat_count = all_threats.len();
        total_threats += threat_count;

        let reg_label = registry.as_deref().unwrap_or("unknown");

        if threat_count == 0 {
            println!(
                "  {:<40} ✓ Clean  [{}]",
                format!("{}@{}", pkg_name, version),
                reg_label
            );
        } else {
            packages_with_issues += 1;
            let worst = results
                .iter()
                .flat_map(|r| r.threats.iter())
                .map(|t| match t.severity {
                    ThreatLevel::Critical => 4,
                    ThreatLevel::High => 3,
                    ThreatLevel::Medium => 2,
                    ThreatLevel::Low => 1,
                    ThreatLevel::Safe => 0,
                })
                .max()
                .unwrap_or(0);

            if worst >= 4 {
                any_critical = true;
            }

            let level_str = match worst {
                4 => "CRITICAL",
                3 => "HIGH",
                2 => "MEDIUM",
                _ => "LOW",
            };
            eprintln!(
                "  {:<40} ✗ {} — {} threat(s)",
                format!("{}@{}", pkg_name, version),
                level_str,
                threat_count
            );

            for result in &results {
                for threat in &result.threats {
                    let sev = match threat.severity {
                        ThreatLevel::Critical => "CRITICAL",
                        ThreatLevel::High => "HIGH    ",
                        ThreatLevel::Medium => "MEDIUM  ",
                        ThreatLevel::Low => "LOW     ",
                        ThreatLevel::Safe => "SAFE    ",
                    };
                    eprintln!(
                        "    [{}] {}:{} — {}",
                        sev,
                        result
                            .file
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        threat.line,
                        threat.description
                    );
                }
            }
        }
    }

    println!();
    println!("  Packages scanned : {}", installed.len());
    println!("  Threats found    : {}", total_threats);
    println!("  Packages flagged : {}", packages_with_issues);

    if total_threats == 0 {
        println!();
        println!("✓ Audit complete. All packages are clean.");
        Ok(true)
    } else {
        println!();
        if any_critical {
            eprintln!(
                "✗ Audit failed: critical threats detected. Remove affected packages immediately."
            );
        } else {
            eprintln!("! Audit complete with warnings. Review flagged packages.");
        }
        Ok(!any_critical)
    }
}

pub async fn update_packages(
    packages: &[String],
    allow_net: Option<&[String]>,
) -> anyhow::Result<()> {
    let lockfile_path = PathBuf::from("3va-lock.json");

    let lockfile = match Lockfile::load(&lockfile_path) {
        Ok(l) => l,
        Err(_) => {
            eprintln!(
                "✗ No 3va-lock.json found. Run '3va install <package> --allow-net=<host>' first."
            );
            anyhow::bail!("Lockfile not found");
        }
    };

    // Determine target packages (all deps or specific subset)
    let all_deps: Vec<String> = lockfile.dependencies.keys().cloned().collect();
    let targets: Vec<String> = if packages.is_empty() {
        all_deps
    } else {
        for pkg in packages {
            if !lockfile.dependencies.contains_key(pkg.as_str()) {
                eprintln!("✗ '{}' is not in the lockfile. Install it first.", pkg);
                anyhow::bail!("Package not found in lockfile: {}", pkg);
            }
        }
        packages.to_vec()
    };

    if targets.is_empty() {
        println!("No packages to update.");
        return Ok(());
    }

    // Map registry → packages that need it
    let by_registry = lockfile.registries_needed(&targets);
    let unknown: Vec<&str> = targets
        .iter()
        .filter(|p| lockfile.registry_for(p).is_none())
        .map(|s| s.as_str())
        .collect();

    // Normalize a host: strip scheme, path, port, trailing slash — hostname only.
    fn normalize_host(h: &str) -> &str {
        let h = h.trim();
        let h = h.strip_prefix("https://").unwrap_or(h);
        let h = h.strip_prefix("http://").unwrap_or(h);
        let h = h.split('/').next().unwrap_or(h);
        let h = h.split(':').next().unwrap_or(h);
        h.trim_end_matches('/')
    }

    // Exact host match — prevents "evil.registry.npmjs.org.attacker.com" from matching "registry.npmjs.org".
    fn host_is_allowed(allowed_host: &str, required_registry: &str) -> bool {
        allowed_host == required_registry
    }

    let allowed: Vec<&str> = allow_net
        .map(|v| v.iter().map(|s| normalize_host(s.as_str())).collect())
        .unwrap_or_default();

    // Find which registries are needed but not allowed
    let missing: Vec<(&str, &Vec<String>)> = by_registry
        .iter()
        .filter(|(reg, _)| !allowed.iter().any(|h| host_is_allowed(h, reg.as_str())))
        .map(|(r, pkgs)| (r.as_str(), pkgs))
        .collect();

    if !missing.is_empty() || (allowed.is_empty() && !by_registry.is_empty()) {
        eprintln!();
        eprintln!("✗ Update requires network access to:");
        eprintln!();
        for (registry, pkgs) in &by_registry {
            eprintln!("    {:<35} ({})", registry, pkgs.join(", "));
        }
        if !unknown.is_empty() {
            eprintln!("    (no registry recorded for: {})", unknown.join(", "));
        }
        eprintln!();
        let hosts: Vec<&str> = by_registry.keys().map(|s| s.as_str()).collect();
        eprintln!("  Run: 3va update --allow-net={}", hosts.join(","));
        anyhow::bail!("Network access denied: --allow-net missing required registries");
    }

    if !unknown.is_empty() {
        eprintln!(
            "! Warning: no registry recorded for: {}. Skipping.",
            unknown.join(", ")
        );
    }

    // Update each package using its stored registry as the allowed host
    println!();
    println!("Updating {} package(s)...", targets.len());

    for pkg in &targets {
        if let Some(reg) = lockfile.registry_for(pkg) {
            let host = reg.to_string();
            install_package_impl(pkg, true, Some(&[host])).await?;
        }
    }

    println!();
    println!("✓ All packages updated.");
    Ok(())
}

async fn install_package_impl(
    input: &str,
    force: bool,
    allow_net: Option<&[String]>,
) -> anyhow::Result<()> {
    let (pkg_name, requested_version) = parse_package_spec(input)?;

    if let Some(ref v) = requested_version
        && v.is_empty()
    {
        eprintln!();
        eprintln!(
            "✗ Error: Empty version is not allowed. Use {}@<version>",
            pkg_name
        );
        anyhow::bail!("Empty version specified");
    }

    // secure-by-default: red denegada sin --allow-net explícito
    let allowed_host = match allow_net.and_then(|hosts| hosts.first()) {
        Some(h) => h.clone(),
        None => {
            eprintln!();
            eprintln!("✗ Network access denied.");
            eprintln!();
            eprintln!("  The package manager requires explicit network permission.");
            eprintln!("  Specify the registry host with --allow-net:");
            eprintln!();
            eprintln!(
                "    3va install {} --allow-net=registry.npmjs.org",
                pkg_name
            );
            eprintln!(
                "    3va install {} --allow-net=registry.yarnpkg.com",
                pkg_name
            );
            eprintln!("    3va install {} --allow-net=jsr.io", pkg_name);
            anyhow::bail!("Network access denied: --allow-net not specified");
        }
    };

    let registry = Registry::from_allowed_host(&allowed_host);

    println!();
    println!("  Package:  {}", pkg_name);
    if let Some(ref v) = requested_version {
        println!("  Version:  {}", v);
    }
    println!(
        "  Registry: {} (allowed via --allow-net={})",
        registry.display_name(),
        allowed_host
    );
    println!();

    // ── Registry lookup ───────────────────────────────────────────────────────
    println!("  Checking registry...");
    let info = match lookup_registry(&registry, &pkg_name).await {
        Ok(i) => i,
        Err(e) => {
            eprintln!();
            eprintln!("✗ {}", e);
            anyhow::bail!("{}", e);
        }
    };

    println!("  ✓ Package found on {}", registry.display_name());
    if let Some(ref latest) = info.latest {
        println!("    Latest:   {}@{}", pkg_name, latest);
    }
    println!("    Versions: {}", info.versions.len());

    // ── Version resolution ────────────────────────────────────────────────────
    let resolved_version = if let Some(ref v) = requested_version {
        if info.versions.contains(v) {
            println!("  ✓ Version {}@{} exists", pkg_name, v);
            v.clone()
        } else {
            eprintln!();
            eprintln!("✗ Version {}@{} not found in registry.", pkg_name, v);

            let nearby = find_nearby_versions(v, &info.versions, 5);
            if !nearby.is_empty() {
                eprintln!();
                eprintln!("  Versions available near {}:", v);
                for nv in &nearby {
                    eprintln!("    {}@{}", pkg_name, nv);
                }
            }
            anyhow::bail!("Version not found: {}@{}", pkg_name, v);
        }
    } else {
        let latest = info
            .latest
            .clone()
            .or_else(|| info.versions.last().cloned())
            .unwrap_or_else(|| "*".to_string());
        println!("  Using latest: {}@{}", pkg_name, latest);
        latest
    };

    // ── Download, verify, extract ─────────────────────────────────────────────
    std::fs::create_dir_all(".3va-cache")
        .map_err(|e| anyhow::anyhow!("Cannot create cache directory: {}", e))?;

    let safe_name = pkg_name.replace('/', "-").trim_matches('-').to_string();
    let cached_tarball =
        PathBuf::from(".3va-cache").join(format!("{}-{}.tgz", safe_name, resolved_version));
    let node_modules_dest = PathBuf::from("node_modules").join(&pkg_name);

    // Check if already fully extracted at correct version (skip if not forced)
    let already_extracted = !force && {
        let pkg_json = node_modules_dest.join("package.json");
        pkg_json.exists()
            && std::fs::read_to_string(&pkg_json)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| v["version"].as_str().map(|s| s.to_string()))
                .as_deref()
                == Some(resolved_version.as_str())
    };

    if already_extracted {
        println!("  ✓ Already extracted in node_modules/{}", pkg_name);
    } else {
        // Fetch or use cached tarball
        let tarball_bytes = if cached_tarball.exists() && !force {
            println!("  ✓ Using cached tarball");
            std::fs::read(&cached_tarball)?
        } else {
            let tarball_url = info
                .version_meta
                .get(&resolved_version)
                .map(|m| m.tarball.clone())
                .unwrap_or_else(|| {
                    format!(
                        "{}/{}/-/{}-{}.tgz",
                        registry.base_url(),
                        pkg_name,
                        pkg_name,
                        resolved_version
                    )
                });

            println!("  Downloading {}@{} ...", pkg_name, resolved_version);
            tracing::info!("Downloading from {}", tarball_url);
            let bytes = download_tarball(&tarball_url).await?;
            std::fs::write(&cached_tarball, &bytes)?;
            bytes
        };

        // Verify integrity against registry metadata
        if let Some(meta) = info.version_meta.get(&resolved_version) {
            if let Some(ref integrity) = meta.integrity {
                print!("  Verifying integrity... ");
                match verify_integrity(&tarball_bytes, integrity) {
                    Ok(()) => println!("✓"),
                    Err(e) => {
                        // Remove corrupt cached tarball
                        let _ = std::fs::remove_file(&cached_tarball);
                        eprintln!();
                        eprintln!("✗ {}", e);
                        anyhow::bail!("{}", e);
                    }
                }
            } else {
                println!("  ! Warning: No integrity hash in registry metadata");
            }
        }

        // Extract to node_modules/
        println!("  Extracting to node_modules/{} ...", pkg_name);
        std::fs::create_dir_all("node_modules")?;
        extract_tarball(&tarball_bytes, &node_modules_dest)?;
        println!("  ✓ Extracted successfully");
    }

    // ── package.json ──────────────────────────────────────────────────────────
    let pkg_json_path = PathBuf::from("package.json");
    let (project_name, project_version, mut deps) = if pkg_json_path.exists() {
        let content = std::fs::read_to_string(&pkg_json_path)?;
        let val: serde_json::Value =
            serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));

        let pname = val["name"].as_str().unwrap_or("project").to_string();
        let pver = val["version"].as_str().unwrap_or("0.0.0").to_string();
        let mut dep_map: HashMap<String, String> = HashMap::new();
        if let Some(deps_obj) = val["dependencies"].as_object() {
            for (k, v) in deps_obj {
                if let Some(ver) = v.as_str() {
                    let normalized = normalize_version(ver);
                    if !normalized.is_empty() {
                        dep_map.insert(k.clone(), normalized);
                    }
                }
            }
        }
        (pname, pver, dep_map)
    } else {
        tracing::info!("No package.json found — creating a minimal one.");
        let minimal = serde_json::json!({
            "name": "project",
            "version": "0.0.0",
            "description": "",
            "main": "index.js",
            "type": "module",
            "dependencies": {}
        });
        std::fs::write(&pkg_json_path, serde_json::to_string_pretty(&minimal)?)?;
        ("project".to_string(), "0.0.0".to_string(), HashMap::new())
    };

    let already_installed = deps.contains_key(&pkg_name);
    let existing_version = deps.get(&pkg_name).cloned();

    if already_installed && !force {
        let existing_ver = existing_version.as_deref().unwrap_or("*");
        let same_version = requested_version
            .as_deref()
            .is_none_or(|rv| rv == existing_ver);
        if same_version {
            println!();
            println!("✓ {}@{} is already installed.", pkg_name, existing_ver);
            println!("  Use 'reinstall' to force reinstall.");
            return Ok(());
        }
        println!(
            "  Updating {}@{} → {}@{}",
            pkg_name, existing_ver, pkg_name, resolved_version
        );
    }

    deps.insert(pkg_name.clone(), resolved_version.clone());

    // Write updated package.json
    if let Ok(content) = std::fs::read_to_string(&pkg_json_path)
        && let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&content)
    {
        if let Some(obj) = val.as_object_mut() {
            let deps_obj = obj
                .entry("dependencies")
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if let Some(deps_map) = deps_obj.as_object_mut() {
                deps_map.insert(
                    pkg_name.clone(),
                    serde_json::Value::String(resolved_version.clone()),
                );
            }
        }
        std::fs::write(&pkg_json_path, serde_json::to_string_pretty(&val).unwrap())?;
    }

    // Generate lockfile
    let cache_dir = PathBuf::from(".3va-cache");
    let mut pm = PackageManager::new(cache_dir);
    let mut lockfile = pm.install(&deps, &project_name, &project_version)?;

    let lockfile_path = PathBuf::from("3va-lock.json");

    // Preserve existing registry assignments from the previous lockfile before overwriting
    if let Ok(old_lock) = Lockfile::load(&lockfile_path) {
        for (name, dep) in &old_lock.dependencies {
            if let Some(reg) = &dep.registry {
                lockfile.set_registry(name, reg);
            }
        }
    }
    // Record the registry for the package we just installed/updated
    lockfile.set_registry(&pkg_name, registry.display_name());

    lockfile.save(&lockfile_path)?;
    tracing::info!(
        "Lockfile written to 3va-lock.json ({} packages).",
        lockfile.packages.len()
    );

    println!();
    if force {
        println!(
            "✓ {}@{} reinstalled successfully.",
            pkg_name, resolved_version
        );
    } else {
        println!(
            "✓ {}@{} installed successfully.",
            pkg_name, resolved_version
        );
    }
    println!("  Run: 3va run <your-file>.ts");

    Ok(())
}
