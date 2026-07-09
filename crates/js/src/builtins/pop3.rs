//! POP3 (Post Office Protocol v3) client built-in module
//!
//! Native functions:
//! - `__pop3Create()` -> id
//! - `__pop3Connect(id, host, port, useTls)` -> throws on failure
//! - `__pop3Send(id, line)` -> throws on failure
//! - `__pop3Read(id, maxBytes)` -> Vec<u8> | throws EAGAIN | throws EOF
//! - `__pop3Close(id)`

use crate::builtins::v8_compat::uint8array_from_bytes;
use native_tls::TlsStream;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
use v8::{FunctionCallbackArguments, PinScope, ReturnValue};
use vvva_permissions::{Capability, PermissionState};

type Pop3Id = u32;

#[allow(clippy::large_enum_variant)]
enum Pop3Conn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl Pop3Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Pop3Conn::Plain(s) => s.read(buf),
            Pop3Conn::Tls(s) => s.read(buf),
        }
    }
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            Pop3Conn::Plain(s) => s.write_all(data),
            Pop3Conn::Tls(s) => s.write_all(data),
        }
    }
    fn shutdown(&mut self) {
        match self {
            Pop3Conn::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            Pop3Conn::Tls(s) => {
                let _ = s.shutdown();
            }
        }
    }
}

static POP3_REGISTRY: OnceLock<Mutex<HashMap<Pop3Id, Pop3Conn>>> = OnceLock::new();

fn pop3_registry() -> &'static Mutex<HashMap<Pop3Id, Pop3Conn>> {
    POP3_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_pop3_id() -> Pop3Id {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn inject_pop3(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    permissions: Arc<PermissionState>,
) {
    let context = scope.get_current_context();
    let global = context.global(scope);
    let _perms = permissions.clone();
    let _perms2 = permissions.clone();
    let _perms3 = permissions.clone();
    let _perms4 = permissions.clone();

    let create_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              _args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = next_pop3_id();
            rv.set(v8::Number::new(scope, id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__pop3Create").unwrap().into(),
        create_fn.into(),
    );

    let perms_ptr = Arc::into_raw(permissions.clone()) as *mut std::ffi::c_void;
    let external = v8::External::new(scope, perms_ptr);
    let connect_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let perms = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const PermissionState)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as Pop3Id;
            let host = args.get(1).to_rust_string_lossy(scope);
            let port = args.get(2).uint32_value(scope).unwrap_or(110) as u16;
            let use_tls = args.get(3).boolean_value(scope);

            if !perms.check(&Capability::Network(host.clone())) {
                let msg = v8::String::new(
                    scope,
                    &format!("Network access denied. Run with --allow-net={}", host),
                )
                .unwrap();
                let err = v8::Exception::error(scope, msg);
                rv.set(err);
                return;
            }

            match TcpStream::connect(format!("{}:{}", host, port)) {
                Ok(tcp) => {
                    let conn = if use_tls {
                        match native_tls::TlsConnector::new() {
                            Ok(connector) => {
                                let fallback = tcp.try_clone().ok();
                                match connector.connect(&host, tcp) {
                                    Ok(tls) => {
                                        if tls.get_ref().set_nonblocking(true).is_ok() {
                                            Pop3Conn::Tls(tls)
                                        } else if let Some(tcp) = fallback {
                                            Pop3Conn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                    Err(_) => {
                                        if let Some(tcp) = fallback {
                                            Pop3Conn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                }
                            }
                            Err(_) => Pop3Conn::Plain(tcp),
                        }
                    } else {
                        let _ = tcp.set_nonblocking(true);
                        Pop3Conn::Plain(tcp)
                    };

                    pop3_registry().lock().unwrap().insert(id, conn);
                    rv.set(v8::undefined(scope).into());
                }
                Err(e) => {
                    let msg = v8::String::new(scope, &format!("Connection failed: {}", e)).unwrap();
                    let err = v8::Exception::error(scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__pop3Connect").unwrap().into(),
        connect_fn.into(),
    );

    let send_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as Pop3Id;
            let line = args.get(1).to_rust_string_lossy(scope);

            let mut reg = pop3_registry().lock().unwrap();
            match reg.get_mut(&id) {
                Some(conn) => match conn.write_all(format!("{}\r\n", line).as_bytes()) {
                    Ok(_) => rv.set(v8::undefined(scope).into()),
                    Err(e) => {
                        let msg = v8::String::new(scope, &e.to_string()).unwrap();
                        let err = v8::Exception::error(scope, msg);
                        rv.set(err);
                    }
                },
                None => {
                    let msg = v8::String::new(scope, "not connected").unwrap();
                    let err = v8::Exception::error(scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__pop3Send").unwrap().into(),
        send_fn.into(),
    );

    let read_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as Pop3Id;
            let max_bytes = args.get(1).uint32_value(scope).unwrap_or(65536) as usize;
            let max = max_bytes.min(65536);

            let mut buf = vec![0u8; max];
            let mut reg = pop3_registry().lock().unwrap();
            match reg.get_mut(&id) {
                Some(conn) => match conn.read(&mut buf) {
                    Ok(0) => {
                        let msg = v8::String::new(scope, "connection closed").unwrap();
                        let err = v8::Exception::error(scope, msg);
                        rv.set(err);
                    }
                    Ok(n) => {
                        buf.truncate(n);
                        rv.set(uint8array_from_bytes(scope, &buf).into());
                    }
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        let msg = v8::String::new(scope, "no data available").unwrap();
                        let err = v8::Exception::error(scope, msg);
                        rv.set(err);
                    }
                    Err(e) => {
                        let msg = v8::String::new(scope, &e.to_string()).unwrap();
                        let err = v8::Exception::error(scope, msg);
                        rv.set(err);
                    }
                },
                None => {
                    let msg = v8::String::new(scope, "not connected").unwrap();
                    let err = v8::Exception::error(scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__pop3Read").unwrap().into(),
        read_fn.into(),
    );

    let close_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as Pop3Id;
            if let Some(mut conn) = pop3_registry().lock().unwrap().remove(&id) {
                conn.shutdown();
            }
            rv.set(v8::Boolean::new(scope, true).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__pop3Close").unwrap().into(),
        close_fn.into(),
    );

    let js_code = r#"
    (function() {
        function Client(options) {
            options = options || {};
            this.host = options.host || 'localhost';
            this.port = options.port || (options.tls ? 995 : 110);
            this.tls = options.tls || false;
            this.username = options.username || '';
            this.password = options.password || '';
            this.timeout = options.timeout || 10000;
            this._id = __pop3Create();
            this._connected = false;
            this._authenticated = false;
            this._lineBuffer = '';
            this._pollTimer = null;
            this._handlers = {};
            this._pendingCmd = null;
            this._pendingCallback = null;
            this._pendingData = null;
            this._pendingMultiline = false;
        }

        Client.prototype.connect = function(callback) {
            var self = this;
            if (callback) self.on('connect', callback);
            setTimeout(function() {
                try {
                    __pop3Connect(self._id, self.host, self.port, self.tls);
                    self._connected = true;
                    self._startPoll();
                    self._pendingCmd = 'GREETING';
                    self._pendingCallback = function(err) {
                        if (err) { self.emit('error', err); return; }
                        self.emit('connect');
                    };
                    self._pendingData = '';
                    self._pendingMultiline = false;
                } catch(e) {
                    self.emit('error', e);
                }
            }, 0);
            return this;
        };

        Client.prototype._startPoll = function() {
            var self = this;
            var delay = 1;
            function poll() {
                if (!self._connected) { self._pollTimer = null; return; }
                try {
                    var chunk = __pop3Read(self._id, 65536);
                    delay = 1;
                    self._lineBuffer += new TextDecoder().decode(new Uint8Array(chunk));
                    var lines = self._lineBuffer.split('\n');
                    self._lineBuffer = lines.pop();
                    for (var i = 0; i < lines.length; i++) {
                        var line = lines[i].replace(/\r$/, '');
                        if (line.length) self._handleLine(line);
                    }
                    self._pollTimer = setTimeout(poll, 0);
                } catch (e) {
                    if (e && e.code === 'EAGAIN') {
                        delay = Math.min(delay * 2, 100);
                        self._pollTimer = setTimeout(poll, delay);
                        return;
                    }
                    self._pollTimer = null;
                    self._connected = false;
                    if (e && e.code === 'EOF') self.emit('close');
                    else self.emit('error', e);
                }
            }
            self._pollTimer = setTimeout(poll, 0);
        };

        Client.prototype._resolvePending = function(err, data) {
            var cb = this._pendingCallback;
            this._pendingCmd = null;
            this._pendingCallback = null;
            this._pendingData = null;
            this._pendingMultiline = false;
            if (cb) cb(err, data);
        };

        Client.prototype._handleLine = function(line) {
            this.emit('raw', line);
            if (line.indexOf('+OK') === 0 || line.indexOf('+ ') === 0) {
                this.emit('response', line);
                if (this._pendingCmd) {
                    if (!this._pendingMultiline) {
                        this._resolvePending(null, line);
                    }
                }
            } else if (line.indexOf('-ERR') === 0) {
                this.emit('error', new Error(line));
                if (this._pendingCmd) this._resolvePending(new Error(line));
            } else if (this._pendingCmd && this._pendingMultiline) {
                if (line === '.') {
                    this._resolvePending(null, this._pendingData || '');
                } else {
                    var body = line.indexOf('..') === 0 ? line.slice(1) : line;
                    this._pendingData = (this._pendingData || '') + body + '\n';
                }
            }
        };

        Client.prototype._sendAwait = function(cmd, callback) {
            this._pendingCmd = cmd;
            this._pendingCallback = callback;
            this._pendingData = '';
            this._pendingMultiline = false;
            try {
                __pop3Send(this._id, cmd);
            } catch (e) {
                this._pendingCallback = null;
                this._pendingCmd = null;
                if (callback) callback(e);
            }
        };

        Client.prototype.login = function(username, password, callback) {
            var self = this;
            this.username = username || this.username;
            this.password = password || this.password;
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            this._sendAwait('USER ' + this.username, function(userErr) {
                if (userErr) { if (callback) callback(userErr); return; }
                self._sendAwait('PASS ' + self.password, function(passErr) {
                    if (passErr) { if (callback) callback(passErr); return; }
                    self._authenticated = true;
                    if (callback) callback();
                });
            });
            return this;
        };

        Client.prototype.stat = function(callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'), null);
                return { messages: 0, size: 0 };
            }
            this._pendingCmd = 'STAT';
            this._pendingCallback = callback;
            this._pendingData = '';
            this._pendingMultiline = false;
            try {
                __pop3Send(this._id, 'STAT');
            } catch(e) {
                this._pendingCallback = null;
                this._pendingCmd = null;
                if (callback) callback(e, null);
            }
            return { messages: 0, size: 0 };
        };

        Client.prototype.list = function(msgNumber, callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'), null);
                return [];
            }
            this._pendingCmd = 'LIST';
            this._pendingCallback = callback;
            this._pendingData = '';
            this._pendingMultiline = (msgNumber === undefined);
            try {
                if (msgNumber !== undefined) {
                    __pop3Send(this._id, 'LIST ' + msgNumber);
                } else {
                    __pop3Send(this._id, 'LIST');
                }
            } catch(e) {
                this._pendingCallback = null;
                this._pendingCmd = null;
                if (callback) callback(e, null);
            }
            return [];
        };

        Client.prototype.retr = function(msgNumber, callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'), null);
                return '';
            }
            this._pendingCmd = 'RETR';
            this._pendingCallback = callback;
            this._pendingData = '';
            this._pendingMultiline = true;
            try {
                __pop3Send(this._id, 'RETR ' + msgNumber);
            } catch(e) {
                this._pendingCallback = null;
                this._pendingCmd = null;
                if (callback) callback(e, null);
            }
            return '';
        };

        Client.prototype.dele = function(msgNumber, callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            try {
                __pop3Send(this._id, 'DELE ' + msgNumber);
                if (callback) callback();
            } catch(e) {
                if (callback) callback(e);
            }
            return this;
        };

        Client.prototype.rset = function(callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            try {
                __pop3Send(this._id, 'RSET');
                if (callback) callback();
            } catch(e) {
                if (callback) callback(e);
            }
            return this;
        };

        Client.prototype.noop = function(callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            try {
                __pop3Send(this._id, 'NOOP');
                if (callback) callback();
            } catch(e) {
                if (callback) callback(e);
            }
            return this;
        };

        Client.prototype.uidl = function(msgNumber, callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'), null);
                return [];
            }
            this._pendingCmd = 'UIDL';
            this._pendingCallback = callback;
            this._pendingData = '';
            this._pendingMultiline = (msgNumber === undefined);
            try {
                if (msgNumber !== undefined) {
                    __pop3Send(this._id, 'UIDL ' + msgNumber);
                } else {
                    __pop3Send(this._id, 'UIDL');
                }
            } catch(e) {
                this._pendingCallback = null;
                this._pendingCmd = null;
                if (callback) callback(e, null);
            }
            return [];
        };

        Client.prototype.quit = function(callback) {
            if (this._connected) {
                try {
                    __pop3Send(this._id, 'QUIT');
                } catch(e) {}
            }
            __pop3Close(this._id);
            this._connected = false;
            this._authenticated = false;
            if (this._pollTimer) {
                clearTimeout(this._pollTimer);
                this._pollTimer = null;
            }
            if (callback) callback();
            return this;
        };

        Client.prototype.disconnect = Client.prototype.quit;
        Client.prototype.close = Client.prototype.quit;
        Client.prototype.retrive = Client.prototype.retr;
        Client.prototype.delete = Client.prototype.dele;
        Client.prototype.reset = Client.prototype.rset;

        Client.prototype.on = Client.prototype.addListener = function(event, handler) {
            this._handlers[event] = this._handlers[event] || [];
            this._handlers[event].push(handler);
            return this;
        };

        Client.prototype.off = Client.prototype.removeListener = function(event, handler) {
            if (this._handlers[event] && handler) {
                var idx = this._handlers[event].indexOf(handler);
                if (idx >= 0) this._handlers[event].splice(idx, 1);
            }
            return this;
        };

        Client.prototype.removeAllListeners = function(event) {
            if (event) this._handlers[event] = [];
            else this._handlers = {};
            return this;
        };

        Client.prototype.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            (this._handlers[event] || []).forEach(function(h) { h.apply(null, args); });
            var handler = this['on' + event];
            if (handler) handler.apply(null, args);
        };

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['pop3'] = { Client: Client };
        globalThis.__requireCache['node:pop3'] = { Client: Client };
        globalThis.pop3 = { Client: Client };
    })();
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }
}
