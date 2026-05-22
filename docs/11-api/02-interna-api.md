# 02 - RUNTIME INTERNAL API

## 2.1 Internal APIs

APIs available for extension and plugin development.

## 2.2 Runtime Core

```rust
// crates/core/src/lib.rs
pub struct Runtime {
    pub permissions: PermissionState,
}

// Methods
impl Runtime {
    pub fn new() -> Self
    pub async fn run(&self) -> anyhow::Result<()>
    pub fn spawn_task(&self, task: Task) -> Handle
}
```

## 2.3 Permissions

```rust
// crates/permissions/src/lib.rs
pub struct PermissionState {
    pub granted: Vec<Capability>,
}

impl PermissionState {
    pub fn new() -> Self
    pub fn grant(&mut self, cap: Capability)
    pub fn check(&self, required: &Capability) -> bool
}
```

## 2.4 JS Engine

```rust
// crates/js/src/lib.rs
pub struct JsEngine {
    runtime: Runtime,
    context: Context,
}

impl JsEngine {
    pub fn new(permissions: &PermissionState) -> anyhow::Result<Self>
    pub fn eval(&self, code: &str) -> anyhow::Result<()>
    pub fn eval_module(&self, code: &str, path: &str) -> anyhow::Result<Value>
}
```

## 2.5 Capability Enum

```rust
pub enum Capability {
    FileRead(PathBuf),
    FileWrite(PathBuf),
    Network(String),
    EnvAccess,
    SpawnProcess,
    FFI,
}
```

## 2.6 Package Manager

```rust
// crates/pm/src/lib.rs
pub async fn install_package(name: &str) -> anyhow::Result<()>
pub struct PackageManifest { ... }
pub struct PackageInfo { ... }
```

---

*Internal API for extensions and contributions.*
