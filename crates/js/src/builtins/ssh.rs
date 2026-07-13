//! SSH/SFTP client built-in module
//!
//! Provides: `require('ssh2')` with `Client` class, backed by real SSH via
//! `russh` (client protocol, password auth, exec channels) and `russh-sftp`
//! (SFTP subsystem: readdir/open/mkdir/rmdir/unlink/rename/stat/read/write).
//!
//! Native functions:
//! - `__sshCreate()` -> id
//! - `__sshConnect(id, host, port, username, password)` -> Promise<envelope>
//! - `__sshExec(id, command)` -> Promise<envelope {stdout, stderr, code}>
//! - `__sshSftp(id)` -> Promise<envelope {sftpId}>
//! - `__sftpReaddir(id, path)` -> Promise<envelope [entries]>
//! - `__sftpReadFile(id, path)` -> Promise<envelope [bytes]>
//! - `__sftpWriteFile(id, path, bytes)` -> Promise<envelope>
//! - `__sftpMkdir(id, path)` / `__sftpRmdir` / `__sftpUnlink` -> Promise<envelope>
//! - `__sftpRename(id, oldPath, newPath)` -> Promise<envelope>
//! - `__sftpStat(id, path)` -> Promise<envelope {size, mtime, mode}>
//! - `__sshClose(id)`

use russh::ChannelMsg;
use russh::client::{self, Handle};
use russh_sftp::client::SftpSession;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

type SshId = u32;
type SftpId = u32;

struct SshHandler;
impl client::Handler for SshHandler {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        Ok(true)
    }
}

struct SshConn {
    handle: Handle<SshHandler>,
    // russh spawns a background task that drives the connection (framing,
    // keepalives, channel dispatch) onto whatever runtime `client::connect`
    // ran on. Every op used to build its OWN throwaway `Runtime::new()` and
    // drop it when done — which killed that driver task the moment connect()
    // returned, so every later exec/sftp call on the same connection failed
    // with a channel send error (the driver task was gone). Keeping the
    // connect-time runtime alive for the connection's whole lifetime, and
    // reusing it for every later op, is what actually keeps the session
    // alive between calls.
    runtime: Arc<tokio::runtime::Runtime>,
}

struct SftpConn {
    sftp: SftpSession,
    // Same reasoning as SshConn::runtime — an SFTP session's stream is a
    // channel over the same SSH connection, driven by that connection's
    // background task.
    runtime: Arc<tokio::runtime::Runtime>,
}

static SSH_REGISTRY: OnceLock<Mutex<HashMap<SshId, Arc<SshConn>>>> = OnceLock::new();
static SFTP_REGISTRY: OnceLock<Mutex<HashMap<SftpId, Arc<SftpConn>>>> = OnceLock::new();

fn ssh_registry() -> &'static Mutex<HashMap<SshId, Arc<SshConn>>> {
    SSH_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sftp_registry() -> &'static Mutex<HashMap<SftpId, Arc<SftpConn>>> {
    SFTP_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_ssh_id() -> SshId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// Every __ssh*/__sftp* operation below spawns a background OS thread with
// its own throwaway tokio runtime to do real blocking network I/O (SSH
// connect, exec, SFTP calls) — necessary since none of this is safe to
// await on the V8 callback thread. Each op gets an id here, the spawned
// thread stores its JSON envelope result keyed by that id when done, and
// `__sshOpPoll` (a plain, non-blocking native function) drains it. This
// used to be missing entirely: the native functions returned `undefined`
// while the spawned thread's result went nowhere but an eprintln!, so
// every `.then()` on them threw "Cannot read properties of undefined
// (reading 'then')" instead of ever settling.
static SSH_OPS: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
fn ssh_ops() -> &'static Mutex<HashMap<u32, String>> {
    SSH_OPS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn next_op_id() -> u32 {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn next_sftp_id() -> SftpId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn get_ssh(id: SshId) -> Option<Arc<SshConn>> {
    ssh_registry().lock().unwrap().get(&id).cloned()
}

fn get_sftp(id: SftpId) -> Option<Arc<SftpConn>> {
    sftp_registry().lock().unwrap().get(&id).cloned()
}

fn ok_envelope(data: serde_json::Value) -> String {
    json!({"ok": true, "data": data}).to_string()
}

fn err_envelope(code: &str, message: impl std::fmt::Display) -> String {
    json!({"ok": false, "code": code, "message": message.to_string()}).to_string()
}

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static INJECT_SSH_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn permissions() -> Arc<PermissionState> {
    INJECT_SSH_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_ssh not called on this thread")
    })
}

pub fn inject_ssh(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    permissions_param: Arc<PermissionState>,
) {
    let context = scope.get_current_context();
    let global = context.global(scope);
    INJECT_SSH_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));

    let create_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              _args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = next_ssh_id();
            rv.set(v8::Number::new(_scope, id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshCreate").unwrap().into(),
        create_fn.into(),
    );

    let connect_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SshId;
            let host = args.get(1).to_rust_string_lossy(_scope);
            let port = args.get(2).uint32_value(_scope).unwrap_or(22) as u16;
            let username = args.get(3).to_rust_string_lossy(_scope);
            let password = args.get(4).to_rust_string_lossy(_scope);
            // Read on this (callback) thread, where the thread-local is
            // actually populated, and move the clone into the spawned
            // thread — permissions() itself would panic if called from
            // there (see PermissionState's doc comment above).
            let perms_for_thread = permissions();
            let op_id = next_op_id();

            std::thread::spawn(move || {
                // Kept alive for the connection's lifetime via SshConn — see
                // its doc comment. Not dropped at the end of this thread.
                let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
                let rt_for_conn = rt.clone();
                let result: String = rt.block_on(async {
                    if !perms_for_thread.check(&Capability::Network(host.clone())) {
                        return err_envelope(
                            "EACCES",
                            format!("Network access denied. Run with --allow-net={}", host),
                        );
                    }

                    let config = Arc::new(client::Config::default());
                    let mut handle =
                        match client::connect(config, (&host[..], port), SshHandler).await {
                            Ok(h) => h,
                            Err(e) => return err_envelope("ECONNREFUSED", e),
                        };

                    match handle.authenticate_password(&username, &password).await {
                        Ok(auth) if auth.success() => {
                            ssh_registry().lock().unwrap().insert(
                                id,
                                Arc::new(SshConn {
                                    handle,
                                    runtime: rt_for_conn,
                                }),
                            );
                            ok_envelope(serde_json::Value::Null)
                        }
                        Ok(_) => err_envelope("EAUTH", "authentication failed"),
                        Err(e) => err_envelope("EAUTH", e),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshConnect").unwrap().into(),
        connect_fn.into(),
    );

    let exec_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SshId;
            let command = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_ssh(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SSH ID"));
                        return;
                    }
                };
                // Reuse the connection's own runtime — see SshConn::runtime's
                // doc comment; a fresh Runtime::new() here would drop out
                // from under the connection's background driver task on the
                // *previous* connect() call, breaking every op after the
                // first.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    let mut channel = match conn.handle.channel_open_session().await {
                        Ok(ch) => ch,
                        Err(e) => {
                            return err_envelope(
                                "EIO",
                                format!("channel_open_session failed: {}", e),
                            );
                        }
                    };

                    if let Err(e) = channel.exec(true, command.as_bytes()).await {
                        return err_envelope("EIO", format!("exec failed: {}", e));
                    }

                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    let mut code = None;
                    loop {
                        match channel.wait().await {
                            Some(ChannelMsg::Data { data }) => stdout.extend_from_slice(&data),
                            Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                                stderr.extend_from_slice(&data)
                            }
                            Some(ChannelMsg::ExitStatus { exit_status }) => {
                                code = Some(exit_status)
                            }
                            Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                            _ => {}
                        }
                    }

                    ok_envelope(json!({
                        "stdout": String::from_utf8_lossy(&stdout),
                        "stderr": String::from_utf8_lossy(&stderr),
                        "code": code.unwrap_or(0),
                    }))
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshExec").unwrap().into(),
        exec_fn.into(),
    );

    let sftp_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SshId;
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_ssh(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SSH ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let rt_for_sftp = rt.clone();
                let result: String = rt.block_on(async move {
                    let channel = match conn.handle.channel_open_session().await {
                        Ok(ch) => ch,
                        Err(e) => {
                            return err_envelope(
                                "EIO",
                                format!("channel_open_session failed: {}", e),
                            );
                        }
                    };
                    if let Err(e) = channel.request_subsystem(true, "sftp").await {
                        return err_envelope(
                            "EIO",
                            format!("sftp subsystem request failed: {}", e),
                        );
                    }
                    let sftp = match SftpSession::new(channel.into_stream()).await {
                        Ok(s) => s,
                        Err(e) => {
                            return err_envelope("EIO", format!("sftp session failed: {}", e));
                        }
                    };

                    let sftp_id = next_sftp_id();
                    sftp_registry().lock().unwrap().insert(
                        sftp_id,
                        Arc::new(SftpConn {
                            sftp,
                            runtime: rt_for_sftp,
                        }),
                    );
                    ok_envelope(json!({ "sftpId": sftp_id }))
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshSftp").unwrap().into(),
        sftp_fn.into(),
    );

    let readdir_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.read_dir(&path).await {
                        Ok(rd) => {
                            let entries: Vec<serde_json::Value> = rd
                                .map(|entry| {
                                    let meta = entry.metadata();
                                    json!({
                                        "filename": entry.file_name(),
                                        "longname": entry.file_name(),
                                        "attrs": {
                                            "size": meta.len(),
                                            "mtime": meta.mtime.unwrap_or(0),
                                            "atime": meta.atime.unwrap_or(0),
                                            "mode": meta.permissions.unwrap_or(0),
                                        }
                                    })
                                })
                                .collect();
                            ok_envelope(serde_json::Value::Array(entries))
                        }
                        Err(e) => err_envelope("EIO", format!("readdir failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpReaddir").unwrap().into(),
        readdir_fn.into(),
    );

    let read_file_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.read(&path).await {
                        Ok(bytes) => ok_envelope(json!(bytes)),
                        Err(e) => err_envelope("EIO", format!("read failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpReadFile").unwrap().into(),
        read_file_fn.into(),
    );

    let write_file_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let data = {
                let maybe_uint8 = v8::Local::<v8::Uint8Array>::try_from(args.get(2)).ok();
                if let Some(uint8) = maybe_uint8 {
                    let len = uint8.byte_length();
                    let mut data = vec![0u8; len];
                    uint8.copy_contents(&mut data);
                    data
                } else {
                    vec![]
                }
            };

            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.write(&path, &data).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("write failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpWriteFile").unwrap().into(),
        write_file_fn.into(),
    );

    let mkdir_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.create_dir(&path).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("mkdir failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpMkdir").unwrap().into(),
        mkdir_fn.into(),
    );

    let rmdir_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.remove_dir(&path).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("rmdir failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpRmdir").unwrap().into(),
        rmdir_fn.into(),
    );

    let unlink_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.remove_file(&path).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("unlink failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpUnlink").unwrap().into(),
        unlink_fn.into(),
    );

    let rename_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let old_path = args.get(1).to_rust_string_lossy(_scope);
            let new_path = args.get(2).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.rename(&old_path, &new_path).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("rename failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpRename").unwrap().into(),
        rename_fn.into(),
    );

    let stat_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.metadata(&path).await {
                        Ok(attrs) => ok_envelope(json!({
                            "size": attrs.len(),
                            "mtime": attrs.mtime.unwrap_or(0),
                            "mode": attrs.permissions.unwrap_or(0),
                        })),
                        Err(e) => err_envelope("EIO", format!("stat failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpStat").unwrap().into(),
        stat_fn.into(),
    );

    let op_poll_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let op_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            match ssh_ops().lock().unwrap().remove(&op_id) {
                Some(json) => rv.set(v8::String::new(_scope, &json).unwrap().into()),
                None => rv.set(v8::null(_scope).into()),
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshOpPoll").unwrap().into(),
        op_poll_fn.into(),
    );

    let ssh_close_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SshId;
            ssh_registry().lock().unwrap().remove(&id);
            rv.set(v8::Boolean::new(_scope, true).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshClose").unwrap().into(),
        ssh_close_fn.into(),
    );

    let js_code = r#"
    (function() {
        // Every __ssh*/__sftp* native function below is synchronous: it
        // starts a background thread and returns an op id immediately,
        // rather than a Promise (a native function cannot hand back a real
        // V8 Promise here). This polls __sshOpPoll(opId) until the
        // background thread's result is ready, wrapped as a Promise so the
        // call sites read the same as before.
        function _pollOp(startFn, args) {
            var opId = startFn.apply(null, args);
            return new Promise(function(resolve) {
                (function check() {
                    var r = __sshOpPoll(opId);
                    if (r === null || r === undefined) { setTimeout(check, 5); return; }
                    resolve(r);
                })();
            });
        }

        function _unwrap(json) {
            var env = JSON.parse(json);
            if (env.ok) return { error: null, data: env.data };
            var err = new Error(env.message);
            err.code = env.code;
            return { error: err, data: null };
        }

        function Client(options) {
            this._id = null;
            this._connected = false;
            this._handlers = {};
            this._opts = options || {};
        }

        Client.prototype.connect = function(options) {
            var self = this;
            options = options || {};
            var host = options.host || 'localhost';
            var port = options.port || 22;
            var username = options.username || 'root';
            var password = options.password || '';

            this._id = __sshCreate();
            _pollOp(__sshConnect, [this._id, host, port, username, password]).then(function(json) {
                var r = _unwrap(json);
                if (r.error) { self.emit('error', r.error); return; }
                self._connected = true;
                self.emit('ready');
            }).catch(function(err) { self.emit('error', err); });

            return this;
        };

        Client.prototype.exec = function(command, callback) {
            var self = this;
            if (!this._connected) {
                var err = Object.assign(new Error('Not connected'), { code: 'ENOTCONN' });
                if (callback) callback(err, null);
                return this;
            }
            _pollOp(__sshExec, [this._id, command]).then(function(json) {
                var r = _unwrap(json);
                if (r.error) { if (callback) callback(r.error, null); return; }
                var ch = new (require('events').EventEmitter)();
                ch.stdout = new (require('events').EventEmitter)();
                ch.stderr = new (require('events').EventEmitter)();
                if (callback) callback(null, ch);
                setTimeout(function() {
                    ch.stdout.emit('data', Buffer.from(r.data.stdout));
                    if (r.data.stderr) ch.stderr.emit('data', Buffer.from(r.data.stderr));
                    ch.emit('close', r.data.code);
                    ch.emit('exit', r.data.code);
                }, 0);
            }).catch(function(err) { if (callback) callback(err, null); });
            return this;
        };

        Client.prototype.shell = function(options, callback) {
            var sh = { on: function() { return this; }, stdin: { write: function() { return this; } } };
            if (typeof options === 'function') options(null, sh);
            else if (callback) callback(null, sh);
            return sh;
        };

        Client.prototype.sftp = function(callback) {
            var self = this;
            if (!this._connected) {
                var err = Object.assign(new Error('Not connected'), { code: 'ENOTCONN' });
                if (callback) callback(err, null);
                return;
            }
            _pollOp(__sshSftp, [this._id]).then(function(json) {
                var r = _unwrap(json);
                if (r.error) { if (callback) callback(r.error, null); return; }
                if (callback) callback(null, new SftpWrapper(r.data.sftpId));
            }).catch(function(err) { if (callback) callback(err, null); });
        };

        Client.prototype.end = function() {
            if (this._id !== null) {
                __sshClose(this._id);
                this._connected = false;
                this._id = null;
            }
        };

        Client.prototype.disconnect = Client.prototype.end;

        Client.prototype.on = Client.prototype.addListener = function(event, listener) {
            this._handlers[event] = this._handlers[event] || [];
            this._handlers[event].push(listener);
            return this;
        };

        Client.prototype.off = Client.prototype.removeListener = function(event, listener) {
            if (this._handlers[event] && listener) {
                var idx = this._handlers[event].indexOf(listener);
                if (idx >= 0) this._handlers[event].splice(idx, 1);
            }
            return this;
        };

        Client.prototype.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            (this._handlers[event] || []).forEach(function(h) { h.apply(null, args); });
        };

        Client.prototype.readFile = function(path, options, callback) {
            if (typeof options === 'function') { callback = options; }
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.readFile(path, callback);
            });
        };

        Client.prototype.writeFile = function(path, data, options, callback) {
            if (typeof options === 'function') { callback = options; }
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.writeFile(path, data, callback);
            });
        };

        Client.prototype.stat = function(path, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err, null); return; }
                sftp.stat(path, callback);
            });
        };

        Client.prototype.mkdir = function(path, attrs, callback) {
            if (typeof attrs === 'function') { callback = attrs; }
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.mkdir(path, callback);
            });
        };

        Client.prototype.rmdir = function(path, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.rmdir(path, callback);
            });
        };

        Client.prototype.unlink = function(path, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.unlink(path, callback);
            });
        };

        Client.prototype.rename = function(from, to, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.rename(from, to, callback);
            });
        };

        Client.prototype.readdir = function(path, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err, []); return; }
                sftp.readdir(path, callback);
            });
        };

        function SftpWrapper(sftpId) {
            this._sftpId = sftpId;
        }

        SftpWrapper.prototype.readdir = function(path, callback) {
            _pollOp(__sftpReaddir, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error, r.error ? [] : r.data);
            }).catch(function(err) { if (callback) callback(err, []); });
        };

        SftpWrapper.prototype.readFile = function(path, options, callback) {
            if (typeof options === 'function') { callback = options; }
            _pollOp(__sftpReadFile, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (r.error) { if (callback) callback(r.error, null); return; }
                if (callback) callback(null, Buffer.from(r.data));
            }).catch(function(err) { if (callback) callback(err, null); });
        };

        SftpWrapper.prototype.writeFile = function(path, data, options, callback) {
            if (typeof options === 'function') { callback = options; }
            var bytes = typeof data === 'string'
                ? Array.from(new TextEncoder().encode(data))
                : Array.from(data instanceof Uint8Array ? data : new Uint8Array(data));
            _pollOp(__sftpWriteFile, [this._sftpId, path, bytes]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.mkdir = function(path, callback) {
            _pollOp(__sftpMkdir, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.rmdir = function(path, callback) {
            _pollOp(__sftpRmdir, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.unlink = function(path, callback) {
            _pollOp(__sftpUnlink, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.rename = function(from, to, callback) {
            _pollOp(__sftpRename, [this._sftpId, from, to]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.stat = function(path, callback) {
            _pollOp(__sftpStat, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error, r.error ? null : r.data);
            }).catch(function(err) { if (callback) callback(err, null); });
        };

        SftpWrapper.prototype.lstat = SftpWrapper.prototype.stat;

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['ssh2'] = { Client: Client };
        globalThis.__requireCache['node:ssh2'] = { Client: Client };
        globalThis.ssh = { Client: Client };
    })();
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }
}
