// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Integration tests for the module-resolution fixes:
//
//   Bug 1 — __fallbackModules unreachable: stubs for packages not in
//            node_modules (debug, ms, proxy-from-env, tr46) now activate
//            even when __resolvePath throws MODULE_NOT_FOUND.
//
//   Bug 3 — require() of a .mjs file: Node 22+ loads it (transpiled to CJS)
//            and returns its exports; only a graph with top-level await
//            throws ERR_REQUIRE_ASYNC_MODULE.
//
// Run: cargo test -p vvva_js --test module_resolution_fixes

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

/// Engine that can read files anywhere under /tmp (for temp-file tests).
async fn engine_with_tmp_read() -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::FileRead(std::env::temp_dir()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

// ── Bug 1: __fallbackModules reachable when package not installed ─────────────

/// Confirm the mechanism works end-to-end: a stub registered in
/// __fallbackModules is returned even if __resolvePath would throw.
#[tokio::test]
async fn fallback_module_returned_when_package_not_in_node_modules() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            // Register a stub for a package that does not exist on disk.
            globalThis.__fallbackModules['__test_nonexistent_pkg__'] = { answer: 42 };
            var m = require('__test_nonexistent_pkg__');
            String(m.answer === 42)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

/// A real installed package must shadow the fallback stub.
/// We use 'path' which is always in __requireCache (built-in), so the fallback
/// must NOT override it.
#[tokio::test]
async fn builtin_shadows_fallback_stub() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            // Try to override path with a stub — the real built-in must win.
            globalThis.__fallbackModules['path'] = { fake: true };
            var p = require('path');
            String(typeof p.join === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

/// The fallback value is returned as-is (function, object, primitive).
#[tokio::test]
async fn fallback_module_can_be_a_function() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            globalThis.__fallbackModules['__test_fn_pkg__'] = function ns() { return 99; };
            var fn_ = require('__test_fn_pkg__');
            String(typeof fn_ === 'function' && fn_() === 99)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── Bug 3: require() of .mjs files ───────────────────────────────────────────

/// require() of an absolute .mjs path loads the module and returns its exports,
/// as Node 22+ does.
#[tokio::test]
async fn require_mjs_loads_its_exports() {
    let mut e = engine_with_tmp_read().await;

    let tmp = std::env::temp_dir().join("_3va_test_esm_mod.mjs");
    std::fs::write(&tmp, "export const value = 42;\nexport default 'esm';\n").unwrap();
    let path = tmp.to_string_lossy().to_string();

    let script = format!(
        r#"
        var code = 'no-throw';
        try {{
            var m = require({:?});
            code = String(m.value) + ':' + String(m.default);
        }} catch(e) {{
            code = e.code || 'no-code';
        }}
        code
        "#,
        path
    );
    let r = e.eval_to_string(&script).await.unwrap();
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(r, "42:esm");
}

/// require() cannot synchronously evaluate a graph with top-level await:
/// Node throws ERR_REQUIRE_ASYNC_MODULE, and import() must be used instead.
#[tokio::test]
async fn require_mjs_with_top_level_await_throws_err_require_async_module() {
    let mut e = engine_with_tmp_read().await;

    let tmp = std::env::temp_dir().join("_3va_test_esm_tla_mod.mjs");
    std::fs::write(
        &tmp,
        "export const before = 1;\nawait Promise.resolve();\nexport const after = 2;\n",
    )
    .unwrap();
    let path = tmp.to_string_lossy().to_string();

    let script = format!(
        r#"
        var code = 'no-throw';
        try {{
            require({:?});
        }} catch(e) {{
            code = e.code || 'no-code';
        }}
        code
        "#,
        path
    );
    let r = e.eval_to_string(&script).await.unwrap();
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(r, "ERR_REQUIRE_ASYNC_MODULE");
}

/// A .cjs file is CommonJS no matter what it contains: it must never be
/// routed through the ES-module loader.
#[tokio::test]
async fn require_cjs_extension_never_throws_err_require_esm() {
    let mut e = engine_with_tmp_read().await;

    let tmp = std::env::temp_dir().join("_3va_test_cjs_mod.cjs");
    std::fs::write(&tmp, "module.exports = { ok: true };\n").unwrap();
    let path = tmp.to_string_lossy().to_string();

    let script = format!(
        r#"
        var code = 'no-throw';
        try {{
            var m = require({:?});
            code = String(m.ok === true ? 'loaded' : 'wrong-value');
        }} catch(e) {{
            code = e.code || e.message;
        }}
        code
        "#,
        path
    );
    let r = e.eval_to_string(&script).await.unwrap();
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(r, "loaded");
}

/// Ordinary .js files must still load normally (no false ERR_REQUIRE_ESM).
#[tokio::test]
async fn require_js_cjs_file_loads_normally() {
    let mut e = engine_with_tmp_read().await;

    let tmp = std::env::temp_dir().join("_3va_test_plain_cjs.js");
    std::fs::write(&tmp, "module.exports = { value: 7 };\n").unwrap();
    let path = tmp.to_string_lossy().to_string();

    let script = format!(
        r#"
        var m = require({:?});
        String(m.value === 7)
        "#,
        path
    );
    let r = e.eval_to_string(&script).await.unwrap();
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(r, "true");
}
