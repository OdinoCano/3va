//! IRC (Internet Relay Chat) client built-in module
//!
//! Native functions:
//! - `__ircCreate(server, port, nick)` -> id
//! - `__ircConnect(id, host, port, useTls)` -> throws on failure
//! - `__ircSend(id, line)` -> throws on failure
//! - `__ircRead(id, maxBytes)` -> Vec<u8> | throws EAGAIN | throws EOF
//! - `__ircClose(id)`

use crate::builtins::v8_compat::uint8array_from_bytes;
use native_tls::TlsStream;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
use v8::{FunctionCallbackArguments, PinScope, ReturnValue};
use vvva_permissions::{Capability, PermissionState};

type IrcId = u32;

#[allow(clippy::large_enum_variant)]
enum IrcConn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl IrcConn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            IrcConn::Plain(s) => s.read(buf),
            IrcConn::Tls(s) => s.read(buf),
        }
    }
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            IrcConn::Plain(s) => s.write_all(data),
            IrcConn::Tls(s) => s.write_all(data),
        }
    }
    fn shutdown(&mut self) {
        match self {
            IrcConn::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            IrcConn::Tls(s) => {
                let _ = s.shutdown();
            }
        }
    }
}

static IRC_REGISTRY: OnceLock<Mutex<HashMap<IrcId, IrcConn>>> = OnceLock::new();

fn irc_registry() -> &'static Mutex<HashMap<IrcId, IrcConn>> {
    IRC_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_irc_id() -> IrcId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn inject_irc(
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
            let id = next_irc_id();
            rv.set(v8::Number::new(scope, id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ircCreate").unwrap().into(),
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
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as IrcId;
            let host = args.get(1).to_rust_string_lossy(scope);
            let port = args.get(2).uint32_value(scope).unwrap_or(6667) as u16;
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
                                            IrcConn::Tls(tls)
                                        } else if let Some(tcp) = fallback {
                                            IrcConn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                    Err(_) => {
                                        if let Some(tcp) = fallback {
                                            IrcConn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                }
                            }
                            Err(_) => IrcConn::Plain(tcp),
                        }
                    } else {
                        let _ = tcp.set_nonblocking(true);
                        IrcConn::Plain(tcp)
                    };

                    irc_registry().lock().unwrap().insert(id, conn);
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
        v8::String::new(scope, "__ircConnect").unwrap().into(),
        connect_fn.into(),
    );

    let send_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as IrcId;
            let line = args.get(1).to_rust_string_lossy(scope);

            let mut reg = irc_registry().lock().unwrap();
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
        v8::String::new(scope, "__ircSend").unwrap().into(),
        send_fn.into(),
    );

    let read_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as IrcId;
            let max_bytes = args.get(1).uint32_value(scope).unwrap_or(65536) as usize;
            let max = max_bytes.min(65536);

            let mut buf = vec![0u8; max];
            let mut reg = irc_registry().lock().unwrap();
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
        v8::String::new(scope, "__ircRead").unwrap().into(),
        read_fn.into(),
    );

    let close_fn = v8::Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as IrcId;
            if let Some(mut conn) = irc_registry().lock().unwrap().remove(&id) {
                conn.shutdown();
            }
            rv.set(v8::Boolean::new(scope, true).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ircClose").unwrap().into(),
        close_fn.into(),
    );

    let js_code = r#"
    (function() {
        function Client(serverOrOptions, nickArg, options) {
            var opts = (serverOrOptions && typeof serverOrOptions === 'object')
                ? serverOrOptions : (options || {});
            var server = (typeof serverOrOptions === 'string') ? serverOrOptions
                : (opts.host || opts.server || 'localhost');
            var nickname = (typeof nickArg === 'string') ? nickArg : (opts.nick || 'user');
            this.server = server;
            this.port = opts.port || 6667;
            this.nickname = nickname;
            this.username = opts.username || nickname;
            this.realname = opts.realname || nickname;
            this.password = opts.password || null;
            this.secure = opts.secure || opts.tls || false;
            this.channels = opts.channels || [];
            this._id = __ircCreate(server, this.port, nickname);
            this._connected = false;
            this._lineBuffer = '';
            this._pollTimer = null;
            this._handlers = {};
        }

        Client.prototype.connect = function(callback) {
            var self = this;
            if (callback) self.on('registered', callback);
            setTimeout(function() {
                try {
                    __ircConnect(self._id, self.server, self.port, self.secure);
                    self._connected = true;
                    self._startPoll();
                    self.emit('connect');
                    if (self.password) self.send('PASS ' + self.password);
                    self.send('NICK ' + self.nickname);
                    self.send('USER ' + self.username + ' 0 * :' + self.realname);
                } catch (e) {
                    self.emit('error', e);
                }
            }, 0);
        };

        Client.prototype._startPoll = function() {
            var self = this;
            var delay = 1;
            function poll() {
                if (!self._connected) { self._pollTimer = null; return; }
                try {
                    var chunk = __ircRead(self._id, 65536);
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

        Client.prototype._handleLine = function(line) {
            this.emit('raw', line);
            var prefix = null;
            if (line.charAt(0) === ':') {
                var sp = line.indexOf(' ');
                prefix = line.slice(1, sp);
                line = line.slice(sp + 1);
            }
            var trailing = null;
            var tidx = line.indexOf(' :');
            if (tidx >= 0) {
                trailing = line.slice(tidx + 2);
                line = line.slice(0, tidx);
            } else if (line.charAt(0) === ':') {
                trailing = line.slice(1);
                line = '';
            }
            var parts = line.length ? line.split(' ') : [];
            var command = (parts.shift() || '').toUpperCase();
            if (trailing !== null) parts.push(trailing);
            var nick = prefix ? prefix.split('!')[0] : null;

            if (command === 'PING') {
                this.send('PONG :' + (trailing || parts[0] || ''));
                this.emit('ping', trailing);
            } else if (command === 'PRIVMSG') {
                this.emit('message', nick, parts[0], parts[1], { nick: nick, prefix: prefix });
                if (parts[0] === this.nickname) this.emit('pm', nick, parts[1]);
            } else if (command === 'NOTICE') {
                this.emit('notice', nick, parts[0], parts[1]);
            } else if (command === 'JOIN') {
                this.emit('join', parts[0] || trailing, nick);
            } else if (command === 'PART') {
                this.emit('part', parts[0], nick, parts[1]);
            } else if (command === 'QUIT') {
                this.emit('quit', nick, trailing);
            } else if (command === '001') {
                this.emit('registered', trailing);
            } else if (command === 'ERROR') {
                this.emit('error', new Error(trailing || 'ERROR'));
            }
        };

        Client.prototype.send = Client.prototype.raw = function(line, callback) {
            if (this._connected) {
                __ircSend(this._id, line);
            }
            if (callback) callback();
        };

        Client.prototype.nick = function(newnick, callback) {
            this.nickname = newnick;
            if (this._connected) this.send("NICK " + newnick);
            if (callback) callback();
        };

        Client.prototype.user = function(username, mode, realname, callback) {
            if (this._connected) this.send("USER " + username + " " + (mode||0) + " * :" + (realname||username));
            if (callback) callback();
        };

        Client.prototype.join = function(channel, key, callback) {
            var line = "JOIN " + channel;
            if (typeof key === 'string') line += " " + key;
            if (this._connected) this.send(line);
            if (typeof key === 'function') key();
            else if (callback) callback();
        };

        Client.prototype.part = function(channel, reason, callback) {
            var line = "PART " + channel;
            if (typeof reason === 'string') line += " :" + reason;
            if (this._connected) this.send(line);
            if (typeof reason === 'function') reason();
            else if (callback) callback();
        };

        Client.prototype.privmsg = function(target, text, callback) {
            if (this._connected) this.send("PRIVMSG " + target + " :" + text);
            if (callback) callback();
        };

        Client.prototype.notice = function(target, text, callback) {
            if (this._connected) this.send("NOTICE " + target + " :" + text);
            if (callback) callback();
        };

        Client.prototype.kick = function(channel, nick, reason, callback) {
            var line = "KICK " + channel + " " + nick;
            if (typeof reason === 'string') line += " :" + reason;
            if (this._connected) this.send(line);
            if (typeof reason === 'function') reason();
            else if (callback) callback();
        };

        Client.prototype.mode = function(target, mode, callback) {
            if (this._connected) this.send("MODE " + target + (mode ? " " + mode : ""));
            if (callback) callback();
        };

        Client.prototype.quit = function(reason, callback) {
            var line = "QUIT";
            if (typeof reason === 'string') line += " :" + reason;
            if (this._connected) { this.send(line); }
            this._connected = false;
            __ircClose(this._id);
            if (typeof reason === 'function') reason();
            else if (callback) callback();
        };

        Client.prototype.disconnect = function() {
            __ircClose(this._id);
            this._connected = false;
        };

        Client.prototype.on = Client.prototype.addListener = function(event, handler) {
            this._handlers[event] = this._handlers[event] || [];
            this._handlers[event].push(handler);
            return this;
        };

        Client.prototype.off = Client.prototype.removeListener = function(event, handler) {
            if (this._handlers[event]) {
                var idx = this._handlers[event].indexOf(handler);
                if (idx >= 0) this._handlers[event].splice(idx, 1);
            }
            return this;
        };

        Client.prototype.removeAllListeners = function(event) {
            if (event) this._handlers[event] = [];
            else this._handlers = {};
        };

        Client.prototype.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            (this._handlers[event] || []).forEach(function(h) { h.apply(null, args); });
        };

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['irc'] = { Client: Client };
        globalThis.__requireCache['node:irc'] = { Client: Client };
        globalThis.irc = { Client: Client };
    })();
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }
}
