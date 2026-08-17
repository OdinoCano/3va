use v8::{ContextScope, HandleScope};

pub fn inject_vm(_scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    Ok(())
}
