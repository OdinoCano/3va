# 04 - AUDIT AND LOGGING

## 4.1 Overview

3va's audit system logs permission decisions and I/O operations. The implementation is in `crates/permissions/src/audit.rs`.

## 4.2 Implemented Types

### `AuditEvent` (enum)

```rust
// crates/permissions/src/audit.rs
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
```

### `AuditLog`

Append-only log of `AuditEvent` values. Serializes to JSON.

```rust
pub struct AuditLog {
    pub events: Vec<AuditEvent>,
}

impl AuditLog {
    pub fn add_event(&mut self, event: AuditEvent)
    pub fn log_permission_denied(&mut self, capability: &str, resource: &str, reason: &str)
    pub fn log_file_access(&mut self, path: &Path, operation: &str, allowed: bool)
    pub fn log_network_access(&mut self, host: &str, port: u16, allowed: bool)
    pub fn to_json(&self) -> String
    pub fn write_to_file(&self, path: &Path) -> std::io::Result<()>
}
```

### `AuditLogger`

Wraps `AuditLog` and optionally mirrors events to stderr.

```rust
pub struct AuditLogger {
    log: AuditLog,
    enable_console: bool,
}

impl AuditLogger {
    pub fn new() -> Self
    pub fn with_console(mut self) -> Self       // enable stderr mirroring
    pub fn log_denied(&mut self, capability: &str, resource: &str, reason: &str)
    pub fn log_file(&mut self, path: &Path, operation: &str, allowed: bool)
    pub fn log_network(&mut self, host: &str, port: u16, allowed: bool)
    pub fn get_log(&self) -> &AuditLog
    pub fn export(&self) -> String              // JSON export
}
```

## 4.3 JSON Output Format

`AuditLog::to_json()` serializes to a JSON array. Each event follows the shape of its variant:

```json
[
  {
    "FileAccess": {
      "timestamp": "2026-05-22T14:30:00.123Z",
      "path": "/app/config.json",
      "operation": "read",
      "allowed": true
    }
  },
  {
    "PermissionDenied": {
      "timestamp": "2026-05-22T14:30:01.000Z",
      "capability": "Network",
      "resource": "api.example.com",
      "reason": "host not in --allow-net list"
    }
  }
]
```

## 4.4 CLI Integration

`PermissionState` integrates with the audit log automatically via `check()`. Every permission decision fires an `AuditEvent` when audit logging is enabled.

```bash
# Write denied-checks only (default)
3va run app.ts --audit-log=./audit.json

# Write all checks (allowed + denied)
3va run app.ts --audit-log=./audit.json --audit-level=all
```

The log is written as JSON to the specified path after execution completes.

### Enabling from Rust

```rust
use std::sync::{Arc, Mutex};
use vvva_permissions::{PermissionState, AuditLog};

let log = Arc::new(Mutex::new(AuditLog::new()));
let mut permissions = PermissionState::new();
permissions.enable_audit(log.clone(), /* denied_only */ true);

// ... run code ...

let log = log.lock().unwrap();
log.write_to_file(std::path::Path::new("audit.json")).unwrap();
println!("{}", log.to_json());
```

## 4.5 Planned Features (not yet implemented)

> **Status: PENDING** — the following are planned design, not current behavior.

### 4.5.1 Log rotation config

```rust
// PLANNED — AuditConfig does not exist yet
pub struct AuditConfig {
    pub level: AuditLevel,              // off, errors, warnings, info, debug, trace
    pub destinations: Vec<AuditDestination>, // File(path), Stdout, Stderr, Syslog
    pub retention_days: u32,
    pub max_file_size: u64,
    pub rotate: bool,
}
```

### 4.5.2 GDPR / ISO 27001

```rust
// PLANNED — not implemented
pub struct GdprConfig {
    pub log_personal_data_access: bool,
    pub retention_days: u32,
    pub right_to_deletion: bool,
}

pub struct Iso27001Config {
    pub log_all_security_events: bool,
    pub immutability: bool,   // append-only, no modification
    pub integrity_check: bool, // checksum per log file
}
```

---

*Implemented in `crates/permissions/src/audit.rs`.*
