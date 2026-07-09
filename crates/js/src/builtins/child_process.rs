use std::sync::Arc;
use v8::{
    ContextScope, Function, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue, Script,
    String as V8String,
};
use vvva_permissions::{Capability, PermissionState};

static CHILD_PROCESS_PERMISSIONS: std::sync::OnceLock<Arc<PermissionState>> =
    std::sync::OnceLock::new();
fn perms() -> &'static Arc<PermissionState> {
    CHILD_PROCESS_PERMISSIONS.get().unwrap()
}

pub fn inject_child_process(
    scope: &mut ContextScope<HandleScope>,
    permissions: Arc<PermissionState>,
) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    CHILD_PROCESS_PERMISSIONS.set(permissions).ok();
    let exec_async_fn = Function::new(
        scope,
        move |_scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let cmd_arg = args.get(0);
            let cmd = cmd_arg
                .to_string(_scope)
                .map(|s| s.to_rust_string_lossy(_scope))
                .unwrap_or_default();
            let args_arg = args.get(1);
            let args_vec: Vec<String> = if args_arg.is_array() {
                let arr = v8::Local::<v8::Array>::try_from(args_arg).unwrap();
                (0..arr.length())
                    .filter_map(|i| {
                        arr.get_index(_scope, i).and_then(|v| {
                            v.to_string(_scope).map(|s| s.to_rust_string_lossy(_scope))
                        })
                    })
                    .collect()
            } else {
                vec![]
            };
            let timeout_ms_arg = args.get(2);
            let _timeout_ms: u64 = timeout_ms_arg.uint32_value(_scope).unwrap_or(0) as u64;

            if !perms().check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
                rv.set(err_str.into());
                return;
            }

            let result = tokio::task::block_in_place(|| {
                let mut c = std::process::Command::new(&cmd);
                c.args(&args_vec);
                c.output()
            });

            match result {
                Ok(output) => {
                    let stdout = std::string::String::from_utf8_lossy(&output.stdout).into_owned();
                    let stderr = std::string::String::from_utf8_lossy(&output.stderr).into_owned();
                    let code = output.status.code().unwrap_or(-1);
                    let json = serde_json::json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "code": code,
                    })
                    .to_string();
                    let result_str = V8String::new(_scope, &json).unwrap();
                    rv.set(result_str.into());
                }
                Err(e) => {
                    let err_str = V8String::new(_scope, &format!("spawn error: {}", e)).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__execAsync").unwrap().into(),
        exec_async_fn.into(),
    );

    let exec_sync_shell_fn = Function::new(
        scope,
        move |_scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let cmd_arg = args.get(0);
            let command = cmd_arg.to_rust_string_lossy(_scope);

            if !perms().check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
                rv.set(err_str.into());
                return;
            }

            let shell = if cfg!(windows) { "cmd" } else { "sh" };
            let flag = if cfg!(windows) { "/C" } else { "-c" };
            let result = std::process::Command::new(shell)
                .args([flag, command.as_str()])
                .output();

            match result {
                Ok(output) => {
                    let stdout = std::string::String::from_utf8_lossy(&output.stdout).into_owned();
                    let stderr = std::string::String::from_utf8_lossy(&output.stderr).into_owned();
                    let code = output.status.code().unwrap_or(-1);
                    let json =
                        serde_json::json!({ "stdout": stdout, "stderr": stderr, "code": code })
                            .to_string();
                    let result_str = V8String::new(_scope, &json).unwrap();
                    rv.set(result_str.into());
                }
                Err(e) => {
                    let err_str = V8String::new(_scope, &format!("execSync error: {}", e)).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__execSyncShell").unwrap().into(),
        exec_sync_shell_fn.into(),
    );

    let spawn_sync_exec_fn = Function::new(scope, move |_scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
        let cmd_arg = args.get(0);
        let cmd = cmd_arg.to_string(_scope).map(|s| s.to_rust_string_lossy(_scope)).unwrap_or_default();
        let args_arg = args.get(1);
        let args_vec: Vec<String> = if args_arg.is_array() {
            let arr = v8::Local::<v8::Array>::try_from(args_arg).unwrap();
            (0..arr.length()).filter_map(|i| {
                arr.get_index(_scope, i).and_then(|v| v.to_string(_scope).map(|s| s.to_rust_string_lossy(_scope)))
            }).collect()
        } else {
            vec![]
        };

        if !perms().check(&Capability::SpawnProcess) {
            let err_str = V8String::new(_scope, "Process spawn denied. Run with --allow-child-process").unwrap();
            rv.set(err_str.into());
            return;
        }

        let result = std::process::Command::new(&cmd)
            .args(&args_vec)
            .output();

        match result {
            Ok(output) => {
                let stdout = std::string::String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = std::string::String::from_utf8_lossy(&output.stderr).into_owned();
                let code = output.status.code().unwrap_or(-1);
                let json = serde_json::json!({ "stdout": stdout, "stderr": stderr, "status": code, "pid": 0 }).to_string();
                let result_str = V8String::new(_scope, &json).unwrap();
                rv.set(result_str.into());
            }
            Err(e) => {
                let err_str = V8String::new(_scope, &format!("spawnSync error: {}", e)).unwrap();
                rv.set(err_str.into());
            }
        }
    }).unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnSyncExec").unwrap().into(),
        spawn_sync_exec_fn.into(),
    );

    let exec_shell_async_fn = Function::new(
        scope,
        move |_scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let cmd_arg = args.get(0);
            let command = cmd_arg
                .to_string(_scope)
                .map(|s| s.to_rust_string_lossy(_scope))
                .unwrap_or_default();

            if !perms().check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
                rv.set(err_str.into());
                return;
            }

            let result = tokio::task::block_in_place(|| {
                let shell = if cfg!(windows) { "cmd" } else { "sh" };
                let flag = if cfg!(windows) { "/C" } else { "-c" };
                std::process::Command::new(shell)
                    .args([flag, &command])
                    .output()
            });

            match result {
                Ok(output) => {
                    let stdout = std::string::String::from_utf8_lossy(&output.stdout).into_owned();
                    let stderr = std::string::String::from_utf8_lossy(&output.stderr).into_owned();
                    let code = output.status.code().unwrap_or(-1);
                    let json = serde_json::json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "code": code,
                    })
                    .to_string();
                    let result_str = V8String::new(_scope, &json).unwrap();
                    rv.set(result_str.into());
                }
                Err(e) => {
                    let err_str = V8String::new(_scope, &format!("shell error: {}", e)).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__execShellAsync").unwrap().into(),
        exec_shell_async_fn.into(),
    );

    let spawn_with_input_fn = Function::new(
        scope,
        move |_scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let cmd_arg = args.get(0);
            let cmd = cmd_arg
                .to_string(_scope)
                .map(|s| s.to_rust_string_lossy(_scope))
                .unwrap_or_default();
            let args_arg = args.get(1);
            let args_vec: Vec<String> = if args_arg.is_array() {
                let arr = v8::Local::<v8::Array>::try_from(args_arg).unwrap();
                (0..arr.length())
                    .filter_map(|i| {
                        arr.get_index(_scope, i).and_then(|v| {
                            v.to_string(_scope).map(|s| s.to_rust_string_lossy(_scope))
                        })
                    })
                    .collect()
            } else {
                vec![]
            };
            let stdin_arg = args.get(2);
            let stdin_data = stdin_arg
                .to_string(_scope)
                .map(|s| s.to_rust_string_lossy(_scope))
                .unwrap_or_default();

            if !perms().check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
                rv.set(err_str.into());
                return;
            }

            let result = tokio::task::block_in_place(|| {
                use std::io::Write;
                let mut child = std::process::Command::new(&cmd)
                    .args(&args_vec)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()?;
                if !stdin_data.is_empty() {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(stdin_data.as_bytes());
                    }
                } else {
                    drop(child.stdin.take());
                }
                child.wait_with_output()
            });

            match result {
                Ok(output) => {
                    let stdout = std::string::String::from_utf8_lossy(&output.stdout).into_owned();
                    let stderr = std::string::String::from_utf8_lossy(&output.stderr).into_owned();
                    let code = output.status.code().unwrap_or(-1);
                    let json =
                        serde_json::json!({"stdout": stdout, "stderr": stderr, "code": code})
                            .to_string();
                    let result_str = V8String::new(_scope, &json).unwrap();
                    rv.set(result_str.into());
                }
                Err(e) => {
                    let err_str = V8String::new(_scope, &format!("spawn error: {}", e)).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnWithInput").unwrap().into(),
        spawn_with_input_fn.into(),
    );

    let spawn_sync_with_stdin_fn = Function::new(scope, move |_scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
        let cmd_arg = args.get(0);
        let cmd = cmd_arg.to_string(_scope).map(|s| s.to_rust_string_lossy(_scope)).unwrap_or_default();
        let args_arg = args.get(1);
        let args_vec: Vec<String> = if args_arg.is_array() {
            let arr = v8::Local::<v8::Array>::try_from(args_arg).unwrap();
            (0..arr.length()).filter_map(|i| {
                arr.get_index(_scope, i).and_then(|v| v.to_string(_scope).map(|s| s.to_rust_string_lossy(_scope)))
            }).collect()
        } else {
            vec![]
        };
        let stdin_arg = args.get(2);
        let stdin_data = stdin_arg.to_string(_scope).map(|s| s.to_rust_string_lossy(_scope)).unwrap_or_default();

        if !perms().check(&Capability::SpawnProcess) {
            let err_str = V8String::new(_scope, "Process spawn denied. Run with --allow-child-process").unwrap();
            rv.set(err_str.into());
            return;
        }

        use std::io::Write;
        let mut child = std::process::Command::new(&cmd)
            .args(&args_vec)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        match child {
            Ok(ref mut c) => {
                if !stdin_data.is_empty() {
                    if let Some(mut stdin) = c.stdin.take() {
                        let _ = stdin.write_all(stdin_data.as_bytes());
                    }
                } else {
                    drop(c.stdin.take());
                }
            }
            Err(e) => {
                let err_str = V8String::new(_scope, &format!("spawn error: {}", e)).unwrap();
                rv.set(err_str.into());
                return;
            }
        }

        match child.unwrap().wait_with_output() {
            Ok(output) => {
                let stdout = std::string::String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = std::string::String::from_utf8_lossy(&output.stderr).into_owned();
                let status = output.status.code().unwrap_or(-1);
                let json = serde_json::json!({"stdout": stdout, "stderr": stderr, "status": status, "pid": 0}).to_string();
                let result_str = V8String::new(_scope, &json).unwrap();
                rv.set(result_str.into());
            }
            Err(e) => {
                let err_str = V8String::new(_scope, &format!("spawn error: {}", e)).unwrap();
                rv.set(err_str.into());
            }
        }
    }).unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnSyncWithStdin").unwrap().into(),
        spawn_sync_with_stdin_fn.into(),
    );

    let js_code = r#"
        (function() {
            function parseOpts(cmd, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                opts = opts || {};
                return { opts: opts, cb: cb };
            }

            var child_process = {
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

                spawn: function(command, args, opts) {
                    args = args || [];
                    opts = opts || {};
                    var stdinChunks = [];
                    var started = false;

                    function runWith(stdinData) {
                        if (stdinData) {
                            __spawnWithInput(command, args, stdinData).then(function(raw) {
                                var r = JSON.parse(raw);
                                if (r.stdout) cp.stdout._listeners.forEach(function(fn) { fn(r.stdout); });
                                if (r.stderr) cp.stderr._listeners.forEach(function(fn) { fn(r.stderr); });
                                cp._exitListeners.forEach(function(fn) { fn(r.code, null); });
                            }).catch(function(e) {
                                cp._exitListeners.forEach(function(fn) { fn(1, null); });
                            });
                        } else {
                            __execAsync(command, args, 0).then(function(raw) {
                                var r = JSON.parse(raw);
                                if (r.stdout) cp.stdout._listeners.forEach(function(fn) { fn(r.stdout); });
                                if (r.stderr) cp.stderr._listeners.forEach(function(fn) { fn(r.stderr); });
                                cp._exitListeners.forEach(function(fn) { fn(r.code, null); });
                            }).catch(function(e) {
                                cp._exitListeners.forEach(function(fn) { fn(1, null); });
                            });
                        }
                    }

                    var cp = {
                        _stdout: '', _stderr: '', _code: null,
                        stdout: { _listeners: [], on: function(ev, fn) { if (ev==='data') this._listeners.push(fn); return this; }, pipe: function() {} },
                        stderr: { _listeners: [], on: function(ev, fn) { if (ev==='data') this._listeners.push(fn); return this; }, pipe: function() {} },
                        stdin: {
                            write: function(chunk) {
                                stdinChunks.push(typeof chunk === 'string' ? chunk : String(chunk));
                                return true;
                            },
                            end: function(chunk) {
                                if (chunk !== undefined) stdinChunks.push(typeof chunk === 'string' ? chunk : String(chunk));
                                if (started) return;
                                started = true;
                                runWith(stdinChunks.join(''));
                            }
                        },
                        _exitListeners: [],
                        on: function(ev, fn) { if (ev==='exit'||ev==='close') this._exitListeners.push(fn); return this; },
                        kill: function() {}
                    };

                    Promise.resolve().then(function() {
                        if (!started) { started = true; runWith(''); }
                    });
                    return cp;
                },

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

                spawnSync: function(command, args, opts) {
                    if (!Array.isArray(args)) { opts = args || {}; args = []; }
                    opts = opts || {};
                    var enc = opts.encoding || null;
                    var raw;
                    if (opts.input !== undefined) {
                        var inputStr = typeof opts.input === 'string' ? opts.input : String(opts.input);
                        raw = JSON.parse(__spawnSyncWithStdin(command, args || [], inputStr));
                    } else {
                        raw = JSON.parse(__spawnSyncExec(command, args || []));
                    }
                    var out = (enc === 'utf8' || enc === 'utf-8') ? raw.stdout : (typeof Buffer !== 'undefined' ? Buffer.from(raw.stdout) : raw.stdout);
                    var err = (enc === 'utf8' || enc === 'utf-8') ? raw.stderr : (typeof Buffer !== 'undefined' ? Buffer.from(raw.stderr) : raw.stderr);
                    return { status: raw.status, stdout: out, stderr: err, pid: raw.pid || 0, signal: null, error: null };
                },

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
    "#;
    let source = V8String::new(scope, js_code).unwrap();
    let _ = Script::compile(scope, source, None).and_then(|s| s.run(scope));

    Ok(())
}
