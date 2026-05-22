# 02 - COMPONENT DESIGN

## 2.1 Component: vvva_core

### 2.1.1 Description
The core component provides the async runtime based on Tokio, managing the main event loop, task scheduling, and coordination between components.

### 2.1.2 Structure

```rust
pub struct Runtime {
    pub permissions: PermissionState,
    event_loop: EventLoop,
    scheduler: Scheduler,
    module_cache: ModuleCache,
}
```

### 2.1.3 Responsibilities
- Initialization and management of the async event loop
- Concurrent task scheduling
- Coordination of loaded modules
- Process lifecycle management

### 2.1.4 Interfaces

#### run()
```rust
pub async fn run(&self) -> anyhow::Result<()>
```
Starts the main event loop and waits for tasks to complete.

#### spawn_task()
```rust
pub fn spawn_task(&self, task: Task) -> Handle
```
Creates a new async task and returns a handle to control it.

### 2.1.5 Dependencies
- tokio (async runtime)
- vvva_permissions (capability verification)
- vvva_js (code execution)

## 2.2 Component: vvva_cli

### 2.2.1 Description
The CLI component provides the command line interface, parsing arguments and routing commands to the appropriate components.

### 2.2.2 Structure

```rust
pub struct Cli {
    command: Command,
    permissions: PermissionState,
    config: Config,
}
```

### 2.2.3 Supported Subcommands

| Command | Description | Example |
|---------|-------------|---------|
| run | Executes a JS/TS file | `3va run app.ts` |
| install | Installs a package | `3va install axios` |
| test | Runs tests | `3va test` |
| build | Bundles code | `3va build index.ts` |
| eval | Evaluates inline code | `3va eval "console.log(1)"` |

### 2.2.4 Permission Flags

| Flag | Description | Example |
|------|-------------|---------|
| --allow-read | Allows file reading | `--allow-read=/app` |
| --allow-write | Allows file writing | `--allow-write=/tmp` |
| --allow-net | Allows network access | `--allow-net=api.example.com` |
| --allow-env | Allows environment variable access | `--allow-env` |
| --allow-child-process | Allows process spawning | `--allow-child-process` |
| --deny-* | Denies a specific permission | `--deny-env` |

## 2.3 Component: vvva_permissions

### 2.3.1 Description
The permission system implements the capability model, storing and verifying the permissions granted by the user.

### 2.3.2 Structure

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    FileRead(PathBuf),
    FileWrite(PathBuf),
    Network(String), // Hostname or IP
    SpawnProcess,
    EnvAccess,
}

pub struct PermissionState {
    pub granted: Vec<Capability>,
}
```

### 2.3.3 Verification Algorithm

```
1. Receive operation request (op_type, resource)
2. FOR each capability IN granted:
    3. IF capability matches op_type AND resource:
       4. RETURN ALLOW
5. RETURN DENY
```

### 2.3.4 Pattern Matching

Network and file permissions support glob patterns:
- `*.example.com` - any subdomain
- `/app/*` - any file in /app
- `192.168.*` - any IP in the range

## 2.4 Component: vvva_js

### 2.4.1 Description
The JS component integrates QuickJS, providing JavaScript and TypeScript execution with support for modules and web APIs.

### 2.4.2 Structure

```rust
pub struct JsEngine {
    runtime: Runtime,
    context: Context,
    module_loader: ModuleLoader,
    polyfills: PolyfillRegistry,
}
```

### 2.4.3 Features
- JavaScript/TypeScript code execution
- ESM and CommonJS support
- Polyfills for Node.js APIs
- Standard web APIs (fetch, WebSocket, etc.)

## 2.5 Component: vvva_pm

### 2.5.1 Description
The package manager handles dependency installation with security verification.

### 2.5.2 Structure

```rust
pub struct PackageManager {
    registry: RegistryClient,
    cache: PackageCache,
    verifier: SignatureVerifier,
    sandbox: Sandbox,
}
```

### 2.5.3 Security Policies
- Post-install scripts: Disabled by default
- Packages untrusted until verification
- Execution in isolated sandbox

## 2.6 Component: vvva_bundler [TO BE IMPLEMENTED]

### 2.6.1 Description
The bundler transpiles and packages TypeScript/JSX code for distribution.

### 2.6.2 Planned Features
- TSX/TS to JS transpilation
- Tree shaking
- Code splitting
- Source maps

---

*Design conforming to IEEE 1012 and component architecture.*
