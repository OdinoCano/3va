//! JavaScript engine crate — wraps QuickJS via rquickjs, exposes `JsEngine` and all built-in modules.
//!
//! # Examples
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use vvva_permissions::PermissionState;
//! use vvva_js::JsEngine;
//!
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! let perms = Arc::new(PermissionState::new());
//! let engine = JsEngine::new(perms).await.unwrap();
//! engine.eval("const x = 1 + 1; console.log(x);").await.unwrap();
//! # });
//! ```

pub mod async_context;
pub mod builtins;
pub mod esm;
pub mod inspector;
pub mod profiler;
pub mod transpiler;

use rquickjs::{AsyncContext, AsyncRuntime, Function};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use vvva_core::Runtime;
use vvva_firewall::Firewall;
use vvva_permissions::PermissionState;

use builtins::TimerManager;
use profiler::Profiler;

/// Convert a `rquickjs::Error` into `anyhow::Error`, extracting the real JS exception
/// message/stack when the variant is `Error::Exception`.
fn catch_js(ctx: &rquickjs::Ctx, e: rquickjs::Error) -> anyhow::Error {
    anyhow::anyhow!("{}", rquickjs::CaughtError::from_error(ctx, e))
}

/// Heuristic: detect ESM by scanning all lines for top-level import/export.
/// Skips blank lines and single-line comments. Handles files where exports
/// appear after other code (e.g. `export default fn` at end of file).
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

pub struct JsEngine {
    runtime: AsyncRuntime,
    context: AsyncContext,
    _permissions: Arc<PermissionState>,
    timer_manager: Arc<TimerManager>,
    runtime_core: Mutex<Runtime>,
    inspector: Option<Arc<inspector::InspectorState>>,
    profiler: Option<Profiler>,
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

    /// Create a `JsEngine` with an optional CDP inspector bound to `inspect_addr`.
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

    /// Create a `JsEngine` with CPU profiling enabled.
    ///
    /// `interval_ms` controls the sampling interval (default: 10 ms).
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
        let runtime = AsyncRuntime::new()?;
        let timer_manager = TimerManager::new();
        let runtime_core = Mutex::new(Runtime::new((*permissions).clone()));

        // 256 MB heap limit; GC triggered at 80% (≈204 MB).
        runtime.set_memory_limit(256 * 1024 * 1024).await;
        runtime.set_gc_threshold(204 * 1024 * 1024).await;

        // Wire the ESM module loader so cross-file imports resolve correctly.
        runtime
            .set_loader(
                esm::EsmResolver,
                esm::EsmLoader {
                    permissions: (*permissions).clone(),
                },
            )
            .await;

        let context = AsyncContext::full(&runtime).await?;

        // Start the CDP inspector server if requested.
        let inspector = inspect_addr.map(inspector::start);

        // Allocate the profiler state so we can share it with the JS push callback.
        let profiler = prof_interval_ms.map(|_| Profiler::new());

        let ws_pool: builtins::websocket::WsPool =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        {
            let perms = permissions.clone();
            let tm = timer_manager.clone();
            let insp = inspector.clone();
            let prof_js = prof_interval_ms.map(profiler::profiler_js);
            let prof_handle = profiler.clone();
            let fw = firewall;
            let pool = ws_pool.clone();
            context
                .with(move |ctx: rquickjs::Ctx| {
                    // Install async context hook FIRST — must be wired before any
                    // Promises are created so continuations capture context IDs.
                    let rt_ptr = ctx.as_raw().as_ptr();
                    let rt_ptr =
                        unsafe { rquickjs_sys::JS_GetRuntime(rt_ptr) } as *mut std::ffi::c_void;
                    unsafe { async_context::install(&ctx, rt_ptr) }?;

                    builtins::inject_all(&ctx, perms, tm, fw, pool)?;

                    // Inject __3va_debugger__ if inspector is active.
                    if let Some(state) = insp {
                        let f = Function::new(ctx.clone(), move || {
                            let s = state.clone();
                            tokio::task::block_in_place(move || s.pause());
                        })?;
                        ctx.globals().set("__3va_debugger__", f)?;
                    }

                    // Inject profiler JS bootstrap and the Rust-side push callback.
                    if let (Some(js_src), Some(handle)) = (prof_js, prof_handle) {
                        // __profilerPush(ts_ms, stack_str, label_or_null) → called by JS
                        let push_handle = handle.clone();
                        ctx.globals().set(
                            "__profilerPush",
                            Function::new(
                                ctx.clone(),
                                move |ts: u64, stack: String, label: rquickjs::Value| {
                                    let lbl = if label.is_null() || label.is_undefined() {
                                        None
                                    } else {
                                        label.as_string().and_then(|s| s.to_string().ok())
                                    };
                                    push_handle.push_raw(ts, &stack, lbl);
                                },
                            )?,
                        )?;
                        ctx.eval::<(), _>(js_src.as_str())?;
                    }

                    Ok::<(), rquickjs::Error>(())
                })
                .await?;
        }

        Ok(Self {
            runtime,
            context,
            _permissions: permissions,
            timer_manager,
            runtime_core,
            inspector,
            profiler,
            ws_pool,
        })
    }

    /// Gracefully close every outgoing WebSocket connection the JS script has opened.
    ///
    /// Iterates the internal [`WsPool`](builtins::websocket::WsPool), sending a
    /// `Close(1001 Going Away)` frame to each peer with a random 100–500 ms jitter
    /// between sends.  The jitter staggers client reconnections so they don't all
    /// hammer the upstream service at the same instant.
    ///
    /// Blocks (via `spawn_blocking`) until all connections are closed or 30 s have
    /// elapsed, whichever comes first.  Call this in a SIGTERM / Ctrl-C handler
    /// before the process exits.
    pub async fn drain_ws_connections(&self) {
        let pool = self.ws_pool.clone();
        tokio::task::spawn_blocking(move || {
            builtins::websocket::drain_ws_pool(&pool, std::time::Duration::from_secs(30));
        })
        .await
        .ok();
    }

    pub async fn eval(&self, code: &str) -> anyhow::Result<()> {
        let code = code.to_string();
        self.context
            .with(|ctx| -> anyhow::Result<()> {
                ctx.eval::<rquickjs::Value, _>(code.as_str())
                    .map(|_| ())
                    .map_err(|e| catch_js(&ctx, e))
            })
            .await?;
        Ok(())
    }

    /// Evaluate a JS expression and return its string value.
    pub async fn eval_to_string(&self, code: &str) -> anyhow::Result<String> {
        let code = code.to_string();
        let result = self
            .context
            .with(|ctx| -> anyhow::Result<String> {
                let val = ctx
                    .eval::<rquickjs::Value, _>(code.as_str())
                    .map_err(|e| catch_js(&ctx, e))?;
                if let Some(s) = val.as_string() {
                    s.to_string().map_err(|e| catch_js(&ctx, e))
                } else {
                    Ok(String::new())
                }
            })
            .await?;
        Ok(result)
    }

    /// Drive one round of the async runtime — resolves pending Promises and Tokio futures.
    pub async fn idle(&self) {
        self.runtime.idle().await;
    }

    /// Like `eval_file` but also sets `process.argv[2+]` to the given script arguments.
    pub async fn eval_file_with_args(&self, path: &Path, args: &[String]) -> anyhow::Result<()> {
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
        self.context
            .with(|ctx| -> anyhow::Result<()> {
                ctx.eval::<(), _>(inject.as_str())
                    .map_err(|e| catch_js(&ctx, e))
            })
            .await?;
        self.eval_file(path).await
    }

    /// Read a file, transpile if TypeScript, set `__filename`/`__dirname`, then eval.
    /// Automatically detects ESM (top-level import/export) and uses Module evaluation.
    pub async fn eval_file(&self, path: &Path) -> anyhow::Result<()> {
        let source = std::fs::read_to_string(path)?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_jsx_ext = matches!(ext, "tsx" | "jsx");
        let transpiled = match ext {
            "tsx" | "jsx" => transpiler::transpile_jsx(&source),
            "ts" | "mts" | "cts" => transpiler::transpile(&source),
            _ => {
                // For .js/.mjs and unknown extensions, auto-detect JSX
                if transpiler::looks_like_jsx(&source) {
                    transpiler::transpile_js(&source)
                } else {
                    source
                }
            }
        };

        // Rewrite `debugger;` → `__3va_debugger__();` when inspector is active.
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

        // Compute a proper file:// URL for import.meta.url.
        let meta_url = url::Url::from_file_path(&canonical)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", filename.replace('\\', "/")));

        let is_esm = is_esm_source(&code);

        // When the entry file is ESM (has import/export), convert to CJS via OXC so
        // all `import` statements become `require()` calls handled by the existing shim.
        // transpile_to_cjs also rewrites import.meta.* → __vvva_meta_* stubs.
        // Non-ESM files may still contain import.meta (e.g. CJS bundles from frameworks),
        // so we always run the import.meta replacer regardless.
        let code = if is_esm {
            transpiler::transpile_to_cjs(&code, is_jsx_ext)
        } else {
            transpiler::replace_import_meta(&code)
        };

        self.context
            .with(|ctx| -> anyhow::Result<()> {
                // Escape backslashes first (Windows paths), then quotes.
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
                    f = f, d = d, u = u,
                );
                ctx.eval::<(), _>(setup.as_str())
                    .map_err(|e| catch_js(&ctx, e))?;

                if transpiler::has_top_level_await(code.as_str()) {
                    let _ = ctx
                        .eval_promise(code.as_str())
                        .map_err(|e| catch_js(&ctx, e))?;
                } else {
                    ctx.eval::<rquickjs::Value, _>(code.as_str())
                        .map(|_| ())
                        .map_err(|e| catch_js(&ctx, e))?;
                }

                Ok(())
            })
            .await?;

        // Drive the event loop: timers + promise microtasks.
        // Note: do NOT call runtime.idle() here — it blocks on pending server-side
        // async tasks (e.g. __httpAcceptAsync) before timers have a chance to fire,
        // causing a deadlock. run_event_loop handles idle() internally.
        self.run_event_loop().await
    }

    /// Run the integrated event loop:
    /// - Fire expired JS timers (setTimeout/setInterval managed by TimerManager)
    /// - Fire expired Rust-level TimerWheel timers
    /// - Process JS promise microtasks (runtime.idle with short timeout)
    /// - Yield to Tokio so concurrent async tasks can make progress
    /// - Sleep until the next timer expiry (max 50ms per iteration)
    pub async fn run_event_loop(&self) -> anyhow::Result<()> {
        let max_iterations = 100_000;
        let mut iterations = 0;
        // Track whether idle() had pending async work (e.g. HTTP server accept loop).
        // When true the loop keeps running even if no JS timers are pending, so that
        // server-side async tasks can complete (and schedule new timers via callbacks).
        // Start as true so the loop always runs at least one iteration — JS Promises
        // created during synchronous module evaluation (e.g. unawaited bootstrap())
        // are not tracked as pending Rust tasks but still need idle() to drain them.
        let mut has_pending_async = true;

        while (self.timer_manager.has_pending()
            || self.runtime_core.lock().unwrap().pending_task_count() > 0
            || has_pending_async)
            && iterations < max_iterations
        {
            iterations += 1;

            // 1. Fire JS-level timers (setTimeout/setInterval)
            let tm = self.timer_manager.clone();
            self.context
                .with(|ctx| builtins::timers::TimerManager::fire_pending(&ctx, tm))
                .await?;

            // 2. Fire Rust-level TimerWheel timers via the core Runtime
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

            // 2.5 Drain process.nextTick queue BEFORE Promise microtasks
            //     (matching Node.js event loop priority).
            self.context
                .with(|ctx| -> rquickjs::Result<()> {
                    let _: rquickjs::Value =
                        ctx.eval("if (typeof __drainNextTick === 'function') __drainNextTick();")?;
                    Ok(())
                })
                .await?;

            // 3. Process JS promise microtasks with a short timeout.
            //    idle() blocks until ALL spawner tasks complete, which includes
            //    persistent server-side accept loops (_acceptNext / __httpAcceptAsync).
            //    We use a 5ms timeout so the loop can keep iterating to fire pending
            //    setTimeout callbacks (needed to resolve httpGet Promises between requests).
            let idle_timed_out =
                tokio::time::timeout(std::time::Duration::from_millis(5), self.runtime.idle())
                    .await
                    .is_err();
            has_pending_async = idle_timed_out;

            // 3.5a Drain NAPI threadsafe function call queue (background threads → JS main thread)
            self.context
                .with(|_ctx| -> rquickjs::Result<()> {
                    unsafe { builtins::napi::drain_tsfn_queue() };
                    Ok(())
                })
                .await?;

            // 3.5 Drain setImmediate queue (Node.js "check" phase, after I/O and promises)
            self.context
                .with(|ctx| -> rquickjs::Result<()> {
                    let _: rquickjs::Value = ctx
                        .eval("if (typeof __drainImmediate === 'function') __drainImmediate();")?;
                    Ok(())
                })
                .await?;

            // 4. Yield to Tokio so concurrent async ops make progress
            tokio::task::yield_now().await;

            // 5. Sleep until the next timer expiry (JS or Rust)
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
                // No pending timers and no pending async — truly idle, brief yield
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
            // If has_pending_async: no sleep, keep looping immediately to
            // process timers that callbacks may have scheduled.
        }

        Ok(())
    }

    /// If profiling is active, stop the JS sampling interval and return the
    /// accumulated `Profiler`. Returns `None` if `--prof` was not enabled.
    pub async fn take_profiler(&self) -> Option<Profiler> {
        if let Some(ref profiler) = self.profiler {
            // Call __profilerStop() in JS to clear the setInterval.
            let _ = self
                .context
                .with(|ctx| {
                    let _: rquickjs::Value =
                        ctx.eval("if (typeof __profilerStop === 'function') __profilerStop();")?;
                    Ok::<(), rquickjs::Error>(())
                })
                .await;
            Some(profiler.clone())
        } else {
            None
        }
    }

    /// Returns `true` if profiling is active on this engine.
    pub fn is_profiling(&self) -> bool {
        self.profiler.is_some()
    }

    /// Take a synthetic heap snapshot compatible with Chrome DevTools Memory panel.
    ///
    /// Since QuickJS does not expose per-object heap internals, this creates an
    /// approximation using memory usage statistics and global object enumeration.
    /// The snapshot includes node categories (string, array, object, function)
    /// with approximate sizes, sufficient for detecting memory leaks by category.
    pub async fn take_heap_snapshot(&self) -> anyhow::Result<String> {
        let memory_stats = self.runtime.memory_usage().await;

        let snapshot = serde_json::json!({
            "snapshot": {
                "meta": {
                    "node_fields": ["type", "name", "id", "self_size", "edge_count", "trace_node_id"],
                    "node_types": [
                        ["hidden", "object", "function", "string", "unknown", "array", "boolean", "number"],
                        "string",
                        "number",
                        "number",
                        "number",
                        "number"
                    ],
                    "edge_fields": ["type", "name_or_index", "to_node", "from_node"],
                    "edge_types": [
                        ["context", "element", "property", "internal", "hidden", "shortcut", "weak"],
                        "string_or_number",
                        "node",
                        "node"
                    ],
                    "trace_function_info_fields": ["function_id", "name", "script_name", "line", "column"],
                    "trace_node_fields": ["id", "function_info_index", "offset", "col", "line"],
                    "sample_fields": ["timestamp_us", "client_id", "heap_size"],
                    "location_fields": ["object_index", "field_index"]
                },
                "node_count": 0,
                "edge_count": 0,
                "trace_function_count": 0
            },
            "nodes": build_synthetic_nodes(&memory_stats),
            "edges": [],
            "strings": build_snapshot_strings(&memory_stats),
            "memory_usage": {
                "malloc_size": memory_stats.malloc_size,
                "malloc_limit": memory_stats.malloc_limit,
                "memory_used_size": memory_stats.memory_used_size,
                "malloc_count": memory_stats.malloc_count,
                "memory_used_count": memory_stats.memory_used_count,
                "atom_count": memory_stats.atom_count,
                "atom_size": memory_stats.atom_size,
                "str_count": memory_stats.str_count,
                "str_size": memory_stats.str_size,
                "obj_count": memory_stats.obj_count,
                "obj_size": memory_stats.obj_size,
                "prop_count": memory_stats.prop_count,
                "prop_size": memory_stats.prop_size,
                "js_func_count": memory_stats.js_func_count,
                "js_func_size": memory_stats.js_func_size,
                "array_count": memory_stats.array_count,
                "fast_array_count": memory_stats.fast_array_count,
            }
        });

        Ok(serde_json::to_string_pretty(&snapshot)?)
    }
}

// ── heap snapshot helpers ─────────────────────────────────────────────────────

fn build_synthetic_nodes(m: &rquickjs::runtime::MemoryUsage) -> serde_json::Value {
    // node_fields: ["type", "name", "id", "self_size", "edge_count", "trace_node_id"]
    // type index matches node_types[0]: hidden=0, object=1, function=2, string=3, array=5
    let mut nodes: Vec<serde_json::Value> = Vec::new();
    let mut id: u64 = 1;

    let categories: &[(&str, u64, u64, u64)] = &[
        // (label, type_idx, count, avg_size)
        (
            "object",
            1,
            m.obj_count as u64,
            (m.obj_size as u64).saturating_div(m.obj_count.max(1) as u64),
        ),
        (
            "string",
            3,
            m.str_count as u64,
            (m.str_size as u64).saturating_div(m.str_count.max(1) as u64),
        ),
        (
            "function",
            2,
            m.js_func_count as u64,
            (m.js_func_size as u64).saturating_div(m.js_func_count.max(1) as u64),
        ),
        ("array", 5, m.array_count as u64, 64),
    ];

    for (label, type_idx, count, avg_size) in categories {
        for i in 0..*count {
            nodes.push(serde_json::json!([
                *type_idx,
                label,
                id + i,
                avg_size,
                0,
                0
            ]));
        }
        id += count;
    }

    serde_json::Value::Array(nodes)
}

fn build_snapshot_strings(m: &rquickjs::runtime::MemoryUsage) -> serde_json::Value {
    let mut strings = vec![
        serde_json::Value::String(String::new()),
        serde_json::Value::String("object".to_string()),
        serde_json::Value::String("string".to_string()),
        serde_json::Value::String("function".to_string()),
        serde_json::Value::String("array".to_string()),
        serde_json::Value::String(format!("atom_count:{}", m.atom_count)),
        serde_json::Value::String(format!("prop_count:{}", m.prop_count)),
    ];
    // Include atom strings as placeholders so the strings table is non-empty
    for i in 0..(m.atom_count.min(32) as u64) {
        strings.push(serde_json::Value::String(format!("atom_{i}")));
    }
    serde_json::Value::Array(strings)
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
        let engine = JsEngine::new(permissions).await.unwrap();

        let result = engine.eval("const x = 1 + 1;").await;
        assert!(result.is_ok());

        let error_result = engine.eval("const x = ;").await;
        assert!(error_result.is_err());
    }

    #[tokio::test]
    async fn test_eval_typescript() {
        let permissions = Arc::new(PermissionState::new());
        let engine = JsEngine::new(permissions).await.unwrap();

        let ts_code = "const x: number = 42;";
        let js_code = transpiler::transpile(ts_code);
        let result = engine.eval(&js_code).await;
        assert!(
            result.is_ok(),
            "TS transpiled code should eval: {:?}",
            result
        );
    }
}

#[cfg(test)]
mod builtin_tests {
    use super::*;
    use vvva_permissions::Capability;

    async fn engine_no_perms() -> JsEngine {
        let perms = Arc::new(PermissionState::new());
        JsEngine::new(perms).await.unwrap()
    }

    /// Poll the async runtime until `globalThis.__done` is non-empty or timeout.
    async fn wait_for_result(engine: &JsEngine, global: &str) -> String {
        for _ in 0..40 {
            engine.idle().await;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            let v = engine
                .eval_to_string(&format!("globalThis.{global} || ''"))
                .await
                .unwrap_or_default();
            if !v.is_empty() {
                return v;
            }
        }
        String::new()
    }

    // ── zlib ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn zlib_require_exposes_api() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                "const z = require('zlib');
                 typeof z.gzip + ',' + typeof z.gunzip + ',' +
                 typeof z.deflate + ',' + typeof z.inflate + ',' +
                 typeof z.deflateRaw + ',' + typeof z.inflateRaw",
            )
            .await
            .unwrap();
        assert_eq!(r, "function,function,function,function,function,function");
    }

    #[tokio::test]
    async fn zlib_constants_are_defined() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                "const c = require('zlib').constants;
                 '' + c.Z_OK + ',' + c.Z_BEST_SPEED + ',' + c.Z_BEST_COMPRESSION",
            )
            .await
            .unwrap();
        assert_eq!(r, "0,1,9");
    }

    #[tokio::test]
    async fn zlib_gzip_gunzip_round_trip() {
        let e = engine_no_perms().await;
        e.eval(
            r#"globalThis.__zlib1 = null;
               const zlib = require('zlib');
               zlib.gzip(Buffer.from('hello world'), function(err, compressed) {
                   if (err) { globalThis.__zlib1 = 'gzip_err:' + err; return; }
                   zlib.gunzip(compressed, function(err2, out) {
                       globalThis.__zlib1 = err2 ? 'gunzip_err:' + err2
                                                 : Buffer.from(out).toString('utf8');
                   });
               });"#,
        )
        .await
        .unwrap();
        let result = wait_for_result(&e, "__zlib1").await;
        assert_eq!(result, "hello world", "gzip/gunzip round-trip: {result}");
    }

    #[tokio::test]
    async fn zlib_deflate_inflate_round_trip() {
        let e = engine_no_perms().await;
        e.eval(
            r#"globalThis.__zlib2 = null;
               const zlib = require('zlib');
               zlib.deflate(Buffer.from('test data'), function(err, compressed) {
                   if (err) { globalThis.__zlib2 = 'deflate_err'; return; }
                   zlib.inflate(compressed, function(err2, out) {
                       globalThis.__zlib2 = err2 ? 'inflate_err'
                                                 : Buffer.from(out).toString('utf8');
                   });
               });"#,
        )
        .await
        .unwrap();
        let result = wait_for_result(&e, "__zlib2").await;
        assert_eq!(result, "test data", "deflate/inflate round-trip: {result}");
    }

    #[tokio::test]
    async fn zlib_deflate_raw_inflate_raw_round_trip() {
        let e = engine_no_perms().await;
        e.eval(
            r#"globalThis.__zlib3 = null;
               const zlib = require('zlib');
               zlib.deflateRaw(Buffer.from('raw deflate'), function(err, compressed) {
                   if (err) { globalThis.__zlib3 = 'err'; return; }
                   zlib.inflateRaw(compressed, function(err2, out) {
                       globalThis.__zlib3 = err2 ? 'err'
                                                 : Buffer.from(out).toString('utf8');
                   });
               });"#,
        )
        .await
        .unwrap();
        let result = wait_for_result(&e, "__zlib3").await;
        assert_eq!(
            result, "raw deflate",
            "deflateRaw/inflateRaw round-trip: {result}"
        );
    }

    #[tokio::test]
    async fn zlib_sync_methods_work() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                r#"(function() {
                       const zlib = require('zlib');
                       var msgs = [];
                       // gzipSync → gunzipSync round-trip
                       try {
                           var compressed = zlib.gzipSync(Buffer.from('hello'));
                           var decompressed = zlib.gunzipSync(compressed);
                           msgs.push(new TextDecoder().decode(decompressed) === 'hello' ? 'ok' : 'wrong');
                       } catch(e) { msgs.push('err:' + e.message); }
                       // deflateSync → inflateSync round-trip
                       try {
                           var c2 = zlib.deflateSync(Buffer.from('world'));
                           var d2 = zlib.inflateSync(c2);
                           msgs.push(new TextDecoder().decode(d2) === 'world' ? 'ok' : 'wrong');
                       } catch(e) { msgs.push('err:' + e.message); }
                       return msgs.join(',');
                   })()"#,
            )
            .await
            .unwrap();
        assert_eq!(r, "ok,ok");
    }

    #[tokio::test]
    async fn zlib_create_methods_return_transform_streams() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                r#"(function() {
                    const z = require('zlib');
                    var streams = [z.createGzip(), z.createGunzip(), z.createDeflate(), z.createInflate()];
                    // Each stream must have write, end, pipe, on methods
                    return streams.map(function(s) {
                        return (typeof s.write === 'function' && typeof s.pipe === 'function' &&
                                typeof s.end === 'function' && typeof s.on === 'function') ? 'ok' : 'stub';
                    }).join(',');
                })()"#,
            )
            .await
            .unwrap();
        assert_eq!(r, "ok,ok,ok,ok");
    }

    #[tokio::test]
    async fn zlib_node_prefix_alias_works() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                "const z1 = require('zlib');
                 const z2 = require('node:zlib');
                 z1 === z2 ? 'same' : 'different'",
            )
            .await
            .unwrap();
        assert_eq!(r, "same");
    }

    // ── child_process ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn child_process_require_exposes_api() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                "const cp = require('child_process');
                 typeof cp.exec + ',' + typeof cp.execFile + ',' +
                 typeof cp.spawn + ',' + typeof cp.promisify",
            )
            .await
            .unwrap();
        assert_eq!(r, "function,function,function,function");
    }

    #[tokio::test]
    async fn child_process_execsync_throws_without_permission() {
        // execSync now works but requires --allow-child-process; without it, a
        // permission error is thrown (not a "not available" message).
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                r#"(function() {
                       try { require('child_process').execSync('echo'); return 'no_throw'; }
                       catch(e) { return 'threw'; }
                   })()"#,
            )
            .await
            .unwrap();
        assert_eq!(r, "threw");
    }

    #[tokio::test]
    async fn child_process_spawnsync_throws_without_permission() {
        // spawnSync now works but requires --allow-child-process.
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                r#"(function() {
                       try { require('child_process').spawnSync('echo'); return 'no_throw'; }
                       catch(e) { return 'threw'; }
                   })()"#,
            )
            .await
            .unwrap();
        assert_eq!(r, "threw");
    }

    #[tokio::test]
    async fn child_process_exec_denied_without_permission() {
        let e = engine_no_perms().await;
        e.eval(
            r#"globalThis.__cp1 = null;
               const { exec } = require('child_process');
               exec('echo hello', function(err, stdout, stderr) {
                   globalThis.__cp1 = err ? 'denied' : 'allowed';
               });"#,
        )
        .await
        .unwrap();
        let result = wait_for_result(&e, "__cp1").await;
        assert_eq!(result, "denied", "exec without permission should be denied");
    }

    #[tokio::test]
    async fn child_process_exec_runs_with_permission() {
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::SpawnProcess);
        let e = JsEngine::new(perms).await.unwrap();
        e.eval(
            r#"globalThis.__cp2 = null;
               const { exec } = require('child_process');
               exec('echo hello3va', function(err, stdout, stderr) {
                   globalThis.__cp2 = err ? 'error:' + err.message : stdout.trim();
               });"#,
        )
        .await
        .unwrap();
        let result = wait_for_result(&e, "__cp2").await;
        assert_eq!(result, "hello3va", "exec with permission: {result}");
    }

    #[tokio::test]
    async fn child_process_execfile_runs_with_permission() {
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::SpawnProcess);
        let e = JsEngine::new(perms).await.unwrap();
        #[cfg(windows)]
        let js = r#"globalThis.__cp3 = null;
               const { execFile } = require('child_process');
               execFile('cmd.exe', ['/c', 'echo', 'execfile_ok'], function(err, stdout) {
                   globalThis.__cp3 = err ? 'error' : stdout.trim();
               });"#;
        #[cfg(not(windows))]
        let js = r#"globalThis.__cp3 = null;
               const { execFile } = require('child_process');
               execFile('/bin/echo', ['execfile_ok'], function(err, stdout) {
                   globalThis.__cp3 = err ? 'error' : stdout.trim();
               });"#;
        e.eval(js).await.unwrap();
        let result = wait_for_result(&e, "__cp3").await;
        assert_eq!(result, "execfile_ok", "execFile with permission: {result}");
    }

    #[tokio::test]
    async fn child_process_spawn_delivers_stdout_with_permission() {
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::SpawnProcess);
        let e = JsEngine::new(perms).await.unwrap();
        #[cfg(windows)]
        let js = r#"globalThis.__cp4 = null;
               const { spawn } = require('child_process');
               var child = spawn('cmd.exe', ['/c', 'echo', 'spawn_ok']);
               child.stdout.on('data', function(data) {
                   globalThis.__cp4 = typeof data === 'string' ? data.trim() : String(data).trim();
               });
               child.on('exit', function(code) {
                   if (globalThis.__cp4 === null) globalThis.__cp4 = 'no_stdout';
               });"#;
        #[cfg(not(windows))]
        let js = r#"globalThis.__cp4 = null;
               const { spawn } = require('child_process');
               var child = spawn('/bin/echo', ['spawn_ok']);
               child.stdout.on('data', function(data) {
                   globalThis.__cp4 = typeof data === 'string' ? data.trim() : String(data).trim();
               });
               child.on('exit', function(code) {
                   if (globalThis.__cp4 === null) globalThis.__cp4 = 'no_stdout';
               });"#;
        e.eval(js).await.unwrap();
        let result = wait_for_result(&e, "__cp4").await;
        assert_eq!(result, "spawn_ok", "spawn with permission: {result}");
    }

    #[tokio::test]
    async fn child_process_exec_nonzero_exit_passes_error() {
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::SpawnProcess);
        let e = JsEngine::new(perms).await.unwrap();
        e.eval(
            r#"globalThis.__cp5 = null;
               const { exec } = require('child_process');
               exec('exit 1', function(err, stdout, stderr) {
                   globalThis.__cp5 = err ? 'got_error' : 'no_error';
               });"#,
        )
        .await
        .unwrap();
        let result = wait_for_result(&e, "__cp5").await;
        assert_eq!(
            result, "got_error",
            "non-zero exit should pass error to callback"
        );
    }

    #[tokio::test]
    async fn child_process_node_prefix_alias_works() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                "const cp1 = require('child_process');
                 const cp2 = require('node:child_process');
                 cp1 === cp2 ? 'same' : 'different'",
            )
            .await
            .unwrap();
        assert_eq!(r, "same");
    }
}
