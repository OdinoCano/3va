// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Node-API handle lifetime.
//!
//! Every `napi_create_*` stores a strong V8 root plus a heap box. They used to be
//! kept until the process exited (handle scopes were no-ops), so an addon that
//! created values in a loop grew without bound. Handles must be released when the
//! native callback returns and when an explicit handle scope closes, and a value
//! escaped from an escapable scope must survive that scope.
//!
//! One test function on purpose: `live_handle_count` is process-wide, so
//! concurrent tests would see each other's handles.

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;

    use vvva_js::JsEngine;
    use vvva_js::builtins::napi::live_handle_count;
    use vvva_permissions::{Capability, PermissionState};

    fn build_addon() -> Option<PathBuf> {
        let dir = tempfile::TempDir::new().unwrap();
        let out = dir.path().join("napi_handles.node");
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("napi_handles.c");
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let ok = Command::new(cc)
            .args(["-shared", "-fPIC", "-O1", "-o"])
            .arg(&out)
            .arg(&src)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return None;
        }
        let out = out.canonicalize().ok()?;
        std::mem::forget(dir);
        Some(out)
    }

    #[tokio::test]
    async fn handles_are_released_by_callbacks_and_scopes() {
        let Some(addon) = build_addon() else {
            eprintln!("skipping handles_are_released_by_callbacks_and_scopes: no C compiler");
            return;
        };
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::FFI(addon.clone()));
        let mut engine = JsEngine::new(perms).await.unwrap();
        engine
            .eval_to_string(&format!(
                "globalThis.a = require({:?}); 'ok'",
                addon.to_string_lossy()
            ))
            .await
            .unwrap();
        let base = live_handle_count();

        // Implicit scope of a native callback.
        engine.eval_to_string("a.churn(10000); 'ok'").await.unwrap();
        assert_eq!(live_handle_count(), base, "callback left handles behind");

        // Explicit handle scopes.
        engine
            .eval_to_string("a.scoped(10000); 'ok'")
            .await
            .unwrap();
        assert_eq!(
            live_handle_count(),
            base,
            "closed handle scopes left handles behind"
        );

        // The escaped value must still be usable after its scope closed.
        let escaped = engine.eval_to_string("a.escape()").await.unwrap();
        assert_eq!(
            escaped, "escaped",
            "escaped handle did not survive its scope"
        );
        assert_eq!(
            live_handle_count(),
            base,
            "escapable scope left handles behind"
        );
    }
}
