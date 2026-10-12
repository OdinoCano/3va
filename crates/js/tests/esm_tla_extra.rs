// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Extra top-level-await / ESM-export regressions found while fixing the
// test262 `module-code/top-level-await` failures:
//   * a module whose body is *only* top-level await (no import/export) must
//     still run as an async module, not as a classic script;
//   * `export const { … } = …` (destructuring) must re-export the bound names;
//   * `export default class extends Base {}` (anonymous) must export the class,
//     not a bogus `module.exports.default = extends`.
//
// Run: cargo test -p vvva_js --test esm_tla_extra

use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

fn fixtures() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/esm_tla_extra")
}

/// Evaluates the fixture entry module, then polls until `probe` (a JS
/// expression) is no longer `undefined`.
async fn run_entry(name: &str, probe: &str) -> String {
    let dir = fixtures();
    let state = PermissionState::new();
    state.grant(Capability::FileRead(dir.clone()));
    let mut e = JsEngine::new(Arc::new(state)).await.unwrap();
    e.eval_file(&dir.join(name)).await.unwrap();
    for _ in 0..200 {
        e.pump_timers().await.unwrap();
        e.idle().await;
        let out = e.eval_to_string(&format!("String({probe})")).await.unwrap();
        if out != "undefined" {
            return out;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("entry never finished");
}

#[tokio::test]
async fn destructuring_and_anonymous_default_class_exports() {
    let out = run_entry("entry_extra.mjs", "JSON.stringify(globalThis.__out)").await;
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["a"], 1);
    assert_eq!(v["c"], 2); // renamed binding `b: c`
    assert_eq!(v["d"], 5); // default initializer
    assert_eq!(v["x"], 10);
    assert_eq!(v["rest"], serde_json::json!([30, 40]));
    assert_eq!(v["anonBase"], true); // anonymous `export default class extends Base`
}

#[tokio::test]
async fn module_with_only_top_level_await_runs_as_a_module() {
    let out = run_entry("only_tla.mjs", "globalThis.__tlaOnly").await;
    assert_eq!(out, "after");
}
