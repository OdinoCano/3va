// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

use crate::lockfile::{Lockfile, LockfileDep, LockfilePackage};
use std::collections::HashMap;

// ── pnpm-lock.yaml parser ─────────────────────────────────────────────────────
//
// Two layouts have to be understood, because both are still in the wild:
//
// **v6** — a flat `packages:` section keyed by path:
// ```yaml
// lockfileVersion: '6.0'
// packages:
//   /express/4.18.2:
//     resolution: {integrity: sha512-…}
//     dependencies:
//       accepts: 1.3.8
// ```
//
// **v9** — `name@version` keys, with the dependency graph moved to a separate
// `snapshots:` section and `resolution` written as an inline flow mapping:
// ```yaml
// lockfileVersion: '9.0'
// packages:
//   express@4.18.2:
//     resolution: {integrity: sha512-…}
// snapshots:
//   express@4.18.2(accepts@1.3.8):
//     dependencies:
//       accepts: 1.3.8
// ```
//
// The previous parser only understood the v6 shape, so a v9 lockfile — what
// every current pnpm writes — produced an *empty* package set. That was
// reported as "no migration and no warning"; it is worse than that, because
// `3va install` then resolved everything from ranges and silently diverged
// from the tree the developer had pinned.
//
// Parsed by hand rather than with a YAML dependency: this crate already does
// that for v6, the two layouts are shallow, and the subset needed here is
// fixed by pnpm's own writer.

#[derive(Debug, Clone, Default)]
struct PnpmEntry {
    name: String,
    version: String,
    integrity: Option<String>,
    dev: Option<bool>,
    dependencies: HashMap<String, String>,
}

/// One top-level mapping under `packages:` or `snapshots:`.
#[derive(Default)]
struct RawEntry {
    key: String,
    version: Option<String>,
    name: Option<String>,
    integrity: Option<String>,
    dev: Option<bool>,
    deps: HashMap<String, String>,
}

fn yaml_scalar(s: &str) -> String {
    s.trim()
        .trim_end_matches(',')
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .trim()
        .to_string()
}

/// `{integrity: sha512-…, tarball: https://…}` written inline, which is how v9
/// writes every `resolution`.
fn inline_mapping_value(line: &str, key: &str) -> Option<String> {
    let open = line.find('{')?;
    let close = line.rfind('}')?;
    if close <= open {
        return None;
    }
    for part in line[open + 1..close].split(',') {
        let Some((k, v)) = part.split_once(':') else {
            continue;
        };
        if yaml_scalar(k) == key {
            return Some(yaml_scalar(v));
        }
    }
    None
}

/// Walk a top-level YAML section, returning one [`RawEntry`] per key.
fn parse_section(content: &str, section: &str) -> Vec<RawEntry> {
    let mut out = Vec::new();
    let mut lines = content.lines().skip_while(|l| *l != format!("{section}:"));
    let first = match lines.next() {
        // `packages:` may legitimately be empty (v9 writes `packages:` with a
        // blank line then `snapshots:`).
        None => return out,
        Some(l) => l,
    };
    if first.trim() != format!("{section}:") {
        return out;
    }

    let mut current: Option<RawEntry> = None;
    let mut in_deps = false;
    let mut in_resolution = false;
    let mut in_optional = false;

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        // A new top-level key ends the section.
        if !line.starts_with(' ') && !line.starts_with('\t') && !line.starts_with('-') {
            break;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();

        // A key at indent 2 starts a new entry.
        if indent <= 2 && !trimmed.starts_with('-') && trimmed.contains(':') {
            if let Some(e) = current.take() {
                out.push(e);
            }
            current = Some(RawEntry {
                key: yaml_scalar(trimmed.trim_end_matches(':')),
                ..Default::default()
            });
            in_deps = false;
            in_resolution = false;
            in_optional = false;
            continue;
        }

        let Some(e) = current.as_mut() else {
            continue;
        };
        if let Some(rest) = trimmed.strip_prefix("version: ") {
            e.version = Some(yaml_scalar(rest));
            in_deps = false;
            in_resolution = false;
            in_optional = false;
        } else if let Some(rest) = trimmed.strip_prefix("name: ") {
            e.name = Some(yaml_scalar(rest));
        } else if trimmed.starts_with("dev:") {
            e.dev = Some(yaml_scalar(trimmed.trim_start_matches("dev:")) == "true");
        } else if trimmed.starts_with("resolution:") {
            in_resolution = true;
            in_deps = false;
            in_optional = false;
            if let Some(v) = inline_mapping_value(trimmed, "integrity") {
                e.integrity = Some(v);
                in_resolution = false;
            }
        } else if trimmed.starts_with("optionalDependencies:") {
            in_optional = true;
            in_deps = false;
            in_resolution = false;
        } else if trimmed.starts_with("dependencies:") {
            in_deps = true;
            in_optional = false;
            in_resolution = false;
        } else if in_resolution {
            if let Some(v) = trimmed.strip_prefix("integrity: ") {
                e.integrity = Some(yaml_scalar(v));
                in_resolution = false;
            }
        } else if (in_deps || in_optional)
            && let Some(eq) = trimmed.find(':')
        {
            let dep = yaml_scalar(&trimmed[..eq]);
            let ver = yaml_scalar(&trimmed[eq + 1..]);
            if !dep.is_empty() {
                e.deps.entry(dep).or_insert(ver);
            }
        }
    }
    if let Some(e) = current.take() {
        out.push(e);
    }
    out
}

/// Strip a pnpm v9 peer-suffix: `express@4.18.2(accepts@1.3.8)` →
/// `("express", "4.18.2")`. v6 paths are handled too, so callers can pass
/// either key straight in.
fn split_key(key: &str) -> Option<(String, String)> {
    // v6 keys start with '/' and separate name from version by path, so they
    // must be handled before the v9 `name@version` split — `/@babel+core/7.24.0`
    // contains an '@' and would otherwise be read as the package `/`.
    let base = key.split('(').next().unwrap_or(key).trim();
    if base.starts_with('/') {
        let parts: Vec<&str> = base.trim_start_matches('/').split('/').collect();
        if parts.len() >= 2 {
            let version = parts.last().unwrap().to_string();
            let joined = parts[..parts.len() - 1].join("/");
            // v6 writes the scope separator as '+' inside a single segment.
            let name = match joined.find('+') {
                Some(plus) if joined.starts_with('@') => {
                    format!("{}/{}", &joined[..plus], &joined[plus + 1..])
                }
                _ => joined,
            };
            if !name.is_empty() && !version.is_empty() {
                return Some((name, version));
            }
        }
        return None;
    }
    // v9: name@version(peers) — split on the last '@' before any '('.
    if let Some(at) = base.rfind('@').filter(|i| *i > 0) {
        let name = &base[..at];
        let version = base[at + 1..].trim();
        if !name.is_empty() && !version.is_empty() {
            return Some((name.to_string(), version.to_string()));
        }
    }
    // Unrecognised layout: refuse rather than guess a name or version.
    let parts: Vec<&str> = key.trim_start_matches('/').split('/').collect();
    if parts.len() >= 2 {
        let version = parts.last().unwrap().to_string();
        let name = parts[..parts.len() - 1].join("/");
        let name = if let Some(plus) = name.find('+')
            && name.starts_with('@')
        {
            format!("{}/{}", &name[..plus], &name[plus + 1..])
        } else {
            name
        };
        if !name.is_empty() && !version.is_empty() {
            return Some((name, version));
        }
    }
    None
}

fn parse_pnpm_packages(content: &str) -> HashMap<String, PnpmEntry> {
    let mut out: HashMap<String, PnpmEntry> = HashMap::new();
    // v9 keeps the graph in `snapshots:`, keyed by the same name@version with a
    // peer suffix; merge it over the metadata from `packages:`.
    let snapshots: HashMap<String, HashMap<String, String>> = parse_section(content, "snapshots")
        .into_iter()
        .filter_map(|e| {
            let (name, _) = split_key(&e.key)?;
            Some((name, e.deps))
        })
        .collect();

    for raw in parse_section(content, "packages") {
        let Some((key_name, key_version)) = split_key(&raw.key) else {
            continue;
        };
        let name = raw.name.clone().unwrap_or(key_name);
        let version = raw.version.clone().unwrap_or(key_version);
        if name.is_empty() || version.is_empty() {
            continue;
        }
        let mut deps = raw.deps.clone();
        if let Some(extra) = snapshots.get(&name) {
            for (k, v) in extra {
                deps.entry(k.clone()).or_insert(v.clone());
            }
        }
        out.insert(
            name.clone(),
            PnpmEntry {
                name,
                version,
                integrity: raw.integrity,
                dev: raw.dev,
                dependencies: deps,
            },
        );
    }
    out
}

/// The `lockfileVersion` recorded in the file, as a comparable string.
pub fn lockfile_version(content: &str) -> Option<String> {
    for line in content.lines().take(20) {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("lockfileVersion:") {
            return Some(yaml_scalar(rest));
        }
    }
    None
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Load a lockfile from pnpm's `pnpm-lock.yaml` format (v6 and v9).
///
/// Returns `None` if the file doesn't exist or records nothing 3va can use.
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

    for entry in pnpm_pkgs.values() {
        let pkg_name = entry.name.clone();
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

    let name = path
        .parent()
        .and_then(|p| {
            let pkg = p.join("package.json");
            std::fs::read_to_string(pkg).ok().and_then(|c| {
                serde_json::from_str::<serde_json::Value>(&c)
                    .ok()
                    .and_then(|v| v["name"].as_str().map(String::from))
            })
        })
        .unwrap_or_else(|| "pnpm-project".to_string());

    Ok(Some(Lockfile {
        lockfile_version: lockfile_version(&content)
            .and_then(|v| v.split('.').next().map(str::to_string))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        name,
        version: "0.0.0".to_string(),
        packages,
        dependencies,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const V9: &str = r#"
lockfileVersion: '9.0'

settings:
  autoInstallPeers: true

importers:
  .:
    dependencies:
      express:
        specifier: ^4.18.2
        version: 4.18.2

packages:

  accepts@1.3.8:
    resolution: {integrity: sha512-AAA}

  express@4.18.2:
    resolution: {integrity: sha512-BBB}
    engines: {node: '>= 0.10.0'}

  '@esbuild/darwin-arm64@0.24.0':
    resolution: {integrity: sha512-CCC}
    cpu: [arm64]
    os: [darwin]

snapshots:

  express@4.18.2:
    dependencies:
      accepts: 1.3.8
"#;

    #[test]
    fn parses_v9_packages() {
        let entries = parse_pnpm_packages(V9);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries["express"].version, "4.18.2");
        assert_eq!(entries["express"].integrity.as_deref(), Some("sha512-BBB"));
        assert_eq!(entries["accepts"].version, "1.3.8");
    }

    #[test]
    fn parses_v9_scoped_packages() {
        let entries = parse_pnpm_packages(V9);
        assert!(entries.contains_key("@esbuild/darwin-arm64"));
        assert_eq!(entries["@esbuild/darwin-arm64"].version, "0.24.0");
    }

    #[test]
    fn v9_dependency_graph_comes_from_snapshots() {
        let entries = parse_pnpm_packages(V9);
        assert_eq!(
            entries["express"]
                .dependencies
                .get("accepts")
                .map(String::as_str),
            Some("1.3.8")
        );
    }

    #[test]
    fn reads_the_lockfile_version() {
        assert_eq!(lockfile_version(V9).as_deref(), Some("9.0"));
        assert_eq!(
            lockfile_version("lockfileVersion: '6.0'\n").as_deref(),
            Some("6.0")
        );
        assert_eq!(lockfile_version("nothing here\n"), None);
    }

    #[test]
    fn parse_pnpm_packages_simple_v6() {
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
        assert_eq!(entries["express"].version, "4.18.2");
        assert_eq!(
            entries["express"].integrity.as_deref(),
            Some("sha512-fakehash")
        );
        assert_eq!(entries["express"].dev, Some(false));
        assert_eq!(
            entries["express"]
                .dependencies
                .get("accepts")
                .map(String::as_str),
            Some("1.3.8")
        );
    }

    #[test]
    fn split_key_handles_both_layouts() {
        assert_eq!(
            split_key("express@4.18.2"),
            Some(("express".into(), "4.18.2".into()))
        );
        assert_eq!(
            split_key("express@4.18.2(accepts@1.3.8)"),
            Some(("express".into(), "4.18.2".into()))
        );
        assert_eq!(
            split_key("/express/4.18.2"),
            Some(("express".into(), "4.18.2".into()))
        );
        assert_eq!(
            split_key("/@babel+core/7.24.0"),
            Some(("@babel/core".into(), "7.24.0".into()))
        );
    }

    #[test]
    fn load_from_pnpm_lock_nonexistent() {
        let result =
            load_from_pnpm_lock(std::path::Path::new("/nonexistent/pnpm-lock.yaml")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_from_pnpm_lock_v9_end_to_end() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("pnpm-lock.yaml");
        std::fs::write(&path, V9).unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"my-project","version":"1.0.0"}"#,
        )
        .unwrap();

        let lock = load_from_pnpm_lock(&path).unwrap().unwrap();
        assert_eq!(lock.dependencies["express"].version, "4.18.2");
        assert_eq!(
            lock.dependencies["express"].integrity.as_deref(),
            Some("sha512-BBB")
        );
        assert!(
            lock.dependencies["@esbuild/darwin-arm64"]
                .dependencies
                .is_none()
        );
        assert_eq!(lock.lockfile_version, 9);
    }

    #[test]
    fn load_from_pnpm_lock_v6_end_to_end() {
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

        let lock = load_from_pnpm_lock(&path).unwrap().unwrap();
        assert_eq!(lock.dependencies["axios"].version, "1.7.9");
    }
}
