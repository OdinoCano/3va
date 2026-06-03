use crate::lockfile::{Lockfile, LockfileDep, LockfilePackage};
use std::collections::HashMap;

// ── pnpm-lock.yaml parser ─────────────────────────────────────────────────────
//
// Format (v6/v9):
//
//   lockfileVersion: '6.0' (or 9.0)
//
//   importers:
//     .:
//       dependencies:
//         express: 4.18.2
//       specifiers:
//         express: ^4.18.2
//
//   packages:
//     /express/4.18.2:
//       resolution: {integrity: sha512-...}
//       dependencies:
//         accepts: 1.3.8
//       engines: {node: '>= 0.10.0'}
//       dev: false
//       name: express
//       version: 4.18.2
//
// We parse the YAML manually for simplicity (avoids adding a serde_yaml dep).

/// Parse a pnpm-lock.yaml file content and return a simplified key → entry map.
fn parse_pnpm_packages(content: &str) -> HashMap<String, PnpmEntry> {
    let mut entries = HashMap::new();

    // Find the `packages:` section
    let packages_start = content.find("packages:");
    let packages_section = match packages_start {
        Some(pos) => &content[pos..], // start at "packages:"
        None => return entries,
    };

    // Split into top-level entries (lines starting with "    /" or "  /")
    let mut current_path: Option<String> = None;
    let mut current_entry: Option<PnpmEntryBuilder> = None;

    for line in packages_section.lines().skip(1) {
        // Stop at next top-level key or blank line separating sections
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        if trimmed.is_empty() {
            // Blank line: end of packages section or separator
            flush_entry(&mut current_path, &mut current_entry, &mut entries);
            continue;
        }

        // New package entry (indent 2 or 4 spaces before /)
        if trimmed.starts_with('/') && indent <= 4 {
            flush_entry(&mut current_path, &mut current_entry, &mut entries);

            // Extract the path: /express/4.18.2
            let path = trimmed.trim_end_matches(':').trim().to_string();
            current_path = Some(path);
            current_entry = Some(PnpmEntryBuilder::new());
            continue;
        }

        // If we have no current entry, skip
        let entry = match current_entry.as_mut() {
            Some(e) => e,
            None => continue,
        };

        // Parse fields
        if let Some(rest) = trimmed.strip_prefix("version: ") {
            entry.version = Some(strip_yaml_value(rest));
        } else if let Some(rest) = trimmed.strip_prefix("name: ") {
            entry.name = Some(strip_yaml_value(rest));
        } else if trimmed.starts_with("resolution:") {
            entry.in_resolution = true;
        } else if trimmed.starts_with("dependencies:") {
            entry.in_deps = true;
            entry.in_opt_deps = false;
            entry.in_resolution = false;
        } else if trimmed.starts_with("optionalDependencies:") {
            entry.in_opt_deps = true;
            entry.in_deps = false;
            entry.in_resolution = false;
        } else if let Some(rest) = trimmed.strip_prefix("dev: ") {
            entry.dev = Some(strip_yaml_value(rest) == "true");
            entry.in_deps = false;
            entry.in_opt_deps = false;
            entry.in_resolution = false;
        } else if entry.in_resolution {
            if let Some(inner) = trimmed.strip_prefix("integrity: ") {
                entry.integrity = Some(strip_yaml_value(inner));
                entry.in_resolution = false;
            }
        } else if (entry.in_deps || entry.in_opt_deps)
            && let Some(eq_pos) = trimmed.find(':')
        {
            let dep_name = trimmed[..eq_pos].trim().to_string();
            let dep_ver = strip_yaml_value(&trimmed[eq_pos + 1..]);
            entry.dependencies.entry(dep_name).or_insert(dep_ver);
        }
    }

    flush_entry(&mut current_path, &mut current_entry, &mut entries);
    entries
}

fn flush_entry(
    path: &mut Option<String>,
    builder: &mut Option<PnpmEntryBuilder>,
    entries: &mut HashMap<String, PnpmEntry>,
) {
    if let (Some(p), Some(b)) = (path.take(), builder.take()) {
        entries.insert(p, b.build());
    }
}

fn strip_yaml_value(s: &str) -> String {
    s.trim().trim_matches('\'').trim_matches('"').to_string()
}

struct PnpmEntryBuilder {
    name: Option<String>,
    version: Option<String>,
    integrity: Option<String>,
    dev: Option<bool>,
    dependencies: HashMap<String, String>,
    in_deps: bool,
    in_opt_deps: bool,
    in_resolution: bool,
}

impl PnpmEntryBuilder {
    fn new() -> Self {
        Self {
            name: None,
            version: None,
            integrity: None,
            dev: None,
            dependencies: HashMap::new(),
            in_deps: false,
            in_opt_deps: false,
            in_resolution: false,
        }
    }

    fn build(self) -> PnpmEntry {
        PnpmEntry {
            name: self.name,
            version: self.version.unwrap_or_else(|| "0.0.0".to_string()),
            integrity: self.integrity,
            dev: self.dev,
            dependencies: self.dependencies,
        }
    }
}

struct PnpmEntry {
    name: Option<String>,
    version: String,
    integrity: Option<String>,
    dev: Option<bool>,
    dependencies: HashMap<String, String>,
}

/// Extract package name from a pnpm path like `/express/4.18.2`.
fn pnpm_path_to_name(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 {
        let name = parts[1..parts.len() - 1].join("/");
        // Handle scoped: @scope/pkg → path is @scope+pkg
        if name.contains('+') && name.starts_with('@') {
            let plus_pos = name.find('+').unwrap();
            let scope = &name[..plus_pos];
            let rest = &name[plus_pos + 1..];
            format!("{}/{}", scope, rest)
        } else {
            name
        }
    } else {
        path.to_string()
    }
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Load a lockfile from pnpm's `pnpm-lock.yaml` format.
///
/// Returns `None` if the file doesn't exist.
pub fn load_from_pnpm_lock(path: &std::path::Path) -> anyhow::Result<Option<Lockfile>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", path.display(), e))?;

    let pnpm_pkgs = parse_pnpm_packages(&content);
    if pnpm_pkgs.is_empty() {
        return Ok(None);
    }

    let mut packages = HashMap::new();
    let mut dependencies = HashMap::new();

    packages.insert(
        String::new(),
        LockfilePackage {
            version: "0.0.0".to_string(),
            resolved: None,
            integrity: None,
            dev: None,
            registry: None,
        },
    );

    for (path_key, entry) in &pnpm_pkgs {
        let pkg_name = entry
            .name
            .clone()
            .unwrap_or_else(|| pnpm_path_to_name(path_key));

        packages.insert(
            format!("node_modules/{}", pkg_name),
            LockfilePackage {
                version: entry.version.clone(),
                resolved: None,
                integrity: entry.integrity.clone(),
                dev: entry.dev,
                registry: None,
            },
        );

        dependencies.entry(pkg_name).or_insert_with(|| {
            let deps = if entry.dependencies.is_empty() {
                None
            } else {
                Some(entry.dependencies.clone())
            };
            LockfileDep {
                version: entry.version.clone(),
                resolved: None,
                integrity: entry.integrity.clone(),
                dependencies: deps,
                dev: entry.dev,
                registry: None,
            }
        });
    }

    Ok(Some(Lockfile {
        lockfile_version: 0,
        name: path
            .parent()
            .and_then(|p| {
                let pkg = p.join("package.json");
                std::fs::read_to_string(pkg).ok().and_then(|c| {
                    serde_json::from_str::<serde_json::Value>(&c)
                        .ok()
                        .and_then(|v| v["name"].as_str().map(String::from))
                })
            })
            .unwrap_or_else(|| "pnpm-project".to_string()),
        version: "0.0.0".to_string(),
        packages,
        dependencies,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_pnpm_packages_simple() {
        let content = r#"
packages:
  /express/4.18.2:
    version: 4.18.2
    engines: {node: '>= 0.10.0'}
    dev: false
    resolution:
      integrity: sha512-fakehash
    dependencies:
      accepts: 1.3.8
  /accepts/1.3.8:
    version: 1.3.8
    dev: false
"#;

        let entries = parse_pnpm_packages(content);
        assert!(entries.contains_key("/express/4.18.2"));
        assert!(entries.contains_key("/accepts/1.3.8"));
        assert_eq!(entries["/express/4.18.2"].version, "4.18.2");
        assert!(
            entries["/express/4.18.2"]
                .dependencies
                .contains_key("accepts")
        );
    }

    #[test]
    fn parse_pnpm_scoped_package() {
        let content = r#"
packages:
  /@babel+core/7.24.0:
    version: 7.24.0
    name: '@babel/core'
    resolution:
      integrity: sha512-fake
    dependencies:
      '@babel/helper-plugin-utils': ^7.24.0
"#;

        let entries = parse_pnpm_packages(content);
        assert!(entries.contains_key("/@babel+core/7.24.0"));
        let entry = &entries["/@babel+core/7.24.0"];
        assert_eq!(entry.name.as_deref(), Some("@babel/core"));
    }

    #[test]
    fn pnpm_path_to_name_works() {
        assert_eq!(pnpm_path_to_name("/express/4.18.2"), "express");
        assert_eq!(pnpm_path_to_name("/lodash/4.17.21"), "lodash");
    }

    #[test]
    fn pnpm_path_to_name_scoped() {
        assert_eq!(pnpm_path_to_name("/@babel+core/7.24.0"), "@babel/core");
    }

    #[test]
    fn load_from_pnpm_lock_nonexistent() {
        let result =
            load_from_pnpm_lock(std::path::Path::new("/nonexistent/pnpm-lock.yaml")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_from_pnpm_lock_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pnpm-lock.yaml");
        std::fs::write(
            &path,
            "packages:\n  /axios/1.7.9:\n    version: 1.7.9\n    dev: false\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"my-project","version":"1.0.0"}"#,
        )
        .unwrap();

        let result = load_from_pnpm_lock(&path).unwrap();
        assert!(result.is_some());
        let lock = result.unwrap();
        assert!(lock.dependencies.contains_key("axios"));
        assert_eq!(lock.dependencies["axios"].version, "1.7.9");
    }
}
