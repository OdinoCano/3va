//! UDP datagram socket backend for the `dgram` Node.js module.
//!
//! Behavioral parity contract (verified against Node.js v24 in
//! `scripts/compat-dgram-dns.sh`):
//! - sockets are created lazily — `createSocket()` does not touch the OS until
//!   `bind()` / `send()` / `connect()`, so `address()` before `bind` throws
//!   (`EBADF`) and `bind()` after an auto-binding `send()` throws
//!   `ERR_SOCKET_ALREADY_BOUND`, exactly like Node.
//! - `send(cb)` calls back with `(err, bytes)`.
//! - `'message'` rinfo carries `{address, family, port, size}` with the correct
//!   `family` for both `udp4` and `udp6` sockets.
//! - native socket errors are mapped to Node errno codes (`EADDRINUSE`,
//!   `EACCES`, `ENOTFOUND`, …) instead of a raw OS string.
//! - real `connect`/`disconnect`/`remoteAddress()` plus the socket-option API
//!   (`setTTL`, `setBroadcast`, multicast group membership, `setRecvBufferSize`
//!   …) via `socket2`.

use base64::Engine;
use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex, OnceLock};
use v8::{ContextScope, Function, HandleScope, PinScope, Script, String as V8String};

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static DGRAM_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn permissions() -> Arc<PermissionState> {
    DGRAM_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_dgram not called on this thread")
    })
}
use vvva_permissions::{Capability, PermissionState};

type SocketId = u32;

struct UdpState {
    socket: Arc<UdpSocket>,
    recv_queue: Arc<Mutex<VecDeque<UdpMessage>>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
}

struct UdpMessage {
    data: Vec<u8>,
    address: std::string::String,
    family: &'static str,
    port: u16,
}

static UDP_REGISTRY: OnceLock<Mutex<HashMap<SocketId, UdpState>>> = OnceLock::new();

fn udp_registry() -> &'static Mutex<HashMap<SocketId, UdpState>> {
    UDP_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_udp_id() -> SocketId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn spawn_recv_loop(
    sock: Arc<UdpSocket>,
    queue: Arc<Mutex<VecDeque<UdpMessage>>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
) {
    std::thread::Builder::new()
        .name("3va-udp-recv".into())
        .spawn(move || {
            let mut buf = [0u8; 65_536];
            loop {
                if closed.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(500)));
                match sock.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        let (address, family) = match src {
                            SocketAddr::V4(a) => (a.ip().to_string(), "IPv4"),
                            SocketAddr::V6(a) => (a.ip().to_string(), "IPv6"),
                        };
                        queue.lock().unwrap().push_back(UdpMessage {
                            data: buf[..n].to_vec(),
                            address,
                            family,
                            port: src.port(),
                        });
                    }
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::TimedOut
                            || e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }
        })
        .ok();
}

// ── Node errno-code mapping ──────────────────────────────────────────────────

/// Map a `std::io::Error` onto a Node.js errno string (`EADDRINUSE`,
/// `EACCES`, `ENOTFOUND`, …). getaddrinfo failures don't set errno, so the
/// "Name or service not known" message is matched explicitly, like libuv does.
fn node_code_for_io_error(e: &std::io::Error) -> std::string::String {
    use std::io::ErrorKind::*;
    match e.kind() {
        AddrInUse => "EADDRINUSE".to_string(),
        AddrNotAvailable => "EADDRNOTAVAIL".to_string(),
        PermissionDenied => "EACCES".to_string(),
        ConnectionRefused => "ECONNREFUSED".to_string(),
        ConnectionAborted => "ECONNABORTED".to_string(),
        ConnectionReset => "ECONNRESET".to_string(),
        NotConnected => "ENOTCONN".to_string(),
        NetworkUnreachable => "ENETUNREACH".to_string(),
        HostUnreachable => "EHOSTUNREACH".to_string(),
        InvalidInput => "EINVAL".to_string(),
        WouldBlock => "EAGAIN".to_string(),
        TimedOut => "ETIMEDOUT".to_string(),
        _ => {
            let msg = e.to_string();
            if msg.contains("Name or service not known")
                || msg.contains("nodename nor servname provided")
            {
                "ENOTFOUND".to_string()
            } else if let Some(errno) = e.raw_os_error() {
                errno_to_code(errno)
            } else {
                "EUNKNOWN".to_string()
            }
        }
    }
}

fn errno_to_code(errno: i32) -> std::string::String {
    let code = match errno {
        libc::EPERM => "EPERM",
        libc::ENOENT => "ENOENT",
        libc::EIO => "EIO",
        libc::EBADF => "EBADF",
        libc::EAGAIN => "EAGAIN",
        libc::ENOMEM => "ENOMEM",
        libc::EACCES => "EACCES",
        libc::EEXIST => "EEXIST",
        libc::EINVAL => "EINVAL",
        libc::ENFILE => "ENFILE",
        libc::EMFILE => "EMFILE",
        libc::ENOTTY => "ENOTTY",
        libc::EPIPE => "EPIPE",
        libc::ENOSYS => "ENOSYS",
        libc::EINTR => "EINTR",
        libc::ENOTDIR => "ENOTDIR",
        libc::EISDIR => "EISDIR",
        libc::ENOTEMPTY => "ENOTEMPTY",
        libc::ENOTCONN => "ENOTCONN",
        libc::ENETUNREACH => "ENETUNREACH",
        libc::EHOSTUNREACH => "EHOSTUNREACH",
        libc::ENETDOWN => "ENETDOWN",
        libc::ECONNREFUSED => "ECONNREFUSED",
        libc::ECONNRESET => "ECONNRESET",
        libc::ECONNABORTED => "ECONNABORTED",
        libc::EADDRINUSE => "EADDRINUSE",
        libc::EADDRNOTAVAIL => "EADDRNOTAVAIL",
        libc::ENOTSUP => "ENOTSUP",
        libc::ETIMEDOUT => "ETIMEDOUT",
        libc::EALREADY => "EALREADY",
        libc::EINPROGRESS => "EINPROGRESS",
        libc::EBUSY => "EBUSY",
        _ => return format!("E{errno}"),
    };
    code.to_string()
}

/// Build the error-JSON string the JS side parses (`{"code":"EADDRINUSE"}`).
/// The JS side owns the port/address context, so it only needs the code.
fn err_json<'s>(scope: &mut PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
    let s = format!(r#"{{"code":"{code}"}}"#);
    V8String::new(scope, &s).unwrap().into()
}

pub fn inject_dgram(
    scope: &mut ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    DGRAM_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));
    let context = scope.get_current_context();
    let global = context.global(scope);

    let udp_create_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let socket_type_arg = args.get(0);
            let socket_type = socket_type_arg.to_rust_string_lossy(_scope);

            let id = next_udp_id();
            let bind_addr: &str = if socket_type == "udp6" {
                "[::]:0"
            } else {
                "0.0.0.0:0"
            };

            let sock = match UdpSocket::bind(bind_addr) {
                Ok(s) => Arc::new(s),
                Err(_) => {
                    rv.set(v8::Integer::new_from_unsigned(_scope, 0).into());
                    return;
                }
            };
            let queue = Arc::new(Mutex::new(VecDeque::new()));
            let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            spawn_recv_loop(sock.clone(), queue.clone(), closed.clone());
            udp_registry().lock().unwrap().insert(
                id,
                UdpState {
                    socket: sock,
                    recv_queue: queue,
                    closed,
                },
            );
            rv.set(v8::Integer::new_from_unsigned(_scope, id).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpCreate").unwrap().into(),
        udp_create_fn.into(),
    );

    let udp_bind_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);
            let port_arg = args.get(1);
            let port: u16 = port_arg.uint32_value(_scope).unwrap_or(0) as u16;
            let address_arg = args.get(2);
            let address = address_arg.to_rust_string_lossy(_scope);

            if !permissions().check(&Capability::Network(address.clone())) {
                let result = V8String::new(
                    _scope,
                    "EACCES: permission denied (--allow-net=<host> required)",
                )
                .unwrap();
                rv.set(result.into());
                return;
            }

            let bind_addr = format!("{address}:{port}");
            let reg = udp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                drop(reg);
                let new_sock = match UdpSocket::bind(&bind_addr) {
                    Ok(s) => Arc::new(s),
                    Err(e) => {
                        let code = node_code_for_io_error(&e);
                        let msg = if code == "ENOTFOUND" {
                            format!("getaddrinfo ENOTFOUND {address}")
                        } else {
                            format!("bind {code} {address}:{port}")
                        };
                        let result = V8String::new(
                            _scope,
                            &format!(r#"{{"code":"{code}","message":"{msg}"}}"#),
                        )
                        .unwrap();
                        rv.set(result.into());
                        return;
                    }
                };
                let mut reg = udp_registry().lock().unwrap();
                if let Some(state) = reg.get_mut(&id) {
                    state
                        .closed
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                    let queue = state.recv_queue.clone();
                    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    spawn_recv_loop(new_sock.clone(), queue.clone(), closed.clone());
                    state.socket = new_sock;
                    state.closed = closed;
                }
                rv.set(v8::null(_scope).into());
            } else {
                let result =
                    V8String::new(_scope, &format!("ENOENT: unknown socket id {}", id)).unwrap();
                rv.set(result.into());
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpBind").unwrap().into(),
        udp_bind_fn.into(),
    );

    let udp_send_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);
            let data_b64_arg = args.get(1);
            let data_b64 = data_b64_arg.to_rust_string_lossy(_scope);
            let port_arg = args.get(4);
            let port: u16 = port_arg.uint32_value(_scope).unwrap_or(0) as u16;
            let address_arg = args.get(5);
            let address = address_arg.to_rust_string_lossy(_scope);

            if !permissions().check(&Capability::Network(address.clone())) {
                let result = V8String::new(
                    _scope,
                    "EACCES: permission denied (--allow-net=<host> required)",
                )
                .unwrap();
                rv.set(result.into());
                return;
            }

            let bytes = match base64_decode(&data_b64) {
                Ok(b) => b,
                Err(e) => {
                    let result = V8String::new(_scope, e.as_str()).unwrap();
                    rv.set(result.into());
                    return;
                }
            };

            let dest = format!("{address}:{port}");
            let reg = udp_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                let result = if address.is_empty() {
                    // Connected-mode send: __udpSend is called without a
                    // destination (send(msg, cb) on a connected socket).
                    state.socket.send(&bytes)
                } else {
                    state.socket.send_to(&bytes, &dest)
                };
                if let Err(e) = result {
                    let code = node_code_for_io_error(&e);
                    let msg = if code == "ENOTFOUND" {
                        format!("getaddrinfo ENOTFOUND {address}")
                    } else {
                        format!("sendto {code} {address}:{port}")
                    };
                    let result =
                        V8String::new(_scope, &format!(r#"{{"code":"{code}","message":"{msg}"}}"#))
                            .unwrap();
                    rv.set(result.into());
                    return;
                }
            } else {
                let result =
                    V8String::new(_scope, &format!("ENOENT: unknown socket id {}", id)).unwrap();
                rv.set(result.into());
                return;
            }
            rv.set(v8::null(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpSend").unwrap().into(),
        udp_send_fn.into(),
    );

    let udp_recv_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);

            let reg = udp_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                let msg = state.recv_queue.lock().unwrap().pop_front();
                drop(reg);
                if let Some(m) = msg {
                    let b64 = base64_encode(&m.data);
                    let json = format!(
                        r#"{{"data":"{}","address":"{}","family":"{}","port":{}}}"#,
                        b64, m.address, m.family, m.port
                    );
                    let result = V8String::new(_scope, &json).unwrap();
                    rv.set(result.into());
                } else {
                    rv.set(v8::null(_scope).into());
                }
            } else {
                rv.set(v8::null(_scope).into());
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpRecv").unwrap().into(),
        udp_recv_fn.into(),
    );

    let udp_address_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);

            let reg = udp_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                if let Ok(a) = state.socket.local_addr() {
                    let (addr, family) = match a {
                        SocketAddr::V4(a4) => (a4.ip().to_string(), "IPv4"),
                        SocketAddr::V6(a6) => (a6.ip().to_string(), "IPv6"),
                    };
                    let json = format!(
                        r#"{{"address":"{}","port":{},"family":"{}"}}"#,
                        addr,
                        a.port(),
                        family
                    );
                    let result = V8String::new(_scope, &json).unwrap();
                    rv.set(result.into());
                } else {
                    rv.set(v8::null(_scope).into());
                }
            } else {
                rv.set(v8::null(_scope).into());
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpAddress").unwrap().into(),
        udp_address_fn.into(),
    );

    let udp_close_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);

            if let Some(state) = udp_registry().lock().unwrap().remove(&id) {
                state
                    .closed
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpClose").unwrap().into(),
        udp_close_fn.into(),
    );

    // ── connected-mode UDP ────────────────────────────────────────────────────
    let udp_connect_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);
            let port_arg = args.get(1);
            let port: u16 = port_arg.uint32_value(_scope).unwrap_or(0) as u16;
            let address_arg = args.get(2);
            let address = address_arg.to_rust_string_lossy(_scope);

            if !permissions().check(&Capability::Network(address.clone())) {
                rv.set(err_json(_scope, "EACCES"));
                return;
            }

            let reg = udp_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                let dest = format!("{address}:{port}");
                if let Err(e) = state.socket.connect(dest) {
                    let code = node_code_for_io_error(&e);
                    let result = V8String::new(
                        _scope,
                        &format!(
                            r#"{{"code":"{code}","message":"connect {code} {address}:{port}"}}"#
                        ),
                    )
                    .unwrap();
                    rv.set(result.into());
                    return;
                }
                drop(reg);
                rv.set(v8::null(_scope).into());
            } else {
                drop(reg);
                rv.set(err_json(_scope, "ENOTCONN"));
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpConnect").unwrap().into(),
        udp_connect_fn.into(),
    );

    let udp_disconnect_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);

            let reg = udp_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                // This std no longer ships UdpSocket::disconnect; disconnect by
                // connect()ing an AF_UNSPEC sockaddr, like libuv does.
                let fd = state.socket.as_raw_fd();
                let addr = libc::sockaddr {
                    sa_family: libc::AF_UNSPEC as libc::sa_family_t,
                    sa_data: [0; 14],
                };
                unsafe {
                    libc::connect(
                        fd,
                        &addr as *const libc::sockaddr,
                        std::mem::size_of::<libc::sockaddr>() as libc::socklen_t,
                    );
                }
                rv.set(v8::null(_scope).into());
            } else {
                rv.set(err_json(_scope, "ENOTCONN"));
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpDisconnect").unwrap().into(),
        udp_disconnect_fn.into(),
    );

    let udp_remote_address_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);

            let reg = udp_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                if let Ok(peer) = state.socket.peer_addr() {
                    let (addr, family) = match peer {
                        SocketAddr::V4(a4) => (a4.ip().to_string(), "IPv4"),
                        SocketAddr::V6(a6) => (a6.ip().to_string(), "IPv6"),
                    };
                    let json = format!(
                        r#"{{"address":"{}","port":{},"family":"{}"}}"#,
                        addr,
                        peer.port(),
                        family
                    );
                    let result = V8String::new(_scope, &json).unwrap();
                    rv.set(result.into());
                } else {
                    rv.set(v8::null(_scope).into());
                }
            } else {
                rv.set(v8::null(_scope).into());
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpRemoteAddress").unwrap().into(),
        udp_remote_address_fn.into(),
    );

    // ── socket options (real, via socket2/libc) ──────────────────────────────
    let socket_opt_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);
            let op_arg = args.get(1);
            let op = op_arg.to_rust_string_lossy(_scope);

            let reg = udp_registry().lock().unwrap();
            let Some(state) = reg.get(&id) else {
                rv.set(err_json(_scope, "ENOTCONN"));
                return;
            };
            let sock_ref = socket2::SockRef::from(&*state.socket);
            let result: std::result::Result<(), std::string::String> = match op.as_str() {
                "setTTL" => {
                    let v = args.get(2).uint32_value(_scope).unwrap_or(0);
                    if state
                        .socket
                        .local_addr()
                        .map(|a| a.is_ipv6())
                        .unwrap_or(false)
                    {
                        sock_ref
                            .set_unicast_hops_v6(v)
                            .map_err(|e| node_code_for_io_error(&e))
                    } else {
                        sock_ref
                            .set_ttl_v4(v)
                            .map_err(|e| node_code_for_io_error(&e))
                    }
                }
                "setBroadcast" => {
                    let on = args.get(2).boolean_value(_scope);
                    sock_ref
                        .set_broadcast(on)
                        .map_err(|e| node_code_for_io_error(&e))
                }
                "setMulticastTTL" => {
                    let v = args.get(2).uint32_value(_scope).unwrap_or(0);
                    if state
                        .socket
                        .local_addr()
                        .map(|a| a.is_ipv6())
                        .unwrap_or(false)
                    {
                        sock_ref
                            .set_multicast_hops_v6(v)
                            .map_err(|e| node_code_for_io_error(&e))
                    } else {
                        sock_ref
                            .set_multicast_ttl_v4(v)
                            .map_err(|e| node_code_for_io_error(&e))
                    }
                }
                "setMulticastLoopback" => {
                    let on = args.get(2).boolean_value(_scope);
                    if state
                        .socket
                        .local_addr()
                        .map(|a| a.is_ipv6())
                        .unwrap_or(false)
                    {
                        sock_ref
                            .set_multicast_loop_v6(on)
                            .map_err(|e| node_code_for_io_error(&e))
                    } else {
                        sock_ref
                            .set_multicast_loop_v4(on)
                            .map_err(|e| node_code_for_io_error(&e))
                    }
                }
                "setRecvBufferSize" => {
                    let v = args.get(2).uint32_value(_scope).unwrap_or(0) as usize;
                    sock_ref
                        .set_recv_buffer_size(v)
                        .map_err(|e| node_code_for_io_error(&e))
                }
                "setSendBufferSize" => {
                    let v = args.get(2).uint32_value(_scope).unwrap_or(0) as usize;
                    sock_ref
                        .set_send_buffer_size(v)
                        .map_err(|e| node_code_for_io_error(&e))
                }
                "setMulticastInterface" => {
                    // IPv4 literal -> IP_MULTICAST_IF; interface *name* (v4 or
                    // v6, e.g. "en0") -> index-based setsockopt.
                    let iface = args.get(2).to_rust_string_lossy(_scope);
                    if let Ok(ip) = iface.parse::<Ipv4Addr>() {
                        sock_ref
                            .set_multicast_if_v4(&ip)
                            .map_err(|e| node_code_for_io_error(&e))
                    } else {
                        use std::ffi::CString;
                        let cname = match CString::new(iface.as_bytes()) {
                            Ok(c) => c,
                            Err(_) => {
                                rv.set(err_json(_scope, "EINVAL"));
                                return;
                            }
                        };
                        let idx = unsafe { libc::if_nametoindex(cname.as_ptr()) };
                        if idx == 0 {
                            rv.set(err_json(_scope, "EINVAL"));
                            return;
                        }
                        if state
                            .socket
                            .local_addr()
                            .map(|a| a.is_ipv6())
                            .unwrap_or(false)
                        {
                            sock_ref
                                .set_multicast_if_v6(idx)
                                .map_err(|e| node_code_for_io_error(&e))
                        } else {
                            // IPv4 IP_MULTICAST_IF takes an address, not an
                            // index; a v4 interface name falls back to the
                            // default interface (INADDR_ANY), matching the
                            // common case.
                            sock_ref
                                .set_multicast_if_v4(&Ipv4Addr::UNSPECIFIED)
                                .map_err(|e| node_code_for_io_error(&e))
                        }
                    }
                }
                "addMembership" => {
                    let group = args.get(2).to_rust_string_lossy(_scope);
                    let iface = args.get(3).to_rust_string_lossy(_scope);
                    if let Ok(g) = group.parse::<Ipv4Addr>() {
                        let interface = iface.parse::<Ipv4Addr>().unwrap_or(Ipv4Addr::UNSPECIFIED);
                        sock_ref
                            .join_multicast_v4(&g, &interface)
                            .map_err(|e| node_code_for_io_error(&e))
                    } else if let Ok(g6) = group.parse::<std::net::Ipv6Addr>() {
                        use std::ffi::CString;
                        let idx = if iface.is_empty() {
                            0
                        } else {
                            match CString::new(iface.as_bytes()) {
                                Ok(c) => unsafe { libc::if_nametoindex(c.as_ptr()) },
                                Err(_) => 0,
                            }
                        };
                        sock_ref
                            .join_multicast_v6(&g6, idx)
                            .map_err(|e| node_code_for_io_error(&e))
                    } else {
                        rv.set(err_json(_scope, "EINVAL"));
                        return;
                    }
                }
                "dropMembership" => {
                    let group = args.get(2).to_rust_string_lossy(_scope);
                    let iface = args.get(3).to_rust_string_lossy(_scope);
                    if let Ok(g) = group.parse::<Ipv4Addr>() {
                        let interface = iface.parse::<Ipv4Addr>().unwrap_or(Ipv4Addr::UNSPECIFIED);
                        sock_ref
                            .leave_multicast_v4(&g, &interface)
                            .map_err(|e| node_code_for_io_error(&e))
                    } else if let Ok(g6) = group.parse::<std::net::Ipv6Addr>() {
                        use std::ffi::CString;
                        let idx = if iface.is_empty() {
                            0
                        } else {
                            match CString::new(iface.as_bytes()) {
                                Ok(c) => unsafe { libc::if_nametoindex(c.as_ptr()) },
                                Err(_) => 0,
                            }
                        };
                        sock_ref
                            .leave_multicast_v6(&g6, idx)
                            .map_err(|e| node_code_for_io_error(&e))
                    } else {
                        rv.set(err_json(_scope, "EINVAL"));
                        return;
                    }
                }
                _ => {
                    rv.set(err_json(_scope, "EINVAL"));
                    return;
                }
            };

            match result {
                Ok(()) => rv.set(v8::null(_scope).into()),
                Err(code) => rv.set(err_json(_scope, &code)),
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpSocketOpt").unwrap().into(),
        socket_opt_fn.into(),
    );

    // ── buffer-size getters (return a number, or error JSON) ─────────────────
    let udp_buffer_size_fn = Function::new(
        scope,
        move |_scope: &mut PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);
            let op_arg = args.get(1);
            let op = op_arg.to_rust_string_lossy(_scope);

            let reg = udp_registry().lock().unwrap();
            let Some(state) = reg.get(&id) else {
                rv.set(err_json(_scope, "ENOTCONN"));
                return;
            };
            let sock_ref = socket2::SockRef::from(&*state.socket);
            let n = match op.as_str() {
                "getRecvBufferSize" => sock_ref.recv_buffer_size(),
                _ => sock_ref.send_buffer_size(),
            };
            match n {
                Ok(n) => rv.set(v8::Integer::new_from_unsigned(_scope, n as u32).into()),
                Err(e) => {
                    let code = node_code_for_io_error(&e);
                    rv.set(err_json(_scope, &code));
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__udpBufferSize").unwrap().into(),
        udp_buffer_size_fn.into(),
    );

    let js_code = r#"
        (function() {
            var EventEmitter = globalThis.__requireCache && globalThis.__requireCache['events'];

            function errFromRaw(raw, syscall, context) {
                var info;
                try { info = JSON.parse(raw); } catch(e) { info = { code: String(raw) }; }
                var e = new Error(info.message || '');
                e.code = info.code;
                // A send/bind to an unresolvable host fails in getaddrinfo, so
                // Node reports syscall 'getaddrinfo' and a hostname, even though
                // the operation was a sendto.
                if (info.code === 'ENOTFOUND') {
                    e.syscall = 'getaddrinfo';
                    e.hostname = context;
                } else {
                    e.syscall = syscall;
                }
                return e;
            }

            function errCode(name, msg) {
                var m = msg || 'Socket is already bound';
                if (name === 'ALREADY_BOUND') m = 'Socket is already bound';
                else if (name === 'DGRAM_NOT_RUNNING') m = 'Not running';
                else if (name === 'DGRAM_NOT_CONNECTED') m = 'Not connected';
                return Object.assign(new Error(m), { code: 'ERR_SOCKET_' + name });
            }

            function Socket(socketType) {
                if (EventEmitter) EventEmitter.call(this);
                this._type = socketType || 'udp4';
                this._id = null;
                this._bound = false;
                this._closed = false;
                var self = this;
                this._pollTimer = setInterval(function() {
                    if (self._closed) return;
                    var id = self._id;
                    if (!id) return;
                    var raw;
                    while ((raw = __udpRecv(id)) !== null && raw !== undefined) {
                        try {
                            var msg = JSON.parse(raw);
                            var buf = typeof Buffer !== 'undefined'
                                ? Buffer.from(msg.data, 'base64')
                                : (function() {
                                    var b = atob(msg.data);
                                    var arr = new Uint8Array(b.length);
                                    for (var i = 0; i < b.length; i++) arr[i] = b.charCodeAt(i);
                                    return arr;
                                })();
                            self.emit('message', buf, { address: msg.address, family: msg.family, port: msg.port, size: buf.length });
                        } catch(e) {}
                    }
                }, 10);
            }

            if (EventEmitter) {
                Socket.prototype = Object.create(EventEmitter.prototype);
                Socket.prototype.constructor = Socket;
            }

            Socket.prototype._defaultAddr = function() {
                return this._type === 'udp6' ? '::' : '0.0.0.0';
            };

            // Lazy socket creation: Node does not touch the OS until bind/send/
            // connect, so send() auto-binds and address() before bind throws.
            Socket.prototype._ensureCreated = function() {
                if (this._closed) throw errCode('DGRAM_NOT_RUNNING');
                if (!this._id) {
                    this._id = __udpCreate(this._type);
                    this._bound = true;
                }
                return this._id;
            };

            Socket.prototype.bind = function(port, address, callback) {
                if (typeof port === 'function') { callback = port; port = 0; address = this._defaultAddr(); }
                if (typeof address === 'function') { callback = address; address = this._defaultAddr(); }
                if (this._bound) throw errCode('ALREADY_BOUND');
                port = port || 0;
                address = address || this._defaultAddr();
                var self = this;
                var id = this._ensureCreated();
                var raw = __udpBind(id, port, address);
                if (raw) {
                    var e = errFromRaw(raw, 'bind', address + ':' + port);
                    if (typeof callback === 'function') setTimeout(function() { callback(e); }, 0);
                    else this.emit('error', e);
                    return this;
                }
                this._bound = true;
                setTimeout(function() {
                    self.emit('listening');
                    if (typeof callback === 'function') callback();
                }, 0);
                return this;
            };

            Socket.prototype.send = function(msg, offset, length, port, address, callback) {
                if (this._closed) throw errCode('DGRAM_NOT_RUNNING');
                if (typeof offset === 'number' && typeof length === 'number') {
                    if (typeof address === 'function') { callback = address; address = undefined; }
                } else {
                    var last = arguments[arguments.length - 1];
                    if (typeof last === 'function') callback = last;
                    if (typeof offset === 'function') {
                        offset = 0;
                        length = msg ? msg.length : 0;
                        port = undefined;
                        address = undefined;
                    } else {
                        port = offset;
                        address = (typeof length === 'function') ? undefined : length;
                        offset = 0;
                        length = msg ? msg.length : 0;
                    }
                }
                if (!this._connected && (typeof port !== 'number' || port < 0 || port > 65535 || (port | 0) !== port)) {
                    throw Object.assign(new TypeError('The argument \'port\' is invalid. Received ' + port), { code: 'ERR_SOCKET_BAD_PORT' });
                }
                if (this._connected) { port = 0; address = ''; }
                else if (typeof port !== 'number') { port = 0; }
                if (!address) address = '';
                var data;
                if (typeof msg === 'string') {
                    data = btoa(msg.slice(offset, offset + length));
                } else if (msg instanceof Uint8Array || (typeof Buffer !== 'undefined' && Buffer.isBuffer(msg))) {
                    var b = '';
                    for (var i = offset; i < offset + length && i < msg.length; i++) b += String.fromCharCode(msg[i]);
                    data = btoa(b);
                } else {
                    data = btoa(String(msg));
                }
                var id = this._ensureCreated();
                var raw = __udpSend(id, data, offset, length, port, address);
                var err = null;
                if (raw) err = errFromRaw(raw, 'sendto', (address || 'connected') + ':' + port);
                if (typeof callback === 'function') {
                    setTimeout(function() { callback(err, err ? undefined : length); }, 0);
                }
                return false;
            };

            Socket.prototype.address = function() {
                if (this._closed) throw errCode('DGRAM_NOT_RUNNING');
                if (!this._id) throw Object.assign(new Error('Socket is not running'), { code: 'EBADF' });
                var raw = __udpAddress(this._id);
                return raw ? JSON.parse(raw) : null;
            };

            Socket.prototype.close = function(callback) {
                this._closed = true;
                clearInterval(this._pollTimer);
                if (this._id) __udpClose(this._id);
                var self = this;
                setTimeout(function() {
                    self.emit('close');
                    if (typeof callback === 'function') callback();
                }, 0);
                return this;
            };

            Socket.prototype.connect = function(port, address, callback) {
                if (typeof address === 'function') { callback = address; address = '127.0.0.1'; }
                address = address || '127.0.0.1';
                var id = this._ensureCreated();
                var raw = __udpConnect(id, port, address);
                if (raw) {
                    var e = errFromRaw(raw, 'connect', address + ':' + port);
                    if (typeof callback === 'function') setTimeout(function() { callback(e); }, 0);
                    else this.emit('error', e);
                } else {
                    this._connected = true;
                    if (typeof callback === 'function') setTimeout(callback, 0);
                }
                return undefined;
            };

            Socket.prototype.disconnect = function() {
                this._connected = false;
                if (this._id) __udpDisconnect(this._id);
                return undefined;
            };

            Socket.prototype.remoteAddress = function() {
                if (this._closed) throw errCode('DGRAM_NOT_RUNNING');
                var id = this._id;
                if (!id) throw Object.assign(new Error('Not connected'), { code: 'ERR_SOCKET_DGRAM_NOT_CONNECTED' });
                var raw = __udpRemoteAddress(id);
                if (!raw) throw Object.assign(new Error('Not connected'), { code: 'ERR_SOCKET_DGRAM_NOT_CONNECTED' });
                return JSON.parse(raw);
            };

            function rangeErr() {
                return Object.assign(new RangeError('The value of "ttl" is out of range'), { code: 'ERR_SOCKET_DGRAM_TTL_RANGE' });
            }

            function socketOpt(self, name, arg2, arg3) {
                var id = self._id;
                if (!id) return;
                var raw = __udpSocketOpt(id, name, arg2, arg3);
                if (raw) throw errFromRaw(raw, name, '');
            }

            Socket.prototype.setTTL = function(ttl) {
                if (typeof ttl !== 'number') throw new TypeError('The "ttl" argument must be of type number');
                if (ttl < 1 || ttl > 255) throw rangeErr();
                socketOpt(this, 'setTTL', ttl);
                return ttl;
            };
            Socket.prototype.setBroadcast = function(flag) {
                socketOpt(this, 'setBroadcast', !!flag);
                return undefined;
            };
            Socket.prototype.setMulticastTTL = function(ttl) {
                if (typeof ttl !== 'number') throw new TypeError('The "ttl" argument must be of type number');
                if (ttl < 0 || ttl > 255) throw rangeErr();
                socketOpt(this, 'setMulticastTTL', ttl);
                return ttl;
            };
            Socket.prototype.setMulticastLoopback = function(flag) {
                socketOpt(this, 'setMulticastLoopback', !!flag);
                return !!flag;
            };
            Socket.prototype.setMulticastInterface = function(multicastInterface) {
                socketOpt(this, 'setMulticastInterface', multicastInterface);
                return undefined;
            };
            Socket.prototype.addMembership = function(multicastAddress, multicastInterface) {
                socketOpt(this, 'addMembership', multicastAddress, multicastInterface || '');
                return undefined;
            };
            Socket.prototype.dropMembership = function(multicastAddress, multicastInterface) {
                socketOpt(this, 'dropMembership', multicastAddress, multicastInterface || '');
                return undefined;
            };
            Socket.prototype.setRecvBufferSize = function(size) {
                if (typeof size !== 'number') throw new TypeError('The "size" argument must be of type number');
                socketOpt(this, 'setRecvBufferSize', size);
                return undefined;
            };
            Socket.prototype.setSendBufferSize = function(size) {
                if (typeof size !== 'number') throw new TypeError('The "size" argument must be of type number');
                socketOpt(this, 'setSendBufferSize', size);
                return undefined;
            };
            Socket.prototype.getRecvBufferSize = function() {
                var id = this._id;
                if (!id) throw errCode('DGRAM_NOT_RUNNING');
                var raw = __udpBufferSize(id, 'getRecvBufferSize');
                if (typeof raw === 'string') throw errFromRaw(raw, 'getsockopt', '');
                return raw;
            };
            Socket.prototype.getSendBufferSize = function() {
                var id = this._id;
                if (!id) throw errCode('DGRAM_NOT_RUNNING');
                var raw = __udpBufferSize(id, 'getSendBufferSize');
                if (typeof raw === 'string') throw errFromRaw(raw, 'getsockopt', '');
                return raw;
            };
            Socket.prototype.ref = function() { return this; };
            Socket.prototype.unref = function() { return this; };

            var dgram = {
                createSocket: function(type, callback) {
                    var sock = new Socket(typeof type === 'object' ? (type.type || 'udp4') : type);
                    if (typeof callback === 'function') sock.on('message', callback);
                    return sock;
                },
                Socket: Socket
            };

            globalThis.__requireCache['dgram'] = dgram;
            globalThis.__requireCache['node:dgram'] = dgram;
        })();
    "#;
    let source = V8String::new(scope, js_code).unwrap();
    let _ = Script::compile(scope, source, None).and_then(|s| s.run(scope));

    Ok(())
}

fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn base64_decode(s: &str) -> std::result::Result<Vec<u8>, std::string::String> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_udp_id_is_monotonic() {
        let a = next_udp_id();
        let b = next_udp_id();
        assert!(b > a);
    }

    #[test]
    fn base64_round_trip() {
        let data = b"hello, UDP!";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn udp_socket_bind_and_send_loopback() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let recv_port = receiver.local_addr().unwrap().port();

        sender
            .send_to(b"ping", format!("127.0.0.1:{recv_port}"))
            .unwrap();

        let mut buf = [0u8; 128];
        receiver
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let (n, _src) = receiver.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"ping");
    }

    #[test]
    fn io_error_maps_to_node_codes() {
        use std::net::{TcpListener, TcpStream};
        // bind a UDP socket on an in-use UDP port -> EADDRINUSE
        let u1 = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = u1.local_addr().unwrap();
        let err = UdpSocket::bind(addr).err().unwrap();
        assert_eq!(node_code_for_io_error(&err), "EADDRINUSE");
        // connect to a refused port -> ECONNREFUSED
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let refused = l.local_addr().unwrap();
        drop(l);
        if let Err(e) = TcpStream::connect(refused) {
            assert_eq!(node_code_for_io_error(&e), "ECONNREFUSED");
        }
    }
}
