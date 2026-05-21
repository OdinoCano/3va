use rquickjs::{Ctx, Function, Result};

pub fn inject_console(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();

    // Single native sink: level ("log"|"info"|"warn"|"error"|"debug") + formatted message.
    // All multi-arg formatting and type coercion happens in the JS polyfill below.
    globals.set(
        "__console_write",
        Function::new(ctx.clone(), |level: String, msg: String| match level.as_str() {
            "warn" => eprintln!("[WARN] {msg}"),
            "error" => eprintln!("[ERROR] {msg}"),
            "info" => println!("[INFO] {msg}"),
            "debug" => println!("[DEBUG] {msg}"),
            _ => println!("{msg}"),
        })?,
    )?;

    // JS polyfill: handles variadic args, type coercion, and object serialization exactly
    // as Node.js console does (strings pass through, everything else gets JSON.stringify).
    ctx.eval::<(), _>(
        r#"(function () {
            function fmt(args) {
                var out = [];
                for (var i = 0; i < args.length; i++) {
                    var a = args[i];
                    if (typeof a === 'string') {
                        out.push(a);
                    } else if (a === null) {
                        out.push('null');
                    } else if (a === undefined) {
                        out.push('undefined');
                    } else if (typeof a === 'object' || Array.isArray(a)) {
                        try { out.push(JSON.stringify(a)); } catch (_) { out.push('[object Object]'); }
                    } else {
                        out.push(String(a));
                    }
                }
                return out.join(' ');
            }
            globalThis.console = {
                log:   function () { __console_write('log',   fmt(arguments)); },
                info:  function () { __console_write('info',  fmt(arguments)); },
                warn:  function () { __console_write('warn',  fmt(arguments)); },
                error: function () { __console_write('error', fmt(arguments)); },
                debug: function () { __console_write('debug', fmt(arguments)); },
            };
        }());"#,
    )?;

    Ok(())
}
