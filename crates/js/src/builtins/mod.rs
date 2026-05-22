pub mod buffer;
pub mod console;
pub mod fetch;
pub mod fs;
pub mod modules;
pub mod process;
pub mod timers;
pub mod websocket;

use rquickjs::Ctx;
use std::sync::Arc;
use vvva_permissions::PermissionState;

pub use timers::TimerManager;

pub fn inject_all(
    ctx: &Ctx,
    permissions: Arc<PermissionState>,
    timer_manager: Arc<TimerManager>,
) -> rquickjs::Result<()> {
    console::inject_console(ctx)?;
    timers::inject_timers(ctx, timer_manager)?;
    buffer::inject_buffer(ctx)?;
    process::inject_process(ctx)?;
    fetch::inject_fetch(ctx, permissions.clone())?;
    fs::inject_fs(ctx, permissions.clone())?;
    modules::inject_require(ctx, permissions.clone())?;
    websocket::inject_websocket(ctx, permissions)?;
    Ok(())
}
