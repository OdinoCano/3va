use rquickjs::Ctx;

pub fn inject_timers(ctx: &Ctx) -> rquickjs::Result<()> {
    ctx.eval::<(), _>("global.setTimeout = function(fn, ms) { return 0; }")?;
    ctx.eval::<(), _>("global.clearTimeout = function(id) {}")?;
    ctx.eval::<(), _>("global.setInterval = function(fn, ms) { return 0; }")?;
    ctx.eval::<(), _>("global.clearInterval = function(id) {}")?;
    Ok(())
}