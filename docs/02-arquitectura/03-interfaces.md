# 03 - INTERFACES AND COMMUNICATION

## 3.1 User Interface (CLI)

### 3.1.1 Invocation Format
```
3va <command> [options] [arguments]
```

### 3.1.2 Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Successful execution |
| 1 | General error |
| 2 | Argument parsing error |
| 3 | Permission error |
| 4 | Module error |
| 5 | Runtime error |
| 126 | Denied permission error |
| 127 | Command not found |

### 3.1.3 Output Format

#### Normal Mode
```
3va run app.ts
> Hello, World!
```

#### Verbose Mode
```
3va run app.ts -v
[DEBUG] Loading module: app.ts
[DEBUG] Checking permissions: FileRead(/path/app.ts)
[INFO] Module loaded successfully
> Hello, World!
```

#### JSON Mode (for scripting)
```json
{
  "success": true,
  "output": "Hello, World!",
  "exitCode": 0
}
```

## 3.2 Component Intercommunication

### 3.2.1 Core ↔ Permissions Interface

```rust
// Core requests permission verification
pub fn check_permission(&self, cap: &Capability) -> bool {
    self.permissions.check(cap)
}
```

### 3.2.2 Core ↔ JS Interface

```rust
// Core delegates execution to JS
pub fn execute(&self, code: &str) -> Result<Value> {
    self.js_engine.eval(code)
}
```

### 3.2.3 CLI ↔ Core Interface

```rust
// CLI builds runtime and executes it
pub fn run_with_permissions(cmd: Command, perms: PermissionState) -> Result<()> {
    let runtime = Runtime::with_permissions(perms);
    runtime.run_command(cmd).await
}
```

## 3.3 Event Interface

### 3.3.1 System Events

| Event | Description | Data |
|-------|-------------|------|
| runtime.start | Runtime start | timestamp, config |
| runtime.exit | Runtime termination | exit_code, duration |
| permission.check | Permission verification | capability, result |
| module.load | Module loading | path, type |
| module.error | Module error | path, error |
| fs.access | Filesystem access | path, operation, allowed |
| net.connect | Network connection | host, port, allowed |

### 3.3.2 Event Format
```rust
pub struct Event {
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub payload: serde_json::Value,
}
```

## 3.4 Extension Interface

### 3.4.1 Security Plugins
Plugins can intercept operations for additional analysis:

```rust
pub trait SecurityPlugin {
    fn on_permission_check(&mut self, cap: &Capability) -> CheckResult;
    fn on_module_load(&mut self, path: &Path) -> LoadResult;
    fn on_fs_access(&mut self, path: &Path, op: FsOp) -> AccessResult;
}
```

### 3.4.2 Lifecycle Hooks
```rust
pub trait LifecycleHook {
    fn pre_run(&mut self, config: &Config);
    fn post_run(&mut self, result: &RunResult);
    fn on_error(&mut self, error: &Error);
}
```

## 3.5 Configuration Interface

### 3.5.1 Configuration File
Location: `~/.3va/config.json` or `./.3va.json`

```json
{
  "permissions": {
    "defaults": {
      "allowRead": false,
      "allowWrite": false,
      "allowNet": false,
      "allowEnv": false,
      "allowChildProcess": false
    }
  },
  "pm": {
    "registry": "https://registry.npmjs.org",
    "postInstallScripts": false,
    "verifySignatures": true
  },
  "logging": {
    "level": "info",
    "format": "text"
  }
}
```

---

*Interfaces conforming to ISO/IEC/IEEE 24765 and software architecture.*
