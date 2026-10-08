// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Illustrative virtual filesystem / network mounting helpers.
//!
//! # ⚠️ This module is not a security boundary
//!
//! `VirtualFs` and `VirtualNetwork` are **not wired into the runtime** — no
//! production caller exists (verified by `rg VirtualFs`/`rg VirtualNetwork`):
//! only the security test suites, the `fuzz_permission_sandbox` target, and
//! this crate's re-export reference them. The enforcement path used by the
//! engine is [`crate::PermissionState::check`] together with
//! `vvva_js::builtins::secure_fs` (descriptor-pinned, TOCTOU-safe opens).
//!
//! `VirtualFs::resolve` does **lexical-only** normalization (`.`/`..`) and does
//! **not** canonicalize or follow symlinks. A symlink inside a mount
//! (`/app/escape -> /outside`) therefore yields `<source>/escape/...`, which
//! *looks* confined but reads/writes outside the mount when opened. This is
//! tracked as **VULN-ESCAPE-03**; it is documented rather than patched because
//! the type is dead code and hardening it would duplicate `secure_fs` without
//! protecting any real call path. Do not use it to gate I/O, and do not treat
//! its output as confinement.
//!
//! Kept for the tests/fuzz target that exercise the lexically-normalizing
//! behavior and as a reference design.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct MountPoint {
    pub source: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Default)]
pub struct VirtualFs {
    mounts: HashMap<PathBuf, MountPoint>,
}

impl VirtualFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mount<P: AsRef<Path>, S: AsRef<Path>>(
        &mut self,
        virtual_path: P,
        source: S,
        read_only: bool,
    ) {
        self.mounts.insert(
            virtual_path.as_ref().to_path_buf(),
            MountPoint {
                source: source.as_ref().to_path_buf(),
                read_only,
            },
        );
    }

    /// Maps a virtual path to a real path under a mount source.
    ///
    /// **Not a confinement check.** The result is derived from a lexical
    /// normalization only (see [`normalize_path`]); it is neither canonicalized
    /// nor symlink-resolved, so the real target may lie outside `source`. See
    /// the module-level note (VULN-ESCAPE-03).
    pub fn resolve(&self, path: &Path) -> Result<PathBuf, String> {
        let normalized = normalize_path(path);

        for (vp, mount) in &self.mounts {
            if let Ok(relative) = normalized.strip_prefix(vp) {
                let real = mount.source.join(relative);
                return Ok(real);
            }
        }
        Err("Path not mounted".to_string())
    }
}

/// Normalizes a path, resolving `.` and `..` without touching the filesystem.
///
/// Purely lexical: it neither canonicalizes nor resolves symlinks, so the
/// result is not guaranteed to stay inside any directory. See the module-level
/// note (VULN-ESCAPE-03).
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            _ => {
                normalized.push(component);
            }
        }
    }
    normalized
}

#[derive(Debug, Default)]
pub struct VirtualNetwork {
    allowed_hosts: HashSet<String>,
}

impl VirtualNetwork {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow_host(&mut self, host: &str) {
        self.allowed_hosts.insert(host.to_string());
    }

    /// Host-allowlist check. Not used by the runtime; the enforced equivalent is
    /// [`crate::PermissionState::check`] with `caps_match`/`host_matches`
    /// (which also honor ports). See the module-level note.
    pub fn is_allowed(&self, host: &str) -> bool {
        self.allowed_hosts.iter().any(|allowed| {
            if allowed == "*" {
                return true;
            }
            if allowed == host {
                return true;
            }
            if let Some(suffix) = allowed.strip_prefix("*.") {
                return host.ends_with(suffix)
                    && host.len() > suffix.len()
                    && host.as_bytes()[host.len() - suffix.len() - 1] == b'.';
            }
            false
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_normalization_traversal() {
        let path = Path::new("/app/config/../../etc/passwd");
        let normalized = normalize_path(path);
        let s = normalized.to_str().unwrap().replace('\\', "/");
        assert_eq!(s, "/etc/passwd");
    }

    #[test]
    fn curdir_components_are_dropped_by_normalize_path() {
        // `Path::components()` normalizes `.` away except at the very start,
        // so a leading `./` is the only way to exercise the CurDir arm.
        let normalized = normalize_path(Path::new("./app/x"));
        assert_eq!(normalized, PathBuf::from("app/x"));
    }

    #[test]
    fn test_virtual_fs_resolution() {
        let mut vfs = VirtualFs::new();
        vfs.mount("/app", "/var/lib/3va/sandbox1", true);

        // Valid resolution
        let resolved = vfs.resolve(Path::new("/app/config.json")).unwrap();
        let s = resolved.to_str().unwrap().replace('\\', "/");
        assert_eq!(s, "/var/lib/3va/sandbox1/config.json");

        // Path traversal attempt gets normalized to stay within bounds or errors if out
        // /app/../etc/passwd -> /etc/passwd -> not starts with /app -> error
        let error = vfs.resolve(Path::new("/app/../etc/passwd"));
        assert!(error.is_err());
        assert_eq!(error.unwrap_err(), "Path not mounted");
    }

    #[test]
    fn test_virtual_network_allow() {
        let mut vnet = VirtualNetwork::new();
        vnet.allow_host("api.github.com");
        vnet.allow_host("*.google.com");

        assert!(vnet.is_allowed("api.github.com"));
        assert!(!vnet.is_allowed("github.com"));

        assert!(vnet.is_allowed("maps.google.com"));
        assert!(vnet.is_allowed("api.maps.google.com"));
        assert!(!vnet.is_allowed("google.com"));
        assert!(!vnet.is_allowed("evildomain.com"));
    }
}
