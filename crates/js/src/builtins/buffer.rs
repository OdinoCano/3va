use rquickjs::Ctx;

pub fn inject_buffer(ctx: &Ctx) -> rquickjs::Result<()> {
    ctx.eval::<(), _>("global.Buffer = class Buffer { constructor(data) { this.data = data || []; } }")?;
    Ok(())
}