use rquickjs::{Array, Ctx, Function, Object, Result, function::Rest};

pub fn inject_process(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();

    // --- process.exit(code?) ---
    globals.set(
        "__processExit",
        Function::new(ctx.clone(), |args: Rest<i32>| -> () {
            let code = args.0.into_iter().next().unwrap_or(0);
            std::process::exit(code);
        })?,
    )?;

    // --- process object built via native Rust APIs (no format-string injection risk) ---
    let process = Object::new(ctx.clone())?;

    // Strings and numbers
    process.set("version", "3va/0.1.0")?;
    process.set("pid", std::process::id())?;

    let versions = Object::new(ctx.clone())?;
    versions.set("3va", "0.1.0")?;
    process.set("versions", versions)?;

    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "unknown"
    };
    process.set("platform", platform)?;

    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    };
    process.set("arch", arch)?;

    // argv: expose real command-line arguments
    let argv = Array::new(ctx.clone())?;
    for (i, arg) in std::env::args().enumerate() {
        argv.set(i, arg)?;
    }
    process.set("argv", argv)?;

    // env: each variable set as a native property — no JSON round-trip
    let env_obj = Object::new(ctx.clone())?;
    for (key, val) in std::env::vars() {
        env_obj.set(key, val)?;
    }
    process.set("env", env_obj)?;

    // exit(): delegate to the native __processExit binding
    ctx.eval::<(), _>("globalThis.__processExit = __processExit;")?;
    process.set(
        "exit",
        Function::new(ctx.clone(), |args: Rest<i32>| -> () {
            let code = args.0.into_iter().next().unwrap_or(0);
            std::process::exit(code);
        })?,
    )?;

    // stdout / stderr: write delegates to console methods installed by inject_console
    let stdout = Object::new(ctx.clone())?;
    stdout.set(
        "write",
        Function::new(ctx.clone(), |msg: String| print!("{msg}"))?,
    )?;
    process.set("stdout", stdout)?;

    let stderr = Object::new(ctx.clone())?;
    stderr.set(
        "write",
        Function::new(ctx.clone(), |msg: String| eprint!("{msg}"))?,
    )?;
    process.set("stderr", stderr)?;

    globals.set("process", process)?;

    // hrtime([prev]): returns [seconds, nanoseconds].
    // Implemented in JS so the return value is a native Array that callers can destructure.
    ctx.eval::<(), _>(
        r#"(function () {
            var _epoch = Date.now();
            process.hrtime = function (prev) {
                var ms = Date.now() - _epoch;
                var s  = Math.floor(ms / 1000);
                var ns = (ms % 1000) * 1000000;
                if (prev) { s -= prev[0]; ns -= prev[1]; if (ns < 0) { s -= 1; ns += 1000000000; } }
                return [s, ns];
            };
        }());"#,
    )?;

    Ok(())
}
