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
    /// Allows reading files at the specified path (prefix match)
    FileRead(PathBuf),
    /// Allows writing files at the specified path (prefix match)
    FileWrite(PathBuf),
    /// Allows network connections to the specified host (exact or wildcard)
    Network(String),
    /// Allows reading ALL environment variables (--allow-env=)
    EnvAccess,
    /// Allows reading a specific environment variable by name (--allow-env=VAR).
    /// Covered by EnvAccess: if EnvAccess is granted, EnvVar(any) is also satisfied.
    EnvVar(String),
    /// Allows creating child processes
    SpawnProcess,
    /// Allows access to native APIs/FFI (path-scoped)
    FFI(PathBuf),
}
```

### 1.3.2 Capability Descriptions

| Capability | Parameter | CLI flag | Description |
|------------|-----------|----------|-------------|
| `FileRead` | `PathBuf` | `--allow-read=PATH` | Read access; granted path acts as a prefix |
| `FileWrite` | `PathBuf` | `--allow-write=PATH` | Write access; granted path acts as a prefix |
| `Network` | `String` | `--allow-net=HOST` | TCP/UDP; supports `*.domain` wildcards |
| `EnvAccess` | — | `--allow-env=` | Access to **all** environment variables |
| `EnvVar` | `String` | `--allow-env=VAR` | Access to one specific named variable |
| `SpawnProcess` | — | `--allow-child-process` | Spawn child processes |
| `FFI` | `PathBuf` | `--allow-ffi=PATH` | Native function calls (path-scoped library access) |

### 1.3.3 `EnvAccess` vs `EnvVar` coverage

`EnvAccess` is the superset capability. When granted, any `EnvVar(x)` check
automatically passes. The inverse is not true — a specific `EnvVar("NODE_ENV")`
does not allow reading any other variable.

```
EnvAccess  ──covers──►  EnvVar("NODE_ENV")
EnvAccess  ──covers──►  EnvVar("PATH")
EnvAccess  ──covers──►  EnvVar("SECRET_KEY")

EnvVar("NODE_ENV")  ──does NOT cover──►  EnvVar("PATH")
EnvVar("NODE_ENV")  ──does NOT cover──►  EnvAccess
```

Enforced in `caps_match` (`crates/permissions/src/capability.rs`):

```rust
// Hash-based (O(1)) lookups — granted and denied are HashSet<Capability>.
(Capability::EnvAccess, Capability::EnvAccess) => true,
(Capability::EnvAccess, Capability::EnvVar(_))  => true,   // all covers specific
(Capability::EnvVar(a), Capability::EnvVar(b))  => a == b, // exact name only
```

`process.env` is filtered at injection time — only variables whose name passes a
`PermissionState::check(&Capability::EnvVar(key))` are populated in the JS object.
Variables that were not granted are absent (not `undefined`, simply not present).

## 1.4 PermissionState

### 1.4.1 Structure

```rust
#[derive(Debug, Default)]
pub struct PermissionState {
    /// Set of granted capabilities (HashSet for O(1) lookups)
    pub granted: HashSet<Capability>,
    /// Set of explicitly denied capabilities (HashSet for O(1) lookups)
    pub denied: HashSet<Capability>,
    /// Global denial flags
    deny_all_fs: bool,
    deny_all_net: bool,
    deny_all_env: bool,
    deny_all_process: bool,
}

impl PermissionState {
    pub fn new() -> Self { ... }

    /// Grant a capability (HashSet::insert — no-op if already present)
    pub fn grant(&self, cap: Capability) { ... }

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

## 1.8 Permission Presets

`PermissionPreset` provides ready-made permission bundles for common scenarios:

| Preset | Grants |
|--------|--------|
| `Minimal` | Nothing — fully sandboxed baseline |
| `Development` | FileRead + FileWrite (cwd), Network (`*`), EnvAccess, SpawnProcess |
| `Production` | FileRead (cwd) only — no write, no network, no env, no processes |
| `NetworkOnly` | Network (`*`) only |

```rust
use vvva_permissions::PermissionPreset;

// Ready-made state
let state = PermissionPreset::Development.into_state();

// Or apply to an existing state (additive)
PermissionPreset::NetworkOnly.apply(&existing_state);
```

---

*Capability model based on Chrome Sandbox and QubesOS security principles.*