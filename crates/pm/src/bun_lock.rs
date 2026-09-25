// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

use crate::lockfile::{Lockfile, LockfileDep, LockfilePackage};
use std::collections::HashMap;

// ── Bun text lockfile (`bun.lock`) ─────────────────────────────────────────────
//
// Bun's binary `bun.lockb` cannot be read from here, and its readable sibling
// `bun.lock` is JSONC with a nested tuple layout:
// ```jsonc
// {
//   "lockfileVersion": 1,
//   "workspaces": { "": { "name": "app", "dependencies": { "ms": "^2.1.3" } } },
//   "packages": {
//     // key is the request, value is ["resolved@version", registry, meta, integrity]
//     "ms": ["ms@2.1.3", "", { "dependencies": {} }, "sha512-…"]
//   }
// }
// ```
// `bun install --save-text-lockfile` writes the text form, so detecting it and
// converting it faithfully is what lets a Bun project keep pinning versions.

/// Strip JSONC comments and trailing commas so `serde_json` will take the file.
///
/// Bun emits both, and it writes a header comment naming the bun version —
/// which is also the cheapest possible detection signal.
pub fn strip_jsonc(content: &str) -> String {
    let bytes = content.as_bytes();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    let mut in_string = false;
    let mut escaped = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            '/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    // Trailing commas before } or ]. The comma is usually followed by a newline
    // and indentation, so the scan has to look back over whitespace rather than
    // only at the previous character.
    let mut cleaned = String::with_capacity(out.len());
    let mut in_str = false;
    let mut escaped = false;
    for c in out.chars() {
        if in_str {
            cleaned.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if c == '"' {
            in_str = true;
        } else if c == '}' || c == ']' {
            while cleaned.chars().next_back().is_some_and(char::is_whitespace) {
                cleaned.pop();
            }
            if cleaned.ends_with(',') {
                cleaned.pop();
            }
        }
        cleaned.push(c);
    }
    cleaned
}

/// `ms@2.1.3` → `("ms", "2.1.3")`; a bare name is a workspace/link entry with
/// no version and is skipped by the caller.
fn split_resolved(spec: &str) -> Option<(String, String)> {
    let at = spec.rfind('@').filter(|i| *i > 0)?;
    let name = spec[..at].to_string();
    let version = spec[at + 1..].trim();
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name, version.to_string()))
}

/// Load a lockfile from Bun's readable `bun.lock`.
///
/// Returns `None` when the file does not exist, is not JSONC at all, or holds
/// no package entries — never a lockfile with invented versions.
pub fn load_from_bun_lock(path: &std::path::Path) -> anyhow::Result<Option<Lockfile>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", path.display(), e))?;
    let value: serde_json::Value = serde_json::from_str(&strip_jsonc(&raw))
        .map_err(|e| anyhow::anyhow!("Cannot parse {}: {}", path.display(), e))?;

    let Some(packages) = value.get("packages").and_then(|p| p.as_object()) else {
        return Ok(None);
    };
    if packages.is_empty() {
        return Ok(None);
    }

    let mut pkgs: HashMap<String, LockfileDep> = HashMap::new();
    for entry in packages.values() {
        let Some(tuple) = entry.as_array() else {
            continue;
        };
        let Some(resolved) = tuple.first().and_then(|v| v.as_str()) else {
            continue;
        };
        let Some((name, version)) = split_resolved(resolved) else {
            continue;
        };
        let integrity = tuple
            .iter()
            .skip(3)
            .find_map(|v| v.as_str())
            .map(|s| s.to_string());
        let dependencies = tuple
            .get(2)
            .and_then(|m| m.get("dependencies"))
            .and_then(|d| d.as_object())
            .map(|d| {
                d.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<HashMap<String, String>>()
            })
            .filter(|d: &HashMap<String, String>| !d.is_empty());
        pkgs.entry(name).or_insert(LockfileDep {
            version,
            resolved: Some(resolved.to_string()),
            integrity,
            dependencies,
            dev: None,
            registry: None,
        });
    }
    if pkgs.is_empty() {
        return Ok(None);
    }

    let mut packages = HashMap::new();
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
    for (name, dep) in &pkgs {
        packages.insert(
            format!("node_modules/{name}"),
            LockfilePackage {
                version: dep.version.clone(),
                resolved: dep.resolved.clone(),
                integrity: dep.integrity.clone(),
                dev: dep.dev,
                registry: None,
            },
        );
    }

    let name = value["workspaces"][""]["name"]
        .as_str()
        .map(String::from)
        .or_else(|| {
            path.parent().and_then(|p| {
                std::fs::read_to_string(p.join("package.json"))
                    .ok()
                    .and_then(|c| {
                        serde_json::from_str::<serde_json::Value>(&c)
                            .ok()
                            .and_then(|v| v["name"].as_str().map(String::from))
                    })
            })
        })
        .unwrap_or_else(|| "bun-project".to_string());

    Ok(Some(Lockfile {
        lockfile_version: value
            .get("lockfileVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32,
        name,
        version: "0.0.0".to_string(),
        packages,
        dependencies: pkgs,
    }))
}

/// Is this a Bun text lockfile? The header comment is the reliable signal;
/// falling back to the `lockfileVersion` + `packages` shape covers a file that
/// has had its comments stripped.
pub fn looks_like_bun_lock(content: &str) -> bool {
    content.contains("bun.lock")
        || (content.contains("\"lockfileVersion\"")
            && content.contains("\"packages\"")
            && !content.contains("\"node_modules/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"// bun.lockb
// bun v1.1.29

{
  "lockfileVersion": 1,
  "workspaces": {
    "": {
      "name": "my-app",
      "dependencies": {
        "ms": "^2.1.3",
      },
    },
  },
  "packages": {
    "ms": ["ms@2.1.3", "", { "dependencies": {} }, "sha512-abc"],
    "express": ["express@4.18.2", "", { "dependencies": { "ms": "2.1.3" } }, "sha512-def"],
  }
}
"#;

    #[test]
    fn jsonc_is_parsed_including_comments_and_trailing_commas() {
        let v: serde_json::Value = serde_json::from_str(&strip_jsonc(SAMPLE)).unwrap();
        assert_eq!(v["lockfileVersion"], 1);
        assert_eq!(v["packages"]["ms"][0], "ms@2.1.3");
    }

    #[test]
    fn detection_uses_the_header() {
        assert!(looks_like_bun_lock(SAMPLE));
        assert!(!looks_like_bun_lock(
            r#"{"lockfileVersion":3,"packages":{"node_modules/ms":{"version":"2.1.3"}}}"#
        ));
    }

    #[test]
    fn loads_versions_integrity_and_deps() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bun.lock");
        std::fs::write(&p, SAMPLE).unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"fallback"}"#).unwrap();

        let lock = load_from_bun_lock(&p).unwrap().unwrap();
        assert_eq!(lock.name, "my-app");
        assert_eq!(lock.dependencies["ms"].version, "2.1.3");
        assert_eq!(
            lock.dependencies["ms"].integrity.as_deref(),
            Some("sha512-abc")
        );
        assert_eq!(
            lock.dependencies["express"]
                .dependencies
                .as_ref()
                .unwrap()
                .get("ms")
                .map(String::as_str),
            Some("2.1.3")
        );
    }

    #[test]
    fn scoped_names_survive_the_rfind_split() {
        assert_eq!(
            split_resolved("@std/path@1.1.6"),
            Some(("@std/path".to_string(), "1.1.6".to_string()))
        );
    }

    #[test]
    fn a_file_without_packages_is_refused_not_invented() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bun.lock");
        std::fs::write(&p, r#"{"lockfileVersion":1,"workspaces":{}}"#).unwrap();
        assert!(load_from_bun_lock(&p).unwrap().is_none());
    }

    #[test]
    fn nonexistent_file_is_none() {
        assert!(
            load_from_bun_lock(std::path::Path::new("/nonexistent/bun.lock"))
                .unwrap()
                .is_none()
        );
    }
}
