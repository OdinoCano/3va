use crate::lockfile::{Lockfile, LockfileDep, LockfilePackage};
use serde::Deserialize;
use std::collections::HashMap;

// ── npm package-lock.json v1 format ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NpmLockV1 {
    name: String,
    version: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    lockfile_version: Option<u32>,
    #[serde(default)]
    dependencies: HashMap<String, NpmDepV1>,
}

#[derive(Debug, Deserialize)]
struct NpmDepV1 {
    version: String,
    resolved: Option<String>,
    integrity: Option<String>,
    dev: Option<bool>,
    #[serde(default)]
    dependencies: Option<HashMap<String, String>>,
    #[serde(default)]
    requires: Option<HashMap<String, String>>,
}

// ── npm package-lock.json v2/v3 format ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NpmLockV2 {
    name: String,
    version: Option<String>,
    lockfile_version: u32,
    #[serde(default)]
    packages: HashMap<String, NpmPackageV2>,
    #[serde(default)]
    dependencies: HashMap<String, NpmDepV2>,
}

#[derive(Debug, Deserialize)]
struct NpmPackageV2 {
    version: Option<String>,
    resolved: Option<String>,
    integrity: Option<String>,
    dev: Option<bool>,
    #[serde(default)]
    dependencies: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct NpmDepV2 {
    version: String,
    resolved: Option<String>,
    integrity: Option<String>,
    dev: Option<bool>,
    #[serde(default)]
    dependencies: Option<HashMap<String, String>>,
    #[serde(default)]
    requires: Option<HashMap<String, String>>,
}

// ── Public API ─────────────────────────────────────────────────────────────────

/// Load a lockfile from npm's `package-lock.json` format (v1, v2, or v3).
///
/// Detects the version automatically from the `lockfileVersion` field.
/// Returns `None` if the file doesn't exist or isn't a valid npm lockfile.
pub fn load_from_package_lock(path: &std::path::Path) -> anyhow::Result<Option<Lockfile>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Cannot read {}: {}", path.display(), e))?;

    // Peek at lockfileVersion to decide parser
    let version_hint: Option<u32> = serde_json::from_str::<serde_json::Value>(&content)
        .ok()
        .and_then(|v| v["lockfileVersion"].as_u64().map(|n| n as u32));

    match version_hint {
        Some(1) => parse_v1(content),
        Some(2) | Some(3) => parse_v2v3(content, version_hint.unwrap()),
        _ => {
            // Try v1 (no lockfileVersion field) as fallback
            parse_v1(content)
        }
    }
    .map(Some)
}

fn parse_v1(content: String) -> anyhow::Result<Lockfile> {
    let npm: NpmLockV1 = serde_json::from_str(&content)?;
    let mut packages = HashMap::new();
    let mut dependencies = HashMap::new();

    for (name, dep) in &npm.dependencies {
        let deps = dep.requires.as_ref().or(dep.dependencies.as_ref()).cloned();

        dependencies.insert(
            name.clone(),
            LockfileDep {
                version: dep.version.clone(),
                resolved: dep.resolved.clone(),
                integrity: dep.integrity.clone(),
                dependencies: deps,
                dev: dep.dev,
                registry: None,
            },
        );

        packages.insert(
            format!("node_modules/{}", name),
            LockfilePackage {
                version: dep.version.clone(),
                resolved: dep.resolved.clone(),
                integrity: dep.integrity.clone(),
                dev: dep.dev,
                registry: None,
            },
        );
    }

    packages.insert(
        String::new(),
        LockfilePackage {
            version: npm.version.clone().unwrap_or_else(|| "0.0.0".to_string()),
            resolved: None,
            integrity: None,
            dev: None,
            registry: None,
        },
    );

    Ok(Lockfile {
        lockfile_version: 1,
        name: npm.name,
        version: npm.version.unwrap_or_else(|| "0.0.0".to_string()),
        packages,
        dependencies,
    })
}

fn parse_v2v3(content: String, _version: u32) -> anyhow::Result<Lockfile> {
    let npm: NpmLockV2 = serde_json::from_str(&content)?;
    let mut packages = HashMap::new();
    let mut dependencies = HashMap::new();

    // Convert packages (v2/v3 key format: "node_modules/pkgname")
    for (key, pkg) in &npm.packages {
        if key.is_empty() {
            packages.insert(
                key.clone(),
                LockfilePackage {
                    version: pkg.version.clone().unwrap_or_else(|| "0.0.0".to_string()),
                    resolved: pkg.resolved.clone(),
                    integrity: pkg.integrity.clone(),
                    dev: pkg.dev,
                    registry: None,
                },
            );
        } else {
            // Extract package name from key like "node_modules/@scope/pkg" or "node_modules/pkg"
            let pkg_name = key.strip_prefix("node_modules/").unwrap_or(key).to_string();

            packages.insert(
                key.clone(),
                LockfilePackage {
                    version: pkg.version.clone().unwrap_or_else(|| "unknown".to_string()),
                    resolved: pkg.resolved.clone(),
                    integrity: pkg.integrity.clone(),
                    dev: pkg.dev,
                    registry: None,
                },
            );

            // Only add top-level packages to dependencies
            if !pkg_name.contains('/') || pkg_name.starts_with('@') {
                let pkg_name_only = pkg_name;
                if !dependencies.contains_key(&pkg_name_only)
                    && let Some(ver) = &pkg.version
                {
                    dependencies.insert(
                        pkg_name_only,
                        LockfileDep {
                            version: ver.clone(),
                            resolved: pkg.resolved.clone(),
                            integrity: pkg.integrity.clone(),
                            dependencies: pkg.dependencies.clone(),
                            dev: pkg.dev,
                            registry: None,
                        },
                    );
                }
            }
        }
    }

    // Also populate from the dependencies block for top-level dep info
    for (name, dep) in &npm.dependencies {
        let deps = dep.requires.as_ref().or(dep.dependencies.as_ref()).cloned();

        dependencies.insert(
            name.clone(),
            LockfileDep {
                version: dep.version.clone(),
                resolved: dep.resolved.clone(),
                integrity: dep.integrity.clone(),
                dependencies: deps,
                dev: dep.dev,
                registry: None,
            },
        );
    }

    Ok(Lockfile {
        lockfile_version: npm.lockfile_version,
        name: npm.name,
        version: npm.version.unwrap_or_else(|| "0.0.0".to_string()),
        packages,
        dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_package_lock_v1() {
        let content = r#"{
            "name": "test-project",
            "version": "1.0.0",
            "lockfileVersion": 1,
            "dependencies": {
                "lodash": {
                    "version": "4.17.21",
                    "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
                    "integrity": "sha512-v2kDEe57lecTulaDIuNTPy3Ry4gLGJ6Z1O3vE1krgXZNrsQ+LFTGHVxVjcXPs17LhbZVGedAJv8XZ1tvj5FvSg=="
                },
                "express": {
                    "version": "4.18.2",
                    "resolved": "https://registry.npmjs.org/express/-/express-4.18.2.tgz",
                    "integrity": "sha512-5/PsL6iGPdfQ/lKM1UuielYgv3BUoJfz1aUwU9vj3S3Hp8S1cB6pNqB6fH1oHj2W4NSQHs4rLwPfsfA==",
                    "requires": {
                        "accepts": "1.3.8"
                    }
                }
            }
        }"#;

        let result = parse_v1(content.to_string()).unwrap();
        assert_eq!(result.name, "test-project");
        assert_eq!(result.lockfile_version, 1);
        assert!(result.dependencies.contains_key("lodash"));
        assert!(result.dependencies.contains_key("express"));
        assert_eq!(result.dependencies["lodash"].version, "4.17.21");
        assert_eq!(
            result.dependencies["express"]
                .dependencies
                .as_ref()
                .unwrap()["accepts"],
            "1.3.8"
        );
    }

    #[test]
    fn parse_package_lock_v3() {
        let content = r#"{
            "name": "test-project",
            "version": "1.0.0",
            "lockfileVersion": 3,
            "packages": {
                "": {
                    "name": "test-project",
                    "version": "1.0.0"
                },
                "node_modules/axios": {
                    "version": "1.7.9",
                    "resolved": "https://registry.npmjs.org/axios/-/axios-1.7.9.tgz",
                    "integrity": "sha512-test",
                    "dev": false
                }
            },
            "dependencies": {
                "axios": {
                    "version": "1.7.9",
                    "resolved": "https://registry.npmjs.org/axios/-/axios-1.7.9.tgz",
                    "integrity": "sha512-test"
                }
            }
        }"#;

        let result = parse_v2v3(content.to_string(), 3).unwrap();
        assert_eq!(result.name, "test-project");
        assert_eq!(result.lockfile_version, 3);
        assert!(result.dependencies.contains_key("axios"));
        assert_eq!(result.dependencies["axios"].version, "1.7.9");
        assert!(result.packages.contains_key("node_modules/axios"));
    }

    #[test]
    fn load_from_package_lock_nonexistent() {
        let result =
            load_from_package_lock(std::path::Path::new("/nonexistent/package-lock.json")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_from_package_lock_v1_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("package-lock.json");
        std::fs::write(&path, r#"{"name":"p","version":"1.0.0","lockfileVersion":1,"dependencies":{"chalk":{"version":"5.3.0"}}}"#).unwrap();

        let result = load_from_package_lock(&path).unwrap();
        assert!(result.is_some());
        let lock = result.unwrap();
        assert!(lock.dependencies.contains_key("chalk"));
        assert_eq!(lock.dependencies["chalk"].version, "5.3.0");
    }
}
