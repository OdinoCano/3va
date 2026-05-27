pub mod auditor;
pub mod fetcher;
pub mod lockfile;
pub mod malware_scanner;
pub mod manifest;
pub mod resolver;
pub mod secrets;
pub mod semver;
pub mod signature_verifier;
pub mod store;
pub mod workspace;

pub use secrets::{SecretFinding, SecretsScanner, Severity as SecretSeverity};
pub use store::{ContentStore, PruneResult, StoreStats};
pub use workspace::{WorkspaceConfig, WorkspacePackage, create_workspace_symlinks, merged_deps};

pub use auditor::{
    AuditReport, VulnFinding, VulnSeverity, Vulnerability, print_audit_report, run_audit,
};
pub use lockfile::Lockfile;
pub use malware_scanner::{MalwareScanner, ScanResult, Threat, ThreatLevel};
pub use manifest::{PackageInfo, PackageManifest, PackagePermissions};
pub use resolver::{DependencyGraph, DependencyNode, Resolver};
pub use semver::{Semver, SemverRange};
pub use signature_verifier::{HashAlgorithm, SignatureInfo, SignatureVerifier, VerificationStatus};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    _cache_dir: PathBuf,
}

impl PackageManager {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            resolver: Resolver::new("https://registry.npmjs.org"),
            _cache_dir: cache_dir,
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

    pub fn load_lockfile(path: &Path) -> anyhow::Result<Lockfile> {
        Lockfile::load(path)
    }

    pub fn save_lockfile(&self, lockfile: &Lockfile, path: &Path) -> anyhow::Result<()> {
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

// ── Download + extract ────────────────────────────────────────────────────────

/// Download a tarball with up to `MAX_RETRIES` retries and exponential backoff.
/// Retries on connection/timeout errors and 5xx responses; fails immediately
/// on 4xx (package not found, auth errors, etc.).
const MAX_RETRIES: u32 = 3;
const RETRY_BASE_MS: u64 = 400;

async fn download_tarball(url: &str) -> anyhow::Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()?;

    let mut last_err = anyhow::anyhow!("unreachable");

    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            let delay_ms = RETRY_BASE_MS * 2u64.pow(attempt - 1);
            tracing::warn!(
                "Download attempt {}/{} for {} failed, retrying in {}ms...",
                attempt,
                MAX_RETRIES,
                url,
                delay_ms
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        let result = client
            .get(url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Network error: {}", e));

        let resp = match result {
            Ok(r) => r,
            Err(e) => {
                last_err = e;
                continue; // transient — retry
            }
        };

        let status = resp.status();
        // 4xx = permanent failure (bad URL, auth, not found); don't retry.
        if status.is_client_error() {
            anyhow::bail!("Failed to download {}: HTTP {} (not retrying)", url, status);
        }
        if !status.is_success() {
            last_err = anyhow::anyhow!("HTTP {} from {}", status, url);
            continue; // 5xx — retry
        }

        return Ok(resp.bytes().await?.to_vec());
    }

    Err(last_err.context(format!(
        "All {} download attempts failed for {}",
        MAX_RETRIES + 1,
        url
    )))
}

pub(crate) fn extract_tarball(data: &[u8], dest: &PathBuf) -> anyhow::Result<()> {
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
    let root = std::env::current_dir()?;
    install_with_transitive(name, false, allow_net, &root, true).await
}

pub async fn reinstall_package(name: &str, allow_net: Option<&[String]>) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    install_with_transitive(name, true, allow_net, &root, true).await
}

/// Install all `dependencies` listed in `project_root/package.json`.
/// This is what `3va install` (no args, no workspace) does.
pub async fn install_from_manifest(
    project_root: &Path,
    allow_net: Option<&[String]>,
) -> anyhow::Result<()> {
    let pkg_json = project_root.join("package.json");
    if !pkg_json.exists() {
        anyhow::bail!(
            "No package.json found in {}.\nCreate one or pass package names explicitly.",
            project_root.display()
        );
    }

    let content = std::fs::read_to_string(&pkg_json)?;
    let val: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Invalid package.json: {}", e))?;

    let mut all_deps: Vec<(String, String)> = Vec::new();
    for key in ["dependencies", "devDependencies"] {
        if let Some(obj) = val[key].as_object() {
            for (name, version) in obj {
                if let Some(ver) = version.as_str() {
                    all_deps.push((name.clone(), ver.to_string()));
                }
            }
        }
    }

    if all_deps.is_empty() {
        println!("Nothing to install — package.json has no dependencies.");
        return Ok(());
    }

    println!();
    println!("Installing {} dep(s) from manifest...", all_deps.len());

    // Install concurrently: each dep gets its own task.
    let mut set = tokio::task::JoinSet::new();
    let allow_net_owned: Option<Vec<String>> = allow_net.map(|v| v.to_vec());
    let root = project_root.to_path_buf();

    for (name, version) in all_deps {
        let spec = format!("{}@{}", name, normalize_version(&version));
        let an = allow_net_owned.clone();
        let r = root.clone();
        set.spawn(
            async move { install_with_transitive(&spec, false, an.as_deref(), &r, false).await },
        );
    }

    let mut errors = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e.to_string()),
            Err(e) => errors.push(format!("task panic: {}", e)),
        }
    }

    if !errors.is_empty() {
        anyhow::bail!(
            "{} dep(s) failed to install:\n{}",
            errors.len(),
            errors.join("\n")
        );
    }

    println!();
    println!("✓ All dependencies installed.");
    Ok(())
}

/// BFS install: installs `root_spec` and all of its transitive dependencies
/// into `project_root/node_modules/`.
///
/// `update_manifest`: when true the package.json and lockfile in
/// `project_root` are updated.  Pass false for transitive deps.
async fn install_with_transitive(
    root_spec: &str,
    force: bool,
    allow_net: Option<&[String]>,
    project_root: &Path,
    update_manifest: bool,
) -> anyhow::Result<()> {
    use std::collections::{HashSet, VecDeque};

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(root_spec.to_string());
    let mut is_root = true;

    while let Some(spec) = queue.pop_front() {
        if visited.contains(&spec) {
            continue;
        }
        visited.insert(spec.clone());

        let current_is_root = is_root;
        is_root = false;

        // Parse just the name to check if already installed.
        let (pkg_name, _) = parse_package_spec(&spec)?;
        let node_modules_dest = project_root.join("node_modules").join(&pkg_name);
        let already_ok = !force && node_modules_dest.join("package.json").exists();

        if !already_ok || (current_is_root && force) {
            install_package_impl(
                &spec,
                force && current_is_root,
                allow_net,
                project_root,
                update_manifest && current_is_root,
            )
            .await?;
        }

        // Enqueue transitive deps from the installed package.json.
        if let Ok(content) = std::fs::read_to_string(node_modules_dest.join("package.json"))
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

/// Like `audit_packages()` but produces no output — for use in JSON mode.
pub fn audit_packages_silent() -> anyhow::Result<bool> {
    let node_modules = PathBuf::from("node_modules");
    if !node_modules.exists() {
        return Ok(true); // no packages → nothing malicious
    }

    let lockfile_path = PathBuf::from("3va-lock.json");
    let installed: Vec<(String, String)> = if lockfile_path.exists() {
        let lock = Lockfile::load(&lockfile_path)?;
        lock.dependencies
            .iter()
            .map(|(name, dep)| (name.clone(), dep.version.clone()))
            .collect()
    } else {
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
                    if name.starts_with('@') {
                        if let Ok(sub) = std::fs::read_dir(&path) {
                            for sub_entry in sub.flatten() {
                                pkgs.push((
                                    format!("{}/{}", name, sub_entry.file_name().to_string_lossy()),
                                    "unknown".to_string(),
                                ));
                            }
                        }
                    } else {
                        pkgs.push((name, "unknown".to_string()));
                    }
                }
            }
        }
        pkgs
    };

    let scanner = MalwareScanner::new();
    let mut any_critical = false;
    for (pkg_name, _version) in &installed {
        let pkg_dir = node_modules.join(pkg_name);
        if !pkg_dir.exists() {
            continue;
        }
        let results = scanner.scan_directory(&pkg_dir);
        for result in &results {
            for threat in &result.threats {
                if matches!(threat.severity, ThreatLevel::Critical) {
                    any_critical = true;
                }
            }
        }
    }
    Ok(!any_critical)
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
    let cwd = std::env::current_dir()?;

    for pkg in &targets {
        if let Some(reg) = lockfile.registry_for(pkg) {
            let host = reg.to_string();
            install_package_impl(pkg, true, Some(&[host]), &cwd, true).await?;
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
    project_root: &Path,
    update_manifest: bool,
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

    // ── Download, store, link ─────────────────────────────────────────────────
    let cache_dir = project_root.join(".3va-cache");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| anyhow::anyhow!("Cannot create cache directory: {}", e))?;

    let safe_pkg = pkg_name.replace('/', "-").trim_matches('-').to_string();
    let cached_tarball = cache_dir.join(format!("{}-{}.tgz", safe_pkg, resolved_version));
    let node_modules = project_root.join("node_modules");
    let node_modules_dest = node_modules.join(&pkg_name);

    // Skip if already linked at the correct version.
    let already_linked = !force && {
        let pkg_json = node_modules_dest.join("package.json");
        pkg_json.exists()
            && std::fs::read_to_string(&pkg_json)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| v["version"].as_str().map(|s| s.to_string()))
                .as_deref()
                == Some(resolved_version.as_str())
    };

    if already_linked {
        println!("  ✓ Already installed in node_modules/{}", pkg_name);
    } else {
        let global_store = store::ContentStore::global();
        let reg_name = registry.display_name();

        // Phase 1: Ensure the package is in the global content-addressable store.
        // A store hit means zero network traffic for every project after the first.
        if global_store.is_cached(reg_name, &pkg_name, &resolved_version) && !force {
            println!("  ✓ Found in global store (~/.3va/store)");
        } else {
            // Fetch bytes — prefer the per-project tarball cache, then network.
            let tarball_bytes: Vec<u8> = if cached_tarball.exists() && !force {
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

            // Verify integrity before writing to the store.
            if let Some(meta) = info.version_meta.get(&resolved_version) {
                print!("  Verifying integrity... ");
                let verifier = SignatureVerifier::sha512();
                match verifier.verify_from_registry(&tarball_bytes, meta.integrity.as_deref()) {
                    VerificationStatus::Verified => println!("✓"),
                    VerificationStatus::Mismatch => {
                        let _ = std::fs::remove_file(&cached_tarball);
                        eprintln!();
                        anyhow::bail!(
                            "Integrity check failed for {}@{}: hash mismatch",
                            pkg_name,
                            resolved_version
                        );
                    }
                    VerificationStatus::Missing => {
                        println!("  (!) No integrity hash in registry — skipping check");
                    }
                    VerificationStatus::Failed(e) => {
                        let _ = std::fs::remove_file(&cached_tarball);
                        anyhow::bail!(
                            "Integrity check failed for {}@{}: {}",
                            pkg_name,
                            resolved_version,
                            e
                        );
                    }
                    VerificationStatus::Unverified => {
                        println!("  (!) Integrity unverified");
                    }
                }
            }

            // Atomically store — concurrent processes writing the same package are safe.
            println!(
                "  Storing {}@{} in global store...",
                pkg_name, resolved_version
            );
            global_store.store_tarball(&tarball_bytes, reg_name, &pkg_name, &resolved_version)?;
        }

        // Phase 2: Hard-link (or copy) from global store into this project's node_modules/.
        println!("  Linking to node_modules/{} ...", pkg_name);
        std::fs::create_dir_all(&node_modules)?;
        global_store.link_to_node_modules(reg_name, &pkg_name, &resolved_version, &node_modules)?;
        println!("  ✓ Linked from ~/.3va/store");
    }

    // ── package.json + lockfile ───────────────────────────────────────────────
    // Only update when asked (top-level installs), not for transitive deps.
    if !update_manifest {
        return Ok(());
    }

    let pkg_json_path = project_root.join("package.json");
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
    let mut pm = PackageManager::new(project_root.join(".3va-cache"));
    let mut lockfile = pm.install(&deps, &project_name, &project_version)?;

    let lockfile_path = project_root.join("3va-lock.json");

    // Preserve existing registry assignments before overwriting.
    if let Ok(old_lock) = Lockfile::load(&lockfile_path) {
        for (name, dep) in &old_lock.dependencies {
            if let Some(reg) = &dep.registry {
                lockfile.set_registry(name, reg);
            }
        }
    }
    lockfile.set_registry(&pkg_name, registry.display_name());

    lockfile.save(&lockfile_path)?;
    tracing::info!("Lockfile written ({} packages).", lockfile.packages.len());

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

// ── Workspace install ─────────────────────────────────────────────────────────

/// Install all dependencies across a workspace rooted at `root`.
///
/// 1. Discovers workspace packages via `3va-workspace.json` or `workspaces`
///    in the root `package.json`.
/// 2. Merges all deps (highest version wins on conflict).
/// 3. Installs each unique dep once into the *root* `node_modules/`, using the
///    global content-addressable store so packages shared with other projects
///    are hard-linked rather than duplicated.
/// 4. Installs each workspace package's own deps into its *local*
///    `node_modules/` via the same store (also just hard-links).
pub async fn install_workspace(
    root: &std::path::Path,
    allow_net: Option<&[String]>,
) -> anyhow::Result<()> {
    let cfg = match workspace::WorkspaceConfig::discover(root)? {
        Some(c) => c,
        None => {
            anyhow::bail!(
                "No workspace config found in {}\n\
                 Create a 3va-workspace.json with a \"packages\" array, or add a\n\
                 \"workspaces\" key to your root package.json.",
                root.display()
            );
        }
    };

    let packages = cfg.resolve_packages(root)?;
    if packages.is_empty() {
        println!("No workspace packages found.");
        return Ok(());
    }

    println!();
    println!("Workspace: {} package(s) discovered", packages.len());
    for p in &packages {
        println!("  {} @ {} ({})", p.name, p.version, p.path.display());
    }
    println!();

    // ── Step 1: install merged deps into the workspace root node_modules/ ─────
    // Deps are deduplicated across all packages; the highest version wins.
    // All packages are installed concurrently into the root node_modules/.
    let merged = workspace::merged_deps(&packages);
    if !merged.is_empty() {
        println!(
            "Installing {} unique dep(s) into workspace root...",
            merged.len()
        );
        println!();

        let allow_net_owned: Option<Vec<String>> = allow_net.map(|v| v.to_vec());
        let mut set = tokio::task::JoinSet::new();

        for (dep_name, dep_version) in &merged {
            let spec = format!("{}@{}", dep_name, normalize_version(dep_version));
            let an = allow_net_owned.clone();
            let r = root.to_path_buf();
            // update_manifest=false for root-level shared deps (each package
            // manages its own manifest; the root has no package.json of its own
            // necessarily).
            set.spawn(async move {
                install_with_transitive(&spec, false, an.as_deref(), &r, false).await
            });
        }

        let mut errors = Vec::new();
        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(())) => {}
                Ok(Err(e)) => errors.push(e.to_string()),
                Err(e) => errors.push(format!("task panic: {}", e)),
            }
        }
        if !errors.is_empty() {
            anyhow::bail!(
                "{} dep(s) failed during workspace root install:\n{}",
                errors.len(),
                errors.join("\n")
            );
        }
    }

    // ── Step 2: install each workspace package's own deps locally ─────────────
    // Packages run sequentially to avoid conflicting lockfile writes, but their
    // downloads are all store-cached from step 1 so this is just hard-links.
    for pkg in &packages {
        if pkg.all_deps.is_empty() {
            continue;
        }
        println!();
        println!(
            "  [{}] linking {} dep(s) into local node_modules...",
            pkg.name,
            pkg.all_deps.len()
        );

        for (dep_name, dep_version) in &pkg.all_deps {
            let spec = format!("{}@{}", dep_name, normalize_version(dep_version));
            // update_manifest=true so each package's package.json + lockfile is updated.
            install_with_transitive(&spec, false, allow_net, &pkg.path, true).await?;
        }
    }

    // ── Step 3: create symlinks for workspace: cross-references ───────────────
    workspace::create_workspace_symlinks(root, &packages)?;

    println!();
    println!("✓ Workspace install complete.");
    println!();

    let stats = store::ContentStore::global().stats();
    println!(
        "  Global store: {} package(s), {}  ({})",
        stats.total_packages,
        stats.human_size(),
        stats.store_path.display()
    );

    Ok(())
}

/// Print a summary of the global content-addressable store.
pub fn store_status() {
    let stats = store::ContentStore::global().stats();
    println!();
    println!("Global store: {}", stats.store_path.display());
    println!("  Packages : {}", stats.total_packages);
    println!("  Disk use : {}", stats.human_size());
    println!();
    if stats.total_packages > 0 {
        println!(
            "  Every project that shares a package version uses hard-links — no duplicate files."
        );
    }
}
