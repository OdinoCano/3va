// Tests de enforcement en los límites de acceso a recursos.
// El crate de permisos previene el acceso no autorizado a recursos del sistema,
// que es la principal defensa contra scripts maliciosos o mal configurados.
// Ver docs/06-permissions/02-enforcement.md y docs/06-permissions/04-audit.md.
//
// Nota: los límites de memoria/CPU (max_memory, max_stack) pertenecen al
// runtime de JS (crates/js) y no al modelo de capacidades — no se prueban aquí.

use std::path::{Path, PathBuf};
use vvva_permissions::{
    Capability, PermissionState,
    audit::AuditLogger,
    enforcement::{EnvEnforcer, FsEnforcer, PermissionError},
};

// ── check_read_recursive: verifica si un directorio padre tiene grant ─────────
// Usado por operaciones que leen árboles de archivos (ej. bundler)

#[test]
fn check_read_recursive_allows_when_parent_granted() {
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/app")));
    let enforcer = FsEnforcer::new(state);
    // El grant de /app cubre cualquier subdirectorio
    assert!(
        enforcer
            .check_read_recursive(Path::new("/app/deep/nested"))
            .is_ok()
    );
    assert!(enforcer.check_read_recursive(Path::new("/app")).is_ok());
}

#[test]
fn check_read_recursive_denied_when_outside_grant() {
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/app")));
    let enforcer = FsEnforcer::new(state);
    assert!(matches!(
        enforcer.check_read_recursive(Path::new("/etc")),
        Err(PermissionError::FileReadDenied { .. })
    ));
    assert!(matches!(
        enforcer.check_read_recursive(Path::new("/home/user")),
        Err(PermissionError::FileReadDenied { .. })
    ));
}

#[test]
fn check_read_recursive_allows_when_child_grant_covers_parent_query() {
    // Si el grant es /app/node_modules y se consulta /app/node_modules/pkg,
    // la operación recursiva debe ser permitida (allowed.starts_with(path) || path.starts_with(allowed))
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/app/node_modules")));
    let enforcer = FsEnforcer::new(state);
    assert!(
        enforcer
            .check_read_recursive(Path::new("/app/node_modules/lodash"))
            .is_ok()
    );
}

// ── Grant raíz (--allow-read) da acceso a cualquier path ─────────────────────

#[test]
fn root_grant_allows_any_path() {
    // --allow-read sin argumento = FileRead("/")
    // Equivalente al preset Node.js (docs/06-permissions/01-capability-model.md §1.6.2)
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/")));
    let enforcer = FsEnforcer::new(state);
    assert!(enforcer.check_read(Path::new("/etc/passwd")).is_ok());
    assert!(
        enforcer
            .check_read(Path::new("/home/user/.ssh/id_rsa"))
            .is_ok()
    );
    assert!(
        enforcer
            .check_read(Path::new("/app/node_modules/pkg/index.js"))
            .is_ok()
    );
}

// ── EnvEnforcer::all() requiere EnvAccess ─────────────────────────────────────

#[test]
fn env_all_denied_without_capability() {
    // all() expone el entorno completo → requiere EnvAccess explícito
    let state = PermissionState::new();
    let enforcer = EnvEnforcer::new(state);
    assert!(matches!(
        enforcer.all(),
        Err(PermissionError::EnvAccessDenied)
    ));
}

#[test]
fn env_all_allowed_with_capability() {
    let state = PermissionState::new();
    state.grant(Capability::EnvAccess);
    let enforcer = EnvEnforcer::new(state);
    assert!(enforcer.all().is_ok());
}

// ── AuditLogger: registro de eventos de seguridad ────────────────────────────
// Documentado en docs/06-permissions/04-audit.md

#[test]
fn audit_logger_records_denied_events() {
    let mut logger = AuditLogger::new();
    logger.log_denied("FileRead", "/etc/passwd", "No permission granted");
    logger.log_denied("Network", "evil.com:443", "Host not in allowlist");

    let log = logger.get_log();
    assert_eq!(log.events.len(), 2);

    let json = logger.export();
    assert!(json.contains("PermissionDenied"));
    assert!(json.contains("/etc/passwd"));
    assert!(json.contains("evil.com:443"));
}

#[test]
fn audit_logger_records_file_access_events() {
    let mut logger = AuditLogger::new();
    logger.log_file(Path::new("/app/config.json"), "read", true);
    logger.log_file(Path::new("/etc/shadow"), "read", false);

    let log = logger.get_log();
    assert_eq!(log.events.len(), 2);

    let json = logger.export();
    assert!(json.contains("FileAccess"));
    assert!(json.contains("config.json"));
    assert!(json.contains("shadow"));
}

#[test]
fn audit_logger_records_network_events() {
    let mut logger = AuditLogger::new();
    // Mirrors scripts/integration_tests.sh registries
    logger.log_network("registry.npmjs.org", 443, true);
    logger.log_network("registry.yarnpkg.com", 443, true);
    logger.log_network("evil.com", 443, false);

    let log = logger.get_log();
    assert_eq!(log.events.len(), 3);

    let json = logger.export();
    assert!(json.contains("NetworkAccess"));
    assert!(json.contains("registry.npmjs.org"));
    assert!(json.contains("evil.com"));
}

#[test]
fn audit_logger_export_is_valid_json() {
    let mut logger = AuditLogger::new();
    logger.log_denied("FileRead", "/etc/passwd", "deny-by-default");
    logger.log_network("jsr.io", 443, true);
    logger.log_file(Path::new("/app/main.js"), "read", true);

    let json = logger.export();
    // Debe ser JSON parseable
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("AuditLogger::export debe producir JSON válido");
    let events = parsed["events"].as_array().unwrap();
    assert_eq!(events.len(), 3);
}

#[test]
fn audit_logger_empty_log_exports_valid_json() {
    let logger = AuditLogger::new();
    let json = logger.export();
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("log vacío debe ser JSON válido");
    assert_eq!(parsed["events"].as_array().unwrap().len(), 0);
}

// ── PermissionState no permite duplicados en grant/deny ──────────────────────

#[test]
fn grant_does_not_duplicate_capabilities() {
    let state = PermissionState::new();
    state.grant(Capability::EnvAccess);
    state.grant(Capability::EnvAccess);
    state.grant(Capability::EnvAccess);

    let granted = state.granted.read().unwrap();
    assert_eq!(
        granted.len(),
        1,
        "grant duplicado no debe multiplicar la lista"
    );
}

#[test]
fn deny_does_not_duplicate_capabilities() {
    let state = PermissionState::new();
    state.deny(Capability::SpawnProcess);
    state.deny(Capability::SpawnProcess);

    let denied = state.denied.read().unwrap();
    assert_eq!(
        denied.len(),
        1,
        "deny duplicado no debe multiplicar la lista"
    );
}
