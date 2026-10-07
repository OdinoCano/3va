// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Regresiones de permisos de `worker_threads` (prompt 4 de la auditoría).
//!
//! Un worker arranca con una *instantánea* de los permisos del padre, nunca con
//! su `Arc<PermissionState>` compartido: así no puede ampliar el sandbox del
//! padre, ni sus denegaciones/prompts contaminan el estado del padre, ni un
//! grant posterior del padre ensancha al worker ya en marcha. Bajo un sandbox
//! restringido, el worker no puede leer fuera de lo concedido, ni conectar a la
//! red, ni lanzar procesos.

use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

const POLLS: usize = 500;

fn write_sandbox(tmp: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let sandbox = tmp.join("sandbox");
    let outside = tmp.join("outside");
    std::fs::create_dir_all(&sandbox).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, "TOP-SECRET\n").unwrap();
    let worker = sandbox.join("worker.js");
    let report = sandbox.join("report.json");
    (sandbox, secret, worker, report)
}

/// Bloquea hasta que el worker haya escrito su informe (o se agota el tiempo).
async fn wait_for_report(engine: &mut JsEngine, report: &Path) -> String {
    for _ in 0..POLLS {
        if report.exists() {
            return std::fs::read_to_string(report).unwrap();
        }
        engine.idle().await;
        let _ = engine.pump_timers().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("el worker no escribio el informe en {:?}", report);
}

// El worker hereda el sandbox restringido del padre: fs fuera del grant, red y
// child_process deben ser denegados, sin efectos observables en el host.
#[tokio::test]
async fn restricted_worker_is_denied_fs_net_and_child_process() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();

    let tmp = tempfile::tempdir().unwrap();
    let (sandbox, secret, worker, report) = write_sandbox(tmp.path());
    let spawned = tmp.path().join("outside").join("spawned.txt");

    let script = format!(
        r#"
var out = {{}};
try {{ __fsReadFileSync('{secret}'); out.fs = 'allowed'; }} catch (e) {{ out.fs = 'denied'; }}
// __tcpConnect returns an Error object on denial (code EACCES), a number on success.
var c = __tcpConnect('127.0.0.1', {port}); out.net = (typeof c === 'number') ? 'allowed' : 'denied';
try {{ __execSyncShell("touch '{spawned}'"); out.cp = 'allowed'; }} catch (e) {{ out.cp = 'denied'; }}
__fsWriteFileSync('{report}', JSON.stringify(out));
"#,
        secret = secret.to_string_lossy().replace('\\', "/"),
        spawned = spawned.to_string_lossy().replace('\\', "/"),
        report = report.to_string_lossy().replace('\\', "/"),
    );
    std::fs::write(&worker, script).unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(sandbox.clone()));
    state.grant(Capability::FileWrite(sandbox.clone()));
    let mut engine = JsEngine::new(state).await.unwrap();

    let path = worker.to_string_lossy().replace('\\', "/");
    engine
        .eval_to_string(&format!(
            "new (require('worker_threads').Worker)('{path}'); 'started'"
        ))
        .await
        .unwrap();

    let report_json = wait_for_report(&mut engine, &report).await;
    assert_eq!(
        report_json, r#"{"fs":"denied","net":"denied","cp":"denied"}"#,
        "el worker restringido ejecuto una operacion fuera de su sandbox: {report_json}"
    );
    assert!(
        !spawned.exists(),
        "el worker lanzo un proceso hijo pese al sandbox"
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock,
        "el worker abrio una conexion de red pese al sandbox"
    );
}

// Compartir el `Arc<PermissionState>` hacía que una denegación del worker
// apareciera como del padre. El worker debe tener su propia instantánea.
#[tokio::test]
async fn worker_does_not_share_permission_state_with_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let (sandbox, secret, worker, report) = write_sandbox(tmp.path());

    let script = format!(
        r#"
var out = {{}};
try {{ __fsReadFileSync('{secret}'); out.fs = 'allowed'; }} catch (e) {{ out.fs = 'denied'; }}
var c = __tcpConnect('127.0.0.1', 9); out.net = (typeof c === 'number') ? 'allowed' : 'denied';
try {{ __execSyncShell('true'); out.cp = 'allowed'; }} catch (e) {{ out.cp = 'denied'; }}
__fsWriteFileSync('{report}', JSON.stringify(out));
"#,
        secret = secret.to_string_lossy().replace('\\', "/"),
        report = report.to_string_lossy().replace('\\', "/"),
    );
    std::fs::write(&worker, script).unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(sandbox.clone()));
    state.grant(Capability::FileWrite(sandbox.clone()));
    let mut engine = JsEngine::new(state.clone()).await.unwrap();

    let path = worker.to_string_lossy().replace('\\', "/");
    engine
        .eval_to_string(&format!(
            "new (require('worker_threads').Worker)('{path}'); 'started'"
        ))
        .await
        .unwrap();

    let _ = wait_for_report(&mut engine, &report).await;

    // El padre no intento nada y su informe de denegaciones debe seguir vacio:
    // las denegaciones del worker pertenecen al worker.
    assert!(
        state.denials().is_empty(),
        "el worker compartio su PermissionState con el padre: {:?}",
        state.denials()
    );
}

// Verificación: `{ eval: true }` no se honra (el primer argumento se trata
// siempre como ruta), así que no abre una vía alternativa de ejecución; sólo
// puede fallar el chequeo `FileRead` sobre el texto.
#[tokio::test]
async fn worker_eval_true_is_not_a_code_execution_path() {
    let tmp = tempfile::tempdir().unwrap();
    let (sandbox, _secret, _worker, _report) = write_sandbox(tmp.path());
    let sentinel = tmp.path().join("outside").join("eval-executed.txt");

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(sandbox));
    let mut engine = JsEngine::new(state).await.unwrap();

    let code = format!("__execSyncShell(\"touch '{}'\")", sentinel.display());
    let code_js = code
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('"', "\\\"");
    let result = engine
        .eval_to_string(&format!(
            "(function() {{ try {{ new (require('worker_threads').Worker)(\"{code_js}\", {{eval:true}}); \
             return 'executed'; }} catch (e) {{ return 'threw'; }} }})()"
        ))
        .await
        .unwrap();

    assert_eq!(
        result, "threw",
        "eval:true trato el codigo como archivo y no lo ejecuto"
    );
    assert!(
        !sentinel.exists(),
        "eval:true ejecuto codigo sin pasar por FileRead"
    );
}
