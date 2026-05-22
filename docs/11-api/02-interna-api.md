# 02 - RUNTIME INTERNAL API

## 2.1 Overview

APIs for the runtime components. All signatures reflect the actual code in `crates/`.

## 2.2 Runtime Core (`crates/core`)

```rust
// crates/core/src/lib.rs
pub struct Runtime {
    pub permissions: PermissionState,
    task_queue: TaskQueue,
    timer_wheel: TimerWheel,
}

impl Runtime {
    pub fn new(permissions: PermissionState) -> Self

    /// Drive the async runtime to completion.
    pub async fn run(&self) -> anyhow::Result<()>

    /// Enqueue a future onto the task queue.
    pub fn schedule_task<F>(&mut self, task_type: TaskType, future: F)
    where F: Future<Output = ()> + Send + 'static

    /// Register a one-shot timer callback.
    pub fn set_timeout<F>(&mut self, delay: Duration, callback: F) -> TimerId
    where F: Fn() + Send + 'static

    /// Register a repeating timer callback.
    pub fn set_interval<F>(&mut self, interval: Duration, callback: F) -> TimerId
    where F: Fn() + Send + 'static

    /// Cancel a timer by id.
    pub fn clear_timeout(&mut self, id: TimerId) -> bool

    /// Fire all timers whose deadline has passed. Returns the fired timers.
    pub fn poll_timers(&mut self) -> Vec<Timer>

    /// Duration until the next timer fires, or None if no timers pending.
    pub fn next_timer_duration(&self) -> Option<Duration>

    /// Number of tasks currently in the queue.
    pub fn pending_task_count(&self) -> usize
}
```

## 2.3 Permissions (`crates/permissions`)

```rust
// crates/permissions/src/lib.rs
pub enum Capability {
    FileRead(PathBuf),
    FileWrite(PathBuf),
    Network(String),   // hostname or IP
    EnvAccess,
    SpawnProcess,
    FFI,
}

pub struct PermissionState { /* internal */ }

impl PermissionState {
    pub fn new() -> Self
    pub fn grant(&self, cap: Capability)
    pub fn check(&self, required: &Capability) -> bool
    pub fn set_interactive(&mut self, interactive: bool)
}
```

## 2.4 JS Engine (`crates/js`)

```rust
// crates/js/src/lib.rs
pub struct JsEngine { /* internal — QuickJS runtime + context */ }

impl JsEngine {
    /// Create a new engine with the given permission state.
    pub async fn new(permissions: Arc<PermissionState>) -> anyhow::Result<Self>

    /// Evaluate a string of JS/TS code (auto-transpiles .ts).
    pub async fn eval(&self, code: &str) -> anyhow::Result<()>

    /// Load and execute a file (detects .ts/.tsx and transpiles).
    pub async fn eval_file(&self, path: &Path) -> anyhow::Result<()>

    /// Evaluate code and return its string representation.
    pub async fn eval_to_string(&self, code: &str) -> anyhow::Result<String>

    /// Drive the integrated event loop until all timers and microtasks complete.
    pub async fn run_event_loop(&self) -> anyhow::Result<()>
}
```

## 2.5 Bundler (`crates/bundler`)

```rust
// crates/bundler/src/lib.rs
pub enum OutputFormat { Iife, Umd, Cjs, Esm }

pub struct BundlerOptions {
    pub format: OutputFormat,   // default: Iife
    pub minify: bool,
    pub sourcemap: bool,
    pub split: bool,
}

pub struct Bundler { /* internal */ }

impl Bundler {
    pub fn new(root: PathBuf) -> Self
    pub fn with_options(self, options: BundlerOptions) -> Self
    pub fn add_entry(&mut self, path: &str) -> anyhow::Result<()>
    pub fn bundle(&mut self) -> anyhow::Result<String>
    pub fn bundle_with_sourcemap(&mut self) -> anyhow::Result<(String, Option<String>)>
}

/// Bundle a single file to an output path.
pub fn bundle_file(
    input: &str,
    output: &str,
    options: Option<BundlerOptions>,
) -> anyhow::Result<()>

/// Bundle once then watch for changes and re-bundle.
pub fn start_watch_mode(
    input: &Path,
    output: &Path,
    options: Option<BundlerOptions>,
) -> anyhow::Result<()>
```

## 2.6 Package Manager (`crates/pm`)

```rust
// crates/pm/src/lib.rs
pub struct PackageManager { /* internal */ }

impl PackageManager {
    pub fn new(cache_dir: PathBuf) -> Self
    pub fn install(/* ... */) -> anyhow::Result<()>
    pub fn load_lockfile(path: &Path) -> anyhow::Result<Lockfile>
    pub fn save_lockfile(&self, lockfile: &Lockfile, path: &Path) -> anyhow::Result<()>
}

/// Run a 3-phase audit (malware + OSV CVE + secrets) in the current directory.
pub async fn run_audit(force_refresh: bool) -> anyhow::Result<AuditReport>

/// Run audit in a specific project directory.
pub async fn run_audit_in(force_refresh: bool, project_dir: &Path) -> anyhow::Result<AuditReport>

pub fn print_audit_report(report: &AuditReport, deny: bool) -> bool
```

---

*Internal API — subject to change between minor versions until 1.0.0 LTS.*
