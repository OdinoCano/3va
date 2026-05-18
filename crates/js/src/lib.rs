pub mod builtins;

use rquickjs::{Context, Runtime};
use vvva_permissions::PermissionState;
use std::rc::Rc;
use std::cell::RefCell;

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
}
