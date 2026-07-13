//! UDP datagram socket backend for the `dgram` Node.js module.

use base64::Engine;
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex, OnceLock};
use v8::{ContextScope, Function, HandleScope, Script, String as V8String};

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
                        let (addr, port) = match src {
                            SocketAddr::V4(a) => (a.ip().to_string(), a.port()),
                            SocketAddr::V6(a) => (a.ip().to_string(), a.port()),
                        };
                        queue.lock().unwrap().push_back(UdpMessage {
                            data: buf[..n].to_vec(),
                            address: addr,
                            port,
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

pub fn inject_dgram(
    scope: &mut ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    DGRAM_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));
    let context = scope.get_current_context();
    let global = context.global(scope);

    let udp_create_fn = Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
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
        move |_scope: &mut v8::PinScope,
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
                    &format!("EACCES: permission denied (--allow-net={})", address),
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
                        let result = V8String::new(_scope, &e.to_string()).unwrap();
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
        move |_scope: &mut v8::PinScope,
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
                    &format!("EACCES: permission denied (--allow-net={})", address),
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
                if let Err(e) = state.socket.send_to(&bytes, &dest) {
                    let result = V8String::new(_scope, &e.to_string()).unwrap();
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
        move |_scope: &mut v8::PinScope,
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
                        r#"{{"data":"{}","address":"{}","port":{},"family":"IPv4"}}"#,
                        b64, m.address, m.port
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
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(_scope).unwrap_or(0);

            let reg = udp_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                if let Ok(a) = state.socket.local_addr() {
                    let (addr, port) = match a {
                        SocketAddr::V4(a4) => (a4.ip().to_string(), a4.port()),
                        SocketAddr::V6(a6) => (a6.ip().to_string(), a6.port()),
                    };
                    let json = format!(
                        r#"{{"address":"{}","port":{},"family":"IPv4"}}"#,
                        addr, port
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
        move |_scope: &mut v8::PinScope,
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

    let js_code = r#"
        (function() {
            var EventEmitter = globalThis.__requireCache && globalThis.__requireCache['events'];

            function Socket(socketType) {
                if (EventEmitter) EventEmitter.call(this);
                this._type = socketType || 'udp4';
                this._id = __udpCreate(this._type);
                this._bound = false;
                this._closed = false;
                var self = this;
                this._pollTimer = setInterval(function() {
                    if (self._closed) return;
                    var raw;
                    while ((raw = __udpRecv(self._id)) !== null && raw !== undefined) {
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
                            self.emit('message', buf, { address: msg.address, port: msg.port, family: msg.family });
                        } catch(e) {}
                    }
                }, 10);
            }

            if (EventEmitter) {
                Socket.prototype = Object.create(EventEmitter.prototype);
                Socket.prototype.constructor = Socket;
            }

            Socket.prototype.bind = function(port, address, callback) {
                if (typeof port === 'function') { callback = port; port = 0; address = '0.0.0.0'; }
                if (typeof address === 'function') { callback = address; address = '0.0.0.0'; }
                address = address || '0.0.0.0';
                port = port || 0;
                var self = this;
                var err = __udpBind(this._id, port, address);
                if (err) {
                    var e = Object.assign(new Error(err), { code: 'EACCES' });
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
                if (typeof offset === 'number' && typeof length === 'number') {
                    if (typeof address === 'function') { callback = address; address = undefined; }
                } else {
                    callback = (typeof length === 'function') ? length : port;
                    address = (typeof length === 'function') ? undefined : length;
                    port = offset;
                    offset = 0;
                    length = msg ? msg.length : 0;
                }
                address = address || '127.0.0.1';
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
                var err = __udpSend(this._id, data, offset, length, port, address);
                if (typeof callback === 'function') {
                    setTimeout(function() { callback(err ? new Error(err) : null); }, 0);
                }
                return this;
            };

            Socket.prototype.address = function() {
                var raw = __udpAddress(this._id);
                return raw ? JSON.parse(raw) : null;
            };

            Socket.prototype.close = function(callback) {
                this._closed = true;
                clearInterval(this._pollTimer);
                __udpClose(this._id);
                var self = this;
                setTimeout(function() {
                    self.emit('close');
                    if (typeof callback === 'function') callback();
                }, 0);
                return this;
            };

            Socket.prototype.setBroadcast = function() { return this; };
            Socket.prototype.setMulticastTTL = function() { return this; };
            Socket.prototype.setMulticastLoopback = function() { return this; };
            Socket.prototype.addMembership = function() { return this; };
            Socket.prototype.dropMembership = function() { return this; };
            Socket.prototype.setTTL = function() { return this; };
            Socket.prototype.unref = function() { return this; };
            Socket.prototype.ref = function() { return this; };

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
}
