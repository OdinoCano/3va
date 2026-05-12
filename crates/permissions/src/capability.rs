use std::path::PathBuf;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    FileRead(PathBuf),
    FileWrite(PathBuf),
    Network(String), // Hostname or IP
    SpawnProcess,
    EnvAccess,
}

#[derive(Debug, Default)]
pub struct PermissionState {
    pub granted: Vec<Capability>,
}

impl PermissionState {
    pub fn new() -> Self {
        Self {
            granted: Vec::new(),
        }
    }

    pub fn grant(&mut self, cap: Capability) {
        if !self.granted.contains(&cap) {
            self.granted.push(cap);
        }
    }

    pub fn check(&self, required: &Capability) -> bool {
        // Simplified check: exact match. In a real system, path and network glob matching would happen here.
        self.granted.contains(required)
    }
}
