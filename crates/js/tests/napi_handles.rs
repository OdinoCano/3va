// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Node-API handle and finalizer lifetime.
//!
//! Every `napi_create_*` stores a strong V8 root plus a heap box. They used to be
//! kept until the process exited (handle scopes were no-ops), so an addon that
//! created values in a loop grew without bound. Handles must be released when the
//! native callback returns and when an explicit handle scope closes, and a value
//! escaped from an escapable scope must survive that scope.
//!
//! The same lifetime rules apply to the two allocations Node-API attaches to a
//! JS object: the `napi_wrap` finalizer and the per-function `NapiBridge`. A
//! finalizer must run once, on the JS thread, when the object is collected or
//! when the environment is torn down — never inside the GC itself — and
//! `napi_remove_wrap` must cancel it without running it.
//!
//! One test function on purpose: `live_handle_count`/`live_bridge_count` are
//! process-wide, so concurrent tests would see each other's handles.

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use vvva_js::JsEngine;
    use vvva_js::builtins::napi::{live_bridge_count, live_handle_count};
    use vvva_permissions::{Capability, PermissionState};

    /// Built addons must outlive their temp dir (dlopen keeps the file mapped),
    /// so hold the guards in a static instead of leaking them: LeakSanitizer
    /// then sees the allocations as reachable and reports nothing of ours.
    static ADDON_DIRS: Mutex<Vec<tempfile::TempDir>> = Mutex::new(Vec::new());

    fn build_addon(file: &str, out_name: &str) -> Option<PathBuf> {
        let dir = tempfile::TempDir::new().unwrap();
        let out = dir.path().join(out_name);
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(file);
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
        let path = out.canonicalize().ok()?;
        ADDON_DIRS.lock().unwrap().push(dir);
        Some(path)
    }

    /// A synchronous full GC. `request_garbage_collection_for_testing` runs the
    /// weak callbacks before it returns, so the deferred finalizers are queued
    /// right after; a GC is still not guaranteed to collect everything, so the
    /// callers loop.
    async fn force_gc(engine: &mut JsEngine) {
        engine
            .with_scope(|scope, _| {
                scope.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
            })
            .await;
    }

    async fn gc_and_drain(engine: &mut JsEngine) {
        for _ in 0..5 {
            force_gc(engine).await;
            vvva_js::builtins::napi::drain_async_completions();
        }
    }

    async fn counter(engine: &mut JsEngine, expr: &str) -> i32 {
        engine.eval_to_string(expr).await.unwrap().parse().unwrap()
    }

    #[tokio::test]
    async fn handles_and_finalizers_are_released() {
        // `request_garbage_collection_for_testing` is gated behind V8's
        // `--expose-gc`; set it before the isolate is created so the test can
        // force a deterministic full GC.
        v8::V8::set_flags_from_string("--expose-gc");
        let Some(addon) = build_addon("napi_handles.c", "napi_handles.node") else {
            eprintln!("skipping handles_and_finalizers_are_released: no C compiler");
            return;
        };
        let Some(fin) = build_addon("napi_finalizers.c", "napi_finalizers.node") else {
            eprintln!("skipping handles_and_finalizers_are_released: no C compiler");
            return;
        };
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::FFI(addon.clone()));
        perms.grant(Capability::FFI(fin.clone()));
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

        // ── napi_wrap finalizers ───────────────────────────────────────────
        engine
            .eval_to_string(&format!(
                "globalThis.f = require({:?}); 'ok'",
                fin.to_string_lossy()
            ))
            .await
            .unwrap();

        // A wrapped object that only the native callback referenced is collected;
        // its finalizer must then run.
        let before = counter(&mut engine, "f.wrapCounter()").await;
        engine.eval_to_string("f.wrapOnce(); 'ok'").await.unwrap();
        gc_and_drain(&mut engine).await;
        assert_eq!(
            counter(&mut engine, "f.wrapCounter()").await,
            before + 1,
            "napi_wrap finalizer did not run after the object was collected"
        );

        // napi_remove_wrap detaches without running the finalizer.
        engine
            .eval_to_string("f.wrapAndRemove(); 'ok'")
            .await
            .unwrap();
        gc_and_drain(&mut engine).await;
        assert_eq!(
            counter(&mut engine, "f.wrapCounter()").await,
            before + 1,
            "napi_remove_wrap must cancel the finalizer"
        );

        // A second napi_wrap on the same object is napi_invalid_arg (1).
        assert_eq!(
            engine.eval_to_string("f.doubleWrapStatus()").await.unwrap(),
            "1",
            "napi_wrap on an already-wrapped object must return napi_invalid_arg"
        );

        // ── per-function bridge ────────────────────────────────────────────
        let bridge_base = live_bridge_count();
        engine
            .eval_to_string("f.churnFunctions(100000); 'ok'")
            .await
            .unwrap();
        gc_and_drain(&mut engine).await;
        let bridge_after = live_bridge_count();
        assert!(
            bridge_after <= bridge_base + 32,
            "creating and dropping 100000 functions leaked bridges: \
             before={bridge_base} after={bridge_after}"
        );

        // ── environment teardown ───────────────────────────────────────────
        // A second engine shares the addon library (same path -> same loaded
        // image), so its finalizer counters are visible through the first
        // engine's functions even after it is dropped.
        let wrap_before_teardown = counter(&mut engine, "f.wrapCounter()").await;
        let inst_before = counter(&mut engine, "f.instanceCounter()").await;
        {
            let perms = Arc::new(PermissionState::new());
            perms.grant(Capability::FFI(fin.clone()));
            let mut engine2 = JsEngine::new(perms).await.unwrap();
            engine2
                .eval_to_string(&format!(
                    "const f2 = require({:?}); \
                     globalThis.kept = f2.wrapKept(); f2.setInstance(); 'ok'",
                    fin.to_string_lossy()
                ))
                .await
                .unwrap();
            // Dropping the engine tears the environment down: the still-live
            // wrapped object and the instance data finalizer must both run once.
            drop(engine2);
        }
        assert_eq!(
            counter(&mut engine, "f.wrapCounter()").await,
            wrap_before_teardown + 1,
            "teardown must run the finalizer of a still-live wrapped object exactly once"
        );
        assert_eq!(
            counter(&mut engine, "f.instanceCounter()").await,
            inst_before + 1,
            "teardown must run the napi_set_instance_data finalizer exactly once"
        );

        // ── a finalizer that calls back into JS during teardown ───────────
        // The called function is addon-created, so it is backed by a NapiBridge.
        // Teardown must run addon finalizers before freeing the bridges: the other
        // order reads a freed bridge here (a heap-use-after-free under ASan; in a
        // normal build it may still appear to work, so the counter is only a
        // functional check and the ASan job is what catches the regression).
        let runs_before = counter(&mut engine, "f.callbackRuns()").await;
        {
            let perms = Arc::new(PermissionState::new());
            perms.grant(Capability::FFI(fin.clone()));
            let mut engine3 = JsEngine::new(perms).await.unwrap();
            engine3
                .eval_to_string(&format!(
                    "const f3 = require({:?}); globalThis.kept3 = f3.wrapCallsJs(); 'ok'",
                    fin.to_string_lossy()
                ))
                .await
                .unwrap();
            drop(engine3);
        }
        assert_eq!(
            counter(&mut engine, "f.callbackRuns()").await,
            runs_before + 1,
            "a finalizer calling an addon function during teardown must run it once"
        );
    }
}
