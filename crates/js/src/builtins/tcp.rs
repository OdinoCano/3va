//! Raw TCP and TLS socket backend for the `net` and `tls` Node.js modules.

use crate::builtins::v8_compat::{uint8array_from_bytes, uint8array_to_vec};
use native_tls::TlsStream;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use v8::{ContextScope, Function, HandleScope, PinScope, Script, String as V8String};
use vvva_crypto;
use vvva_permissions::{Capability, PermissionState};

#[allow(clippy::large_enum_variant)]
enum TcpConn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl TcpConn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            TcpConn::Plain(s) => s.read(buf),
            TcpConn::Tls(s) => s.read(buf),
        }
    }
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            TcpConn::Plain(s) => s.write_all(data),
            TcpConn::Tls(s) => s.write_all(data),
        }
    }
    fn shutdown(&mut self) {
        match self {
            TcpConn::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            TcpConn::Tls(s) => {
                let _ = s.shutdown();
            }
        }
    }
}

fn js_err<'s>(scope: &mut PinScope<'s, '_>, msg: &str) -> v8::Local<'s, v8::Value> {
    V8String::new(scope, msg).unwrap().into()
}

fn js_code_err<'s>(
    scope: &mut PinScope<'s, '_>,
    code: &str,
    msg: impl AsRef<str>,
) -> v8::Local<'s, v8::Value> {
    let msg = msg.as_ref();
    let src = format!(
        "(function(){{var e=new Error(\"{}\");e.code=\"{}\";return e;}})()",
        msg, code
    );
    let source = V8String::new(scope, &src).unwrap();
    Script::compile(scope, source, None)
        .and_then(|s| s.run(scope))
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn pq_tls_connect_blocking(
    host: &str,
    port: u16,
) -> std::result::Result<(TlsStream<TcpStream>, String), String> {
    let connector = native_tls::TlsConnector::new().map_err(|e| format!("TLS init: {e}"))?;
    let tcp =
        TcpStream::connect(format!("{host}:{port}")).map_err(|e| format!("ECONNREFUSED: {e}"))?;
    let mut tls = connector
        .connect(host, tcp)
        .map_err(|e| format!("TLS handshake failed: {e}"))?;

    let kp = vvva_crypto::kem::MlKemKeypair::generate();
    let ek_bytes = kp.encapsulation_key_bytes();

    let ek_len = (ek_bytes.len() as u32).to_be_bytes();
    tls.write_all(&ek_len)
        .and_then(|_| tls.write_all(&ek_bytes))
        .map_err(|e| format!("PQ TLS send ek: {e}"))?;

    let mut ct_len_buf = [0u8; 4];
    tls.read_exact(&mut ct_len_buf)
        .map_err(|e| format!("PQ TLS recv ct len: {e}"))?;
    let ct_len = u32::from_be_bytes(ct_len_buf) as usize;
    if ct_len != 1088 {
        return Err(format!("PQ TLS: invalid ciphertext length {ct_len}"));
    }
    let mut ct_bytes = vec![0u8; ct_len];
    tls.read_exact(&mut ct_bytes)
        .map_err(|e| format!("PQ TLS recv ct: {e}"))?;

    let ct = vvva_crypto::MlKemCiphertext::from_bytes(&ct_bytes)
        .map_err(|e| format!("PQ TLS ct decode: {e}"))?;
    let ss = vvva_crypto::decapsulate(&kp.dk, &ct);
    let ss_hex = hex::encode(ss.0);

    tls.get_ref()
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;

    Ok((tls, ss_hex))
}

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static TCP_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn permissions() -> Arc<PermissionState> {
    TCP_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_tcp not called on this thread")
    })
}
static TCP_POOL: std::sync::OnceLock<Arc<Mutex<HashMap<u32, TcpConn>>>> =
    std::sync::OnceLock::new();
fn pool() -> &'static Arc<Mutex<HashMap<u32, TcpConn>>> {
    TCP_POOL.get().unwrap()
}
static TCP_NEXT_ID: std::sync::OnceLock<Arc<Mutex<u32>>> = std::sync::OnceLock::new();
fn next_id() -> &'static Arc<Mutex<u32>> {
    TCP_NEXT_ID.get().unwrap()
}
#[allow(clippy::type_complexity)]
static TCP_LISTENERS: std::sync::OnceLock<Arc<Mutex<HashMap<u32, Arc<std::net::TcpListener>>>>> =
    std::sync::OnceLock::new();
fn listeners() -> &'static Arc<Mutex<HashMap<u32, Arc<std::net::TcpListener>>>> {
    TCP_LISTENERS.get().unwrap()
}
static TCP_NEXT_LISTENER_ID: std::sync::OnceLock<Arc<Mutex<u32>>> = std::sync::OnceLock::new();
fn next_listener_id() -> &'static Arc<Mutex<u32>> {
    TCP_NEXT_LISTENER_ID.get().unwrap()
}

pub fn inject_tcp(
    scope: &mut ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    TCP_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));
    TCP_POOL.set(Arc::new(Mutex::new(HashMap::new()))).ok();
    TCP_NEXT_ID.set(Arc::new(Mutex::new(0))).ok();

    let alloc_id =
        |pool: &Arc<Mutex<HashMap<u32, TcpConn>>>, nid: &Arc<Mutex<u32>>, conn: TcpConn| -> u32 {
            let id = {
                let mut n = nid.lock().unwrap();
                let id = *n;
                *n = n.wrapping_add(1);
                id
            };
            pool.lock().unwrap().insert(id, conn);
            id
        };

    let context = scope.get_current_context();
    let global = context.global(scope);

    {
        let tcp_connect_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let host_arg = args.get(0);
                let host = host_arg.to_rust_string_lossy(_scope);
                let port_arg = args.get(1);
                let port: u16 = port_arg.uint32_value(_scope).unwrap_or(0) as u16;

                if !permissions().check(&Capability::Network(host.clone())) {
                    let err = js_code_err(
                        _scope,
                        "EACCES",
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }

                match TcpStream::connect(format!("{}:{}", host, port)) {
                    Ok(stream) => {
                        if let Err(e) = stream.set_nonblocking(true) {
                            let err = js_err(_scope, &e.to_string());
                            rv.set(err);
                            return;
                        }
                        let id = alloc_id(pool(), next_id(), TcpConn::Plain(stream));
                        rv.set(v8::Integer::new_from_unsigned(_scope, id).into());
                    }
                    Err(e) => {
                        let err = js_code_err(_scope, "ECONNREFUSED", e.to_string());
                        rv.set(err);
                    }
                }
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__tcpConnect").unwrap().into(),
            tcp_connect_fn.into(),
        );
    }

    {
        let tcp_connect_tls_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let host_arg = args.get(0);
                let host = host_arg.to_rust_string_lossy(_scope);
                let port_arg = args.get(1);
                let port: u16 = port_arg.uint32_value(_scope).unwrap_or(0) as u16;

                if !permissions().check(&Capability::Network(host.clone())) {
                    let err = js_code_err(
                        _scope,
                        "EACCES",
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }

                let connector = match native_tls::TlsConnector::new() {
                    Ok(c) => c,
                    Err(e) => {
                        let err = js_err(_scope, &format!("TLS init failed: {}", e));
                        rv.set(err);
                        return;
                    }
                };

                match TcpStream::connect(format!("{}:{}", host, port)) {
                    Ok(tcp) => match connector.connect(&host, tcp) {
                        Ok(tls) => {
                            if let Err(e) = tls.get_ref().set_nonblocking(true) {
                                let err = js_err(_scope, &e.to_string());
                                rv.set(err);
                                return;
                            }
                            let id = alloc_id(pool(), next_id(), TcpConn::Tls(tls));
                            rv.set(v8::Integer::new_from_unsigned(_scope, id).into());
                        }
                        Err(e) => {
                            let err = js_code_err(
                                _scope,
                                "ECONNRESET",
                                format!("TLS handshake failed: {}", e),
                            );
                            rv.set(err);
                        }
                    },
                    Err(e) => {
                        let err = js_code_err(_scope, "ECONNREFUSED", e.to_string());
                        rv.set(err);
                    }
                }
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__tcpConnectTls").unwrap().into(),
            tcp_connect_tls_fn.into(),
        );
    }

    {
        let tcp_write_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let id_arg = args.get(0);
                let id = id_arg.uint32_value(_scope).unwrap_or(0);
                let data_arg = args.get(1);
                let data: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg)
                {
                    uint8array_to_vec(_scope, arr)
                } else {
                    vec![]
                };

                let mut guard = pool().lock().unwrap();
                match guard.get_mut(&id) {
                    Some(conn) => {
                        if let Err(e) = conn.write_all(&data) {
                            let err = js_code_err(_scope, "EPIPE", e.to_string());
                            rv.set(err);
                        } else {
                            rv.set(v8::undefined(_scope).into());
                        }
                    }
                    None => {
                        let err = js_err(_scope, &format!("tcpWrite: unknown socket {}", id));
                        rv.set(err);
                    }
                }
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__tcpWrite").unwrap().into(),
            tcp_write_fn.into(),
        );
    }

    {
        let tcp_read_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let id_arg = args.get(0);
                let id = id_arg.uint32_value(_scope).unwrap_or(0);
                let max_bytes_arg = args.get(1);
                let max_bytes: u32 = max_bytes_arg.uint32_value(_scope).unwrap_or(65536);

                let max = (max_bytes as usize).min(65536);
                let mut buf = vec![0u8; max];
                let mut guard = pool().lock().unwrap();

                match guard.get_mut(&id) {
                    Some(conn) => match conn.read(&mut buf) {
                        Ok(0) => {
                            let err = js_code_err(_scope, "EOF", "connection closed");
                            rv.set(err);
                        }
                        Ok(n) => {
                            buf.truncate(n);
                            let result = uint8array_from_bytes(_scope, &buf);
                            rv.set(result.into());
                        }
                        Err(ref e)
                            if e.kind() == io::ErrorKind::WouldBlock
                                || e.kind() == io::ErrorKind::TimedOut =>
                        {
                            let err = js_code_err(_scope, "EAGAIN", "no data available");
                            rv.set(err);
                        }
                        Err(e) => {
                            let err = js_code_err(_scope, "EIO", e.to_string());
                            rv.set(err);
                        }
                    },
                    None => {
                        let err = js_err(_scope, &format!("tcpRead: unknown socket {}", id));
                        rv.set(err);
                    }
                }
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__tcpRead").unwrap().into(),
            tcp_read_fn.into(),
        );
    }

    {
        let tcp_set_timeout_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  _args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                rv.set(v8::undefined(_scope).into());
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__tcpSetTimeout").unwrap().into(),
            tcp_set_timeout_fn.into(),
        );
    }

    {
        let tcp_close_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let id_arg = args.get(0);
                let id = id_arg.uint32_value(_scope).unwrap_or(0);
                if let Some(mut conn) = pool().lock().unwrap().remove(&id) {
                    conn.shutdown();
                }
                rv.set(v8::undefined(_scope).into());
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__tcpClose").unwrap().into(),
            tcp_close_fn.into(),
        );
    }

    TCP_LISTENERS.set(Arc::new(Mutex::new(HashMap::new()))).ok();
    TCP_NEXT_LISTENER_ID.set(Arc::new(Mutex::new(0))).ok();

    {
        let net_listen_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let port_arg = args.get(0);
                let port: u16 = port_arg.uint32_value(_scope).unwrap_or(0) as u16;
                let host_arg = args.get(1);
                let host = host_arg.to_rust_string_lossy(_scope);

                if !permissions().check_bind(&host) {
                    let err = js_code_err(
                        _scope,
                        "EACCES",
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }

                match std::net::TcpListener::bind(format!("{}:{}", host, port)) {
                    Ok(std_l) => {
                        if let Err(e) = std_l.set_nonblocking(true) {
                            let err = js_err(_scope, &e.to_string());
                            rv.set(err);
                            return;
                        }
                        let id = {
                            let mut n = next_listener_id().lock().unwrap();
                            let id = *n;
                            *n = n.wrapping_add(1);
                            id
                        };
                        listeners().lock().unwrap().insert(id, Arc::new(std_l));
                        rv.set(v8::Integer::new_from_unsigned(_scope, id).into());
                    }
                    Err(e) => {
                        let err = js_code_err(_scope, "EADDRINUSE", e.to_string());
                        rv.set(err);
                    }
                }
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__netListen").unwrap().into(),
            net_listen_fn.into(),
        );
    }

    {
        let net_accept_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let server_id_arg = args.get(0);
                let server_id = server_id_arg.uint32_value(_scope).unwrap_or(0);

                let listener = {
                    let g = listeners().lock().unwrap();
                    g.get(&server_id).cloned()
                };
                let listener = match listener {
                    Some(l) => l,
                    None => {
                        let err_str = V8String::new(_scope, "unknown server id").unwrap();
                        rv.set(err_str.into());
                        return;
                    }
                };

                // The listener is nonblocking (set at bind time in __netListen), so
                // this is a single non-blocking accept attempt — same polling model
                // as __tcpRead: WouldBlock maps to an EAGAIN-coded error the JS side
                // retries on a timer instead of a call that blocks the whole engine.
                match listener.accept() {
                    Ok((std_stream, _addr)) => {
                        if let Err(e) = std_stream.set_nonblocking(true) {
                            let err = js_err(_scope, &e.to_string());
                            rv.set(err);
                            return;
                        }
                        let conn_id = {
                            let mut n = next_id().lock().unwrap();
                            let id = *n;
                            *n = n.wrapping_add(1);
                            id
                        };
                        pool()
                            .lock()
                            .unwrap()
                            .insert(conn_id, TcpConn::Plain(std_stream));
                        rv.set(v8::Integer::new_from_unsigned(_scope, conn_id).into());
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        let err = js_code_err(_scope, "EAGAIN", "no pending connection");
                        rv.set(err);
                    }
                    Err(e) => {
                        let err = js_code_err(_scope, "EIO", e.to_string());
                        rv.set(err);
                    }
                }
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__netAcceptAsync").unwrap().into(),
            net_accept_fn.into(),
        );
    }

    {
        let net_close_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let server_id_arg = args.get(0);
                let server_id = server_id_arg.uint32_value(_scope).unwrap_or(0);
                listeners().lock().unwrap().remove(&server_id);
                rv.set(v8::undefined(_scope).into());
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__netClose").unwrap().into(),
            net_close_fn.into(),
        );
    }

    {
        let pq_tls_connect_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let host_arg = args.get(0);
                let host = host_arg.to_rust_string_lossy(_scope);
                let port_arg = args.get(1);
                let port: u16 = port_arg.uint32_value(_scope).unwrap_or(0) as u16;

                if !permissions().check(&Capability::Network(host.clone())) {
                    let err = js_code_err(
                        _scope,
                        "EACCES",
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }

                let result = tokio::task::block_in_place(|| pq_tls_connect_blocking(&host, port));

                match result {
                    Ok((tls, ss_hex)) => {
                        let conn_id = alloc_id(pool(), next_id(), TcpConn::Tls(tls));
                        let json =
                            serde_json::json!({ "connId": conn_id, "pqSharedSecret": ss_hex })
                                .to_string();
                        let result_str = V8String::new(_scope, &json).unwrap();
                        rv.set(result_str.into());
                    }
                    Err(e) => {
                        let err_str = V8String::new(_scope, &e).unwrap();
                        rv.set(err_str.into());
                    }
                }
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__pqTlsConnect").unwrap().into(),
            pq_tls_connect_fn.into(),
        );
    }

    Ok(())
}
