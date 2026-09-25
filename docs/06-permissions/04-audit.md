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

## 4.5 Denial Summary (no flag required)

An audit log is opt-in and answers "what happened"; a *denial summary* answers
"what do I have to grant to make this work", which is the question a user
actually has when a run fails. Every `check()` that returns false records the
capability in a `DenialTally` (first-refused order + hit count), and `3va run`
prints it before propagating the run's own error — a run that failed *because*
of a denial is exactly the run whose denials you need to see.

```
[!] 2 permissions were denied during this run:
    read /etc/hosts (denied 2x)
      grant with: --allow-read=/etc/hosts
    connect to api.example.com:8443
      grant with: --allow-net=api.example.com:8443
```

Notes:

- **Deduped, counted.** Repeats of the same capability are counted but not
  re-listed. `(denied 2x)` separates one incidental read from a retry loop
  hammering the same wall 400 times — the latter is a bug in the script, and
  the count is what makes that visible.
- **The flag, not a description.** `vvva_permissions::grant_flag()` returns the
  exact flag to add. "network access to api.example.com:8443" is something you
  read; `--allow-net=api.example.com:8443` is something you paste.
- **Wasm and `--prof` runs report too**, so the summary is not a JS-only
  feature.
- **Binds are reported separately.** A refused `server.listen()` says
  `bind a local server on 0.0.0.0` and that it needs *any* `--allow-net` grant —
  not `grant with: --allow-net=0.0.0.0`, which would point at a destination the
  script never connects to. `0.0.0.0` is where a local server binds, not a host
  anyone reaches out to.
- **Nothing is granted.** The summary is a report; it never widens permissions
  on its own. Use `3va permissions learn --write` to record the requirements.

`--trace-denials` switches to reporting each refusal the moment it happens
(`  [denied] write ./out.txt`) instead of one block at the end, deduplicated
the same way. Use it when the summary can't tell you *which* call was refused
— e.g. a library that reads an optional config file at several layers.

The tally is per-`PermissionState`, and `Clone` starts a fresh one: the summary
describes the run that is finishing, not every check made since process start.

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
