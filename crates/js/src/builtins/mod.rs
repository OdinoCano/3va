pub mod buffer;
pub mod console;
pub mod fetch;
pub mod fs;
pub mod modules;
pub mod process;
pub mod timers;

use rquickjs::Ctx;
use std::rc::Rc;
use std::cell::RefCell;
use vvva_permissions::PermissionState;

pub fn inject_all(ctx: &Ctx, permissions: Rc<RefCell<PermissionState>>) -> rquickjs::Result<()> {
    console::inject_console(ctx)?;
    timers::inject_timers(ctx)?;
    buffer::inject_buffer(ctx)?;
    process::inject_process(ctx)?;
    fetch::inject_fetch(ctx, permissions.clone())?;
    fs::inject_fs(ctx, permissions.clone())?;
    modules::inject_require(ctx, permissions)?;
    Ok(())
}