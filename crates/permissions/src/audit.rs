use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEvent {
    PermissionDenied {
        timestamp: DateTime<Utc>,
        capability: String,
        resource: String,
        reason: String,
    },
    PermissionGranted {
        timestamp: DateTime<Utc>,
        capability: String,
        resource: String,
    },
    FileAccess {
        timestamp: DateTime<Utc>,
        path: PathBuf,
        operation: String,
        allowed: bool,
    },
    NetworkAccess {
        timestamp: DateTime<Utc>,
        host: String,
        port: u16,
        allowed: bool,
    },
    ProcessSpawn {
        timestamp: DateTime<Utc>,
        command: String,
        allowed: bool,
    },
    EnvAccess {
        timestamp: DateTime<Utc>,
        variable: String,
        allowed: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub events: Vec<AuditEvent>,
}

impl AuditLog {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn add_event(&mut self, event: AuditEvent) {
        self.events.push(event);
    }

    pub fn log_permission_denied(&mut self, capability: &str, resource: &str, reason: &str) {
        self.add_event(AuditEvent::PermissionDenied {
            timestamp: Utc::now(),
            capability: capability.to_string(),
            resource: resource.to_string(),
            reason: reason.to_string(),
        });
    }

    pub fn log_file_access(&mut self, path: &Path, operation: &str, allowed: bool) {
        self.add_event(AuditEvent::FileAccess {
            timestamp: Utc::now(),
            path: path.to_path_buf(),
            operation: operation.to_string(),
            allowed,
        });
    }

    pub fn log_network_access(&mut self, host: &str, port: u16, allowed: bool) {
        self.add_event(AuditEvent::NetworkAccess {
            timestamp: Utc::now(),
            host: host.to_string(),
            port,
            allowed,
        });
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AuditLogger {
    log: AuditLog,
    enable_console: bool,
}

impl AuditLogger {
    pub fn new() -> Self {
        Self {
            log: AuditLog::new(),
            enable_console: false,
        }
    }

    pub fn with_console(mut self) -> Self {
        self.enable_console = true;
        self
    }

    pub fn log_denied(&mut self, capability: &str, resource: &str, reason: &str) {
        self.log.log_permission_denied(capability, resource, reason);
        if self.enable_console {
            eprintln!("[AUDIT] DENIED {} {} - {}", capability, resource, reason);
        }
    }

    pub fn log_file(&mut self, path: &Path, operation: &str, allowed: bool) {
        self.log.log_file_access(path, operation, allowed);
        if self.enable_console && !allowed {
            eprintln!(
                "[AUDIT] FILE {} {} - {}",
                operation,
                path.display(),
                if allowed { "allowed" } else { "denied" }
            );
        }
    }

    pub fn log_network(&mut self, host: &str, port: u16, allowed: bool) {
        self.log.log_network_access(host, port, allowed);
        if self.enable_console && !allowed {
            eprintln!(
                "[AUDIT] NETWORK {}:{} - {}",
                host,
                port,
                if allowed { "allowed" } else { "denied" }
            );
        }
    }

    pub fn get_log(&self) -> &AuditLog {
        &self.log
    }

    pub fn export(&self) -> String {
        self.log.to_json()
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_logger() {
        let mut logger = AuditLogger::new();
        logger.log_denied("FileRead", "/etc/passwd", "No permission granted");
        logger.log_network("evil.com", 443, false);

        let json = logger.export();
        assert!(json.contains("PermissionDenied"));
        assert!(json.contains("NetworkAccess"));
    }
}
