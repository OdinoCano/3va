use std::path::PathBuf;
use serde::{Serialize, Deserialize};

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

/// Estado de permisos del proceso, conforme al modelo deny-by-default.
///
/// Algoritmo de verificación:
/// 1. Si `deny_all_<tipo>` es true → DENY
/// 2. Si la capability está en `denied`  → DENY
/// 3. Si la capability está en `granted` → ALLOW
/// 4. Por defecto                        → DENY
#[derive(Debug, Default, Clone)]
pub struct PermissionState {
    /// Capabilities concedidas explícitamente por el usuario.
    pub granted: Vec<Capability>,
    /// Capabilities denegadas explícitamente (tienen precedencia sobre granted).
    pub denied: Vec<Capability>,

    // Flags de denegación global por categoría
    deny_all_fs: bool,
    deny_all_net: bool,
    deny_all_env: bool,
    deny_all_process: bool,
}

impl PermissionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Concede una capability. No la agrega si ya existe.
    pub fn grant(&mut self, cap: Capability) {
        if !self.granted.contains(&cap) {
            self.granted.push(cap);
        }
    }

    /// Deniega una capability explícita (tiene precedencia sobre `grant`).
    pub fn deny(&mut self, cap: Capability) {
        if !self.denied.contains(&cap) {
            self.denied.push(cap);
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

    /// Verifica si una operación está permitida.
    ///
    /// Para paths de archivo y hosts de red, el matching es por prefijo/subdominio,
    /// no por igualdad exacta, reflejando el comportamiento documentado.
    pub fn check(&self, required: &Capability) -> bool {
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
        if self.denied.iter().any(|d| caps_match(d, required)) {
            return false;
        }

        // Paso 3: granted-list
        self.granted.iter().any(|g| caps_match(g, required))
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
        let mut state = PermissionState::new();
        state.grant(Capability::EnvAccess);
        assert!(state.check(&Capability::EnvAccess));
    }

    #[test]
    fn deny_overrides_grant() {
        let mut state = PermissionState::new();
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
        let mut state = PermissionState::new();
        state.grant(Capability::FileRead(PathBuf::from("/app")));
        assert!(state.check(&Capability::FileRead(PathBuf::from("/app/config.json"))));
        assert!(!state.check(&Capability::FileRead(PathBuf::from("/etc/passwd"))));
    }

    #[test]
    fn wildcard_host_matching() {
        let mut state = PermissionState::new();
        state.grant(Capability::Network("*.example.com".to_string()));
        assert!(state.check(&Capability::Network("api.example.com".to_string())));
        assert!(!state.check(&Capability::Network("evil.com".to_string())));
        assert!(!state.check(&Capability::Network("example.com".to_string())));
    }
}
