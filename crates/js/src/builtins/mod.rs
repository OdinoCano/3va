pub mod buffer;
pub mod child_process;
pub mod console;
pub mod crypto;
pub mod fetch;
pub mod fs;
pub mod modules;
pub mod process;
pub mod timers;
pub mod websocket;
pub mod zlib;

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
    process::inject_process(ctx, permissions.clone())?;
    // Node.js packages expect `global` and `globalThis` to be the same object.
    ctx.eval::<(), _>("globalThis.global = globalThis; globalThis.GLOBAL = globalThis;")?;
    fetch::inject_fetch(ctx, permissions.clone())?;
    fs::inject_fs(ctx, permissions.clone())?;
    modules::inject_require(ctx, permissions.clone())?;
    websocket::inject_websocket(ctx, permissions.clone())?;
    // These run after inject_require so they can overwrite the placeholder stubs
    zlib::inject_zlib(ctx)?;
    child_process::inject_child_process(ctx, permissions)?;
    crypto::inject_crypto(ctx)?;
    Ok(())
}
