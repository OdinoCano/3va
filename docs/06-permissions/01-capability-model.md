# 01 - CAPABILITY MODEL

## 1.1 Model Philosophy

3va's permission system implements a capability model based on the "deny-by-default" principle, where no process has access to system resources without an explicitly user-granted Capability.

## 1.2 Permission System Architecture

### 1.2.1 Component Diagram

```
┌────────────────────────────────────────────────────────────────┐
│                      CLI (Usuario)                            │
│              --allow-read --allow-net --allow-env             │
└─────────────────────────────────┬──────────────────────────────┘
                                  │
                                  ▼
┌────────────────────────────────────────────────────────────────┐
│                    PermissionState                            │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │ Granted Capabilities:                                    │  │
│  │   - FileRead(PathBuf)                                   │  │
│  │   - Network(String)                                     │  │
│  │   - EnvAccess                                           │  │
│   │   - (empty deny-list)                                   │  │
│  └─────────────────────────────────────────────────────────┘  │
└─────────────────────────────────┬──────────────────────────────┘
                                  │
          ┌───────────────────────┼───────────────────────┐
          ▼                       ▼                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   FileSystem   │    │   Network      │    │   Environment  │
│   Verifier     │    │   Verifier     │    │   Verifier     │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

## 1.3 Enum Capability

### 1.3.1 Definition

```rust
// crates/permissions/src/capability.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// Allows reading files at the specified path
    FileRead(PathBuf),
    /// Allows writing files at the specified path
    FileWrite(PathBuf),
    /// Allows network connections to the specified host/IP
    Network(String),
    /// Allows access to environment variables
    EnvAccess,
    /// Allows creating child processes
    SpawnProcess,
    /// Allows access to native APIs/FFI
    FFI,
}
```

### 1.3.2 Capability Descriptions

| Capability | Resource | Description |
|------------|----------|-------------|
| FileRead | PathBuf | Allows reading files/directories |
| FileWrite | PathBuf | Allows writing/creating files |
| Network | String | Allows TCP/UDP connections |
| EnvAccess | - | Allows reading environment variables |
| SpawnProcess | - | Allows creating child processes |
| FFI | - | Allows native function calls |

## 1.4 PermissionState

### 1.4.1 Structure

```rust
#[derive(Debug, Default)]
pub struct PermissionState {
    /// List of granted capabilities
    pub granted: Vec<Capability>,
    /// List of explicitly denied capabilities
    pub denied: Vec<Capability>,
    /// Global denial flags
    deny_all_fs: bool,
    deny_all_net: bool,
    deny_all_env: bool,
    deny_all_process: bool,
}

impl PermissionState {
    pub fn new() -> Self { ... }

    /// Grant a capability
    pub fn grant(&mut self, cap: Capability) { ... }

    /// Deny a specific capability
    pub fn deny(&mut self, cap: Capability) { ... }

    /// Check if an operation is allowed
    pub fn check(&self, required: &Capability) -> bool { ... }
}
```

### 1.4.2 Verification Algorithm

```
check(required_capability):
    1. IF deny_all_<type> IS true:
       RETURN false

    2. IF required_capability IS IN denied:
       RETURN false

    3. FOR each cap IN granted:
        4. IF cap MATCHES required_capability:
           RETURN true

    5. RETURN false  (deny-by-default)
```

## 1.5 Pattern Matching

### 1.5.1 File Patterns

```rust
// Glob pattern support for paths
impl Capability {
    pub fn matches_path(&self, path: &PathBuf) -> bool {
        match self {
            Capability::FileRead(allowed) => {
                // Exact match
                path.starts_with(allowed) ||
                // Glob patterns (future)
                matches_glob(path, allowed)
            }
            _ => false
        }
    }

    pub fn matches_glob(path: &Path, pattern: &Path) -> bool {
        // Glob matching implementation
        // *.js -> matches any .js file
        // /app/* -> matches anything in /app
        // /app/**/*.ts -> recursive .ts files
    }
}
```

### 1.5.2 Network Patterns

```rust
// Network pattern support
impl Capability {
    pub fn matches_host(&self, host: &str) -> bool {
        match self {
            Capability::Network(allowed) => {
                // Exact match
                host == allowed ||
                // Wildcard: *.example.com
                allowed.starts_with("*.") &&
                    host.ends_with(&allowed[1..]) ||
                // CIDR: 192.168.0.0/16 (future)
                matches_cidr(host, allowed)
            }
            _ => false
        }
    }
}
```

### 1.5.3 Matching Examples

| Pattern | Match | No Match |
|---------|-------|----------|
| `/app/*` | `/app/file.js` | `/app/sub/file.js` |
| `/app/**` | `/app/file.js`, `/app/sub/file.js` | `/other/file.js` |
| `*.example.com` | `api.example.com` | `example.com`, `evil.com` |
| `api.example.com` | `api.example.com` | `other.example.com` |

## 1.6 CLI Construction

### 1.6.1 Flag Parsing

```rust
pub fn from_args(args: &Args) -> PermissionState {
    let mut state = PermissionState::new();

    // --allow-read
    if args.flag_allow_read {
        // Allow all
        state.grant(Capability::FileRead(PathBuf::from("/")));
    } else if let Some(paths) = &args.flag_allow_read_paths {
        for path in paths {
            state.grant(Capability::FileRead(PathBuf::from(path)));
        }
    }

    // --allow-net
    if args.flag_allow_net {
        state.grant(Capability::Network("*".to_string()));
    } else if let Some(hosts) = &args.flag_allow_net_hosts {
        for host in hosts {
            state.grant(Capability::Network(host.clone()));
        }
    }

    // --allow-env
    if args.flag_allow_env {
        state.grant(Capability::EnvAccess);
    }

    // --allow-child-process
    if args.flag_allow_child_process {
        state.grant(Capability::SpawnProcess);
    }

    // --deny-* (revoke specific permissions)
    if args.flag_deny_env {
        state.deny(Capability::EnvAccess);
    }

    state
}
```

### 1.6.2 Presets

```rust
pub enum PermissionPreset {
    /// No permissions (deny-all)
    None,
    /// Equivalent to Node.js (allows everything)
    Node,
    /// Simulates browser
    Browser,
    /// Restricted environment
    Minimal,
}

impl PermissionPreset {
    pub fn apply(&self, state: &mut PermissionState) {
        match self {
            PermissionPreset::None => {
                // deny-by-default, no grants
            }
            PermissionPreset::Node => {
                state.grant(Capability::FileRead(PathBuf::from("/")));
                state.grant(Capability::FileWrite(PathBuf::from("/")));
                state.grant(Capability::Network("*".to_string()));
                state.grant(Capability::EnvAccess);
                state.grant(Capability::SpawnProcess);
            }
            PermissionPreset::Browser => {
                state.grant(Capability::Network("*".to_string()));
                state.grant(Capability::FileRead(PathBuf::from(".")));
                state.grant(Capability::FileWrite(PathBuf::from("./.cache")));
            }
            PermissionPreset::Minimal => {
                // Stdio only
            }
        }
    }
}
```

---

*Capability model based on Chrome Sandbox and QubesOS security principles.*