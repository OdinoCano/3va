//! FTP (File Transfer Protocol) client built-in module
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
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

static FTP_PERMISSIONS: std::sync::OnceLock<Arc<PermissionState>> = std::sync::OnceLock::new();
fn permissions() -> &'static Arc<PermissionState> {
    FTP_PERMISSIONS.get().unwrap()
}
use crate::builtins::v8_compat::{uint8array_from_bytes, uint8array_to_vec};

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

pub fn inject_ftp(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    permissions_param: Arc<PermissionState>,
) {
    let context = scope.get_current_context();
    let global = context.global(scope);
    FTP_PERMISSIONS.set(permissions_param).ok();

    let create_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              _args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = next_ftp_id();
            ftp_registry().lock().unwrap().insert(
                id,
                FtpState {
                    control: None,
                    data: None,
                    use_tls: false,
                },
            );
            rv.set(v8::Number::new(_scope, id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpCreate").unwrap().into(),
        create_fn.into(),
    );

    let connect_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as FtpId;
            let host = args.get(1).to_rust_string_lossy(_scope);
            let port = args.get(2).uint32_value(_scope).unwrap_or(21) as u16;
            let use_tls = args.get(3).boolean_value(_scope);

            if !permissions().check(&Capability::Network(host.clone())) {
                let msg = v8::String::new(
                    _scope,
                    &format!("Network access denied. Run with --allow-net={}", host),
                )
                .unwrap();
                let err = v8::Exception::error(_scope, msg);
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
                                            FtpConn::Tls(tls)
                                        } else if let Some(tcp) = fallback {
                                            FtpConn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                    Err(_) => {
                                        if let Some(tcp) = fallback {
                                            FtpConn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                }
                            }
                            Err(_) => FtpConn::Plain(tcp),
                        }
                    } else {
                        let _ = tcp.set_nonblocking(true);
                        FtpConn::Plain(tcp)
                    };

                    let mut reg = ftp_registry().lock().unwrap();
                    if let Some(state) = reg.get_mut(&id) {
                        state.control = Some(conn);
                        state.use_tls = use_tls;
                    }
                    rv.set(v8::undefined(_scope).into());
                }
                Err(e) => {
                    let msg =
                        v8::String::new(_scope, &format!("Connection failed: {}", e)).unwrap();
                    let err = v8::Exception::error(_scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpConnect").unwrap().into(),
        connect_fn.into(),
    );

    let send_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as FtpId;
            let line = args.get(1).to_rust_string_lossy(_scope);

            let mut reg = ftp_registry().lock().unwrap();
            match reg.get_mut(&id).and_then(|s| s.control.as_mut()) {
                Some(conn) => match conn.write_all(format!("{}\r\n", line).as_bytes()) {
                    Ok(_) => rv.set(v8::undefined(_scope).into()),
                    Err(e) => {
                        let msg = v8::String::new(_scope, &e.to_string()).unwrap();
                        let err = v8::Exception::error(_scope, msg);
                        rv.set(err);
                    }
                },
                None => {
                    let msg = v8::String::new(_scope, "not connected").unwrap();
                    let err = v8::Exception::error(_scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpSend").unwrap().into(),
        send_fn.into(),
    );

    let read_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as FtpId;
            let max_bytes = args.get(1).uint32_value(_scope).unwrap_or(65536) as usize;
            let max = max_bytes.min(65536);

            let mut buf = vec![0u8; max];
            let mut reg = ftp_registry().lock().unwrap();
            match reg.get_mut(&id).and_then(|s| s.control.as_mut()) {
                Some(conn) => match conn.read(&mut buf) {
                    Ok(0) => {
                        let msg = v8::String::new(_scope, "connection closed").unwrap();
                        let err = v8::Exception::error(_scope, msg);
                        rv.set(err);
                    }
                    Ok(n) => {
                        buf.truncate(n);
                        rv.set(uint8array_from_bytes(_scope, &buf).into());
                    }
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        let msg = v8::String::new(_scope, "no data available").unwrap();
                        let err = v8::Exception::error(_scope, msg);
                        rv.set(err);
                    }
                    Err(e) => {
                        let msg = v8::String::new(_scope, &e.to_string()).unwrap();
                        let err = v8::Exception::error(_scope, msg);
                        rv.set(err);
                    }
                },
                None => {
                    let msg = v8::String::new(_scope, "not connected").unwrap();
                    let err = v8::Exception::error(_scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpRead").unwrap().into(),
        read_fn.into(),
    );

    let data_connect_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as FtpId;
            let host = args.get(1).to_rust_string_lossy(_scope);
            let port = args.get(2).uint32_value(_scope).unwrap_or(0) as u16;

            if !permissions().check(&Capability::Network(host.clone())) {
                let msg = v8::String::new(
                    _scope,
                    &format!("Network access denied. Run with --allow-net={}", host),
                )
                .unwrap();
                let err = v8::Exception::error(_scope, msg);
                rv.set(err);
                return;
            }

            match TcpStream::connect(format!("{}:{}", host, port)) {
                Ok(tcp) => {
                    let use_tls = ftp_registry()
                        .lock()
                        .unwrap()
                        .get(&id)
                        .map(|s| s.use_tls)
                        .unwrap_or(false);
                    let conn = if use_tls {
                        match native_tls::TlsConnector::new() {
                            Ok(connector) => {
                                let fallback = tcp.try_clone().ok();
                                match connector.connect(&host, tcp) {
                                    Ok(tls) => {
                                        if tls.get_ref().set_nonblocking(true).is_ok() {
                                            FtpConn::Tls(tls)
                                        } else if let Some(tcp) = fallback {
                                            FtpConn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                    Err(_) => {
                                        if let Some(tcp) = fallback {
                                            FtpConn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                }
                            }
                            Err(_) => FtpConn::Plain(tcp),
                        }
                    } else {
                        let _ = tcp.set_nonblocking(true);
                        FtpConn::Plain(tcp)
                    };

                    let mut reg = ftp_registry().lock().unwrap();
                    if let Some(state) = reg.get_mut(&id) {
                        state.data = Some(conn);
                    }
                    rv.set(v8::undefined(_scope).into());
                }
                Err(e) => {
                    let msg =
                        v8::String::new(_scope, &format!("Connection failed: {}", e)).unwrap();
                    let err = v8::Exception::error(_scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpDataConnect").unwrap().into(),
        data_connect_fn.into(),
    );

    let data_read_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as FtpId;
            let max_bytes = args.get(1).uint32_value(_scope).unwrap_or(65536) as usize;
            let max = max_bytes.min(65536);

            let mut buf = vec![0u8; max];
            let mut reg = ftp_registry().lock().unwrap();
            match reg.get_mut(&id).and_then(|s| s.data.as_mut()) {
                Some(conn) => match conn.read(&mut buf) {
                    Ok(0) => {
                        let msg = v8::String::new(_scope, "data connection closed").unwrap();
                        let err = v8::Exception::error(_scope, msg);
                        rv.set(err);
                    }
                    Ok(n) => {
                        buf.truncate(n);
                        rv.set(uint8array_from_bytes(_scope, &buf).into());
                    }
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        let msg = v8::String::new(_scope, "no data available").unwrap();
                        let err = v8::Exception::error(_scope, msg);
                        rv.set(err);
                    }
                    Err(e) => {
                        let msg = v8::String::new(_scope, &e.to_string()).unwrap();
                        let err = v8::Exception::error(_scope, msg);
                        rv.set(err);
                    }
                },
                None => {
                    let msg = v8::String::new(_scope, "not connected").unwrap();
                    let err = v8::Exception::error(_scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpDataRead").unwrap().into(),
        data_read_fn.into(),
    );

    let data_write_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as FtpId;
            let data = v8::Local::<v8::Uint8Array>::try_from(args.get(1))
                .map(|arr| uint8array_to_vec(_scope, arr))
                .unwrap_or_default();

            let mut reg = ftp_registry().lock().unwrap();
            match reg.get_mut(&id).and_then(|s| s.data.as_mut()) {
                Some(conn) => match conn.write_all(&data) {
                    Ok(_) => rv.set(v8::undefined(_scope).into()),
                    Err(e) => {
                        let msg = v8::String::new(_scope, &e.to_string()).unwrap();
                        let err = v8::Exception::error(_scope, msg);
                        rv.set(err);
                    }
                },
                None => {
                    let msg = v8::String::new(_scope, "not connected").unwrap();
                    let err = v8::Exception::error(_scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpDataWrite").unwrap().into(),
        data_write_fn.into(),
    );

    let data_close_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as FtpId;
            if let Some(state) = ftp_registry().lock().unwrap().get_mut(&id)
                && let Some(mut conn) = state.data.take()
            {
                conn.shutdown();
            }
            rv.set(v8::Boolean::new(_scope, true).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpDataClose").unwrap().into(),
        data_close_fn.into(),
    );

    let close_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as FtpId;
            if let Some(mut state) = ftp_registry().lock().unwrap().remove(&id) {
                if let Some(mut conn) = state.control.take() {
                    let _ = conn.write_all(b"QUIT\r\n");
                    conn.shutdown();
                }
                if let Some(mut conn) = state.data.take() {
                    conn.shutdown();
                }
            }
            rv.set(v8::Boolean::new(_scope, true).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__ftpClose").unwrap().into(),
        close_fn.into(),
    );

    let js_code = r#"
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
                                var parts = line.match(/^([drwx\-]{10})\s +\d+\s +\S+\s +\S+\s +(\d+)\s +\w+\s +[\d\s:]+[\d]\s +(.*)$/);
                                if (parts) {
                                    return { name: parts[3], size: parseInt(parts[2]), isDirectory: parts[1][0] === 'd' };
                                }
                                return { name: line, size: 0, isDirectory: false };
                            });
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
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }
}
