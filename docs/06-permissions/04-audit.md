# 04 - AUDIT AND LOGGING

## 4.1 Audit System

3va implements a complete audit system that logs all sensitive operations for regulatory compliance and forensic analysis.

## 4.2 Eventos Auditados

### 4.2.1 Event Categories

| Category | Description | Events |
|----------|-------------|--------|
| permission | Permission checks | check, allow, deny |
| fs | File system operations | read, write, delete, mkdir |
| network | Network operations | connect, send, receive |
| process | Process creation | spawn, exec |
| env | Environment variable access | get, set |
| module | Module loading | load, resolve |
| security | Security events | blocked, flagged |

### 4.2.2 Event Format

```rust
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,           // RFC 3339
    pub event_id: Uuid,                      // Unique identifier
    pub category: AuditCategory,             // permission, fs, network, etc.
    pub action: String,                      // read, write, connect, etc.
    pub resource: Option<String>,            // Path, URL, etc.
    pub principal: Principal,                // User/session info
    pub decision: AuditDecision,             // allow, deny
    pub reason: Option<String>,              // Why it was allowed/denied
    pub metadata: HashMap<String, String>,   // Additional data
    pub source: EventSource,                 // CLI, API, module
}

pub enum AuditDecision {
    Allow,
    Deny(String),  // Denial reason
}

pub enum EventSource {
    UserCode,
    Builtin,
    Package(String),
    CLI,
}
```

### 4.2.3 Event Example

```json
{
  "timestamp": "2026-05-18T14:30:00.123Z",
  "eventId": "550e8400-e29b-41d4-a716-446655440000",
  "category": "fs",
  "action": "read",
  "resource": "/app/config.json",
  "principal": {
    "user": "root",
    "session": "abc123"
  },
  "decision": "allow",
  "source": "userCode",
  "metadata": {
    "mode": "sync",
    "size": "1024"
  }
}
```

## 4.3 Audit Configuration

### 4.3.1 Logging Levels

| Level | Events Logged |
|-------|---------------|
| off | None |
| errors | Only denials and errors |
| warnings | errors + warnings |
| info | warnings + main operations |
| debug | info + all details |
| trace | debug + debugging information |

### 4.3.2 Configuration

```rust
pub struct AuditConfig {
    pub level: AuditLevel,
    pub destinations: Vec<AuditDestination>,
    pub retention_days: u32,
    pub max_file_size: u64,
    pub rotate: bool,
    pub filters: AuditFilters,
}

pub enum AuditDestination {
    File(PathBuf),
    Stdout,
    Stderr,
    Syslog,
    Custom(Box<dyn AuditSink>),
}

pub struct AuditFilters {
    pub categories: Vec<AuditCategory>,
    pub min_decision: AuditDecision,  // Only allow decisions >= level
    pub resources: Option<Vec<String>>,  // Filter by specific resources
}
```

### 4.3.2 CLI Configuration

```bash
# Log to file
3va run app.ts --audit-log=/var/log/3va/audit.log

# Log to stdout
3va run app.ts --audit-log=stdout

# Detail level
3va run app.ts --audit-level=info --audit-log=/var/log/3va/audit.log

# Filter only denials
3va run app.ts --audit-level=errors
```

## 4.4 Implementation

### 4.4.1 Audit Logger

```rust
pub struct AuditLogger {
    config: AuditConfig,
    writer: Box<dyn Write>,
    formatter: AuditFormatter,
}

impl AuditLogger {
    pub fn log(&self, event: AuditEvent) {
        // 1. Filter according to config
        if !self.should_log(&event) {
            return;
        }

        // 2. Format
        let formatted = self.formatter.format(&event);

        // 3. Write
        if let Err(e) = self.writer.write(formatted) {
            eprintln!("Audit log write failed: {}", e);
        }
    }

    fn should_log(&self, event: &AuditEvent) -> bool {
        // Check level
        if !event.category.enabled_at(self.config.level) {
            return false;
        }

        // Check filters
        if let Some(resources) = &self.config.filters.resources {
            if let Some(resource) = &event.resource {
                return resources.iter().any(|r| resource.contains(r));
            }
        }

        true
    }
}
```

### 4.4.2 Permissions Integration

```rust
// En PermissionState
pub fn check_with_audit(&self, cap: &Capability) -> bool {
    let decision = if self.check(cap) {
        AuditDecision::Allow
    } else {
        AuditDecision::Deny("No matching capability".to_string())
    };

    audit::log(AuditEvent {
        category: AuditCategory::Permission,
        action: "check".to_string(),
        resource: Some(format!("{:?}", cap)),
        decision,
        ..Default::default()
    });

    decision == AuditDecision::Allow
}
```

## 4.5 Log Rotation

### 4.5.1 Configuration

```rust
pub struct LogRotation {
    pub max_size: u64,        // Maximum size per file
    pub max_files: u32,       // Maximum number of files
    pub compress: bool,       // Compress old files
}

impl LogRotation {
    pub fn should_rotate(&self, current_size: u64) -> bool {
        current_size >= self.max_size
    }

    pub fn rotate(&self, path: &Path) -> std::io::Result<Vec<PathBuf>> {
        // 1. Rename current file to .1
        // 2. Compress old files if enabled
        // 3. Delete files > max_files
    }
}
```

## 4.6 Regulatory Compliance

### 4.6.1 GDPR

```rust
// For GDPR compliance:
// - Logging of personal data access
// - Configurable retention
// - Right to deletion

pub struct GdprConfig {
    pub log_personal_data_access: bool,
    pub personal_data_patterns: Vec<Regex>,
    pub retention_days: u32,
    pub right_to_deletion: bool,
}
```

### 4.6.2 ISO 27001

```rust
// ISO 27001 compliance:
// - Security auditing
// - Traceability
// - Non-repudiation

pub struct Iso27001Config {
    pub log_all_security_events: bool,
    pub log_access_control: bool,
    pub immutability: bool,  // Logs cannot be modified
    pub integrity_check: bool,  // Log checksum
}
```

## 4.7 Analysis Tools

### 4.7.1 Audit CLI

```bash
# View audit logs
3va audit view --file /var/log/3va/audit.log

# Filter by category
3va audit view --category=denied

# Filter by time
3va audit view --since="2026-05-18T10:00:00Z"

# Generate report
3va audit report --output=audit-report.html

# Statistics
3va audit stats --period=24h
```

### 4.7.2 Log Aggregation

```rust
// Aggregation from multiple sources
pub struct AuditAggregator {
    sources: Vec<Box<dyn AuditSource>>,
}

impl AuditAggregator {
    pub fn query(&self, query: AuditQuery) -> Vec<AuditEvent> {
        // Aggregate events from multiple sources
        // and return unified results
    }
}
```

---

*Audit compliant with ISO 27001, GDPR, and security standards.*

## 4.8 Implementation Status (May 2026)

### ✅ Implemented

```rust
// crates/permissions/src/audit.rs
pub struct AuditLogger {
    log: AuditLog,
    enable_console: bool,
}

impl AuditLogger {
    pub fn log_denied(&mut self, capability: &str, resource: &str, reason: &str);
    pub fn log_file(&mut self, path: &PathBuf, operation: &str, allowed: bool);
    pub fn log_network(&mut self, host: &str, port: u16, allowed: bool);
    pub fn export(&self) -> String;  // Exports to JSON
}
```

### 📋 Pending
- Log rotation
- Audit CLI (`3va audit`)
- Automatic integration with PermissionState
- GDPR/ISO 27001 configs