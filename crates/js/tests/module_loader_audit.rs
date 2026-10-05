// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Module-loader audit regressions:
//
//   - requiring a (sandboxed) module must never write outside the grants.
//     The transpiler used to dump transpiled sources to fixed `/tmp/...`
//     paths whenever the source merely *contained* a marker string. A
//     planted symlink at one of those paths turned a `require()` of an
//     attacker-authored module into an arbitrary-file overwrite (and the
//     world-readable dumps leaked dependency source to other local users).
//
// Run: cargo test -p vvva_js --test module_loader_audit

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

/// Performs a symlink-plant + require and returns whether the symlink's
/// target was left untouched (the safe outcome).
#[cfg(unix)]
async fn victim_survives_require(
    marker: &str,
    file_name: &str,
    tmp_path: &str,
    source: &str,
) -> bool {
    let tmp = tempfile::tempdir().unwrap();
    let sandbox = tmp.path();

    let victim = sandbox.join("victim.txt");
    std::fs::write(&victim, marker).unwrap();

    // Both the dump path and the required module live in /tmp so that the
    // module's directory is not itself the sandbox (we still only grant the
    // sandbox below).
    let _ = std::fs::remove_file(tmp_path);
    std::os::unix::fs::symlink(&victim, tmp_path).unwrap();

    let module = sandbox.join(file_name);
    std::fs::write(&module, source).unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(sandbox.to_path_buf()));
    let mut engine = JsEngine::new(state).await.unwrap();

    let dir = sandbox.to_string_lossy().replace('\\', "/");
    let module_path = module.to_string_lossy().replace('\\', "/");
    engine
        .eval_to_string(&format!(
            "globalThis.__dirname = {dir:?}; require({module_path:?});"
        ))
        .await
        .unwrap();

    let after = std::fs::read_to_string(&victim).unwrap_or_default();
    let _ = std::fs::remove_file(tmp_path);
    after == marker
}

/// The transpiler's `static_esm_to_cjs` dump fires on any ESM source that
/// contains `createContentTypesGenerator` and `export {`.
#[cfg(unix)]
#[tokio::test]
async fn require_does_not_overwrite_through_tmp_symlink_transpiler() {
    let survived = victim_survives_require(
        "SENTINEL-TRANSPILER",
        "evil.js",
        "/tmp/tg_sesmtocjs_input.js",
        "export const createContentTypesGenerator = 1;\nexport { createContentTypesGenerator };\n",
    )
    .await;
    assert!(
        survived,
        "require() overwrote an arbitrary file via /tmp/tg_sesmtocjs_input.js"
    );
}

/// The module loader itself dumps a source to `/tmp/zod_core_transpiled.js`
/// when its path contains both `zod` and `core.js`.
#[cfg(unix)]
#[tokio::test]
async fn require_does_not_overwrite_through_tmp_symlink_modules() {
    let survived = victim_survives_require(
        "SENTINEL-MODULES",
        "zodcore.js",
        "/tmp/zod_core_transpiled.js",
        "module.exports = { value: 1 };\n",
    )
    .await;
    assert!(
        survived,
        "require() overwrote an arbitrary file via /tmp/zod_core_transpiled.js"
    );
}

// ── The loader must not read outside the grants ──────────────────────────────

/// `require()` of an out-of-grant module must fail and must not return the
/// file's exports. `how` builds the specifier from the outside file's path.
async fn require_outside_is_denied(how: impl FnOnce(&str) -> String) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let sandbox = tmp.path().join("sandbox");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.js");
    std::fs::write(&secret, "module.exports = { leaked: 'TOPSECRET' };\n").unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(sandbox.clone()));
    let mut engine = JsEngine::new(state).await.unwrap();

    let dir = sandbox.to_string_lossy().replace('\\', "/");
    let secret_path = secret.to_string_lossy().replace('\\', "/");
    let specifier = how(&secret_path);
    engine
        .eval_to_string(&format!(
            "globalThis.__dirname = {dir:?}; \
             var out = 'loaded'; \
             try {{ var m = require({specifier:?}); out = m && m.leaked ? 'LEAKED:' + m.leaked : 'loaded'; }} \
             catch (e) {{ out = 'denied:' + (e.message || String(e)); }} out"
        ))
        .await
        .unwrap()
}

#[tokio::test]
async fn require_absolute_outside_grant_is_denied() {
    let out = require_outside_is_denied(|p| p.to_string()).await;
    assert!(!out.contains("LEAKED"), "module content leaked: {out}");
    assert!(out.starts_with("denied:"), "expected denial, got: {out}");
}

#[tokio::test]
async fn require_relative_traversal_outside_grant_is_denied() {
    let tmp = tempfile::tempdir().unwrap();
    let sandbox = tmp.path().join("sandbox");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("secret.js"),
        "module.exports = { leaked: 'TOPSECRET' };\n",
    )
    .unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(sandbox.clone()));
    let mut engine = JsEngine::new(state).await.unwrap();

    let dir = sandbox.to_string_lossy().replace('\\', "/");
    let out = engine
        .eval_to_string(&format!(
            "globalThis.__dirname = {dir:?}; \
             var out = 'loaded'; \
             try {{ var m = require('../outside/secret.js'); out = m && m.leaked ? 'LEAKED' : 'loaded'; }} \
             catch (e) {{ out = 'denied:' + (e.message || String(e)); }} out"
        ))
        .await
        .unwrap();
    assert!(!out.contains("LEAKED"), "module content leaked: {out}");
    assert!(out.starts_with("denied:"), "expected denial, got: {out}");
}

#[tokio::test]
async fn require_file_url_outside_grant_is_denied() {
    let out = require_outside_is_denied(|p| format!("file://{}", p)).await;
    assert!(!out.contains("LEAKED"), "module content leaked: {out}");
    assert!(out.starts_with("denied:"), "expected denial, got: {out}");
}

#[cfg(unix)]
#[tokio::test]
async fn require_symlink_escaping_grant_is_denied() {
    let tmp = tempfile::tempdir().unwrap();
    let sandbox = tmp.path().join("sandbox");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.js");
    std::fs::write(&secret, "module.exports = { leaked: 'TOPSECRET' };\n").unwrap();
    std::os::unix::fs::symlink(&secret, sandbox.join("link.js")).unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(sandbox.clone()));
    let mut engine = JsEngine::new(state).await.unwrap();

    let dir = sandbox.to_string_lossy().replace('\\', "/");
    let out = engine
        .eval_to_string(&format!(
            "globalThis.__dirname = {dir:?}; \
             var out = 'loaded'; \
             try {{ var m = require('./link.js'); out = m && m.leaked ? 'LEAKED' : 'loaded'; }} \
             catch (e) {{ out = 'denied:' + (e.message || String(e)); }} out"
        ))
        .await
        .unwrap();
    assert!(!out.contains("LEAKED"), "symlink escape leaked: {out}");
    assert!(out.starts_with("denied:"), "expected denial, got: {out}");
}

#[tokio::test]
async fn require_inside_grant_still_works() {
    let tmp = tempfile::tempdir().unwrap();
    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::write(sandbox.join("ok.js"), "module.exports = { ok: 7 };\n").unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(sandbox.clone()));
    let mut engine = JsEngine::new(state).await.unwrap();

    let dir = sandbox.to_string_lossy().replace('\\', "/");
    let out = engine
        .eval_to_string(&format!(
            "globalThis.__dirname = {dir:?}; var m = require('./ok.js'); String(m.ok === 7)"
        ))
        .await
        .unwrap();
    assert_eq!(out, "true");
}
