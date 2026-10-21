// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Native `import()` from *classic* scripts: 3va has no v8::Module, so
// `JsEngine` installs a host `HostImportModuleDynamically` callback that
// delegates to the same `__importAsync`/`require()` path the transpiled ESM
// route uses. Before this hook existed, `import()` in a script threw
// `Error: Not supported` (V8 had no module loader to call). These fixtures are
// plain `.js` (ESM syntax detected by content) and `.mjs` (Node 22+ also lets
// require() load those; a graph with top-level await still needs import()).
//
// Run: cargo test -p vvva_js --test esm_dynamic_import

use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/esm_dynamic_import")
}

/// Runs `code` as a classic script with `__dirname` pointed at the fixture
/// directory, then pumps the loop until it sets `globalThis.__out`.
async fn run(code: &str) -> String {
    let dir = fixtures();
    let state = PermissionState::new();
    state.grant(Capability::FileRead(dir.clone()));
    let mut e = JsEngine::new(Arc::new(state)).await.unwrap();
    let prelude = format!(
        "globalThis.__dirname = {:?};",
        dir.to_string_lossy().to_string()
    );
    e.eval(&prelude).await.unwrap();
    e.eval(code).await.unwrap();
    for _ in 0..200 {
        e.pump_timers().await.unwrap();
        e.idle().await;
        let out = e.eval_to_string("String(globalThis.__out)").await.unwrap();
        if out != "undefined" {
            return out;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("classic-script import() never settled");
}

#[tokio::test]
async fn import_from_classic_script_loads_mjs() {
    let out = run("globalThis.__out = 'pending';\n\
         import('./x.mjs').then(m => { globalThis.__out = m.value + ':' + m.default; });\n")
    .await;
    assert_eq!(out, "mjs:x");
}

#[tokio::test]
async fn import_from_classic_script_waits_for_mjs_tla() {
    let out = run("globalThis.__out = 'pending';\n\
         import('./tla.mjs').then(m => { globalThis.__out = m.before + ':' + m.after; });\n")
    .await;
    assert_eq!(out, "1:2");
}

#[tokio::test]
async fn import_from_classic_script_resolves_named_and_default() {
    let out = run("globalThis.__out = 'pending';\n\
         import('./dep.js').then(m => { globalThis.__out = m.x + ':' + m.default; });\n")
    .await;
    assert_eq!(out, "41:dep");
}

#[tokio::test]
async fn import_from_classic_script_stringifies_specifier_and_nests() {
    let out = run("globalThis.__out = 'pending';\n\
         const spec = { toString() { return './nested.js'; } };\n\
         import(spec).then(m => { globalThis.__out = String(m.x); });\n")
    .await;
    assert_eq!(out, "41");
}

#[tokio::test]
async fn import_from_classic_script_waits_for_tla_dependency() {
    let out = run("globalThis.__out = 'pending';\n\
         import('./tla.js').then(m => { globalThis.__out = m.late; });\n\
         globalThis.__finishDep();\n")
    .await;
    assert_eq!(out, "late");
}

#[tokio::test]
async fn import_from_classic_script_rejects_for_missing_module() {
    let out = run("globalThis.__out = 'pending';\n\
         import('./does-not-exist.js').then(\n\
           () => { globalThis.__out = 'resolved'; },\n\
           e => { globalThis.__out = 'rejected'; });\n")
    .await;
    assert_eq!(out, "rejected");
}

/// A required module's own `import('./x')` resolves from *its* directory, not
/// the entry's: the host callback uses the importing script's path (the
/// resource name), so two modules in different directories each find their own
/// sibling.
#[tokio::test]
async fn import_in_a_required_module_resolves_from_that_modules_directory() {
    let temp = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(temp.path().join("sub")).unwrap();
    std::fs::write(
        temp.path().join("sub/lib.js"),
        "exports.dyn = async () => (await import('./esm.mjs')).v;\n",
    )
    .unwrap();
    std::fs::write(temp.path().join("sub/esm.mjs"), "export const v = 99;\n").unwrap();
    std::fs::write(
        temp.path().join("entry.js"),
        "require('./sub/lib.js').dyn().then(v => { globalThis.__out = String(v); });\n",
    )
    .unwrap();

    let state = PermissionState::new();
    state.grant(Capability::FileRead(temp.path().to_path_buf()));
    let mut e = JsEngine::new(Arc::new(state)).await.unwrap();
    e.eval_file(&temp.path().join("entry.js")).await.unwrap();
    for _ in 0..200 {
        e.pump_timers().await.unwrap();
        e.idle().await;
        let out = e.eval_to_string("String(globalThis.__out)").await.unwrap();
        if out != "undefined" {
            assert_eq!(out, "99");
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("dynamic import() in a required module never settled");
}
