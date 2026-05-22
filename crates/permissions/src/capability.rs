use crate::audit::{AuditEvent, AuditLog};
use chrono::Utc;
use serde::{Deserialize, Serialize};
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
    /// Permite leer variables de entorno.
    EnvAccess,
    /// Permite llamadas FFI a librerías nativas.
    FFI,
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
    pub granted: RwLock<Vec<Capability>>,
    /// Capabilities denegadas explícitamente (tienen precedencia sobre granted).
    pub denied: RwLock<Vec<Capability>>,

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
        let mut granted = self.granted.write().unwrap();
        if !granted.contains(&cap) {
            granted.push(cap);
        }
    }

    /// Deniega una capability explícita (tiene precedencia sobre `grant`).
    pub fn deny(&self, cap: Capability) {
        let mut denied = self.denied.write().unwrap();
        if !denied.contains(&cap) {
            denied.push(cap);
        }
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
        self.granted.read().unwrap().clone()
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
            Capability::EnvAccess if self.deny_all_env => {
                return false;
            }
            Capability::SpawnProcess if self.deny_all_process => {
                return false;
            }
            _ => {}
        }

        // Paso 2: deny-list explícita
        {
            let denied = self.denied.read().unwrap();
            if denied.iter().any(|d| caps_match(d, required)) {
                return false;
            }
        }

        // Paso 3: granted-list
        {
            let granted = self.granted.read().unwrap();
            if granted.iter().any(|g| caps_match(g, required)) {
                return true;
            }
        }

        // Paso 4: Modo interactivo
        if self.interactive {
            return self.prompt_user(required);
        }

        false
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
            Capability::FFI => AuditEvent::PermissionDenied {
                timestamp: Utc::now(),
                capability: "FFI".to_string(),
                resource: "native".to_string(),
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

        // En scripts no interactivos (pipes, CI, integration tests) stdin no es un TTY.
        // Bloquear esperando input causaría un hang indefinido — denegar silenciosamente.
        if !std::io::stdin().is_terminal() {
            self.deny(required.clone());
            return false;
        }

        let msg = match required {
            Capability::FileRead(p) => format!("leer el archivo '{}'", p.display()),
            Capability::FileWrite(p) => format!("escribir el archivo '{}'", p.display()),
            Capability::Network(h) => format!("conectarse a la red '{}'", h),
            Capability::SpawnProcess => "crear procesos hijos".to_string(),
            Capability::EnvAccess => "acceder a variables de entorno".to_string(),
            Capability::FFI => "acceder a llamadas FFI nativas".to_string(),
        };

        eprint!(
            "\n[!] El script está intentando {msg}.\n¿Permitir? [y (Sí una vez) / N (Denegar) / A (Permitir Siempre)] "
        );
        let _ = std::io::stdout().flush();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let choice = input.trim();
            if choice.eq_ignore_ascii_case("y") {
                return true; // Permitido solo esta vez
            } else if choice == "A" {
                self.grant(required.clone());
                return true; // Permitido siempre
            }
        }

        // Cualquier otra cosa (N o enter) es deny
        self.deny(required.clone());
        false
    }
}

impl Clone for PermissionState {
    fn clone(&self) -> Self {
        Self {
            granted: RwLock::new(self.granted.read().unwrap().clone()),
            denied: RwLock::new(self.denied.read().unwrap().clone()),
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

/// Evalúa si la capability `granted` cubre a `required`.
///
/// - `FileRead`/`FileWrite`: el path requerido debe comenzar con el path concedido.
/// - `Network`: el host requerido debe coincidir exactamente o por wildcard `*.host`.
/// - El resto: igualdad exacta.
fn caps_match(granted: &Capability, required: &Capability) -> bool {
    match (granted, required) {
        (Capability::FileRead(allowed), Capability::FileRead(target)) => {
            target.starts_with(allowed)
        }
        (Capability::FileWrite(allowed), Capability::FileWrite(target)) => {
            target.starts_with(allowed)
        }
        (Capability::Network(allowed), Capability::Network(target)) => {
            host_matches(allowed, target)
        }
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
}
