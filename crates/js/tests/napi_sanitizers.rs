// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Node-API hardening tests.
//!
//! `napi.rs` implements the N-API C ABI that a native `.node` addon talks to.
//! The addon is native code, but the host still must not exhibit UB for
//! malformed inputs — a buggy (not necessarily malicious) addon should get a
//! clean `napi_invalid_arg` rather than a memory error.
//!
//! `tests/fixtures/napi_hostile.c` is compiled at test time into a `.node`
//! addon that feeds the API invalid UTF-8. Under ASan (`-Zsanitizer=address`)
//! the same addon is compiled with `-fsanitize=address` so any out-of-bounds
//! read the host performs while trusting the addon is reported.

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;

    use vvva_js::JsEngine;
    use vvva_permissions::{Capability, PermissionState};

    /// Compile the hostile addon into a temporary directory. Returns `None`
    /// (and the test skips) when no C compiler is available.
    fn build_addon() -> Option<PathBuf> {
        let dir = tempfile::TempDir::new().unwrap();
        let out = dir.path().join("napi_hostile.node");
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("napi_hostile.c");

        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let mut cmd = Command::new(cc);
        cmd.arg("-shared")
            .arg("-fPIC")
            .arg("-O1")
            .arg("-o")
            .arg(&out)
            .arg(&src);
        let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            return None;
        }

        let out = out.canonicalize().ok()?;
        // The addon must outlive the TempDir guard; leaking it is fine in a test.
        std::mem::forget(dir);
        Some(out)
    }

    #[tokio::test]
    async fn napi_rejects_invalid_utf8_from_addon() {
        let Some(addon) = build_addon() else {
            eprintln!("skipping napi_rejects_invalid_utf8_from_addon: no C compiler");
            return;
        };

        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::FFI(addon.clone()));
        let mut engine = JsEngine::new(perms).await.unwrap();

        let js = format!(
            "const a = require({:?}); a.string_status + ',' + a.class_status",
            addon.to_string_lossy()
        );
        let result = engine
            .eval_to_string(&js)
            .await
            .expect("hostile addon load should not crash the engine");

        // napi_invalid_arg == 1 for both entry points. Before validation this
        // was 0 (the invalid bytes were accepted as a string).
        assert_eq!(
            result, "1,1",
            "invalid UTF-8 from an addon must be rejected with napi_invalid_arg"
        );
    }

    /// ASan probe for the second suspicion: an addon that claims a `len` far
    /// larger than the buffer it passes. This is a contract violation by the
    /// addon (the host cannot know the allocation size), so it is documented
    /// rather than "fixed"; the test exists to show what the sanitizer sees.
    ///
    /// Run under ASan with:
    /// `RUSTFLAGS="-Zsanitizer=address -C link-args=-rdynamic" cargo +nightly
    ///  test -p vvva_js --test napi_sanitizers --target
    ///  x86_64-unknown-linux-gnu -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "ASan probe: needs -Zsanitizer=address to observe the overflow"]
    async fn napi_attacker_length_overruns_addon_buffer() {
        // SAFETY: this ignored test owns the process; nothing else reads it.
        unsafe { std::env::set_var("NAPI_HOSTILE_OOB", "1") };
        let addon = build_addon().expect("a C compiler is required for this probe");
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::FFI(addon.clone()));
        let mut engine = JsEngine::new(perms).await.unwrap();
        let js = format!("require({:?}); 'loaded'", addon.to_string_lossy());
        // Under ASan this aborts with heap-buffer-overflow before returning.
        let _ = engine.eval_to_string(&js).await;
    }
}
