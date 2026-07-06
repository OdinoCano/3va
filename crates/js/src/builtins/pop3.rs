//! POP3 (Post Office Protocol v3) client built-in module
//!
//! Provides: `require('pop3')` with `Client` class, backed by a real TCP (or TLS)
//! socket — connect/send/read all do genuine network I/O over the POP3 line
//! protocol (RFC 1939), following the same connection-pool + non-blocking
//! poll pattern as `tcp.rs`'s `net.Socket`.
//!
//! Native functions:
//! - `__pop3Create()` -> id
//! - `__pop3Connect(id, host, port, useTls)` -> throws on failure
//! - `__pop3Send(id, line)` -> throws on failure
//! - `__pop3Read(id, maxBytes)` -> Vec<u8> | throws EAGAIN | throws EOF
//! - `__pop3Close(id)`

use native_tls::TlsStream;
use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
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

fn js_code_err(ctx: &Ctx<'_>, code: &str, msg: String) -> rquickjs::Error {
    let escaped_msg = msg.replace('\\', "\\\\").replace('"', "\\\"");
    let src = format!(
        "(function(){{var e=new Error(\"{msg}\");e.code=\"{code}\";return e;}})()",
        msg = escaped_msg,
        code = code
    );
    match ctx.eval::<rquickjs::Value<'_>, _>(src) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

pub fn inject_pop3(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // __pop3Create() -> id
    let create_fn = Function::new(ctx.clone(), move || -> Pop3Id { next_pop3_id() })?;
    globals.set("__pop3Create", create_fn)?;

    // __pop3Connect(id, host, port, useTls) -> undefined | throws
    let perms = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: Pop3Id, host: String, port: u16, use_tls: bool| -> Result<()> {
            if !perms.check(&Capability::Network(host.clone())) {
                return Err(js_code_err(
                    &ctx,
                    "EACCES",
                    format!("Network access denied. Run with --allow-net={}", host),
                ));
            }

            let tcp = TcpStream::connect(format!("{host}:{port}"))
                .map_err(|e| js_code_err(&ctx, "ECONNREFUSED", e.to_string()))?;

            let conn = if use_tls {
                let connector = native_tls::TlsConnector::new()
                    .map_err(|e| js_code_err(&ctx, "EIO", format!("TLS init failed: {e}")))?;
                let tls = connector.connect(&host, tcp).map_err(|e| {
                    js_code_err(&ctx, "ECONNRESET", format!("TLS handshake failed: {e}"))
                })?;
                tls.get_ref()
                    .set_nonblocking(true)
                    .map_err(|e| js_code_err(&ctx, "EIO", e.to_string()))?;
                Pop3Conn::Tls(tls)
            } else {
                tcp.set_nonblocking(true)
                    .map_err(|e| js_code_err(&ctx, "EIO", e.to_string()))?;
                Pop3Conn::Plain(tcp)
            };

            pop3_registry().lock().unwrap().insert(id, conn);
            Ok(())
        },
    )?;
    globals.set("__pop3Connect", connect_fn)?;

    // __pop3Send(id, line) -> throws on failure
    let send_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: Pop3Id, line: String| -> Result<()> {
            let mut reg = pop3_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;
            conn.write_all(format!("{line}\r\n").as_bytes())
                .map_err(|e| js_code_err(&ctx, "EPIPE", e.to_string()))
        },
    )?;
    globals.set("__pop3Send", send_fn)?;

    // __pop3Read(id, maxBytes) -> Vec<u8> | throws EAGAIN | throws EOF
    let read_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: Pop3Id, max_bytes: u32| -> Result<Vec<u8>> {
            let max = (max_bytes as usize).min(65536);
            let mut buf = vec![0u8; max];
            let mut reg = pop3_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;

            match conn.read(&mut buf) {
                Ok(0) => Err(js_code_err(&ctx, "EOF", "connection closed".into())),
                Ok(n) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    Err(js_code_err(&ctx, "EAGAIN", "no data available".into()))
                }
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    Err(js_code_err(&ctx, "EAGAIN", "no data available".into()))
                }
                Err(e) => Err(js_code_err(&ctx, "EIO", e.to_string())),
            }
        },
    )?;
    globals.set("__pop3Read", read_fn)?;

    // __pop3Close(id)
    let close_fn = Function::new(ctx.clone(), move |id: Pop3Id| -> bool {
        if let Some(mut conn) = pop3_registry().lock().unwrap().remove(&id) {
            conn.shutdown();
        }
        true
    })?;
    globals.set("__pop3Close", close_fn)?;

    ctx.eval::<(), _>(
        r#"
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
                    // The server sends its greeting banner unsolicited, right
                    // after accept — wait for it (marking pending state with
                    // nothing to send) so it can't be misread as the response
                    // to whatever command runs first (e.g. login()'s USER).
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

        // RFC 1939 §3: the status line (+OK/-ERR) is the first line of every
        // response. Only RETR/TOP and argument-less LIST/UIDL continue with a
        // multiline body terminated by a lone '.' — STAT, DELE, RSET, NOOP, and
        // LIST/UIDL with a specific message number resolve on that first line.
        Client.prototype._handleLine = function(line) {
            this.emit('raw', line);
            if (line.indexOf('+OK') === 0 || line.indexOf('+ ') === 0) {
                this.emit('response', line);
                if (this._pendingCmd) {
                    if (!this._pendingMultiline) {
                        this._resolvePending(null, line);
                    }
                    // else: status line for a multiline response — the body
                    // follows as subsequent non-status lines (see below).
                }
            } else if (line.indexOf('-ERR') === 0) {
                this.emit('error', new Error(line));
                if (this._pendingCmd) this._resolvePending(new Error(line));
            } else if (this._pendingCmd && this._pendingMultiline) {
                if (line === '.') {
                    this._resolvePending(null, this._pendingData || '');
                } else {
                    // Undo RFC 1939 dot-stuffing: a body line that legitimately
                    // starts with '.' is sent on the wire as '..'.
                    var body = line.indexOf('..') === 0 ? line.slice(1) : line;
                    this._pendingData = (this._pendingData || '') + body + '\n';
                }
            }
        };

        // Sends one command and waits for its real single-line +OK/-ERR before
        // resolving — used by login() so USER/PASS are properly lock-stepped
        // instead of firing both before either response is read, which used to
        // let unrelated buffered lines (the server greeting, USER's own +OK)
        // get misattributed to whatever command ran next (e.g. stat()).
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
            // A specific message number gets a single-line reply; no argument
            // lists every message and gets a multiline reply (RFC 1939 §7).
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
    "#,
    )?;

    Ok(())
}
