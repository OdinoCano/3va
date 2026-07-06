//! IRC (Internet Relay Chat) client built-in module
//!
//! Provides: `require('irc')` with `Client` class, backed by a real TCP (or TLS)
//! socket — connect/send/read all do genuine network I/O over the IRC line
//! protocol (RFC 1459/2812), following the same connection-pool + non-blocking
//! poll pattern as `tcp.rs`'s `net.Socket`.
//!
//! Native functions:
//! - `__ircCreate(server, port, nick)` -> id
//! - `__ircConnect(id, host, port, useTls)` -> throws on failure
//! - `__ircSend(id, line)` -> throws on failure
//! - `__ircRead(id, maxBytes)` -> Vec<u8> | throws EAGAIN | throws EOF
//! - `__ircClose(id)`

use native_tls::TlsStream;
use rquickjs::{Ctx, Function, Result, function::Rest};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
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

pub fn inject_irc(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // __ircCreate(server, port, nick) -> id
    // The id is just a registry slot; no socket exists until __ircConnect.
    let create_fn = Function::new(
        ctx.clone(),
        move |_server: String, _port: u16, _nick: String| -> IrcId { next_irc_id() },
    )?;
    globals.set("__ircCreate", create_fn)?;

    // __ircConnect(id, host, port, useTls) -> undefined | throws
    let perms = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: IrcId, host: String, port: u16, use_tls: bool| -> Result<()> {
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
                IrcConn::Tls(tls)
            } else {
                tcp.set_nonblocking(true)
                    .map_err(|e| js_code_err(&ctx, "EIO", e.to_string()))?;
                IrcConn::Plain(tcp)
            };

            irc_registry().lock().unwrap().insert(id, conn);
            Ok(())
        },
    )?;
    globals.set("__ircConnect", connect_fn)?;

    // __ircSend(id, line) -> undefined | throws — appends the mandatory CRLF.
    let send_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: IrcId, line: String| -> Result<()> {
            let mut reg = irc_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;
            conn.write_all(format!("{line}\r\n").as_bytes())
                .map_err(|e| js_code_err(&ctx, "EPIPE", e.to_string()))
        },
    )?;
    globals.set("__ircSend", send_fn)?;

    // __ircRead(id, maxBytes) -> Vec<u8> | throws EAGAIN | throws EOF
    let read_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: IrcId, max_bytes: u32| -> Result<Vec<u8>> {
            let max = (max_bytes as usize).min(65536);
            let mut buf = vec![0u8; max];
            let mut reg = irc_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;

            match conn.read(&mut buf) {
                Ok(0) => Err(js_code_err(&ctx, "EOF", "connection closed".into())),
                Ok(n) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(ref e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    Err(js_code_err(&ctx, "EAGAIN", "no data available".into()))
                }
                Err(e) => Err(js_code_err(&ctx, "EIO", e.to_string())),
            }
        },
    )?;
    globals.set("__ircRead", read_fn)?;

    // __ircClose(id)
    let close_fn = Function::new(ctx.clone(), move |_args: Rest<u32>| -> bool {
        if let Some(id) = _args.0.into_iter().next()
            && let Some(mut conn) = irc_registry().lock().unwrap().remove(&id)
        {
            conn.shutdown();
        }
        true
    })?;
    globals.set("__ircClose", close_fn)?;

    ctx.eval::<(), _>(r#"
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

        // Parses one IRC protocol line per RFC 2812 §2.3.1:
        // [':' prefix SPACE] command [params] ['SPACE ':' trailing]
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
    "#)?;

    Ok(())
}
