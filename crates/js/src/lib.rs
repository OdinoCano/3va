pub mod builtins;
pub mod esm;
pub mod transpiler;

use rquickjs::{Context, Module, Runtime};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use vvva_permissions::PermissionState;

/// Heuristic: check for top-level import/export statements indicating ESM.
fn is_esm_source(code: &str) -> bool {
    for line in code.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
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
        // Stop at first non-comment, non-blank, non-import/export line
        break;
    }
    false
}

pub struct JsEngine {
    runtime: Runtime,
    context: Context,
    permissions: Rc<RefCell<PermissionState>>,
}

impl JsEngine {
    pub fn new(permissions: &PermissionState) -> anyhow::Result<Self> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;

        let perms = Rc::new(RefCell::new(permissions.clone()));

        context.with(|ctx: rquickjs::Ctx| {
            let _ = builtins::inject_all(&ctx, perms.clone());
            Ok::<(), rquickjs::Error>(())
        })?;

        Ok(Self {
            runtime,
            context,
            permissions: perms,
        })
    }

    pub fn eval(&self, code: &str) -> anyhow::Result<()> {
        self.context.with(|ctx| {
            let _res: rquickjs::Value = ctx.eval(code)?;
            Ok::<(), rquickjs::Error>(())
        })?;
        Ok(())
    }

    /// Read a file, transpile if TypeScript, set `__filename`/`__dirname`, then eval.
    /// Automatically detects ESM (top-level import/export) and uses Module evaluation.
    pub fn eval_file(&self, path: &Path) -> anyhow::Result<()> {
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

        // Detect ESM: top-level import/export statements
        let is_esm = is_esm_source(&code);

        self.context.with(|ctx| {
            if is_esm {
                // Evaluate as ECMAScript Module
                let module = Module::declare(ctx.clone(), filename.as_str(), code.as_str())?;
                // eval() returns (Module<Evaluated>, Promise) — we only need to drive evaluation
                let (_module_eval, _promise) = module.eval()?;
                Ok::<(), rquickjs::Error>(())
            } else {
                // CommonJS / script mode
                let setup = format!(
                    "globalThis.__filename = '{}'; globalThis.__dirname = '{}';",
                    filename.replace('\'', "\\'"),
                    dirname.replace('\'', "\\'"),
                );
                ctx.eval::<(), _>(setup.as_str())?;
                let _: rquickjs::Value = ctx.eval(code.as_str())?;
                Ok::<(), rquickjs::Error>(())
            }
        })?;

        Ok(())
    }

    /// Run the event loop: poll expired timers and fire callbacks until no pending timers remain.
    /// Uses a spin-sleep approach — appropriate for short-lived scripts.
    pub fn run_event_loop(&self) -> anyhow::Result<()> {
        use builtins::timers::TIMER_MANAGER;
        use std::sync::Arc;

        let manager = TIMER_MANAGER.with(|m| Arc::clone(m));

        // Loop until no more pending timers
        let max_iterations = 10_000; // safety limit
        let mut iterations = 0;

        while manager.has_pending() && iterations < max_iterations {
            iterations += 1;

            // If the next timer hasn't fired yet, sleep a short while
            if let Some(wait) = manager.next_expiry() {
                if wait.as_millis() > 0 {
                    std::thread::sleep(wait.min(std::time::Duration::from_millis(50)));
                }
            }

            // Fire any expired timers
            self.context.with(|ctx| {
                builtins::timers::TimerManager::fire_pending(&ctx, Arc::clone(&manager))
            })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_initialization() {
        let permissions = PermissionState::new();
        let engine = JsEngine::new(&permissions);

        assert!(engine.is_ok(), "Engine failed to initialize");
    }

    #[test]
    fn test_engine_evaluation() {
        let permissions = PermissionState::new();
        let engine = JsEngine::new(&permissions).unwrap();

        // Valid syntax should succeed
        let result = engine.eval("const x = 1 + 1;");
        assert!(result.is_ok());

        // Invalid syntax should fail
        let error_result = engine.eval("const x = ;");
        assert!(error_result.is_err());
    }

    #[test]
    fn test_eval_typescript() {
        let permissions = PermissionState::new();
        let engine = JsEngine::new(&permissions).unwrap();

        // TypeScript with type annotations — transpiler should strip them
        let ts_code = "const x: number = 42;";
        let js_code = transpiler::transpile(ts_code);
        let result = engine.eval(&js_code);
        assert!(
            result.is_ok(),
            "TS transpiled code should eval: {:?}",
            result
        );
    }
}
