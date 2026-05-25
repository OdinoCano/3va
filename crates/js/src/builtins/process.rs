use rquickjs::{Array, Ctx, Function, Object, Result, function::Rest};
use std::sync::Arc;
use vvva_permissions::{Capability, PermissionState};

pub fn inject_process(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
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
    // Expose fake Node.js-compatible version strings so packages checking
    // process.versions.node / process.versions.v8 don't crash.
    versions.set("node", "20.0.0")?;
    versions.set("v8", "11.3.244.8-node.20")?;
    versions.set("uv", "1.44.2")?;
    versions.set("zlib", "1.2.13")?;
    versions.set("openssl", "3.0.0")?;
    versions.set("modules", "115")?;
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

    // argv[0] = path to runtime binary (Node-compatible convention).
    // argv[1] = script path and argv[2+] = script args are set by eval_file/eval_file_with_args.
    let argv = Array::new(ctx.clone())?;
    let bin = std::env::args().next().unwrap_or_else(|| "3va".to_string());
    argv.set(0usize, bin)?;
    process.set("argv", argv)?;

    // env: only expose variables that pass the permission check.
    // EnvAccess (all) or EnvVar(key) grants are both accepted by caps_match.
    let env_obj = Object::new(ctx.clone())?;
    for (key, val) in std::env::vars() {
        if permissions.check(&Capability::EnvVar(key.clone())) {
            env_obj.set(key, val)?;
        }
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

    // cwd(): return the process working directory
    let cwd_str = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("/"))
        .to_string_lossy()
        .to_string();
    let cwd_str_clone = cwd_str.clone();
    process.set(
        "cwd",
        Function::new(ctx.clone(), move || cwd_str_clone.clone())?,
    )?;

    // chdir(): no-op stub — sandboxed runtime doesn't change working dir
    process.set(
        "chdir",
        Function::new(ctx.clone(), |_: Rest<String>| {})?,
    )?;

    // stdout / stderr: write delegates to console methods installed by inject_console
    let stdout = Object::new(ctx.clone())?;
    stdout.set(
        "write",
        Function::new(ctx.clone(), |msg: String| print!("{msg}"))?,
    )?;
    stdout.set("fd", 1i32)?;
    stdout.set("isTTY", false)?;
    process.set("stdout", stdout)?;

    let stderr = Object::new(ctx.clone())?;
    stderr.set(
        "write",
        Function::new(ctx.clone(), |msg: String| eprint!("{msg}"))?,
    )?;
    stderr.set("fd", 2i32)?;
    stderr.set("isTTY", false)?;
    process.set("stderr", stderr)?;

    globals.set("process", process)?;

    // hrtime, nextTick, setImmediate — implemented in JS after process is on globalThis.
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
            process.hrtime.bigint = function() { return BigInt(Date.now()) * BigInt(1000000); };

            // nextTick: schedule callback in a microtask (closest to Node's behaviour in QuickJS)
            var _nextTickQueue = [];
            var _nextTickScheduled = false;
            process.nextTick = function(cb) {
                var args = Array.prototype.slice.call(arguments, 1);
                _nextTickQueue.push({ fn: cb, args: args });
                if (!_nextTickScheduled) {
                    _nextTickScheduled = true;
                    Promise.resolve().then(function() {
                        _nextTickScheduled = false;
                        var queue = _nextTickQueue.splice(0);
                        for (var i = 0; i < queue.length; i++) {
                            try { queue[i].fn.apply(null, queue[i].args); } catch(e) {}
                        }
                    });
                }
            };

            // setImmediate / clearImmediate
            if (typeof setImmediate === 'undefined') {
                globalThis.setImmediate = function(cb) {
                    var args = Array.prototype.slice.call(arguments, 1);
                    return setTimeout(function() { cb.apply(null, args); }, 0);
                };
                globalThis.clearImmediate = clearTimeout;
            }
        }());"#,
    )?;

    Ok(())
}
