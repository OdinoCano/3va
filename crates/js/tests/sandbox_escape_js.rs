// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Regresiones de escape del sandbox desde JS (VULN-ESCAPE-01/02, corregidas).
//!
//! 01: el transpilador/cargador ya no escribe volcados de depuración en `/tmp`.
//! 02: `__workerCreate` exige `Capability::FileRead` antes de leer el archivo.

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

#[cfg(unix)]
#[tokio::test]
async fn transpiler_does_not_write_to_tmp() {
    const TMP_TARGET: &str = "/tmp/cloudflare_plugin_out.js";
    let _ = std::fs::remove_file(TMP_TARGET);

    let tmp = tempfile::tempdir().unwrap();
    let victim = tmp.path().join("victim.txt");
    std::fs::write(&victim, "ORIGINAL-DATA\n").unwrap();
    std::os::unix::fs::symlink(&victim, TMP_TARGET).unwrap();

    let entry = tmp.path().join("evil.mjs");
    std::fs::write(&entry, "// assertWranglerVersion\nexport const x = 1;\n").unwrap();

    let mut engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    engine.eval_file(&entry).await.unwrap();

    let after = std::fs::read_to_string(&victim).unwrap();
    let _ = std::fs::remove_file(TMP_TARGET);
    assert_eq!(
        after, "ORIGINAL-DATA\n",
        "el transpilador escribio via symlink en /tmp"
    );
}

#[tokio::test]
async fn worker_create_requires_file_read() {
    let tmp = tempfile::tempdir().unwrap();
    let payload = tmp.path().join("payload.js");
    std::fs::write(&payload, "globalThis.__x = 1;\n").unwrap();

    let mut engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    let path = payload.to_string_lossy().replace('\\', "/");
    let r = engine
        .eval_to_string(&format!("__workerCreate('{path}', 'null')"))
        .await;
    assert!(r.is_err(), "__workerCreate leyo un archivo sin FileRead");
}
