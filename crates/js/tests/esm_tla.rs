// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Top-level await in ESM modules: `await` at module top level must actually wait
// (it used to be stripped to a fire-and-forget call, leaving `const x = await f()`
// as a Promise). Expected values are what Node prints for the same fixtures.
//
// Run: cargo test -p vvva_js --test esm_tla

use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/esm_tla")
}

async fn run_entry(name: &str) -> String {
    let dir = fixtures();
    let state = PermissionState::new();
    state.grant(Capability::FileRead(dir.clone()));
    let mut e = JsEngine::new(Arc::new(state)).await.unwrap();
    e.eval_file(&dir.join(name)).await.unwrap();
    for _ in 0..200 {
        e.pump_timers().await.unwrap();
        e.idle().await;
        let out = e.eval_to_string("String(globalThis.__out)").await.unwrap();
        if out != "undefined" {
            return out;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("entry never finished: top-level await did not resolve");
}

#[tokio::test]
async fn top_level_await_waits_in_imported_and_entry_modules() {
    let out = run_entry("entry.mjs").await;
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["eol"], "\n"); // db.js: const os = await import(...); export const eol = os.EOL
    assert_eq!(v["sep"], "/"); // await import() inside an exported function
    assert_eq!(v["ok"], "string"); // try { x = await import() } catch {}
    assert_eq!(v["v"], 42); // export default v after `const v = await ...`
    assert_eq!(v["r"], "hi"); // await import(`./${name}.js`)
    assert_eq!(v["out"], serde_json::json!([2, 4, 9])); // await inside for / if
    assert_eq!(v["res"], serde_json::json!([1, 2])); // for await at top level
    assert_eq!(v["k"], serde_json::json!({"k": "v"})); // export default await ...
    assert_eq!(v["again"], "string"); // TLA module imported by another module
    assert_eq!(v["a"], 1); // import attributes: with { type: "json" }
    assert_eq!(v["x"], 5);
    assert_eq!(v["q"], 1); // export + await import() in functions only (no TLA)
    assert_eq!(v["dynW"], "late"); // import() waits for a timer-paced TLA module
}

#[tokio::test]
async fn importer_of_pending_tla_module_settles_after_it_and_dup_attr_keys_rejected() {
    let v: serde_json::Value = serde_json::from_str(&run_entry("order_entry.mjs").await).unwrap();
    assert_eq!(v["logs"], serde_json::json!(["B", "A"])); // A depends on B's pending TLA
    assert_eq!(v["dup"], "SyntaxError"); // duplicate `with` keys stay an early error
}

#[tokio::test]
async fn entry_with_only_top_level_await_is_run_as_a_module() {
    assert_eq!(run_entry("tla_only.js").await, "5"); // no import/export, just `await`
}
