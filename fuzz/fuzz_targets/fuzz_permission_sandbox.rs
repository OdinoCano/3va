#![no_main]

use libfuzzer_sys::fuzz_target;
use std::path::{Path, PathBuf};
use vvva_permissions::{Capability, PermissionState};
use vvva_permissions::sandbox::{VirtualFs, VirtualNetwork};

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };

    // ── VirtualFs: path-traversal invariant ────────────────────────────────
    // After a successful resolve(), the result must stay inside the mount source.
    let mut vfs = VirtualFs::new();
    vfs.mount("/app",  "/sandbox/app",  true);
    vfs.mount("/data", "/sandbox/data", false);

    if let Ok(resolved) = vfs.resolve(Path::new(s)) {
        let contained = resolved.starts_with("/sandbox/app")
            || resolved.starts_with("/sandbox/data");
        assert!(
            contained,
            "path-traversal escape: input={s:?} resolved={resolved:?}"
        );
    }

    // Confirm that the classic `/../` escape never resolves.
    let traversal = format!("/app/../{s}");
    if let Ok(resolved) = vfs.resolve(Path::new(&traversal)) {
        assert!(
            resolved.starts_with("/sandbox/app") || resolved.starts_with("/sandbox/data"),
            "traversal via /../: input={traversal:?} resolved={resolved:?}"
        );
    }

    // ── VirtualNetwork: wildcard matching never panics ──────────────────────
    let mut vnet = VirtualNetwork::new();
    vnet.allow_host("*.safe.example");
    vnet.allow_host("trusted.example");
    let _ = vnet.is_allowed(s);

    // ── PermissionState: deny_all overrides any grant ──────────────────────
    {
        let mut state = PermissionState::new();
        state.grant(Capability::FileRead(PathBuf::from("/")));
        state.grant(Capability::FileWrite(PathBuf::from("/")));
        state.deny_all_fs();

        assert!(
            !state.check(&Capability::FileRead(PathBuf::from(s))),
            "deny_all_fs must block FileRead"
        );
        assert!(
            !state.check(&Capability::FileWrite(PathBuf::from(s))),
            "deny_all_fs must block FileWrite"
        );
    }

    // ── PermissionState: explicit deny always overrides explicit grant ──────
    {
        let state = PermissionState::new();
        let cap = Capability::FileRead(PathBuf::from(s));
        state.grant(cap.clone());
        state.deny(cap.clone());
        assert!(
            !state.check(&cap),
            "deny must override grant for path {s:?}"
        );
    }

    {
        let state = PermissionState::new();
        let cap = Capability::Network(s.to_string());
        state.grant(cap.clone());
        state.deny(cap.clone());
        assert!(
            !state.check(&cap),
            "deny must override grant for host {s:?}"
        );
    }

    // ── PermissionState: deny_all_net overrides any network grant ──────────
    {
        let mut state = PermissionState::new();
        state.grant(Capability::Network("*".to_string()));
        state.deny_all_net();
        assert!(
            !state.check(&Capability::Network(s.to_string())),
            "deny_all_net must block Network for host {s:?}"
        );
    }

    // ── PermissionState: no grant → deny by default ─────────────────────────
    {
        let state = PermissionState::new();
        // Must never panic regardless of the fuzz input.
        let _ = state.check(&Capability::FileRead(PathBuf::from(s)));
        let _ = state.check(&Capability::FileWrite(PathBuf::from(s)));
        let _ = state.check(&Capability::Network(s.to_string()));
        let _ = state.check(&Capability::SpawnProcess);
        let _ = state.check(&Capability::EnvAccess);
    }
});
