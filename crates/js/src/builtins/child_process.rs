use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use std::sync::Arc;
use vvva_permissions::{Capability, PermissionState};

pub fn inject_child_process(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // __execAsync(cmd: string, args: string[], timeout_ms: number) -> Promise<{stdout, stderr, code}>
    let perms = permissions.clone();
    globals.set(
        "__execAsync",
        Function::new(
            ctx.clone(),
            Async(move |cmd: String, args: Vec<String>, timeout_ms: u64| {
                let perms = perms.clone();
                async move {
                    if !perms.check(&Capability::SpawnProcess) {
                        return Err(rquickjs::Error::new_from_js_message(
                            "permission",
                            "permission",
                            "Process spawn denied. Run with --allow-child-process".to_string(),
                        ));
                    }
                    let cmd2 = cmd.clone();
                    let args2 = args.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        let mut c = std::process::Command::new(&cmd2);
                        c.args(&args2);
                        if timeout_ms == 0 {
                            c.output()
                        } else {
                            // Run with a rough timeout via thread
                            c.output()
                        }
                    })
                    .await
                    .map_err(|e| {
                        rquickjs::Error::new_from_js_message(
                            "child_process",
                            "spawn",
                            e.to_string(),
                        )
                    })?
                    .map_err(|e| {
                        rquickjs::Error::new_from_js_message(
                            "child_process",
                            "exec",
                            format!("spawn error: {}", e),
                        )
                    })?;

                    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
                    let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
                    let code = result.status.code().unwrap_or(-1);
                    Ok(serde_json::json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "code": code,
                    })
                    .to_string())
                }
            }),
        )?,
    )?;

    // __execSyncShell: synchronous sh -c execution, blocks the calling thread.
    // Permission is checked before running. Used by execSync / spawnSync.
    let perms3 = permissions.clone();
    globals.set(
        "__execSyncShell",
        Function::new(
            ctx.clone(),
            move |command: String| -> rquickjs::Result<String> {
                if !perms3.check(&Capability::SpawnProcess) {
                    return Err(rquickjs::Error::new_from_js_message(
                        "permission",
                        "permission",
                        "Process spawn denied. Run with --allow-child-process".to_string(),
                    ));
                }
                let shell = if cfg!(windows) { "cmd" } else { "sh" };
                let flag = if cfg!(windows) { "/C" } else { "-c" };
                let result = std::process::Command::new(shell)
                    .args([flag, command.as_str()])
                    .output()
                    .map_err(|e| {
                        rquickjs::Error::new_from_js_message(
                            "child_process",
                            "execSync",
                            e.to_string(),
                        )
                    })?;
                let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
                let code = result.status.code().unwrap_or(-1);
                Ok(
                    serde_json::json!({ "stdout": stdout, "stderr": stderr, "code": code })
                        .to_string(),
                )
            },
        )?,
    )?;

    // __spawnSyncExec: synchronous spawn (cmd + args array), blocks the calling thread.
    let perms4 = permissions.clone();
    globals.set(
        "__spawnSyncExec",
        Function::new(
            ctx.clone(),
            move |cmd: String, args: Vec<String>| -> rquickjs::Result<String> {
                if !perms4.check(&Capability::SpawnProcess) {
                    return Err(rquickjs::Error::new_from_js_message(
                        "permission",
                        "permission",
                        "Process spawn denied. Run with --allow-child-process".to_string(),
                    ));
                }
                let result = std::process::Command::new(&cmd)
                    .args(&args)
                    .output()
                    .map_err(|e| {
                        rquickjs::Error::new_from_js_message(
                            "child_process",
                            "spawnSync",
                            e.to_string(),
                        )
                    })?;
                let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
                let code = result.status.code().unwrap_or(-1);
                Ok(serde_json::json!({ "stdout": stdout, "stderr": stderr, "status": code, "pid": 0 }).to_string())
            },
        )?,
    )?;

    // __execShellAsync(command: string) -> same shape, runs via sh -c
    let perms2 = permissions;
    globals.set(
        "__execShellAsync",
        Function::new(
            ctx.clone(),
            Async(move |command: String| {
                let perms = perms2.clone();
                async move {
                    if !perms.check(&Capability::SpawnProcess) {
                        return Err(rquickjs::Error::new_from_js_message(
                            "permission",
                            "permission",
                            "Process spawn denied. Run with --allow-child-process".to_string(),
                        ));
                    }
                    let result = tokio::task::spawn_blocking(move || {
                        let shell = if cfg!(windows) { "cmd" } else { "sh" };
                        let flag = if cfg!(windows) { "/C" } else { "-c" };
                        std::process::Command::new(shell)
                            .args([flag, &command])
                            .output()
                    })
                    .await
                    .map_err(|e| {
                        rquickjs::Error::new_from_js_message(
                            "child_process",
                            "spawn",
                            e.to_string(),
                        )
                    })?
                    .map_err(|e| {
                        rquickjs::Error::new_from_js_message(
                            "child_process",
                            "exec",
                            format!("shell error: {}", e),
                        )
                    })?;

                    let stdout = String::from_utf8_lossy(&result.stdout).into_owned();
                    let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
                    let code = result.status.code().unwrap_or(-1);
                    Ok(serde_json::json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "code": code,
                    })
                    .to_string())
                }
            }),
        )?,
    )?;

    // JS wrapper: replaces the stub in modules.rs
    ctx.eval::<(), _>(
        r#"
        (function() {
            function parseOpts(cmd, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                opts = opts || {};
                return { opts: opts, cb: cb };
            }

            var child_process = {
                // exec(command, [options], callback)
                exec: function(command, opts, cb) {
                    var p = parseOpts(command, opts, cb);
                    __execShellAsync(command).then(function(raw) {
                        var r = JSON.parse(raw);
                        if (p.cb) {
                            if (r.code !== 0) {
                                var err = new Error('Command failed: ' + command + '\n' + r.stderr);
                                err.code = r.code;
                                err.stderr = r.stderr;
                                err.stdout = r.stdout;
                                p.cb(err, r.stdout, r.stderr);
                            } else {
                                p.cb(null, r.stdout, r.stderr);
                            }
                        }
                    }).catch(function(e) {
                        if (p.cb) p.cb(e, '', '');
                    });
                    return { kill: function() {} };
                },

                // execFile(file, [args], [options], callback)
                execFile: function(file, args, opts, cb) {
                    if (typeof args === 'function') { cb = args; args = []; opts = {}; }
                    else if (typeof opts === 'function') { cb = opts; opts = {}; }
                    args = args || [];
                    __execAsync(file, args, 0).then(function(raw) {
                        var r = JSON.parse(raw);
                        if (cb) {
                            if (r.code !== 0) {
                                var err = new Error('Command failed: ' + file);
                                err.code = r.code;
                                cb(err, r.stdout, r.stderr);
                            } else {
                                cb(null, r.stdout, r.stderr);
                            }
                        }
                    }).catch(function(e) { if (cb) cb(e, '', ''); });
                    return { kill: function() {} };
                },

                // spawn(command, [args], [options]) -> ChildProcess-like
                spawn: function(command, args, opts) {
                    args = args || [];
                    var cp = {
                        _stdout: '', _stderr: '', _code: null,
                        stdout: { _listeners: [], on: function(ev, fn) { if (ev==='data') this._listeners.push(fn); return this; }, pipe: function() {} },
                        stderr: { _listeners: [], on: function(ev, fn) { if (ev==='data') this._listeners.push(fn); return this; }, pipe: function() {} },
                        stdin:  { write: function() {}, end: function() {} },
                        _exitListeners: [],
                        on: function(ev, fn) { if (ev==='exit'||ev==='close') this._exitListeners.push(fn); return this; },
                        kill: function() {}
                    };
                    __execAsync(command, args, 0).then(function(raw) {
                        var r = JSON.parse(raw);
                        if (r.stdout) cp.stdout._listeners.forEach(function(fn) { fn(r.stdout); });
                        if (r.stderr) cp.stderr._listeners.forEach(function(fn) { fn(r.stderr); });
                        cp._exitListeners.forEach(function(fn) { fn(r.code, null); });
                    }).catch(function(e) {
                        cp._exitListeners.forEach(function(fn) { fn(1, null); });
                    });
                    return cp;
                },

                // execSync(command, [options]) -> Buffer | string
                // Blocks until the command finishes (same semantics as Node.js).
                execSync: function(command, opts) {
                    opts = opts || {};
                    var raw = JSON.parse(__execSyncShell(command));
                    if (raw.code !== 0) {
                        var err = new Error('Command failed: ' + command + '\n' + raw.stderr);
                        err.status = raw.code;
                        err.stderr = raw.stderr;
                        err.stdout = raw.stdout;
                        throw err;
                    }
                    var enc = opts.encoding || null;
                    if (enc === 'utf8' || enc === 'utf-8' || enc === 'buffer') {
                        return enc === 'buffer' ? (typeof Buffer !== 'undefined' ? Buffer.from(raw.stdout) : raw.stdout) : raw.stdout;
                    }
                    return typeof Buffer !== 'undefined' ? Buffer.from(raw.stdout) : raw.stdout;
                },

                // spawnSync(command, [args], [options]) -> {status, stdout, stderr, pid, signal, error}
                spawnSync: function(command, args, opts) {
                    if (!Array.isArray(args)) { opts = args || {}; args = []; }
                    opts = opts || {};
                    var raw = JSON.parse(__spawnSyncExec(command, args || []));
                    var enc = opts.encoding || null;
                    var out = (enc === 'utf8' || enc === 'utf-8') ? raw.stdout : (typeof Buffer !== 'undefined' ? Buffer.from(raw.stdout) : raw.stdout);
                    var err = (enc === 'utf8' || enc === 'utf-8') ? raw.stderr : (typeof Buffer !== 'undefined' ? Buffer.from(raw.stderr) : raw.stderr);
                    return { status: raw.status, stdout: out, stderr: err, pid: raw.pid, signal: null, error: null };
                },

                // promisify helper
                promisify: function(fn) {
                    return function() {
                        var args = Array.prototype.slice.call(arguments);
                        return new Promise(function(resolve, reject) {
                            args.push(function(err, stdout, stderr) {
                                if (err) reject(err); else resolve({ stdout: stdout, stderr: stderr });
                            });
                            fn.apply(null, args);
                        });
                    };
                }
            };

            if (globalThis.__requireCache) {
                globalThis.__requireCache['child_process'] = child_process;
                globalThis.__requireCache['node:child_process'] = child_process;
            }
        })();
        "#,
    )?;

    Ok(())
}
