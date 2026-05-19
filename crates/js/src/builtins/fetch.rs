use rquickjs::{Ctx, Result};
use std::cell::RefCell;
use std::rc::Rc;
use vvva_permissions::PermissionState;

pub fn inject_fetch(ctx: &Ctx, _permissions: Rc<RefCell<PermissionState>>) -> Result<()> {
    ctx.eval::<(), _>(
        r#"
        globalThis.fetch = async function(url) {
            throw new Error('fetch() not implemented - network access requires --allow-net');
        }
    "#,
    )?;
    Ok(())
}
