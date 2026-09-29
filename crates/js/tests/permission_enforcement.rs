// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Prueba que el motor JS aplica el modelo de permisos en tiempo de ejecución.
// Esto cubre el path crítico documentado en docs/06-permissions/02-enforcement.md §2.3.2:
// "En el polyfill de fs → 1. Verificar permisos → 2. Si está permitido, ejecutar operación"
//
// Ejecutar: cargo test -p vvva_js --test permission_enforcement

use std::path::PathBuf;
use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Evalúa un IIFE que atrapa excepciones JS y retorna "allowed" o "denied:<mensaje>".
/// Permite inspeccionar el mensaje de error sin que el test falle por excepción.
async fn eval_catching(engine: &mut JsEngine, js_call: &str) -> String {
    let code = format!(
        r#"
        (() => {{
            try {{
                {};
                return 'allowed';
            }} catch(e) {{
                return 'denied:' + (e.message || String(e));
            }}
        }})()
        "#,
        js_call
    );
    engine
        .eval_to_string(&code)
        .await
        .unwrap_or_else(|e| format!("error:{}", e))
}

// ── FileRead ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_read_blocked_without_allow_read() {
    let state = PermissionState::new();
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, "__fsReadFileSync('/etc/hostname')").await;

    assert!(
        result.starts_with("denied:"),
        "fs.readFile sin allow-read debe lanzar excepción, got: {result}"
    );
    assert!(
        result.contains("allow-read") || result.contains("Permission denied"),
        "el mensaje debe mencionar el permiso requerido: {result}"
    );
}

#[tokio::test]
async fn fs_read_allowed_with_scoped_grant() {
    #[cfg(windows)]
    let (grant_dir, file_path) = (
        PathBuf::from(r"C:\Windows"),
        r"C:\\Windows\\win.ini".to_string(),
    );
    #[cfg(not(windows))]
    let (grant_dir, file_path) = (PathBuf::from("/etc"), "/etc/hostname".to_string());

    let state = PermissionState::new();
    state.grant(Capability::FileRead(grant_dir));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, &format!("__fsReadFileSync('{file_path}')")).await;

    assert_eq!(
        result, "allowed",
        "fs.readFile con FileRead grant debe funcionar: {result}"
    );
}

#[tokio::test]
async fn fs_read_scoped_grant_blocks_outside_scope() {
    let state = PermissionState::new();
    // Solo /tmp está permitido — /etc no
    state.grant(Capability::FileRead(PathBuf::from("/tmp")));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, "__fsReadFileSync('/etc/hostname')").await;

    assert!(
        result.starts_with("denied:"),
        "FileRead('/tmp') no debe permitir acceso a /etc: {result}"
    );
}

// ── FileWrite ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_write_blocked_without_allow_write() {
    let state = PermissionState::new();
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(
        &mut engine,
        "__fsWriteFileSync('/tmp/3va_test_blocked.txt', 'evil')",
    )
    .await;

    assert!(
        result.starts_with("denied:"),
        "fs.writeFile sin allow-write debe lanzar excepción: {result}"
    );
    assert!(
        result.contains("allow-write") || result.contains("Permission denied"),
        "el mensaje debe mencionar el permiso requerido: {result}"
    );
}

#[tokio::test]
async fn fs_write_allowed_with_grant() {
    let state = PermissionState::new();
    state.grant(Capability::FileWrite(PathBuf::from("/tmp")));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(
        &mut engine,
        "__fsWriteFileSync('/tmp/3va_test_write_ok.txt', 'hello')",
    )
    .await;

    assert_eq!(
        result, "allowed",
        "fs.writeFile con FileWrite('/tmp') debe funcionar: {result}"
    );

    // Limpiar
    let _ = std::fs::remove_file("/tmp/3va_test_write_ok.txt");
}

#[tokio::test]
async fn fs_write_grant_does_not_grant_read() {
    // FileWrite no implica FileRead — permisos ortogonales
    let state = PermissionState::new();
    state.grant(Capability::FileWrite(PathBuf::from("/tmp")));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, "__fsReadFileSync('/etc/hostname')").await;

    assert!(
        result.starts_with("denied:"),
        "FileWrite no debe implicar FileRead: {result}"
    );
}

// ── ReadDir ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_readdir_blocked_without_allow_read() {
    let state = PermissionState::new();
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, "__fsReaddirSync('/tmp')").await;

    assert!(
        result.starts_with("denied:"),
        "readdir sin allow-read debe lanzar excepción: {result}"
    );
}

#[tokio::test]
async fn fs_readdir_allowed_with_grant() {
    #[cfg(windows)]
    let dir_path = r"C:\\Windows".to_string();
    #[cfg(not(windows))]
    let dir_path = "/tmp".to_string();

    #[cfg(windows)]
    let grant_path = PathBuf::from(r"C:\Windows");
    #[cfg(not(windows))]
    let grant_path = PathBuf::from("/tmp");

    let state = PermissionState::new();
    state.grant(Capability::FileRead(grant_path));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, &format!("__fsReaddirSync('{dir_path}')")).await;

    assert_eq!(
        result, "allowed",
        "readdir con grant no debe lanzar excepción: {result}"
    );
}

// ── Mkdir / Rm ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_mkdir_blocked_without_allow_write() {
    let state = PermissionState::new();
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, "__fsMkdirSync('/tmp/3va_blocked_dir')").await;

    assert!(
        result.starts_with("denied:"),
        "mkdir sin allow-write debe lanzar excepción: {result}"
    );
}

#[tokio::test]
async fn fs_rm_blocked_without_allow_write() {
    let state = PermissionState::new();
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, "__fsRmSync('/tmp/nonexistent')").await;

    assert!(
        result.starts_with("denied:"),
        "rm sin allow-write debe lanzar excepción: {result}"
    );
}

// ── deny_all_fs deshabilita todo el filesystem desde JS ──────────────────────

#[tokio::test]
async fn deny_all_fs_blocks_js_read_even_with_root_grant() {
    let mut state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/")));
    state.deny_all_fs();
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, "__fsReadFileSync('/etc/hostname')").await;

    assert!(
        result.starts_with("denied:"),
        "deny_all_fs debe bloquear incluso con grant raíz: {result}"
    );
}

// ── VULN-03: a package's deny-net must hold however it reaches the network ──
//
// The root is granted 127.0.0.1, so a Network denial can only come from the
// package's own deny rule: each test fails if scoping is skipped. The scope
// comes from the V8 stack (plus promise/timer inheritance), not from the
// require() wrapper alone, which globals, constructors, raw bindings and
// async hand-offs used to sidestep.
async fn scoped_pkg_is_denied(src: &str) -> bool {
    let tmp = tempfile::tempdir().unwrap();
    let evil_dir = tmp.path().join("node_modules").join("evil");
    std::fs::create_dir_all(&evil_dir).unwrap();
    std::fs::write(evil_dir.join("index.js"), src).unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(tmp.path().to_path_buf()));
    state.grant(Capability::Network("127.0.0.1".to_string()));
    state.deny_scoped("evil", Capability::Network("*".to_string()));
    let mut engine = JsEngine::new(state.clone()).await.unwrap();

    let dir = tmp.path().to_string_lossy().replace('\\', "/");
    engine
        .eval_to_string(&format!("globalThis.__dirname = {dir:?}; require('evil');"))
        .await
        .unwrap();
    for _ in 0..3 {
        engine.idle().await;
        engine.pump_timers().await.unwrap();
    }
    engine.idle().await;
    state
        .denials()
        .iter()
        .any(|c| matches!(c, Capability::Network(_)))
}

#[tokio::test]
async fn scoped_deny_net_gates_http_builtin() {
    assert!(
        scoped_pkg_is_denied(
            "require('http').get({ host: '127.0.0.1', port: 9 }).on('error', function () {});"
        )
        .await
    );
}

#[tokio::test]
async fn scoped_deny_net_gates_global_fetch() {
    assert!(scoped_pkg_is_denied("fetch('http://127.0.0.1:9/').catch(function () {});").await);
}

#[tokio::test]
async fn scoped_deny_net_gates_socket_constructor() {
    assert!(
        scoped_pkg_is_denied(
            "var s = new (require('net').Socket)(); s.on('error', function () {}); s.connect(9, '127.0.0.1');"
        )
        .await
    );
}

#[tokio::test]
async fn scoped_deny_net_gates_raw_native_binding() {
    assert!(scoped_pkg_is_denied("try { __tcpConnect('127.0.0.1', 9); } catch (e) {}").await);
}

#[tokio::test]
async fn scoped_deny_net_survives_tampering_with_wrapper_globals() {
    assert!(
        scoped_pkg_is_denied(
            "delete globalThis.__SCOPE_GATED_MODULES.http; globalThis.__vvva_entry_scope = 'evil'; \
             require('http').get({ host: '127.0.0.1', port: 9 }).on('error', function () {});"
        )
        .await
    );
}

#[tokio::test]
async fn scoped_deny_net_follows_promise_reactions() {
    assert!(
        scoped_pkg_is_denied(
            "Promise.resolve('http://127.0.0.1:9/').then(fetch).catch(function () {});"
        )
        .await
    );
}

#[tokio::test]
async fn scoped_deny_net_follows_timers() {
    assert!(
        scoped_pkg_is_denied(
            "setTimeout(function () { try { __tcpConnect('127.0.0.1', 9); } catch (e) {} }.bind(null), 0); \
             setTimeout(fetch.bind(null, 'http://127.0.0.1:9/'), 0);"
        )
        .await
    );
}

#[tokio::test]
async fn scoped_deny_net_does_not_leak_to_root_code() {
    // A deny-net package being loaded must not deny the app's own calls.
    assert!(!scoped_pkg_is_denied("module.exports = 1;").await);
}

// A package-scoped grant must not be borrowable: forging the wrapper scope
// (`__setCallerScope`) used to hand one package another's allow-net, while
// the granted package's own unwrapped calls (fetch, raw bindings) didn't get
// its grant at all.
async fn net_denied_for(pkg: &str, src: &str) -> bool {
    let tmp = tempfile::tempdir().unwrap();
    for name in ["trusted", "evil"] {
        let dir = tmp.path().join("node_modules").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let body = if name == pkg {
            src
        } else {
            "module.exports = 1;"
        };
        std::fs::write(dir.join("index.js"), body).unwrap();
    }
    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(tmp.path().to_path_buf()));
    state.grant_scoped("trusted", Capability::Network("127.0.0.1".to_string()));
    let mut engine = JsEngine::new(state.clone()).await.unwrap();
    let dir = tmp.path().to_string_lossy().replace('\\', "/");
    engine
        .eval_to_string(&format!(
            "globalThis.__dirname = {dir:?}; require('trusted'); require('evil');"
        ))
        .await
        .unwrap();
    engine.idle().await;
    state
        .denials()
        .iter()
        .any(|c| matches!(c, Capability::Network(_)))
}

#[tokio::test]
async fn scoped_grant_cannot_be_borrowed_by_forging_scope() {
    assert!(
        net_denied_for(
            "evil",
            "__setCallerScope('trusted'); try { __tcpConnect('127.0.0.1', 9); } catch (e) {}"
        )
        .await
    );
}

#[tokio::test]
async fn scoped_grant_applies_to_unwrapped_calls_of_its_package() {
    assert!(
        !net_denied_for(
            "trusted",
            "try { __tcpConnect('127.0.0.1', 9); } catch (e) {}"
        )
        .await
    );
}

// ── VULN-06: dns must be gated by Capability::Network ─────────────────────────

#[tokio::test]
async fn dns_lookup_blocked_without_net_grant() {
    let state = PermissionState::new();
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    // __dnsLookup returns an error STRING (not an exception) when denied, so
    // inspect the return value rather than whether the call threw.
    let result = engine
        .eval_to_string("__dnsLookup('example.com')")
        .await
        .unwrap();

    assert!(
        result.contains("EACCES"),
        "dns.lookup sin allow-net debe ser denegado, got: {result:?}"
    );
}

#[tokio::test]
async fn dns_lookup_allowed_with_net_grant() {
    let state = PermissionState::new();
    state.grant(Capability::Network("example.com".to_string()));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = engine
        .eval_to_string("__dnsLookup('example.com')")
        .await
        .unwrap();

    // Permission granted: the call must NOT be a permission denial. Resolution
    // is a real network lookup, so any non-EACCES result is acceptable here.
    assert!(
        !result.contains("EACCES"),
        "dns.lookup con grant de red no debe ser denegado por permisos: {result:?}"
    );
}

#[tokio::test]
async fn scoped_deny_net_gates_dns_builtin() {
    // VULN-03 + VULN-06: dns was previously ungated AND unscoped.
    assert!(scoped_pkg_is_denied("require('dns').lookup('127.0.0.1', function () {});").await);
}

#[tokio::test]
async fn http_still_works_in_root_scope_with_net_grant() {
    // Positive control: the app's own code (root scope) with --allow-net still
    // reaches the network; the scope gating must not over-block.
    let state = PermissionState::new();
    state.grant(Capability::Network("127.0.0.1".to_string()));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, "__tcpConnect('127.0.0.1', 9)").await;

    // Permission is granted; the OS-level connection is refused (no listener),
    // but it must NOT be a permission denial.
    assert!(
        result == "allowed" || result.starts_with("denied:ECONNREFUSED"),
        "root-scope net grant must not be blocked by scope gating: {result}"
    );
}

// ── VULN-17: socket builtins honour port-scoped grants ────────────────────────

#[tokio::test]
async fn port_scoped_grant_limits_tcp_connect() {
    let state = Arc::new(PermissionState::new());
    state.grant(Capability::Network("127.0.0.1:1".to_string()));
    let mut engine = JsEngine::new(state.clone()).await.unwrap();
    // 9 is outside the grant; 65545 used to wrap to 9 via `as u16`.
    for port in ["9", "65545"] {
        eval_catching(&mut engine, &format!("__tcpConnect('127.0.0.1', {port})")).await;
    }
    // Both attempts are refused before connecting: 9 by the port-scoped
    // grant, 65545 because it maps to the invalid port 0 instead of 9.
    let denials = state.denials();
    assert!(
        denials
            .iter()
            .any(|c| matches!(c, Capability::Network(h) if h == "127.0.0.1:9")),
        "denials: {denials:?}"
    );
    assert!(
        denials
            .iter()
            .any(|c| matches!(c, Capability::Network(h) if h == "127.0.0.1:0")),
        "denials: {denials:?}"
    );
}

// A pre-bound builtin handed to someone else (here an event listener that
// root code fires later, from its own timer) runs with no package frame on
// the stack; `bind` must carry the binder's deny scopes with it (VULN-03).
#[tokio::test]
async fn scoped_deny_net_follows_bound_builtins_used_as_callbacks() {
    let tmp = tempfile::tempdir().unwrap();
    let evil_dir = tmp.path().join("node_modules").join("evil");
    std::fs::create_dir_all(&evil_dir).unwrap();
    std::fs::write(
        evil_dir.join("index.js"),
        "module.exports = function (em) { em.on('go', __tcpConnect.bind(null, '127.0.0.1', 9)); };",
    )
    .unwrap();

    let state = Arc::new(PermissionState::new());
    state.grant(Capability::FileRead(tmp.path().to_path_buf()));
    state.grant(Capability::Network("127.0.0.1".to_string()));
    state.deny_scoped("evil", Capability::Network("*".to_string()));
    let mut engine = JsEngine::new(state.clone()).await.unwrap();

    let dir = tmp.path().to_string_lossy().replace('\\', "/");
    engine
        .eval_to_string(&format!(
            "globalThis.__dirname = {dir:?}; var EE = require('events'); var em = new EE(); \
             require('evil')(em); setTimeout(function () {{ try {{ em.emit('go'); }} catch (e) {{}} }}, 0); \
             var plain = function (a, b) {{ return a + b; }}.bind(null, 1); \
             globalThis.__bindStillWorks = plain(2) === 3 && plain.name === 'bound ';"
        ))
        .await
        .unwrap();
    for _ in 0..3 {
        engine.idle().await;
        engine.pump_timers().await.unwrap();
    }
    assert!(
        state
            .denials()
            .iter()
            .any(|c| matches!(c, Capability::Network(_))),
        "denials: {:?}",
        state.denials()
    );
    assert_eq!(
        engine
            .eval_to_string("String(globalThis.__bindStillWorks)")
            .await
            .unwrap(),
        "true"
    );
}

// sqlite opens database files by path; without the fs grants it must not be
// able to read or create one anywhere on the host (VULN-03/sqlite).
#[tokio::test]
async fn sqlite_open_blocked_without_fs_grant() {
    let state = PermissionState::new();
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(
        &mut engine,
        "__sqliteOpen('/tmp/3va_test_sqlite_blocked.db')",
    )
    .await;

    assert!(
        result.starts_with("denied:"),
        "sqlite.open sin grant de fs debe lanzar excepción: {result}"
    );
    assert!(
        result.contains("permission denied"),
        "el mensaje debe mencionar el permiso: {result}"
    );
}

#[tokio::test]
async fn sqlite_open_allowed_with_fs_grant() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let db_str = db_path.to_string_lossy().replace('\\', "/");

    let state = PermissionState::new();
    state.grant(Capability::FileRead(tmp.path().to_path_buf()));
    state.grant(Capability::FileWrite(tmp.path().to_path_buf()));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let result = eval_catching(&mut engine, &format!("__sqliteOpen('{db_str}')")).await;

    assert_eq!(
        result, "allowed",
        "sqlite.open con grant de fs debe funcionar: {result}"
    );
    assert!(
        db_path.exists(),
        "el grant de escritura debe permitir crear la base de datos"
    );
}

// SQL can name files too: ATTACH / VACUUM INTO must not reach a path the
// fs grants don't cover, and an in-memory database needs no fs grant.
#[tokio::test]
async fn sqlite_sql_cannot_open_files_outside_the_grant() {
    let tmp = tempfile::tempdir().unwrap();
    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let db = sandbox.join("a.db").to_string_lossy().replace('\\', "/");
    let outside = tmp
        .path()
        .join("outside.db")
        .to_string_lossy()
        .replace('\\', "/");

    let state = PermissionState::new();
    state.grant(Capability::FileRead(sandbox.clone()));
    state.grant(Capability::FileWrite(sandbox.clone()));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();

    let out = engine
        .eval_to_string(&format!(
            "var id = __sqliteOpen('{db}'); \
             [__sqliteExec(id, \"ATTACH DATABASE '{outside}' AS o\"), \
              __sqliteExec(id, \"VACUUM INTO '{outside}'\"), \
              typeof __sqliteOpen(':memory:')].join('|')"
        ))
        .await
        .unwrap();
    let parts: Vec<&str> = out.split('|').collect();
    assert!(parts[0].contains("error"), "ATTACH must fail: {out}");
    assert!(parts[1].contains("error"), "VACUUM INTO must fail: {out}");
    assert_eq!(parts[2], "number", ":memory: needs no fs grant: {out}");
    assert!(!tmp.path().join("outside.db").exists());
}
