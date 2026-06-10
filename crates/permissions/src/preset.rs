use crate::capability::{Capability, PermissionState};
use std::path::PathBuf;

/// Predefined permission bundles for common runtime scenarios.
///
/// Each preset configures a [`PermissionState`] with a sensible set of
/// capabilities. Use [`PermissionPreset::apply`] to populate a state, or
/// [`PermissionPreset::into_state`] to get a ready-made one.
///
/// # Examples
///
/// ```
/// use vvva_permissions::PermissionPreset;
///
/// let state = PermissionPreset::Development.into_state();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionPreset {
    /// No permissions granted. Equivalent to running with no `--allow-*` flags.
    /// Useful as a baseline for tests or sandboxed evaluation.
    Minimal,

    /// Broad permissions for local development: full filesystem read/write
    /// under the current working directory, unrestricted network, all env vars,
    /// and child-process spawning.
    Development,

    /// Conservative permissions for production deployments: read-only
    /// filesystem access under the current working directory, no env access,
    /// no child-process spawning. Network must be explicitly added.
    Production,

    /// Network access only — no filesystem, no env, no process spawning.
    /// Suitable for pure HTTP/WebSocket services whose code is pre-bundled.
    NetworkOnly,
}

impl PermissionPreset {
    /// Apply this preset to an existing [`PermissionState`].
    pub fn apply(&self, state: &PermissionState) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        match self {
            PermissionPreset::Minimal => {
                // Nothing to grant — deny-by-default already covers this.
            }

            PermissionPreset::Development => {
                state.grant(Capability::FileRead(cwd.clone()));
                state.grant(Capability::FileWrite(cwd));
                state.grant(Capability::Network("*".to_string()));
                state.grant(Capability::EnvAccess);
                state.grant(Capability::SpawnProcess);
            }

            PermissionPreset::Production => {
                state.grant(Capability::FileRead(cwd));
                // No write, no env, no process spawning, no network by default.
            }

            PermissionPreset::NetworkOnly => {
                state.grant(Capability::Network("*".to_string()));
            }
        }
    }

    /// Create a new [`PermissionState`] pre-configured with this preset.
    pub fn into_state(self) -> PermissionState {
        let state = PermissionState::new();
        self.apply(&state);
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_denies_everything() {
        let state = PermissionPreset::Minimal.into_state();
        assert!(!state.check(&Capability::Network("example.com".into())));
        assert!(!state.check(&Capability::EnvAccess));
        assert!(!state.check(&Capability::SpawnProcess));
    }

    #[test]
    fn development_grants_broad_access() {
        let state = PermissionPreset::Development.into_state();
        assert!(state.check(&Capability::Network("example.com".into())));
        assert!(state.check(&Capability::EnvAccess));
        assert!(state.check(&Capability::SpawnProcess));
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        assert!(state.check(&Capability::FileRead(cwd.clone())));
        assert!(state.check(&Capability::FileWrite(cwd)));
    }

    #[test]
    fn production_allows_read_only() {
        let state = PermissionPreset::Production.into_state();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        assert!(state.check(&Capability::FileRead(cwd)));
        assert!(!state.check(&Capability::EnvAccess));
        assert!(!state.check(&Capability::SpawnProcess));
        assert!(!state.check(&Capability::Network("example.com".into())));
    }

    #[test]
    fn network_only_allows_net_denies_fs() {
        let state = PermissionPreset::NetworkOnly.into_state();
        assert!(state.check(&Capability::Network("api.example.com".into())));
        assert!(!state.check(&Capability::EnvAccess));
        assert!(!state.check(&Capability::SpawnProcess));
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        assert!(!state.check(&Capability::FileRead(cwd)));
    }

    #[test]
    fn apply_is_additive() {
        let state = PermissionPreset::NetworkOnly.into_state();
        PermissionPreset::Production.apply(&state);
        assert!(state.check(&Capability::Network("x.io".into())));
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        assert!(state.check(&Capability::FileRead(cwd)));
    }
}
