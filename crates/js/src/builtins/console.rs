use rquickjs::Ctx;

pub fn inject_console(ctx: &Ctx) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(r#"global.console = { log: function() { this._log.apply(this, arguments); } }"#)?;
    Ok(())
}