use rquickjs::{Ctx, Function, Result, function::Rest};

pub fn inject_process(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();

    // Native process.exit(code?)
    globals.set(
        "__processExit",
        Function::new(ctx.clone(), |args: Rest<i32>| -> () {
            let code = args.0.into_iter().next().unwrap_or(0);
            std::process::exit(code);
        })?,
    )?;

    // Collect real environment variables into a JS object literal
    let env_json = serde_json::to_string(
        &std::env::vars().collect::<std::collections::HashMap<String, String>>(),
    )
    .unwrap_or_else(|_| "{}".to_string());

    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    };

    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "unknown"
    };

    let code = format!(
        r#"
        globalThis.process = {{
            version:  '3va/0.1.0',
            versions: {{ '3va': '0.1.0' }},
            platform: '{}',
            arch:     '{}',
            pid:      {},
            argv:     [],
            env:      {},
            exit:     function(code) {{ __processExit(code || 0); }},
            stdout:   {{ write: function(s) {{ console.log(s); }} }},
            stderr:   {{ write: function(s) {{ console.error(s); }} }},
            hrtime:   function() {{
                var t = Date.now();
                return [Math.floor(t / 1000), (t % 1000) * 1e6];
            }},
        }};
        "#,
        platform,
        arch,
        std::process::id(),
        env_json,
    );
    ctx.eval::<(), _>(code.as_str())?;

    Ok(())
}
