use rquickjs::{Ctx, Result, Function};
use vvva_permissions::{PermissionState, Capability};
use std::rc::Rc;
use std::cell::RefCell;

pub fn inject_fetch(ctx: &Ctx, permissions: Rc<RefCell<PermissionState>>) -> Result<()> {
    ctx.eval::<(), _>(r#"
        global.fetch = async function(url) {
            throw new Error('fetch() not implemented - network access requires --allow-net');
        }
    "#)?;
    Ok(())
}