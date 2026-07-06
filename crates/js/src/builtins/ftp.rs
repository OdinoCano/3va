//! FTP (File Transfer Protocol) client built-in module
//!
//! Provides: `require('ftp')` with `Client` class, backed by real TCP (or TLS)
//! sockets for both control and data connections (RFC 959).
//!
//! Control-connection commands (USER/PASS/CWD/PWD/PASV/MKD/...) are all
//! fire-and-forget natives; response parsing and per-command correlation
//! happens JS-side via the same non-blocking poll + `_pendingCmd`/
//! `_pendingCallback` pattern used by `pop3.rs`, instead of blocking the
//! whole engine in a native retry loop waiting for the server.
//!
//! Native functions:
//! - `__ftpCreate()` -> id
//! - `__ftpConnect(id, host, port, useTls)` -> throws on failure
//! - `__ftpSend(id, line)` -> throws on failure
//! - `__ftpRead(id, maxBytes)` -> Vec<u8> | throws EAGAIN | throws EOF
//! - `__ftpClose(id)`
//! - `__ftpDataConnect(id, host, port)` -> throws on failure
//! - `__ftpDataRead(id, maxBytes)` -> Vec<u8> | throws EAGAIN | throws EOF
//! - `__ftpDataWrite(id, data)` -> throws on failure
//! - `__ftpDataClose(id)`

use native_tls::TlsStream;
use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

type FtpId = u32;

#[allow(clippy::large_enum_variant)]
enum FtpConn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl FtpConn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            FtpConn::Plain(s) => s.read(buf),
            FtpConn::Tls(s) => s.read(buf),
        }
    }
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            FtpConn::Plain(s) => s.write_all(data),
            FtpConn::Tls(s) => s.write_all(data),
        }
    }
    fn shutdown(&mut self) {
        match self {
            FtpConn::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            FtpConn::Tls(s) => {
                let _ = s.shutdown();
            }
        }
    }
}

struct FtpState {
    control: Option<FtpConn>,
    data: Option<FtpConn>,
    use_tls: bool,
}

static FTP_REGISTRY: OnceLock<Mutex<HashMap<FtpId, FtpState>>> = OnceLock::new();

fn ftp_registry() -> &'static Mutex<HashMap<FtpId, FtpState>> {
    FTP_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_ftp_id() -> FtpId {
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

pub fn inject_ftp(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // __ftpCreate() -> id
    let create_fn = Function::new(ctx.clone(), move || -> FtpId {
        let id = next_ftp_id();
        ftp_registry().lock().unwrap().insert(
            id,
            FtpState {
                control: None,
                data: None,
                use_tls: false,
            },
        );
        id
    })?;
    globals.set("__ftpCreate", create_fn)?;

    // __ftpConnect(id, host, port, useTls) -> undefined | throws
    let perms = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: FtpId, host: String, port: u16, use_tls: bool| -> Result<()> {
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
                FtpConn::Tls(tls)
            } else {
                tcp.set_nonblocking(true)
                    .map_err(|e| js_code_err(&ctx, "EIO", e.to_string()))?;
                FtpConn::Plain(tcp)
            };

            let mut reg = ftp_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.control = Some(conn);
                state.use_tls = use_tls;
                Ok(())
            } else {
                Err(js_code_err(&ctx, "ENOTCONN", "Invalid FTP ID".to_string()))
            }
        },
    )?;
    globals.set("__ftpConnect", connect_fn)?;

    // __ftpSend(id, line) -> throws on failure
    let send_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: FtpId, line: String| -> Result<()> {
            let mut reg = ftp_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .and_then(|s| s.control.as_mut())
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;
            conn.write_all(format!("{line}\r\n").as_bytes())
                .map_err(|e| js_code_err(&ctx, "EPIPE", e.to_string()))
        },
    )?;
    globals.set("__ftpSend", send_fn)?;

    // __ftpRead(id, maxBytes) -> Vec<u8> | throws EAGAIN | throws EOF
    let read_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: FtpId, max_bytes: u32| -> Result<Vec<u8>> {
            let max = (max_bytes as usize).min(65536);
            let mut buf = vec![0u8; max];
            let mut reg = ftp_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .and_then(|s| s.control.as_mut())
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
    globals.set("__ftpRead", read_fn)?;

    // __ftpDataConnect(id, host, port) -> throws on failure
    let perms_data = permissions.clone();
    let data_connect_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: FtpId, host: String, port: u16| -> Result<()> {
            if !perms_data.check(&Capability::Network(host.clone())) {
                return Err(js_code_err(
                    &ctx,
                    "EACCES",
                    format!("Network access denied. Run with --allow-net={}", host),
                ));
            }

            let tcp = TcpStream::connect(format!("{host}:{port}"))
                .map_err(|e| js_code_err(&ctx, "ECONNREFUSED", e.to_string()))?;

            let conn = if ftp_registry()
                .lock()
                .unwrap()
                .get(&id)
                .map(|s| s.use_tls)
                .unwrap_or(false)
            {
                let connector = native_tls::TlsConnector::new()
                    .map_err(|e| js_code_err(&ctx, "EIO", format!("TLS init failed: {e}")))?;
                let tls = connector.connect(&host, tcp).map_err(|e| {
                    js_code_err(&ctx, "ECONNRESET", format!("TLS handshake failed: {e}"))
                })?;
                tls.get_ref()
                    .set_nonblocking(true)
                    .map_err(|e| js_code_err(&ctx, "EIO", e.to_string()))?;
                FtpConn::Tls(tls)
            } else {
                tcp.set_nonblocking(true)
                    .map_err(|e| js_code_err(&ctx, "EIO", e.to_string()))?;
                FtpConn::Plain(tcp)
            };

            let mut reg = ftp_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.data = Some(conn);
                Ok(())
            } else {
                Err(js_code_err(&ctx, "ENOTCONN", "Invalid FTP ID".to_string()))
            }
        },
    )?;
    globals.set("__ftpDataConnect", data_connect_fn)?;

    // __ftpDataRead(id, maxBytes) -> Vec<u8> | throws EAGAIN | throws EOF
    let data_read_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: FtpId, max_bytes: u32| -> Result<Vec<u8>> {
            let max = (max_bytes as usize).min(65536);
            let mut buf = vec![0u8; max];
            let mut reg = ftp_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .and_then(|s| s.data.as_mut())
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;

            match conn.read(&mut buf) {
                Ok(0) => Err(js_code_err(&ctx, "EOF", "data connection closed".into())),
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
    globals.set("__ftpDataRead", data_read_fn)?;

    // __ftpDataWrite(id, data) -> throws on failure
    let data_write_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: FtpId, data: Vec<u8>| -> Result<()> {
            let mut reg = ftp_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .and_then(|s| s.data.as_mut())
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;
            conn.write_all(&data)
                .map_err(|e| js_code_err(&ctx, "EPIPE", e.to_string()))
        },
    )?;
    globals.set("__ftpDataWrite", data_write_fn)?;

    // __ftpDataClose(id)
    let data_close_fn = Function::new(ctx.clone(), move |id: FtpId| -> bool {
        if let Some(state) = ftp_registry().lock().unwrap().get_mut(&id)
            && let Some(mut conn) = state.data.take()
        {
            conn.shutdown();
        }
        true
    })?;
    globals.set("__ftpDataClose", data_close_fn)?;

    // __ftpClose(id)
    let close_fn = Function::new(ctx.clone(), move |id: FtpId| -> bool {
        if let Some(mut state) = ftp_registry().lock().unwrap().remove(&id) {
            if let Some(mut conn) = state.control.take() {
                let _ = conn.write_all(b"QUIT\r\n");
                conn.shutdown();
            }
            if let Some(mut conn) = state.data.take() {
                conn.shutdown();
            }
        }
        true
    })?;
    globals.set("__ftpClose", close_fn)?;

    ctx.eval::<(), _>(
        r#"
    (function() {
        function Client() {
            this._id = __ftpCreate();
            this._connected = false;
            this._authenticated = false;
            this._host = '';
            this._port = 21;
            this._tls = false;
            this._usePassive = true;
            this._lineBuffer = '';
            this._pollTimer = null;
            this._handlers = {};
            this._cwd = '/';
            this.timeout = 30000;
            this._pendingCmd = null;
            this._pendingCallback = null;
            this._pendingData = null;
            this._pendingMultiline = false;
            this._pendingCode = null;
            this._pendingTimer = null;
        }

        Client.prototype.connect = function(options, callback) {
            var self = this;
            options = options || {};
            this._host = options.host || 'localhost';
            this._port = options.port || (options.secure ? 990 : 21);
            this._tls = options.secure || false;
            this._username = options.username || 'anonymous';
            this._password = options.password || '';
            this._usePassive = options.usePassive !== false;
            this.timeout = options.timeout || 30000;

            if (callback) self.on('connect', callback);
            setTimeout(function() {
                try {
                    __ftpConnect(self._id, self._host, self._port, self._tls);
                    self._connected = true;
                    self._startPoll();
                    // The server sends its greeting (220) unsolicited, right
                    // after accept — wait for it before sending anything, or
                    // it can race with (and be misattributed to) the first
                    // real command, same class of bug fixed in pop3.rs.
                    self._awaitReply(false, function(err) {
                        if (err) { self.emit('error', err); return; }
                        self.emit('connect');
                        if (self._username || self._password) {
                            self.login(self._username, self._password);
                        }
                    });
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
                    var chunk = __ftpRead(self._id, 65536);
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

        // Marks the client as waiting for the next control-connection reply,
        // resolving `_pendingCallback` from `_handleLine` once it arrives (or
        // from the timer below if the server never answers — this replaces
        // the old native retry loop that blocked the *entire* engine with no
        // timeout at all when a server went quiet).
        Client.prototype._awaitReply = function(multiline, callback) {
            var self = this;
            this._pendingCmd = true;
            this._pendingCallback = callback;
            this._pendingData = '';
            this._pendingMultiline = multiline;
            this._pendingCode = null;
            if (this._pendingTimer) clearTimeout(this._pendingTimer);
            this._pendingTimer = setTimeout(function() {
                if (self._pendingCallback === callback) {
                    self._resolvePending(Object.assign(new Error('FTP control reply timed out'), { code: 'ETIMEDOUT' }));
                }
            }, this.timeout || 30000);
        };

        Client.prototype._resolvePending = function(err, data) {
            if (this._pendingTimer) { clearTimeout(this._pendingTimer); this._pendingTimer = null; }
            var cb = this._pendingCallback;
            this._pendingCmd = null;
            this._pendingCallback = null;
            this._pendingData = null;
            this._pendingMultiline = false;
            this._pendingCode = null;
            if (cb) cb(err, data);
        };

        // Sends one command and waits for its real reply (RFC 959 §4.2:
        // "CODE-text" starts/continues a multiline reply, "CODE text" ends it
        // or is a complete single-line reply on its own).
        Client.prototype._sendAwait = function(cmd, multiline, callback) {
            this._awaitReply(multiline, callback);
            try {
                __ftpSend(this._id, cmd);
            } catch (e) {
                this._resolvePending(e);
            }
        };

        Client.prototype._handleLine = function(line) {
            this.emit('raw', line);
            var m = line.match(/^(\d{3})([ -])(.*)$/);
            if (!m) {
                if (this._pendingCmd && this._pendingMultiline && this._pendingCode) {
                    this._pendingData += line + '\n';
                }
                return;
            }
            var code = m[1], sep = m[2], text = m[3];
            var isError = code.charAt(0) === '4' || code.charAt(0) === '5';

            if (!this._pendingCmd) {
                // Unsolicited line (e.g. the async "226 Transfer complete"
                // that follows a data transfer well after its control reply
                // already resolved). Kept as events for compatibility.
                if (code === '226') this.emit('dataend', line);
                else if (code === '150' || code === '125') this.emit('data', line);
                else if (isError) this.emit('error', new Error(line));
                return;
            }
            if (sep === '-') {
                this._pendingCode = code;
                this._pendingData += text + '\n';
                return;
            }
            if (this._pendingMultiline && this._pendingCode && code === this._pendingCode) {
                this._pendingData += text + '\n';
                this._resolvePending(isError ? new Error(code + ' ' + text) : null, this._pendingData);
                return;
            }
            this._resolvePending(isError ? new Error(code + ' ' + text) : null, { code: code, text: text });
        };

        Client.prototype.login = function(username, password, callback) {
            var self = this;
            if (typeof username === 'function') { callback = username; username = null; }
            if (typeof password === 'function') { callback = password; password = null; }
            this._username = username || this._username;
            this._password = password || this._password;
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            this._sendAwait('USER ' + this._username, false, function(err, reply) {
                if (err) { if (callback) callback(err); return; }
                if (reply.code === '230') {
                    // Server accepted the username alone (e.g. some anonymous setups).
                    self._authenticated = true;
                    self.emit('ready');
                    if (callback) callback();
                    return;
                }
                self._sendAwait('PASS ' + self._password, false, function(err2) {
                    if (err2) { if (callback) callback(err2); return; }
                    self._authenticated = true;
                    self.emit('ready');
                    if (callback) callback();
                });
            });
            return this;
        };

        Client.prototype.pwd = function(callback) {
            var self = this;
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'), null);
                return this;
            }
            this._sendAwait('PWD', false, function(err, reply) {
                if (err) { if (callback) callback(err, null); return; }
                var m = reply.text.match(/"((?:[^"]|"")*)"/);
                var path = m ? m[1].replace(/""/g, '"') : reply.text;
                self._cwd = path;
                if (callback) callback(null, path);
            });
            return this;
        };

        Client.prototype.cwd = function(path, callback) {
            var self = this;
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            this._sendAwait('CWD ' + path, false, function(err) {
                if (!err) self._cwd = path;
                if (callback) callback(err);
            });
            return this;
        };

        Client.prototype.mkdir = function(path, recursive, callback) {
            if (typeof recursive === 'function') { callback = recursive; }
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            this._sendAwait('MKD ' + path, false, function(err) {
                if (callback) callback(err);
            });
            return this;
        };

        Client.prototype.rmdir = function(path, recursive, callback) {
            if (typeof recursive === 'function') { callback = recursive; }
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            this._sendAwait('RMD ' + path, false, function(err) {
                if (callback) callback(err);
            });
            return this;
        };

        // Requests a passive-mode data port (RFC 959 §4.1.2, PASV) and parses
        // the "227 ... (h1,h2,h3,h4,p1,p2)" reply into a {host, port} pair.
        Client.prototype._pasv = function(callback) {
            this._sendAwait('PASV', false, function(err, reply) {
                if (err) { callback(err); return; }
                var m = reply.text.match(/\((\d+),(\d+),(\d+),(\d+),(\d+),(\d+)\)/);
                if (!m) { callback(new Error('PASV: could not parse reply: ' + reply.text)); return; }
                var host = m[1] + '.' + m[2] + '.' + m[3] + '.' + m[4];
                var port = parseInt(m[5], 10) * 256 + parseInt(m[6], 10);
                callback(null, { host: host, port: port });
            });
        };

        Client.prototype.list = function(path, callback) {
            if (typeof path === 'function') { callback = path; path = undefined; }
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'), []);
                return [];
            }
            if (!this._usePassive) {
                if (callback) callback(new Error('Active mode not implemented'), []);
                return [];
            }
            var self = this;
            this._pasv(function(err, info) {
                if (err) { if (callback) callback(err, []); return; }
                try {
                    __ftpDataConnect(self._id, info.host, info.port);
                } catch (e) {
                    if (callback) callback(e, []);
                    return;
                }
                self._sendAwait('LIST' + (path ? ' ' + path : ''), false, function(err2) {
                    if (err2) { __ftpDataClose(self._id); if (callback) callback(err2, []); return; }
                    var data = '';
                    var delay = 1;
                    function readData() {
                        try {
                            var chunk = __ftpDataRead(self._id, 65536);
                            data += new TextDecoder().decode(new Uint8Array(chunk));
                            delay = 1;
                            setTimeout(readData, 0);
                        } catch (e) {
                            if (e && e.code === 'EAGAIN') {
                                delay = Math.min(delay * 2, 100);
                                setTimeout(readData, delay);
                                return;
                            }
                            __ftpDataClose(self._id);
                            var lines = data.split('\n').filter(function(l) { return l.length > 0; });
                            var items = lines.map(function(line) {
                                var parts = line.match(/^([drwx\-]{10})\s+\d+\s+\S+\s +\S+\s +(\d+)\s +\w+\s +[\d\s:]+[\d]\s +(.*)$/);
                                if (parts) {
                                    return { name: parts[3], size: parseInt(parts[2]), isDirectory: parts[1][0] === 'd' };
                                }
                                return { name: line, size: 0, isDirectory: false };
                            });
                            // Consume the trailing "226 Transfer complete" (or
                            // "426 ..." on failure) before returning control —
                            // otherwise it lingers unconsumed on the control
                            // channel and gets misattributed to whatever the
                            // next command happens to be (the same class of
                            // bug fixed in pop3.rs's login()/greeting race).
                            self._awaitReply(false, function() {
                                if (callback) callback(null, items);
                            });
                        }
                    }
                    readData();
                });
            });
            return [];
        };

        // Downloads `remotePath` over a real PASV data connection and resolves
        // with its bytes (as a Buffer if available, else a Uint8Array).
        Client.prototype.get = function(remotePath, callback) {
            var self = this;
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'), null);
                return this;
            }
            if (!this._usePassive) {
                if (callback) callback(new Error('Active mode not implemented'), null);
                return this;
            }
            this._pasv(function(err, info) {
                if (err) { if (callback) callback(err, null); return; }
                try {
                    __ftpDataConnect(self._id, info.host, info.port);
                } catch (e) {
                    if (callback) callback(e, null);
                    return;
                }
                self._sendAwait('RETR ' + remotePath, false, function(err2) {
                    if (err2) { __ftpDataClose(self._id); if (callback) callback(err2, null); return; }
                    var chunks = [];
                    var delay = 1;
                    function readData() {
                        try {
                            var chunk = __ftpDataRead(self._id, 65536);
                            chunks.push(new Uint8Array(chunk));
                            delay = 1;
                            setTimeout(readData, 0);
                        } catch (e) {
                            if (e && e.code === 'EAGAIN') {
                                delay = Math.min(delay * 2, 100);
                                setTimeout(readData, delay);
                                return;
                            }
                            __ftpDataClose(self._id);
                            if (e && e.code !== 'EOF') { if (callback) callback(e, null); return; }
                            var total = 0;
                            for (var i = 0; i < chunks.length; i++) total += chunks[i].length;
                            var buf = new Uint8Array(total);
                            var off = 0;
                            for (var i = 0; i < chunks.length; i++) { buf.set(chunks[i], off); off += chunks[i].length; }
                            var result = typeof Buffer !== 'undefined' ? Buffer.from(buf) : buf;
                            // See list()'s comment above: consume the trailing
                            // "226"/"426" control reply before returning.
                            self._awaitReply(false, function() {
                                if (callback) callback(null, result);
                            });
                        }
                    }
                    readData();
                });
            });
            return this;
        };

        // `input` is the data to upload (a string or Uint8Array/Buffer) — this
        // module only speaks the FTP protocol, it doesn't read local files;
        // callers wanting to upload a file read it themselves (e.g. via `fs`)
        // and pass the bytes here.
        Client.prototype._upload = function(cmd, input, remotePath, callback) {
            var self = this;
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            if (!this._usePassive) {
                if (callback) callback(new Error('Active mode not implemented'));
                return this;
            }
            var bytes;
            if (typeof input === 'string') {
                bytes = new TextEncoder().encode(input);
            } else if (input instanceof Uint8Array) {
                bytes = input;
            } else {
                if (callback) callback(new Error(
                    cmd + '() expects a string or Uint8Array/Buffer of data to upload, not a filesystem path'
                ));
                return this;
            }
            this._pasv(function(err, info) {
                if (err) { if (callback) callback(err); return; }
                try {
                    __ftpDataConnect(self._id, info.host, info.port);
                } catch (e) {
                    if (callback) callback(e);
                    return;
                }
                self._sendAwait(cmd + ' ' + remotePath, false, function(err2) {
                    if (err2) { __ftpDataClose(self._id); if (callback) callback(err2); return; }
                    try {
                        __ftpDataWrite(self._id, Array.from(bytes));
                    } catch (e) {
                        __ftpDataClose(self._id);
                        if (callback) callback(e);
                        return;
                    }
                    __ftpDataClose(self._id);
                    // See list()'s comment above: consume the trailing
                    // "226"/"426" control reply before returning.
                    self._awaitReply(false, function() {
                        if (callback) callback();
                    });
                });
            });
            return this;
        };

        Client.prototype.put = function(input, remotePath, callback) {
            return this._upload('STOR', input, remotePath, callback);
        };

        Client.prototype.append = function(input, remotePath, callback) {
            return this._upload('APPE', input, remotePath, callback);
        };

        Client.prototype.delete = function(path, callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            this._sendAwait('DELE ' + path, false, function(err) {
                if (callback) callback(err);
            });
            return this;
        };

        Client.prototype.rename = function(fromPath, toPath, callback) {
            var self = this;
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            this._sendAwait('RNFR ' + fromPath, false, function(err) {
                if (err) { if (callback) callback(err); return; }
                self._sendAwait('RNTO ' + toPath, false, function(err2) {
                    if (callback) callback(err2);
                });
            });
            return this;
        };

        Client.prototype.quit = function(callback) {
            if (this._connected) {
                try {
                    __ftpClose(this._id);
                } catch(e) {}
            }
            this._connected = false;
            this._authenticated = false;
            if (this._pollTimer) {
                clearTimeout(this._pollTimer);
                this._pollTimer = null;
            }
            if (callback) callback();
            return this;
        };

        Client.prototype.close = Client.prototype.quit;

        Client.prototype.site = function(command, callback) {
            if (!this._connected) {
                if (callback) callback(new Error('Not connected'));
                return this;
            }
            this._sendAwait('SITE ' + command, false, function(err) {
                if (callback) callback(err);
            });
            return this;
        };

        Client.prototype.status = function(callback) {
            var s = this._connected ? (this._authenticated ? 'Logged in' : 'Connected') : 'Disconnected';
            if (callback) callback(null, s);
            return s;
        };

        Client.prototype.systemType = function(callback) {
            if (callback) callback(null, 'UNIX');
            return 'UNIX';
        };

        Client.prototype.disconnect = Client.prototype.quit;

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
        globalThis.__requireCache['ftp'] = { Client: Client };
        globalThis.__requireCache['node:ftp'] = { Client: Client };
        globalThis.ftp = { Client: Client };
    })();
    "#,
    )?;

    Ok(())
}
