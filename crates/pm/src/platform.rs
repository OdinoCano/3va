// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Platform detection for `optionalDependencies`.
//!
//! npm's convention is that a package may declare prebuilt, per-platform
//! binaries under `optionalDependencies` (esbuild's `@esbuild/linux-x64`,
//! rollup's `@rollup/rollup-linux-x64-gnu`, lightningcss, swc, …). Those are
//! plain data — a `.node` file or a prebuilt binary that is *executed*, never
//! a lifecycle script — so installing them opens no new capability that
//! `3va run --allow-ffi` does not already gate. Skipping them, as 3va did
//! until now, is what broke every native toolchain on the platform.
//!
//! Each candidate's own `os` / `cpu` / `libc` fields decide whether it applies
//! to the machine doing the install, using npm's own matching rules:
//!
//! * every listed value must match (`os: ["linux", "freebsd"]` is a union),
//! * `!` negates (`os: ["!win32"]` means "any OS but win32"),
//! * an empty list or a missing field means "any",
//! * `libc` is only consulted on Linux, where glibc and musl are the only two
//!   ABI-incompatible worlds that matter.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub cpu: String,
    pub libc: String,
}

impl Platform {
    /// The platform 3va itself was compiled for and is running on.
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            cpu: rust_arch_to_node_arch(std::env::consts::ARCH).to_string(),
            libc: detect_libc(),
        }
    }

    /// For tests: an arbitrary triple.
    pub fn new(os: &str, cpu: &str, libc: &str) -> Self {
        Self {
            os: os.to_string(),
            cpu: cpu.to_string(),
            libc: libc.to_string(),
        }
    }

    fn value_for(&self, field: &str) -> &str {
        match field {
            "os" => &self.os,
            "cpu" => &self.cpu,
            "libc" => &self.libc,
            _ => "",
        }
    }
}

/// Node's `process.arch` spelling for a Rust target arch. npm ecosystem
/// packages spell this `x64`/`arm64`, not `x86_64`/`aarch64`.
pub fn rust_arch_to_node_arch(arch: &str) -> &str {
    match arch {
        "x86_64" | "x86" => "x64",
        "aarch64" => "arm64",
        "arm" => "arm",
        "x86_64_gnu" | "x86_64-musl" => "x64",
        other => other,
    }
}

/// glibc or musl, detected without spawning a process.
///
/// `ldd --version` would be authoritative but costs a fork on every install;
/// the presence of the musl loader in the standard library directories is
/// equally reliable in practice — glibc systems ship `ld-linux-*.so` and no
/// `ld-musl-*.so.1`, Alpine/musl systems are the other way around.
pub fn detect_libc() -> String {
    if cfg!(target_os = "macos") {
        return "darwin".to_string();
    }
    if cfg!(target_os = "windows") {
        return "win32".to_string();
    }
    for dir in ["/lib", "/lib64", "/usr/lib", "/usr/lib64", "/usr/local/lib"] {
        let Ok(entries) = std::fs::read_dir(Path::new(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("ld-musl-") && name.ends_with(".so.1") {
                return "musl".to_string();
            }
        }
    }
    "glibc".to_string()
}

/// Does `value` (one entry of an `os`/`cpu`/`libc` array) select `platform`?
fn value_matches(value: &str, field: &str, platform: &Platform) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if let Some(negated) = value.strip_prefix('!') {
        // `!win32` matches everything that is not win32. `libc` negation is
        // only meaningful on Linux, where we can actually tell the two apart.
        if field == "libc" && platform.os != "linux" {
            return true;
        }
        return platform.value_for(field) != negated.trim();
    }
    platform.value_for(field) == value
}

/// Does a single `os`/`cpu`/`libc` field (string or array) select `platform`?
pub fn field_accepts(field: &str, value: &serde_json::Value, platform: &Platform) -> bool {
    // A field that is present but empty ("os": []) means "any" in npm.
    let items: Vec<&str> = match value {
        serde_json::Value::String(s) => vec![s.as_str()],
        serde_json::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
        _ => return true,
    };
    if items.is_empty() {
        return true;
    }
    if field == "libc" && platform.os != "linux" {
        // Non-Linux targets have no meaningful libc value; only an explicit
        // positive libc constraint (rare) can rule one out.
        return !items.iter().any(|v| !v.starts_with('!') && *v == "musl");
    }
    // A field is a union: every listed value is an accepted alternative, so a
    // single match selects the package.
    items.iter().any(|v| value_matches(v, field, platform))
}

/// Does the whole manifest's `os`/`cpu`/`libc` triple select `platform`?
/// All three must agree; any field may be missing.
pub fn manifest_accepts(manifest: &serde_json::Value, platform: &Platform) -> bool {
    for field in ["os", "cpu", "libc"] {
        if let Some(v) = manifest.get(field)
            && !field_accepts(field, v, platform)
        {
            return false;
        }
    }
    true
}

/// The candidate `optionalDependencies` declared by a package version.
///
/// This is only the *candidate* list: a parent manifest names its optional
/// dependencies and their ranges, but the `os`/`cpu`/`libc` filter that decides
/// whether `@esbuild/darwin-arm64` may be fetched lives in the *candidate's own*
/// registry metadata, which is not available yet. Callers resolve each candidate
/// and then run [`optional_dep_applies`] on the version object they got back.
pub fn optional_dep_candidates(version_meta: &serde_json::Value) -> Vec<(String, String)> {
    let Some(obj) = version_meta
        .get("optionalDependencies")
        .and_then(|v| v.as_object())
    else {
        return Vec::new();
    };
    obj.iter()
        .filter_map(|(name, range)| {
            let range = range.as_str()?;
            // A null/false-y range is not installable at all.
            if range.is_empty() || range.contains("false") {
                return None;
            }
            Some((name.clone(), range.to_string()))
        })
        .collect()
}

/// Does a resolved optional dependency's own metadata select `platform`?
///
/// A prebuilt binary that declares a foreign `os` or `cpu` is not merely
/// unnecessary here, it is wrong: installing it can pull a Darwin build into a
/// Linux tree. Missing metadata means "no constraint", so it applies.
pub fn optional_dep_applies(candidate_meta: &serde_json::Value, platform: &Platform) -> bool {
    manifest_accepts(candidate_meta, platform)
}

/// Keep the candidates that apply, given each candidate's resolved metadata.
///
/// A candidate with no metadata at all is kept — we cannot prove it does not
/// apply, and silently dropping an unresolvable optional dependency would
/// produce a broken install.
pub fn filter_optional_deps(
    candidates: &[(String, String)],
    metadata: &dyn Fn(&str) -> Option<serde_json::Value>,
    platform: &Platform,
) -> Vec<(String, String)> {
    candidates
        .iter()
        .filter(|(name, _)| match metadata(name) {
            Some(meta) => optional_dep_applies(&meta, platform),
            None => true,
        })
        .cloned()
        .collect()
}

/// Version objects in an npm packument, paired with their names, so callers
/// can filter with [`optional_dep_applies`] without re-walking the JSON.
pub fn version_objects(packument: &serde_json::Value) -> Vec<(&str, &serde_json::Value)> {
    packument
        .get("versions")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.as_str(), v)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arch_names_match_npm_spelling() {
        assert_eq!(rust_arch_to_node_arch("x86_64"), "x64");
        assert_eq!(rust_arch_to_node_arch("aarch64"), "arm64");
        assert_eq!(rust_arch_to_node_arch("arm"), "arm");
    }

    #[test]
    fn current_platform_is_self_consistent() {
        let p = Platform::current();
        assert!(!p.os.is_empty());
        assert!(!p.cpu.is_empty());
        assert!(!p.libc.is_empty());
        // Rust spells the arch differently from npm; the mapping must have run.
        assert_ne!(p.cpu, std::env::consts::ARCH);
    }

    #[test]
    fn os_and_cpu_must_both_match() {
        let p = Platform::new("linux", "x64", "glibc");
        assert!(manifest_accepts(
            &json!({"os": ["linux"], "cpu": ["x64"]}),
            &p
        ));
        assert!(!manifest_accepts(&json!({"os": ["darwin"]}), &p));
        assert!(!manifest_accepts(&json!({"cpu": ["arm64"]}), &p));
    }

    #[test]
    fn missing_or_empty_fields_mean_any() {
        let p = Platform::new("linux", "x64", "glibc");
        assert!(manifest_accepts(&json!({}), &p));
        assert!(manifest_accepts(&json!({"os": []}), &p));
    }

    #[test]
    fn os_is_a_union_of_alternatives() {
        let p = Platform::new("freebsd", "x64", "glibc");
        assert!(manifest_accepts(&json!({"os": ["darwin", "freebsd"]}), &p));
    }

    #[test]
    fn bang_negates() {
        let p = Platform::new("linux", "x64", "glibc");
        assert!(manifest_accepts(&json!({"os": ["!win32"]}), &p));
        assert!(!manifest_accepts(&json!({"os": ["!linux"]}), &p));
    }

    #[test]
    fn libc_only_rules_on_linux() {
        let linux_musl = Platform::new("linux", "x64", "musl");
        let linux_glibc = Platform::new("linux", "x64", "glibc");
        let darwin = Platform::new("darwin", "arm64", "darwin");

        assert!(manifest_accepts(&json!({"libc": ["musl"]}), &linux_musl));
        assert!(!manifest_accepts(&json!({"libc": ["musl"]}), &linux_glibc));
        assert!(manifest_accepts(&json!({"libc": ["glibc"]}), &linux_glibc));

        // A musl-only binary is meaningless on macOS: refuse it everywhere.
        assert!(!manifest_accepts(&json!({"libc": ["musl"]}), &darwin));
        // ...but "any libc" is fine.
        assert!(manifest_accepts(&json!({"libc": []}), &darwin));
    }

    #[test]
    fn version_objects_lists_every_published_version() {
        let pack = json!({"versions": {"1.0.0": {"os": ["linux"]}, "2.0.0": {}}});
        let v = version_objects(&pack);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, "1.0.0");
    }
}
