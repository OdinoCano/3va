// Prueba que el motor JS aplica el modelo de permisos en tiempo de ejecución.
// Esto cubre el path crítico documentado en docs/06-permissions/02-enforcement.md §2.3.2:
// "En el polyfill de fs → 1. Verificar permisos → 2. Si está permitido, ejecutar operación"
//
// Ejecutar: cargo test -p vvva_js --test permission_enforcement

use std::path::PathBuf;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Evalúa un IIFE que atrapa excepciones JS y retorna "allowed" o "denied:<mensaje>".
/// Permite inspeccionar el mensaje de error sin que el test falle por excepción.
fn eval_catching(engine: &JsEngine, js_call: &str) -> String {
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
        .unwrap_or_else(|e| format!("error:{}", e))
}

// ── FileRead ─────────────────────────────────────────────────────────────────

#[test]
fn fs_read_blocked_without_allow_read() {
    let state = PermissionState::new();
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsReadFileSync('/etc/hostname')");

    assert!(
        result.starts_with("denied:"),
        "fs.readFile sin allow-read debe lanzar excepción, got: {result}"
    );
    assert!(
        result.contains("allow-read") || result.contains("Permission denied"),
        "el mensaje debe mencionar el permiso requerido: {result}"
    );
}

#[test]
fn fs_read_allowed_with_scoped_grant() {
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/etc")));
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsReadFileSync('/etc/hostname')");

    assert_eq!(
        result, "allowed",
        "fs.readFile con FileRead('/etc') debe funcionar: {result}"
    );
}

#[test]
fn fs_read_scoped_grant_blocks_outside_scope() {
    let state = PermissionState::new();
    // Solo /tmp está permitido — /etc no
    state.grant(Capability::FileRead(PathBuf::from("/tmp")));
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsReadFileSync('/etc/hostname')");

    assert!(
        result.starts_with("denied:"),
        "FileRead('/tmp') no debe permitir acceso a /etc: {result}"
    );
}

// ── FileWrite ────────────────────────────────────────────────────────────────

#[test]
fn fs_write_blocked_without_allow_write() {
    let state = PermissionState::new();
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(
        &engine,
        "__fsWriteFileSync('/tmp/3va_test_blocked.txt', 'evil')",
    );

    assert!(
        result.starts_with("denied:"),
        "fs.writeFile sin allow-write debe lanzar excepción: {result}"
    );
    assert!(
        result.contains("allow-write") || result.contains("Permission denied"),
        "el mensaje debe mencionar el permiso requerido: {result}"
    );
}

#[test]
fn fs_write_allowed_with_grant() {
    let state = PermissionState::new();
    state.grant(Capability::FileWrite(PathBuf::from("/tmp")));
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(
        &engine,
        "__fsWriteFileSync('/tmp/3va_test_write_ok.txt', 'hello')",
    );

    assert_eq!(
        result, "allowed",
        "fs.writeFile con FileWrite('/tmp') debe funcionar: {result}"
    );

    // Limpiar
    let _ = std::fs::remove_file("/tmp/3va_test_write_ok.txt");
}

#[test]
fn fs_write_grant_does_not_grant_read() {
    // FileWrite no implica FileRead — permisos ortogonales
    let state = PermissionState::new();
    state.grant(Capability::FileWrite(PathBuf::from("/tmp")));
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsReadFileSync('/etc/hostname')");

    assert!(
        result.starts_with("denied:"),
        "FileWrite no debe implicar FileRead: {result}"
    );
}

// ── ReadDir ──────────────────────────────────────────────────────────────────

#[test]
fn fs_readdir_blocked_without_allow_read() {
    let state = PermissionState::new();
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsReaddirSync('/tmp')");

    assert!(
        result.starts_with("denied:"),
        "readdir sin allow-read debe lanzar excepción: {result}"
    );
}

#[test]
fn fs_readdir_allowed_with_grant() {
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/tmp")));
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsReaddirSync('/tmp')");

    assert_eq!(
        result, "allowed",
        "readdir con grant no debe lanzar excepción: {result}"
    );
}

// ── Mkdir / Rm ────────────────────────────────────────────────────────────────

#[test]
fn fs_mkdir_blocked_without_allow_write() {
    let state = PermissionState::new();
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsMkdirSync('/tmp/3va_blocked_dir')");

    assert!(
        result.starts_with("denied:"),
        "mkdir sin allow-write debe lanzar excepción: {result}"
    );
}

#[test]
fn fs_rm_blocked_without_allow_write() {
    let state = PermissionState::new();
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsRmSync('/tmp/nonexistent')");

    assert!(
        result.starts_with("denied:"),
        "rm sin allow-write debe lanzar excepción: {result}"
    );
}

// ── deny_all_fs deshabilita todo el filesystem desde JS ──────────────────────

#[test]
fn deny_all_fs_blocks_js_read_even_with_root_grant() {
    let mut state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/")));
    state.deny_all_fs();
    let engine = JsEngine::new(&state).unwrap();

    let result = eval_catching(&engine, "__fsReadFileSync('/etc/hostname')");

    assert!(
        result.starts_with("denied:"),
        "deny_all_fs debe bloquear incluso con grant raíz: {result}"
    );
}
