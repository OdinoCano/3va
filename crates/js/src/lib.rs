pub mod builtins;
pub mod esm;
pub mod transpiler;

use rquickjs::{AsyncContext, AsyncRuntime, Module};
use std::path::Path;
use std::sync::{Arc, Mutex};
use vvva_core::Runtime;
use vvva_permissions::PermissionState;

use builtins::TimerManager;

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
    #[allow(dead_code)]
    runtime: AsyncRuntime,
    context: AsyncContext,
    _permissions: Arc<PermissionState>,
    timer_manager: Arc<TimerManager>,
    runtime_core: Mutex<Runtime>,
}

impl JsEngine {
    pub async fn new(permissions: Arc<PermissionState>) -> anyhow::Result<Self> {
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

        {
            let perms = permissions.clone();
            let tm = timer_manager.clone();
            context
                .with(|ctx: rquickjs::Ctx| {
                    builtins::inject_all(&ctx, perms, tm)?;
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
        })
    }

    pub async fn eval(&self, code: &str) -> anyhow::Result<()> {
        let code = code.to_string();
        self.context
            .with(|ctx| {
                let _res: rquickjs::Value = ctx.eval(code.as_str())?;
                Ok::<(), rquickjs::Error>(())
            })
            .await?;
        Ok(())
    }

    /// Evaluate a JS expression and return its string value.
    pub async fn eval_to_string(&self, code: &str) -> anyhow::Result<String> {
        let code = code.to_string();
        let result = self
            .context
            .with(|ctx| -> rquickjs::Result<String> {
                let val: rquickjs::Value = ctx.eval(code.as_str())?;
                if let Some(s) = val.as_string() {
                    Ok(s.to_string()?)
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
        let script_arg = filename.replace('"', "\\\"");
        let extra: String = args
            .iter()
            .map(|a| format!(", \"{}\"", a.replace('"', "\\\"")))
            .collect();
        let inject = format!(
            "if (globalThis.process && Array.isArray(globalThis.process.argv)) \
             {{ globalThis.process.argv = [globalThis.process.argv[0], \"{script_arg}\"{extra}]; }}"
        );
        self.context
            .with(|ctx| {
                ctx.eval::<(), _>(inject.as_str())?;
                Ok::<(), rquickjs::Error>(())
            })
            .await?;
        self.eval_file(path).await
    }

    /// Read a file, transpile if TypeScript, set `__filename`/`__dirname`, then eval.
    /// Automatically detects ESM (top-level import/export) and uses Module evaluation.
    pub async fn eval_file(&self, path: &Path) -> anyhow::Result<()> {
        let source = std::fs::read_to_string(path)?;

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let code = match ext {
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

        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let filename = canonical.to_string_lossy().to_string();
        let dirname = canonical
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let is_esm = is_esm_source(&code);

        self.context
            .with(|ctx| {
                if is_esm {
                    // Set argv[1] to the script path for ESM files too
                    let argv_setup = format!(
                        "if (globalThis.process && Array.isArray(globalThis.process.argv) \
                         && globalThis.process.argv.length < 2) \
                         {{ globalThis.process.argv.push('{}'); }}",
                        filename.replace('\'', "\\'")
                    );
                    ctx.eval::<(), _>(argv_setup.as_str())?;
                    let module = Module::declare(ctx.clone(), filename.as_str(), code.as_str())?;
                    let (_module_eval, _promise) = module.eval()?;
                    Ok::<(), rquickjs::Error>(())
                } else {
                    let setup = format!(
                        "globalThis.__filename = '{f}'; globalThis.__dirname = '{d}';\
                         if (globalThis.process && Array.isArray(globalThis.process.argv) \
                         && globalThis.process.argv.length < 2) \
                         {{ globalThis.process.argv.push('{f}'); }}",
                        f = filename.replace('\'', "\\'"),
                        d = dirname.replace('\'', "\\'"),
                    );
                    ctx.eval::<(), _>(setup.as_str())?;
                    let _: rquickjs::Value = ctx.eval(code.as_str())?;
                    Ok::<(), rquickjs::Error>(())
                }
            })
            .await?;

        // Drain pending JS microtasks / promise callbacks.
        self.runtime.idle().await;

        // Drive the event loop: timers + promise microtasks.
        self.run_event_loop().await
    }

    /// Run the integrated event loop:
    /// - Fire expired JS timers (setTimeout/setInterval managed by TimerManager)
    /// - Fire expired Rust-level TimerWheel timers
    /// - Process JS promise microtasks (runtime.idle)
    /// - Yield to Tokio so concurrent async tasks can make progress
    /// - Sleep until the next timer expiry (max 50ms per iteration)
    pub async fn run_event_loop(&self) -> anyhow::Result<()> {
        let max_iterations = 10_000;
        let mut iterations = 0;

        while (self.timer_manager.has_pending()
            || self.runtime_core.lock().unwrap().pending_task_count() > 0)
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

            // 3. Process JS promise microtasks
            self.runtime.idle().await;

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
            } else if wait.is_none() {
                // No pending timers at all, but still have tasks — brief yield
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        }

        Ok(())
    }
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
    async fn zlib_sync_methods_throw_not_available() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                r#"(function() {
                       const zlib = require('zlib');
                       var msgs = [];
                       ['gzipSync','gunzipSync','deflateSync','inflateSync'].forEach(function(fn) {
                           try { zlib[fn](Buffer.from('x')); msgs.push('no_throw'); }
                           catch(e) { msgs.push(e.message.includes('not available') ? 'ok' : 'wrong'); }
                       });
                       return msgs.join(',');
                   })()"#,
            )
            .await
            .unwrap();
        assert_eq!(r, "ok,ok,ok,ok");
    }

    #[tokio::test]
    async fn zlib_create_methods_return_stub_objects() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                "const z = require('zlib');
                 typeof z.createGzip() + ',' + typeof z.createGunzip() + ',' +
                 typeof z.createDeflate() + ',' + typeof z.createInflate()",
            )
            .await
            .unwrap();
        assert_eq!(r, "object,object,object,object");
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
    async fn child_process_execsync_always_throws() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                r#"(function() {
                       try { require('child_process').execSync('echo'); return 'no_throw'; }
                       catch(e) { return e.message.includes('not available') ? 'ok' : 'wrong:' + e.message; }
                   })()"#,
            )
            .await
            .unwrap();
        assert_eq!(r, "ok");
    }

    #[tokio::test]
    async fn child_process_spawnsync_always_throws() {
        let e = engine_no_perms().await;
        let r = e
            .eval_to_string(
                r#"(function() {
                       try { require('child_process').spawnSync('echo'); return 'no_throw'; }
                       catch(e) { return e.message.includes('not available') ? 'ok' : 'wrong:' + e.message; }
                   })()"#,
            )
            .await
            .unwrap();
        assert_eq!(r, "ok");
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
        e.eval(
            r#"globalThis.__cp3 = null;
               const { execFile } = require('child_process');
               execFile('/bin/echo', ['execfile_ok'], function(err, stdout) {
                   globalThis.__cp3 = err ? 'error' : stdout.trim();
               });"#,
        )
        .await
        .unwrap();
        let result = wait_for_result(&e, "__cp3").await;
        assert_eq!(result, "execfile_ok", "execFile with permission: {result}");
    }

    #[tokio::test]
    async fn child_process_spawn_delivers_stdout_with_permission() {
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::SpawnProcess);
        let e = JsEngine::new(perms).await.unwrap();
        e.eval(
            r#"globalThis.__cp4 = null;
               const { spawn } = require('child_process');
               var child = spawn('/bin/echo', ['spawn_ok']);
               child.stdout.on('data', function(data) {
                   globalThis.__cp4 = typeof data === 'string' ? data.trim() : String(data).trim();
               });
               child.on('exit', function(code) {
                   if (globalThis.__cp4 === null) globalThis.__cp4 = 'no_stdout';
               });"#,
        )
        .await
        .unwrap();
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
