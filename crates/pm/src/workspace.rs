use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Config ────────────────────────────────────────────────────────────────────

/// Contents of `3va-workspace.json` at the monorepo root.
///
/// Also accepted: a `"workspaces"` array in the root `package.json`
/// (pnpm/Yarn-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Glob-style patterns for workspace package directories.
    /// Only the `dir/*` form (one level of wildcard) is supported today.
    /// Example: `["packages/*", "apps/*"]`
    pub packages: Vec<String>,
}

impl WorkspaceConfig {
    /// Discover workspace config from `root`, returning `None` if this is not
    /// a workspace root (i.e., neither `3va-workspace.json` nor a
    /// `"workspaces"` key in `package.json` exists).
    pub fn discover(root: &Path) -> anyhow::Result<Option<Self>> {
        // 1. Prefer dedicated 3va-workspace.json
        let json_path = root.join("3va-workspace.json");
        if json_path.exists() {
            let content = std::fs::read_to_string(&json_path)?;
            let cfg: WorkspaceConfig = serde_json::from_str(&content)
                .map_err(|e| anyhow::anyhow!("Invalid 3va-workspace.json: {}", e))?;
            return Ok(Some(cfg));
        }

        // 2. Fall back to "workspaces" array in package.json (pnpm/Yarn compat)
        let pkg_json = root.join("package.json");
        if pkg_json.exists() {
            let content = std::fs::read_to_string(&pkg_json)?;
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
                && let Some(ws) = val["workspaces"].as_array()
            {
                let packages: Vec<String> = ws
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                if !packages.is_empty() {
                    return Ok(Some(WorkspaceConfig { packages }));
                }
            }
        }

        Ok(None)
    }

    /// Walk all glob patterns and return metadata for every workspace package.
    pub fn resolve_packages(&self, root: &Path) -> anyhow::Result<Vec<WorkspacePackage>> {
        let mut result = Vec::new();

        for pattern in &self.packages {
            for pkg_dir in expand_pattern(root, pattern) {
                let pkg_json_path = pkg_dir.join("package.json");
                if !pkg_json_path.exists() {
                    continue;
                }
                match WorkspacePackage::from_dir(&pkg_dir) {
                    Ok(pkg) => result.push(pkg),
                    Err(e) => {
                        tracing::warn!("Skipping {}: {}", pkg_dir.display(), e);
                    }
                }
            }
        }

        Ok(result)
    }

    /// Save to `root/3va-workspace.json`.
    pub fn save(&self, root: &Path) -> anyhow::Result<()> {
        let path = root.join("3va-workspace.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

// ── Package ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WorkspacePackage {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    /// All runtime + dev deps combined (used for installation).
    pub all_deps: HashMap<String, String>,
}

impl WorkspacePackage {
    pub fn from_dir(dir: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(dir.join("package.json"))
            .map_err(|e| anyhow::anyhow!("Cannot read package.json in {}: {}", dir.display(), e))?;

        let val: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Invalid package.json in {}: {}", dir.display(), e))?;

        let name = val["name"]
            .as_str()
            .unwrap_or_else(|| {
                dir.file_name()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("unknown")
            })
            .to_string();
        let version = val["version"].as_str().unwrap_or("0.0.0").to_string();

        let mut all_deps: HashMap<String, String> = HashMap::new();
        for key in ["dependencies", "devDependencies", "peerDependencies"] {
            if let Some(obj) = val[key].as_object() {
                for (k, v) in obj {
                    if let Some(ver) = v.as_str() {
                        // Don't include workspace: cross-references as install targets
                        if ver.starts_with("workspace:") {
                            continue;
                        }
                        all_deps.entry(k.clone()).or_insert_with(|| ver.to_string());
                    }
                }
            }
        }

        Ok(WorkspacePackage {
            name,
            version,
            path: dir.to_path_buf(),
            all_deps,
        })
    }
}

// ── Glob expansion ────────────────────────────────────────────────────────────

/// Expand a single pattern relative to `root`.
///
/// Supported forms:
/// - `packages/*`  → every immediate subdirectory of `{root}/packages/`
/// - `apps/my-app` → exactly `{root}/apps/my-app` if it is a directory
/// - `*`           → every immediate subdirectory of `root`
fn expand_pattern(root: &Path, pattern: &str) -> Vec<PathBuf> {
    if pattern.ends_with("/*") || pattern == "*" {
        let base = if pattern == "*" {
            root.to_path_buf()
        } else {
            root.join(pattern.trim_end_matches("/*"))
        };
        match std::fs::read_dir(&base) {
            Ok(entries) => entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.path())
                .collect(),
            Err(_) => vec![],
        }
    } else {
        // Literal path
        let path = root.join(pattern);
        if path.is_dir() { vec![path] } else { vec![] }
    }
}

// ── Workspace symlinks ────────────────────────────────────────────────────────

/// Create `node_modules/{name}` symlinks in every workspace package that
/// declares a `workspace:*` (or `workspace:^version`) dependency on another
/// package in the same monorepo.
///
/// This mirrors how pnpm handles cross-package references: instead of
/// downloading a package from the registry, we just symlink the local source.
pub fn create_workspace_symlinks(root: &Path, packages: &[WorkspacePackage]) -> anyhow::Result<()> {
    // Build a map of name → path for all workspace packages.
    let name_to_path: std::collections::HashMap<&str, &Path> = packages
        .iter()
        .map(|p| (p.name.as_str(), p.path.as_path()))
        .collect();

    for pkg in packages {
        let pkg_json_path = pkg.path.join("package.json");
        if !pkg_json_path.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&pkg_json_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let val: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let node_modules = pkg.path.join("node_modules");

        for key in ["dependencies", "devDependencies", "peerDependencies"] {
            if let Some(obj) = val[key].as_object() {
                for (dep_name, dep_version) in obj {
                    let ver = match dep_version.as_str() {
                        Some(v) => v,
                        None => continue,
                    };
                    if !ver.starts_with("workspace:") {
                        continue;
                    }

                    // Resolve the target workspace package.
                    let target_path = match name_to_path.get(dep_name.as_str()) {
                        Some(p) => *p,
                        None => {
                            tracing::warn!(
                                "{}: workspace:{} not found — skipping symlink",
                                pkg.name,
                                dep_name
                            );
                            continue;
                        }
                    };

                    std::fs::create_dir_all(&node_modules)?;

                    let link_path = if dep_name.contains('/') {
                        let scope_dir =
                            node_modules.join(dep_name.split('/').next().unwrap_or(dep_name));
                        std::fs::create_dir_all(&scope_dir)?;
                        node_modules.join(dep_name.as_str())
                    } else {
                        node_modules.join(dep_name.as_str())
                    };

                    // Remove stale link/dir if present.
                    if link_path.exists() || link_path.is_symlink() {
                        if link_path.is_symlink() || link_path.is_file() {
                            std::fs::remove_file(&link_path)?;
                        } else {
                            std::fs::remove_dir_all(&link_path)?;
                        }
                    }

                    // Use absolute target so the symlink works regardless of CWD.
                    let abs_target = if target_path.is_absolute() {
                        target_path.to_path_buf()
                    } else {
                        root.join(target_path)
                    };

                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&abs_target, &link_path).map_err(|e| {
                        anyhow::anyhow!(
                            "Cannot create symlink {} → {}: {}",
                            link_path.display(),
                            abs_target.display(),
                            e
                        )
                    })?;

                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(&abs_target, &link_path).map_err(|e| {
                        anyhow::anyhow!(
                            "Cannot create symlink {} → {}: {}",
                            link_path.display(),
                            abs_target.display(),
                            e
                        )
                    })?;

                    tracing::info!(
                        "{}: linked workspace dep {} → {}",
                        pkg.name,
                        dep_name,
                        abs_target.display()
                    );
                }
            }
        }
    }

    Ok(())
}

// ── Install coordinator ───────────────────────────────────────────────────────

/// Collect all unique external deps across every workspace package, merging
/// versions (highest wins when there is a conflict).
pub fn merged_deps(packages: &[WorkspacePackage]) -> HashMap<String, String> {
    let mut merged: HashMap<String, String> = HashMap::new();

    for pkg in packages {
        for (name, version) in &pkg.all_deps {
            let entry = merged
                .entry(name.clone())
                .or_insert_with(|| version.clone());
            // When two packages want different versions, keep the higher one.
            if semver_gt(version, entry) {
                *entry = version.clone();
            }
        }
    }

    merged
}

/// Very small semver comparator — enough to pick the higher of two version
/// strings like `"1.2.3"` or `"^2.0.0"`.  Returns `true` if `a > b`.
fn semver_gt(a: &str, b: &str) -> bool {
    fn score(v: &str) -> (u64, u64, u64) {
        let v = v.trim_start_matches(['^', '~', 'v', '>', '=', ' ']);
        let v = v.split(['-', '+']).next().unwrap_or(v);
        let mut parts = v.split('.');
        let major: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minor: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    }
    score(a) > score(b)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pkg_json(dir: &Path, name: &str, deps: &[(&str, &str)]) {
        let mut dep_map = serde_json::Map::new();
        for (k, v) in deps {
            dep_map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
        let val = serde_json::json!({
            "name": name,
            "version": "1.0.0",
            "dependencies": dep_map
        });
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            serde_json::to_string_pretty(&val).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn discover_from_workspace_json() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = WorkspaceConfig {
            packages: vec!["packages/*".to_string()],
        };
        cfg.save(tmp.path()).unwrap();
        let found = WorkspaceConfig::discover(tmp.path()).unwrap().unwrap();
        assert_eq!(found.packages, vec!["packages/*"]);
    }

    #[test]
    fn discover_from_package_json_workspaces() {
        let tmp = tempfile::tempdir().unwrap();
        let val = serde_json::json!({
            "name": "root",
            "workspaces": ["packages/*", "apps/*"]
        });
        std::fs::write(
            tmp.path().join("package.json"),
            serde_json::to_string(&val).unwrap(),
        )
        .unwrap();

        let found = WorkspaceConfig::discover(tmp.path()).unwrap().unwrap();
        assert_eq!(found.packages.len(), 2);
    }

    #[test]
    fn discover_returns_none_for_plain_project() {
        let tmp = tempfile::tempdir().unwrap();
        let val = serde_json::json!({ "name": "my-app", "version": "1.0.0" });
        std::fs::write(
            tmp.path().join("package.json"),
            serde_json::to_string(&val).unwrap(),
        )
        .unwrap();

        assert!(WorkspaceConfig::discover(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn resolve_packages_with_glob() {
        let tmp = tempfile::tempdir().unwrap();
        let packages_dir = tmp.path().join("packages");
        make_pkg_json(&packages_dir.join("core"), "core", &[("lodash", "4.17.21")]);
        make_pkg_json(&packages_dir.join("ui"), "ui", &[("react", "18.2.0")]);

        let cfg = WorkspaceConfig {
            packages: vec!["packages/*".to_string()],
        };
        let pkgs = cfg.resolve_packages(tmp.path()).unwrap();
        assert_eq!(pkgs.len(), 2);

        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"core"));
        assert!(names.contains(&"ui"));
    }

    #[test]
    fn workspace_package_skips_workspace_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let val = serde_json::json!({
            "name": "app",
            "version": "1.0.0",
            "dependencies": {
                "core": "workspace:*",
                "axios": "1.7.9"
            }
        });
        std::fs::write(
            tmp.path().join("package.json"),
            serde_json::to_string(&val).unwrap(),
        )
        .unwrap();
        let pkg = WorkspacePackage::from_dir(tmp.path()).unwrap();
        assert!(
            !pkg.all_deps.contains_key("core"),
            "workspace: refs must be skipped"
        );
        assert!(pkg.all_deps.contains_key("axios"));
    }

    #[test]
    fn merged_deps_deduplicates_and_picks_higher() {
        let a = WorkspacePackage {
            name: "a".into(),
            version: "1.0.0".into(),
            path: PathBuf::from("/a"),
            all_deps: [("lodash".to_string(), "4.17.0".to_string())].into(),
        };
        let b = WorkspacePackage {
            name: "b".into(),
            version: "1.0.0".into(),
            path: PathBuf::from("/b"),
            all_deps: [("lodash".to_string(), "4.17.21".to_string())].into(),
        };
        let merged = merged_deps(&[a, b]);
        assert_eq!(merged["lodash"], "4.17.21");
    }

    #[test]
    fn semver_gt_works() {
        assert!(semver_gt("2.0.0", "1.9.9"));
        assert!(semver_gt("1.1.0", "1.0.99"));
        assert!(!semver_gt("1.0.0", "1.0.0"));
        assert!(!semver_gt("0.9.0", "1.0.0"));
        assert!(semver_gt("^2.0.0", "^1.5.0"));
    }
}
