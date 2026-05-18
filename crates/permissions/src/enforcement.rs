use std::path::PathBuf;
use std::sync::Arc;
use crate::capability::{Capability, PermissionState};

#[derive(Debug, Clone, thiserror::Error)]
pub enum PermissionError {
    #[error("Permission denied: FileRead({path})")]
    FileReadDenied { path: PathBuf },

    #[error("Permission denied: FileWrite({path})")]
    FileWriteDenied { path: PathBuf },

    #[error("Permission denied: Network({host}:{port})")]
    NetworkDenied { host: String, port: u16 },

    #[error("Permission denied: EnvAccess")]
    EnvAccessDenied,

    #[error("Permission denied: ProcessSpawn")]
    ProcessSpawnDenied,

    #[error("Environment variable not allowed: {0}")]
    EnvVarNotAllowed(String),

    #[error("Command not allowed: {0}")]
    CommandNotAllowed(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}

impl PermissionError {
    pub fn category(&self) -> &'static str {
        match self {
            PermissionError::FileReadDenied { .. } => "FileSystem",
            PermissionError::FileWriteDenied { .. } => "FileSystem",
            PermissionError::NetworkDenied { .. } => "Network",
            PermissionError::EnvAccessDenied => "Environment",
            PermissionError::ProcessSpawnDenied => "Process",
            PermissionError::EnvVarNotAllowed(_) => "Environment",
            PermissionError::CommandNotAllowed(_) => "Process",
            PermissionError::InvalidUrl(_) => "Network",
        }
    }
}

pub struct FsEnforcer {
    permission_state: Arc<PermissionState>,
}

impl FsEnforcer {
    pub fn new(state: PermissionState) -> Self {
        Self {
            permission_state: Arc::new(state),
        }
    }

    pub fn from_arc(state: Arc<PermissionState>) -> Self {
        Self { permission_state: state }
    }

    pub fn check_read(&self, path: &std::path::Path) -> Result<(), PermissionError> {
        let cap = Capability::FileRead(path.to_path_buf());
        if self.permission_state.check(&cap) {
            Ok(())
        } else {
            Err(PermissionError::FileReadDenied {
                path: path.to_path_buf(),
            })
        }
    }

    pub fn check_write(&self, path: &std::path::Path) -> Result<(), PermissionError> {
        let cap = Capability::FileWrite(path.to_path_buf());
        if self.permission_state.check(&cap) {
            Ok(())
        } else {
            Err(PermissionError::FileWriteDenied {
                path: path.to_path_buf(),
            })
        }
    }

    pub fn check_read_recursive(&self, path: &std::path::Path) -> Result<(), PermissionError> {
        let granted = self.permission_state.granted.read().unwrap();
        for cap in granted.iter() {
            if let Capability::FileRead(allowed) = cap {
                if path.starts_with(allowed) || allowed.starts_with(path) {
                    return Ok(());
                }
            }
        }

        Err(PermissionError::FileReadDenied {
            path: path.to_path_buf(),
        })
    }
}

pub struct NetEnforcer {
    permission_state: Arc<PermissionState>,
}

impl NetEnforcer {
    pub fn new(state: PermissionState) -> Self {
        Self {
            permission_state: Arc::new(state),
        }
    }

    pub fn from_arc(state: Arc<PermissionState>) -> Self {
        Self { permission_state: state }
    }

    pub fn check_connect(&self, host: &str, port: u16) -> Result<(), PermissionError> {
        let cap = Capability::Network(host.to_string());
        if self.permission_state.check(&cap) {
            Ok(())
        } else {
            Err(PermissionError::NetworkDenied {
                host: host.to_string(),
                port,
            })
        }
    }

    pub fn check_url(&self, url: &url::Url) -> Result<(), PermissionError> {
        let host = url.host_str().ok_or_else(|| {
            PermissionError::InvalidUrl(url.to_string())
        })?;

        self.check_connect(host, url.port().unwrap_or(80))
    }
}

pub struct EnvEnforcer {
    permission_state: Arc<PermissionState>,
    allowed_vars: std::collections::HashSet<String>,
}

impl EnvEnforcer {
    pub fn new(state: PermissionState) -> Self {
        Self {
            permission_state: Arc::new(state),
            allowed_vars: std::collections::HashSet::new(),
        }
    }

    pub fn from_arc(state: Arc<PermissionState>) -> Self {
        Self {
            permission_state: state,
            allowed_vars: std::collections::HashSet::new(),
        }
    }

    pub fn with_allowed_vars(mut self, vars: Vec<String>) -> Self {
        self.allowed_vars = vars.into_iter().collect();
        self
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, PermissionError> {
        if !self.permission_state.check(&Capability::EnvAccess) {
            return Err(PermissionError::EnvAccessDenied);
        }

        if !self.allowed_vars.is_empty() && !self.allowed_vars.contains(key) {
            return Err(PermissionError::EnvVarNotAllowed(key.to_string()));
        }

        Ok(std::env::var(key).ok())
    }

    pub fn all(&self) -> Result<std::collections::HashMap<String, String>, PermissionError> {
        if !self.permission_state.check(&Capability::EnvAccess) {
            return Err(PermissionError::EnvAccessDenied);
        }

        Ok(std::env::vars().collect())
    }
}

pub struct ProcessEnforcer {
    permission_state: Arc<PermissionState>,
    allowed_commands: std::collections::HashSet<String>,
}

impl ProcessEnforcer {
    pub fn new(state: PermissionState) -> Self {
        Self {
            permission_state: Arc::new(state),
            allowed_commands: std::collections::HashSet::new(),
        }
    }

    pub fn from_arc(state: Arc<PermissionState>) -> Self {
        Self {
            permission_state: state,
            allowed_commands: std::collections::HashSet::new(),
        }
    }

    pub fn with_allowed_commands(mut self, commands: Vec<String>) -> Self {
        self.allowed_commands = commands.into_iter().collect();
        self
    }

    pub fn spawn(&self, cmd: &str, _args: &[String]) -> Result<(), PermissionError> {
        if !self.permission_state.check(&Capability::SpawnProcess) {
            return Err(PermissionError::ProcessSpawnDenied);
        }

        if !self.allowed_commands.is_empty() && !self.allowed_commands.contains(cmd) {
            return Err(PermissionError::CommandNotAllowed(cmd.to_string()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fs_enforcer_read_allowed() {
        let state = PermissionState::new();
        state.grant(Capability::FileRead(std::path::PathBuf::from("/app")));

        let enforcer = FsEnforcer::new(state);
        let result = enforcer.check_read(std::path::Path::new("/app/config.json"));

        assert!(result.is_ok());
    }

    #[test]
    fn test_fs_enforcer_read_denied() {
        let state = PermissionState::new();
        let enforcer = FsEnforcer::new(state);
        let result = enforcer.check_read(std::path::Path::new("/etc/passwd"));

        assert!(result.is_err());
    }

    #[test]
    fn test_net_enforcer_allowed() {
        let state = PermissionState::new();
        state.grant(Capability::Network("api.example.com".to_string()));

        let enforcer = NetEnforcer::new(state);
        let result = enforcer.check_connect("api.example.com", 443);

        assert!(result.is_ok());
    }

    #[test]
    fn test_net_enforcer_denied() {
        let state = PermissionState::new();
        let enforcer = NetEnforcer::new(state);
        let result = enforcer.check_connect("evil.com", 443);

        assert!(result.is_err());
    }

    #[test]
    fn test_env_enforcer_allowed() {
        let state = PermissionState::new();
        state.grant(Capability::EnvAccess);

        let enforcer = EnvEnforcer::new(state);
        let result = enforcer.get("PATH");

        assert!(result.is_ok());
    }

    #[test]
    fn test_env_enforcer_denied() {
        let state = PermissionState::new();
        let enforcer = EnvEnforcer::new(state);
        let result = enforcer.get("PATH");

        assert!(result.is_err());
    }

    #[test]
    fn test_process_enforcer_allowed() {
        let state = PermissionState::new();
        state.grant(Capability::SpawnProcess);

        let enforcer = ProcessEnforcer::new(state);
        let result = enforcer.spawn("ls", &["-la".to_string()]);

        assert!(result.is_ok());
    }

    #[test]
    fn test_process_enforcer_denied() {
        let state = PermissionState::new();
        let enforcer = ProcessEnforcer::new(state);
        let result = enforcer.spawn("ls", &["-la".to_string()]);

        assert!(result.is_err());
    }
}