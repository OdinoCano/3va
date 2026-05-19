pub mod fetcher;
pub mod lockfile;
pub mod manifest;
pub mod malware_scanner;
pub mod resolver;
pub mod semver;
pub mod signature_verifier;

pub use manifest::{PackageManifest, PackageInfo, PackagePermissions};
pub use lockfile::Lockfile;
pub use malware_scanner::{MalwareScanner, ScanResult, Threat, ThreatLevel};
pub use resolver::{DependencyGraph, DependencyNode, Resolver};
pub use semver::{Semver, SemverRange};
pub use signature_verifier::{SignatureVerifier, VerificationStatus, HashAlgorithm, SignatureInfo};

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
        let h = host.trim().trim_start_matches("https://").trim_start_matches("http://");
        let h = h.split('/').next().unwrap_or(h); // quita paths
        if h.contains("jsr.io") {
            Registry::Jsr
        } else if h.contains("yarnpkg.com") {
            Registry::Yarn
        } else if h.contains("npmjs.org") || h.contains("npmjs.com") {
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
    cache_dir: PathBuf,
}

impl PackageManager {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            resolver: Resolver::new("https://registry.npmjs.org"),
            cache_dir,
        }
    }

    pub fn install(&mut self, deps: &HashMap<String, String>, project_name: &str, project_version: &str) -> anyhow::Result<Lockfile> {
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

struct RegistryInfo {
    versions: Vec<String>,
    latest: Option<String>,
}

async fn lookup_npm_compat(client: &reqwest::Client, base_url: &str, pkg_name: &str) -> anyhow::Result<RegistryInfo> {
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
    let versions: Vec<String> = data["versions"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();

    Ok(RegistryInfo { versions, latest })
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

    let url = format!("https://jsr.io/api/scopes/{}/packages/{}/versions", scope, name);
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;

    if resp.status().as_u16() == 404 {
        anyhow::bail!("Package '{}' not found on jsr.io", pkg_name);
    }
    if !resp.status().is_success() {
        anyhow::bail!("JSR returned HTTP {}", resp.status());
    }

    let data: serde_json::Value = resp.json().await?;
    let versions: Vec<String> = data["items"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item["version"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let latest = versions.last().cloned();
    Ok(RegistryInfo { versions, latest })
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
    scored.iter().take(count).map(|(_, v)| (*v).clone()).collect()
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
    let v = version.trim_start_matches('^').trim_start_matches('*').to_string();
    if v.is_empty() { "*".to_string() } else { v }
}

fn parse_package_spec(input: &str) -> anyhow::Result<(String, Option<String>)> {
    // Scoped packages: @scope/name or @scope/name@version
    if input.starts_with('@') {
        let after_at = &input[1..];
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
        anyhow::bail!("Invalid scoped package format: '{}'. Expected @scope/name", input);
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
    if let Some(v) = version {
        if v.contains(':') {
            anyhow::bail!("Invalid version '{}'. Use name@version format (not name:version)", v);
        }
    }

    Ok((name.to_string(), version.map(|s| s.to_string())))
}

// ── Public API ────────────────────────────────────────────────────────────────

pub async fn install_package(name: &str, allow_net: Option<&[String]>) -> anyhow::Result<()> {
    install_package_impl(name, false, allow_net).await
}

pub async fn reinstall_package(name: &str, allow_net: Option<&[String]>) -> anyhow::Result<()> {
    install_package_impl(name, true, allow_net).await
}

async fn install_package_impl(input: &str, force: bool, allow_net: Option<&[String]>) -> anyhow::Result<()> {
    let (pkg_name, requested_version) = parse_package_spec(input)?;

    if let Some(ref v) = requested_version {
        if v.is_empty() {
            eprintln!();
            eprintln!("✗ Error: Empty version is not allowed. Use {}@<version>", pkg_name);
            anyhow::bail!("Empty version specified");
        }
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
            eprintln!("    3va install {} --allow-net=registry.npmjs.org", pkg_name);
            eprintln!("    3va install {} --allow-net=registry.yarnpkg.com", pkg_name);
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
    println!("  Registry: {} (allowed via --allow-net={})", registry.display_name(), allowed_host);
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

    // ── Already installed check ───────────────────────────────────────────────
    if let Err(e) = std::fs::create_dir_all(".3va-cache") {
        eprintln!("✗ Error: Cannot create cache directory: {}", e);
        anyhow::bail!("Failed to create cache directory");
    }

    let tarball_path = PathBuf::from(".3va-cache")
        .join(format!("{}.tgz", pkg_name.replace('/', "-")));

    tracing::info!("Verifying signatures for '{}'...", pkg_name);
    let verifier = SignatureVerifier::default();
    match verifier.verify_from_registry(&pkg_name, &resolved_version, &tarball_path) {
        VerificationStatus::Verified => println!("  ✓ Signatures verified"),
        VerificationStatus::Unverified => println!("  ! Warning: Package signatures not verified"),
        VerificationStatus::Missing => println!("  ! Warning: No signature data available for verification"),
        VerificationStatus::Mismatch => {
            eprintln!("✗ Error: Package hash mismatch — possible tampering detected!");
            anyhow::bail!("Signature verification failed: hash mismatch");
        }
        VerificationStatus::Failed(reason) => {
            eprintln!("✗ Error: Signature verification failed: {}", reason);
            anyhow::bail!("Signature verification failed");
        }
    }

    let cache_path = PathBuf::from(".3va-cache")
        .join(format!("{}-cache", pkg_name.replace('/', "-")));
    std::fs::create_dir_all(&cache_path).ok();

    // ── package.json ──────────────────────────────────────────────────────────
    let pkg_json_path = PathBuf::from("package.json");
    let (project_name, project_version, mut deps) = if pkg_json_path.exists() {
        let content = std::fs::read_to_string(&pkg_json_path)?;
        let val: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| serde_json::json!({}));

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
        let same_version = requested_version.as_deref().map_or(true, |rv| rv == existing_ver);
        if same_version {
            println!();
            println!("✓ {}@{} is already installed.", pkg_name, existing_ver);
            println!("  Use 'reinstall' to force reinstall.");
            return Ok(());
        }
        println!("  Updating {}@{} → {}@{}", pkg_name, existing_ver, pkg_name, resolved_version);
    }

    deps.insert(pkg_name.clone(), resolved_version.clone());

    // Write updated package.json
    if let Ok(content) = std::fs::read_to_string(&pkg_json_path) {
        if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(&content) {
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
    }

    // Generate lockfile
    let cache_dir = PathBuf::from(".3va-cache");
    let mut pm = PackageManager::new(cache_dir);
    let lockfile = pm.install(&deps, &project_name, &project_version)?;

    let lockfile_path = PathBuf::from("3va-lock.json");
    lockfile.save(&lockfile_path)?;
    tracing::info!("Lockfile written to 3va-lock.json ({} packages).", lockfile.packages.len());

    println!();
    if force {
        println!("✓ {}@{} reinstalled successfully.", pkg_name, resolved_version);
    } else {
        println!("✓ {}@{} installed successfully.", pkg_name, resolved_version);
    }
    println!("  Run: 3va run <your-file>.ts");

    Ok(())
}
