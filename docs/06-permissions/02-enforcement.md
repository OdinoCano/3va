# 02 - POLICY ENFORCEMENT

## 2.1 Enforcement System

The enforcement system applies permission policies at runtime, intercepting sensitive operations and checking against PermissionState.

## 2.2 Interception Points

### 2.2.1 Verification Points

```
┌─────────────────────────────────────────────────────────────────┐
│                        User Code                              │
│         (JavaScript/TypeScript running in the runtime)        │
└─────────────────────────┬───────────────────────────────────────┘
                          │
    ┌─────────────────────┴─────────────────────┐
    │                                         │
    ▼                                         ▼
┌─────────────┐                        ┌─────────────┐
│  filesystem │                        │   network   │
│   .read()   │                        │  fetch()   │
└──────┬──────┘                        └──────┬──────┘
       │                                        │
       ▼                                        ▼
┌─────────────┐                        ┌─────────────┐
│   fs_hook   │                        │ net_hook    │
│  (verifier) │                        │ (verifier)  │
└──────┬──────┘                        └──────┬──────┘
       │                                        │
       ▼                                        ▼
┌───────────────────────────────────────────────────────────────┐
│                   PermissionState                             │
│              (capability verification)                        │
└───────────────────────────────────────────────────────────────┘
                          │
                     ┌────┴────┐
                     │ ALLOW   │ DENY
                     ▼         ▼
              ┌─────────┐  ┌─────────┐
               │ execute │  │ throw   │
               │ operation│  │ Security│
              └─────────┘  └─────────┘
```

### 2.2.2 Intercepted Operations

| Module | Operation | Required Capability |
|--------|-----------|---------------------|
| fs.readFile | Read file | FileRead |
| fs.writeFile | Write file | FileWrite |
| fs.readDir | List directory | FileRead |
| fetch | HTTP request | Network |
| net.connect | TCP/UDP | Network |
| process.env | Read environment | EnvAccess |
| child_process.spawn | Create process | SpawnProcess |

## 2.3 FileSystem Enforcement

### 2.3.1 Hook Implementation

```rust
pub struct FsEnforcer {
    permission_state: Arc<PermissionState>,
}

impl FsEnforcer {
    pub fn new(state: PermissionState) -> Self {
        Self {
            permission_state: Arc::new(state),
        }
    }

    pub fn check_read(&self, path: &Path) -> Result<(), PermissionError> {
        let cap = Capability::FileRead(path.to_path_buf());

        if self.permission_state.check(&cap) {
            Ok(())
        } else {
            Err(PermissionError::FileReadDenied {
                path: path.to_path_buf(),
            })
        }
    }

    pub fn check_write(&self, path: &Path) -> Result<(), PermissionError> {
        let cap = Capability::FileWrite(path.to_path_buf());

        if self.permission_state.check(&cap) {
            Ok(())
        } else {
            Err(PermissionError::FileWriteDenied {
                path: path.to_path_buf(),
            })
        }
    }

    pub fn check_read_recursive(&self, path: &Path) -> Result<(), PermissionError> {
        // For operations that read recursively
        // Verify the base path
        for cap in &self.permission_state.granted {
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
```

### 2.3.2 Polyfill Integration

```rust
// In the fs polyfill
pub fn read_file_sync(path: &str) -> Result<String, Error> {
    // 1. Check permissions
    enforcer.check_read(Path::new(path))?;

    // 2. If allowed, execute operation
    std::fs::read_to_string(path)
}
```

## 2.4 Network Enforcement

### 2.4.1 Network Verification

```rust
pub struct NetEnforcer {
    permission_state: Arc<PermissionState>,
}

impl NetEnforcer {
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

    pub fn check_url(&self, url: &Url) -> Result<(), PermissionError> {
        let host = url.host_str().ok_or_else(|| {
            PermissionError::InvalidUrl(url.to_string())
        })?;

        self.check_connect(host, url.port().unwrap_or(80))
    }
}
```

### 2.4.2 fetch Interception

```rust
// Fetch polyfill with verification
pub async fn secure_fetch(url: &str, init: RequestInit) -> Result<Response> {
    let parsed_url = Url::parse(url)?;

    // Check permission
    enforcer.check_url(&parsed_url)?;

    // Additional security validations
    validate_no_malicious_redirects(&parsed_url)?;
    validate_content_length(init.body)?;

    // Execute real fetch
    native_fetch(url, init).await
}
```

## 2.5 Environment Enforcement

### 2.5.1 Environment Variable Access

```rust
pub struct EnvEnforcer {
    permission_state: Arc<PermissionState>,
    allowed_vars: HashSet<String>,
}

impl EnvEnforcer {
    pub fn get(&self, key: &str) -> Result<Option<String>, PermissionError> {
        if !self.permission_state.check(&Capability::EnvAccess) {
            return Err(PermissionError::EnvAccessDenied);
        }

        // Optional: restrict allowed_vars
        if !self.allowed_vars.is_empty() && !self.allowed_vars.contains(key) {
            return Err(PermissionError::EnvVarNotAllowed(key.to_string()));
        }

        Ok(std::env::var(key).ok())
    }

    pub fn all(&self) -> Result<HashMap<String, String>, PermissionError> {
        if !self.permission_state.check(&Capability::EnvAccess) {
            return Err(PermissionError::EnvAccessDenied);
        }

        Ok(std::env::vars().collect())
    }
}
```

## 2.6 Proceso Enforcement

### 2.6.1 Process Spawn

```rust
pub struct ProcessEnforcer {
    permission_state: Arc<PermissionState>,
    allowed_commands: HashSet<String>,
}

impl ProcessEnforcer {
    pub fn spawn(&self, cmd: &str, args: &[String]) -> Result<(), PermissionError> {
        if !self.permission_state.check(&Capability::SpawnProcess) {
            return Err(PermissionError::ProcessSpawnDenied);
        }

        // Check if the command is on the whitelist
        if !self.allowed_commands.is_empty() && !self.allowed_commands.contains(cmd) {
            return Err(PermissionError::CommandNotAllowed(cmd.to_string()));
        }

        Ok(())
    }
}
```

## 2.7 Manejo de Errores

### 2.7.1 Error Types

```rust
#[derive(Error, Debug)]
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
```

### 2.7.2 Throw in JavaScript

```rust
// Convert permission error to JavaScript error
pub fn throw_permission_error(ctx: &Context, error: PermissionError) {
    ctx.with(|ctx| {
        let error_msg = error.to_string();
        let _ = ctx.eval(&format!(
            "throw new Error('{}: {}')",
            error.category(),
            error_msg
        ));
    });
}
```

## 2.8 Regulatory Compliance (ISO/IEC)

This Enforcers design is strictly aligned with **ISO/IEC 27002** information security controls:
- Network segmentation policies (`NetEnforcer`) ensure protection against unauthorized exposure.
- Filesystem interception (`FsEnforcer`) supports storage media protection compliance.
- The capability-based model ensures rigorous implementation of **Least Privilege** and **Defense in Depth** required by the standard.

---

*Fully implemented in `crates/permissions/src/enforcement.rs`.*