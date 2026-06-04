//! UDP datagram socket backend for the `dgram` Node.js module.
//!
//! # Architecture
//!
//! UDP sockets are held in a global `Arc<Mutex<HashMap<u32, UdpState>>>`.
//! The JS side calls `__udpBind`, `__udpSend`, `__udpClose`, and polls for
//! incoming messages with `__udpRecv` (returns `null` when the queue is empty).
//!
//! Sockets receive via a background thread that parks in `recv_from` and
//! pushes datagrams into a `VecDeque<UdpMessage>` protected by a `Mutex`.
//!
//! ## Permissions
//!
//! All operations require `Capability::Network(<host>)`.

use rquickjs::{Ctx, Function, Result};
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

// ── Socket registry ───────────────────────────────────────────────────────────

type SocketId = u32;

struct UdpState {
    socket: Arc<UdpSocket>,
    recv_queue: Arc<Mutex<VecDeque<UdpMessage>>>,
    /// Shutdown flag — set to `true` to stop the receiver thread.
    closed: Arc<std::sync::atomic::AtomicBool>,
}

struct UdpMessage {
    data: Vec<u8>,
    address: String,
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

// ── Background receiver thread ────────────────────────────────────────────────

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
                // 500 ms read timeout so we can check the closed flag.
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
                            || e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        // timeout — loop to check closed flag
                    }
                    Err(_) => break,
                }
            }
        })
        .ok();
}

// ── Native function injection ─────────────────────────────────────────────────

pub fn inject_dgram(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // __udpCreate(socketType) → socketId
    // socketType: "udp4" | "udp6" (we bind :: or 0.0.0.0 depending on type)
    let create_fn = Function::new(ctx.clone(), move |socket_type: String| -> u32 {
        let id = next_udp_id();
        let bind_addr: &str = if socket_type == "udp6" {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        };
        let sock = match UdpSocket::bind(bind_addr) {
            Ok(s) => Arc::new(s),
            Err(_) => return 0,
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
        id
    })?;
    globals.set("__udpCreate", create_fn)?;

    // __udpBind(id, port, address) → null|errorString
    let perms_bind = permissions.clone();
    let bind_fn = Function::new(
        ctx.clone(),
        move |id: u32, port: u16, address: String| -> Option<String> {
            if !perms_bind.check(&Capability::Network(address.clone())) {
                return Some(format!(
                    "EACCES: permission denied (--allow-net={})",
                    address
                ));
            }
            let bind_addr = format!("{address}:{port}");
            let reg = udp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                // The socket is already bound to an ephemeral port. Re-bind by
                // rebinding is not straightforward; instead we close the old socket
                // and create a new one on the specified address.
                drop(reg);
                let new_sock = match UdpSocket::bind(&bind_addr) {
                    Ok(s) => Arc::new(s),
                    Err(e) => return Some(e.to_string()),
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
                None
            } else {
                Some(format!("ENOENT: unknown socket id {id}"))
            }
        },
    )?;
    globals.set("__udpBind", bind_fn)?;

    // __udpSend(id, dataBase64, offset, length, port, address) → null|errorString
    let perms_send = permissions.clone();
    let send_fn = Function::new(
        ctx.clone(),
        move |id: u32,
              data_b64: String,
              _offset: u32,
              _length: u32,
              port: u16,
              address: String|
              -> Option<String> {
            if !perms_send.check(&Capability::Network(address.clone())) {
                return Some(format!(
                    "EACCES: permission denied (--allow-net={})",
                    address
                ));
            }
            let bytes = match base64_decode(&data_b64) {
                Ok(b) => b,
                Err(e) => return Some(e),
            };
            let dest = format!("{address}:{port}");
            let reg = udp_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                if let Err(e) = state.socket.send_to(&bytes, &dest) {
                    return Some(e.to_string());
                }
            } else {
                return Some(format!("ENOENT: unknown socket id {id}"));
            }
            None
        },
    )?;
    globals.set("__udpSend", send_fn)?;

    // __udpRecv(id) → null | { data: base64, address, port, family }
    let recv_fn = Function::new(ctx.clone(), move |id: u32| -> Option<String> {
        let reg = udp_registry().lock().unwrap();
        if let Some(state) = reg.get(&id) {
            let msg = state.recv_queue.lock().unwrap().pop_front();
            drop(reg);
            msg.map(|m| {
                let b64 = base64_encode(&m.data);
                format!(
                    r#"{{"data":"{}","address":"{}","port":{},"family":"IPv4"}}"#,
                    b64, m.address, m.port
                )
            })
        } else {
            None
        }
    })?;
    globals.set("__udpRecv", recv_fn)?;

    // __udpAddress(id) → { address, port, family } | null
    let addr_fn = Function::new(ctx.clone(), move |id: u32| -> Option<String> {
        let reg = udp_registry().lock().unwrap();
        reg.get(&id).and_then(|state| {
            state.socket.local_addr().ok().map(|a| {
                let (addr, port) = match a {
                    SocketAddr::V4(a4) => (a4.ip().to_string(), a4.port()),
                    SocketAddr::V6(a6) => (a6.ip().to_string(), a6.port()),
                };
                format!(
                    r#"{{"address":"{}","port":{},"family":"IPv4"}}"#,
                    addr, port
                )
            })
        })
    })?;
    globals.set("__udpAddress", addr_fn)?;

    // __udpClose(id)
    let close_fn = Function::new(ctx.clone(), move |id: u32| {
        if let Some(state) = udp_registry().lock().unwrap().remove(&id) {
            state
                .closed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    })?;
    globals.set("__udpClose", close_fn)?;

    // Inject the JS-level `dgram` module backed by the native functions above.
    ctx.eval::<(), _>(
        r#"
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
                    // Convert base64 data back to Buffer
                    var buf = typeof Buffer !== 'undefined'
                        ? Buffer.from(msg.data, 'base64')
                        : (function() {
                            var b = atob(msg.data);
                            var arr = new Uint8Array(b.length);
                            for (var i = 0; i < b.length; i++) arr[i] = b.charCodeAt(i);
                            return arr;
                        }());
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
        // Overload: send(msg, port, address, [callback])
        if (typeof offset === 'number' && typeof length === 'number') {
            // full signature
        } else {
            callback = address;
            address = port;
            port = length;
            length = msg.length;
            offset = 0;
        }
        var data;
        if (typeof msg === 'string') {
            data = btoa(msg);
        } else if (msg instanceof Uint8Array || (typeof Buffer !== 'undefined' && Buffer.isBuffer(msg))) {
            var b = '';
            for (var i = 0; i < msg.length; i++) b += String.fromCharCode(msg[i]);
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
}());
        "#,
    )?;

    Ok(())
}

// ── Minimal base64 helpers (no extra dep) ─────────────────────────────────────

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn base64_decode(s: &str) -> std::result::Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| e.to_string())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

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
        // Create two real OS sockets and exchange one datagram.
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
