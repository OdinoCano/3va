// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Native async work must keep the event loop alive.
//!
//! With no timers pending, `run_event_loop` neither slept nor treated in-flight
//! native work as "unlimited", so it spun through its bounded iteration cap in
//! microseconds and returned before a 60 ms+ work item (e.g. `bcrypt.hash` at
//! cost 10) completed: the script exited without ever running the callback.

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;

    use vvva_js::JsEngine;
    use vvva_permissions::{Capability, PermissionState};

    fn build_addon() -> Option<PathBuf> {
        let dir = tempfile::TempDir::new().unwrap();
        let out = dir.path().join("napi_async.node");
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("napi_async.c");
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

    /// Long enough that an event loop spinning through its iteration cap gives up
    /// first, even in a slow debug build.
    const DELAY_MS: u32 = 3000;

    #[tokio::test]
    async fn event_loop_waits_for_slow_native_async_work() {
        let Some(addon) = build_addon() else {
            eprintln!("skipping event_loop_waits_for_slow_native_async_work: no C compiler");
            return;
        };
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::FFI(addon.clone()));
        let mut engine = JsEngine::new(perms).await.unwrap();

        engine
            .eval_to_string(&format!(
                "globalThis.result = 'pending'; \
                 require({:?}).slow({}).then(function (v) {{ globalThis.result = String(v); }}); 'ok'",
                addon.to_string_lossy(),
                DELAY_MS
            ))
            .await
            .unwrap();

        // No timers, no listeners: the only pending work is the native item.
        engine.run_event_loop().await.unwrap();

        assert_eq!(
            engine.eval_to_string("globalThis.result").await.unwrap(),
            "42",
            "the event loop returned before the native async work completed"
        );
    }
}
