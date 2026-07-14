//! Package manager — install, update, audit, and lockfile management for 3va projects.

pub mod auditor;
pub mod fetcher;
pub mod lockfile;
pub mod malware_scanner;
pub mod manifest;
pub mod npmrc;
pub mod package_lock;
pub mod pnpm_lock;
pub mod resolver;
pub mod secrets;
pub mod semver;
pub mod signature_verifier;
pub mod store;
pub mod workspace;
pub mod workspace_v2;
pub mod yarn_lock;

pub use secrets::{SecretFinding, SecretsScanner, Severity as SecretSeverity};
pub use store::{ContentStore, PruneResult, StoreStats, virtual_entry_name};
pub use workspace::{WorkspaceConfig, WorkspacePackage, create_workspace_symlinks, merged_deps};
pub use workspace_v2::{
    PackageRunResult, RunStatus, WorkspaceGraph, git_changed_files, print_run_results,
    run_workspace_script,
};

pub use auditor::{
    AuditReport, VulnFinding, VulnSeverity, Vulnerability, print_audit_report, run_audit,
};
pub use lockfile::Lockfile;
pub use malware_scanner::{MalwareScanner, ScanResult, Threat, ThreatLevel};
pub use manifest::{PackageInfo, PackageManifest, PackagePermissions};
pub use npmrc::{NpmrcConfig, discover_npmrc, parse_npmrc, resolve_registry};
pub use package_lock::load_from_package_lock;
pub use pnpm_lock::load_from_pnpm_lock;
pub use resolver::{DependencyGraph, DependencyNode, Resolver};
pub use semver::{Semver, SemverRange};
pub use signature_verifier::{HashAlgorithm, SignatureInfo, SignatureVerifier, VerificationStatus};
pub use yarn_lock::load_from_yarn_lock;

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
        // Empty host means --allow-net= (allow all) → default to npm registry
        if h.is_empty() {
            return Registry::Npm;
        }
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

// ── Lockfile auto-detection ────────────────────────────────────────────────────

/// Try to detect and load any supported lockfile format from the given project
/// directory.  Checks in order: `3va-lock.json`, `package-lock.json`,
/// `yarn.lock`, `pnpm-lock.yaml`.
pub fn detect_lockfile(project_root: &std::path::Path) -> anyhow::Result<Option<Lockfile>> {
    // 1. Native format
    let native = project_root.join("3va-lock.json");
    if native.exists() {
        return Lockfile::load(&native).map(Some);
    }
    // 2. npm package-lock.json
    let npm = project_root.join("package-lock.json");
    if npm.exists() {
        return package_lock::load_from_package_lock(&npm);
    }
    // 3. yarn.lock
    let yarn = project_root.join("yarn.lock");
    if yarn.exists() {
        return yarn_lock::load_from_yarn_lock(&yarn);
    }
    // 4. pnpm-lock.yaml
    let pnpm = project_root.join("pnpm-lock.yaml");
    if pnpm.exists() {
        return pnpm_lock::load_from_pnpm_lock(&pnpm);
    }
    Ok(None)
}

/// Migrate an external lockfile to the native `3va-lock.json` format.
///
/// Reads any supported lockfile and writes it as `3va-lock.json` in the same
/// directory.  Returns `true` if a migration happened.
pub fn migrate_lockfile(project_root: &std::path::Path) -> anyhow::Result<bool> {
    if project_root.join("3va-lock.json").exists() {
        return Ok(false); // Already migrated
    }
    let lockfile = match detect_lockfile(project_root)? {
        Some(l) => l,
        None => return Ok(false),
    };
    let out = project_root.join("3va-lock.json");
    lockfile.save(&out)?;
    println!("✓ Migrated lockfile → {}", out.display());
    Ok(true)
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

    pub async fn install(
        &mut self,
        deps: &HashMap<String, String>,
        project_name: &str,
        project_version: &str,
    ) -> anyhow::Result<Lockfile> {
        let graph = self.resolver.resolve(deps).await;
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
        .header(
            "Accept",
            "application/vnd.npm.install-v1+json, application/json",
        )
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

/// Fetch only the metadata for a specific version using the abbreviated endpoint.
/// Much faster than downloading the full packument (all versions).
/// Returns RegistryInfo and transitive dep specs (name, version-range).
async fn lookup_npm_version(
    client: &reqwest::Client,
    base_url: &str,
    pkg_name: &str,
    version: &str,
) -> anyhow::Result<(RegistryInfo, Vec<(String, String)>)> {
    let url = format!("{}/{}/{}", base_url, pkg_name, version);
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;

    if resp.status().as_u16() == 404 || !resp.status().is_success() {
        // Fall back to full packument to get dist-tags etc.
        return lookup_npm_compat_with_deps(client, base_url, pkg_name).await;
    }

    let meta: serde_json::Value = resp.json().await?;
    let resolved_ver = meta["version"].as_str().unwrap_or(version).to_string();
    let tarball = meta["dist"]["tarball"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            format!(
                "{}/{}/-/{}-{}.tgz",
                base_url, pkg_name, pkg_name, resolved_ver
            )
        });
    let integrity = meta["dist"]["integrity"].as_str().map(|s| s.to_string());
    let mut version_meta = HashMap::new();
    version_meta.insert(resolved_ver.clone(), VersionMeta { tarball, integrity });

    // Collect transitive dep specs (name + version range) from the registry response.
    // Peers are installed the same way regular deps are — same BFS, same
    // --allow-net grant the user already gave, no separate prompt.
    let dep_specs = collect_dep_specs(&meta);

    let info = RegistryInfo {
        versions: vec![resolved_ver],
        latest: None,
        version_meta,
    };
    Ok((info, dep_specs))
}

/// Pull `dependencies` + `peerDependencies` (name, range) pairs out of an
/// npm registry version object. Peers are treated identically to regular
/// deps so autoinstall reuses the same BFS and permission grant.
fn collect_dep_specs(meta: &serde_json::Value) -> Vec<(String, String)> {
    let mut dep_specs = Vec::new();
    for key in ["dependencies", "peerDependencies"] {
        if let Some(deps) = meta[key].as_object() {
            for (dep_name, dep_ver) in deps {
                if let Some(dv) = dep_ver.as_str() {
                    dep_specs.push((dep_name.clone(), dv.to_string()));
                }
            }
        }
    }
    dep_specs
}

async fn lookup_npm_compat_with_deps(
    client: &reqwest::Client,
    base_url: &str,
    pkg_name: &str,
) -> anyhow::Result<(RegistryInfo, Vec<(String, String)>)> {
    let info = lookup_npm_compat(client, base_url, pkg_name).await?;
    // For the full packument we don't know which version was picked yet,
    // return empty deps — they'll be read after extract.
    Ok((info, Vec::new()))
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
    let npm_name = format!("@jsr/{}__{}", scope, name);
    lookup_npm_compat(client, "https://npm.jsr.io", &npm_name)
        .await
        .map_err(|e| anyhow::anyhow!("Package '{}' not found on jsr.io: {}", pkg_name, e))
}

/// Lookup JSR and return (RegistryInfo, dep_specs) for parallel BFS.
async fn lookup_jsr_with_deps(
    client: &reqwest::Client,
    pkg_name: &str,
    version: &str,
) -> anyhow::Result<(RegistryInfo, Vec<(String, String)>)> {
    if !pkg_name.starts_with('@') || !pkg_name.contains('/') {
        anyhow::bail!(
            "JSR only supports scoped packages (e.g. @scope/name). '{}' is not a valid JSR package name",
            pkg_name
        );
    }
    let trimmed = pkg_name.trim_start_matches('@');
    let (scope, name) = trimmed.split_once('/').unwrap();
    let npm_name = format!("@jsr/{}__{}", scope, name);
    lookup_npm_version(client, "https://npm.jsr.io", &npm_name, version)
        .await
        .map_err(|e| anyhow::anyhow!("Package '{}' not found on jsr.io: {}", pkg_name, e))
}

async fn lookup_registry(registry: &Registry, pkg_name: &str) -> anyhow::Result<RegistryInfo> {
    let client = reqwest::Client::builder().gzip(true).build()?;
    match registry {
        Registry::Jsr => lookup_jsr(&client, pkg_name).await,
        Registry::Npm | Registry::Yarn | Registry::Custom(_) => {
            lookup_npm_compat(&client, registry.base_url(), pkg_name).await
        }
    }
}

// ── Version utilities ─────────────────────────────────────────────────────────

/// Pick the highest version in `versions` that satisfies `range_str`.
/// Falls back to `latest` tag, then to the last element, then "latest".
fn select_best_version(versions: &[String], range_str: &str, latest: Option<&str>) -> String {
    let range = SemverRange::parse(range_str).unwrap_or(SemverRange::Any);
    versions
        .iter()
        .filter_map(|v| Semver::parse(v).map(|sv| (sv, v.as_str())))
        .filter(|(sv, _)| range.matches(sv))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, v)| v.to_string())
        .or_else(|| latest.map(|s| s.to_string()))
        .or_else(|| versions.last().cloned())
        .unwrap_or_else(|| "latest".to_string())
}

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

/// Download a tarball with a pre-built client (for parallel installs).
async fn download_tarball_with_client(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut last_err = anyhow::anyhow!("unreachable");

    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            let delay_ms = RETRY_BASE_MS * 2u64.pow(attempt - 1);
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
                continue;
            }
        };

        let status = resp.status();
        if status.is_client_error() {
            anyhow::bail!("Failed to download {}: HTTP {} (not retrying)", url, status);
        }
        if !status.is_success() {
            last_err = anyhow::anyhow!("HTTP {} from {}", status, url);
            continue;
        }

        return Ok(resp.bytes().await?.to_vec());
    }

    Err(last_err.context(format!(
        "All {} download attempts failed for {}",
        MAX_RETRIES + 1,
        url
    )))
}

// ── Zero-Installs ──────────────────────────────────────────────────────────────
// A `.3va/cache/` directory meant to be committed to the repo (like Yarn's
// `.yarn/cache`): every tarball fetched here is written back with a sidecar
// `.sha512` file recording the exact integrity hash it was verified against.
// On a later install — including a fresh clone or CI checkout — the tarball
// bytes are re-verified against that sidecar hash before being trusted, so a
// checked-in cache never becomes an unverified trust root.
//
// ponytail: this only skips the *tarball download*, not the registry
// metadata call that resolves a version range to an exact version — true
// fully-offline install would need the BFS to consult the lockfile before
// ever hitting the network. Add that if a project asks for real offline CI.

fn zero_install_cache_enabled(manifest_val: Option<&serde_json::Value>) -> bool {
    std::env::var("_3VA_ZERO_INSTALL_CACHE").as_deref() == Ok("1")
        || manifest_val.and_then(|val| val["3va"]["zeroInstallCache"].as_bool()) == Some(true)
}

fn zero_install_cache_paths(project_root: &Path, name: &str, version: &str) -> (PathBuf, PathBuf) {
    let dir = project_root.join(".3va").join("cache");
    let base = format!("{}@{}.tgz", store::virtual_entry_name(name), version);
    (dir.join(&base), dir.join(format!("{}.sha512", base)))
}

/// Read + verify a tarball from the committed zero-install cache. Returns the
/// bytes only if they hash-match the sidecar recorded at cache-write time.
fn read_zero_install_cache(project_root: &Path, name: &str, version: &str) -> Option<Vec<u8>> {
    let (tgz_path, hash_path) = zero_install_cache_paths(project_root, name, version);
    let bytes = std::fs::read(&tgz_path).ok()?;
    let integrity = std::fs::read_to_string(&hash_path).ok()?;
    match SignatureVerifier::sha512().verify_tarball(&bytes, integrity.trim()) {
        VerificationStatus::Verified => Some(bytes),
        _ => None,
    }
}

/// Write an already-verified tarball into the committed zero-install cache.
fn write_zero_install_cache(
    project_root: &Path,
    name: &str,
    version: &str,
    bytes: &[u8],
    integrity: &str,
) {
    let (tgz_path, hash_path) = zero_install_cache_paths(project_root, name, version);
    if let Some(dir) = tgz_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&tgz_path, bytes);
    let _ = std::fs::write(&hash_path, integrity);
}

// ── dlx ───────────────────────────────────────────────────────────────────────
// `3va dlx <pkg>`: fetch through the exact same verified-download path as
// `install`, then hand the entry file to the caller so it can be executed
// through the *existing* `3va run` sandbox — not a special case, no new
// execution surface.

/// Resolve, download, verify, and store `spec` (e.g. `cowsay` or
/// `cowsay@1.6.0`), then return the path to a read-execute scratch copy of
/// the package (a real copy, not a store hardlink, so nothing the package
/// does at runtime can touch the shared global store).
pub async fn dlx_fetch(
    project_root: &Path,
    spec: &str,
    allow_net: Option<&[String]>,
) -> anyhow::Result<PathBuf> {
    let (name, requested_ver) = parse_package_spec(spec)?;
    let allowed_host = match allow_net {
        None => anyhow::bail!("Network access denied: --allow-net not specified"),
        Some(hosts) => hosts.first().map(|s| s.as_str()).unwrap_or("").to_string(),
    };
    let registry = Registry::from_allowed_host(&allowed_host);
    let reg_name = registry.display_name().to_string();
    let base_url = registry.base_url().to_string();
    let client = reqwest::Client::builder().gzip(true).build()?;
    let version_to_fetch = requested_ver.as_deref().unwrap_or("latest");

    let (info, _deps) = match &registry {
        Registry::Jsr => lookup_jsr_with_deps(&client, &name, version_to_fetch).await?,
        Registry::Npm | Registry::Yarn | Registry::Custom(_) => {
            lookup_npm_version(&client, &base_url, &name, version_to_fetch).await?
        }
    };
    let ver = select_best_version(&info.versions, version_to_fetch, info.latest.as_deref());
    let Some(meta) = info.version_meta.get(&ver) else {
        anyhow::bail!("No version of '{}' satisfies '{}'", name, version_to_fetch);
    };

    let global_store = store::ContentStore::global();
    if !global_store.is_cached(&reg_name, &name, &ver) {
        let bytes = download_tarball_with_client(&client, &meta.tarball).await?;
        let verifier = SignatureVerifier::sha512();
        if let Some(int_hash) = &meta.integrity
            && matches!(
                verifier.verify_from_registry(&bytes, Some(int_hash)),
                VerificationStatus::Mismatch | VerificationStatus::Failed(_)
            )
        {
            anyhow::bail!("Integrity check failed for {}@{}", name, ver);
        }
        global_store.store_tarball(&bytes, &reg_name, &name, &ver)?;
    }

    let entry = format!("{}@{}", store::virtual_entry_name(&name), ver);
    let run_dir = project_root.join(".3va").join("dlx").join(&entry);
    if !run_dir.join("package.json").exists() {
        let pristine = global_store.package_path(&reg_name, &name, &ver);
        store::copy_dir_recursive(&pristine, &run_dir)?;
    }
    Ok(run_dir)
}

/// Resolve a dlx-fetched package's executable entry: its `bin` field if
/// present (string or the first entry of an object), else `main`, else
/// `index.js`.
pub fn dlx_entry(pkg_dir: &Path) -> anyhow::Result<PathBuf> {
    let content = std::fs::read_to_string(pkg_dir.join("package.json"))?;
    let val: serde_json::Value = serde_json::from_str(&content)?;
    let rel = match &val["bin"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(obj) => {
            let name = val["name"].as_str().unwrap_or("");
            let short_name = name.rsplit('/').next().unwrap_or(name);
            obj.get(short_name)
                .or_else(|| obj.values().next())
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow::anyhow!("Package has no usable 'bin' entry"))?
        }
        _ => val["main"].as_str().unwrap_or("index.js").to_string(),
    };
    Ok(pkg_dir.join(rel))
}

// ── Auto-install before script run ──────────────────────────────────────────
// `3va run` / `test` / `dev` compare a hash of package.json's dependency
// fields against the hash recorded by the last successful install. On a
// mismatch the CLI prompts once before running — never implicit, never
// silent; the human's "y" at that prompt is the only thing that grants the
// auto-install network access.

fn manifest_dep_fields(val: &serde_json::Value) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for key in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
        "overrides",
        "resolutions",
        "configDependencies",
    ] {
        if let Some(v) = val.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    serde_json::Value::Object(out)
}

/// Serialize with sorted object keys so field/key reordering in
/// `package.json` never spuriously changes the hash.
fn sorted_json_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let inner: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("{:?}:{}", k, sorted_json_string(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(arr) => {
            let inner: Vec<String> = arr.iter().map(sorted_json_string).collect();
            format!("[{}]", inner.join(","))
        }
        other => other.to_string(),
    }
}

/// SHA-256 (hex) of every dependency-related field in `package.json`.
/// `None` if there's no parseable `package.json` at all.
pub fn manifest_dep_hash(project_root: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let content = std::fs::read_to_string(project_root.join("package.json")).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let canonical = sorted_json_string(&manifest_dep_fields(&val));
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Some(format!("{:x}", hasher.finalize()))
}

fn install_hash_marker(project_root: &Path) -> PathBuf {
    project_root.join("node_modules").join(".3va-install-hash")
}

/// Record the current dependency hash as "installed". Call after any
/// successful install that touches `package.json`'s dependency fields.
pub fn record_install_hash(project_root: &Path) {
    if let Some(hash) = manifest_dep_hash(project_root) {
        let _ = std::fs::create_dir_all(project_root.join("node_modules"));
        let _ = std::fs::write(install_hash_marker(project_root), hash);
    }
}

/// True if `package.json`'s dependency fields have changed since the last
/// recorded install.
///
/// ponytail: a missing marker with an existing `node_modules/` (e.g. a
/// project installed before this version, or `node_modules` from another
/// tool) is treated as "up to date" rather than forcing a prompt on every
/// run — the marker gets created on the next explicit `3va install` and the
/// check becomes authoritative from then on. Upgrade path if that's ever
/// wrong often enough to matter: hash the installed tree itself instead of
/// trusting the marker's mere presence.
pub fn needs_install(project_root: &Path) -> bool {
    let Some(current) = manifest_dep_hash(project_root) else {
        return false;
    };
    match std::fs::read_to_string(install_hash_marker(project_root)) {
        Ok(recorded) => recorded.trim() != current,
        Err(_) => !project_root.join("node_modules").exists(),
    }
}

// ── Patching dependencies ────────────────────────────────────────────────────
// `3va patch <pkg>` / `3va patch-commit <pkg>`: pure file diffs, applied at
// install time by overwriting/removing files — never a script, never code
// execution. The "diff" is deliberately not unified-diff text: it's the full
// contents of every changed file plus a list of removed paths, stored under
// `patches/{name}@{version}/`. Simpler to generate and apply than parsing or
// emitting patch syntax, and still fully reviewable in a PR.

fn resolve_installed_registry(project_root: &Path, name: &str) -> String {
    if let Ok(lock) = Lockfile::load(&project_root.join("3va-lock.json"))
        && let Some(dep) = lock.dependencies.get(name)
        && let Some(reg) = &dep.registry
    {
        return reg.clone();
    }
    Registry::Npm.display_name().to_string()
}

/// `3va patch <pkg>`: copy the installed package into an editable scratch
/// directory (a real copy, never a store hardlink) and return its path.
pub fn patch_start(project_root: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let node_modules = project_root.join("node_modules");
    let installed_dir = node_modules.join(name);
    if !installed_dir.join("package.json").exists() {
        anyhow::bail!("'{}' is not installed — run 3va install first.", name);
    }
    let version = read_package_version(&installed_dir);
    let registry = resolve_installed_registry(project_root, name);
    let store = store::ContentStore::global();
    let pristine = store.package_path(&registry, name, &version);
    if !pristine.exists() {
        anyhow::bail!("'{}@{}' not found in the global store.", name, version);
    }

    let entry = format!("{}@{}", store::virtual_entry_name(name), version);
    let work_dir = project_root.join(".3va").join("patch-work").join(&entry);
    if work_dir.exists() {
        std::fs::remove_dir_all(&work_dir)?;
    }
    store::copy_dir_recursive(&pristine, &work_dir)?;
    Ok(work_dir)
}

/// `3va patch-commit <pkg>`: diff the edited scratch copy against the
/// pristine store version and write the result under `patches/`.
pub fn patch_commit(project_root: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let node_modules = project_root.join("node_modules");
    let installed_dir = node_modules.join(name);
    let version = read_package_version(&installed_dir);
    let registry = resolve_installed_registry(project_root, name);
    let store = store::ContentStore::global();
    let pristine = store.package_path(&registry, name, &version);

    let entry = format!("{}@{}", store::virtual_entry_name(name), version);
    let work_dir = project_root.join(".3va").join("patch-work").join(&entry);
    if !work_dir.exists() {
        anyhow::bail!(
            "No in-progress patch for '{}' — run '3va patch {}' first.",
            name,
            name
        );
    }

    let patch_dir = project_root.join("patches").join(&entry);
    if patch_dir.exists() {
        std::fs::remove_dir_all(&patch_dir)?;
    }

    let mut changed = 0usize;
    let mut removed_paths = Vec::new();
    diff_dirs(
        &pristine,
        &work_dir,
        Path::new(""),
        &patch_dir,
        &mut changed,
        &mut removed_paths,
    )?;

    if !removed_paths.is_empty() {
        std::fs::create_dir_all(&patch_dir)?;
        std::fs::write(patch_dir.join(".removed"), removed_paths.join("\n"))?;
    }

    std::fs::remove_dir_all(&work_dir)?;

    if changed == 0 && removed_paths.is_empty() {
        anyhow::bail!("No changes detected for '{}' — nothing to commit.", name);
    }
    Ok(patch_dir)
}

/// Diff `pristine` vs `edited`, writing every changed/added file (at its
/// matching relative path) into `patch_dir`, and appending removed paths
/// (present in pristine, missing in edited) to `removed`.
fn diff_dirs(
    pristine: &Path,
    edited: &Path,
    rel: &Path,
    patch_dir: &Path,
    changed: &mut usize,
    removed: &mut Vec<String>,
) -> anyhow::Result<()> {
    use std::collections::HashSet;

    let pristine_dir = pristine.join(rel);
    let edited_dir = edited.join(rel);

    let mut names: HashSet<std::ffi::OsString> = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(&pristine_dir) {
        names.extend(entries.flatten().map(|e| e.file_name()));
    }
    if let Ok(entries) = std::fs::read_dir(&edited_dir) {
        names.extend(entries.flatten().map(|e| e.file_name()));
    }

    for name in names {
        let rel_child = rel.join(&name);
        let p_path = pristine_dir.join(&name);
        let e_path = edited_dir.join(&name);
        let e_exists = e_path.exists();

        if p_path.is_dir() || e_path.is_dir() {
            if e_exists {
                diff_dirs(pristine, edited, &rel_child, patch_dir, changed, removed)?;
            } else {
                collect_all_files(&p_path, &rel_child, removed);
            }
            continue;
        }

        if !e_exists {
            removed.push(rel_child.to_string_lossy().replace('\\', "/"));
            continue;
        }

        let p_bytes = std::fs::read(&p_path).unwrap_or_default();
        let e_bytes = std::fs::read(&e_path)?;
        if p_bytes != e_bytes {
            let dest = patch_dir.join(&rel_child);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &e_bytes)?;
            *changed += 1;
        }
    }
    Ok(())
}

fn collect_all_files(dir: &Path, rel: &Path, removed: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            let rel_child = rel.join(entry.file_name());
            if p.is_dir() {
                collect_all_files(&p, &rel_child, removed);
            } else {
                removed.push(rel_child.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// Apply a committed patch (if one exists) to a freshly-linked package.
/// Always deletes-then-rewrites each changed file, so a hardlink into the
/// shared global store is broken before any byte is written — the store
/// itself is never mutated by a patch.
fn apply_patch_if_present(project_root: &Path, node_modules: &Path, name: &str, version: &str) {
    let entry = format!("{}@{}", store::virtual_entry_name(name), version);
    let patch_dir = project_root.join("patches").join(&entry);
    if !patch_dir.exists() {
        return;
    }

    let link_path = node_modules.join(name);
    let target_dir = std::fs::canonicalize(&link_path).unwrap_or(link_path);

    apply_patch_files(&patch_dir, Path::new(""), &target_dir);

    if let Ok(removed) = std::fs::read_to_string(patch_dir.join(".removed")) {
        for rel in removed.lines().filter(|l| !l.is_empty()) {
            let _ = std::fs::remove_file(target_dir.join(rel));
        }
    }
    println!("  ✓ patch applied: {}@{}", name, version);
}

fn apply_patch_files(patch_dir: &Path, rel: &Path, target_dir: &Path) {
    let dir = patch_dir.join(rel);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        if name == ".removed" {
            continue;
        }
        let rel_child = rel.join(&name);
        if p.is_dir() {
            apply_patch_files(patch_dir, &rel_child, target_dir);
        } else if let Ok(bytes) = std::fs::read(&p) {
            let dest = target_dir.join(&rel_child);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::remove_file(&dest); // break any hardlink to the store
            let _ = std::fs::write(&dest, &bytes);
        }
    }
}

/// Download a tarball with up to `MAX_RETRIES` retries and exponential backoff.
/// Retries on connection/timeout errors and 5xx responses; fails immediately
/// on 4xx (package not found, auth errors, etc.).
const MAX_RETRIES: u32 = 3;
const RETRY_BASE_MS: u64 = 400;

async fn download_tarball(url: &str) -> anyhow::Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .gzip(false) // tarballs are already gzipped at file level — don't double-decompress
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

pub fn remove_package(name: &str) -> anyhow::Result<()> {
    let root = std::env::current_dir()?;
    let pkg_name = name
        .split('@')
        .next()
        .unwrap_or(name)
        .trim_start_matches('@')
        .to_string();
    let pkg_name_full = if name.starts_with('@') {
        // scoped package like @scope/pkg
        let parts: Vec<&str> = name.trim_start_matches('@').splitn(2, '/').collect();
        if parts.len() == 2 {
            format!(
                "@{}",
                parts[0..2].join("/").split('@').next().unwrap_or(name)
            )
        } else {
            pkg_name
        }
    } else {
        pkg_name
    };

    // Remove from node_modules
    let nm_path = root.join("node_modules").join(&pkg_name_full);
    if nm_path.exists() {
        std::fs::remove_dir_all(&nm_path).map_err(|e| {
            anyhow::anyhow!("Failed to remove node_modules/{}: {}", pkg_name_full, e)
        })?;
    } else {
        anyhow::bail!(
            "Package '{}' is not installed in node_modules.",
            pkg_name_full
        );
    }

    // Remove from package.json
    let pkg_json_path = root.join("package.json");
    if pkg_json_path.exists() {
        let content = std::fs::read_to_string(&pkg_json_path)?;
        if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
            let mut modified = false;
            for field in &[
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ] {
                if let Some(obj) = json[field].as_object_mut()
                    && obj.remove(&pkg_name_full).is_some()
                {
                    modified = true;
                }
            }
            if modified {
                let updated = serde_json::to_string_pretty(&json)?;
                std::fs::write(&pkg_json_path, updated + "\n")?;
            }
        }
    }

    // Remove from lockfile
    let lock_path = root.join("3va-lock.json");
    if lock_path.exists()
        && let Ok(content) = std::fs::read_to_string(&lock_path)
        && let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content)
    {
        if let Some(deps) = json["dependencies"].as_object_mut() {
            let keys_to_remove: Vec<String> = deps
                .keys()
                .filter(|k| k.starts_with(&pkg_name_full))
                .cloned()
                .collect();
            for k in keys_to_remove {
                deps.remove(&k);
            }
        }
        let updated = serde_json::to_string_pretty(&json)?;
        let _ = std::fs::write(&lock_path, updated + "\n");
    }

    println!("  ✓ Removed package '{}'", pkg_name_full);
    Ok(())
}

/// Install all `dependencies` listed in `project_root/package.json`.
/// This is what `3va install` (no args, no workspace) does.
///
/// Automatically detects existing lockfiles (`package-lock.json`, `yarn.lock`,
/// `pnpm-lock.yaml`) and reads `.npmrc` for private registry configuration.
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

    // Auto-migrate existing lockfile
    if let Ok(true) = migrate_lockfile(project_root) {
        // Already migrated above
    }

    // Detect .npmrc for private registry support
    let npmrc = npmrc::discover_npmrc(Some(project_root));
    if let Some(reg) = &npmrc.registry
        && reg != "https://registry.npmjs.org"
    {
        tracing::info!("Using custom registry from .npmrc: {}", reg);
    }
    if !npmrc.auth_tokens.is_empty() {
        tracing::info!("Found {} auth token(s) in .npmrc", npmrc.auth_tokens.len());
    }
    if !npmrc.scoped_registries.is_empty() {
        tracing::info!(
            "Found {} scoped registry(ies) in .npmrc",
            npmrc.scoped_registries.len()
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

    record_install_hash(project_root);
    println!();
    println!("✓ All dependencies installed.");
    Ok(())
}

/// Install every entry in `package.json`'s `configDependencies` map into
/// `.3va/config-deps/{name}@{version}/` — for build tooling to read, never
/// linked into `node_modules`, never on any module-resolution path, so
/// there is no execution surface here at all. Not transitive: each entry
/// is fetched and stored exactly as requested, same as a normal install,
/// just parked in a side store instead of `node_modules`.
pub async fn install_config_deps(
    project_root: &Path,
    allow_net: Option<&[String]>,
) -> anyhow::Result<()> {
    let pkg_json = project_root.join("package.json");
    let content = std::fs::read_to_string(&pkg_json).unwrap_or_default();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
    let Some(deps) = val["configDependencies"].as_object() else {
        println!("Nothing to install — package.json has no configDependencies.");
        return Ok(());
    };
    if deps.is_empty() {
        println!("Nothing to install — package.json has no configDependencies.");
        return Ok(());
    }

    let allowed_host = match allow_net {
        None => anyhow::bail!("Network access denied: --allow-net not specified"),
        Some(hosts) => hosts.first().map(|s| s.as_str()).unwrap_or("").to_string(),
    };
    let registry = Registry::from_allowed_host(&allowed_host);
    let reg_name = registry.display_name().to_string();
    let base_url = registry.base_url().to_string();
    let client = reqwest::Client::builder().gzip(true).build()?;
    let global_store = ContentStore::global();
    let verifier = SignatureVerifier::sha512();
    let config_deps_root = project_root.join(".3va").join("config-deps");

    for (name, range) in deps {
        let Some(range) = range.as_str() else {
            continue;
        };
        let (info, _deps) = match &registry {
            Registry::Jsr => lookup_jsr_with_deps(&client, name, range).await?,
            Registry::Npm | Registry::Yarn | Registry::Custom(_) => {
                lookup_npm_version(&client, &base_url, name, range).await?
            }
        };
        let ver = select_best_version(&info.versions, range, info.latest.as_deref());
        let Some(meta) = info.version_meta.get(&ver) else {
            anyhow::bail!("No version of '{}' satisfies '{}'", name, range);
        };

        if !global_store.is_cached(&reg_name, name, &ver) {
            let bytes = download_tarball_with_client(&client, &meta.tarball).await?;
            if let Some(int_hash) = &meta.integrity
                && matches!(
                    verifier.verify_from_registry(&bytes, Some(int_hash)),
                    VerificationStatus::Mismatch | VerificationStatus::Failed(_)
                )
            {
                anyhow::bail!("Integrity check failed for {}@{}", name, ver);
            }
            global_store.store_tarball(&bytes, &reg_name, name, &ver)?;
        }

        global_store.link_config_dep(&reg_name, name, &ver, &config_deps_root)?;
        println!("  ✓ {}@{} (config-only)", name, ver);
    }

    println!();
    println!("✓ All config dependencies installed.");
    Ok(())
}

/// BFS install: installs `root_spec` and all of its transitive dependencies
/// into `project_root/node_modules/`.
///
/// Phase 1: resolve the full dependency graph in parallel (concurrent metadata
///          fetches, BFS wave-by-wave so each wave is fully parallel).
/// Phase 2: download + extract all tarballs in parallel with JoinSet.
///
/// `update_manifest`: when true the package.json and lockfile in
/// `project_root` are updated.  Pass false for transitive deps.
type PackageFetchResult = anyhow::Result<(String, String, RegistryInfo, Vec<(String, String)>)>;

async fn install_with_transitive(
    root_spec: &str,
    force: bool,
    allow_net: Option<&[String]>,
    project_root: &Path,
    update_manifest: bool,
) -> anyhow::Result<()> {
    use std::collections::{HashMap as Map, HashSet};
    use tokio::task::JoinSet;

    // ── Determine registry ────────────────────────────────────────────────────
    // allow_net=None → permission denied; allow_net=Some([]) → allow all (--allow-net=)
    let allowed_host = match allow_net {
        None => {
            let (pkg_name, _) = parse_package_spec(root_spec)?;
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
            anyhow::bail!("Network access denied: --allow-net not specified");
        }
        Some(hosts) => hosts.first().map(|s| s.as_str()).unwrap_or("").to_string(),
    };
    let registry = Registry::from_allowed_host(&allowed_host);
    let base_url = registry.base_url().to_string();

    // Two clients:
    // - meta_client: for JSON metadata requests (gzip on → server sends compressed JSON)
    // - dl_client: for tarball downloads (gzip off → tarballs are already .tgz, double-decompression breaks them)
    let meta_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .pool_max_idle_per_host(16)
        .gzip(true)
        .build()?;
    let dl_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .pool_max_idle_per_host(8)
        .gzip(false)
        .build()?;
    // Alias for metadata phase
    let client = meta_client.clone();

    // ── Phase 1: parallel BFS metadata resolution ─────────────────────────────
    // resolved: name → (version, tarball_url, integrity)
    let mut resolved: Map<String, (String, String, Option<String>)> = Map::new();
    let mut visited: HashSet<String> = HashSet::new();

    let manifest_val: Option<serde_json::Value> =
        std::fs::read_to_string(project_root.join("package.json"))
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok());

    // `overrides` (npm) / `resolutions` (Yarn): forces a package's resolved
    // version range regardless of what any dependent requested. Same
    // precedence as npm — an override always wins for the whole install,
    // since this resolver already keeps a single flat version per name.
    let overrides: Map<String, String> = manifest_val
        .as_ref()
        .and_then(|val| {
            ["overrides", "resolutions"]
                .into_iter()
                .find_map(|key| val[key].as_object().cloned())
        })
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // `--node-linker=hoisted` (env var set by the CLI flag) or the
    // project's own `"3va": {"nodeLinker": "hoisted"}` manifest key selects
    // a flat top-level node_modules instead of the isolated CAS/symlink
    // layout. Isolated stays the default either way.
    let hoisted = std::env::var("_3VA_NODE_LINKER").as_deref() == Ok("hoisted")
        || manifest_val
            .as_ref()
            .and_then(|val| val["3va"]["nodeLinker"].as_str())
            == Some("hoisted");

    // Start with the root package
    let (root_name, root_requested_ver) = parse_package_spec(root_spec)?;
    let mut current_wave: Vec<(String, Option<String>)> =
        vec![(root_name.clone(), root_requested_ver)];

    println!();
    println!("  Resolving dependency graph...");

    while !current_wave.is_empty() {
        // Deduplicate wave and skip already-resolved packages
        let wave: Vec<(String, Option<String>)> = current_wave
            .drain(..)
            .filter(|(name, _)| !visited.contains(name.as_str()))
            .map(|(name, ver)| {
                let ver = apply_override(&overrides, &name, ver);
                (name, ver)
            })
            .collect();

        if wave.is_empty() {
            break;
        }
        for (name, _) in &wave {
            visited.insert(name.clone());
        }

        // Check which packages are already installed at the right version
        let needs_fetch: Vec<(String, Option<String>)> = wave
            .iter()
            .filter(|(name, _)| {
                if force {
                    return true;
                }
                let dest = project_root.join("node_modules").join(name);
                !dest.join("package.json").exists()
            })
            .cloned()
            .collect();

        if needs_fetch.is_empty() {
            // All in this wave are already installed; still need to read their deps
            for (name, _) in &wave {
                let dest = project_root.join("node_modules").join(name);
                if let Ok(content) = std::fs::read_to_string(dest.join("package.json"))
                    && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
                {
                    if let Some(ver) = val["version"].as_str() {
                        resolved.entry(name.clone()).or_insert_with(|| {
                            let tarball = format!("{}/{}/-/{}-{}.tgz", base_url, name, name, ver);
                            (ver.to_string(), tarball, None)
                        });
                    }
                    for (dep_name, dep_range) in collect_dep_specs(&val) {
                        if !visited.contains(dep_name.as_str()) {
                            current_wave.push((dep_name, Some(dep_range)));
                        }
                    }
                }
            }
            continue;
        }

        // Fetch metadata for this wave concurrently
        let mut set: JoinSet<PackageFetchResult> = JoinSet::new();
        for (pkg_name, requested_ver) in needs_fetch {
            let client = client.clone();
            let base = base_url.clone();
            let registry = registry.clone();
            set.spawn(async move {
                let version_to_fetch = requested_ver.as_deref().unwrap_or("latest");
                let (info, deps) = match &registry {
                    Registry::Jsr => {
                        lookup_jsr_with_deps(&client, &pkg_name, version_to_fetch).await?
                    }
                    Registry::Npm | Registry::Yarn | Registry::Custom(_) => {
                        lookup_npm_version(&client, &base, &pkg_name, version_to_fetch).await?
                    }
                };
                Ok((pkg_name, version_to_fetch.to_string(), info, deps))
            });
        }

        // name → version_range: dedup within wave while preserving the requested range.
        let mut next_wave_deps: HashMap<String, String> = HashMap::new();
        while let Some(result) = set.join_next().await {
            match result {
                Ok(Ok((pkg_name, version_to_fetch, info, dep_specs))) => {
                    // Pick the highest version satisfying the requested range.
                    let ver = select_best_version(
                        &info.versions,
                        &version_to_fetch,
                        info.latest.as_deref(),
                    );
                    if let Some(meta) = info.version_meta.get(&ver) {
                        resolved.insert(
                            pkg_name.clone(),
                            (ver.clone(), meta.tarball.clone(), meta.integrity.clone()),
                        );
                    }
                    // Propagate transitive dep specs (name + version range).
                    for (dep_name, dep_range) in dep_specs {
                        if !visited.contains(dep_name.as_str()) {
                            next_wave_deps.entry(dep_name).or_insert(dep_range);
                        }
                    }
                    // Also check disk in case the package was already extracted
                    // (covers the fallback full-packument path where dep_specs is empty).
                    let dest = project_root.join("node_modules").join(&pkg_name);
                    if dest.join("package.json").exists()
                        && let Ok(content) = std::fs::read_to_string(dest.join("package.json"))
                        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
                    {
                        for (dep_name, dep_range) in collect_dep_specs(&val) {
                            if !visited.contains(dep_name.as_str()) {
                                next_wave_deps.entry(dep_name).or_insert(dep_range);
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!("Failed to resolve package: {}", e);
                }
                Err(e) => {
                    tracing::warn!("Task panic during resolution: {}", e);
                }
            }
        }
        for (dep, range) in next_wave_deps {
            current_wave.push((dep, Some(range)));
        }
    }

    // ── Phase 2: parallel download + extract ──────────────────────────────────
    let to_install: Vec<(String, String, String, Option<String>)> = resolved
        .iter()
        .filter(|(name, (ver, _, _))| {
            if force && name.as_str() == root_name.as_str() {
                return true;
            }
            let dest = project_root.join("node_modules").join(name.as_str());
            let pkg_json = dest.join("package.json");
            if !pkg_json.exists() {
                return true;
            }
            // Check if installed version matches
            let installed_ver = std::fs::read_to_string(&pkg_json)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| v["version"].as_str().map(|s| s.to_string()));
            installed_ver.as_deref() != Some(ver.as_str())
        })
        .map(|(name, (ver, tarball, integrity))| {
            (
                name.clone(),
                ver.clone(),
                tarball.clone(),
                integrity.clone(),
            )
        })
        .collect();

    if to_install.is_empty() {
        println!("  ✓ All dependencies already installed.");
    } else {
        println!(
            "  Downloading {} package(s) in parallel...",
            to_install.len()
        );

        let global_store = store::ContentStore::global();
        let reg_name = registry.display_name().to_string();
        let cache_dir = project_root.join(".3va-cache");
        std::fs::create_dir_all(&cache_dir)?;
        let node_modules = project_root.join("node_modules");
        std::fs::create_dir_all(&node_modules)?;
        let zero_install_on = zero_install_cache_enabled(manifest_val.as_ref());
        let project_root_owned = project_root.to_path_buf();

        #[allow(clippy::type_complexity)]
        let mut dl_set: JoinSet<(
            String,
            String,
            anyhow::Result<(Vec<u8>, Option<String>)>,
        )> = JoinSet::new();

        for (pkg_name, ver, tarball_url, integrity) in to_install.iter().cloned() {
            let client = dl_client.clone();
            let safe_pkg = pkg_name.replace('/', "-").trim_matches('-').to_string();
            let cached_path = cache_dir.join(format!("{}-{}.tgz", safe_pkg, ver));
            let gs = global_store.clone();
            let rn = reg_name.clone();
            let root = project_root_owned.clone();

            dl_set.spawn(async move {
                let result = async {
                    // Zero-install cache: committed, hash-verified — checked
                    // before any network call, including the global store.
                    if zero_install_on
                        && let Some(bytes) = read_zero_install_cache(&root, &pkg_name, &ver)
                    {
                        return Ok((bytes, integrity));
                    }
                    // Check global store first (zero network)
                    if gs.is_cached(&rn, &pkg_name, &ver) {
                        return Ok((Vec::new(), integrity));
                    }
                    // Check per-project tarball cache
                    if cached_path.exists() {
                        let bytes = std::fs::read(&cached_path)?;
                        return Ok((bytes, integrity));
                    }
                    // Download
                    let bytes = download_tarball_with_client(&client, &tarball_url)
                        .await
                        .map_err(|e| anyhow::anyhow!("{}@{} — {}", pkg_name, ver, e))?;
                    let _ = std::fs::write(&cached_path, &bytes);
                    Ok((bytes, integrity))
                }
                .await;
                (pkg_name, ver, result)
            });
        }

        let verifier = SignatureVerifier::sha512();
        let mut errors: Vec<String> = Vec::new();

        while let Some(result) = dl_set.join_next().await {
            match result {
                Ok((pkg_name, ver, Ok((bytes, integrity)))) => {
                    // If bytes is empty → was in global store already
                    let final_bytes = if bytes.is_empty() {
                        Vec::new() // store.link_to_virtual_store handles it
                    } else {
                        // Verify integrity
                        if let Some(ref int_hash) = integrity {
                            match verifier.verify_from_registry(&bytes, Some(int_hash)) {
                                VerificationStatus::Mismatch | VerificationStatus::Failed(_) => {
                                    errors.push(format!(
                                        "Integrity check failed for {}@{}",
                                        pkg_name, ver
                                    ));
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        // Store globally
                        let _ = global_store.store_tarball(&bytes, &reg_name, &pkg_name, &ver);
                        if zero_install_on && let Some(int_hash) = &integrity {
                            write_zero_install_cache(
                                project_root,
                                &pkg_name,
                                &ver,
                                &bytes,
                                int_hash,
                            );
                        }
                        bytes
                    };

                    // Link into node_modules — hoisted (flat copy/hardlink) or
                    // isolated (CAS + symlink, the default).
                    if hoisted {
                        if let Err(e) =
                            global_store.link_hoisted(&reg_name, &pkg_name, &ver, &node_modules)
                        {
                            let dest = node_modules.join(&pkg_name);
                            if !final_bytes.is_empty() {
                                if let Err(e2) = extract_tarball(&final_bytes, &dest) {
                                    errors.push(format!(
                                        "Extract failed for {}@{}: {}",
                                        pkg_name, ver, e2
                                    ));
                                    continue;
                                }
                            } else {
                                errors.push(format!(
                                    "Store link failed for {}@{}: {}",
                                    pkg_name, ver, e
                                ));
                                continue;
                            }
                        }
                    } else {
                        let virtual_path = match global_store.link_to_virtual_store(
                            &reg_name,
                            &pkg_name,
                            &ver,
                            &node_modules,
                        ) {
                            Ok(p) => p,
                            Err(e) => {
                                // If store link fails (bytes not in store), extract directly
                                let dest = node_modules.join(&pkg_name);
                                if !final_bytes.is_empty() {
                                    if let Err(e2) = extract_tarball(&final_bytes, &dest) {
                                        errors.push(format!(
                                            "Extract failed for {}@{}: {}",
                                            pkg_name, ver, e2
                                        ));
                                    }
                                } else {
                                    errors.push(format!(
                                        "Store link failed for {}@{}: {}",
                                        pkg_name, ver, e
                                    ));
                                }
                                println!("  ✓ {}@{}", pkg_name, ver);
                                continue;
                            }
                        };

                        if let Err(e) =
                            create_virtual_symlink(&pkg_name, &ver, &node_modules, &virtual_path)
                        {
                            errors.push(format!("Symlink failed for {}@{}: {}", pkg_name, ver, e));
                            continue;
                        }
                    }

                    apply_patch_if_present(project_root, &node_modules, &pkg_name, &ver);

                    // ── Lifecycle scripts (postinstall / install) ─────────────────
                    // Security: blocked by default; opt-in via 3VA_ALLOW_SCRIPTS=1
                    // or --allow-scripts flag (future).
                    if std::env::var("3VA_ALLOW_SCRIPTS").as_deref() == Ok("1") {
                        let pkg_dir = node_modules.join(&pkg_name);
                        let scripts_path = pkg_dir.join("package.json");
                        if let Ok(scripts_content) = std::fs::read_to_string(&scripts_path)
                            && let Ok(scripts_val) =
                                serde_json::from_str::<serde_json::Value>(&scripts_content)
                        {
                            let lifecycle_scripts = ["preinstall", "install", "postinstall"];
                            for lifecycle in &lifecycle_scripts {
                                if let Some(script) = scripts_val["scripts"]
                                    .get(*lifecycle)
                                    .and_then(|v| v.as_str())
                                {
                                    println!(
                                        "  ⚙  Running {lifecycle} script for {pkg_name}@{ver}"
                                    );
                                    let shell = if cfg!(windows) { "cmd" } else { "sh" };
                                    let flag = if cfg!(windows) { "/C" } else { "-c" };
                                    match std::process::Command::new(shell)
                                        .args([flag, script])
                                        .current_dir(&pkg_dir)
                                        .output()
                                    {
                                        Ok(output) => {
                                            if !output.status.success() {
                                                let stderr =
                                                    String::from_utf8_lossy(&output.stderr);
                                                eprintln!(
                                                    "  ⚠  {lifecycle} script failed for \
                                                         {pkg_name}@{ver}: {stderr}"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            eprintln!(
                                                "  ⚠  Could not run {lifecycle} for \
                                                     {pkg_name}@{ver}: {e}"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    } else if let Ok(scripts_content) =
                        std::fs::read_to_string(node_modules.join(&pkg_name).join("package.json"))
                        && let Ok(scripts_val) =
                            serde_json::from_str::<serde_json::Value>(&scripts_content)
                    {
                        let has_lifecycle = ["preinstall", "install", "postinstall"]
                            .iter()
                            .any(|s| scripts_val["scripts"].get(*s).is_some());
                        if has_lifecycle {
                            println!(
                                "  ⚠  {pkg_name}@{ver} has lifecycle scripts. \
                                 Set 3VA_ALLOW_SCRIPTS=1 to enable."
                            );
                        }
                    }
                    println!("  ✓ {}@{}", pkg_name, ver);
                }
                Ok((pkg_name, ver, Err(e))) => {
                    errors.push(format!("Download failed {}@{}: {}", pkg_name, ver, e));
                }
                Err(e) => {
                    errors.push(format!("Task panic: {}", e));
                }
            }
        }

        if !errors.is_empty() {
            for e in &errors {
                eprintln!("  ✗ {}", e);
            }
            anyhow::bail!("{} package(s) failed to install", errors.len());
        }

        println!();
        println!("  ✓ {} package(s) installed.", to_install.len());
    }

    // ── Update package.json + lockfile (root package only) ────────────────────
    if !update_manifest {
        return Ok(());
    }

    // Delegate manifest update to a lightweight helper using the same logic as before
    install_package_impl(root_spec, force, allow_net, project_root, true)
        .await
        .or_else(|_| {
            // If impl fails (package already there), update manifest manually
            update_manifest_only(root_spec, project_root)
        })
}

/// Update only the package.json and lockfile without re-downloading anything.
fn update_manifest_only(root_spec: &str, project_root: &Path) -> anyhow::Result<()> {
    let (pkg_name, _) = parse_package_spec(root_spec)?;
    let node_modules = project_root.join("node_modules");
    let pkg_json_path = project_root.join("package.json");

    // Read installed version
    let installed_ver = std::fs::read_to_string(node_modules.join(&pkg_name).join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["version"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "*".to_string());

    // Update package.json
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
                    dep_map.insert(k.clone(), ver.to_string());
                }
            }
        }
        (pname, pver, dep_map)
    } else {
        ("project".to_string(), "0.0.0".to_string(), HashMap::new())
    };

    deps.insert(pkg_name.clone(), installed_ver.clone());

    let deps_json: serde_json::Value = deps
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into();

    let manifest = serde_json::json!({
        "name": project_name,
        "version": project_version,
        "dependencies": deps_json
    });
    std::fs::write(
        &pkg_json_path,
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    record_install_hash(project_root);

    Ok(())
}

fn collect_installed(node_modules: &Path) -> anyhow::Result<Vec<(String, String, Option<String>)>> {
    let lockfile_path = node_modules
        .parent()
        .unwrap_or(Path::new("."))
        .join("3va-lock.json");
    if lockfile_path.exists() {
        let lock = Lockfile::load(&lockfile_path)?;
        return Ok(lock
            .dependencies
            .iter()
            .map(|(name, dep)| (name.clone(), dep.version.clone(), dep.registry.clone()))
            .collect());
    }
    let mut pkgs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(node_modules) {
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
                            let pkg_name =
                                format!("{}/{}", name, sub_entry.file_name().to_string_lossy());
                            let version = read_package_version(&sub_entry.path());
                            pkgs.push((pkg_name, version, None));
                        }
                    }
                } else {
                    let version = read_package_version(&path);
                    pkgs.push((name, version, None));
                }
            }
        }
    }
    Ok(pkgs)
}

fn read_package_version(pkg_dir: &Path) -> String {
    let pkg_json = pkg_dir.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_json)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
        && let Some(v) = val["version"].as_str()
    {
        v.to_string()
    } else {
        "unknown".to_string()
    }
}

/// One row of `3va licenses`: package name, resolved version, license
/// identifier (or "UNKNOWN" if the package declares none).
pub struct PackageLicense {
    pub name: String,
    pub version: String,
    pub license: String,
}

/// List the license of every installed package, read straight from each
/// package's own `package.json` in `node_modules/` — no new fetch, no
/// license metadata cached anywhere else.
pub fn list_licenses() -> anyhow::Result<Vec<PackageLicense>> {
    let node_modules = PathBuf::from("node_modules");
    if !node_modules.exists() {
        anyhow::bail!("node_modules/ not found. Run '3va install' first.");
    }

    let installed = collect_installed(&node_modules)?;
    Ok(installed
        .into_iter()
        .map(|(name, version, _registry)| {
            let license = read_package_license(&node_modules.join(&name));
            PackageLicense {
                name,
                version,
                license,
            }
        })
        .collect())
}

fn read_package_license(pkg_dir: &Path) -> String {
    let pkg_json = pkg_dir.join("package.json");
    let Ok(content) = std::fs::read_to_string(&pkg_json) else {
        return "UNKNOWN".to_string();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
        return "UNKNOWN".to_string();
    };
    // Modern packages use a "license" string (SPDX id). Older packages used
    // a "licenses" array of {type, url} objects — join those if present.
    if let Some(s) = val["license"].as_str() {
        return s.to_string();
    }
    if let Some(obj) = val["license"].as_object()
        && let Some(t) = obj["type"].as_str()
    {
        return t.to_string();
    }
    if let Some(arr) = val["licenses"].as_array() {
        let types: Vec<&str> = arr.iter().filter_map(|l| l["type"].as_str()).collect();
        if !types.is_empty() {
            return types.join(" OR ");
        }
    }
    "UNKNOWN".to_string()
}

/// Build a CycloneDX 1.5 SBOM (as JSON) from the resolved lockfile. Pure
/// data transform — no network, no re-resolution.
pub fn generate_sbom(lockfile: &Lockfile) -> serde_json::Value {
    let components: Vec<serde_json::Value> = lockfile
        .dependencies
        .iter()
        .map(|(name, dep)| {
            let purl = format!("pkg:npm/{}@{}", name, dep.version);
            let mut component = serde_json::json!({
                "type": "library",
                "name": name,
                "version": dep.version,
                "purl": purl,
            });
            if let Some(integrity) = &dep.integrity
                && let Some((alg, hash)) = integrity.split_once('-')
            {
                let alg = match alg {
                    "sha512" => "SHA-512",
                    "sha256" => "SHA-256",
                    "sha1" => "SHA-1",
                    other => other,
                };
                component["hashes"] = serde_json::json!([{"alg": alg, "content": hash}]);
            }
            component
        })
        .collect();

    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "name": lockfile.name,
                "version": lockfile.version,
            }
        },
        "components": components,
    })
}

pub fn audit_packages() -> anyhow::Result<bool> {
    let node_modules = PathBuf::from("node_modules");

    if !node_modules.exists() {
        eprintln!(
            "✗ node_modules/ not found. Run '3va install <package> --allow-net=<host>' first."
        );
        anyhow::bail!("node_modules not found");
    }

    let installed = collect_installed(&node_modules)?;

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
    println!("  Packages scanned  : {}", installed.len());
    println!("  Heuristic warnings: {}", total_threats);
    println!("  Packages flagged  : {}", packages_with_issues);

    if total_threats == 0 {
        println!();
        println!("✓ Heuristic scan complete. Nothing suspicious found.");
        Ok(true)
    } else {
        println!();
        if any_critical {
            eprintln!(
                "✗ Heuristic scan failed: critical-severity pattern detected. Review the flagged package(s) above — these are pattern matches, not confirmed malware."
            );
        } else {
            eprintln!(
                "! Heuristic scan found lower-severity patterns worth a look; none are confirmed malicious."
            );
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

    let installed = collect_installed(&node_modules)?;

    let scanner = MalwareScanner::new();
    let mut any_critical = false;
    for (pkg_name, _version, _registry) in &installed {
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

/// Create the top-level symlink `node_modules/{name}` → `.3va/{entry}@{ver}/node_modules/{name}`.
///
/// The symlink uses a relative target so the `node_modules/` directory can be
/// moved without breaking links.  On non-Unix platforms we fall back to a
/// direct hard-link copy because creating directory symlinks requires elevated
/// privileges on Windows.
#[allow(unused_variables)] // `version` used only in non-Windows cfg blocks
fn create_virtual_symlink(
    name: &str,
    version: &str,
    node_modules: &Path,
    _virtual_pkg_path: &Path,
) -> anyhow::Result<()> {
    let link_path = if name.contains('/') {
        let scope_dir = node_modules.join(name.split('/').next().unwrap_or(name));
        std::fs::create_dir_all(&scope_dir)?;
        node_modules.join(name)
    } else {
        node_modules.join(name)
    };

    // Remove stale link or directory before (re)creating.
    if link_path.is_symlink() {
        std::fs::remove_file(&link_path)?;
    } else if link_path.exists() {
        std::fs::remove_dir_all(&link_path)?;
    }

    #[cfg(unix)]
    {
        let entry = format!("{}@{}", store::virtual_entry_name(name), version);
        // Relative path from the symlink's containing directory to the virtual-store pkg.
        // Non-scoped: link at node_modules/pkg       → .3va/pkg@ver/node_modules/pkg
        // Scoped:     link at node_modules/@s/pkg    → ../.3va/@s+pkg@ver/node_modules/@s/pkg
        let rel_target = if name.contains('/') {
            format!("../.3va/{}/node_modules/{}", entry, name)
        } else {
            format!(".3va/{}/node_modules/{}", entry, name)
        };
        std::os::unix::fs::symlink(&rel_target, &link_path).map_err(|e| {
            anyhow::anyhow!(
                "Cannot create symlink {} → {}: {}",
                link_path.display(),
                rel_target,
                e
            )
        })?;
    }

    #[cfg(not(unix))]
    {
        // Windows fallback: hard-link directly (no virtual store indirection).
        store::link_or_copy_dir(_virtual_pkg_path, &link_path)?;
    }

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

        // Phase 2: Hard-link into per-project virtual store, then symlink into node_modules/.
        //
        // Layout:
        //   node_modules/.3va/pkg@ver/node_modules/pkg/  ← hard-links from ~/.3va/store
        //   node_modules/pkg                              ← symlink → .3va/pkg@ver/node_modules/pkg
        //
        // This mirrors pnpm's .pnpm topology: top-level node_modules/ contains only
        // symlinks; actual bytes are shared via the global content-addressable store.
        println!(
            "  Linking {}@{} → node_modules/.3va/",
            pkg_name, resolved_version
        );
        std::fs::create_dir_all(&node_modules)?;
        let virtual_path = global_store.link_to_virtual_store(
            reg_name,
            &pkg_name,
            &resolved_version,
            &node_modules,
        )?;
        create_virtual_symlink(&pkg_name, &resolved_version, &node_modules, &virtual_path)?;
        println!(
            "  ✓ node_modules/{} → .3va/{}@{}",
            pkg_name,
            store::virtual_entry_name(&pkg_name),
            resolved_version
        );
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
    let mut lockfile = pm.install(&deps, &project_name, &project_version).await?;

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
    println!("Global store  {}", stats.store_path.display());
    println!("  Packages cached : {}", stats.total_packages);
    println!("  Disk used       : {}", stats.human_size());

    // Virtual store for the current project (node_modules/.3va/).
    if let Ok(cwd) = std::env::current_dir() {
        let virtual_root = cwd.join("node_modules").join(".3va");
        if virtual_root.exists() {
            let mut entries: Vec<String> = std::fs::read_dir(&virtual_root)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            entries.sort();

            println!();
            println!("Virtual store   node_modules/.3va/");
            println!("  Packages linked : {}", entries.len());
            for name in &entries {
                println!("  • {}", name);
            }
            println!();
            println!("  node_modules/<pkg>  →  .3va/<pkg>@<ver>/node_modules/<pkg>");
            println!("  Files are hard-linked from the global store — zero duplication.");
        }
    }

    if stats.total_packages > 0 {
        println!();
        println!("  Every project sharing a package version reads the same bytes from disk.");
    }
    println!();
}

/// `overrides`/`resolutions` always win over whatever version range a
/// dependent requested — same precedence npm uses.
fn apply_override(
    overrides: &std::collections::HashMap<String, String>,
    name: &str,
    requested: Option<String>,
) -> Option<String> {
    overrides.get(name).cloned().or(requested)
}

#[cfg(test)]
mod auto_install_tests {
    use super::{manifest_dep_hash, needs_install, record_install_hash};

    fn write_pkg(dir: &std::path::Path, body: &str) {
        std::fs::write(dir.join("package.json"), body).unwrap();
    }

    #[test]
    fn hash_ignores_key_order() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write_pkg(
            a.path(),
            r#"{"dependencies":{"a":"1.0.0","b":"2.0.0"},"devDependencies":{}}"#,
        );
        write_pkg(
            b.path(),
            r#"{"devDependencies":{},"dependencies":{"b":"2.0.0","a":"1.0.0"}}"#,
        );
        assert_eq!(manifest_dep_hash(a.path()), manifest_dep_hash(b.path()));
    }

    #[test]
    fn hash_ignores_unrelated_fields() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write_pkg(
            a.path(),
            r#"{"name":"x","dependencies":{"lodash":"^4.0.0"}}"#,
        );
        write_pkg(
            b.path(),
            r#"{"name":"y","version":"9.9.9","dependencies":{"lodash":"^4.0.0"}}"#,
        );
        assert_eq!(manifest_dep_hash(a.path()), manifest_dep_hash(b.path()));
    }

    #[test]
    fn hash_changes_when_deps_change() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(dir.path(), r#"{"dependencies":{"lodash":"^4.0.0"}}"#);
        let h1 = manifest_dep_hash(dir.path());
        write_pkg(dir.path(), r#"{"dependencies":{"lodash":"^4.1.0"}}"#);
        let h2 = manifest_dep_hash(dir.path());
        assert_ne!(h1, h2);
    }

    #[test]
    fn no_package_json_needs_no_install() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!needs_install(dir.path()));
    }

    #[test]
    fn missing_node_modules_needs_install() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(dir.path(), r#"{"dependencies":{"lodash":"^4.0.0"}}"#);
        assert!(needs_install(dir.path()));
    }

    #[test]
    fn recorded_hash_matches_current_manifest() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(dir.path(), r#"{"dependencies":{"lodash":"^4.0.0"}}"#);
        record_install_hash(dir.path());
        assert!(!needs_install(dir.path()));
    }

    #[test]
    fn changed_deps_after_install_needs_reinstall() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(dir.path(), r#"{"dependencies":{"lodash":"^4.0.0"}}"#);
        record_install_hash(dir.path());
        write_pkg(dir.path(), r#"{"dependencies":{"lodash":"^4.1.0"}}"#);
        assert!(needs_install(dir.path()));
    }

    #[test]
    fn stale_node_modules_without_marker_is_not_nagged() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(dir.path(), r#"{"dependencies":{"lodash":"^4.0.0"}}"#);
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        assert!(!needs_install(dir.path()));
    }
}

#[cfg(test)]
mod dlx_entry_tests {
    use super::dlx_entry;

    fn write_pkg(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), body).unwrap();
        dir
    }

    #[test]
    fn resolves_string_bin() {
        let dir = write_pkg(r#"{"name":"cowsay","bin":"./cli.js"}"#);
        assert_eq!(dlx_entry(dir.path()).unwrap(), dir.path().join("./cli.js"));
    }

    #[test]
    fn resolves_object_bin_matching_package_name() {
        let dir = write_pkg(r#"{"name":"cowsay","bin":{"cowsay":"./bin/cowsay.js"}}"#);
        assert_eq!(
            dlx_entry(dir.path()).unwrap(),
            dir.path().join("./bin/cowsay.js")
        );
    }

    #[test]
    fn falls_back_to_main_without_bin() {
        let dir = write_pkg(r#"{"name":"lib-only","main":"./index.js"}"#);
        assert_eq!(
            dlx_entry(dir.path()).unwrap(),
            dir.path().join("./index.js")
        );
    }

    #[test]
    fn falls_back_to_index_js_without_bin_or_main() {
        let dir = write_pkg(r#"{"name":"bare"}"#);
        assert_eq!(dlx_entry(dir.path()).unwrap(), dir.path().join("index.js"));
    }
}

#[cfg(test)]
mod patch_tests {
    use super::{apply_patch_if_present, diff_dirs};
    use std::path::Path;

    #[test]
    fn diff_captures_changed_added_and_removed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let pristine = tmp.path().join("pristine");
        let edited = tmp.path().join("edited");
        let patch_dir = tmp.path().join("patch");

        std::fs::create_dir_all(pristine.join("lib")).unwrap();
        std::fs::create_dir_all(edited.join("lib")).unwrap();

        std::fs::write(pristine.join("index.js"), "original").unwrap();
        std::fs::write(edited.join("index.js"), "patched").unwrap();

        std::fs::write(pristine.join("lib/keep.js"), "same").unwrap();
        std::fs::write(edited.join("lib/keep.js"), "same").unwrap();

        std::fs::write(pristine.join("lib/gone.js"), "bye").unwrap();
        // edited/lib/gone.js intentionally absent

        std::fs::write(edited.join("new.js"), "brand new").unwrap();

        let mut changed = 0usize;
        let mut removed = Vec::new();
        diff_dirs(
            &pristine,
            &edited,
            Path::new(""),
            &patch_dir,
            &mut changed,
            &mut removed,
        )
        .unwrap();

        assert_eq!(changed, 2, "index.js changed + new.js added");
        assert_eq!(
            std::fs::read_to_string(patch_dir.join("index.js")).unwrap(),
            "patched"
        );
        assert_eq!(
            std::fs::read_to_string(patch_dir.join("new.js")).unwrap(),
            "brand new"
        );
        assert!(!patch_dir.join("lib/keep.js").exists());
        assert_eq!(removed, vec!["lib/gone.js".to_string()]);
    }

    #[test]
    fn apply_patch_writes_changes_and_removals_without_touching_store() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path();
        let node_modules = project_root.join("node_modules");
        let installed = node_modules.join("left-pad");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("index.js"), "original").unwrap();
        std::fs::write(installed.join("extra.js"), "will be removed").unwrap();

        let patch_dir = project_root.join("patches").join("left-pad@1.3.0");
        std::fs::create_dir_all(&patch_dir).unwrap();
        std::fs::write(patch_dir.join("index.js"), "patched").unwrap();
        std::fs::write(patch_dir.join(".removed"), "extra.js").unwrap();

        apply_patch_if_present(project_root, &node_modules, "left-pad", "1.3.0");

        assert_eq!(
            std::fs::read_to_string(installed.join("index.js")).unwrap(),
            "patched"
        );
        assert!(!installed.join("extra.js").exists());
    }

    #[test]
    fn no_patch_dir_is_a_silent_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let node_modules = tmp.path().join("node_modules");
        std::fs::create_dir_all(node_modules.join("left-pad")).unwrap();
        // Should not panic when patches/ doesn't exist.
        apply_patch_if_present(tmp.path(), &node_modules, "left-pad", "1.3.0");
    }
}

#[cfg(test)]
mod zero_install_cache_tests {
    use super::{read_zero_install_cache, write_zero_install_cache, zero_install_cache_enabled};

    fn sri_sha512(bytes: &[u8]) -> String {
        use base64::Engine;
        use sha2::{Digest, Sha512};
        let mut h = Sha512::new();
        h.update(bytes);
        format!(
            "sha512-{}",
            base64::engine::general_purpose::STANDARD.encode(h.finalize())
        )
    }

    #[test]
    fn disabled_by_default() {
        assert!(!zero_install_cache_enabled(None));
        let val = serde_json::json!({});
        assert!(!zero_install_cache_enabled(Some(&val)));
    }

    #[test]
    fn enabled_via_manifest_key() {
        let val = serde_json::json!({"3va": {"zeroInstallCache": true}});
        assert!(zero_install_cache_enabled(Some(&val)));
    }

    #[test]
    fn round_trips_a_verified_tarball() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"fake tarball bytes";
        let integrity = sri_sha512(bytes);

        write_zero_install_cache(dir.path(), "left-pad", "1.3.0", bytes, &integrity);
        let restored = read_zero_install_cache(dir.path(), "left-pad", "1.3.0");
        assert_eq!(restored.as_deref(), Some(bytes.as_slice()));
    }

    #[test]
    fn rejects_tampered_cache_entry() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"fake tarball bytes";
        let integrity = sri_sha512(bytes);
        write_zero_install_cache(dir.path(), "left-pad", "1.3.0", bytes, &integrity);

        // Tamper with the cached tarball after it was written.
        let (tgz_path, _) = super::zero_install_cache_paths(dir.path(), "left-pad", "1.3.0");
        std::fs::write(&tgz_path, b"tampered bytes").unwrap();

        assert!(read_zero_install_cache(dir.path(), "left-pad", "1.3.0").is_none());
    }

    #[test]
    fn missing_cache_entry_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_zero_install_cache(dir.path(), "left-pad", "1.3.0").is_none());
    }
}

#[cfg(test)]
mod config_deps_tests {
    use super::install_config_deps;

    #[tokio::test]
    async fn no_configdependencies_key_returns_early_without_network() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"demo"}"#).unwrap();
        // No --allow-net given: if this tried to hit the network it would
        // bail on missing permission instead of returning Ok.
        install_config_deps(dir.path(), None).await.unwrap();
    }

    #[tokio::test]
    async fn empty_configdependencies_returns_early_without_network() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"demo","configDependencies":{}}"#,
        )
        .unwrap();
        install_config_deps(dir.path(), None).await.unwrap();
    }
}

#[cfg(test)]
mod peer_dep_tests {
    use super::collect_dep_specs;

    #[test]
    fn merges_dependencies_and_peer_dependencies() {
        let meta = serde_json::json!({
            "dependencies": {"lodash": "^4.0.0"},
            "peerDependencies": {"react": "^18.0.0"},
        });
        let mut specs = collect_dep_specs(&meta);
        specs.sort();
        assert_eq!(
            specs,
            vec![
                ("lodash".to_string(), "^4.0.0".to_string()),
                ("react".to_string(), "^18.0.0".to_string()),
            ]
        );
    }

    #[test]
    fn missing_peer_dependencies_is_fine() {
        let meta = serde_json::json!({"dependencies": {"lodash": "^4.0.0"}});
        assert_eq!(
            collect_dep_specs(&meta),
            vec![("lodash".to_string(), "^4.0.0".to_string())]
        );
    }
}

#[cfg(test)]
mod sbom_tests {
    use super::generate_sbom;
    use crate::lockfile::{Lockfile, LockfileDep, LockfilePackage};
    use std::collections::HashMap;

    fn sample_lockfile() -> Lockfile {
        let mut dependencies = HashMap::new();
        dependencies.insert(
            "left-pad".to_string(),
            LockfileDep {
                version: "1.3.0".to_string(),
                resolved: Some("https://registry.npmjs.org/left-pad/-/left-pad-1.3.0.tgz".into()),
                integrity: Some("sha512-abc123==".to_string()),
                dependencies: None,
                dev: None,
                registry: None,
            },
        );
        let mut packages = HashMap::new();
        packages.insert(
            "".to_string(),
            LockfilePackage {
                version: "1.0.0".to_string(),
                resolved: None,
                integrity: None,
                dev: None,
                registry: None,
            },
        );
        Lockfile {
            lockfile_version: 1,
            name: "demo".to_string(),
            version: "1.0.0".to_string(),
            packages,
            dependencies,
        }
    }

    #[test]
    fn emits_cyclonedx_shape() {
        let bom = generate_sbom(&sample_lockfile());
        assert_eq!(bom["bomFormat"], "CycloneDX");
        assert_eq!(bom["specVersion"], "1.5");
        assert_eq!(bom["metadata"]["component"]["name"], "demo");
        let components = bom["components"].as_array().unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0]["name"], "left-pad");
        assert_eq!(components[0]["purl"], "pkg:npm/left-pad@1.3.0");
        assert_eq!(components[0]["hashes"][0]["alg"], "SHA-512");
        assert_eq!(components[0]["hashes"][0]["content"], "abc123==");
    }
}

#[cfg(test)]
mod license_tests {
    use super::read_package_license;
    use std::io::Write;

    fn write_pkg_json(dir: &std::path::Path, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let mut f = std::fs::File::create(dir.join("package.json")).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn reads_spdx_string_license() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg_json(dir.path(), r#"{"license": "MIT"}"#);
        assert_eq!(read_package_license(dir.path()), "MIT");
    }

    #[test]
    fn reads_legacy_license_object() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg_json(dir.path(), r#"{"license": {"type": "ISC", "url": "x"}}"#);
        assert_eq!(read_package_license(dir.path()), "ISC");
    }

    #[test]
    fn reads_legacy_licenses_array() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg_json(
            dir.path(),
            r#"{"licenses": [{"type": "MIT"}, {"type": "Apache-2.0"}]}"#,
        );
        assert_eq!(read_package_license(dir.path()), "MIT OR Apache-2.0");
    }

    #[test]
    fn missing_package_json_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_package_license(dir.path()), "UNKNOWN");
    }
}

#[cfg(test)]
mod overrides_tests {
    use super::apply_override;
    use std::collections::HashMap;

    #[test]
    fn override_wins_over_requested_range() {
        let mut overrides = HashMap::new();
        overrides.insert("left-pad".to_string(), "1.3.0".to_string());

        assert_eq!(
            apply_override(&overrides, "left-pad", Some("^1.0.0".to_string())),
            Some("1.3.0".to_string())
        );
    }

    #[test]
    fn requested_range_passes_through_without_override() {
        let overrides = HashMap::new();
        assert_eq!(
            apply_override(&overrides, "left-pad", Some("^1.0.0".to_string())),
            Some("^1.0.0".to_string())
        );
    }

    #[test]
    fn override_applies_even_with_no_requested_range() {
        let mut overrides = HashMap::new();
        overrides.insert("left-pad".to_string(), "1.3.0".to_string());
        assert_eq!(
            apply_override(&overrides, "left-pad", None),
            Some("1.3.0".to_string())
        );
    }
}
