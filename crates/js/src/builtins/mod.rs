pub mod buffer;
pub mod console;
pub mod process;
pub mod timers;

use rquickjs::Ctx;

pub fn inject_all(ctx: &Ctx) -> rquickjs::Result<()> {
    console::inject_console(ctx)?;
    timers::inject_timers(ctx)?;
    buffer::inject_buffer(ctx)?;
    process::inject_process(ctx)?;
    Ok(())
}