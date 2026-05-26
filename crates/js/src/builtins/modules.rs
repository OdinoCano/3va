use rquickjs::{Ctx, Function, Result, function::Rest};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use vvva_permissions::{Capability, PermissionState};

/// Inject CommonJS `require()`, `module`, `exports`, `__filename`, `__dirname` globals.
///
/// Strategy: inject a native `__readFile(path) -> String` function that handles
/// permission checks and file I/O. The JS-side `require()` wrapper handles the
/// module caching, wrapping, and evaluation — avoiding rquickjs `Value<'js>` lifetime
/// issues in closures.
pub fn inject_require(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // Initialize module cache and CommonJS globals
    ctx.eval::<(), _>(
        r#"
        globalThis.__requireCache = {};
        globalThis.module = { exports: {} };
        globalThis.exports = globalThis.module.exports;
        globalThis.__filename = '';
        globalThis.__dirname = '';
        // React Native globals
        globalThis.__DEV__ = false;
    "#,
    )?;

    // Native __readFile(resolvedAbsPath) -> String
    // Accepts an already-resolved absolute path; does permission check + optional TS transpile.
    let perms = permissions.clone();
    let read_file_fn = Function::new(ctx.clone(), move |args: Rest<String>| -> Result<String> {
        let path_str =
            args.0.into_iter().next().ok_or_else(|| {
                rquickjs::Error::new_from_js("value", "__readFile() needs a path")
            })?;

        let full_path = PathBuf::from(&path_str);

        // Permission check
        if !perms.check(&Capability::FileRead(full_path.clone())) {
            return Err(rquickjs::Error::new_from_js(
                "permission",
                "permission denied",
            ));
        }

        // Read the file
        let source = std::fs::read_to_string(&full_path)
            .map_err(|_| rquickjs::Error::new_from_js("io", "file not found"))?;

        // Transpile based on extension and content:
        // - .ts / .tsx: always strip TypeScript types (JSX enabled for .tsx)
        // - .jsx: always transform JSX
        // - .js / .mjs / .cjs: always try OXC transpilation (handles JSX and strips Flow-like annotations)
        let source = if path_str.ends_with(".tsx") || path_str.ends_with(".jsx") {
            crate::transpiler::transpile_jsx(&source)
        } else if path_str.ends_with(".ts") || path_str.ends_with(".mts") || path_str.ends_with(".cts") {
            crate::transpiler::transpile(&source)
        } else {
            crate::transpiler::transpile_js(&source)
        };

        Ok(source)
    })?;
    globals.set("__readFile", read_file_fn)?;

    // Native __resolvePath(path, basedir?) -> String
    // basedir is used as the base for relative imports (e.g. require('./lib/foo') inside a package).
    let resolve_fn = Function::new(ctx.clone(), |args: Rest<String>| -> String {
        let mut it = args.0.into_iter();
        let path_str = it.next().unwrap_or_default();
        let basedir = it.next(); // optional second arg
        resolve_path_from(&path_str, basedir.as_deref())
            .to_string_lossy()
            .to_string()
    })?;
    globals.set("__resolvePath", resolve_fn)?;

    // Register Node.js built-in module shims in the require cache by their bare names.
    // These are looked up before any file resolution, so require('util') etc. always work.
    ctx.eval::<(), _>(r#"
        (function() {
            // ── util ──────────────────────────────────────────────────────────────
            var util = {
                inherits: function(ctor, superCtor) {
                    ctor.super_ = superCtor;
                    ctor.prototype = Object.create(superCtor.prototype, {
                        constructor: { value: ctor, enumerable: false, writable: true, configurable: true }
                    });
                },
                format: function() {
                    var args = Array.prototype.slice.call(arguments);
                    if (typeof args[0] !== 'string') return args.join(' ');
                    var i = 1;
                    return args[0].replace(/%[sdjioO%]/g, function(m) {
                        if (m === '%%') return '%';
                        if (i >= args.length) return m;
                        var a = args[i++];
                        if (m === '%s') return String(a);
                        if (m === '%d') return Number(a);
                        if (m === '%j' || m === '%o' || m === '%O') { try { return JSON.stringify(a); } catch(e) { return '[Circular]'; } }
                        if (m === '%i') return parseInt(a);
                        return m;
                    });
                },
                inspect: function(obj) { try { return JSON.stringify(obj, null, 2); } catch(e) { return String(obj); } },
                promisify: function(fn) {
                    return function() {
                        var args = Array.prototype.slice.call(arguments);
                        return new Promise(function(resolve, reject) {
                            args.push(function(err, val) { if (err) reject(err); else resolve(val); });
                            fn.apply(null, args);
                        });
                    };
                },
                isBuffer: function(obj) { return obj instanceof Buffer; },
                isString: function(obj) { return typeof obj === 'string'; },
                isNumber: function(obj) { return typeof obj === 'number'; },
                isBoolean: function(obj) { return typeof obj === 'boolean'; },
                isFunction: function(obj) { return typeof obj === 'function'; },
                isArray: Array.isArray,
                isNull: function(obj) { return obj === null; },
                isUndefined: function(obj) { return obj === undefined; },
                isObject: function(obj) { return typeof obj === 'object' && obj !== null; },
                deprecate: function(fn) { return fn; },
                types: { isRegExp: function(v) { return v instanceof RegExp; }, isDate: function(v) { return v instanceof Date; } },
                TextEncoder: globalThis.TextEncoder,
                TextDecoder: globalThis.TextDecoder
            };
            globalThis.__requireCache['util'] = util;

            // ── events ────────────────────────────────────────────────────────────
            function EventEmitter() { this._events = {}; this._maxListeners = 10; }
            EventEmitter.prototype.on = EventEmitter.prototype.addListener = function(ev, fn) {
                if (!this._events[ev]) this._events[ev] = [];
                this._events[ev].push(fn);
                return this;
            };
            EventEmitter.prototype.once = function(ev, fn) {
                var self = this;
                function wrapper() { self.removeListener(ev, wrapper); fn.apply(this, arguments); }
                wrapper._orig = fn;
                return this.on(ev, wrapper);
            };
            EventEmitter.prototype.removeListener = EventEmitter.prototype.off = function(ev, fn) {
                if (!this._events[ev]) return this;
                this._events[ev] = this._events[ev].filter(function(f) { return f !== fn && f._orig !== fn; });
                return this;
            };
            EventEmitter.prototype.removeAllListeners = function(ev) {
                if (ev) { delete this._events[ev]; } else { this._events = {}; } return this;
            };
            EventEmitter.prototype.emit = function(ev) {
                var args = Array.prototype.slice.call(arguments, 1);
                var listeners = (this._events[ev] || []).slice();
                listeners.forEach(function(fn) { fn.apply(null, args); });
                return listeners.length > 0;
            };
            EventEmitter.prototype.listeners = function(ev) { return (this._events[ev] || []).slice(); };
            EventEmitter.prototype.listenerCount = function(ev) { return (this._events[ev] || []).length; };
            EventEmitter.prototype.setMaxListeners = function(n) { this._maxListeners = n; return this; };
            EventEmitter.EventEmitter = EventEmitter;
            globalThis.__requireCache['events'] = EventEmitter;

            // ── stream ────────────────────────────────────────────────────────────
            function Stream() { EventEmitter.call(this); }
            util.inherits(Stream, EventEmitter);
            Stream.prototype.pipe = function() { return this; };
            function Readable(opts) { Stream.call(this); this.readable = true; this._buf = []; this._ended = false; }
            util.inherits(Readable, Stream);
            Readable.prototype.read = function() { return null; };
            Readable.prototype.push = function(chunk) { if (chunk === null) { this._ended = true; this.emit('end'); } else { this.emit('data', chunk); } };
            Readable.prototype.pipe = function(dest) { this.on('data', function(c) { dest.write(c); }); this.on('end', function() { dest.end(); }); return dest; };
            function Writable(opts) { Stream.call(this); this.writable = true; }
            util.inherits(Writable, Stream);
            Writable.prototype.write = function(chunk, enc, cb) { if (typeof enc === 'function') cb = enc; if (cb) cb(); return true; };
            Writable.prototype.end = function(chunk, enc, cb) { if (chunk) this.write(chunk); this.emit('finish'); if (cb) cb(); };
            function Transform(opts) { Readable.call(this); this.writable = true; }
            util.inherits(Transform, Readable);
            Transform.prototype.write = function(chunk, enc, cb) { this.push(chunk); if (cb) cb(); return true; };
            Transform.prototype.end = function(chunk, enc, cb) { if (chunk) this.write(chunk); this.push(null); if (cb) cb(); };
            var stream = { Stream: Stream, Readable: Readable, Writable: Writable, Transform: Transform, PassThrough: Transform };
            globalThis.__requireCache['stream'] = stream;
            globalThis.__requireCache['stream/web'] = stream;
            globalThis.__requireCache['readable-stream'] = stream;

            // ── path ──────────────────────────────────────────────────────────────
            var path = {
                sep: '/',
                delimiter: ':',
                join: function() { return Array.prototype.slice.call(arguments).join('/').replace(/\/+/g, '/'); },
                resolve: function() {
                    var parts = Array.prototype.slice.call(arguments);
                    var result = parts[parts.length - 1];
                    for (var i = parts.length - 2; i >= 0; i--) {
                        if (parts[i].startsWith('/')) { result = parts[i] + '/' + result; break; }
                        result = parts[i] + '/' + result;
                    }
                    return result.replace(/\/+/g, '/');
                },
                dirname: function(p) { var i = p.lastIndexOf('/'); return i < 0 ? '.' : p.slice(0, i) || '/'; },
                basename: function(p, ext) { var b = p.slice(p.lastIndexOf('/') + 1); return ext && b.endsWith(ext) ? b.slice(0, b.length - ext.length) : b; },
                extname: function(p) { var b = p.slice(p.lastIndexOf('/') + 1); var i = b.lastIndexOf('.'); return i > 0 ? b.slice(i) : ''; },
                isAbsolute: function(p) { return p.startsWith('/'); },
                normalize: function(p) { return p.replace(/\/+/g, '/'); },
                relative: function(from, to) { return to; },
                parse: function(p) { return { root: p.startsWith('/') ? '/' : '', dir: path.dirname(p), base: path.basename(p), ext: path.extname(p), name: path.basename(p, path.extname(p)) }; },
                format: function(obj) { return (obj.dir ? obj.dir + '/' : '') + (obj.base || (obj.name || '') + (obj.ext || '')); }
            };
            globalThis.__requireCache['path'] = path;
            globalThis.__requireCache['node:path'] = path;

            // ── buffer ────────────────────────────────────────────────────────────
            var bufMod = { Buffer: globalThis.Buffer };
            globalThis.__requireCache['buffer'] = bufMod;
            globalThis.__requireCache['node:buffer'] = bufMod;

            // ── os ────────────────────────────────────────────────────────────────
            var os = {
                platform: function() { return 'linux'; },
                type: function() { return 'Linux'; },
                arch: function() { return 'x64'; },
                hostname: function() { return 'localhost'; },
                homedir: function() { return '/home/user'; },
                tmpdir: function() { return '/tmp'; },
                EOL: '\n',
                cpus: function() { return []; },
                totalmem: function() { return 1073741824; },
                freemem: function() { return 536870912; },
                networkInterfaces: function() { return {}; },
                userInfo: function() { return { username: 'user', uid: 1000, gid: 1000, shell: '/bin/sh', homedir: '/home/user' }; },
                release: function() { return '6.0.0'; },
                version: function() { return '#1 SMP'; },
                uptime: function() { return 0; },
                loadavg: function() { return [0, 0, 0]; },
                endianness: function() { return 'LE'; },
                constants: { signals: {}, errno: {} }
            };
            globalThis.__requireCache['os'] = os;
            globalThis.__requireCache['node:os'] = os;

            // ── url ───────────────────────────────────────────────────────────────
            var url = {
                parse: function(s) { try { var u = new URL(s); return { protocol: u.protocol, host: u.host, hostname: u.hostname, port: u.port, pathname: u.pathname, search: u.search, hash: u.hash, href: u.href }; } catch(e) { return { href: s }; } },
                format: function(obj) { if (typeof obj === 'string') return obj; return (obj.protocol ? obj.protocol + '//' : '') + (obj.host || obj.hostname || '') + (obj.pathname || '/') + (obj.search || ''); },
                resolve: function(from, to) { return to; },
                URL: URL,
                URLSearchParams: URLSearchParams
            };
            globalThis.__requireCache['url'] = url;
            globalThis.__requireCache['node:url'] = url;

            // ── querystring ───────────────────────────────────────────────────────
            var qs = {
                stringify: function(obj) { return Object.keys(obj).map(function(k) { return encodeURIComponent(k) + '=' + encodeURIComponent(obj[k]); }).join('&'); },
                parse: function(s) { var o = {}; s.split('&').forEach(function(p) { var kv = p.split('='); if (kv[0]) o[decodeURIComponent(kv[0])] = decodeURIComponent(kv[1] || ''); }); return o; },
                escape: encodeURIComponent,
                unescape: decodeURIComponent
            };
            globalThis.__requireCache['querystring'] = qs;
            globalThis.__requireCache['node:querystring'] = qs;
            globalThis.__requireCache['qs'] = qs;

            // ── string_decoder ────────────────────────────────────────────────────
            function StringDecoder(enc) { this.encoding = enc || 'utf8'; }
            StringDecoder.prototype.write = function(buf) { return typeof buf === 'string' ? buf : String.fromCharCode.apply(null, buf); };
            StringDecoder.prototype.end = function() { return ''; };
            globalThis.__requireCache['string_decoder'] = { StringDecoder: StringDecoder };
            globalThis.__requireCache['node:string_decoder'] = { StringDecoder: StringDecoder };

            // ── assert ────────────────────────────────────────────────────────────
            function assert(val, msg) { if (!val) throw new Error(msg || 'Assertion failed'); }
            assert.ok = assert;
            assert.equal = function(a, b, msg) { if (a != b) throw new Error(msg || a + ' != ' + b); };
            assert.strictEqual = function(a, b, msg) { if (a !== b) throw new Error(msg || a + ' !== ' + b); };
            assert.deepEqual = function(a, b, msg) { if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(msg || 'deep equal failed'); };
            assert.notEqual = function(a, b, msg) { if (a == b) throw new Error(msg || a + ' == ' + b); };
            assert.throws = function(fn, expected, msg) { try { fn(); } catch(e) { return; } throw new Error(msg || 'Expected throw'); };
            assert.doesNotThrow = function(fn, msg) { try { fn(); } catch(e) { throw new Error(msg || 'Unexpected throw: ' + e.message); } };
            globalThis.__requireCache['assert'] = assert;
            globalThis.__requireCache['node:assert'] = assert;

            // ── http / https (client backed by __fetchAsync) ──────────────────────
            var STATUS_CODES = {
                100:'Continue',101:'Switching Protocols',200:'OK',201:'Created',
                204:'No Content',206:'Partial Content',301:'Moved Permanently',
                302:'Found',304:'Not Modified',400:'Bad Request',401:'Unauthorized',
                403:'Forbidden',404:'Not Found',405:'Method Not Allowed',
                408:'Request Timeout',409:'Conflict',410:'Gone',422:'Unprocessable Entity',
                429:'Too Many Requests',500:'Internal Server Error',502:'Bad Gateway',
                503:'Service Unavailable',504:'Gateway Timeout'
            };

            function buildUrl(opts) {
                if (typeof opts === 'string') return opts;
                var protocol = opts.protocol || 'http:';
                var host = opts.hostname || opts.host || 'localhost';
                var port = opts.port ? ':' + opts.port : '';
                var path = opts.path || '/';
                return protocol + '//' + host + port + path;
            }

            function makeHttpModule(defaultProtocol) {
                function request(opts, cb) {
                    var url = buildUrl(opts);
                    if (!/^https?:\/\//.test(url)) url = defaultProtocol + '//' + url;
                    var method = (typeof opts === 'object' && opts.method ? opts.method : 'GET').toUpperCase();
                    var headers = (typeof opts === 'object' && opts.headers) ? opts.headers : {};
                    var bodyParts = [];
                    var listeners = { data: [], end: [], error: [], response: [] };

                    var req = {
                        write: function(chunk) { bodyParts.push(chunk); },
                        end: function(chunk) {
                            if (chunk) bodyParts.push(chunk);
                            var body = bodyParts.length ? bodyParts.join('') : undefined;
                            __fetchAsync(url, method, JSON.stringify(headers), body).then(function(raw) {
                                var d = JSON.parse(raw);
                                var res = {
                                    statusCode: d.status,
                                    statusMessage: d.statusText,
                                    headers: d.headers,
                                    _body: d.body,
                                    _listeners: { data: [], end: [], error: [] },
                                    on: function(ev, fn) { this._listeners[ev] = this._listeners[ev] || []; this._listeners[ev].push(fn); return this; },
                                    pipe: function(dest) { if (dest && dest.write) dest.write(d.body); if (dest && dest.end) dest.end(); return dest; },
                                    resume: function() {}
                                };
                                // deliver body
                                if (cb) cb(res);
                                listeners.response.forEach(function(fn) { fn(res); });
                                setTimeout(function() {
                                    res._listeners.data.forEach(function(fn) { fn(d.body); });
                                    res._listeners.end.forEach(function(fn) { fn(); });
                                    listeners.data.forEach(function(fn) { fn(d.body); });
                                    listeners.end.forEach(function(fn) { fn(); });
                                }, 0);
                            }).catch(function(e) {
                                listeners.error.forEach(function(fn) { fn(e); });
                                req._elisteners && req._elisteners.forEach(function(fn) { fn(e); });
                            });
                        },
                        on: function(ev, fn) { listeners[ev] = listeners[ev] || []; listeners[ev].push(fn); return this; },
                        setHeader: function(k, v) { headers[k] = v; },
                        getHeader: function(k) { return headers[k]; },
                        removeHeader: function(k) { delete headers[k]; },
                        abort: function() {},
                        setTimeout: function(ms, cb) { if (cb) setTimeout(cb, ms); }
                    };
                    return req;
                }

                return {
                    request: request,
                    get: function(url, opts, cb) {
                        if (typeof opts === 'function') { cb = opts; opts = {}; }
                        var req = request(typeof url === 'string' ? Object.assign({ path: url }, opts) : url, cb);
                        req.end();
                        return req;
                    },
                    STATUS_CODES: STATUS_CODES,
                    createServer: function() { return { listen: function() {}, close: function() {} }; },
                    Agent: function() {},
                    globalAgent: {}
                };
            }

            var httpMod = makeHttpModule('http:');
            var httpsMod = makeHttpModule('https:');
            globalThis.__requireCache['http'] = httpMod;
            globalThis.__requireCache['https'] = httpsMod;
            globalThis.__requireCache['node:http'] = httpMod;
            globalThis.__requireCache['node:https'] = httpsMod;

            // ── crypto (minimal) ─────────────────────────────────────────────────
            var crypto = {
                randomBytes: function(n) { var a = new Uint8Array(n); for (var i=0;i<n;i++) a[i]=Math.random()*256|0; return a; },
                createHash: function(alg) { return { _data:'', update: function(d){ this._data+=d; return this; }, digest: function(enc){ return enc==='hex'?btoa(this._data):''; } }; },
                createHmac: function(alg, key) { return { update: function(d){ return this; }, digest: function(enc){ return ''; } }; },
                randomUUID: function() { return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g,function(c){var r=Math.random()*16|0;return(c==='x'?r:(r&0x3|0x8)).toString(16);}); },
                getRandomValues: function(arr) { for(var i=0;i<arr.length;i++) arr[i]=Math.random()*256|0; return arr; }
            };
            globalThis.__requireCache['crypto'] = crypto;
            globalThis.__requireCache['node:crypto'] = crypto;

            // ── zlib — real impl injected by zlib.rs builtin after this block ─────
            globalThis.__requireCache['zlib'] = { gzip: function(b,cb){cb(null,b);}, gunzip: function(b,cb){cb(null,b);}, deflate: function(b,cb){cb(null,b);}, inflate: function(b,cb){cb(null,b);}, constants: {} };
            globalThis.__requireCache['node:zlib'] = globalThis.__requireCache['zlib'];

            // ── net / tls — backed by __tcpConnect / __tcpConnectTls ─────────────
            function Socket(opts) {
                EventEmitter.call(this);
                opts = opts || {};
                this._id = null;
                this._tls = !!(opts.tls);
                this._connected = false;
                this._destroyed = false;
                this._encoding = null;
                this._pollTimer = null;
                this.readable = true;
                this.writable = true;
                this.connecting = false;
                this.pending = true;
                this.destroyed = false;
                this.bytesRead = 0;
                this.bytesWritten = 0;
                this.remoteAddress = '';
                this.remotePort = 0;
                this.remoteFamily = 'IPv4';
                this.localAddress = '127.0.0.1';
                this.localPort = 0;
            }
            util.inherits(Socket, EventEmitter);

            Socket.prototype.connect = function(portOrOpts, host, callback) {
                if (typeof portOrOpts === 'object' && portOrOpts !== null) {
                    var o = portOrOpts;
                    if (typeof host === 'function') callback = host;
                    host = o.host || 'localhost';
                    portOrOpts = o.port;
                    if (o.servername || typeof o.rejectUnauthorized !== 'undefined') this._tls = true;
                } else {
                    if (typeof host === 'function') { callback = host; host = 'localhost'; }
                }
                host = host || 'localhost';
                var port = portOrOpts;
                var self = this;
                self.connecting = true;
                self.pending = false;
                self.remoteAddress = host;
                self.remotePort = port;
                if (callback) self.once('connect', callback);
                setTimeout(function() {
                    if (self._destroyed) return;
                    try {
                        if (self._tls) {
                            self._id = __tcpConnectTls(host, String(port));
                        } else {
                            self._id = __tcpConnect(host, String(port));
                        }
                        self.connecting = false;
                        self._connected = true;
                        self.emit('connect');
                        self.emit('ready');
                        self._startPoll();
                    } catch(e) {
                        self.connecting = false;
                        self.emit('error', e);
                        self.emit('close', true);
                    }
                }, 0);
                return this;
            };

            Socket.prototype._startPoll = function() {
                var self = this;
                self._pollTimer = setInterval(function() {
                    if (self._destroyed || self._id === null) {
                        clearInterval(self._pollTimer);
                        self._pollTimer = null;
                        return;
                    }
                    try {
                        var chunk = __tcpRead(self._id, 65536);
                        self.bytesRead += chunk.length;
                        var data = self._encoding
                            ? new TextDecoder(self._encoding).decode(new Uint8Array(chunk))
                            : new Uint8Array(chunk);
                        self.emit('data', data);
                    } catch(e) {
                        if (e.code === 'EAGAIN') return; // no data yet
                        clearInterval(self._pollTimer);
                        self._pollTimer = null;
                        if (e.code === 'EOF') {
                            self.emit('end');
                            self.emit('close', false);
                        } else {
                            self.emit('error', e);
                            self.emit('close', true);
                        }
                    }
                }, 5);
            };

            Socket.prototype.write = function(data, encoding, callback) {
                if (typeof encoding === 'function') { callback = encoding; encoding = null; }
                if (this._destroyed || this._id === null) {
                    var err = new Error('write after end');
                    if (callback) callback(err); else this.emit('error', err);
                    return false;
                }
                try {
                    var bytes;
                    if (typeof data === 'string') {
                        var enc = encoding || 'utf8';
                        if (enc === 'hex') {
                            var hex = data;
                            bytes = [];
                            for (var i = 0; i < hex.length; i += 2) bytes.push(parseInt(hex.substr(i, 2), 16));
                        } else {
                            bytes = Array.from(new TextEncoder().encode(data));
                        }
                    } else {
                        bytes = Array.from(new Uint8Array(data.buffer ? data.buffer : data));
                    }
                    __tcpWrite(this._id, bytes);
                    this.bytesWritten += bytes.length;
                    if (callback) callback(null);
                    return true;
                } catch(e) {
                    if (callback) callback(e); else this.emit('error', e);
                    return false;
                }
            };

            Socket.prototype.end = function(data, encoding, callback) {
                if (typeof data === 'function') { callback = data; data = null; }
                else if (typeof encoding === 'function') { callback = encoding; encoding = null; }
                if (data) this.write(data, encoding);
                this.destroy();
                if (callback) callback();
                return this;
            };

            Socket.prototype.destroy = function(err) {
                if (this._destroyed) return this;
                this._destroyed = true;
                this.destroyed = true;
                if (this._pollTimer) { clearInterval(this._pollTimer); this._pollTimer = null; }
                if (this._id !== null) {
                    try { __tcpClose(this._id); } catch(_) {}
                    this._id = null;
                }
                if (err) this.emit('error', err);
                this.emit('close', !!err);
                return this;
            };

            Socket.prototype.setEncoding = function(enc) { this._encoding = enc; return this; };
            Socket.prototype.setTimeout = function(ms, cb) {
                if (cb) this.once('timeout', cb);
                if (this._id !== null) { try { __tcpSetTimeout(this._id, ms); } catch(_) {} }
                return this;
            };
            Socket.prototype.setNoDelay = function() { return this; };
            Socket.prototype.setKeepAlive = function() { return this; };
            Socket.prototype.ref = function() { return this; };
            Socket.prototype.unref = function() { return this; };
            Socket.prototype.pause = function() { return this; };
            Socket.prototype.resume = function() { return this; };
            Socket.prototype.address = function() {
                return { address: this.remoteAddress, port: this.remotePort, family: this.remoteFamily };
            };
            Socket.prototype.pipe = function(dest) {
                this.on('data', function(chunk) { dest.write(chunk); });
                this.on('end', function() { dest.end(); });
                return dest;
            };

            function TLSSocket(socket, opts) {
                Socket.call(this, opts || {});
                this._tls = true;
                if (socket && socket._id != null) this._id = socket._id;
            }
            util.inherits(TLSSocket, Socket);
            TLSSocket.prototype.getCipher = function() { return { name: 'TLS_AES_256_GCM_SHA384', version: 'TLSv1.3' }; };
            TLSSocket.prototype.getPeerCertificate = function() { return {}; };
            TLSSocket.prototype.authorized = true;

            var netMod = {
                Socket: Socket,
                createConnection: function(opts, cb) {
                    var s = new Socket();
                    if (typeof opts === 'number') {
                        var port = opts, host = typeof cb === 'string' ? cb : 'localhost';
                        cb = typeof cb === 'function' ? cb : arguments[2];
                        s.connect(port, host, cb);
                    } else {
                        s.connect(opts, cb);
                    }
                    return s;
                },
                connect: function() { return netMod.createConnection.apply(netMod, arguments); },
                createServer: function(opts, cb) {
                    if (typeof opts === 'function') { cb = opts; opts = {}; }
                    return { listen: function() {}, close: function() {}, address: function() { return {}; } };
                },
                isIP: function(addr) { return /^\d+\.\d+\.\d+\.\d+$/.test(addr) ? 4 : 0; },
                isIPv4: function(addr) { return /^\d+\.\d+\.\d+\.\d+$/.test(addr); },
                isIPv6: function(addr) { return false; },
            };

            var tlsMod = {
                TLSSocket: TLSSocket,
                connect: function(opts, cb) {
                    if (typeof opts === 'number') {
                        var o = { port: opts, host: typeof cb === 'string' ? cb : 'localhost' };
                        cb = typeof cb === 'function' ? cb : arguments[2];
                        opts = o;
                    }
                    var s = new TLSSocket(null, { tls: true });
                    s.connect(opts, cb);
                    return s;
                },
                createServer: function(opts, cb) {
                    if (typeof opts === 'function') { cb = opts; opts = {}; }
                    return { listen: function() {}, close: function() {}, address: function() { return {}; } };
                },
                createSecureContext: function() { return {}; },
                checkServerIdentity: function(host, cert) { return undefined; },
            };

            globalThis.__requireCache['net'] = netMod;
            globalThis.__requireCache['tls'] = tlsMod;
            globalThis.__requireCache['node:net'] = netMod;
            globalThis.__requireCache['node:tls'] = tlsMod;

            // ── child_process — real impl injected by child_process.rs builtin ────
            globalThis.__requireCache['child_process'] = { exec: function(cmd,cb){if(cb)cb(null,'','');}, spawn: function(){return new EventEmitter();}, execSync: function(){throw new Error('execSync not available');}};
            globalThis.__requireCache['node:child_process'] = globalThis.__requireCache['child_process'];

            // ── fs (proxy to globalThis.fs) ───────────────────────────────────────
            globalThis.__requireCache['fs'] = globalThis.fs || {};
            globalThis.__requireCache['node:fs'] = globalThis.fs || {};
            globalThis.__requireCache['fs/promises'] = {};
            globalThis.__requireCache['node:fs/promises'] = {};

            // ── process (proxy to globalThis.process) ─────────────────────────────
            globalThis.__requireCache['process'] = globalThis.process || {};
            globalThis.__requireCache['node:process'] = globalThis.process || {};

            // ── perf_hooks ────────────────────────────────────────────────────────
            globalThis.__requireCache['perf_hooks'] = { performance: { now: function() { return Date.now(); } } };
            globalThis.__requireCache['node:perf_hooks'] = globalThis.__requireCache['perf_hooks'];
            globalThis.__requireCache['perf_hooks'].PerformanceObserver = function() {};

            // ── agent-base / https-proxy-agent (proxy stubs, not supported in sandbox) ──
            function AgentBase() {}
            AgentBase.prototype.addRequest = function() {};
            globalThis.__requireCache['agent-base'] = { Agent: AgentBase };
            var HttpsProxyAgentStub = function(opts) { AgentBase.call(this); this.proxy = opts; };
            util.inherits(HttpsProxyAgentStub, AgentBase);
            HttpsProxyAgentStub.prototype.callback = function(req, opts) { return opts; };
            globalThis.__requireCache['https-proxy-agent'] = { HttpsProxyAgent: HttpsProxyAgentStub };

            // ── http2 — client backed by __fetchAsync ─────────────────────────────
            var HTTP2_CONSTANTS = {
                HTTP2_HEADER_METHOD: ':method', HTTP2_HEADER_PATH: ':path',
                HTTP2_HEADER_SCHEME: ':scheme', HTTP2_HEADER_AUTHORITY: ':authority',
                HTTP2_HEADER_STATUS: ':status',
                HTTP_STATUS_OK: 200, HTTP_STATUS_CREATED: 201, HTTP_STATUS_NO_CONTENT: 204,
                HTTP_STATUS_BAD_REQUEST: 400, HTTP_STATUS_UNAUTHORIZED: 401,
                HTTP_STATUS_FORBIDDEN: 403, HTTP_STATUS_NOT_FOUND: 404,
                HTTP_STATUS_INTERNAL_SERVER_ERROR: 500,
                NGHTTP2_NO_ERROR: 0,
                DEFAULT_SETTINGS_HEADER_TABLE_SIZE: 4096,
                DEFAULT_SETTINGS_ENABLE_PUSH: 1,
                DEFAULT_SETTINGS_INITIAL_WINDOW_SIZE: 65535,
                DEFAULT_SETTINGS_MAX_FRAME_SIZE: 16384,
            };

            function Http2Request(authority, headers) {
                EventEmitter.call(this);
                this._authority = authority;
                this._headers = headers || {};
                this._body = [];
                this._responseListeners = [];
                this._dataListeners = [];
                this._endListeners = [];
                this._sent = false;
            }
            util.inherits(Http2Request, EventEmitter);

            Http2Request.prototype.write = function(chunk) {
                this._body.push(typeof chunk === 'string' ? chunk : new TextDecoder().decode(new Uint8Array(chunk)));
                return this;
            };

            Http2Request.prototype.end = function(chunk) {
                if (chunk) this.write(chunk);
                if (this._sent) return this;
                this._sent = true;
                var self = this;
                var h = self._headers;
                var method = (h[':method'] || 'GET').toUpperCase();
                var path = h[':path'] || '/';
                var scheme = h[':scheme'] || 'https';
                var authority = h[':authority'] || self._authority;
                var url = scheme + '://' + authority + path;
                var sendHeaders = {};
                Object.keys(h).forEach(function(k) { if (k.charAt(0) !== ':') sendHeaders[k] = h[k]; });
                var body = self._body.length ? self._body.join('') : undefined;
                __fetchAsync(url, method, JSON.stringify(sendHeaders), body).then(function(raw) {
                    var d = JSON.parse(raw);
                    var respHeaders = { ':status': d.status };
                    Object.keys(d.headers).forEach(function(k) { respHeaders[k] = d.headers[k]; });
                    self.emit('response', respHeaders, 0);
                    setTimeout(function() {
                        if (d.body) self.emit('data', d.body);
                        self.emit('end');
                        self.emit('close');
                    }, 0);
                }).catch(function(e) {
                    self.emit('error', e);
                });
                return this;
            };

            Http2Request.prototype.setEncoding = function(enc) { return this; };
            Http2Request.prototype.setTimeout = function(ms, cb) { if (cb) setTimeout(cb, ms); return this; };
            Http2Request.prototype.close = function() { return this; };

            function Http2Session(authority) {
                EventEmitter.call(this);
                this._authority = authority.replace(/^https?:\/\//, '');
                this._closed = false;
            }
            util.inherits(Http2Session, EventEmitter);

            Http2Session.prototype.request = function(headers, opts) {
                var h = Object.assign({ ':authority': this._authority }, headers || {});
                var req = new Http2Request(this._authority, h);
                var self = this;
                // Emit 'connect' asynchronously so listeners can be attached first.
                setTimeout(function() { self.emit('connect', self, {}); }, 0);
                return req;
            };

            Http2Session.prototype.close = function(cb) {
                this._closed = true;
                if (cb) cb();
                this.emit('close');
            };

            Http2Session.prototype.destroy = function() { this.close(); };
            Http2Session.prototype.setTimeout = function(ms, cb) { if (cb) setTimeout(cb, ms); return this; };
            Http2Session.prototype.ping = function(cb) { if (cb) cb(null, 0, {}); };
            Http2Session.prototype.settings = function() {};
            Http2Session.prototype.remoteSettings = { headerTableSize: 4096, enablePush: true, initialWindowSize: 65535, maxFrameSize: 16384, maxHeaderListSize: Infinity };
            Http2Session.prototype.localSettings = Http2Session.prototype.remoteSettings;

            var http2Mod = {
                connect: function(authority, opts, cb) {
                    if (typeof opts === 'function') { cb = opts; opts = {}; }
                    var session = new Http2Session(authority);
                    if (cb) session.once('connect', cb);
                    setTimeout(function() { session.emit('connect', session, {}); }, 0);
                    return session;
                },
                createServer: function(opts, cb) {
                    if (typeof opts === 'function') { cb = opts; }
                    return { listen: function() {}, close: function() {}, address: function() { return {}; } };
                },
                createSecureServer: function(opts, cb) { return http2Mod.createServer(opts, cb); },
                constants: HTTP2_CONSTANTS,
                sensitiveHeaders: Symbol('sensitiveHeaders'),
            };

            globalThis.__requireCache['http2'] = http2Mod;
            globalThis.__requireCache['node:http2'] = http2Mod;

            // ── debug (common logging utility) ───────────────────────────────────
            globalThis.__requireCache['debug'] = function(ns) { return function() {}; };
            globalThis.__requireCache['ms'] = function(val) { return typeof val === 'string' ? 0 : val; };

            // ── proxy-from-env ───────────────────────────────────────────────────
            globalThis.__requireCache['proxy-from-env'] = { getProxyForUrl: function() { return null; } };

            // ── AbortSignal ───────────────────────────────────────────────────────
            function AbortSignal() {
                this.aborted = false;
                this.reason = undefined;
                this._listeners = [];
            }
            AbortSignal.prototype.addEventListener = function(type, listener) {
                if (type !== 'abort') return;
                if (this.aborted) { listener({ type: 'abort', target: this }); return; }
                this._listeners.push(listener);
            };
            AbortSignal.prototype.removeEventListener = function(type, listener) {
                if (type === 'abort') this._listeners = this._listeners.filter(function(l) { return l !== listener; });
            };
            AbortSignal.prototype.throwIfAborted = function() {
                if (this.aborted) throw this.reason || new Error('AbortError');
            };
            AbortSignal.prototype._abort = function(reason) {
                if (this.aborted) return;
                this.aborted = true;
                this.reason = reason !== undefined ? reason : new Error('AbortError');
                var self = this;
                this._listeners.forEach(function(l) { l({ type: 'abort', target: self }); });
                this._listeners = [];
            };
            AbortSignal.timeout = function(ms) {
                var s = new AbortSignal();
                setTimeout(function() { s._abort(new Error('TimeoutError')); }, ms);
                return s;
            };
            AbortSignal.abort = function(reason) {
                var s = new AbortSignal();
                s._abort(reason);
                return s;
            };

            // ── AbortController ───────────────────────────────────────────────────
            function AbortController() {
                this.signal = new AbortSignal();
            }
            AbortController.prototype.abort = function(reason) {
                this.signal._abort(reason);
            };

            globalThis.AbortSignal = AbortSignal;
            globalThis.AbortController = AbortController;

            // ── Blob / File ───────────────────────────────────────────────────────
            function Blob(parts, options) {
                parts = parts || [];
                options = options || {};
                this.type = String(options.type || '');
                this._data = parts.map(function(p) {
                    if (typeof p === 'string') return p;
                    if (p && typeof p._data === 'string') return p._data;
                    if (p instanceof Uint8Array || Array.isArray(p)) {
                        return Array.from(p).map(function(b) { return String.fromCharCode(b & 0xff); }).join('');
                    }
                    if (p instanceof ArrayBuffer) {
                        return Array.from(new Uint8Array(p)).map(function(b) { return String.fromCharCode(b); }).join('');
                    }
                    return String(p);
                }).join('');
                this.size = this._data.length;
            }
            Blob.prototype.text = function() { var d = this._data; return Promise.resolve(d); };
            Blob.prototype.arrayBuffer = function() {
                var d = this._data;
                var buf = new ArrayBuffer(d.length);
                var view = new Uint8Array(buf);
                for (var i = 0; i < d.length; i++) view[i] = d.charCodeAt(i);
                return Promise.resolve(buf);
            };
            Blob.prototype.bytes = function() {
                var d = this._data;
                var arr = new Uint8Array(d.length);
                for (var i = 0; i < d.length; i++) arr[i] = d.charCodeAt(i);
                return Promise.resolve(arr);
            };
            Blob.prototype.slice = function(start, end, type) {
                return new Blob([this._data.slice(start, end)], { type: type || this.type });
            };
            Blob.prototype.stream = function() {
                var d = this._data;
                return new ReadableStream({ start: function(c) { c.enqueue(d); c.close(); } });
            };

            function File(parts, name, options) {
                Blob.call(this, parts, options);
                this.name = String(name || '');
                this.lastModified = (options && options.lastModified != null) ? options.lastModified : Date.now();
            }
            File.prototype = Object.create(Blob.prototype);
            File.prototype.constructor = File;

            globalThis.Blob = Blob;
            globalThis.File = File;

            // ── ReadableStream / WritableStream / TransformStream ─────────────────
            function ReadableStream(underlyingSource, strategy) {
                underlyingSource = underlyingSource || {};
                var self = this;
                this._chunks = [];
                this._closed = false;
                this._error = null;
                this._waiting = [];
                this.locked = false;

                var controller = {
                    enqueue: function(chunk) { self._push({ value: chunk, done: false }); },
                    close: function() { self._closed = true; self._push({ value: undefined, done: true }); },
                    error: function(e) { self._error = e; self._push(null); },
                    desiredSize: 1
                };
                this._controller = controller;
                try { if (underlyingSource.start) underlyingSource.start(controller); } catch(e) { this._error = e; }
                this._source = underlyingSource;
            }
            ReadableStream.prototype._push = function(item) {
                if (this._waiting.length > 0) {
                    var resolve = this._waiting.shift();
                    if (this._error) resolve({ error: this._error });
                    else resolve(item);
                } else if (item && !item.done) {
                    this._chunks.push(item.value);
                }
            };
            ReadableStream.prototype.getReader = function() {
                var self = this;
                this.locked = true;
                return {
                    read: function() {
                        return new Promise(function(resolve, reject) {
                            if (self._error) return reject(self._error);
                            if (self._chunks.length > 0) return resolve({ value: self._chunks.shift(), done: false });
                            if (self._closed) return resolve({ value: undefined, done: true });
                            self._waiting.push(function(item) {
                                if (item && item.error) reject(item.error);
                                else resolve(item || { value: undefined, done: true });
                            });
                        });
                    },
                    cancel: function(reason) { self._closed = true; self._waiting.forEach(function(r) { r({ value: undefined, done: true }); }); self._waiting = []; return Promise.resolve(); },
                    releaseLock: function() { self.locked = false; }
                };
            };
            ReadableStream.prototype.pipeTo = function(writable, options) {
                var reader = this.getReader();
                var writer = writable.getWriter();
                function pump() {
                    return reader.read().then(function(res) {
                        if (res.done) return writer.close();
                        return writer.write(res.value).then(pump);
                    });
                }
                return pump();
            };
            ReadableStream.prototype.pipeThrough = function(transform, options) {
                this.pipeTo(transform.writable);
                return transform.readable;
            };
            ReadableStream.prototype.tee = function() {
                var chunks1 = [], chunks2 = [], waiters1 = [], waiters2 = [];
                var closed = false;
                var reader = this.getReader();
                function pump() {
                    reader.read().then(function(res) {
                        if (res.done) { closed = true; [waiters1, waiters2].forEach(function(ws) { ws.forEach(function(r) { r({ value: undefined, done: true }); }); }); return; }
                        chunks1.push(res.value); chunks2.push(res.value);
                        if (waiters1.length) waiters1.shift()({ value: chunks1.shift(), done: false });
                        if (waiters2.length) waiters2.shift()({ value: chunks2.shift(), done: false });
                        pump();
                    });
                }
                pump();
                function makeStream(chunks, waiters) {
                    return new ReadableStream({ start: function(c) {
                        chunks._ctrl = c;
                    }});
                }
                var s1 = { getReader: function() { return { read: function() { return new Promise(function(r) { if (chunks1.length) r({ value: chunks1.shift(), done: false }); else if (closed) r({ value: undefined, done: true }); else waiters1.push(r); }); }, releaseLock: function() {} }; } };
                var s2 = { getReader: function() { return { read: function() { return new Promise(function(r) { if (chunks2.length) r({ value: chunks2.shift(), done: false }); else if (closed) r({ value: undefined, done: true }); else waiters2.push(r); }); }, releaseLock: function() {} }; } };
                return [s1, s2];
            };

            function WritableStream(underlyingSink, strategy) {
                underlyingSink = underlyingSink || {};
                this._sink = underlyingSink;
                this._closed = false;
                this._writer = null;
                var controller = { error: function(e) {} };
                try { if (underlyingSink.start) underlyingSink.start(controller); } catch(e) {}
            }
            WritableStream.prototype.getWriter = function() {
                var self = this;
                return {
                    write: function(chunk) {
                        if (self._closed) return Promise.reject(new Error('WritableStream is closed'));
                        if (!self._sink.write) return Promise.resolve();
                        try { var r = self._sink.write(chunk); return (r && typeof r.then === 'function') ? r : Promise.resolve(); }
                        catch(e) { return Promise.reject(e); }
                    },
                    close: function() {
                        self._closed = true;
                        if (self._sink.close) { try { self._sink.close(); } catch(e) {} }
                        return Promise.resolve();
                    },
                    abort: function(reason) {
                        self._closed = true;
                        if (self._sink.abort) { try { self._sink.abort(reason); } catch(e) {} }
                        return Promise.resolve();
                    },
                    releaseLock: function() {},
                    closed: Promise.resolve(),
                    desiredSize: 1,
                    ready: Promise.resolve()
                };
            };
            Object.defineProperty(WritableStream.prototype, 'locked', { get: function() { return false; } });

            function TransformStream(transformer, writableStrategy, readableStrategy) {
                transformer = transformer || {};
                var readableCtrl;
                this.readable = new ReadableStream({
                    start: function(c) { readableCtrl = c; }
                });
                var readableController = {
                    enqueue: function(chunk) { readableCtrl.enqueue(chunk); },
                    terminate: function() { readableCtrl.close(); },
                    error: function(e) { if (readableCtrl.error) readableCtrl.error(e); }
                };
                this.writable = new WritableStream({
                    write: function(chunk) {
                        if (transformer.transform) return transformer.transform(chunk, readableController);
                    },
                    close: function() {
                        if (transformer.flush) return transformer.flush(readableController);
                        readableController.terminate();
                    }
                });
            }

            globalThis.ReadableStream = ReadableStream;
            globalThis.WritableStream = WritableStream;
            globalThis.TransformStream = TransformStream;

            // ── FormData ──────────────────────────────────────────────────────────
            function FormData() { this._entries = []; }
            FormData.prototype.append = function(name, value, filename) { this._entries.push({ name: String(name), value: value, filename: filename }); };
            FormData.prototype.set = function(name, value, filename) { this.delete(name); this._entries.push({ name: String(name), value: value, filename: filename }); };
            FormData.prototype.get = function(name) { var e = this._entries.find(function(e) { return e.name === String(name); }); return e ? e.value : null; };
            FormData.prototype.getAll = function(name) { return this._entries.filter(function(e) { return e.name === String(name); }).map(function(e) { return e.value; }); };
            FormData.prototype.has = function(name) { return this._entries.some(function(e) { return e.name === String(name); }); };
            FormData.prototype.delete = function(name) { this._entries = this._entries.filter(function(e) { return e.name !== String(name); }); };
            FormData.prototype.forEach = function(cb) { this._entries.forEach(function(e) { cb(e.value, e.name); }); };
            FormData.prototype[Symbol.iterator] = function() {
                var i = 0; var entries = this._entries;
                return { next: function() { return i < entries.length ? { value: [entries[i].name, entries[i++].value], done: false } : { done: true, value: undefined }; } };
            };
            FormData.prototype.entries = function() { return this[Symbol.iterator](); };
            FormData.prototype.keys = function() { var i=0,e=this._entries; return { next:function(){ return i<e.length?{value:e[i++].name,done:false}:{done:true,value:undefined}; }, [Symbol.iterator]:function(){return this;} }; };
            FormData.prototype.values = function() { var i=0,e=this._entries; return { next:function(){ return i<e.length?{value:e[i++].value,done:false}:{done:true,value:undefined}; }, [Symbol.iterator]:function(){return this;} }; };

            globalThis.FormData = FormData;

            // ── URLSearchParams ───────────────────────────────────────────────────
            function URLSearchParams(init) {
                this._params = [];
                if (!init) return;
                if (typeof init === 'string') {
                    var s = init.replace(/^\?/, '');
                    if (s) s.split('&').forEach(function(pair) {
                        var idx = pair.indexOf('=');
                        var k = idx >= 0 ? pair.slice(0, idx) : pair;
                        var v = idx >= 0 ? pair.slice(idx + 1) : '';
                        this._params.push([decodeURIComponent(k.replace(/\+/g,' ')), decodeURIComponent(v.replace(/\+/g,' '))]);
                    }, this);
                } else if (Array.isArray(init)) {
                    init.forEach(function(p) { this._params.push([String(p[0]), String(p[1])]); }, this);
                } else if (init && typeof init === 'object') {
                    Object.keys(init).forEach(function(k) { this._params.push([k, String(init[k])]); }, this);
                }
            }
            URLSearchParams.prototype.append = function(k, v) { this._params.push([String(k), String(v)]); };
            URLSearchParams.prototype.set = function(k, v) {
                var found = false;
                this._params = this._params.filter(function(p) {
                    if (p[0] === String(k)) { if (!found) { found = true; p[1] = String(v); return true; } return false; } return true;
                });
                if (!found) this._params.push([String(k), String(v)]);
            };
            URLSearchParams.prototype.get = function(k) {
                var p = this._params.find(function(p) { return p[0] === String(k); });
                return p ? p[1] : null;
            };
            URLSearchParams.prototype.getAll = function(k) { return this._params.filter(function(p) { return p[0] === String(k); }).map(function(p) { return p[1]; }); };
            URLSearchParams.prototype.has = function(k) { return this._params.some(function(p) { return p[0] === String(k); }); };
            URLSearchParams.prototype.delete = function(k) { this._params = this._params.filter(function(p) { return p[0] !== String(k); }); };
            URLSearchParams.prototype.forEach = function(cb) { this._params.forEach(function(p) { cb(p[1], p[0]); }); };
            URLSearchParams.prototype.keys = function() { var i=0,ps=this._params; return { next:function(){ return i<ps.length?{value:ps[i++][0],done:false}:{done:true,value:undefined}; }, [Symbol.iterator]:function(){return this;} }; };
            URLSearchParams.prototype.values = function() { var i=0,ps=this._params; return { next:function(){ return i<ps.length?{value:ps[i++][1],done:false}:{done:true,value:undefined}; }, [Symbol.iterator]:function(){return this;} }; };
            URLSearchParams.prototype.entries = function() { var i=0,ps=this._params; return { next:function(){ return i<ps.length?{value:[ps[i][0],ps[i++][1]],done:false}:{done:true,value:undefined}; }, [Symbol.iterator]:function(){return this;} }; };
            URLSearchParams.prototype[Symbol.iterator] = URLSearchParams.prototype.entries;
            URLSearchParams.prototype.toString = function() {
                return this._params.map(function(p) { return encodeURIComponent(p[0]) + '=' + encodeURIComponent(p[1]); }).join('&');
            };
            Object.defineProperty(URLSearchParams.prototype, 'size', { get: function() { return this._params.length; } });
            globalThis.URLSearchParams = URLSearchParams;

            // ── URL ───────────────────────────────────────────────────────────────
            function URL(input, base) {
                if (typeof input !== 'string') throw new TypeError('Invalid URL');
                // Resolve relative URLs against base
                if (base) {
                    var b = typeof base === 'string' ? new URL(base) : base;
                    if (!/^[a-zA-Z][a-zA-Z0-9+\-.]*:/.test(input)) {
                        if (input.charAt(0) === '/') {
                            input = b.protocol + '//' + b.host + input;
                        } else {
                            var dir = b.pathname.replace(/\/[^/]*$/, '/');
                            input = b.protocol + '//' + b.host + dir + input;
                        }
                    }
                }
                var m = input.match(/^([a-zA-Z][a-zA-Z0-9+\-.]*):\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/);
                if (!m) {
                    // try protocol-relative or data URIs
                    var m2 = input.match(/^([a-zA-Z][a-zA-Z0-9+\-.]*):([^/].*)?$/);
                    if (m2) {
                        this.protocol = m2[1] + ':';
                        this.host = ''; this.hostname = ''; this.port = '';
                        this.pathname = m2[2] || '';
                        this.search = ''; this.hash = '';
                        this.username = ''; this.password = '';
                        this.origin = 'null';
                        this.href = input;
                        this.searchParams = new URLSearchParams();
                        return;
                    }
                    throw new TypeError('Invalid URL: ' + input);
                }
                this.protocol = m[1] + ':';
                var authority = m[2] || '';
                // userinfo
                var atIdx = authority.lastIndexOf('@');
                if (atIdx >= 0) {
                    var userinfo = authority.slice(0, atIdx).split(':');
                    this.username = decodeURIComponent(userinfo[0] || '');
                    this.password = decodeURIComponent(userinfo[1] || '');
                    authority = authority.slice(atIdx + 1);
                } else {
                    this.username = ''; this.password = '';
                }
                var hostPort = authority.split(':');
                this.hostname = hostPort[0].toLowerCase();
                this.port = hostPort[1] || '';
                this.host = this.hostname + (this.port ? ':' + this.port : '');
                this.pathname = m[3] || '/';
                this.search = m[4] || '';
                this.hash = m[5] || '';
                var defaultPorts = { 'http:': '80', 'https:': '443', 'ftp:': '21' };
                this.origin = this.protocol + '//' + this.hostname +
                    (this.port && this.port !== defaultPorts[this.protocol] ? ':' + this.port : '');
                this.href = this.protocol + '//' +
                    (this.username ? encodeURIComponent(this.username) + (this.password ? ':' + encodeURIComponent(this.password) : '') + '@' : '') +
                    this.host + this.pathname + this.search + this.hash;
                this.searchParams = new URLSearchParams(this.search);
            }
            URL.prototype.toString = function() { return this.href; };
            URL.prototype.toJSON = function() { return this.href; };
            URL.canParse = function(input, base) { try { new URL(input, base); return true; } catch(e) { return false; } };
            globalThis.URL = URL;

            // ── FileReader ────────────────────────────────────────────────────────
            function FileReader() {
                this.readyState = 0; // EMPTY
                this.result = null;
                this.error = null;
                this.onload = null;
                this.onerror = null;
                this.onloadend = null;
                this.onloadstart = null;
                this.onprogress = null;
                this.onabort = null;
                this._aborted = false;
            }
            FileReader.EMPTY = 0; FileReader.LOADING = 1; FileReader.DONE = 2;
            FileReader.prototype.EMPTY = 0; FileReader.prototype.LOADING = 1; FileReader.prototype.DONE = 2;
            FileReader.prototype._read = function(blob, encoding) {
                var self = this;
                self.readyState = 1;
                self._aborted = false;
                if (self.onloadstart) self.onloadstart({ type: 'loadstart', target: self });
                blob.text().then(function(text) {
                    if (self._aborted) return;
                    self.result = encoding ? text : text;
                    self.readyState = 2;
                    if (self.onload) self.onload({ type: 'load', target: self });
                    if (self.onloadend) self.onloadend({ type: 'loadend', target: self });
                }).catch(function(e) {
                    self.error = e;
                    self.readyState = 2;
                    if (self.onerror) self.onerror({ type: 'error', target: self });
                    if (self.onloadend) self.onloadend({ type: 'loadend', target: self });
                });
            };
            FileReader.prototype.readAsText = function(blob, encoding) { this._read(blob, encoding || 'utf-8'); };
            FileReader.prototype.readAsDataURL = function(blob) {
                var self = this;
                self.readyState = 1;
                blob.text().then(function(text) {
                    if (self._aborted) return;
                    self.result = 'data:' + (blob.type || 'application/octet-stream') + ';base64,' + btoa(text);
                    self.readyState = 2;
                    if (self.onload) self.onload({ type: 'load', target: self });
                    if (self.onloadend) self.onloadend({ type: 'loadend', target: self });
                });
            };
            FileReader.prototype.readAsArrayBuffer = function(blob) {
                var self = this;
                self.readyState = 1;
                blob.arrayBuffer().then(function(buf) {
                    if (self._aborted) return;
                    self.result = buf;
                    self.readyState = 2;
                    if (self.onload) self.onload({ type: 'load', target: self });
                    if (self.onloadend) self.onloadend({ type: 'loadend', target: self });
                });
            };
            FileReader.prototype.readAsBinaryString = function(blob) { this.readAsText(blob); };
            FileReader.prototype.abort = function() {
                this._aborted = true;
                this.readyState = 2;
                this.result = null;
                if (this.onabort) this.onabort({ type: 'abort', target: this });
                if (this.onloadend) this.onloadend({ type: 'loadend', target: this });
            };
            globalThis.FileReader = FileReader;

            // ── module (Node.js Module API) ───────────────────────────────────────
            // Next.js and many packages inspect Module._resolveFilename, Module._cache,
            // Module.prototype.require, and createRequire. We provide a shim that is
            // structurally compatible without implementing the full Node.js loader.
            function NodeModule(id, parent) {
                this.id = id || '';
                this.filename = id || '';
                this.loaded = true;
                this.parent = parent || null;
                this.children = [];
                this.exports = {};
                this.paths = [];
            }
            NodeModule.prototype.require = function(id) { return require(id); };
            NodeModule._cache = globalThis.__requireCache;
            NodeModule._extensions = { '.js': function(){}, '.json': function(){}, '.node': function(){} };
            NodeModule._resolveFilename = function(request, parent, isMain, options) {
                // Built-in modules (bare names or node: prefix) pass through
                var bare = request.replace(/^node:/, '');
                if (NodeModule.builtinModules.indexOf(bare) >= 0) return request;
                if (bare === 'module') return request;
                // Delegate to 3va's own resolver exposed via __resolvePath
                if (typeof __resolvePath === 'function') {
                    var base = (parent && parent.filename) ? parent.filename.replace(/\/[^\/]+$/, '') : undefined;
                    return base ? __resolvePath(request, base) : __resolvePath(request);
                }
                return request;
            };
            NodeModule._load = function(request, parent, isMain) {
                return require(request);
            };
            NodeModule.createRequire = function(filenameOrUrl) {
                var base = String(filenameOrUrl).replace(/\/[^\/]+$/, '');
                return function(id) {
                    var resolved = __resolvePath(id, base);
                    return require(resolved || id);
                };
            };
            NodeModule.createRequireFromPath = NodeModule.createRequire;
            NodeModule.builtinModules = [
                'assert','buffer','child_process','crypto','events','fs','http','https',
                'module','net','os','path','perf_hooks','process','querystring','stream',
                'string_decoder','tls','url','util','zlib'
            ];
            NodeModule.isBuiltin = function(name) {
                var n = name.replace(/^node:/, '');
                return NodeModule.builtinModules.indexOf(n) >= 0;
            };
            NodeModule.syncBuiltinESMExports = function() {};
            NodeModule.Module = NodeModule;

            globalThis.__requireCache['module'] = NodeModule;
            globalThis.__requireCache['node:module'] = NodeModule;
        })();
    "#)?;

    // ESM→CJS inline transformer for loading ESM packages via require()
    ctx.eval::<(), _>(r#"
        globalThis.__esmToCjs = function(src) {
            var lines = src.split('\n');
            var out = [];
            var i = 0;

            while (i < lines.length) {
                var line = lines[i];
                var trimmed = line.trim();

                // import 'specifier'  (side-effect import)
                var m;
                if ((m = trimmed.match(/^import\s+['"]([^'"]+)['"]\s*;?$/))) {
                    out.push('require(' + JSON.stringify(m[1]) + ');');
                    i++; continue;
                }
                // import defaultExport from 'specifier'
                if ((m = trimmed.match(/^import\s+(\w+)\s+from\s+['"]([^'"]+)['"]\s*;?$/))) {
                    out.push('var ' + m[1] + ' = (function(m){return m&&m.__esModule?m.default:m;})(require(' + JSON.stringify(m[2]) + '));');
                    i++; continue;
                }
                // import * as ns from 'specifier'
                if ((m = trimmed.match(/^import\s+\*\s+as\s+(\w+)\s+from\s+['"]([^'"]+)['"]\s*;?$/))) {
                    out.push('var ' + m[1] + ' = require(' + JSON.stringify(m[2]) + ');');
                    i++; continue;
                }
                // import { a, b as c } from 'specifier'  (single line)
                if ((m = trimmed.match(/^import\s+\{([^}]+)\}\s+from\s+['"]([^'"]+)['"]\s*;?$/))) {
                    var named = m[1].split(',').map(function(s){
                        s = s.trim();
                        var parts = s.split(/\s+as\s+/);
                        if (parts.length === 2) return parts[0].trim() + ': ' + parts[1].trim();
                        return s;
                    }).join(', ');
                    out.push('var {' + named + '} = require(' + JSON.stringify(m[2]) + ');');
                    i++; continue;
                }
                // import defaultExport, { a, b } from 'specifier'
                if ((m = trimmed.match(/^import\s+(\w+)\s*,\s*\{([^}]+)\}\s+from\s+['"]([^'"]+)['"]\s*;?$/))) {
                    out.push('var __mod' + i + ' = require(' + JSON.stringify(m[3]) + ');');
                    out.push('var ' + m[1] + ' = __mod' + i + '.default || __mod' + i + ';');
                    var named2 = m[2].split(',').map(function(s){ return s.trim(); }).join(', ');
                    out.push('var {' + named2 + '} = __mod' + i + ';');
                    i++; continue;
                }
                // export default expression;
                if ((m = trimmed.match(/^export\s+default\s+(.*)/))) {
                    out.push('module.exports.default = module.exports = ' + m[1]);
                    i++; continue;
                }
                // export { a, b as c }
                if ((m = trimmed.match(/^export\s+\{([^}]+)\}\s*;?$/))) {
                    m[1].split(',').forEach(function(s) {
                        s = s.trim();
                        var parts = s.split(/\s+as\s+/);
                        var local = parts[0].trim(), exported = (parts[1] || parts[0]).trim();
                        out.push('module.exports.' + exported + ' = ' + local + ';');
                    });
                    i++; continue;
                }
                // export { a, b } from 'specifier'  (re-export)
                if ((m = trimmed.match(/^export\s+\{([^}]+)\}\s+from\s+['"]([^'"]+)['"]\s*;?$/))) {
                    out.push('(function(){var __re = require(' + JSON.stringify(m[2]) + ');');
                    m[1].split(',').forEach(function(s) {
                        s = s.trim();
                        var parts = s.split(/\s+as\s+/);
                        var local = parts[0].trim(), exported = (parts[1] || parts[0]).trim();
                        out.push('module.exports.' + exported + ' = __re.' + local + ';');
                    });
                    out.push('})();');
                    i++; continue;
                }
                // export * from 'specifier'
                if ((m = trimmed.match(/^export\s+\*\s+from\s+['"]([^'"]+)['"]\s*;?$/))) {
                    out.push('(function(){var __re=require(' + JSON.stringify(m[1]) + ');for(var __k in __re){if(__k!=="default")module.exports[__k]=__re[__k];}})();');
                    i++; continue;
                }
                // export const/let/var/function/class X ...
                if ((m = trimmed.match(/^export\s+(const|let|var|function|class|async\s+function)\s+(\w+)/))) {
                    var decl = trimmed.replace(/^export\s+/, '');
                    var exportName = m[2];
                    out.push(decl);
                    out.push('module.exports.' + exportName + ' = ' + exportName + ';');
                    i++; continue;
                }

                out.push(line);
                i++;
            }

            out.push('if(typeof module.exports.default==="undefined"&&Object.keys(module.exports).length>0){module.exports.__esModule=true;}');
            return out.join('\n');
        };
    "#)?;

    // JS-level require() implementation
    // This avoids rquickjs Value<'js> lifetime issues by keeping all evaluation in JS.
    ctx.eval::<(), _>(r#"
        globalThis.require = function(path) {
            // Strip node: prefix for built-in module resolution
            if (path.indexOf('node:') === 0) {
                var bare = path.slice(5);
                if (globalThis.__requireCache[bare] !== undefined) {
                    return globalThis.__requireCache[bare];
                }
                if (globalThis.__requireCache[path] !== undefined) {
                    return globalThis.__requireCache[path];
                }
            }

            // Check built-ins and bare-name cache first (before path resolution)
            if (globalThis.__requireCache[path] !== undefined) {
                return globalThis.__requireCache[path];
            }

            // Resolve relative to the currently-executing file's directory
            var resolvedPath = __resolvePath(path, globalThis.__dirname);

            // Check cache by resolved path
            if (globalThis.__requireCache[resolvedPath] !== undefined) {
                return globalThis.__requireCache[resolvedPath];
            }

            // Read and (if needed) transpile the file (path is already absolute)
            var source = __readFile(resolvedPath);

            // Compute dirname
            var dirname = resolvedPath.replace(/\/[^\/]*$/, '') || '.';
            var filename = resolvedPath;

            var result;

            // JSON files: parse directly instead of eval-ing as JS
            if (resolvedPath.endsWith('.json')) {
                result = JSON.parse(source);
                globalThis.__requireCache[resolvedPath] = result;
                return result;
            }

            // Save and restore outer module state
            var savedModule = globalThis.module;
            var savedExports = globalThis.exports;
            var savedFilename = globalThis.__filename;
            var savedDirname = globalThis.__dirname;

            globalThis.module = { exports: {} };
            globalThis.exports = globalThis.module.exports;
            globalThis.__filename = filename;
            globalThis.__dirname = dirname;

            // If the file uses ESM syntax, inline-convert to CJS before wrapping.
            if (/^\s*(import\s|import\{|export\s|export\{|export\s*default)/m.test(source)) {
                source = __esmToCjs(source);
            }

            // Execute the module with CJS wrapper
            // We use eval() with the module wrapper
            var wrapper = '(function(exports, module, __filename, __dirname) {\n' +
                source +
                '\n})(globalThis.exports, globalThis.module, globalThis.__filename, globalThis.__dirname);';
            eval(wrapper);

            result = globalThis.module.exports;

            // Restore outer state
            globalThis.module = savedModule;
            globalThis.exports = savedExports;
            globalThis.__filename = savedFilename;
            globalThis.__dirname = savedDirname;

            // Cache the result
            globalThis.__requireCache[resolvedPath] = result;

            return result;
        };

        // Node.js require.extensions — used by packages to register hooks for custom file types
        globalThis.require.extensions = {};
    "#)?;

    Ok(())
}

/// Split a bare specifier into (package_name, optional_subpath).
/// e.g. "es-errors/eval" → ("es-errors", Some("eval"))
///      "@scope/pkg"      → ("@scope/pkg", None)
///      "@scope/pkg/sub"  → ("@scope/pkg", Some("sub"))
fn split_bare_specifier(spec: &str) -> (&str, Option<&str>) {
    if spec.starts_with('@') {
        // Scoped: first slash separates scope/name; second slash starts subpath
        if let Some(slash1) = spec.find('/') {
            let after = &spec[slash1 + 1..];
            if let Some(slash2) = after.find('/') {
                let name_end = slash1 + 1 + slash2;
                return (&spec[..name_end], Some(&spec[name_end + 1..]));
            }
        }
        (spec, None)
    } else if let Some(slash) = spec.find('/') {
        (&spec[..slash], Some(&spec[slash + 1..]))
    } else {
        (spec, None)
    }
}

/// Resolve a module path relative to `basedir` (or CWD if None).
pub fn resolve_path_from(path: &str, basedir: Option<&str>) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let base_dir = basedir
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| cwd.clone());

    if path.starts_with("./") || path.starts_with("../") {
        resolve_file_path(&base_dir.join(path))
    } else if path.starts_with('/') {
        resolve_file_path(&PathBuf::from(path))
    } else {
        // Bare specifier: may have a subpath like 'pkg/subpath' or '@scope/pkg/sub'
        let (pkg_name, subpath) = split_bare_specifier(path);
        resolve_node_module_from(&base_dir, &cwd, pkg_name, subpath)
    }
}

/// Convenience wrapper using CWD as the base.
pub fn resolve_path(path: &str) -> PathBuf {
    resolve_path_from(path, None)
}

/// Try path as-is, then with common extensions appended, then index files.
fn resolve_file_path(base: &Path) -> PathBuf {
    if base.is_file() {
        return base.to_path_buf();
    }
    // If the path is a directory with a package.json, resolve its entry point
    if base.is_dir() && base.join("package.json").is_file() {
        let entry = resolve_node_module_entry(base);
        if entry.is_file() {
            return entry;
        }
    }
    // Append extensions as strings to avoid PathBuf::with_extension clobbering
    // filenames that already contain dots (e.g. "Reflect.getPrototypeOf").
    let base_str = base.to_string_lossy();
    for ext in &["js", "ts", "mjs", "cjs"] {
        let p = PathBuf::from(format!("{}.{}", base_str, ext));
        if p.is_file() {
            return p;
        }
    }
    for index in &["index.js", "index.ts"] {
        let p = base.join(index);
        if p.is_file() {
            return p;
        }
    }
    base.to_path_buf()
}

/// Walk up from `start` toward `root` looking for node_modules/<name>.
/// If `subpath` is Some, resolve that file within the package dir instead of using the entry point.
fn resolve_node_module_from(
    start: &Path,
    root: &Path,
    name: &str,
    subpath: Option<&str>,
) -> PathBuf {
    let mut dir = start.to_path_buf();
    loop {
        let pkg_dir = dir.join("node_modules").join(name);
        if pkg_dir.is_dir() {
            return resolve_in_pkg_dir(&pkg_dir, subpath);
        }
        if dir == *root || !dir.pop() {
            break;
        }
    }
    // Final fallback: root/node_modules/<name>
    let pkg_dir = root.join("node_modules").join(name);
    resolve_in_pkg_dir(&pkg_dir, subpath)
}

fn resolve_in_pkg_dir(pkg_dir: &Path, subpath: Option<&str>) -> PathBuf {
    if let Some(sub) = subpath {
        let p = pkg_dir.join(sub);
        // First try resolving as a file (with extension probing)
        let resolved = resolve_file_path(&p);
        if resolved.is_file() {
            return resolved;
        }
        // If the path is a directory, look for its package.json entry point
        // (common for packages like @swc/helpers/_/_interop_require_default/)
        if p.is_dir() {
            let entry = resolve_node_module_entry(&p);
            if entry.is_file() {
                return entry;
            }
        }
        p
    } else {
        let resolved = resolve_node_module_entry(pkg_dir);
        if resolved.is_file() {
            return resolved;
        }
        pkg_dir.to_path_buf()
    }
}

/// Extract the CJS-compatible path from a package exports value.
/// Walks nested objects preferring "require" and "default" conditions.
fn exports_cjs_path(val: &serde_json::Value) -> Option<&str> {
    match val {
        serde_json::Value::String(s) => Some(s.as_str()),
        serde_json::Value::Object(_) => {
            // Prefer require condition (CJS), then node, then default
            for key in &["require", "node", "default"] {
                if let Some(child) = val.get(key)
                    && let Some(path) = exports_cjs_path(child)
                {
                    return Some(path);
                }
            }
            None
        }
        _ => None,
    }
}

/// Given an already-located package directory, find its entry point.
fn resolve_node_module_entry(pkg_dir: &Path) -> PathBuf {
    let pkg_json_path = pkg_dir.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_json_path)
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
    {
        if let Some(exports) = json.get("exports") {
            // Standard form: exports["."] is the entry condition map
            if let Some(exp_dot) = exports.get(".")
                && let Some(path_str) = exports_cjs_path(exp_dot)
            {
                let p = pkg_dir.join(path_str.trim_start_matches("./"));
                let resolved = resolve_file_path(&p);
                if resolved.is_file() {
                    return resolved;
                }
            }
            // Shorthand form: exports itself IS the condition map (no "." key)
            if exports.is_object()
                && !exports.as_object().is_some_and(|m| m.contains_key("."))
                && let Some(path_str) = exports_cjs_path(exports)
            {
                let p = pkg_dir.join(path_str.trim_start_matches("./"));
                let resolved = resolve_file_path(&p);
                if resolved.is_file() {
                    return resolved;
                }
            }
        }
        // Fallback to "main"
        if let Some(main) = json["main"].as_str() {
            let main_path = pkg_dir.join(main);
            let resolved = resolve_file_path(&main_path);
            if resolved.is_file() {
                return resolved;
            }
        }
    }
    // Fallback: index.js / index.ts
    resolve_file_path(pkg_dir)
}
