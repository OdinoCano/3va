use rquickjs::Ctx;

pub fn inject_process(ctx: &Ctx) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(
        r#"globalThis.process = { version: "3va/0.1.0", platform: "linux", arch: "x64", pid: 0 }"#,
    )?;
    Ok(())
}
