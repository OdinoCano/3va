// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Native `import()` from *classic* scripts: 3va has no v8::Module, so
// `JsEngine` installs a host `HostImportModuleDynamically` callback that
// delegates to the same `__importAsync`/`require()` path the transpiled ESM
// route uses. Before this hook existed, `import()` in a script threw
// `Error: Not supported` (V8 had no module loader to call). These fixtures are
// plain `.js` (ESM syntax detected by content) because requiring a `.mjs`
// still hits the intentional `ERR_REQUIRE_ESM` guard.
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
