# 02 - API INTERNA DEL RUNTIME

## 2.1 APIs Internas

APIs disponibles para desarrollo de extensiones y plugins.

## 2.2 Runtime Core

```rust
// crates/core/src/lib.rs
pub struct Runtime {
    pub permissions: PermissionState,
}

// Métodos
impl Runtime {
    pub fn new() -> Self
    pub async fn run(&self) -> anyhow::Result<()>
    pub fn spawn_task(&self, task: Task) -> Handle
}
```

## 2.3 Permisos

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

## 2.4 Motor JS

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

*API interna para extensiones y contribuciones.*