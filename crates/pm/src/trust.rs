// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Per-package trust decisions, declared in the project's own `package.json`.
//!
//! The scanner grades packages by their contents, which means a package whose
//! *job* is to spawn a compiler, shell out to a platform tool or evaluate
//! generated code gets graded like malware. 3va's answer is not to loosen the
//! rules — it is to let the project pin the exact artifact it has already
//! reviewed, so the decision is auditable and expires by itself:
//!
//! ```json
//! {
//!   "3va": {
//!     "trusted": ["vite@8.3.0", "@swc/core@1.7.0"],
//!     "trustedIntegrity": { "vite@8.3.0": "sha512-…" },
//!     "onlyBuiltDependencies": ["better-sqlite3", "sharp", "esbuild"]
//!   }
//! }
//! ```
//!
//! A pin is `name` or `name@version`. Without a version the pin never expires,
//! so the loader records that and the install says so out loud. With a version
//! it stops applying the moment the lockfile moves — bumping `vite` re-scans
//! it, which is the whole point. An `integrity` hash additionally pins the
//! bytes, so a registry re-publishing the same version cannot inherit the
//! decision.
//!
//! `onlyBuiltDependencies` is 3va's name for pnpm's and Bun's build-script
//! allowlist. It is opt-in per package, and it is the *only* thing that lets
//! `preinstall`/`install`/`postinstall` run — the default remains that they
//! never do, and `3va doctor --compat` reports every package that ships one.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustEntry {
    pub name: String,
    /// `None` means "any version" — permitted, but reported as unversioned.
    pub version: Option<String>,
}

impl TrustEntry {
    pub fn matches(&self, name: &str, version: &str) -> bool {
        if self.name != name {
            return false;
        }
        match &self.version {
            None => true,
            Some(v) => semver_equal(v, version),
        }
    }
}

/// `1.2.3` and `v1.2.3` and `=1.2.3` are the same artifact; `1.2` and `1.2.x`
/// are ranges and never match a pin.
fn semver_equal(pin: &str, version: &str) -> bool {
    let clean = |s: &str| {
        s.trim()
            .trim_start_matches('=')
            .trim_start_matches('v')
            .to_string()
    };
    clean(pin) == clean(version)
}

#[derive(Debug, Default, Clone)]
pub struct TrustPolicy {
    entries: Vec<TrustEntry>,
    /// package name → expected tarball integrity (sha512-…).
    integrity: BTreeMap<String, String>,
    /// Packages whose lifecycle scripts are allowed to run.
    build_allowed: Vec<String>,
}

impl TrustPolicy {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Read the `"3va"` block of a manifest. A malformed block is a hard error
    /// for the caller: silently treating a typo'd trust list as empty would
    /// quietly block every install instead of loudly rejecting the config.
    pub fn from_manifest(manifest: &Value) -> anyhow::Result<Self> {
        let mut policy = Self::default();
        let Some(block) = manifest.get("3va") else {
            return Ok(policy);
        };
        if block.is_null() {
            return Ok(policy);
        }
        let obj = block
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("package.json: \"3va\" must be an object"))?;

        if let Some(list) = obj.get("trusted") {
            policy.entries = parse_trust_list(list)?;
        }
        if let Some(map) = obj.get("trustedIntegrity") {
            let map = map.as_object().ok_or_else(|| {
                anyhow::anyhow!("package.json: \"3va.trustedIntegrity\" must be an object")
            })?;
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    policy.integrity.insert(k.clone(), s.to_string());
                }
            }
        }
        for key in ["onlyBuiltDependencies", "trustedDependencies"] {
            if let Some(list) = obj.get(key) {
                policy.build_allowed.extend(parse_name_list(list, key)?);
            }
        }
        // Bun's `trustedDependencies` lives at the top level.
        for key in ["trustedDependencies", "onlyBuiltDependencies"] {
            if let Some(list) = manifest.get(key) {
                policy.build_allowed.extend(parse_name_list(list, key)?);
            }
        }
        policy.build_allowed.sort();
        policy.build_allowed.dedup();
        Ok(policy)
    }

    /// Load the policy from `<root>/package.json`, or an empty one when there
    /// is no manifest.
    pub fn from_project(root: &Path) -> anyhow::Result<Self> {
        let path = root.join("package.json");
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Ok(Self::empty());
        };
        let manifest: Value = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Invalid package.json: {e}"))?;
        Self::from_manifest(&manifest)
    }

    pub fn entries(&self) -> &[TrustEntry] {
        &self.entries
    }

    /// Is `name@version` pinned by the project? An entry with no version is
    /// returned as `Unversioned` so the caller can warn about it.
    pub fn status(&self, name: &str, version: &str, integrity: Option<&str>) -> TrustStatus {
        let Some(entry) = self.entries.iter().find(|e| e.matches(name, version)) else {
            return TrustStatus::NotTrusted;
        };
        if let Some(expected) = self.integrity.get(&entry_key(entry)) {
            match integrity {
                // No hash from the registry: the pin cannot be honoured, so it
                // does not apply. Failing open here would let a registry drop
                // the integrity field and inherit a reviewed decision.
                None => return TrustStatus::IntegrityUnknown,
                Some(actual) if integrity_eq(expected, actual) => {}
                Some(_) => return TrustStatus::IntegrityMismatch,
            }
        }
        match &entry.version {
            Some(_) => TrustStatus::Trusted,
            None => TrustStatus::Unversioned,
        }
    }

    pub fn is_trusted(&self, name: &str, version: &str, integrity: Option<&str>) -> bool {
        matches!(
            self.status(name, version, integrity),
            TrustStatus::Trusted | TrustStatus::Unversioned
        )
    }

    /// Is this package allowed to run lifecycle scripts?
    pub fn allows_lifecycle(&self, name: &str) -> bool {
        self.build_allowed.iter().any(|n| n == name)
    }

    pub fn build_allowed(&self) -> &[String] {
        &self.build_allowed
    }

    /// The trust policy for a project, or an explicitly empty one.
    ///
    /// A malformed `"3va"` block must not become a crash in the middle of an
    /// install, but it also must not become an empty policy that silently
    /// allows nothing and looks like a deliberate decision.
    pub fn from_manifest_or_empty(manifest: Option<&Value>) -> Self {
        match manifest {
            Some(m) => Self::from_manifest(m).unwrap_or_else(|e| {
                eprintln!("! ignoring the \"3va\" block in package.json: {e}");
                Self::empty()
            }),
            None => Self::empty(),
        }
    }

    /// Flags for re-entering `3va run` on a lifecycle script, derived from the
    /// package's own declared permissions.
    ///
    /// The script runs with exactly what its manifest asked for and nothing
    /// more. A downloader that needs the network still has to have declared
    /// `allowNet`, so opting into a lifecycle script does not silently hand it
    /// the machine.
    pub fn sandbox_argv(&self, package_dir: &Path) -> Vec<String> {
        let mut argv = Vec::new();
        let manifest: Value = std::fs::read_to_string(package_dir.join("package.json"))
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or(Value::Null);
        let perms = &manifest["3va"]["permissions"];
        if let Some(list) = perms["allowNet"].as_array() {
            for v in list.iter().filter_map(|v| v.as_str()) {
                argv.push(format!("--allow-net={v}"));
            }
        }
        for (flag, key) in [
            ("--allow-fs-write", "allowFsWrite"),
            ("--allow-env", "allowEnv"),
            ("--allow-ffi", "allowFfi"),
            ("--allow-child-process", "allowChildProcess"),
        ] {
            if let Some(list) = perms[key].as_array() {
                for v in list.iter().filter_map(|v| v.as_str()) {
                    argv.push(format!("{flag}={v}"));
                }
            } else if perms[key].as_bool() == Some(true) {
                argv.push(flag.to_string());
            }
        }
        argv
    }
}

fn entry_key(entry: &TrustEntry) -> String {
    match &entry.version {
        Some(v) => format!("{}@{}", entry.name, v),
        None => entry.name.clone(),
    }
}

/// Integrity strings can differ only in algorithm or in whitespace; compare the
/// meaningful part so `sha512-abc` matches a registry's `sha512-abc  `.
fn integrity_eq(a: &str, b: &str) -> bool {
    match (a.split_whitespace().next(), b.split_whitespace().next()) {
        (Some(x), Some(y)) => x == y && !x.is_empty(),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustStatus {
    NotTrusted,
    /// Pinned to this exact version and (if given) this exact integrity.
    Trusted,
    /// Pinned by name with no version — applies forever, including to whatever
    /// that name resolves to next week.
    Unversioned,
    /// A pin exists but the registry served different bytes than recorded.
    IntegrityMismatch,
    /// A pin with an integrity hash, but the registry published no hash.
    IntegrityUnknown,
}

fn parse_trust_list(value: &Value) -> anyhow::Result<Vec<TrustEntry>> {
    let items = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("package.json: \"3va.trusted\" must be an array"))?;
    let mut out = Vec::new();
    for item in items {
        let s = item.as_str().ok_or_else(|| {
            anyhow::anyhow!("package.json: \"3va.trusted\" entries must be strings")
        })?;
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        // Split on the last '@' so "@scope/pkg@1.2.3" keeps its scope.
        match s.rfind('@').filter(|i| *i > 0) {
            Some(i) => {
                let (name, version) = (&s[..i], &s[i + 1..]);
                if version.is_empty() {
                    out.push(TrustEntry {
                        name: name.to_string(),
                        version: None,
                    });
                } else {
                    out.push(TrustEntry {
                        name: name.to_string(),
                        version: Some(version.to_string()),
                    });
                }
            }
            None => out.push(TrustEntry {
                name: s.to_string(),
                version: None,
            }),
        }
    }
    Ok(out)
}

fn parse_name_list(value: &Value, key: &str) -> anyhow::Result<Vec<String>> {
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in items {
        match item.as_str() {
            Some(s) if !s.trim().is_empty() => out.push(s.trim().to_string()),
            Some(_) => {}
            None => {
                return Err(anyhow::anyhow!(
                    "package.json: \"{key}\" entries must be strings"
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_absent_3va_block_means_no_pins() {
        let p = TrustPolicy::from_manifest(&json!({})).unwrap();
        assert!(p.entries().is_empty());
        assert!(!p.is_trusted("vite", "8.3.0", None));
    }

    #[test]
    fn a_version_pin_expires_when_the_version_moves() {
        let p = TrustPolicy::from_manifest(&json!({"3va": {"trusted": ["vite@8.3.0"]}})).unwrap();
        assert!(p.is_trusted("vite", "8.3.0", None));
        // The whole point: bumping the version re-scans it.
        assert!(!p.is_trusted("vite", "8.4.0", None));
    }

    #[test]
    fn scoped_pins_split_on_the_last_at() {
        let p =
            TrustPolicy::from_manifest(&json!({"3va": {"trusted": ["@swc/core@1.7.0"]}})).unwrap();
        assert!(p.is_trusted("@swc/core", "1.7.0", None));
        assert!(!p.is_trusted("@swc/core", "1.7.1", None));
        assert!(!p.is_trusted("@swc/other", "1.7.0", None));
    }

    #[test]
    fn a_bare_name_pins_every_version_and_says_so() {
        let p = TrustPolicy::from_manifest(&json!({"3va": {"trusted": ["rolldown"]}})).unwrap();
        assert_eq!(
            p.status("rolldown", "0.1.0", None),
            TrustStatus::Unversioned
        );
        assert_eq!(
            p.status("rolldown", "9.9.9", None),
            TrustStatus::Unversioned
        );
    }

    #[test]
    fn integrity_pins_the_bytes_not_just_the_version() {
        let p = TrustPolicy::from_manifest(&json!({
            "3va": {
                "trusted": ["vite@8.3.0"],
                "trustedIntegrity": {"vite@8.3.0": "sha512-AAAA"}
            }
        }))
        .unwrap();
        assert_eq!(
            p.status("vite", "8.3.0", Some("sha512-AAAA")),
            TrustStatus::Trusted
        );
        assert_eq!(
            p.status("vite", "8.3.0", Some("sha512-BBBB")),
            TrustStatus::IntegrityMismatch
        );
        // Fail closed when the registry gives us nothing to compare.
        assert_eq!(
            p.status("vite", "8.3.0", None),
            TrustStatus::IntegrityUnknown
        );
    }

    #[test]
    fn build_allowlist_is_read_from_every_supported_spelling() {
        let p = TrustPolicy::from_manifest(&json!({
            "3va": {"onlyBuiltDependencies": ["sharp", "better-sqlite3"]},
            "trustedDependencies": ["bcrypt"]
        }))
        .unwrap();
        assert!(p.allows_lifecycle("sharp"));
        assert!(p.allows_lifecycle("better-sqlite3"));
        assert!(p.allows_lifecycle("bcrypt"));
        assert!(!p.allows_lifecycle("left-pad"));
    }

    #[test]
    fn sandbox_argv_comes_from_the_scripts_own_declared_permissions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{
              "3va": {
                "permissions": {
                  "allowNet": ["api.example.com"],
                  "allowFsWrite": ["./build"],
                  "allowEnv": ["NODE_ENV"],
                  "allowFfi": true
                }
              }
            }"#,
        )
        .unwrap();
        let policy = TrustPolicy::empty();
        let argv = policy.sandbox_argv(dir.path());
        assert!(argv.contains(&"--allow-net=api.example.com".to_string()));
        assert!(argv.contains(&"--allow-fs-write=./build".to_string()));
        assert!(argv.contains(&"--allow-env=NODE_ENV".to_string()));
        assert!(argv.contains(&"--allow-ffi".to_string()));
    }

    #[test]
    fn a_script_with_no_declared_permissions_gets_no_flags() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"x"}"#).unwrap();
        assert!(TrustPolicy::empty().sandbox_argv(dir.path()).is_empty());
    }

    #[test]
    fn a_malformed_block_degrades_to_empty_with_a_warning_not_a_crash() {
        // Installing must not abort because of a typo in a manifest field.
        let policy = TrustPolicy::from_manifest_or_empty(Some(&serde_json::json!({
            "3va": { "trusted": "not-a-list" }
        })));
        assert!(policy.entries().is_empty());
        assert!(
            TrustPolicy::from_manifest_or_empty(None)
                .entries()
                .is_empty()
        );
    }

    #[test]
    fn a_malformed_trust_block_is_an_error_not_an_empty_policy() {
        let err = TrustPolicy::from_manifest(&json!({"3va": {"trusted": "vite"}})).unwrap_err();
        assert!(err.to_string().contains("must be an array"));
        let err = TrustPolicy::from_manifest(&json!({"3va": 5})).unwrap_err();
        assert!(err.to_string().contains("must be an object"));
    }

    #[test]
    fn from_project_survives_a_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let p = TrustPolicy::from_project(dir.path()).unwrap();
        assert!(p.entries().is_empty());
    }

    #[test]
    fn from_project_reads_the_real_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"3va":{"trusted":["vite@8.3.0"]}}"#,
        )
        .unwrap();
        let p = TrustPolicy::from_project(dir.path()).unwrap();
        assert!(p.is_trusted("vite", "8.3.0", None));
    }
}
