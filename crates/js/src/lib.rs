pub mod builtins;
pub mod esm;
pub mod transpiler;

use rquickjs::{AsyncContext, AsyncRuntime, Module};
use std::path::Path;
use std::sync::Arc;
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
}

impl JsEngine {
    pub async fn new(permissions: Arc<PermissionState>) -> anyhow::Result<Self> {
        let runtime = AsyncRuntime::new()?;
        let timer_manager = TimerManager::new();

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

    /// Read a file, transpile if TypeScript, set `__filename`/`__dirname`, then eval.
    /// Automatically detects ESM (top-level import/export) and uses Module evaluation.
    pub async fn eval_file(&self, path: &Path) -> anyhow::Result<()> {
        let source = std::fs::read_to_string(path)?;

        // Transpile TypeScript
        let code = if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ts") | Some("tsx")
        ) {
            transpiler::transpile(&source)
        } else {
            source
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
                    let module = Module::declare(ctx.clone(), filename.as_str(), code.as_str())?;
                    let (_module_eval, _promise) = module.eval()?;
                    Ok::<(), rquickjs::Error>(())
                } else {
                    let setup = format!(
                        "globalThis.__filename = '{}'; globalThis.__dirname = '{}';",
                        filename.replace('\'', "\\'"),
                        dirname.replace('\'', "\\'"),
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

    /// Run the event loop: poll expired timers and fire callbacks until no pending timers remain.
    pub async fn run_event_loop(&self) -> anyhow::Result<()> {
        let max_iterations = 10_000;
        let mut iterations = 0;

        while self.timer_manager.has_pending() && iterations < max_iterations {
            iterations += 1;

            if let Some(wait) = self.timer_manager.next_expiry()
                && wait.as_millis() > 0
            {
                tokio::time::sleep(wait.min(std::time::Duration::from_millis(50))).await;
            }

            let tm = self.timer_manager.clone();
            self.context
                .with(|ctx| builtins::timers::TimerManager::fire_pending(&ctx, tm))
                .await?;
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
