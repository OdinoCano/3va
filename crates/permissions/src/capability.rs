use crate::audit::{AuditEvent, AuditLog};
use crate::scope::{self, ROOT_SCOPE};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Un permiso explícito para realizar una operación específica sobre un recurso.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Permite leer archivos en el path especificado (soporta prefijos).
    FileRead(PathBuf),
    /// Permite escribir archivos en el path especificado (soporta prefijos).
    FileWrite(PathBuf),
    /// Permite conexiones de red al host especificado (soporta wildcard `*.host`).
    Network(String),
    /// Permite crear procesos hijos.
    SpawnProcess,
    /// Permite leer todas las variables de entorno (equivale a --allow-env sin scope).
    EnvAccess,
    /// Permite leer una variable de entorno específica (--allow-env=VAR).
    /// `EnvAccess` (todas) cubre cualquier `EnvVar`; `EnvVar(a)` solo cubre `EnvVar(a)`.
    EnvVar(String),
    /// Permite llamadas FFI a librerías nativas en el path especificado.
    /// `FFI(PathBuf::from("/"))` equivale a `--allow-ffi` sin restricción de path.
    FFI(PathBuf),
}

use std::io::Write;
use std::sync::RwLock;

/// Estado de permisos del proceso, conforme al modelo deny-by-default.
///
/// Algoritmo de verificación:
/// 1. Si `deny_all_<tipo>` es true → DENY
/// 2. Si la capability está en `denied`  → DENY
/// 3. Si la capability está en `granted` → ALLOW
/// 4. Si `interactive` es true → PROMPT AL USUARIO
/// 5. Por defecto                        → DENY
#[derive(Debug, Default)]
pub struct PermissionState {
    /// Capabilities concedidas explícitamente por el usuario.
    pub granted: RwLock<HashSet<Capability>>,
    /// Capabilities denegadas explícitamente (tienen precedencia sobre granted).
    pub denied: RwLock<HashSet<Capability>>,

    /// Grants scoped to a specific dependency (`package.json["3va"].permissions.<name>`),
    /// keyed by package name. Checked in addition to (never instead of) the
    /// global `granted`/`denied` sets above — see [`crate::scope`] for how the
    /// "current" scope is determined at check() time.
    scoped_granted: RwLock<HashMap<String, HashSet<Capability>>>,
    /// Scoped denies — win over both scoped and global grants, same as the
    /// global `denied` set does.
    scoped_denied: RwLock<HashMap<String, HashSet<Capability>>>,

    /// Si está activado, lanza un prompt en consola cuando se detecta un permiso no configurado.
    pub interactive: bool,

    // Flags de denegación global por categoría
    deny_all_fs: bool,
    deny_all_net: bool,
    deny_all_env: bool,
    deny_all_process: bool,

    /// Shared audit log; when Some, every check() call appends an AuditEvent.
    pub audit_log: Option<Arc<Mutex<AuditLog>>>,
    /// When true, only denied checks are logged; when false, all checks are logged.
    pub audit_denied_only: bool,
}

impl PermissionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Activa el modo interactivo para preguntar al usuario al vuelo.
    pub fn set_interactive(&mut self, interactive: bool) {
        self.interactive = interactive;
    }

    /// Concede una capability. No la agrega si ya existe.
    pub fn grant(&self, cap: Capability) {
        self.granted.write().unwrap().insert(cap);
    }

    /// Deniega una capability explícita (tiene precedencia sobre `grant`).
    pub fn deny(&self, cap: Capability) {
        self.denied.write().unwrap().insert(cap);
    }

    /// Concede una capability solo cuando el código que ejecuta pertenece al
    /// scope dado (nombre de paquete, o [`ROOT_SCOPE`] para el código de la
    /// app). No amplía el scope global — un grant aquí para `"axios"` no
    /// aplica a ningún otro paquete.
    pub fn grant_scoped(&self, scope: &str, cap: Capability) {
        if scope == ROOT_SCOPE {
            return self.grant(cap);
        }
        self.scoped_granted
            .write()
            .unwrap()
            .entry(scope.to_string())
            .or_default()
            .insert(cap);
    }

    /// Deniega una capability solo dentro de un scope específico. Gana sobre
    /// cualquier grant (global o del mismo scope), igual que `deny`.
    pub fn deny_scoped(&self, scope: &str, cap: Capability) {
        if scope == ROOT_SCOPE {
            return self.deny(cap);
        }
        self.scoped_denied
            .write()
            .unwrap()
            .entry(scope.to_string())
            .or_default()
            .insert(cap);
    }

    /// Deniega toda la categoría de filesystem (lectura y escritura).
    pub fn deny_all_fs(&mut self) {
        self.deny_all_fs = true;
    }

    /// Deniega todo acceso de red.
    pub fn deny_all_net(&mut self) {
        self.deny_all_net = true;
    }

    /// Deniega todo acceso a variables de entorno.
    pub fn deny_all_env(&mut self) {
        self.deny_all_env = true;
    }

    /// Deniega la creación de cualquier proceso hijo.
    pub fn deny_all_process(&mut self) {
        self.deny_all_process = true;
    }

    /// Attach a shared AuditLog. Every subsequent check() call appends an event.
    /// Set `denied_only = true` to only record checks that were denied.
    pub fn enable_audit(&mut self, log: Arc<Mutex<AuditLog>>, denied_only: bool) {
        self.audit_log = Some(log);
        self.audit_denied_only = denied_only;
    }

    /// Retorna una copia de todas las capabilities concedidas actualmente.
    pub fn list_granted(&self) -> Vec<Capability> {
        self.granted.read().unwrap().iter().cloned().collect()
    }

    /// Verifica si una operación está permitida.
    ///
    /// Para paths de archivo y hosts de red, el matching es por prefijo/subdominio,
    /// no por igualdad exacta, reflejando el comportamiento documentado.
    pub fn check(&self, required: &Capability) -> bool {
        let result = self.check_inner(required);
        self.record_audit(required, result);
        result
    }

    fn check_inner(&self, required: &Capability) -> bool {
        // Paso 1: deny_all global por categoría
        match required {
            Capability::FileRead(_) | Capability::FileWrite(_) if self.deny_all_fs => {
                return false;
            }
            Capability::Network(_) if self.deny_all_net => {
                return false;
            }
            Capability::EnvAccess | Capability::EnvVar(_) if self.deny_all_env => {
                return false;
            }
            Capability::SpawnProcess if self.deny_all_process => {
                return false;
            }
            _ => {}
        }

        // Which package's code is currently executing (set by the JS engine's
        // require() wrapper; "." / ROOT_SCOPE for the app's own code, or when
        // no scoped grants were ever declared for this project).
        let active_scope = scope::current_scope();

        // Paso 2: deny-list explícita — global primero, luego la del scope activo.
        {
            let denied = self.denied.read().unwrap();
            if denied.iter().any(|d| caps_match(d, required)) {
                return false;
            }
        }
        if active_scope != ROOT_SCOPE {
            let scoped_denied = self.scoped_denied.read().unwrap();
            if let Some(set) = scoped_denied.get(&active_scope)
                && set.iter().any(|d| caps_match(d, required))
            {
                return false;
            }
        }

        // Paso 3: granted-list — global primero, luego la del scope activo.
        {
            let granted = self.granted.read().unwrap();
            if granted.iter().any(|g| caps_match(g, required)) {
                return true;
            }
        }
        if active_scope != ROOT_SCOPE {
            let scoped_granted = self.scoped_granted.read().unwrap();
            if let Some(set) = scoped_granted.get(&active_scope)
                && set.iter().any(|g| caps_match(g, required))
            {
                return true;
            }
        }

        // Paso 4: Modo interactivo
        if self.interactive {
            return self.prompt_user(required);
        }

        false
    }

    /// Check permission to *bind/listen* a local server on `host`, as opposed
    /// to connecting out to it.
    ///
    /// `http.createServer().listen(port)` and `net.createServer().listen(port)`
    /// default their bind host to `0.0.0.0` (all interfaces) regardless of
    /// which specific remote host the user wrote in `allow-net` — so a plain
    /// `check(&Capability::Network(host))` almost always fails even when the
    /// user clearly intended to let the script run its own server (they
    /// granted `allow-net: ["127.0.0.1"]` or some other host, not literally
    /// `"0.0.0.0"`). Running a server you wrote is a different risk than
    /// connecting out to arbitrary hosts, so any existing network grant is
    /// treated as authorization to bind on a local/wildcard address.
    ///
    /// This must never be reused for outbound checks (fetch/http.request/tcp
    /// connect) — there, the granted host must still match the real
    /// destination, or granting `allow-net: ["api.example.com"]` would also
    /// silently permit SSRF-style requests to `127.0.0.1`.
    pub fn check_bind(&self, host: &str) -> bool {
        let required = Capability::Network(host.to_string());

        if self.deny_all_net {
            self.record_audit(&required, false);
            return false;
        }
        {
            let denied = self.denied.read().unwrap();
            if denied.iter().any(|d| caps_match(d, &required)) {
                drop(denied);
                self.record_audit(&required, false);
                return false;
            }
        }
        if is_local_bind_host(host) {
            let granted = self.granted.read().unwrap();
            if granted.iter().any(|g| matches!(g, Capability::Network(_))) {
                drop(granted);
                self.record_audit(&required, true);
                return true;
            }
        }
        self.check(&required)
    }

    fn record_audit(&self, cap: &Capability, allowed: bool) {
        let log = match &self.audit_log {
            Some(l) => l,
            None => return,
        };
        if self.audit_denied_only && allowed {
            return;
        }
        let event = match cap {
            Capability::FileRead(p) => AuditEvent::FileAccess {
                timestamp: Utc::now(),
                path: p.clone(),
                operation: "read".to_string(),
                allowed,
            },
            Capability::FileWrite(p) => AuditEvent::FileAccess {
                timestamp: Utc::now(),
                path: p.clone(),
                operation: "write".to_string(),
                allowed,
            },
            Capability::Network(h) => AuditEvent::NetworkAccess {
                timestamp: Utc::now(),
                host: h.clone(),
                port: 0,
                allowed,
            },
            Capability::SpawnProcess => AuditEvent::ProcessSpawn {
                timestamp: Utc::now(),
                command: "*".to_string(),
                allowed,
            },
            Capability::EnvAccess => AuditEvent::EnvAccess {
                timestamp: Utc::now(),
                variable: "*".to_string(),
                allowed,
            },
            Capability::EnvVar(v) => AuditEvent::EnvAccess {
                timestamp: Utc::now(),
                variable: v.clone(),
                allowed,
            },
            Capability::FFI(p) => AuditEvent::PermissionDenied {
                timestamp: Utc::now(),
                capability: "FFI".to_string(),
                resource: p.display().to_string(),
                reason: if allowed {
                    "allowed".to_string()
                } else {
                    "not granted".to_string()
                },
            },
        };
        if let Ok(mut l) = log.lock() {
            l.add_event(event);
        }
    }

    fn prompt_user(&self, required: &Capability) -> bool {
        use std::io::IsTerminal;

        // Non-interactive contexts (pipes, CI, integration tests) have no TTY.
        // Blocking for input would hang indefinitely — deny silently instead.
        if !std::io::stdin().is_terminal() {
            self.deny(required.clone());
            return false;
        }

        let msg = match required {
            Capability::FileRead(p) => format!("read file '{}'", p.display()),
            Capability::FileWrite(p) => format!("write file '{}'", p.display()),
            Capability::Network(h) => format!("connect to network host '{h}'"),
            Capability::SpawnProcess => "spawn child processes".to_string(),
            Capability::EnvAccess => "access all environment variables".to_string(),
            Capability::EnvVar(v) => format!("access environment variable '{v}'"),
            Capability::FFI(p) => format!("call native library '{}'", p.display()),
        };

        eprint!(
            "\n[!] The script is requesting permission to {msg}.\nAllow? [y (once) / N (deny) / A (always)] "
        );
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let choice = input.trim();
            if choice.eq_ignore_ascii_case("y") {
                return true;
            } else if choice == "A" {
                self.grant(required.clone());
                return true;
            }
        }

        // Anything else (N or enter) is deny
        self.deny(required.clone());
        false
    }
}

impl Clone for PermissionState {
    fn clone(&self) -> Self {
        Self {
            granted: RwLock::new(self.granted.read().unwrap().clone()),
            denied: RwLock::new(self.denied.read().unwrap().clone()),
            scoped_granted: RwLock::new(self.scoped_granted.read().unwrap().clone()),
            scoped_denied: RwLock::new(self.scoped_denied.read().unwrap().clone()),
            interactive: self.interactive,
            deny_all_fs: self.deny_all_fs,
            deny_all_net: self.deny_all_net,
            deny_all_env: self.deny_all_env,
            deny_all_process: self.deny_all_process,
            audit_log: self.audit_log.clone(),
            audit_denied_only: self.audit_denied_only,
        }
    }
}

// Evalúa si la capability `granted` cubre a `required`.
// - FileRead/FileWrite: el path requerido debe comenzar con el path concedido.
// - Network: el host requerido debe coincidir exactamente o por wildcard *.host.
// - El resto: igualdad exacta.

// Strip Windows \\?\ extended-length path prefix so comparisons work
// regardless of whether the path was produced by canonicalize() or not.
#[cfg(windows)]
fn normalize_path(p: &std::path::Path) -> std::borrow::Cow<'_, std::path::Path> {
    let s = p.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        std::borrow::Cow::Owned(std::path::PathBuf::from(stripped))
    } else {
        std::borrow::Cow::Borrowed(p)
    }
}
#[cfg(not(windows))]
fn normalize_path(p: &std::path::Path) -> std::borrow::Cow<'_, std::path::Path> {
    std::borrow::Cow::Borrowed(p)
}

/// Resolve symlinks for path comparison — falls back to the original path when
/// canonicalize fails (e.g. path doesn't exist yet).
fn canon_path(p: &std::path::Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Check if `target` is covered by `allowed`, resolving symlinks on both sides
/// so that `/lib64` (symlink → `/usr/lib64`) matches `--allow-read=/lib64`.
fn path_covered_by(target: &std::path::Path, allowed: &std::path::Path) -> bool {
    let norm_t = normalize_path(target);
    let norm_a = normalize_path(allowed);
    if norm_t.starts_with(norm_a.as_ref()) {
        return true;
    }
    // Re-check after resolving symlinks on both sides.
    canon_path(norm_t.as_ref()).starts_with(canon_path(norm_a.as_ref()))
}

/// Wildcard/loopback addresses a server binds to when the app didn't ask for
/// a specific host — these describe *this machine*, not a remote target.
fn is_local_bind_host(host: &str) -> bool {
    matches!(host, "0.0.0.0" | "::" | "127.0.0.1" | "::1" | "localhost")
}

fn caps_match(granted: &Capability, required: &Capability) -> bool {
    match (granted, required) {
        (Capability::FileRead(allowed), Capability::FileRead(target)) => {
            path_covered_by(target, allowed)
        }
        (Capability::FileWrite(allowed), Capability::FileWrite(target)) => {
            path_covered_by(target, allowed)
        }
        (Capability::Network(allowed), Capability::Network(target)) => {
            host_matches(allowed, target)
        }
        // EnvAccess (all) covers both EnvAccess and any specific EnvVar.
        (Capability::EnvAccess, Capability::EnvAccess) => true,
        (Capability::EnvAccess, Capability::EnvVar(_)) => true,
        (Capability::EnvVar(a), Capability::EnvVar(b)) => a == b,
        // FFI: el path requerido debe comenzar con el path concedido (con symlinks).
        (Capability::FFI(allowed), Capability::FFI(target)) => path_covered_by(target, allowed),
        // Capabilities sin parámetros: igualdad de variante
        (a, b) => a == b,
    }
}

/// Matching de host/wildcard: `*.example.com` cubre `api.example.com`.
fn host_matches(pattern: &str, host: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern == host {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host.ends_with(suffix)
            && host.len() > suffix.len()
            && host.as_bytes()[host.len() - suffix.len() - 1] == b'.';
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_bind_allows_wildcard_bind_host_when_any_net_grant_exists() {
        // The common real-world case: package.json grants allow-net for a
        // specific host, but http.createServer().listen(port) with no host
        // defaults to binding "0.0.0.0" — that must still be allowed to run
        // the server the user clearly intended to permit.
        let state = PermissionState::new();
        state.grant(Capability::Network("127.0.0.1".to_string()));
        assert!(state.check_bind("0.0.0.0"));
        assert!(state.check_bind("127.0.0.1"));
        assert!(state.check_bind("localhost"));
    }

    #[test]
    fn check_bind_denies_without_any_net_grant() {
        let state = PermissionState::new();
        assert!(!state.check_bind("0.0.0.0"));
    }

    #[test]
    fn check_bind_respects_explicit_deny_net() {
        let state = PermissionState::new();
        state.grant(Capability::Network("127.0.0.1".to_string()));
        state.deny(Capability::Network("0.0.0.0".to_string()));
        assert!(!state.check_bind("0.0.0.0"));
    }

    #[test]
    fn check_bind_does_not_relax_outbound_connect_checks() {
        // Granting allow-net for one host must never let an *outbound*
        // connection reach a different host, even a loopback one (SSRF).
        // check_bind's relaxation is bind-only — plain check() must stay strict.
        let state = PermissionState::new();
        state.grant(Capability::Network("api.example.com".to_string()));
        assert!(!state.check(&Capability::Network("127.0.0.1".to_string())));
        assert!(!state.check(&Capability::Network("0.0.0.0".to_string())));
    }

    #[test]
    fn deny_by_default() {
        let state = PermissionState::new();
        assert!(!state.check(&Capability::EnvAccess));
        assert!(!state.check(&Capability::FileRead(PathBuf::from("/etc"))));
    }

    #[test]
    fn grant_allows() {
        let state = PermissionState::new();
        state.grant(Capability::EnvAccess);
        assert!(state.check(&Capability::EnvAccess));
    }

    #[test]
    fn deny_overrides_grant() {
        let state = PermissionState::new();
        state.grant(Capability::EnvAccess);
        state.deny(Capability::EnvAccess);
        assert!(!state.check(&Capability::EnvAccess));
    }

    #[test]
    fn deny_all_overrides_grant() {
        let mut state = PermissionState::new();
        state.grant(Capability::FileRead(PathBuf::from("/")));
        state.deny_all_fs();
        assert!(!state.check(&Capability::FileRead(PathBuf::from("/app"))));
    }

    #[test]
    fn path_prefix_matching() {
        let state = PermissionState::new();
        state.grant(Capability::FileRead(PathBuf::from("/app")));
        assert!(state.check(&Capability::FileRead(PathBuf::from("/app/config.json"))));
        assert!(!state.check(&Capability::FileRead(PathBuf::from("/etc/passwd"))));
    }

    #[test]
    fn wildcard_host_matching() {
        let state = PermissionState::new();
        state.grant(Capability::Network("*.example.com".to_string()));
        assert!(state.check(&Capability::Network("api.example.com".to_string())));
        assert!(!state.check(&Capability::Network("evil.com".to_string())));
        assert!(!state.check(&Capability::Network("example.com".to_string())));
    }

    // ── scoped (per-package) grants ────────────────────────────────────────────
    // These tests set the thread-local "current scope" directly, mirroring
    // what the JS engine's require() wrapper does before a package touches a
    // capability-gated builtin. Serialized via a mutex since thread-locals are
    // per-thread but `cargo test` may reuse the test-runner thread across
    // `#[test]` fns run serially in the same binary — reset scope on exit.

    #[test]
    fn scoped_grant_does_not_leak_to_other_scopes() {
        let state = PermissionState::new();
        state.grant_scoped("axios", Capability::Network("api.example.com".to_string()));

        crate::scope::set_current_scope("axios");
        assert!(state.check(&Capability::Network("api.example.com".to_string())));

        crate::scope::set_current_scope("express");
        assert!(!state.check(&Capability::Network("api.example.com".to_string())));

        crate::scope::set_current_scope(ROOT_SCOPE);
        assert!(!state.check(&Capability::Network("api.example.com".to_string())));
    }

    #[test]
    fn scoped_deny_overrides_global_grant_for_that_scope_only() {
        let state = PermissionState::new();
        state.grant(Capability::Network("*".to_string()));
        state.deny_scoped(
            "sketchy-pkg",
            Capability::Network("internal.corp".to_string()),
        );

        crate::scope::set_current_scope("sketchy-pkg");
        assert!(!state.check(&Capability::Network("internal.corp".to_string())));
        // The wildcard grant still covers a different host in that same scope.
        assert!(state.check(&Capability::Network("other.example.com".to_string())));

        crate::scope::set_current_scope("some-other-pkg");
        assert!(state.check(&Capability::Network("internal.corp".to_string())));

        crate::scope::set_current_scope(ROOT_SCOPE);
    }

    #[test]
    fn root_scope_grant_is_stored_globally_not_as_a_scope_entry() {
        let state = PermissionState::new();
        state.grant_scoped(ROOT_SCOPE, Capability::EnvAccess);
        assert!(
            state
                .granted
                .read()
                .unwrap()
                .contains(&Capability::EnvAccess)
        );
        assert!(state.scoped_granted.read().unwrap().is_empty());
    }
}
