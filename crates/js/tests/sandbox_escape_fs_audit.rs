// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Regresiones de la auditoría de `std::fs::*` (mismo patrón que VULN-ESCAPE-01/02).
//!
//! `__localStorageRead`/`__localStorageSave` son globales que el script puede
//! invocar y tocaban el disco sin comprobar `Capability::FileRead/FileWrite`.
//! Con un sandbox vacío (`PermissionState::new()`) deben denegar, no operar.

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn eval_catching(engine: &mut JsEngine, js_call: &str) -> String {
    let code = format!(
        "(() => {{ try {{ {js_call}; return 'allowed'; }} \
         catch(e) {{ return 'denied:' + (e.message || String(e)); }} }})()"
    );
    engine
        .eval_to_string(&code)
        .await
        .unwrap_or_else(|e| format!("error:{e}"))
}

#[tokio::test]
async fn local_storage_read_requires_file_read() {
    let mut engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    let r = eval_catching(&mut engine, "__localStorageRead()").await;
    assert!(
        r.starts_with("denied:"),
        "__localStorageRead leyó sin Capability::FileRead: {r}"
    );
}

#[tokio::test]
async fn local_storage_write_requires_file_write() {
    let mut engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    let r = eval_catching(&mut engine, "__localStorageSave('{\"x\":1}')").await;
    assert!(
        r.starts_with("denied:"),
        "__localStorageSave escribió sin Capability::FileWrite: {r}"
    );
}
