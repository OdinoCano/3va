//! JavaScript engine crate — wraps V8 via the `v8` crate, exposes `JsEngine` and all built-in modules.

pub mod async_context;
pub mod builtins;
pub mod esm;
pub mod inspector;
pub mod profiler;
pub mod rejection_tracker;
pub mod transpiler;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use v8::Isolate;
use vvva_core::Runtime;
use vvva_firewall::Firewall;
use vvva_permissions::PermissionState;

use builtins::TimerManager;
use profiler::Profiler;

static INSPECTOR_STATE_CELL: std::sync::OnceLock<Arc<inspector::InspectorState>> =
    std::sync::OnceLock::new();
static PROFILER_HANDLE_CELL: std::sync::OnceLock<Profiler> = std::sync::OnceLock::new();

// V8 only supports being initialized once per process; a second
// `v8::V8::initialize()` call (e.g. from a second `JsEngine` in the same
// process, as happens constantly across `#[tokio::test]` functions) corrupts
// global V8 state ("Invalid global state" panics deep in the v8 crate).
static V8_INIT: std::sync::Once = std::sync::Once::new();

// Retained so `run_event_loop`/`idle` can pump V8's own background→foreground
// task queue (used by e.g. async WebAssembly compilation). Without this,
// `WebAssembly.instantiate()`'s promise never settles: the microtask
// checkpoint alone does not run tasks V8 posts to the platform.
static V8_PLATFORM: std::sync::OnceLock<v8::SharedRef<v8::Platform>> = std::sync::OnceLock::new();

/// Initializes the V8 platform, if it hasn't been already. Safe to call any
/// number of times from any number of places in the process — e.g. once per
/// `JsEngine`, plus any standalone `v8::Isolate` created outside of one
/// (like the CJS export-name probe in `vvva_cli`).
pub fn ensure_v8_initialized() {
    V8_INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform.clone());
        v8::V8::initialize();
        let _ = V8_PLATFORM.set(platform);
    });
}

/// Runs one ready V8 platform task (foreground or background-completed) for
/// `isolate`, if any is pending. Deliberately not looped to exhaustion: a
/// task that reposts more work could otherwise spin this call forever. The
/// caller (idle()/run_event_loop()) already runs repeatedly, so tasks drain
/// incrementally across iterations just like timers do.
fn pump_v8_platform_tasks(isolate: &v8::Isolate) {
    if let Some(platform) = V8_PLATFORM.get() {
        v8::Platform::pump_message_loop(platform, isolate, false);
    }
}

pub struct JsEngine {
    isolate: v8::OwnedIsolate,
    context: Option<v8::Global<v8::Context>>,
    _permissions: Arc<PermissionState>,
    timer_manager: Arc<TimerManager>,
    runtime_core: Mutex<Runtime>,
    inspector: Option<Arc<inspector::InspectorState>>,
    profiler: Option<Profiler>,
    profiler_interval_ms: u32,
    ws_pool: builtins::websocket::WsPool,
}

impl JsEngine {
    pub async fn new(permissions: Arc<PermissionState>) -> anyhow::Result<Self> {
        Self::new_full(permissions, None, None, None).await
    }

    pub async fn new_with_firewall(
        permissions: Arc<PermissionState>,
        firewall: Arc<Firewall>,
    ) -> anyhow::Result<Self> {
        Self::new_full(permissions, None, None, Some(firewall)).await
    }

    pub async fn new_with_inspector(
        permissions: Arc<PermissionState>,
        inspect_addr: Option<SocketAddr>,
    ) -> anyhow::Result<Self> {
        Self::new_full(permissions, inspect_addr, None, None).await
    }

    pub async fn new_with_firewall_and_inspector(
        permissions: Arc<PermissionState>,
        firewall: Arc<Firewall>,
        inspect_addr: Option<SocketAddr>,
    ) -> anyhow::Result<Self> {
        Self::new_full(permissions, inspect_addr, None, Some(firewall)).await
    }

    pub async fn new_with_profiler(
        permissions: Arc<PermissionState>,
        interval_ms: u32,
    ) -> anyhow::Result<Self> {
        Self::new_full(permissions, None, Some(interval_ms), None).await
    }

    async fn new_full(
        permissions: Arc<PermissionState>,
        inspect_addr: Option<SocketAddr>,
        prof_interval_ms: Option<u32>,
        firewall: Option<Arc<Firewall>>,
    ) -> anyhow::Result<Self> {
        ensure_v8_initialized();

        let mut isolate = Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);

        let timer_manager = TimerManager::new();
        let runtime_core = Mutex::new(Runtime::new((*permissions).clone()));

        let inspector = inspect_addr.map(inspector::start);
        let profiler = prof_interval_ms.map(|_| Profiler::new());

        let ws_pool: builtins::websocket::WsPool =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        let mut engine = Self {
            isolate,
            context: None,
            _permissions: permissions.clone(),
            timer_manager: timer_manager.clone(),
            runtime_core,
            inspector,
            profiler,
            profiler_interval_ms: prof_interval_ms.unwrap_or(100),
            ws_pool: ws_pool.clone(),
        };

        engine.initialize(permissions, timer_manager, firewall, ws_pool)?;

        Ok(engine)
    }

    fn initialize(
        &mut self,
        permissions: Arc<PermissionState>,
        timer_manager: Arc<TimerManager>,
        firewall: Option<Arc<Firewall>>,
        ws_pool: builtins::websocket::WsPool,
    ) -> anyhow::Result<()> {
        let inspector_state = self.inspector.clone();
        let interval_ms = self.profiler_interval_ms;
        let profiler = self.profiler.clone();

        rejection_tracker::install(&mut self.isolate);
        let mut handle_scope_storage = Box::pin(v8::HandleScope::new(&mut *self.isolate));
        let mut handle_scope = handle_scope_storage.as_mut().init();
        let context = v8::Context::new(&handle_scope, Default::default());
        self.context = Some(v8::Global::new(&handle_scope, context));
        let mut scope = v8::ContextScope::new(&mut handle_scope, context);

        async_context::install(&mut scope, &permissions)?;

        builtins::inject_all(&mut scope, permissions, timer_manager, firewall, ws_pool)?;

        if let Some(state) = inspector_state {
            INSPECTOR_STATE_CELL.set(state).ok();
            let callback = v8::Function::new(
                &mut scope,
                move |_scope: &mut v8::PinScope,
                      _args: v8::FunctionCallbackArguments,
                      _rv: v8::ReturnValue| {
                    let s = INSPECTOR_STATE_CELL.get().unwrap().clone();
                    tokio::task::block_in_place(move || s.pause());
                },
            )
            .unwrap();
            let context = scope.get_current_context();
            let global = context.global(&scope);
            let key = v8::String::new(&scope, "__3va_debugger__").unwrap().into();
            global.set(&scope, key, callback.into());
        }

        if let (Some(js_src), Some(handle)) = (
            profiler
                .as_ref()
                .map(|_| profiler::profiler_js(interval_ms)),
            profiler.clone(),
        ) {
            PROFILER_HANDLE_CELL.set(handle).ok();
            let callback = v8::Function::new(
                &mut scope,
                move |scope: &mut v8::PinScope,
                      args: v8::FunctionCallbackArguments,
                      mut _rv: v8::ReturnValue| {
                    let ts = args.get(0).uint32_value(scope).unwrap_or(0);
                    let stack = args.get(1).to_rust_string_lossy(scope);
                    let label = args.get(2);
                    let lbl = if label.is_null_or_undefined() {
                        None
                    } else {
                        Some(label.to_rust_string_lossy(scope))
                    };
                    PROFILER_HANDLE_CELL
                        .get()
                        .unwrap()
                        .push_raw(ts as u64, &stack, lbl);
                },
            )
            .unwrap();
            let context = scope.get_current_context();
            let global = context.global(&scope);
            let key = v8::String::new(&scope, "__profilerPush").unwrap().into();
            global.set(&scope, key, callback.into());

            let src = v8::String::new(&scope, &js_src).unwrap();
            let _ = v8::Script::compile(&scope, src, None).and_then(|s| s.run(&scope));
        }

        Ok(())
    }

    pub async fn drain_ws_connections(&self) {
        let pool = self.ws_pool.clone();
        tokio::task::spawn_blocking(move || {
            builtins::websocket::drain_ws_pool(&pool, std::time::Duration::from_secs(30));
        })
        .await
        .ok();
    }

    pub async fn eval(&mut self, code: &str) -> anyhow::Result<()> {
        let code = code.to_string();
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let scope = v8::ContextScope::new(&mut scope, context);
        let source = v8::String::new(&scope, &code).unwrap();
        let script = v8::Script::compile(&scope, source, None)
            .ok_or_else(|| anyhow::anyhow!("compile error"))?;
        let _result = script
            .run(&scope)
            .ok_or_else(|| anyhow::anyhow!("execution error"))?;
        Ok(())
    }

    pub async fn eval_to_string(&mut self, code: &str) -> anyhow::Result<String> {
        let code = code.to_string();
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let scope = v8::ContextScope::new(&mut scope, context);
        let source = v8::String::new(&scope, &code).unwrap();
        let script = v8::Script::compile(&scope, source, None)
            .ok_or_else(|| anyhow::anyhow!("compile error"))?;
        let result = script
            .run(&scope)
            .ok_or_else(|| anyhow::anyhow!("execution error"))?;
        Ok(result.to_rust_string_lossy(&scope))
    }

    pub async fn idle(&mut self) {
        pump_v8_platform_tasks(&self.isolate);
        self.isolate.perform_microtask_checkpoint();
    }

    pub async fn with_scope<R>(
        &mut self,
        f: impl FnOnce(&mut v8::ContextScope<v8::HandleScope>) -> R,
    ) -> R {
        let context_global = self.context.clone().expect("engine not initialized");
        let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
        let mut scope = scope.init();
        let context = v8::Local::new(&scope, &context_global);
        let mut scope = v8::ContextScope::new(&mut scope, context);
        f(&mut scope)
    }

    pub async fn eval_file_with_args(
        &mut self,
        path: &Path,
        args: &[String],
    ) -> anyhow::Result<()> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let filename = canonical.to_string_lossy().to_string();
        let script_arg = filename.replace('\\', "\\\\").replace('"', "\\\"");
        let extra: String = args
            .iter()
            .map(|a| format!(", \"{}\"", a.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect();
        let inject = format!(
            "if (globalThis.process && Array.isArray(globalThis.process.argv)) \
             {{ globalThis.process.argv = [globalThis.process.argv[0], \"{script_arg}\"{extra}]; }}"
        );
        self.eval(&inject).await?;
        self.eval_file(path).await
    }

    pub async fn eval_file(&mut self, path: &Path) -> anyhow::Result<()> {
        let source = std::fs::read_to_string(path)?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        // Entry-point files with real static import/export syntax must go
        // through transpile_to_cjs (ESM→CJS conversion), not plain
        // transpile()/transpile_js() — those only strip TS types/JSX and
        // leave import/export untouched, which V8 rejects when the file is
        // run as a classic script (not a compiled ES Module).
        let is_esm = ext == "mjs" || is_esm_source(&source);
        let transpiled = if is_esm {
            transpiler::transpile_to_cjs(&source, matches!(ext, "tsx" | "jsx"))
        } else {
            match ext {
                "tsx" | "jsx" => transpiler::transpile_jsx(&source),
                "ts" | "mts" | "cts" => transpiler::transpile(&source),
                _ => {
                    if transpiler::looks_like_jsx(&source) {
                        transpiler::transpile_js(&source)
                    } else {
                        source
                    }
                }
            }
        };

        let code = if self.inspector.is_some() {
            inspector::rewrite_debugger_statements(&transpiled).into_owned()
        } else {
            transpiled
        };

        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let filename = canonical.to_string_lossy().to_string();
        let dirname = canonical
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let meta_url = url::Url::from_file_path(&canonical)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", filename.replace('\\', "/")));

        let code = transpiler::replace_import_meta(&code);

        {
            let context_global = self.context.clone().expect("engine not initialized");
            let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
            let mut scope = scope.init();
            let context = v8::Local::new(&scope, &context_global);
            let scope = v8::ContextScope::new(&mut scope, context);

            let f = filename.replace('\\', "\\\\").replace('\'', "\\'");
            let d = dirname.replace('\\', "\\\\").replace('\'', "\\'");
            let u = meta_url.replace('\\', "\\\\").replace('\'', "\\'");

            let setup = format!(
                "globalThis.__filename = '{f}'; globalThis.__dirname = '{d}';\
             globalThis.__vvva_meta_url__ = '{u}';\
             globalThis.__vvva_meta_env__ = (typeof process !== 'undefined' ? \
               Object.assign(Object.create(null), \
                 {{ MODE: (process.env && process.env.NODE_ENV) || 'production', \
                    PROD: (process.env && process.env.NODE_ENV) !== 'development', \
                    DEV:  (process.env && process.env.NODE_ENV) === 'development', \
                    SSR:  true, \
                    BASE_URL: '/' }}, process.env) : \
               {{ MODE: 'production', PROD: true, DEV: false, SSR: true, BASE_URL: '/' }});\
             if (typeof globalThis.__vvva_meta_resolve__ === 'undefined') \
               globalThis.__vvva_meta_resolve__ = function(s) {{ return require.resolve(s); }};\
             if (typeof globalThis.__vvva_meta_glob__ === 'undefined') \
               globalThis.__vvva_meta_glob__ = function() {{ return {{}}; }};\
             if (globalThis.process && Array.isArray(globalThis.process.argv) \
             && globalThis.process.argv.length < 2) \
             {{ globalThis.process.argv.push('{f}'); }}\
             if (typeof globalThis.require !== 'undefined') \
               globalThis.require.main = {{ \
                 id: '.', filename: '{f}', loaded: true, \
                 exports: {{}}, parent: null, children: [], paths: [] \
               }};",
                f = f,
                d = d,
                u = u,
            );

            let setup_src = v8::String::new(&scope, &setup).unwrap();
            let _ = v8::Script::compile(&scope, setup_src, None).and_then(|s| s.run(&scope));

            // Unlike the setup script above (internal boilerplate, always
            // valid), the file's own code must propagate compile/run
            // failures — a `let _ = ...` here (as before) silently
            // discarded syntax errors and exceptions thrown during
            // evaluation (e.g. a require() permission denial), so
            // eval_file() always returned Ok even when the script never
            // actually ran.
            let code_src = v8::String::new(&scope, &code).unwrap();
            v8::Script::compile(&scope, code_src, None)
                .ok_or_else(|| anyhow::anyhow!("compile error"))?
                .run(&scope)
                .ok_or_else(|| anyhow::anyhow!("execution error"))?;
        }

        self.run_event_loop().await?;

        Ok(())
    }

    pub async fn run_event_loop(&mut self) -> anyhow::Result<()> {
        let max_iterations = 100_000;
        let mut iterations = 0;
        let has_pending_async = true;

        while (self.timer_manager.has_pending()
            || self.runtime_core.lock().unwrap().pending_task_count() > 0
            || has_pending_async)
            && iterations < max_iterations
        {
            iterations += 1;

            let tm = self.timer_manager.clone();
            {
                let context_global = self.context.clone().expect("engine not initialized");
                let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
                let mut scope = scope.init();
                let context = v8::Local::new(&scope, &context_global);
                let mut scope = v8::ContextScope::new(&mut scope, context);
                builtins::timers::TimerManager::fire_pending(&mut scope, tm)?;
            }
            pump_v8_platform_tasks(&self.isolate);
            self.isolate.perform_microtask_checkpoint();

            let expired = self.runtime_core.lock().unwrap().poll_timers();
            for timer in expired {
                (timer.callback)();
                if timer.repeating
                    && let Some(interval) = timer.interval
                {
                    self.runtime_core
                        .lock()
                        .unwrap()
                        .set_timeout(interval, timer.callback);
                }
            }

            tokio::task::yield_now().await;

            let next_js = self.timer_manager.next_expiry();
            let next_rust = self.runtime_core.lock().unwrap().next_timer_duration();
            let wait = match (next_js, next_rust) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };

            if let Some(wait) = wait
                && wait > std::time::Duration::ZERO
            {
                tokio::time::sleep(wait.min(std::time::Duration::from_millis(50))).await;
            } else if wait.is_none() && !has_pending_async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        }

        Ok(())
    }

    pub async fn take_profiler(&self) -> Option<Profiler> {
        self.profiler.clone()
    }

    pub fn is_profiling(&self) -> bool {
        self.profiler.is_some()
    }

    pub async fn take_heap_snapshot(&mut self) -> anyhow::Result<String> {
        // V8's own heap profiler — real nodes/edges/strings for every live
        // object, not a hand-rolled stub. It streams the serialized
        // .heapsnapshot JSON (Chrome DevTools format) as byte chunks that
        // just need concatenating.
        let mut buf: Vec<u8> = Vec::new();
        self.isolate.take_heap_snapshot(|chunk| {
            buf.extend_from_slice(chunk);
            true
        });
        Ok(std::string::String::from_utf8(buf)?)
    }
}

fn is_esm_source(code: &str) -> bool {
    let mut in_block_comment = false;
    for line in code.lines() {
        let trimmed = line.trim();
        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("/*") {
            in_block_comment = true;
            continue;
        }
        if trimmed.starts_with("import ")
            || trimmed.starts_with("import{")
            || trimmed.starts_with("export ")
            || trimmed.starts_with("export{")
            || trimmed.starts_with("export default")
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_engine_initialization() {
        let permissions = Arc::new(PermissionState::new());
        let engine = JsEngine::new(permissions).await;
        assert!(engine.is_ok(), "Engine failed to initialize");
    }

    #[tokio::test]
    async fn test_engine_evaluation() {
        let permissions = Arc::new(PermissionState::new());
        let mut engine = JsEngine::new(permissions).await.unwrap();

        let result = engine.eval("const x = 1 + 1;").await;
        assert!(result.is_ok());

        let error_result = engine.eval("const x = ;").await;
        assert!(error_result.is_err());
    }
}
