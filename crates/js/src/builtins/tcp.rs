// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Raw TCP and TLS socket backend for the `net` and `tls` Node.js modules.

use super::tls::TlsStream;
use crate::builtins::v8_compat::{uint8array_from_bytes, uint8array_to_vec};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use v8::{ContextScope, Function, HandleScope, PinScope, Script, String as V8String};
use vvva_permissions::{Capability, PermissionState};

#[allow(clippy::large_enum_variant)]
enum TcpConn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
    /// Real in-handshake hybrid PQ TLS (RFC 10024 X25519MLKEM768), see
    /// `pq_tls_connect_blocking` below — distinct from `Tls` because rustls'
    /// `ClientConnection` is a different concrete type than native-tls' stream.
    PqTls(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
}

impl TcpConn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            TcpConn::Plain(s) => s.read(buf),
            TcpConn::Tls(s) => s.read(buf),
            TcpConn::PqTls(s) => s.read(buf),
        }
    }
    /// One write of as many bytes as the socket takes right now. The sockets
    /// are nonblocking, so a full send buffer is `WouldBlock`, not an error.
    fn write_some(&mut self, data: &[u8]) -> io::Result<usize> {
        match self {
            TcpConn::Plain(s) => s.write(data),
            TcpConn::Tls(s) => s.write(data),
            TcpConn::PqTls(s) => s.write(data),
        }
    }
    fn set_nodelay(&self, on: bool) {
        // Node enables TCP_NODELAY on client sockets by default; without it a
        // client that writes several small messages back to back (e.g. pg's
        // Parse/Bind/Execute/Sync) stalls on the Nagle/delayed-ACK interaction
        // — each write waits for the peer's ~40 ms delayed ACK.
        let sock = match self {
            TcpConn::Plain(s) => Some(s),
            TcpConn::Tls(s) => Some(s.get_ref()),
            TcpConn::PqTls(s) => Some(&s.sock),
        };
        if let Some(s) = sock {
            let _ = s.set_nodelay(on);
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
            TcpConn::PqTls(s) => {
                s.conn.send_close_notify();
                let _ = s.conn.complete_io(&mut s.sock);
                let _ = s.sock.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    /// Close only the write half (`Socket.end()`): the peer sees a FIN (or a
    /// TLS `close_notify`) but the connection stays readable, so a server that
    /// answers *after* the client's end is still heard. `shutdown()` above
    /// closes both directions, which is what `destroy()` uses.
    fn shutdown_write(&mut self) {
        match self {
            TcpConn::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Write);
            }
            TcpConn::Tls(s) => {
                // rustls: send close_notify and flush; the underlying TCP
                // write side is not torn down, so reads keep working.
                let _ = s.shutdown();
            }
            TcpConn::PqTls(s) => {
                s.conn.send_close_notify();
                let _ = s.conn.complete_io(&mut s.sock);
                let _ = s.sock.shutdown(std::net::Shutdown::Write);
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

/// Client config for real in-handshake hybrid PQ-TLS: RFC 10024 X25519MLKEM768,
/// via rustls' aws-lc-rs provider with the `prefer-post-quantum` group ordering
/// (client offers the PQ hybrid group first; a server that doesn't support it
/// still negotiates plain classical X25519 — no separate fallback code needed,
/// that's the point of a *hybrid* group). See
/// docs/10-security/06-pq-tls-hybrid-design.md for the full design rationale.
pub(crate) fn pq_tls_client_config_native() -> std::result::Result<Arc<rustls::ClientConfig>, String>
{
    static CONFIG: std::sync::OnceLock<std::result::Result<Arc<rustls::ClientConfig>, String>> =
        std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            let native = rustls_native_certs::load_native_certs();
            for cert in native.certs {
                roots
                    .add(cert)
                    .map_err(|e| format!("PQ TLS: bad root cert: {e}"))?;
            }
            // Node's own bundle, so HTTPS still works in images without ca-certificates.
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            add_extra_ca_certs(&mut roots);
            pq_tls_client_config_from_roots(roots)
        })
        .clone()
}

/// Append the trust anchors named by `NODE_EXTRA_CA_CERTS`, mirroring Node's
/// behavior for the same variable. The file is a PEM bundle (one or more
/// certificates); a missing or malformed entry is reported and skipped rather
/// than failing the whole TLS setup, since the remaining roots are still
/// usable. Without this, a 3va client cannot verify a private CA that Node
/// trusts, and `fetch()` fails with `UnknownIssuer`.
fn add_extra_ca_certs(roots: &mut rustls::RootCertStore) {
    let Ok(path) = std::env::var("NODE_EXTRA_CA_CERTS") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    let pem = match std::fs::read(&path) {
        Ok(pem) => pem,
        Err(e) => {
            eprintln!("[3va-tls] NODE_EXTRA_CA_CERTS: cannot read {path}: {e}");
            return;
        }
    };
    let mut reader = std::io::BufReader::new(pem.as_slice());
    for cert in rustls_pemfile::certs(&mut reader) {
        match cert {
            Ok(cert) => {
                if let Err(e) = roots.add(cert) {
                    eprintln!("[3va-tls] NODE_EXTRA_CA_CERTS: bad certificate in {path}: {e}");
                }
            }
            Err(e) => eprintln!("[3va-tls] NODE_EXTRA_CA_CERTS: bad PEM in {path}: {e}"),
        }
    }
}

/// Builds a fresh (uncached) config trusting only the given CA PEM — the
/// `tls.pqConnect(host, port, { ca })` path, mirroring Node's `tls` `ca`
/// option, used for self-signed/private-CA endpoints (including the
/// integration test's local `openssl s_server`).
fn pq_tls_client_config_with_ca(
    ca_pem: &str,
) -> std::result::Result<Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    let mut reader = std::io::BufReader::new(ca_pem.as_bytes());
    for cert in rustls_pemfile::certs(&mut reader) {
        let cert = cert.map_err(|e| format!("PQ TLS: bad ca PEM: {e}"))?;
        roots
            .add(cert)
            .map_err(|e| format!("PQ TLS: bad ca cert: {e}"))?;
    }
    pq_tls_client_config_from_roots(roots)
}

pub(crate) fn pq_tls_client_config_from_roots(
    roots: rustls::RootCertStore,
) -> std::result::Result<Arc<rustls::ClientConfig>, String> {
    let provider = super::tls::provider();
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("PQ TLS: protocol versions: {e}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

fn pq_tls_connect_blocking(
    host: &str,
    port: u16,
    ca_pem: Option<&str>,
) -> std::result::Result<
    (
        rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
        String,
        bool,
    ),
    String,
> {
    let config = match ca_pem {
        Some(pem) => pq_tls_client_config_with_ca(pem)?,
        None => pq_tls_client_config_native()?,
    };
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| format!("PQ TLS: invalid server name {host:?}: {e}"))?;
    let conn = rustls::ClientConnection::new(config, server_name)
        .map_err(|e| format!("PQ TLS init: {e}"))?;

    let tcp = permissions()
        .vetted_addrs(host, port)
        .and_then(|a| TcpStream::connect(&a[..]))
        .map_err(|e| format!("ECONNREFUSED: {e}"))?;
    let mut stream = rustls::StreamOwned::new(conn, tcp);

    while stream.conn.is_handshaking() {
        stream
            .conn
            .complete_io(&mut stream.sock)
            .map_err(|e| format!("TLS handshake failed: {e}"))?;
    }

    let group = stream
        .conn
        .negotiated_key_exchange_group()
        .map(|g| g.name())
        .and_then(|n| n.as_str())
        .unwrap_or("unknown")
        .to_string();
    let pq_negotiated = stream
        .conn
        .negotiated_key_exchange_group()
        .map(|g| g.name() == rustls::NamedGroup::X25519MLKEM768)
        .unwrap_or(false);

    stream
        .sock
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;

    Ok((stream, group, pq_negotiated))
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

/// Mirrors `listeners()`'s open-listener count without needing to lock it —
/// `run_event_loop` checks this every iteration, including the hot path
/// while serving real traffic, so it has to be an atomic load, not a mutex
/// lock. Like libuv's active-handle count: an open listener has to keep the
/// loop alive even while idle, since "waiting for the next connection"
/// isn't a pending timer or task.
static TCP_ACTIVE_LISTENERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Listener ids the application `unref()`'d: they no longer count in
/// `TCP_ACTIVE_LISTENERS`, so they do not keep the process alive.
fn unref_listeners() -> &'static Mutex<std::collections::HashSet<u32>> {
    static SET: std::sync::OnceLock<Mutex<std::collections::HashSet<u32>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

pub fn has_active_listeners() -> bool {
    TCP_ACTIVE_LISTENERS.load(std::sync::atomic::Ordering::Relaxed) > 0
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
                let port: u16 = super::port_from_js(port_arg.uint32_value(_scope), 0);

                if !permissions().check(&Capability::Network(vvva_permissions::authority(
                    &host, port,
                ))) {
                    let err = js_code_err(
                        _scope,
                        "EACCES",
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }

                match permissions()
                    .vetted_addrs(&host, port)
                    .and_then(|a| TcpStream::connect(&a[..]))
                {
                    Ok(stream) => {
                        if let Err(e) = stream.set_nonblocking(true) {
                            let err = js_err(_scope, &e.to_string());
                            rv.set(err);
                            return;
                        }
                        let _ = stream.set_nodelay(true);
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
                let port: u16 = super::port_from_js(port_arg.uint32_value(_scope), 0);

                if !permissions().check(&Capability::Network(vvva_permissions::authority(
                    &host, port,
                ))) {
                    let err = js_code_err(
                        _scope,
                        "EACCES",
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }

                // The TLS options the request named (`ca`, `cert`, `key`,
                // `rejectUnauthorized`).
                let opts_json = args.get(2).to_rust_string_lossy(_scope);
                let opts = super::tls::TlsOptions::from_json(&opts_json);
                // Skipping the certificate verification is the TLS version of
                // a plaintext connection: it needs the user's permission
                // (`--allow-insecure`), or a loopback host. Without this, a
                // dependency could turn `rejectUnauthorized: false` into
                // "accept any certificate" for every host.
                if !opts.reject_unauthorized
                    && !vvva_permissions::insecure::insecure_tls_allowed(&host)
                {
                    let err = js_code_err(
                        _scope,
                        "ERR_TLS_VERIFICATION_SKIPPED",
                        vvva_permissions::insecure::insecure_tls_denied_message(&host),
                    );
                    rv.set(err);
                    return;
                }
                let connector = match super::tls::TlsConnector::with_options(Some(&opts)) {
                    Ok(c) => c,
                    Err(e) => {
                        let err = js_err(_scope, &format!("TLS init failed: {}", e));
                        rv.set(err);
                        return;
                    }
                };

                match permissions()
                    .vetted_addrs(&host, port)
                    .and_then(|a| TcpStream::connect(&a[..]))
                {
                    Ok(tcp) => match connector.connect(&host, tcp) {
                        Ok(tls) => {
                            if let Err(e) = tls.get_ref().set_nonblocking(true) {
                                let err = js_err(_scope, &e.to_string());
                                rv.set(err);
                                return;
                            }
                            let _ = tls.get_ref().set_nodelay(true);
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
                        // Write what the socket takes now. When its send buffer
                        // is full, return how many bytes went out (a number)
                        // and let JS retry the rest on a later tick: blocking
                        // here would deadlock a peer that lives in this same
                        // event loop, and `write_all` on a nonblocking socket
                        // failed with EAGAIN ("Resource temporarily
                        // unavailable") on any large write.
                        let mut off = 0usize;
                        let mut failure: Option<std::io::Error> = None;
                        while off < data.len() {
                            match conn.write_some(&data[off..]) {
                                Ok(0) => {
                                    failure = Some(std::io::ErrorKind::WriteZero.into());
                                    break;
                                }
                                Ok(n) => off += n,
                                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                                Err(e) => {
                                    failure = Some(e);
                                    break;
                                }
                            }
                        }
                        if let Some(e) = failure {
                            let err = js_code_err(_scope, "EPIPE", e.to_string());
                            rv.set(err);
                        } else if off == data.len() {
                            rv.set(v8::undefined(_scope).into());
                        } else {
                            rv.set(v8::Integer::new_from_unsigned(_scope, off as u32).into());
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
                        // A peer that reset the connection reads as
                        // ECONNRESET, not a generic EIO: `http` maps it to the
                        // documented "socket hang up"/"aborted" and Node tests
                        // accept ECONNRESET/ECONNABORTED/EPIPE, never EIO.
                        Err(e)
                            if e.kind() == io::ErrorKind::ConnectionReset
                                || e.kind() == io::ErrorKind::ConnectionAborted =>
                        {
                            let err = js_code_err(_scope, "ECONNRESET", "connection reset by peer");
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

        {
            let tcp_set_nodelay_fn = Function::new(
                scope,
                move |_scope: &mut v8::PinScope,
                      args: v8::FunctionCallbackArguments,
                      mut rv: v8::ReturnValue| {
                    let id = args.get(0).uint32_value(_scope).unwrap_or(0);
                    let on = args.get(1).uint32_value(_scope).unwrap_or(0) != 0;
                    let guard = pool().lock().unwrap();
                    if let Some(conn) = guard.get(&id) {
                        conn.set_nodelay(on);
                    }
                    rv.set(v8::undefined(_scope).into());
                },
            )
            .unwrap();
            global.set(
                scope,
                V8String::new(scope, "__tcpSetNoDelay").unwrap().into(),
                tcp_set_nodelay_fn.into(),
            );
        }
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

    {
        // Half-close: flush whatever is queued, then shut down only the write
        // side. The pooled connection stays in the table so reads keep working
        // until the peer closes (or the JS side destroys the socket).
        let tcp_shutdown_write_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let id_arg = args.get(0);
                let id = id_arg.uint32_value(_scope).unwrap_or(0);
                if let Some(conn) = pool().lock().unwrap().get_mut(&id) {
                    conn.shutdown_write();
                }
                rv.set(v8::undefined(_scope).into());
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__tcpShutdownWrite").unwrap().into(),
            tcp_shutdown_write_fn.into(),
        );
    }

    TCP_LISTENERS.set(Arc::new(Mutex::new(HashMap::new()))).ok();
    TCP_NEXT_LISTENER_ID.set(Arc::new(Mutex::new(0))).ok();

    {
        // `__netListenerAddr(id)`: the bound "ip:port" of a listener (the real
        // port after `listen(0)`), or null.
        let net_listener_addr_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let id = args.get(0).uint32_value(_scope).unwrap_or(0);
                let addr = listeners()
                    .lock()
                    .unwrap()
                    .get(&id)
                    .and_then(|l| l.local_addr().ok());
                match addr {
                    Some(a) => rv.set(V8String::new(_scope, &a.to_string()).unwrap().into()),
                    None => rv.set(v8::null(_scope).into()),
                }
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__netListenerAddr").unwrap().into(),
            net_listener_addr_fn.into(),
        );
    }

    {
        let net_listen_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let port_arg = args.get(0);
                let port: u16 = super::port_from_js(port_arg.uint32_value(_scope), 0);
                let host_arg = args.get(1);
                let host = host_arg.to_rust_string_lossy(_scope);

                let Some(host) = permissions().bind_host(&host) else {
                    let err = js_code_err(
                        _scope,
                        "EACCES",
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                };

                let reuse_port = args.get(2).boolean_value(_scope);
                match crate::builtins::http_server::bind_listener_opts(
                    &format!("{}:{}", host, port),
                    reuse_port,
                ) {
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
                        TCP_ACTIVE_LISTENERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        // `__netRef(id, ref)`: server.ref() / server.unref().
        let net_ref_fn = Function::new(
            scope,
            move |_scope: &mut v8::PinScope,
                  args: v8::FunctionCallbackArguments,
                  mut rv: v8::ReturnValue| {
                let id = args.get(0).uint32_value(_scope).unwrap_or(0);
                let refd = args.get(1).boolean_value(_scope);
                if listeners().lock().unwrap().contains_key(&id) {
                    let mut set = unref_listeners().lock().unwrap();
                    if !refd {
                        if set.insert(id) {
                            TCP_ACTIVE_LISTENERS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        }
                    } else if set.remove(&id) {
                        TCP_ACTIVE_LISTENERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                rv.set(v8::undefined(_scope).into());
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__netRef").unwrap().into(),
            net_ref_fn.into(),
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
                if listeners().lock().unwrap().remove(&server_id).is_some()
                    && !unref_listeners().lock().unwrap().remove(&server_id)
                {
                    TCP_ACTIVE_LISTENERS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
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
                let port: u16 = super::port_from_js(port_arg.uint32_value(_scope), 0);
                let ca_arg = args.get(2);
                let ca_pem = if ca_arg.is_string() {
                    Some(ca_arg.to_rust_string_lossy(_scope))
                } else {
                    None
                };

                if !permissions().check(&Capability::Network(vvva_permissions::authority(
                    &host, port,
                ))) {
                    let err = js_code_err(
                        _scope,
                        "EACCES",
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }

                let result = tokio::task::block_in_place(|| {
                    pq_tls_connect_blocking(&host, port, ca_pem.as_deref())
                });

                match result {
                    Ok((stream, group, pq_negotiated)) => {
                        let _ = stream.sock.set_nodelay(true);
                        let conn_id = alloc_id(pool(), next_id(), TcpConn::PqTls(stream));
                        let json = serde_json::json!({
                            "connId": conn_id,
                            "group": group,
                            "pqNegotiated": pq_negotiated,
                        })
                        .to_string();
                        let result_str = V8String::new(_scope, &json).unwrap();
                        rv.set(result_str.into());
                    }
                    Err(e) => {
                        let err = js_code_err(_scope, "ECONNRESET", e);
                        rv.set(err);
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
