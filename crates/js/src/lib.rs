use rquickjs::{Context, Runtime};
use vvva_permissions::PermissionState;

pub struct JsEngine {
    runtime: Runtime,
    context: Context,
}

impl JsEngine {
    pub fn new(_permissions: &PermissionState) -> anyhow::Result<Self> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        
        Ok(Self {
            runtime,
            context,
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
