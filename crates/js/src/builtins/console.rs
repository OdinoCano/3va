use rquickjs::{Ctx, Function, Object, Result};

pub fn inject_console(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();
    let console = Object::new(ctx.clone())?;

    let log_func = Function::new(ctx.clone(), |msg: String| {
        println!("{}", msg);
    })?;
    console.set("log", log_func)?;

    let warn_func = Function::new(ctx.clone(), |msg: String| {
        eprintln!("[WARN] {}", msg);
    })?;
    console.set("warn", warn_func)?;

    let error_func = Function::new(ctx.clone(), |msg: String| {
        eprintln!("[ERROR] {}", msg);
    })?;
    console.set("error", error_func)?;

    let info_func = Function::new(ctx.clone(), |msg: String| {
        println!("[INFO] {}", msg);
    })?;
    console.set("info", info_func)?;

    let debug_func = Function::new(ctx.clone(), |msg: String| {
        println!("[DEBUG] {}", msg);
    })?;
    console.set("debug", debug_func)?;

    globals.set("console", console)?;

    Ok(())
}
