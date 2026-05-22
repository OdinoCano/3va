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

## 4.4 Planned Features (not yet implemented)

> **Status: PENDING** — the following are planned design, not current behavior.

### 4.4.1 CLI integration

```bash
# PLANNED — flags do not exist yet
3va run app.ts --audit-log=/var/log/3va/audit.log
3va run app.ts --audit-log=stdout
3va run app.ts --audit-level=errors

# PLANNED — sub-commands do not exist yet
3va audit view --file /var/log/3va/audit.log
3va audit view --category=denied --since="2026-05-18T10:00:00Z"
3va audit report --output=audit-report.html
3va audit stats --period=24h
```

### 4.4.2 Log rotation config

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

### 4.4.3 Automatic integration with `PermissionState`

Currently `AuditLogger` must be driven manually. Planned: `PermissionState::check` will fire audit events automatically on every allow/deny decision.

### 4.4.4 GDPR / ISO 27001

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
