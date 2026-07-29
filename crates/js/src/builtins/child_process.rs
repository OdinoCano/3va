use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use v8::{
    ContextScope, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue, Script,
    String as V8String,
};
use vvva_permissions::{Capability, PermissionState};

struct StreamChild {
    stdin: Option<std::process::ChildStdin>,
    stdout_buf: Arc<Mutex<Vec<u8>>>,
    done: Arc<AtomicBool>,
    _child: std::process::Child,
}

static CHILD_TABLE: OnceLock<Mutex<HashMap<u32, StreamChild>>> = OnceLock::new();
static NEXT_CHILD_ID: AtomicU32 = AtomicU32::new(1);

fn child_table() -> &'static Mutex<HashMap<u32, StreamChild>> {
    CHILD_TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn inject_child_process(
    scope: &mut ContextScope<HandleScope>,
    permissions: Arc<PermissionState>,
) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    // Leak once per engine and hand every closure a pointer via v8::External,
    // instead of a process-wide static (which corrupted permission checks
    // across concurrently-running engines/tests).
    let perms_ptr = Arc::into_raw(permissions) as *mut std::ffi::c_void;
    let external = v8::External::new(scope, perms_ptr);

    let exec_async_fn = v8::Function::builder(
        |_scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let perms = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const PermissionState)
            };
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

            if !perms.check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
                rv.set(err_str.into());
                return;
            }

            // Plain blocking std::process::Command — not wrapped in
            // tokio::task::block_in_place, which requires a multi_thread
            // runtime and panics outright on current_thread (e.g. plain
            // `#[tokio::test]`); see fs_watch's __fsWatchNext fix for the
            // same bug pattern. Nothing here is a tokio operation.
            let result = {
                let mut c = std::process::Command::new(&cmd);
                c.args(&args_vec);
                c.output()
            };

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
    .data(external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__execAsync").unwrap().into(),
        exec_async_fn.into(),
    );

    let exec_sync_shell_fn = v8::Function::builder(
        |_scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let perms = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const PermissionState)
            };
            let cmd_arg = args.get(0);
            let command = cmd_arg.to_rust_string_lossy(_scope);

            if !perms.check(&Capability::SpawnProcess) {
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
    .data(external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__execSyncShell").unwrap().into(),
        exec_sync_shell_fn.into(),
    );

    let spawn_sync_exec_fn = v8::Function::builder(
        |_scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let perms = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const PermissionState)
            };
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

            if !perms.check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
                rv.set(err_str.into());
                return;
            }

            let result = std::process::Command::new(&cmd).args(&args_vec).output();

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
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnSyncExec").unwrap().into(),
        spawn_sync_exec_fn.into(),
    );

    let exec_shell_async_fn = v8::Function::builder(
        |_scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let perms = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const PermissionState)
            };
            let cmd_arg = args.get(0);
            let command = cmd_arg
                .to_string(_scope)
                .map(|s| s.to_rust_string_lossy(_scope))
                .unwrap_or_default();

            if !perms.check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
                rv.set(err_str.into());
                return;
            }

            // See the plain-blocking-command comment above __execAsync's
            // native fn — same reasoning, no block_in_place needed.
            let result = {
                let shell = if cfg!(windows) { "cmd" } else { "sh" };
                let flag = if cfg!(windows) { "/C" } else { "-c" };
                std::process::Command::new(shell)
                    .args([flag, &command])
                    .output()
            };

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
    .data(external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__execShellAsync").unwrap().into(),
        exec_shell_async_fn.into(),
    );

    let spawn_with_input_fn = v8::Function::builder(
        |_scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let perms = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const PermissionState)
            };
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

            if !perms.check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
                let err = v8::Exception::error(_scope, err_str);
                _scope.throw_exception(err);
                return;
            }

            // See the plain-blocking-command comment above __execAsync's
            // native fn — same reasoning, no block_in_place needed.
            let result = (|| {
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
            })();

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
                    let err = v8::Exception::error(_scope, err_str);
                    _scope.throw_exception(err);
                }
            }
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnWithInput").unwrap().into(),
        spawn_with_input_fn.into(),
    );

    let spawn_sync_with_stdin_fn = v8::Function::builder(
        |_scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let perms = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const PermissionState)
            };
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

            if !perms.check(&Capability::SpawnProcess) {
                let err_str = V8String::new(
                    _scope,
                    "Process spawn denied. Run with --allow-child-process",
                )
                .unwrap();
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
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnSyncWithStdin").unwrap().into(),
        spawn_sync_with_stdin_fn.into(),
    );

    // ── __spawnCreate(cmd, args) → child_id ──────────────────────────────────
    let spawn_create_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let perms = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const PermissionState)
            };
            if !perms.check(&Capability::SpawnProcess) {
                let e = V8String::new(scope, "Process spawn denied").unwrap();
                scope.throw_exception(e.into());
                return;
            }
            let cmd = args.get(0).to_rust_string_lossy(scope);
            let args_arr = args.get(1);
            let arg_vec: Vec<String> = if args_arr.is_array() {
                let arr = v8::Local::<v8::Array>::try_from(args_arr).unwrap();
                (0..arr.length())
                    .filter_map(|i| {
                        arr.get_index(scope, i)
                            .map(|v| v.to_rust_string_lossy(scope))
                    })
                    .collect()
            } else {
                vec![]
            };

            let child_result = std::process::Command::new(&cmd)
                .args(&arg_vec)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit())
                .spawn();

            match child_result {
                Ok(mut child) => {
                    let stdin = child.stdin.take().unwrap();
                    let stdout = child.stdout.take().unwrap();
                    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
                    let done = Arc::new(AtomicBool::new(false));
                    let buf2 = buf.clone();
                    let done2 = done.clone();
                    std::thread::spawn(move || {
                        use std::io::Read;
                        let mut reader = stdout;
                        let mut tmp = [0u8; 4096];
                        loop {
                            match reader.read(&mut tmp) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => buf2.lock().unwrap().extend_from_slice(&tmp[..n]),
                            }
                        }
                        done2.store(true, Ordering::SeqCst);
                    });
                    let id = NEXT_CHILD_ID.fetch_add(1, Ordering::SeqCst);
                    child_table().lock().unwrap().insert(
                        id,
                        StreamChild {
                            stdin: Some(stdin),
                            stdout_buf: buf,
                            done,
                            _child: child,
                        },
                    );
                    rv.set(v8::Integer::new_from_unsigned(scope, id).into());
                }
                Err(e) => {
                    let msg = V8String::new(scope, &format!("spawn error: {}", e)).unwrap();
                    scope.throw_exception(msg.into());
                }
            }
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnCreate").unwrap().into(),
        spawn_create_fn.into(),
    );

    // ── __spawnWrite(id, bytes: Uint8Array) ───────────────────────────────────
    let spawn_write_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);
            let bytes = crate::builtins::v8_compat::js_value_to_bytes(scope, args.get(1));
            let mut table = child_table().lock().unwrap();
            if let Some(child) = table.get_mut(&id)
                && let Some(stdin) = child.stdin.as_mut()
            {
                let _ = stdin.write_all(&bytes);
                let _ = stdin.flush();
            }
        },
    )
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnWrite").unwrap().into(),
        spawn_write_fn.into(),
    );

    // ── __spawnEnd(id) — close the child's stdin pipe so it can finish ────────
    let spawn_end_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);
            let mut table = child_table().lock().unwrap();
            if let Some(child) = table.get_mut(&id) {
                // Closing the pipe is what allows `cat` to flush its stdout
                // and exit. Drop the stdin handle to send EOF to the child.
                let _ = child.stdin.take();
            }
        },
    )
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnEnd").unwrap().into(),
        spawn_end_fn.into(),
    );

    // ── __spawnPollOut(id) → Uint8Array of buffered stdout data ──────────────
    let spawn_poll_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);
            let table = child_table().lock().unwrap();
            if let Some(child) = table.get(&id) {
                let mut buf = child.stdout_buf.lock().unwrap();
                if !buf.is_empty() {
                    let data = buf.drain(..).collect::<Vec<u8>>();
                    drop(buf);
                    drop(table);
                    let arr = crate::builtins::v8_compat::uint8array_from_bytes(scope, &data);
                    rv.set(arr.into());
                }
            }
        },
    )
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnPollOut").unwrap().into(),
        spawn_poll_fn.into(),
    );

    // ── __spawnIsDone(id) → bool ──────────────────────────────────────────────
    let spawn_done_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);
            let table = child_table().lock().unwrap();
            let done = table
                .get(&id)
                .map(|c| c.done.load(Ordering::SeqCst))
                .unwrap_or(true);
            rv.set(v8::Boolean::new(scope, done).into());
        },
    )
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnIsDone").unwrap().into(),
        spawn_done_fn.into(),
    );

    // ── __spawnKill(id) ───────────────────────────────────────────────────────
    let spawn_kill_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);
            child_table().lock().unwrap().remove(&id);
        },
    )
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__spawnKill").unwrap().into(),
        spawn_kill_fn.into(),
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
                    Promise.resolve(__execShellAsync(command)).then(function(raw) {
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
                    Promise.resolve(__execAsync(file, args, 0)).then(function(raw) {
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
                    var id = __spawnCreate(command, args);
                    var pollTimer = null;

                    function toBytes(chunk) {
                        if (typeof chunk === 'string') {
                            if (typeof Buffer !== 'undefined') return new Uint8Array(Buffer.from(chunk));
                            var out = [];
                            for (var i = 0; i < chunk.length; i++) out.push(chunk.charCodeAt(i) & 0xff);
                            return out;
                        }
                        return chunk;
                    }
                    function bytesToChunk(data) {
                        if (data == null) return data;
                        if (typeof Buffer !== 'undefined') return Buffer.from(data);
                        try { return new TextDecoder('utf-8', { fatal: false }).decode(data); }
                        catch (e) {
                            var s = '';
                            for (var i = 0; i < data.length; i++) s += String.fromCharCode(data[i]);
                            return s;
                        }
                    }
                    function drainAndFinish() {
                        var data = __spawnPollOut(id);
                        if (data) stdout._dataListeners.forEach(function(fn) { fn(bytesToChunk(data)); });
                        if (__spawnIsDone(id)) {
                            var tail = __spawnPollOut(id);
                            if (tail) stdout._dataListeners.forEach(function(fn) { fn(bytesToChunk(tail)); });
                            stdout._endListeners.forEach(function(fn) { fn(); });
                            cp._closeListeners.forEach(function(fn) { fn(0, null); });
                            __spawnKill(id);
                            return true;
                        }
                        return false;
                    }
                    function schedulePoll() {
                        Promise.resolve().then(function() {
                            if (drainAndFinish()) return;
                            if (typeof setTimeout === 'function') setTimeout(schedulePoll, 0);
                        });
                    }
                    var stdin = {
                        write: function(chunk, cb) {
                            __spawnWrite(id, toBytes(chunk));
                            if (cb) cb(null);
                            return true;
                        },
                        on: function(ev, fn) { return this; },
                        end: function(chunk) {
                            if (chunk !== undefined && chunk !== null) __spawnWrite(id, toBytes(chunk));
                            __spawnEnd(id);
                            schedulePoll();
                            return this;
                        },
                        destroy: function() { __spawnKill(id); },
                        unref: function() {}
                    };
                    var stdout = {
                        _dataListeners: [],
                        _endListeners: [],
                        on: function(ev, fn) {
                            if (ev === 'data') this._dataListeners.push(fn);
                            else if (ev === 'end') this._endListeners.push(fn);
                            return this;
                        },
                        destroy: function() {},
                        unref: function() {}
                    };
                    var stderr = {
                        _listeners: [],
                        on: function(ev, fn) { if (ev === 'data') this._listeners.push(fn); return this; },
                        destroy: function() {},
                        unref: function() {}
                    };
                    var cp = {
                        stdin: stdin,
                        stdout: stdout,
                        stderr: stderr,
                        _closeListeners: [],
                        _errorListeners: [],
                        on: function(ev, fn) {
                            if (ev === 'close' || ev === 'exit') this._closeListeners.push(fn);
                            else if (ev === 'error') this._errorListeners.push(fn);
                            return this;
                        },
                        kill: function() { __spawnKill(id); },
                        ref: function() {},
                        unref: function() {}
                    };

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
