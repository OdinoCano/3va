// Tests del modelo de capacidades contra bypass de permisos.
// Cubre el algoritmo deny-by-default documentado en docs/06-permissions/01-capability-model.md
// y los enforcers de docs/06-permissions/02-enforcement.md.

use std::path::PathBuf;
use vvva_permissions::{
    enforcement::{EnvEnforcer, FsEnforcer, NetEnforcer, PermissionError, ProcessEnforcer},
    Capability, PermissionState,
};

// ── Deny-by-default (paso 5 del algoritmo de verificación) ───────────────────

#[test]
fn deny_by_default_fs_read() {
    let state = PermissionState::new();
    let enforcer = FsEnforcer::new(state);
    let result = enforcer.check_read(std::path::Path::new("/etc/passwd"));
    assert!(
        matches!(result, Err(PermissionError::FileReadDenied { .. })),
        "FileRead debe ser denegado sin capabilities"
    );
}

#[test]
fn deny_by_default_fs_write() {
    let state = PermissionState::new();
    let enforcer = FsEnforcer::new(state);
    let result = enforcer.check_write(std::path::Path::new("/tmp/evil.sh"));
    assert!(matches!(result, Err(PermissionError::FileWriteDenied { .. })));
}

#[test]
fn deny_by_default_network() {
    let state = PermissionState::new();
    let enforcer = NetEnforcer::new(state);
    let result = enforcer.check_connect("registry.npmjs.org", 443);
    assert!(matches!(result, Err(PermissionError::NetworkDenied { .. })));
}

#[test]
fn deny_by_default_env() {
    let state = PermissionState::new();
    let enforcer = EnvEnforcer::new(state);
    let result = enforcer.get("PATH");
    assert!(matches!(result, Err(PermissionError::EnvAccessDenied)));
}

#[test]
fn deny_by_default_process() {
    let state = PermissionState::new();
    let enforcer = ProcessEnforcer::new(state);
    let result = enforcer.spawn("ls", &["-la".to_string()]);
    assert!(matches!(result, Err(PermissionError::ProcessSpawnDenied)));
}

// ── Scope de FileRead: el path concedido actúa como prefijo ──────────────────
// Documentado en docs/06-permissions/01-capability-model.md §1.5.1

#[test]
fn scoped_read_allows_subpath() {
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/app")));
    let enforcer = FsEnforcer::new(state);
    assert!(enforcer.check_read(std::path::Path::new("/app/config.json")).is_ok());
    assert!(enforcer.check_read(std::path::Path::new("/app/subdir/file.rs")).is_ok());
}

#[test]
fn scoped_read_denies_sibling_path() {
    // --allow-read=/app no debe dar acceso a /etc ni a /home
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/app")));
    let enforcer = FsEnforcer::new(state);
    assert!(matches!(
        enforcer.check_read(std::path::Path::new("/etc/passwd")),
        Err(PermissionError::FileReadDenied { .. })
    ));
    assert!(matches!(
        enforcer.check_read(std::path::Path::new("/home/user/.ssh/id_rsa")),
        Err(PermissionError::FileReadDenied { .. })
    ));
}

#[test]
fn scoped_read_denies_path_with_same_prefix_string() {
    // /application no debe ser permitido por un grant de /app
    let state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/app")));
    let enforcer = FsEnforcer::new(state);
    // PathBuf::starts_with es component-aware, no string prefix
    assert!(matches!(
        enforcer.check_read(std::path::Path::new("/application/secret")),
        Err(PermissionError::FileReadDenied { .. })
    ));
}

// ── deny explícita tiene precedencia sobre grant (paso 2 del algoritmo) ──────

#[test]
fn deny_overrides_grant_env() {
    let state = PermissionState::new();
    state.grant(Capability::EnvAccess);
    state.deny(Capability::EnvAccess);
    let enforcer = EnvEnforcer::new(state);
    assert!(matches!(
        enforcer.get("PATH"),
        Err(PermissionError::EnvAccessDenied)
    ));
}

#[test]
fn deny_overrides_grant_process() {
    let state = PermissionState::new();
    state.grant(Capability::SpawnProcess);
    state.deny(Capability::SpawnProcess);
    let enforcer = ProcessEnforcer::new(state);
    assert!(matches!(
        enforcer.spawn("ls", &[]),
        Err(PermissionError::ProcessSpawnDenied)
    ));
}

// ── deny_all_* tiene precedencia sobre cualquier grant (paso 1 del algoritmo) ─

#[test]
fn deny_all_fs_blocks_granted_read() {
    let mut state = PermissionState::new();
    state.grant(Capability::FileRead(PathBuf::from("/")));
    state.deny_all_fs();
    let enforcer = FsEnforcer::new(state);
    assert!(matches!(
        enforcer.check_read(std::path::Path::new("/app/config.json")),
        Err(PermissionError::FileReadDenied { .. })
    ));
}

#[test]
fn deny_all_net_blocks_granted_network() {
    let mut state = PermissionState::new();
    state.grant(Capability::Network("*".to_string()));
    state.deny_all_net();
    let enforcer = NetEnforcer::new(state);
    assert!(matches!(
        enforcer.check_connect("registry.npmjs.org", 443),
        Err(PermissionError::NetworkDenied { .. })
    ));
}

#[test]
fn deny_all_env_blocks_granted_env() {
    let mut state = PermissionState::new();
    state.grant(Capability::EnvAccess);
    state.deny_all_env();
    let enforcer = EnvEnforcer::new(state);
    assert!(matches!(
        enforcer.get("PATH"),
        Err(PermissionError::EnvAccessDenied)
    ));
}

// ── Wildcard de red: documentado en docs/06-permissions/01-capability-model.md §1.5.2 ─

#[test]
fn wildcard_star_network_allows_all_hosts() {
    // --allow-net (sin argumento) = Network("*")
    let state = PermissionState::new();
    state.grant(Capability::Network("*".to_string()));
    let enforcer = NetEnforcer::new(state);
    assert!(enforcer.check_connect("registry.npmjs.org", 443).is_ok());
    assert!(enforcer.check_connect("jsr.io", 443).is_ok());
    assert!(enforcer.check_connect("registry.yarnpkg.com", 443).is_ok());
}

#[test]
fn wildcard_subdomain_allows_subdomain_not_parent() {
    // *.example.com cubre api.example.com pero NO example.com
    // (tabla en docs/06-permissions/01-capability-model.md §1.5.3)
    let state = PermissionState::new();
    state.grant(Capability::Network("*.example.com".to_string()));
    let enforcer = NetEnforcer::new(state);
    assert!(enforcer.check_connect("api.example.com", 443).is_ok());
    assert!(enforcer.check_connect("cdn.example.com", 443).is_ok());
    assert!(matches!(
        enforcer.check_connect("example.com", 443),
        Err(PermissionError::NetworkDenied { .. })
    ));
    assert!(matches!(
        enforcer.check_connect("evil.com", 443),
        Err(PermissionError::NetworkDenied { .. })
    ));
}

// ── EnvEnforcer: lista de variables permitidas ────────────────────────────────
// Documentado en docs/06-permissions/02-enforcement.md §2.5

#[test]
fn env_enforcer_with_allowlist_restricts_to_listed_vars() {
    let state = PermissionState::new();
    state.grant(Capability::EnvAccess);
    let enforcer = EnvEnforcer::new(state).with_allowed_vars(vec!["PATH".to_string()]);
    assert!(enforcer.get("PATH").is_ok());
    assert!(matches!(
        enforcer.get("SECRET_KEY"),
        Err(PermissionError::EnvVarNotAllowed(_))
    ));
    assert!(matches!(
        enforcer.get("AWS_SECRET_ACCESS_KEY"),
        Err(PermissionError::EnvVarNotAllowed(_))
    ));
}

#[test]
fn env_enforcer_all_denied_without_capability() {
    // all() expone todas las variables → requiere EnvAccess
    let state = PermissionState::new();
    let enforcer = EnvEnforcer::new(state);
    assert!(matches!(enforcer.all(), Err(PermissionError::EnvAccessDenied)));
}

// ── ProcessEnforcer: lista de comandos permitidos ─────────────────────────────
// Documentado en docs/06-permissions/02-enforcement.md §2.6

#[test]
fn process_enforcer_command_allowlist_blocks_unlisted() {
    let state = PermissionState::new();
    state.grant(Capability::SpawnProcess);
    let enforcer =
        ProcessEnforcer::new(state).with_allowed_commands(vec!["ls".to_string(), "cat".to_string()]);
    assert!(enforcer.spawn("ls", &["-la".to_string()]).is_ok());
    assert!(matches!(
        enforcer.spawn("rm", &["-rf".to_string(), "/".to_string()]),
        Err(PermissionError::CommandNotAllowed(_))
    ));
    assert!(matches!(
        enforcer.spawn("curl", &[]),
        Err(PermissionError::CommandNotAllowed(_))
    ));
}

// ── PermissionError::category() para el audit log ────────────────────────────

#[test]
fn permission_error_category_matches_enforcement_area() {
    let fs_err = PermissionError::FileReadDenied {
        path: PathBuf::from("/etc/passwd"),
    };
    assert_eq!(fs_err.category(), "FileSystem");

    let net_err = PermissionError::NetworkDenied {
        host: "evil.com".to_string(),
        port: 443,
    };
    assert_eq!(net_err.category(), "Network");

    assert_eq!(PermissionError::EnvAccessDenied.category(), "Environment");
    assert_eq!(PermissionError::ProcessSpawnDenied.category(), "Process");
}
