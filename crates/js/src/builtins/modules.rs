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
        } else if path_str.ends_with(".ts")
            || path_str.ends_with(".mts")
            || path_str.ends_with(".cts")
        {
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
                inspect: function(obj, opts) {
                    var customSym = typeof Symbol !== 'undefined' && Symbol.for ? Symbol.for('nodejs.util.inspect.custom') : null;
                    var depth = (opts && typeof opts.depth === 'number') ? opts.depth : 2;
                    function _ins(val, d, seen) {
                        if (val === null) return 'null';
                        if (val === undefined) return 'undefined';
                        if (typeof val === 'string') return "'" + val.replace(/\\/g,'\\\\').replace(/'/g,"\\'").replace(/\n/g,'\\n').replace(/\r/g,'\\r').replace(/\t/g,'\\t') + "'";
                        if (typeof val === 'number' || typeof val === 'boolean' || typeof val === 'bigint') return String(val);
                        if (typeof val === 'function') return '[Function: ' + (val.name || 'anonymous') + ']';
                        if (typeof val === 'symbol') return val.toString();
                        if (val instanceof RegExp) return val.toString();
                        if (val instanceof Date) return val.toISOString();
                        if (val instanceof Error) return '[' + (val.constructor && val.constructor.name || 'Error') + ': ' + val.message + ']';
                        if (customSym && typeof val[customSym] === 'function') {
                            try { return String(val[customSym](d, opts || {})); } catch(e) {}
                        }
                        if (seen.indexOf(val) !== -1) return '[Circular *]';
                        var seen2 = seen.concat([val]);
                        if (Array.isArray(val)) {
                            if (d > depth) return '[Array]';
                            return '[ ' + val.map(function(v) { return _ins(v, d+1, seen2); }).join(', ') + ' ]';
                        }
                        if (typeof val === 'object') {
                            if (d > depth) return '[Object]';
                            var keys = Object.keys(val);
                            if (keys.length === 0) return '{}';
                            var indent = '  '.repeat ? '  '.repeat(d+1) : Array(d+2).join('  ');
                            var close = '  '.repeat ? '  '.repeat(d) : Array(d+1).join('  ');
                            return '{\n' + keys.map(function(k) {
                                return indent + k + ': ' + _ins(val[k], d+1, seen2);
                            }).join(',\n') + '\n' + close + '}';
                        }
                        return String(val);
                    }
                    return _ins(obj, 0, []);
                },
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
                parseArgs: function(config) {
                    config = config || {};
                    var argv = config.args || (typeof process !== 'undefined' && Array.isArray(process.argv) ? process.argv.slice(2) : []);
                    var options = config.options || {};
                    var strict = config.strict !== false;
                    var allowPositionals = config.allowPositionals !== false;
                    var values = {};
                    var positionals = [];
                    var tokens = [];
                    Object.keys(options).forEach(function(key) { if (options[key].default !== undefined) values[key] = options[key].default; });
                    var i = 0;
                    while (i < argv.length) {
                        var arg = argv[i];
                        if (arg === '--') {
                            for (var j = i + 1; j < argv.length; j++) { positionals.push(argv[j]); tokens.push({ kind: 'positional', value: argv[j] }); }
                            break;
                        }
                        if (arg.slice(0, 2) === '--') {
                            var eqIdx = arg.indexOf('=');
                            var name = eqIdx !== -1 ? arg.slice(2, eqIdx) : arg.slice(2);
                            var opt = options[name];
                            if (!opt && strict) throw new TypeError('Unknown option \'' + arg + '\'');
                            var type = opt ? (opt.type || 'string') : 'string';
                            var val;
                            if (eqIdx !== -1) {
                                val = arg.slice(eqIdx + 1);
                            } else if (type === 'boolean') {
                                val = true;
                            } else {
                                val = (i + 1 < argv.length && argv[i + 1].slice(0, 1) !== '-') ? argv[++i] : '';
                            }
                            if (type === 'boolean') val = (val === true || val === 'true' || val === '1');
                            if (opt && opt.multiple) { if (!Array.isArray(values[name])) values[name] = []; values[name].push(val); }
                            else values[name] = val;
                            tokens.push({ kind: 'option', name: name, rawName: '--' + name, value: typeof val === 'boolean' ? undefined : val, inlineValue: eqIdx !== -1 });
                        } else if (arg.slice(0, 1) === '-' && arg.length > 1) {
                            var sname = arg.slice(1);
                            tokens.push({ kind: 'option', name: sname, rawName: '-' + sname, value: undefined, inlineValue: false });
                        } else {
                            if (!allowPositionals && strict) throw new TypeError('Unexpected positional argument \'' + arg + '\'');
                            positionals.push(arg);
                            tokens.push({ kind: 'positional', value: arg });
                        }
                        i++;
                    }
                    var result = { values: values, positionals: positionals };
                    if (config.tokens) result.tokens = tokens;
                    return result;
                },
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
            EventEmitter.prototype.listeners = function(ev) {
                return (this._events[ev] || []).map(function(f) { return f._orig || f; });
            };
            EventEmitter.prototype.rawListeners = function(ev) { return (this._events[ev] || []).slice(); };
            EventEmitter.prototype.listenerCount = function(ev) { return (this._events[ev] || []).length; };
            EventEmitter.prototype.setMaxListeners = function(n) { this._maxListeners = n; return this; };
            EventEmitter.prototype.getMaxListeners = function() { return this._maxListeners; };
            EventEmitter.prototype.eventNames = function() { return Object.keys(this._events).filter(function(k) { return this._events[k] && this._events[k].length > 0; }, this); };
            EventEmitter.prototype.prependListener = function(ev, fn) {
                if (!this._events[ev]) this._events[ev] = [];
                this._events[ev].unshift(fn);
                return this;
            };
            EventEmitter.prototype.prependOnceListener = function(ev, fn) {
                var self = this;
                function wrapper() { self.removeListener(ev, wrapper); fn.apply(this, arguments); }
                wrapper._orig = fn;
                return this.prependListener(ev, wrapper);
            };
            EventEmitter.defaultMaxListeners = 10;
            EventEmitter.setMaxListeners = function(n) { EventEmitter.defaultMaxListeners = n; };
            EventEmitter.getMaxListeners = function() { return EventEmitter.defaultMaxListeners; };
            EventEmitter.listenerCount = function(emitter, ev) { return emitter.listenerCount(ev); };
            EventEmitter.EventEmitter = EventEmitter;
            globalThis.__requireCache['events'] = EventEmitter;

            // ── stream ────────────────────────────────────────────────────────────
            function Stream() { EventEmitter.call(this); }
            util.inherits(Stream, EventEmitter);
            Stream.prototype.pipe = function() { return this; };
            // ── Readable ──────────────────────────────────────────────────────────
            function Readable(opts) {
                Stream.call(this);
                this.readable = true;
                this._readableState = { objectMode: !!(opts && opts.objectMode), highWaterMark: (opts && opts.highWaterMark) || 16384, length: 0, buffer: [], ended: false, flowing: false };
                if (opts && typeof opts.read === 'function') this._read = opts.read;
            }
            util.inherits(Readable, Stream);
            Readable.prototype._read = function(size) {};
            Readable.prototype.read = function(n) {
                if (!this._readableState.flowing) this._read(n || 0);
                var buf = this._readableState.buffer.splice(0);
                return buf.length ? Buffer.concat ? Buffer.concat(buf) : buf[0] : null;
            };
            Readable.prototype.push = function(chunk) {
                if (chunk === null) {
                    this._readableState.ended = true;
                    this.emit('end');
                } else {
                    this._readableState.buffer.push(chunk);
                    this.emit('data', chunk);
                }
                return !this._readableState.ended;
            };
            Readable.prototype.unshift = function(chunk) { if (chunk != null) this.emit('data', chunk); return this; };
            Readable.prototype.pipe = function(dest, opts) {
                var self = this;
                this.on('data', function(c) { if (dest.write(c) === false && self.pause) self.pause(); });
                this.on('end', function() { if (!opts || !opts.end === false) dest.end(); });
                dest.emit('pipe', this);
                return dest;
            };
            Readable.prototype.unpipe = function(dest) { return this; };
            Readable.prototype.resume = function() { this._readableState.flowing = true; this._read(0); return this; };
            Readable.prototype.pause = function() { this._readableState.flowing = false; return this; };
            Readable.prototype.setEncoding = function(enc) { this._encoding = enc; return this; };
            Readable.prototype.destroy = function(err) { if (err) this.emit('error', err); this.emit('close'); return this; };
            Readable.prototype[Symbol.asyncIterator] = function() {
                var self = this, done = false;
                return {
                    next: function() {
                        return new Promise(function(resolve) {
                            if (done) return resolve({ done: true });
                            self.once('data', function(chunk) { resolve({ value: chunk, done: false }); });
                            self.once('end', function() { done = true; resolve({ done: true }); });
                        });
                    },
                    return: function() { return Promise.resolve({ done: true }); }
                };
            };

            // ── Writable ──────────────────────────────────────────────────────────
            function Writable(opts) {
                Stream.call(this);
                this.writable = true;
                this._writableState = { objectMode: !!(opts && opts.objectMode), highWaterMark: (opts && opts.highWaterMark) || 16384, length: 0, finished: false };
                if (opts && typeof opts.write === 'function') this._write = opts.write;
                if (opts && typeof opts.final === 'function') this._final = opts.final;
            }
            util.inherits(Writable, Stream);
            Writable.prototype._write = function(chunk, encoding, callback) { callback(); };
            Writable.prototype._final = function(callback) { callback(); };
            Writable.prototype.write = function(chunk, encoding, cb) {
                if (typeof encoding === 'function') { cb = encoding; encoding = 'utf8'; }
                var self = this;
                this._write(chunk, encoding || 'utf8', function(err) {
                    if (err) { self.emit('error', err); if (cb) cb(err); return; }
                    if (cb) cb(null);
                });
                return true;
            };
            Writable.prototype.end = function(chunk, encoding, cb) {
                if (typeof chunk === 'function') { cb = chunk; chunk = null; }
                if (typeof encoding === 'function') { cb = encoding; encoding = null; }
                var self = this;
                function finish() {
                    self._writableState.finished = true;
                    self._final(function(err) {
                        if (err) self.emit('error', err);
                        self.emit('finish');
                        self.emit('close');
                        if (cb) cb(err || null);
                    });
                }
                if (chunk != null) { this.write(chunk, encoding, function(err) { if (!err) finish(); else if (cb) cb(err); }); }
                else { finish(); }
            };
            Writable.prototype.destroy = function(err) { if (err) this.emit('error', err); this.emit('close'); return this; };
            Writable.prototype.setDefaultEncoding = function() { return this; };
            Writable.prototype.cork = function() {};
            Writable.prototype.uncork = function() {};

            // ── Transform ─────────────────────────────────────────────────────────
            function Transform(opts) {
                Readable.call(this, opts);
                this.writable = true;
                this._writableState = { objectMode: !!(opts && opts.objectMode), finished: false };
                if (opts && typeof opts.transform === 'function') this._transform = opts.transform;
                if (opts && typeof opts.flush === 'function') this._flush = opts.flush;
            }
            util.inherits(Transform, Readable);
            Transform.prototype._transform = function(chunk, encoding, callback) { this.push(chunk); callback(); };
            Transform.prototype._flush = function(callback) { callback(); };
            Transform.prototype.write = function(chunk, encoding, cb) {
                if (typeof encoding === 'function') { cb = encoding; encoding = 'utf8'; }
                var self = this;
                this._transform(chunk, encoding || 'utf8', function(err, data) {
                    if (err) { self.emit('error', err); if (cb) cb(err); return; }
                    if (data != null) self.push(data);
                    if (cb) cb(null);
                });
                return true;
            };
            Transform.prototype.end = function(chunk, encoding, cb) {
                if (typeof chunk === 'function') { cb = chunk; chunk = null; }
                if (typeof encoding === 'function') { cb = encoding; encoding = null; }
                var self = this;
                function doFlush() {
                    self._flush(function(err, data) {
                        if (err) { self.emit('error', err); if (cb) cb(err); return; }
                        if (data != null) self.push(data);
                        self.push(null);
                        self._writableState.finished = true;
                        self.emit('finish');
                        if (cb) cb(null);
                    });
                }
                if (chunk != null) { this.write(chunk, encoding, function(err) { if (!err) doFlush(); else if (cb) cb(err); }); }
                else { doFlush(); }
            };
            Transform.prototype.destroy = function(err) { if (err) this.emit('error', err); this.emit('close'); return this; };
            Transform.prototype.cork = function() {};
            Transform.prototype.uncork = function() {};

            // ── PassThrough ───────────────────────────────────────────────────────
            function PassThrough(opts) { Transform.call(this, opts); }
            util.inherits(PassThrough, Transform);
            PassThrough.prototype._transform = function(chunk, enc, cb) { this.push(chunk); cb(); };

            // ── Duplex ────────────────────────────────────────────────────────────
            function Duplex(opts) {
                Readable.call(this, opts);
                this.writable = true;
                this._writableState = { finished: false };
                if (opts && typeof opts.write === 'function') this._write = opts.write;
            }
            util.inherits(Duplex, Readable);
            Duplex.prototype._write = Writable.prototype._write;
            Duplex.prototype.write = Writable.prototype.write;
            Duplex.prototype.end = Writable.prototype.end;

            var stream = {
                Stream: Stream, Readable: Readable, Writable: Writable,
                Transform: Transform, PassThrough: PassThrough, Duplex: Duplex,
                isReadable: function(s) { return !!(s && s.readable); },
                isWritable: function(s) { return !!(s && s.writable); },
                isStream: function(s) { return !!(s && (s.readable || s.writable)); },
            };
            globalThis.__requireCache['stream'] = stream;
            globalThis.__requireCache['stream/web'] = stream;
            globalThis.__requireCache['readable-stream'] = stream;

            // ── path ──────────────────────────────────────────────────────────────
            function makePath(sep, isAbsFn) {
                function normalize(p) {
                    var abs = isAbsFn(p);
                    var parts = p.split(sep).filter(function(s,i) { return s !== '' || i === 0; });
                    var out = [];
                    for (var i = 0; i < parts.length; i++) {
                        if (parts[i] === '..') { if (out.length > 0 && out[out.length-1] !== '..') out.pop(); else if (!abs) out.push('..'); }
                        else if (parts[i] !== '.') out.push(parts[i]);
                    }
                    var r = out.join(sep);
                    if (abs && r[0] !== sep) r = sep + r;
                    return r || (abs ? sep : '.');
                }
                var m = {
                    sep: sep,
                    delimiter: sep === '/' ? ':' : ';',
                    join: function() {
                        var args = Array.prototype.slice.call(arguments).filter(function(a) { return a != null; });
                        return normalize(args.join(sep));
                    },
                    resolve: function() {
                        var args = Array.prototype.slice.call(arguments);
                        var resolved = '';
                        for (var i = args.length - 1; i >= -1; i--) {
                            var p = i >= 0 ? String(args[i]) : ((typeof process !== 'undefined' && process.cwd) ? process.cwd() : '/');
                            if (!p) continue;
                            resolved = p + sep + resolved;
                            if (isAbsFn(p)) break;
                        }
                        return normalize(resolved);
                    },
                    dirname: function(p) {
                        p = String(p);
                        var i = p.lastIndexOf(sep);
                        if (i < 0) return '.';
                        if (i === 0) return sep;
                        return p.slice(0, i);
                    },
                    basename: function(p, ext) {
                        p = String(p);
                        var b = p.slice(p.lastIndexOf(sep) + 1);
                        if (b === '' && p.length > 1) b = p.slice(p.slice(0, p.length - 1).lastIndexOf(sep) + 1, p.length - 1);
                        return ext && b.endsWith(ext) ? b.slice(0, b.length - ext.length) : b;
                    },
                    extname: function(p) {
                        p = String(p);
                        var b = p.slice(p.lastIndexOf(sep) + 1);
                        var i = b.lastIndexOf('.');
                        return i > 0 ? b.slice(i) : '';
                    },
                    isAbsolute: isAbsFn,
                    normalize: normalize,
                    relative: function(from, to) {
                        from = m.resolve(from); to = m.resolve(to);
                        var f = from.split(sep).filter(Boolean);
                        var t = to.split(sep).filter(Boolean);
                        var common = 0;
                        while (common < f.length && common < t.length && f[common] === t[common]) common++;
                        var up = f.length - common;
                        var parts = [];
                        for (var i = 0; i < up; i++) parts.push('..');
                        parts = parts.concat(t.slice(common));
                        return parts.join(sep) || '.';
                    },
                    parse: function(p) {
                        p = String(p);
                        var root = isAbsFn(p) ? sep : '';
                        var base = m.basename(p);
                        var ext  = m.extname(p);
                        var name = ext ? base.slice(0, base.length - ext.length) : base;
                        return { root: root, dir: m.dirname(p), base: base, ext: ext, name: name };
                    },
                    format: function(obj) {
                        return (obj.dir ? obj.dir + sep : '') + (obj.base || (obj.name || '') + (obj.ext || ''));
                    },
                    toNamespacedPath: function(p) { return p; },
                    matchesGlob: function() { return false; },
                };
                return m;
            }
            var path = makePath('/', function(p) { return String(p).startsWith('/'); });
            var pathWin32 = makePath('\\', function(p) { return /^[A-Za-z]:[\\\/]/.test(p) || String(p).startsWith('\\\\'); });
            path.posix = path;
            path.win32 = pathWin32;
            pathWin32.posix = path;
            pathWin32.win32 = pathWin32;
            globalThis.__requireCache['path'] = path;
            globalThis.__requireCache['node:path'] = path;
            globalThis.__requireCache['path/posix']     = path;
            globalThis.__requireCache['node:path/posix'] = path;
            globalThis.__requireCache['path/win32']     = pathWin32;
            globalThis.__requireCache['node:path/win32'] = pathWin32;

            // ── buffer ────────────────────────────────────────────────────────────
            var bufMod = { Buffer: globalThis.Buffer };
            globalThis.__requireCache['buffer'] = bufMod;
            globalThis.__requireCache['node:buffer'] = bufMod;

            // ── os ────────────────────────────────────────────────────────────────
            var _osPlatform = (process && process.platform) || 'linux';
            var _osArch     = (process && process.arch)     || 'x64';
            var os = {
                platform: function() { return _osPlatform; },
                type: function() {
                    return _osPlatform === 'darwin' ? 'Darwin' : _osPlatform === 'win32' ? 'Windows_NT' : 'Linux';
                },
                arch: function() { return _osArch; },
                hostname: function() { return typeof __osHostname === 'function' ? __osHostname() : 'localhost'; },
                homedir: function() {
                    return (process && process.env && (process.env.HOME || process.env.USERPROFILE)) || '/home/user';
                },
                tmpdir: function() {
                    return (process && process.env && (process.env.TMPDIR || process.env.TEMP)) || '/tmp';
                },
                EOL: _osPlatform === 'win32' ? '\r\n' : '\n',
                cpus: function() {
                    // Read /proc/cpuinfo count if available (via env hint), return stubs
                    var n = (typeof __osCpuCount === 'function') ? __osCpuCount() : 1;
                    var arr = [];
                    for (var i = 0; i < n; i++) arr.push({ model: 'Generic', speed: 0, times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 } });
                    return arr;
                },
                totalmem: function() { return typeof __osMemTotal === 'function' ? __osMemTotal() : 1073741824; },
                freemem:  function() { return typeof __osMemFree  === 'function' ? __osMemFree()  :  536870912; },
                networkInterfaces: function() { return {}; },
                userInfo: function(opts) {
                    var u = (process && process.env && process.env.USER) || 'user';
                    var h = (process && process.env && process.env.HOME) || '/home/' + u;
                    return { username: u, uid: -1, gid: -1, shell: '/bin/sh', homedir: h };
                },
                release: function() { return '6.0.0'; },
                version: function() { return '#1 SMP'; },
                uptime: function() { return typeof __osUptime === 'function' ? __osUptime() : 0; },
                loadavg: function() {
                    // Parse /proc/loadavg if available (value injected as env hint)
                    return [0, 0, 0];
                },
                endianness: function() { return 'LE'; },
                availableParallelism: function() { return 1; },
                constants: {
                    signals: { SIGHUP: 1, SIGINT: 2, SIGTERM: 15, SIGKILL: 9, SIGPIPE: 13, SIGCHLD: 17, SIGUSR1: 10, SIGUSR2: 12 },
                    errno: { ENOENT: -2, EACCES: -13, EEXIST: -17, EISDIR: -21, ENOTDIR: -20, ENOTEMPTY: -39, EPERM: -1 },
                    priority: { PRIORITY_LOW: 19, PRIORITY_BELOW_NORMAL: 10, PRIORITY_NORMAL: 0, PRIORITY_ABOVE_NORMAL: -7, PRIORITY_HIGH: -14, PRIORITY_HIGHEST: -20 }
                },
                getPriority: function() { return 0; },
                setPriority: function() {},
                machine: function() { return _osArch; },
            };
            globalThis.__requireCache['os'] = os;
            globalThis.__requireCache['node:os'] = os;

            // ── url ───────────────────────────────────────────────────────────────
            var url = {
                parse: function(s) { try { var u = new URL(s); return { protocol: u.protocol, host: u.host, hostname: u.hostname, port: u.port, pathname: u.pathname, search: u.search, hash: u.hash, href: u.href, path: (u.pathname + (u.search || '')), slashes: true, auth: null, query: u.search ? u.search.slice(1) : null }; } catch(e) { return { href: s }; } },
                format: function(obj) {
                    if (typeof obj === 'string') return obj;
                    if (obj && typeof obj.href === 'string' && !obj.protocol && !obj.host) return obj.href;
                    return (obj.protocol ? obj.protocol + '//' : '') + (obj.auth ? obj.auth + '@' : '') + (obj.host || obj.hostname || '') + (obj.pathname || '/') + (obj.search || '');
                },
                resolve: function(from, to) {
                    try { return new URL(to, from).href; } catch(e) { return to; }
                },
                URL: URL,
                URLSearchParams: URLSearchParams,

                // fileURLToPath('file:///path/to/file') → '/path/to/file'
                // Used by Vite, ESM loaders, and any code with import.meta.url
                fileURLToPath: function(fileUrl) {
                    var href = typeof fileUrl === 'string' ? fileUrl : (fileUrl && fileUrl.href);
                    if (!href) throw new TypeError('fileURLToPath: argument must be a file URL string or URL object');
                    if (!href.startsWith('file://')) throw new TypeError('fileURLToPath: argument must use the file: protocol, got ' + href);
                    // Strip 'file://' (authority is empty for local files)
                    var path = href.slice('file://'.length);
                    // Remove optional empty authority (file:///path → /path)
                    if (path.startsWith('/')) {
                        // Decode %XX sequences in the path
                        try { path = decodeURIComponent(path.replace(/\+/g, '%2B')); } catch(e) {}
                    }
                    return path;
                },

                // pathToFileURL('/path/to/file') → URL { href: 'file:///path/to/file' }
                pathToFileURL: function(filePath) {
                    if (typeof filePath !== 'string') throw new TypeError('pathToFileURL: argument must be a string');
                    var abs = filePath.startsWith('/') ? filePath : '/' + filePath;
                    // Encode special characters (keep / as-is)
                    var encoded = abs.split('/').map(function(seg) {
                        return seg.replace(/[^A-Za-z0-9\-._~!$&'()*+,;=:@]/g, function(c) {
                            return encodeURIComponent(c);
                        });
                    }).join('/');
                    var href = 'file://' + encoded;
                    try { return new URL(href); } catch(e) { return { href: href, pathname: abs, protocol: 'file:', toString: function() { return href; } }; }
                }
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
            // Structural deep equality — handles circular refs, TypedArrays, Maps, Sets, Dates
            function _deepEq(a, b, strict, seen) {
                if (strict ? a === b : a == b) return true;
                if (a === null || b === null || a === undefined || b === undefined) return a === b;
                if (typeof a !== typeof b) return false;
                if (typeof a !== 'object' && typeof a !== 'function') return strict ? a === b : a == b;
                // Circular reference guard
                for (var i = 0; i < seen.length; i++) { if (seen[i][0] === a && seen[i][1] === b) return true; }
                var seen2 = seen.concat([[a, b]]);
                // Date
                if (a instanceof Date && b instanceof Date) return a.getTime() === b.getTime();
                // RegExp
                if (a instanceof RegExp && b instanceof RegExp) return a.toString() === b.toString();
                // TypedArray / Buffer
                if (a instanceof Uint8Array && b instanceof Uint8Array) {
                    if (a.length !== b.length) return false;
                    for (var i=0;i<a.length;i++) if (a[i]!==b[i]) return false;
                    return true;
                }
                // Map
                if (typeof Map !== 'undefined' && a instanceof Map && b instanceof Map) {
                    if (a.size !== b.size) return false;
                    var ok = true;
                    a.forEach(function(v,k) { if (!b.has(k) || !_deepEq(v, b.get(k), strict, seen2)) ok = false; });
                    return ok;
                }
                // Set
                if (typeof Set !== 'undefined' && a instanceof Set && b instanceof Set) {
                    if (a.size !== b.size) return false;
                    var ok = true;
                    a.forEach(function(v) {
                        var found = false;
                        b.forEach(function(w) { if (!found && _deepEq(v, w, strict, seen2)) found = true; });
                        if (!found) ok = false;
                    });
                    return ok;
                }
                // Array
                if (Array.isArray(a) !== Array.isArray(b)) return false;
                if (Array.isArray(a)) {
                    if (a.length !== b.length) return false;
                    for (var i=0;i<a.length;i++) if (!_deepEq(a[i],b[i],strict,seen2)) return false;
                    return true;
                }
                // Plain object
                var ka = Object.keys(a), kb = Object.keys(b);
                if (ka.length !== kb.length) return false;
                for (var i=0;i<ka.length;i++) {
                    var k = ka[i];
                    if (!Object.prototype.hasOwnProperty.call(b, k)) return false;
                    if (!_deepEq(a[k], b[k], strict, seen2)) return false;
                }
                return true;
            }

            assert.ok = assert;
            assert.equal = function(a, b, msg) { if (a != b) throw new Error(msg || a + ' != ' + b); };
            assert.notEqual = function(a, b, msg) { if (a == b) throw new Error(msg || a + ' == ' + b); };
            assert.strictEqual = function(a, b, msg) { if (a !== b) throw new Error(msg || JSON.stringify(a) + ' !== ' + JSON.stringify(b)); };
            assert.notStrictEqual = function(a, b, msg) { if (a === b) throw new Error(msg || 'Expected values to not be strictly equal'); };
            assert.deepEqual = function(a, b, msg) { if (!_deepEq(a,b,false,[])) throw new Error(msg || 'deepEqual failed'); };
            assert.notDeepEqual = function(a, b, msg) { if (_deepEq(a,b,false,[])) throw new Error(msg || 'notDeepEqual failed'); };
            assert.deepStrictEqual = function(a, b, msg) { if (!_deepEq(a,b,true,[])) throw new Error(msg || 'deepStrictEqual failed'); };
            assert.notDeepStrictEqual = function(a, b, msg) { if (_deepEq(a,b,true,[])) throw new Error(msg || 'notDeepStrictEqual failed'); };
            assert.throws = function(fn, expected, msg) {
                try { fn(); } catch(e) {
                    if (!expected) return;
                    if (typeof expected === 'function' && e instanceof expected) return;
                    if (expected instanceof RegExp && expected.test(e.message)) return;
                    if (typeof expected === 'object' && expected.message && e.message === expected.message) return;
                    return;
                }
                throw new Error(msg || 'Expected function to throw');
            };
            assert.doesNotThrow = function(fn, msg) { try { fn(); } catch(e) { throw new Error(msg || 'Unexpected throw: ' + e.message); } };
            assert.ifError = function(v) { if (v) throw v; };
            assert.fail = function(msg) { throw new Error(msg || 'assert.fail'); };
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
                    createServer: function(opts, handler) {
                        if (typeof opts === 'function') { handler = opts; opts = {}; }
                        handler = handler || function() {};
                        var serverId = null;
                        var server = {
                            listening: false,
                            _host: '0.0.0.0',
                            _port: 0,
                            address: function() { return serverId !== null ? { address: server._host, port: server._port, family: 'IPv4' } : null; },
                            listen: function(port, host, cb) {
                                if (typeof port === 'object' && port !== null) { var o = port; port = o.port || 0; host = o.host || '0.0.0.0'; cb = host; }
                                if (typeof host === 'function') { cb = host; host = '0.0.0.0'; }
                                if (typeof port === 'function') { cb = port; port = 0; host = '0.0.0.0'; }
                                host = host || '0.0.0.0';
                                port = port || 0;
                                server._host = host;
                                server._port = port;
                                try {
                                    serverId = __httpListen(port, host);
                                    server.listening = true;
                                } catch(e) {
                                    server.emit('error', e);
                                    return server;
                                }
                                if (cb) setTimeout(cb, 0);
                                server.emit('listening');
                                _acceptNext();
                                return server;
                            },
                            close: function(cb) {
                                if (serverId !== null) { __httpClose(serverId); serverId = null; server.listening = false; }
                                if (cb) setTimeout(cb, 0);
                                return server;
                            },
                            _listeners: {},
                            on: function(ev, fn) { server._listeners[ev] = server._listeners[ev] || []; server._listeners[ev].push(fn); return server; },
                            once: function(ev, fn) {
                                function wrapper() { fn.apply(this, arguments); server.off(ev, wrapper); }
                                return server.on(ev, wrapper);
                            },
                            off: function(ev, fn) {
                                if (server._listeners[ev]) server._listeners[ev] = server._listeners[ev].filter(function(f) { return f !== fn; });
                                return server;
                            },
                            emit: function(ev) {
                                var args = Array.prototype.slice.call(arguments, 1);
                                (server._listeners[ev] || []).forEach(function(fn) { fn.apply(server, args); });
                            }
                        };

                        function _acceptNext() {
                            if (serverId === null) return;
                            __httpAcceptAsync(serverId).then(function(reqJson) {
                                var reqData = JSON.parse(reqJson);
                                _handleRequest(reqData);
                                _acceptNext();
                            }).catch(function(e) {
                                if (serverId !== null) {
                                    server.emit('error', e);
                                    _acceptNext();
                                }
                            });
                        }

                        function _handleRequest(reqData) {
                            var connId = reqData.conn_id;
                            var req = {
                                method: reqData.method,
                                url: reqData.url,
                                httpVersion: '1.1',
                                headers: reqData.headers,
                                rawHeaders: (function() {
                                    var arr = [];
                                    var h = reqData.headers || {};
                                    Object.keys(h).forEach(function(k) { arr.push(k, h[k]); });
                                    return arr;
                                })(),
                                socket: { remoteAddress: '127.0.0.1', remotePort: 0 },
                                _body: reqData.body || '',
                                _listeners: {},
                                on: function(ev, fn) { req._listeners[ev] = req._listeners[ev] || []; req._listeners[ev].push(fn); return req; },
                                emit: function(ev) { var args = Array.prototype.slice.call(arguments, 1); (req._listeners[ev]||[]).forEach(function(fn){fn.apply(req,args);}); },
                                setEncoding: function() {},
                                resume: function() { setTimeout(function() { (req._listeners.data||[]).forEach(function(fn){fn(req._body);}); (req._listeners.end||[]).forEach(function(fn){fn();}); }, 0); return req; },
                                pipe: function(dest) { if (dest && dest.write) dest.write(req._body); if (dest && dest.end) dest.end(); return dest; },
                                destroy: function() {}
                            };

                            var _responded = false;
                            var res = {
                                statusCode: 200,
                                statusMessage: '',
                                _headers: {},
                                _chunks: [],
                                writableEnded: false,
                                setHeader: function(k, v) { res._headers[k] = v; return res; },
                                getHeader: function(k) { return res._headers[k]; },
                                removeHeader: function(k) { delete res._headers[k]; return res; },
                                hasHeader: function(k) { return k in res._headers; },
                                writeHead: function(status, msg, headers) {
                                    if (typeof msg === 'object') { headers = msg; msg = ''; }
                                    res.statusCode = status;
                                    if (msg) res.statusMessage = msg;
                                    if (headers) Object.keys(headers).forEach(function(k) { res._headers[k] = headers[k]; });
                                    return res;
                                },
                                write: function(chunk) {
                                    if (chunk !== undefined && chunk !== null) {
                                        res._chunks.push(typeof chunk === 'string' ? chunk : new TextDecoder().decode(chunk));
                                    }
                                    return true;
                                },
                                end: function(chunk) {
                                    if (_responded) return;
                                    _responded = true;
                                    if (chunk !== undefined && chunk !== null) res.write(chunk);
                                    res.writableEnded = true;
                                    var body = res._chunks.join('');
                                    var st = res.statusCode || 200;
                                    var stText = res.statusMessage || STATUS_CODES[st] || 'OK';
                                    if (!res._headers['Content-Type'] && !res._headers['content-type']) {
                                        res._headers['Content-Type'] = 'text/plain';
                                    }
                                    try { __httpRespond(connId, st, stText, JSON.stringify(res._headers), body); }
                                    catch(e) { /* connection may have been closed */ }
                                },
                                _listeners: {},
                                on: function(ev, fn) { res._listeners[ev] = res._listeners[ev] || []; res._listeners[ev].push(fn); return res; },
                                emit: function(ev) { var args = Array.prototype.slice.call(arguments, 1); (res._listeners[ev]||[]).forEach(function(fn){fn.apply(res,args);}); },
                                destroy: function() { _responded = true; }
                            };

                            try { handler(req, res); } catch(e) {
                                if (!_responded) {
                                    try { __httpRespond(connId, 500, 'Internal Server Error', JSON.stringify({'Content-Type':'text/plain'}), 'Internal Server Error'); } catch(_) {}
                                    _responded = true;
                                }
                            }
                        }

                        return server;
                    },
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

            // ── crypto — wraps globalThis.crypto (SubtleCrypto) + Node.js-compat helpers ─
            var cryptoMod = {
                subtle: globalThis.crypto ? globalThis.crypto.subtle : undefined,
                getRandomValues: function(arr) { return globalThis.crypto.getRandomValues(arr); },
                randomUUID: function() { return globalThis.crypto.randomUUID(); },
                randomBytes: function(n) { var a = new Uint8Array(n); globalThis.crypto.getRandomValues(a); return a; },
                // Node.js streaming hash/hmac — digest() returns a Promise<string|Uint8Array>
                createHash: function(alg) {
                    var chunks = [];
                    var ha = alg === 'sha1' ? 'SHA-1' : alg === 'sha256' || alg === 'sha-256' ? 'SHA-256' : alg === 'sha384' || alg === 'sha-384' ? 'SHA-384' : alg === 'sha512' || alg === 'sha-512' ? 'SHA-512' : alg.toUpperCase();
                    return {
                        update: function(d) { chunks.push(typeof d === 'string' ? new TextEncoder().encode(d) : d); return this; },
                        digest: function(enc) {
                            var total = chunks.reduce(function(s, c) { return s + c.length; }, 0);
                            var data = new Uint8Array(total); var off = 0;
                            chunks.forEach(function(c) { data.set(c, off); off += c.length; });
                            var p = globalThis.crypto.subtle.digest(ha, data);
                            if (enc === 'hex') return p.then(function(b) { return Array.from(new Uint8Array(b)).map(function(x) { return x.toString(16).padStart(2,'0'); }).join(''); });
                            if (enc === 'base64') return p.then(function(b) { return btoa(String.fromCharCode.apply(null, Array.from(new Uint8Array(b)))); });
                            return p.then(function(b) { return new Uint8Array(b); });
                        }
                    };
                },
                createHmac: function(alg, key) {
                    var chunks = [];
                    var kd = typeof key === 'string' ? new TextEncoder().encode(key) : key;
                    var ha = alg === 'sha1' ? 'SHA-1' : alg === 'sha256' || alg === 'sha-256' ? 'SHA-256' : alg === 'sha384' || alg === 'sha-384' ? 'SHA-384' : alg === 'sha512' || alg === 'sha-512' ? 'SHA-512' : alg.toUpperCase();
                    return {
                        update: function(d) { chunks.push(typeof d === 'string' ? new TextEncoder().encode(d) : d); return this; },
                        digest: function(enc) {
                            var total = chunks.reduce(function(s, c) { return s + c.length; }, 0);
                            var data = new Uint8Array(total); var off = 0;
                            chunks.forEach(function(c) { data.set(c, off); off += c.length; });
                            var p = globalThis.crypto.subtle.importKey('raw', kd, { name: 'HMAC', hash: ha }, false, ['sign'])
                                .then(function(k) { return globalThis.crypto.subtle.sign('HMAC', k, data); });
                            if (enc === 'hex') return p.then(function(b) { return Array.from(new Uint8Array(b)).map(function(x) { return x.toString(16).padStart(2,'0'); }).join(''); });
                            if (enc === 'base64') return p.then(function(b) { return btoa(String.fromCharCode.apply(null, Array.from(new Uint8Array(b)))); });
                            return p.then(function(b) { return new Uint8Array(b); });
                        }
                    };
                },
                timingSafeEqual: function(a, b) {
                    if (a.length !== b.length) return false;
                    var r = 0; for (var i = 0; i < a.length; i++) r |= a[i] ^ b[i];
                    return r === 0;
                },
                pbkdf2: function(password, salt, iterations, keylen, digest, cb) {
                    var p = typeof password === 'string' ? new TextEncoder().encode(password) : password;
                    var s = typeof salt === 'string' ? new TextEncoder().encode(salt) : salt;
                    var ha = digest === 'sha1' ? 'SHA-1' : digest === 'sha256' ? 'SHA-256' : digest === 'sha512' ? 'SHA-512' : 'SHA-256';
                    globalThis.crypto.subtle.importKey('raw', p, { name: 'PBKDF2' }, false, ['deriveBits'])
                        .then(function(k) { return globalThis.crypto.subtle.deriveBits({ name: 'PBKDF2', hash: ha, salt: s, iterations: iterations }, k, keylen * 8); })
                        .then(function(b) { cb(null, new Uint8Array(b)); }).catch(cb);
                },
                pbkdf2Sync: function() { throw new Error('pbkdf2Sync: use async crypto.pbkdf2() in this runtime'); },
                createCipheriv: function() { throw new Error('createCipheriv: use crypto.subtle.encrypt() instead'); },
                createDecipheriv: function() { throw new Error('createDecipheriv: use crypto.subtle.decrypt() instead'); },
                constants: { SSL_OP_NO_SSLv2: 0, SSL_OP_NO_SSLv3: 0, SSL_OP_NO_TLSv1: 0 },
            };
            globalThis.__requireCache['crypto'] = cryptoMod;
            globalThis.__requireCache['node:crypto'] = cryptoMod;

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
                var delay = 1; // ms — starts low, backs off when idle, resets on data
                function poll() {
                    if (self._destroyed || self._id === null) {
                        self._pollTimer = null;
                        return;
                    }
                    if (self._paused) {
                        self._pollTimer = setTimeout(poll, 50);
                        return;
                    }
                    try {
                        var chunk = __tcpRead(self._id, 65536);
                        delay = 1; // data received — reset backoff
                        self.bytesRead += chunk.length;
                        var data = self._encoding
                            ? new TextDecoder(self._encoding).decode(new Uint8Array(chunk))
                            : new Uint8Array(chunk);
                        self.emit('data', data);
                        // More data may be waiting — schedule immediately
                        self._pollTimer = setTimeout(poll, 0);
                    } catch(e) {
                        if (e && e.code === 'EAGAIN') {
                            // No data yet — exponential backoff, cap at 100 ms
                            delay = Math.min(delay * 2, 100);
                            self._pollTimer = setTimeout(poll, delay);
                            return;
                        }
                        self._pollTimer = null;
                        if (e && e.code === 'EOF') {
                            self.readable = false;
                            self.emit('end');
                            if (!self.writable) { self.destroyed = true; self._destroyed = true; }
                            self.emit('close', false);
                        } else {
                            self.emit('error', e);
                            self.destroyed = true; self._destroyed = true;
                            self.emit('close', true);
                        }
                    }
                }
                self._pollTimer = setTimeout(poll, 0);
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
                if (this._pollTimer) { clearTimeout(this._pollTimer); this._pollTimer = null; }
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
                createServer: function(opts, connectionListener) {
                    if (typeof opts === 'function') { connectionListener = opts; opts = {}; }
                    connectionListener = connectionListener || function() {};
                    var serverId = null;
                    var server = {
                        listening: false,
                        _host: '0.0.0.0',
                        _port: 0,
                        address: function() { return serverId !== null ? { address: server._host, port: server._port, family: 'IPv4' } : null; },
                        listen: function(port, host, cb) {
                            if (typeof port === 'object' && port !== null) { var o = port; port = o.port || 0; host = o.host || '0.0.0.0'; cb = host; }
                            if (typeof host === 'function') { cb = host; host = '0.0.0.0'; }
                            if (typeof port === 'function') { cb = port; port = 0; host = '0.0.0.0'; }
                            host = host || '0.0.0.0';
                            port = port || 0;
                            server._host = host;
                            server._port = port;
                            try {
                                serverId = __netListen(port, host);
                                server.listening = true;
                            } catch(e) {
                                server.emit('error', e);
                                return server;
                            }
                            if (cb) setTimeout(cb, 0);
                            server.emit('listening');
                            _acceptNext();
                            return server;
                        },
                        close: function(cb) {
                            if (serverId !== null) { __netClose(serverId); serverId = null; server.listening = false; }
                            if (cb) setTimeout(cb, 0);
                            return server;
                        },
                        _listeners: {},
                        on: function(ev, fn) { server._listeners[ev] = server._listeners[ev] || []; server._listeners[ev].push(fn); return server; },
                        once: function(ev, fn) { function w() { fn.apply(this, arguments); server.off(ev, w); } return server.on(ev, w); },
                        off: function(ev, fn) { if (server._listeners[ev]) server._listeners[ev] = server._listeners[ev].filter(function(f){return f!==fn;}); return server; },
                        emit: function(ev) { var args = Array.prototype.slice.call(arguments,1); (server._listeners[ev]||[]).forEach(function(fn){fn.apply(server,args);}); }
                    };

                    function _acceptNext() {
                        if (serverId === null) return;
                        __netAcceptAsync(serverId).then(function(connId) {
                            var socket = new Socket({});
                            socket._id = connId;
                            socket.readable = true;
                            socket.writable = true;
                            socket._startPoll();
                            server.emit('connection', socket);
                            connectionListener(socket);
                            _acceptNext();
                        }).catch(function(e) {
                            if (serverId !== null) {
                                server.emit('error', e);
                                _acceptNext();
                            }
                        });
                    }

                    return server;
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

            // ── child_process — placeholder; overwritten by child_process.rs inject ─
            globalThis.__requireCache['child_process'] = {};
            globalThis.__requireCache['node:child_process'] = globalThis.__requireCache['child_process'];

            // ── fs (proxy to globalThis.fs) ───────────────────────────────────────
            // fs and fs/promises are built by fs.rs inject_fs() which runs before
            // inject_require.  Just proxy them; avoid overwriting with empty stubs.
            globalThis.__requireCache['fs'] = globalThis.fs || {};
            globalThis.__requireCache['node:fs'] = globalThis.fs || {};
            var _fsP = (globalThis.fs && globalThis.fs.promises) || {};
            globalThis.__requireCache['fs/promises'] = _fsP;
            globalThis.__requireCache['node:fs/promises'] = _fsP;

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

    // ── WinterCG / Web Platform globals ──────────────────────────────────────
    ctx.eval::<(), _>(r#"
        // self === globalThis (required by workers and many edge frameworks)
        globalThis.self = globalThis;

        // structuredClone — spec-compliant deep clone (WHATWG Structured Clone Algorithm subset).
        // Handles: primitives, Date, RegExp, ArrayBuffer, TypedArray/DataView, Map, Set, Array,
        // plain objects. Throws DataCloneError for functions, symbols, and circular references.
        (function() {
            function _clone(val, seen) {
                if (val === null || val === undefined) return val;
                var t = typeof val;
                if (t === 'number' || t === 'string' || t === 'boolean' || t === 'bigint') return val;
                if (t === 'symbol' || t === 'function')
                    throw Object.assign(new TypeError('structuredClone: ' + t + ' values cannot be cloned'), { name: 'DataCloneError' });
                if (seen.has(val)) throw Object.assign(new TypeError('structuredClone: circular reference detected'), { name: 'DataCloneError' });
                if (val instanceof Date) return new Date(val.getTime());
                if (val instanceof RegExp) return new RegExp(val.source, val.flags);
                if (val instanceof ArrayBuffer) {
                    var ab = new ArrayBuffer(val.byteLength);
                    new Uint8Array(ab).set(new Uint8Array(val));
                    return ab;
                }
                if (val instanceof DataView) {
                    var ab2 = new ArrayBuffer(val.buffer.byteLength);
                    new Uint8Array(ab2).set(new Uint8Array(val.buffer));
                    return new DataView(ab2, val.byteOffset, val.byteLength);
                }
                if (ArrayBuffer.isView(val)) {
                    var ab3 = new ArrayBuffer(val.buffer.byteLength);
                    new Uint8Array(ab3).set(new Uint8Array(val.buffer));
                    return new val.constructor(ab3, val.byteOffset, val.length);
                }
                if (val instanceof Error) {
                    var e2 = new val.constructor(val.message);
                    if (val.stack) e2.stack = val.stack;
                    return e2;
                }
                if (val instanceof Map) {
                    seen.set(val, true);
                    var m = new Map();
                    val.forEach(function(v, k) { m.set(_clone(k, seen), _clone(v, seen)); });
                    seen.delete(val);
                    return m;
                }
                if (val instanceof Set) {
                    seen.set(val, true);
                    var s = new Set();
                    val.forEach(function(v) { s.add(_clone(v, seen)); });
                    seen.delete(val);
                    return s;
                }
                if (Array.isArray(val)) {
                    seen.set(val, true);
                    var arr = new Array(val.length);
                    for (var i = 0; i < val.length; i++) arr[i] = _clone(val[i], seen);
                    seen.delete(val);
                    return arr;
                }
                if (t === 'object') {
                    seen.set(val, true);
                    var obj = {};
                    var ks = Object.keys(val);
                    for (var i = 0; i < ks.length; i++) obj[ks[i]] = _clone(val[ks[i]], seen);
                    seen.delete(val);
                    return obj;
                }
                return val;
            }
            globalThis.structuredClone = function structuredClone(val) {
                return _clone(val, new WeakMap());
            };
        })();

        // navigator — minimal WinterCG-compatible object
        if (!globalThis.navigator) {
            globalThis.navigator = Object.freeze({
                userAgent: '3va/0.1 (QuickJS)',
                language: 'en-US',
                languages: Object.freeze(['en-US', 'en']),
                onLine: true,
                hardwareConcurrency: 1,
                platform: 'Linux x86_64',
                cookieEnabled: false,
                doNotTrack: '1',
            });
        }

        // ── Headers ──────────────────────────────────────────────────────────
        (function() {
            function Headers(init) {
                this._list = []; // [[lowercase-name, value], ...]
                if (init instanceof Headers) {
                    for (var i = 0; i < init._list.length; i++) this._list.push([init._list[i][0], init._list[i][1]]);
                } else if (Array.isArray(init)) {
                    for (var i = 0; i < init.length; i++) this.append(init[i][0], init[i][1]);
                } else if (init && typeof init === 'object') {
                    var ks = Object.keys(init);
                    for (var i = 0; i < ks.length; i++) this.append(ks[i], init[ks[i]]);
                }
            }
            Headers.prototype.append = function(name, value) {
                this._list.push([name.toLowerCase(), String(value)]);
            };
            Headers.prototype['delete'] = function(name) {
                var l = name.toLowerCase();
                this._list = this._list.filter(function(e) { return e[0] !== l; });
            };
            Headers.prototype.get = function(name) {
                var l = name.toLowerCase(); var vals = [];
                for (var i = 0; i < this._list.length; i++) { if (this._list[i][0] === l) vals.push(this._list[i][1]); }
                return vals.length ? vals.join(', ') : null;
            };
            Headers.prototype.getSetCookie = function() {
                return this._list.filter(function(e) { return e[0] === 'set-cookie'; }).map(function(e) { return e[1]; });
            };
            Headers.prototype.has = function(name) {
                var l = name.toLowerCase();
                for (var i = 0; i < this._list.length; i++) { if (this._list[i][0] === l) return true; }
                return false;
            };
            Headers.prototype.set = function(name, value) {
                var l = name.toLowerCase(); var found = false;
                this._list = this._list.filter(function(e) {
                    if (e[0] !== l) return true;
                    if (!found) { found = true; e[1] = String(value); return true; }
                    return false;
                });
                if (!found) this._list.push([l, String(value)]);
            };
            Headers.prototype.forEach = function(cb, thisArg) {
                for (var i = 0; i < this._list.length; i++) cb.call(thisArg, this._list[i][1], this._list[i][0], this);
            };
            function _makeIter(list, mapFn) {
                var i = 0;
                var it = { next: function() {
                    if (i < list.length) return { value: mapFn(list[i++]), done: false };
                    return { value: undefined, done: true };
                }};
                it[Symbol.iterator] = function() { return this; };
                return it;
            }
            Headers.prototype.entries = function() { return _makeIter(this._list, function(e) { return [e[0], e[1]]; }); };
            Headers.prototype.keys = function() { return _makeIter(this._list, function(e) { return e[0]; }); };
            Headers.prototype.values = function() { return _makeIter(this._list, function(e) { return e[1]; }); };
            Headers.prototype[Symbol.iterator] = function() { return this.entries(); };
            Headers.prototype.toJSON = function() {
                var obj = {};
                this.forEach(function(v, k) { obj[k] = obj[k] !== undefined ? obj[k] + ', ' + v : v; });
                return obj;
            };
            globalThis.Headers = Headers;
        })();

        // ── shared formData body parser (used by Request and Response) ────────
        globalThis._parseBodyAsFormData = function(ct, body) {
            ct = (ct || '').toLowerCase();
            var fd = new FormData();
            if (ct.indexOf('application/x-www-form-urlencoded') >= 0) {
                var pairs = body.split('&');
                for (var i = 0; i < pairs.length; i++) {
                    if (!pairs[i]) continue;
                    var idx = pairs[i].indexOf('=');
                    var k = decodeURIComponent(idx >= 0 ? pairs[i].slice(0, idx) : pairs[i]).replace(/\+/g, ' ');
                    var v = decodeURIComponent(idx >= 0 ? pairs[i].slice(idx + 1) : '').replace(/\+/g, ' ');
                    fd.append(k, v);
                }
                return fd;
            }
            if (ct.indexOf('multipart/form-data') >= 0) {
                var bm = ct.match(/boundary=([^\s;]+)/);
                if (!bm) throw new Error('formData: missing boundary in Content-Type');
                var boundary = '--' + bm[1];
                var parts = body.split(boundary);
                for (var i = 1; i < parts.length - 1; i++) {
                    var part = parts[i];
                    if (part.startsWith('\r\n')) part = part.slice(2);
                    if (part.endsWith('\r\n')) part = part.slice(0, -2);
                    var headerEnd = part.indexOf('\r\n\r\n');
                    if (headerEnd < 0) continue;
                    var rawHdrs = part.slice(0, headerEnd);
                    var partBody = part.slice(headerEnd + 4);
                    var cd = '';
                    var hlines = rawHdrs.split('\r\n');
                    for (var j = 0; j < hlines.length; j++) {
                        if (hlines[j].toLowerCase().startsWith('content-disposition:')) {
                            cd = hlines[j].slice(hlines[j].indexOf(':') + 1).trim();
                        }
                    }
                    var nm = cd.match(/name="([^"]*)"/);
                    var fm = cd.match(/filename="([^"]*)"/);
                    if (!nm) continue;
                    if (fm) fd.append(nm[1], new File([partBody], fm[1]));
                    else fd.append(nm[1], partBody);
                }
                return fd;
            }
            throw new TypeError('formData: unsupported Content-Type: ' + ct);
        };

        // ── Request ──────────────────────────────────────────────────────────
        (function() {
            function Request(input, init) {
                init = init || {};
                if (input && typeof input === 'object' && 'url' in input) {
                    // Clone from existing Request
                    this.url = String(input.url);
                    this.method = (init.method || input.method || 'GET').toUpperCase();
                    this.headers = new Headers(init.headers != null ? init.headers : input.headers);
                    this._body = init.body != null ? String(init.body) : (input._body || null);
                    this.signal = init.signal || input.signal || null;
                } else {
                    this.url = String(input);
                    this.method = (init.method || 'GET').toUpperCase();
                    this.headers = new Headers(init.headers);
                    this._body = init.body != null ? String(init.body) : null;
                    this.signal = init.signal || null;
                }
                this.bodyUsed = false;
                this.duplex = init.duplex || 'half';
                this.mode = init.mode || 'cors';
                this.credentials = init.credentials || 'same-origin';
                this.cache = init.cache || 'default';
                this.redirect = init.redirect || 'follow';
                this.referrer = init.referrer || 'about:client';
                this.integrity = init.integrity || '';
                this.keepalive = !!init.keepalive;
            }
            Request.prototype._consumeBody = function() {
                if (this.bodyUsed) return Promise.reject(new TypeError('Body already used'));
                this.bodyUsed = true;
                return Promise.resolve(this._body || '');
            };
            Request.prototype.text = function() { return this._consumeBody(); };
            Request.prototype.json = function() {
                return this._consumeBody().then(function(t) {
                    try { return JSON.parse(t); } catch(e) { throw new SyntaxError('Invalid JSON: ' + e.message); }
                });
            };
            Request.prototype.arrayBuffer = function() {
                return this._consumeBody().then(function(s) {
                    var buf = new ArrayBuffer(s.length); var v = new Uint8Array(buf);
                    for (var i = 0; i < s.length; i++) v[i] = s.charCodeAt(i) & 0xff;
                    return buf;
                });
            };
            Request.prototype.bytes = function() {
                return this.arrayBuffer().then(function(b) { return new Uint8Array(b); });
            };
            Request.prototype.blob = function() {
                return this._consumeBody().then(function(s) { return new Blob([s]); });
            };
            Request.prototype.formData = function() {
                var self = this;
                return this._consumeBody().then(function(body) {
                    return globalThis._parseBodyAsFormData(self.headers.get('content-type'), body);
                });
            };
            Request.prototype.clone = function() {
                if (this.bodyUsed) throw new TypeError('Cannot clone: body already read');
                return new Request(this.url, { method: this.method, headers: this.headers, body: this._body, signal: this.signal });
            };
            globalThis.Request = Request;
        })();

        // ── Response ─────────────────────────────────────────────────────────
        (function() {
            function Response(body, init) {
                init = init || {};
                this.status = init.status != null ? Number(init.status) : 200;
                this.statusText = init.statusText != null ? String(init.statusText) : '';
                this.headers = new Headers(init.headers);
                this.url = init.url || '';
                this.redirected = !!init.redirected;
                this.type = init.type || 'default';
                this.ok = this.status >= 200 && this.status < 300;
                this.bodyUsed = false;
                this._blob = null;
                if (body == null) {
                    this._body = null;
                } else if (body instanceof Blob) {
                    this._blob = body; this._body = null;
                } else if (body instanceof ArrayBuffer || ArrayBuffer.isView(body)) {
                    var bytes = body instanceof ArrayBuffer ? new Uint8Array(body) : new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
                    this._body = new TextDecoder().decode(bytes);
                } else {
                    this._body = String(body);
                }
            }
            Response.prototype._consumeBody = function() {
                if (this.bodyUsed) return Promise.reject(new TypeError('Body already used'));
                this.bodyUsed = true;
                if (this._blob) return this._blob.text();
                return Promise.resolve(this._body != null ? this._body : '');
            };
            Response.prototype.text = function() { return this._consumeBody(); };
            Response.prototype.json = function() {
                return this._consumeBody().then(function(t) {
                    try { return JSON.parse(t); } catch(e) { throw new SyntaxError('Invalid JSON: ' + e.message); }
                });
            };
            Response.prototype.arrayBuffer = function() {
                return this._consumeBody().then(function(s) {
                    var buf = new ArrayBuffer(s.length); var v = new Uint8Array(buf);
                    for (var i = 0; i < s.length; i++) v[i] = s.charCodeAt(i) & 0xff;
                    return buf;
                });
            };
            Response.prototype.bytes = function() {
                return this.arrayBuffer().then(function(b) { return new Uint8Array(b); });
            };
            Response.prototype.blob = function() {
                return this._consumeBody().then(function(s) { return new Blob([s]); });
            };
            Response.prototype.formData = function() {
                var self = this;
                return this._consumeBody().then(function(body) {
                    return globalThis._parseBodyAsFormData(self.headers.get('content-type'), body);
                });
            };
            Response.prototype.clone = function() {
                if (this.bodyUsed) throw new TypeError('Cannot clone: body already read');
                return new Response(this._blob || this._body, {
                    status: this.status, statusText: this.statusText,
                    headers: this.headers, url: this.url,
                    redirected: this.redirected, type: this.type,
                });
            };
            Response.json = function(data, init) {
                init = init || {};
                var h = new Headers(init.headers);
                if (!h.has('content-type')) h.set('content-type', 'application/json');
                return new Response(JSON.stringify(data), Object.assign({}, init, { headers: h }));
            };
            Response.error = function() {
                return new Response(null, { status: 0, statusText: '', type: 'error' });
            };
            Response.redirect = function(url, status) {
                status = status || 302;
                if ([301, 302, 303, 307, 308].indexOf(status) < 0) throw new RangeError('Invalid redirect status code: ' + status);
                var h = new Headers(); h.set('location', String(url));
                return new Response(null, { status: status, headers: h });
            };
            globalThis.Response = Response;
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

            // Strip jsr: prefix — resolve as a scoped package in node_modules/
            // e.g. require('jsr:@scope/name') → require('@scope/name')
            if (path.indexOf('jsr:') === 0) {
                path = path.slice(4);
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

    inject_missing_node_modules(ctx)?;

    Ok(())
}

// ── Missing Node.js built-ins ─────────────────────────────────────────────────
//
// Every module here is either completely absent from Node or was previously an
// empty stub.  The goal is "crash-free compatibility": popular packages can
// import these modules without throwing, and the most-used APIs work correctly.

fn inject_missing_node_modules(ctx: &Ctx) -> Result<()> {
    ctx.eval::<(), _>(r#"
(function() {
'use strict';

// ── async_hooks ───────────────────────────────────────────────────────────────
// AsyncLocalStorage backed by the patched QuickJS job hook.
//
// Architecture (from the root up):
//   C layer  — JS_ExecutePendingJob calls job_before_execute_hook(ctx_id)
//              before each Promise continuation, which updates a Rust
//              thread-local with the ctx_id captured at enqueue time.
//   Rust layer — __asyncCtxGet/__asyncCtxSet read/write that thread-local;
//                __asyncCtxAlloc/__asyncCtxRead/__asyncCtxFree manage data.
//   JS layer  — AsyncLocalStorage.run() allocates a child ctx, sets it as
//               the active ID, calls fn(), then restores the parent.
//               getStore() reads from the currently active ctx_id.
//
// Because the C hook fires at the runtime level — not via Promise.prototype.then
// monkey-patching — context propagates correctly across concurrent request
// chains and nested async operations without any userland overhead.
(function() {
    // Guard: native functions must be present (installed by async_context.rs).
    if (typeof __asyncCtxGet !== 'function') return;

    var _alsKeyCounter = 0;

    function AsyncLocalStorage() {
        // Each instance gets a unique string key into the context data map.
        this._key = '__als_' + (++_alsKeyCounter);
        this._disabled = false;
    }

    AsyncLocalStorage.prototype.run = function(store, fn) {
        if (this._disabled) return fn.apply(null, Array.prototype.slice.call(arguments, 2));
        var args = Array.prototype.slice.call(arguments, 2);
        var parentId = __asyncCtxGet();
        var json = JSON.stringify(store === undefined ? null : store);
        var newId = __asyncCtxAlloc(parentId, this._key, json);
        var freeId = newId;

        __asyncCtxSet(newId);
        var result;
        try {
            result = fn.apply(null, args);
        } finally {
            // Restore parent context so code after run() runs in the right scope.
            __asyncCtxSet(parentId);
        }

        // Free context data only after the entire async chain completes.
        // For async functions fn() returns a pending Promise; we attach cleanup
        // to its settlement so continuations can still read the store.
        // For sync functions we free immediately.
        // We do NOT use Promise.resolve().then(cleanup) here because that would
        // enqueue cleanup before the async chain's continuations, freeing data
        // that is still needed.
        if (result !== null && result !== undefined && typeof result.then === 'function') {
            var cleanup = function() { __asyncCtxFree(freeId); };
            result.then(cleanup, cleanup);
        } else {
            __asyncCtxFree(freeId);
        }

        return result;
    };

    AsyncLocalStorage.prototype.getStore = function() {
        if (this._disabled) return undefined;
        var ctxId = __asyncCtxGet();
        if (ctxId === 0) return undefined;
        var raw = __asyncCtxRead(ctxId, this._key);
        if (raw === undefined || raw === null) return undefined;
        try {
            var v = JSON.parse(raw);
            return v === null ? undefined : v;
        } catch(e) { return undefined; }
    };

    AsyncLocalStorage.prototype.enterWith = function(store) {
        if (this._disabled) return;
        var ctxId = __asyncCtxGet();
        var json = JSON.stringify(store === undefined ? null : store);
        if (ctxId === 0) {
            var newId = __asyncCtxAlloc(0, this._key, json);
            __asyncCtxSet(newId);
        } else {
            // Re-allocate current context with the new value.
            var newId2 = __asyncCtxAlloc(ctxId, this._key, json);
            __asyncCtxSet(newId2);
        }
    };

    AsyncLocalStorage.prototype.exit = function(fn) {
        var args = Array.prototype.slice.call(arguments, 1);
        var prev = __asyncCtxGet();
        __asyncCtxSet(0);
        try { return fn.apply(null, args); }
        finally { __asyncCtxSet(prev); }
    };

    AsyncLocalStorage.prototype.disable = function() {
        this._disabled = true;
    };

    AsyncLocalStorage.snapshot = function() {
        var capturedId = __asyncCtxGet();
        return function restore(fn) {
            var args = Array.prototype.slice.call(arguments, 1);
            var prev = __asyncCtxGet();
            __asyncCtxSet(capturedId);
            try { return fn.apply(null, args); }
            finally { __asyncCtxSet(prev); }
        };
    };

    function AsyncResource(type) {
        this.type = type || 'AsyncResource';
        this._ctxId = __asyncCtxGet();
        this._destroyed = false;
        // Retain the context so it outlives the run() call that created it.
        if (this._ctxId !== 0) __asyncCtxRetain(this._ctxId);
    }

    AsyncResource.prototype.runInAsyncScope = function(fn, thisArg) {
        var args = Array.prototype.slice.call(arguments, 2);
        var prev = __asyncCtxGet();
        __asyncCtxSet(this._ctxId);
        try { return fn.apply(thisArg, args); }
        finally { __asyncCtxSet(prev); }
    };

    AsyncResource.prototype.emitDestroy = function() {
        if (!this._destroyed && this._ctxId !== 0) {
            this._destroyed = true;
            __asyncCtxFree(this._ctxId);
        }
        return this;
    };

    AsyncResource.prototype.bind = function(fn, thisArg) {
        var self = this;
        return function() {
            return self.runInAsyncScope(fn, thisArg || this, ...arguments);
        };
    };

    AsyncResource.bind = function(fn, type, thisArg) {
        return new AsyncResource(type || fn.name || 'bound').bind(fn, thisArg);
    };

    var asyncHooks = {
        AsyncLocalStorage: AsyncLocalStorage,
        AsyncResource: AsyncResource,
        createHook: function() { return { enable: function() { return this; }, disable: function() { return this; } }; },
        executionAsyncId: function() { return __asyncCtxGet(); },
        triggerAsyncId: function() { return 0; },
        executionAsyncResource: function() { return null; }
    };

    globalThis.__requireCache['async_hooks'] = asyncHooks;
    globalThis.__requireCache['node:async_hooks'] = asyncHooks;
}());

// ── worker_threads ────────────────────────────────────────────────────────────
// 3va is single-threaded; expose a safe stub so imports don't throw.
// isMainThread: true means code that branches on it will take the main path.
(function() {
    var EventEmitter = globalThis.__requireCache['events'];

    function MessageChannel() {
        var port1 = new MessagePort();
        var port2 = new MessagePort();
        port1._peer = port2;
        port2._peer = port1;
        this.port1 = port1;
        this.port2 = port2;
    }

    function MessagePort() {
        EventEmitter.call(this);
        this._peer = null;
    }
    if (EventEmitter) {
        MessagePort.prototype = Object.create(EventEmitter.prototype);
        MessagePort.prototype.constructor = MessagePort;
    }
    MessagePort.prototype.postMessage = function(data) {
        var self = this;
        if (self._peer) {
            var peer = self._peer;
            setTimeout(function() { peer.emit('message', data); }, 0);
        }
    };
    MessagePort.prototype.start = function() {};
    MessagePort.prototype.close = function() { this.emit('close'); };
    MessagePort.prototype.unref = function() { return this; };
    MessagePort.prototype.ref = function() { return this; };

    var workerThreads = {
        isMainThread: true,
        threadId: 0,
        workerData: null,
        parentPort: null,
        resourceLimits: {},
        MessageChannel: MessageChannel,
        MessagePort: MessagePort,
        Worker: function Worker(filename, options) {
            EventEmitter.call(this);
            this.threadId = 0;
            this.resourceLimits = {};
        },
        receiveMessageOnPort: function() { return null; },
        moveMessagePortToContext: function(port) { return port; },
        markAsUntransferable: function() {},
        getEnvironmentData: function() { return undefined; },
        setEnvironmentData: function() {}
    };
    if (EventEmitter) {
        workerThreads.Worker.prototype = Object.create(EventEmitter.prototype);
        workerThreads.Worker.prototype.constructor = workerThreads.Worker;
    }
    workerThreads.Worker.prototype.postMessage = function() {};
    workerThreads.Worker.prototype.terminate = function() {
        return Promise.resolve(0);
    };
    workerThreads.Worker.prototype.unref = function() { return this; };
    workerThreads.Worker.prototype.ref = function() { return this; };

    globalThis.__requireCache['worker_threads'] = workerThreads;
    globalThis.__requireCache['node:worker_threads'] = workerThreads;
}());

// ── vm ────────────────────────────────────────────────────────────────────────
// Used by webpack, Jest, and many build tools for module evaluation.
(function() {
    function Script(code, options) {
        this._code = code;
        this._filename = (options && options.filename) || '<anonymous>';
    }

    Script.prototype.runInThisContext = function(options) {
        // eslint-disable-next-line no-new-func
        return (new Function('return (' + this._code + ')'))();
    };

    Script.prototype.runInNewContext = function(sandbox, options) {
        return Script.prototype.runInThisContext.call(this, options);
    };

    Script.prototype.runInContext = function(ctx, options) {
        return Script.prototype.runInThisContext.call(this, options);
    };

    function createContext(sandbox) {
        return sandbox || {};
    }

    function isContext(obj) {
        return typeof obj === 'object' && obj !== null;
    }

    function runInThisContext(code, options) {
        return new Script(code, options).runInThisContext(options);
    }

    function runInNewContext(code, sandbox, options) {
        return new Script(code, options).runInNewContext(sandbox, options);
    }

    function compileFunction(code, params, options) {
        params = params || [];
        // eslint-disable-next-line no-new-func
        return new Function(params.join(', '), code);
    }

    var vm = {
        Script: Script,
        createContext: createContext,
        isContext: isContext,
        runInThisContext: runInThisContext,
        runInNewContext: runInNewContext,
        runInContext: runInNewContext,
        compileFunction: compileFunction
    };

    globalThis.__requireCache['vm'] = vm;
    globalThis.__requireCache['node:vm'] = vm;
}());

// ── timers/promises ───────────────────────────────────────────────────────────
// Modern Node code uses `import { setTimeout } from 'timers/promises'`.
(function() {
    function setTimeoutP(delay, value, options) {
        return new Promise(function(resolve, reject) {
            var id = setTimeout(function() { resolve(value); }, delay || 0);
            if (options && options.signal) {
                options.signal.addEventListener('abort', function() {
                    clearTimeout(id);
                    reject(new DOMException('The operation was aborted.', 'AbortError'));
                });
            }
        });
    }

    function setImmediateP(value, options) {
        return new Promise(function(resolve, reject) {
            var id = setImmediate(function() { resolve(value); });
            if (options && options.signal) {
                options.signal.addEventListener('abort', function() {
                    clearImmediate(id);
                    reject(new DOMException('The operation was aborted.', 'AbortError'));
                });
            }
        });
    }

    function setIntervalP(delay, value, options) {
        var aborted = false;
        if (options && options.signal) {
            options.signal.addEventListener('abort', function() { aborted = true; });
        }
        return {
            [Symbol.asyncIterator]: function() {
                return {
                    next: function() {
                        if (aborted) return Promise.resolve({ value: undefined, done: true });
                        return new Promise(function(resolve) {
                            setTimeout(function() {
                                resolve({ value: value, done: false });
                            }, delay || 0);
                        });
                    },
                    return: function() {
                        aborted = true;
                        return Promise.resolve({ value: undefined, done: true });
                    }
                };
            }
        };
    }

    var timersPromises = {
        setTimeout: setTimeoutP,
        setImmediate: setImmediateP,
        setInterval: setIntervalP,
        scheduler: {
            wait: setTimeoutP,
            yield: function() { return setImmediateP(undefined); }
        }
    };

    globalThis.__requireCache['timers/promises'] = timersPromises;
    globalThis.__requireCache['node:timers/promises'] = timersPromises;
    globalThis.__requireCache['timers'] = Object.assign(
        globalThis.__requireCache['timers'] || {},
        { setTimeout: setTimeout, setInterval: setInterval, setImmediate: setImmediate,
          clearTimeout: clearTimeout, clearInterval: clearInterval,
          clearImmediate: clearImmediate, promises: timersPromises }
    );
    globalThis.__requireCache['node:timers'] = globalThis.__requireCache['timers'];
}());

// ── tty ───────────────────────────────────────────────────────────────────────
// chalk, ink, and any color library checks tty.isatty() and process.stdout.isTTY.
(function() {
    var stream = globalThis.__requireCache['stream'];

    function ReadStream(fd, options) {
        if (stream && stream.Readable) stream.Readable.call(this, options);
        this.fd = fd;
        this.isTTY = false;
        this.isRaw = false;
        this.columns = 80;
        this.rows = 24;
    }
    if (stream && stream.Readable) {
        ReadStream.prototype = Object.create(stream.Readable.prototype);
        ReadStream.prototype.constructor = ReadStream;
    }
    ReadStream.prototype.setRawMode = function(mode) { this.isRaw = mode; return this; };
    ReadStream.prototype.unref = function() { return this; };
    ReadStream.prototype.ref = function() { return this; };

    function WriteStream(fd, options) {
        if (stream && stream.Writable) stream.Writable.call(this, options);
        this.fd = fd;
        this.isTTY = false;
        this.columns = 80;
        this.rows = 24;
    }
    if (stream && stream.Writable) {
        WriteStream.prototype = Object.create(stream.Writable.prototype);
        WriteStream.prototype.constructor = WriteStream;
    }
    WriteStream.prototype.clearLine = function(dir, cb) { if (cb) cb(); return true; };
    WriteStream.prototype.clearScreenDown = function(cb) { if (cb) cb(); return true; };
    WriteStream.prototype.cursorTo = function(x, y, cb) { if (cb) cb(); return true; };
    WriteStream.prototype.moveCursor = function(dx, dy, cb) { if (cb) cb(); return true; };
    WriteStream.prototype.getColorDepth = function() { return 1; };
    WriteStream.prototype.hasColors = function() { return false; };
    WriteStream.prototype.getWindowSize = function() { return [this.columns, this.rows]; };
    WriteStream.prototype.unref = function() { return this; };
    WriteStream.prototype.ref = function() { return this; };

    var tty = {
        isatty: function(fd) { return false; },
        ReadStream: ReadStream,
        WriteStream: WriteStream
    };

    globalThis.__requireCache['tty'] = tty;
    globalThis.__requireCache['node:tty'] = tty;
}());

// ── readline ──────────────────────────────────────────────────────────────────
(function() {
    var EventEmitter = globalThis.__requireCache['events'];

    function Interface(options) {
        EventEmitter.call(this);
        if (typeof options === 'object') {
            this.input = options.input;
            this.output = options.output;
            this.terminal = options.terminal || false;
        }
        this.line = '';
        this._closed = false;
    }
    if (EventEmitter) {
        Interface.prototype = Object.create(EventEmitter.prototype);
        Interface.prototype.constructor = Interface;
    }

    Interface.prototype.setPrompt = function(prompt) { this._prompt = prompt; return this; };
    Interface.prototype.prompt = function(preserveCursor) { if (this.output) this.output.write && this.output.write(this._prompt || ''); };
    Interface.prototype.question = function(query, options, cb) {
        if (typeof options === 'function') { cb = options; options = {}; }
        if (this.output && this.output.write) this.output.write(query);
        if (cb) setTimeout(function() { cb(''); }, 0);
        return Promise.resolve('');
    };
    Interface.prototype.close = function() {
        this._closed = true;
        this.emit('close');
    };
    Interface.prototype.pause = function() { this.emit('pause'); return this; };
    Interface.prototype.resume = function() { this.emit('resume'); return this; };
    Interface.prototype.write = function(data, key) {};
    Interface.prototype[Symbol.asyncIterator] = function() {
        var self = this;
        return {
            next: function() {
                if (self._closed) return Promise.resolve({ value: undefined, done: true });
                return Promise.resolve({ value: '', done: true });
            },
            return: function() {
                self.close();
                return Promise.resolve({ value: undefined, done: true });
            }
        };
    };

    function createInterface(options) {
        return new Interface(options);
    }

    function clearLine(stream, dir, cb) { if (cb) cb(); return true; }
    function clearScreenDown(stream, cb) { if (cb) cb(); return true; }
    function cursorTo(stream, x, y, cb) { if (cb) cb(); return true; }
    function moveCursor(stream, dx, dy, cb) { if (cb) cb(); return true; }
    function emitKeypressEvents(stream) {}

    var readline = {
        Interface: Interface,
        createInterface: createInterface,
        clearLine: clearLine,
        clearScreenDown: clearScreenDown,
        cursorTo: cursorTo,
        moveCursor: moveCursor,
        emitKeypressEvents: emitKeypressEvents,
        promises: {
            createInterface: createInterface
        }
    };

    globalThis.__requireCache['readline'] = readline;
    globalThis.__requireCache['node:readline'] = readline;
    globalThis.__requireCache['readline/promises'] = readline.promises;
    globalThis.__requireCache['node:readline/promises'] = readline.promises;
}());

// ── diagnostics_channel ───────────────────────────────────────────────────────
// Used by OpenTelemetry, undici, and Node.js monitoring tools.
(function() {
    var _channels = {};

    function Channel(name) {
        this.name = name;
        this._subscribers = [];
    }

    Object.defineProperty(Channel.prototype, 'hasSubscribers', {
        get: function() { return this._subscribers.length > 0; }
    });

    Channel.prototype.subscribe = function(fn) {
        this._subscribers.push(fn);
    };

    Channel.prototype.unsubscribe = function(fn) {
        var idx = this._subscribers.indexOf(fn);
        if (idx !== -1) this._subscribers.splice(idx, 1);
        return idx !== -1;
    };

    Channel.prototype.publish = function(message) {
        for (var i = 0; i < this._subscribers.length; i++) {
            try { this._subscribers[i](message, this.name); } catch(e) {}
        }
    };

    Channel.prototype.bindStore = function(store, transform) {};
    Channel.prototype.unbindStore = function(store) {};
    Channel.prototype.runStores = function(data, fn, thisArg) {
        return fn.call(thisArg);
    };

    function channel(name) {
        if (!_channels[name]) _channels[name] = new Channel(name);
        return _channels[name];
    }

    function hasSubscribers(name) {
        return !!(_channels[name] && _channels[name].hasSubscribers);
    }

    function subscribe(name, fn) {
        channel(name).subscribe(fn);
    }

    function unsubscribe(name, fn) {
        return channel(name).unsubscribe(fn);
    }

    function tracingChannel(nameOrChannels) {
        var prefix = typeof nameOrChannels === 'string' ? nameOrChannels : '';
        return {
            start: channel(prefix + '.start'),
            end: channel(prefix + '.end'),
            asyncStart: channel(prefix + '.asyncStart'),
            asyncEnd: channel(prefix + '.asyncEnd'),
            error: channel(prefix + '.error'),
            traceSync: function(fn, context, thisArg) { return fn.call(thisArg, context); },
            tracePromise: function(fn, context, thisArg) {
                var result;
                try { result = fn.call(thisArg, context); } catch(e) { return Promise.reject(e); }
                return Promise.resolve(result);
            },
            traceCallback: function(fn, position, context, thisArg) {
                return fn.bind(thisArg);
            }
        };
    }

    var diagnosticsChannel = {
        channel: channel,
        hasSubscribers: hasSubscribers,
        subscribe: subscribe,
        unsubscribe: unsubscribe,
        tracingChannel: tracingChannel,
        Channel: Channel
    };

    globalThis.__requireCache['diagnostics_channel'] = diagnosticsChannel;
    globalThis.__requireCache['node:diagnostics_channel'] = diagnosticsChannel;
}());

// ── domain ────────────────────────────────────────────────────────────────────
// Legacy, but many packages (like 'async') have domain as an optional dep.
(function() {
    var EventEmitter = globalThis.__requireCache['events'];

    function Domain() {
        EventEmitter.call(this);
        this.members = [];
    }
    if (EventEmitter) {
        Domain.prototype = Object.create(EventEmitter.prototype);
        Domain.prototype.constructor = Domain;
    }
    Domain.prototype.run = function(fn) {
        try { return fn(); } catch(e) { this.emit('error', e); }
    };
    Domain.prototype.add = function(emitter) { this.members.push(emitter); };
    Domain.prototype.remove = function(emitter) {
        var idx = this.members.indexOf(emitter);
        if (idx !== -1) this.members.splice(idx, 1);
    };
    Domain.prototype.bind = function(fn) {
        var self = this;
        return function() {
            try { return fn.apply(this, arguments); } catch(e) { self.emit('error', e); }
        };
    };
    Domain.prototype.intercept = function(fn) {
        var self = this;
        return function(err) {
            if (err) { self.emit('error', err); return; }
            try { return fn.apply(this, Array.prototype.slice.call(arguments, 1)); }
            catch(e) { self.emit('error', e); }
        };
    };
    Domain.prototype.enter = function() {};
    Domain.prototype.exit = function() {};
    Domain.prototype.dispose = function() {};

    var domain = {
        Domain: Domain,
        create: function() { return new Domain(); },
        createDomain: function() { return new Domain(); },
        active: null
    };

    globalThis.__requireCache['domain'] = domain;
    globalThis.__requireCache['node:domain'] = domain;
}());

// ── dns ───────────────────────────────────────────────────────────────────────
// Many network packages import dns but 3va can't do real DNS; return stubs
// that signal "not supported" gracefully.
(function() {
    function notSupported(name) {
        return function() {
            var cb = arguments[arguments.length - 1];
            var err = Object.assign(new Error('DNS not supported in 3va: ' + name),
                { code: 'ENOTSUP', syscall: name });
            if (typeof cb === 'function') { setTimeout(function() { cb(err); }, 0); }
            else throw err;
        };
    }

    var dns = {
        lookup: notSupported('lookup'),
        lookupService: notSupported('lookupService'),
        resolve: notSupported('resolve'),
        resolve4: notSupported('resolve4'),
        resolve6: notSupported('resolve6'),
        resolveMx: notSupported('resolveMx'),
        resolveTxt: notSupported('resolveTxt'),
        resolveSrv: notSupported('resolveSrv'),
        resolveNs: notSupported('resolveNs'),
        resolveCname: notSupported('resolveCname'),
        resolveNaptr: notSupported('resolveNaptr'),
        resolvePtr: notSupported('resolvePtr'),
        resolveSoa: notSupported('resolveSoa'),
        resolveAny: notSupported('resolveAny'),
        reverse: notSupported('reverse'),
        setServers: function() {},
        getServers: function() { return []; },
        ADDRCONFIG: 0,
        V4MAPPED: 8,
        ALL: 16,
        promises: {
            lookup: function(hostname) { return Promise.reject(Object.assign(new Error('DNS not supported'), { code: 'ENOTSUP' })); },
            resolve: function(hostname) { return Promise.reject(Object.assign(new Error('DNS not supported'), { code: 'ENOTSUP' })); },
            resolve4: function(hostname) { return Promise.reject(Object.assign(new Error('DNS not supported'), { code: 'ENOTSUP' })); },
            resolve6: function(hostname) { return Promise.reject(Object.assign(new Error('DNS not supported'), { code: 'ENOTSUP' })); }
        }
    };
    dns.Resolver = function() { return dns; };

    globalThis.__requireCache['dns'] = dns;
    globalThis.__requireCache['node:dns'] = dns;
    globalThis.__requireCache['dns/promises'] = dns.promises;
    globalThis.__requireCache['node:dns/promises'] = dns.promises;
}());

// ── v8 ────────────────────────────────────────────────────────────────────────
// Used by Jest, clinic, and memory profiling tools.
(function() {
    var v8 = {
        getHeapStatistics: function() {
            return { total_heap_size: 0, total_heap_size_executable: 0,
                     total_physical_size: 0, total_available_size: 134217728,
                     used_heap_size: 0, heap_size_limit: 536870912,
                     malloced_memory: 0, peak_malloced_memory: 0,
                     does_zap_garbage: 0, number_of_native_contexts: 0,
                     number_of_detached_contexts: 0 };
        },
        getHeapSpaceStatistics: function() { return []; },
        getHeapCodeStatistics: function() { return { code_and_metadata_size: 0, bytecode_and_metadata_size: 0, external_script_source_size: 0 }; },
        getHeapSnapshot: function() { return null; },
        setFlagsFromString: function() {},
        stopCoverage: function() {},
        takeCoverage: function() {},
        writeHeapSnapshot: function() { return ''; },
        serialize: function(v) { return Buffer.from(JSON.stringify(v)); },
        deserialize: function(b) { try { return JSON.parse(Buffer.from(b).toString()); } catch(e) { return null; } },
        Serializer: function() {},
        Deserializer: function() {},
        DefaultSerializer: function() {},
        DefaultDeserializer: function() {},
        promiseHooks: { onInit: function() {}, onSettled: function() {}, onBefore: function() {}, onAfter: function() {}, createHook: function() { return { enable: function(){}, disable: function(){} }; } }
    };

    globalThis.__requireCache['v8'] = v8;
    globalThis.__requireCache['node:v8'] = v8;
}());

// ── stream completeness: pipeline, finished, Readable.from ───────────────────
(function() {
    var stream = globalThis.__requireCache['stream'];
    if (!stream) return;

    // pipeline(src, ...transforms, dest[, cb]) — chains streams and handles errors
    stream.pipeline = function pipeline() {
        var args = Array.prototype.slice.call(arguments);
        var cb = typeof args[args.length - 1] === 'function' ? args.pop() : null;
        var streams = args;
        if (streams.length < 2) {
            var err = new Error('pipeline requires at least 2 streams');
            if (cb) { cb(err); return; }
            throw err;
        }
        var error = null;
        function done(err) {
            if (!error) {
                error = err || null;
                if (cb) cb(error);
            }
        }
        for (var i = 0; i < streams.length - 1; i++) {
            (function(src, dest) {
                if (src && src.pipe) src.pipe(dest);
                if (src && src.on) {
                    src.on('error', done);
                    src.on('end', function() {
                        if (dest && dest.end) dest.end();
                    });
                }
            })(streams[i], streams[i + 1]);
        }
        var last = streams[streams.length - 1];
        if (last && last.on) last.on('finish', function() { done(null); });
        return last;
    };

    // finished(stream, [options,] cb) — fires cb when stream is done/error
    stream.finished = function finished(s, options, cb) {
        if (typeof options === 'function') { cb = options; options = {}; }
        if (!cb) return new Promise(function(resolve, reject) {
            stream.finished(s, options, function(err) { err ? reject(err) : resolve(); });
        });
        function onError(err) { cleanup(); cb(err); }
        function onEnd() { cleanup(); cb(null); }
        function onFinish() { cleanup(); cb(null); }
        function cleanup() {
            if (s.removeListener) {
                s.removeListener('error', onError);
                s.removeListener('end', onEnd);
                s.removeListener('finish', onFinish);
            }
        }
        if (s.on) {
            s.on('error', onError);
            s.on('end', onEnd);
            s.on('finish', onFinish);
        } else {
            cb(null);
        }
        return cleanup;
    };

    // Readable.from(iterable) — creates a Readable from an async/sync iterable
    stream.Readable.from = function from(iterable, options) {
        var r = new stream.Readable(Object.assign({ objectMode: true }, options));
        r._reading = false;
        function consume() {
            if (iterable && typeof iterable[Symbol.asyncIterator] === 'function') {
                var it = iterable[Symbol.asyncIterator]();
                (function loop() {
                    it.next().then(function(res) {
                        if (res.done) { r.push(null); }
                        else { r.push(res.value); loop(); }
                    }, function(err) { r.emit('error', err); });
                })();
            } else if (iterable && typeof iterable[Symbol.iterator] === 'function') {
                for (var val of iterable) r.push(val);
                r.push(null);
            } else {
                r.push(iterable);
                r.push(null);
            }
        }
        setTimeout(consume, 0);
        return r;
    };

    // stream/promises — Node 15+ async wrappers
    var streamPromises = {
        pipeline: function() {
            var args = Array.prototype.slice.call(arguments);
            return new Promise(function(resolve, reject) {
                args.push(function(err) { err ? reject(err) : resolve(); });
                stream.pipeline.apply(null, args);
            });
        },
        finished: function(s, options) {
            return stream.finished(s, options);
        }
    };
    globalThis.__requireCache['stream/promises'] = streamPromises;
    globalThis.__requireCache['node:stream/promises'] = streamPromises;
    globalThis.__requireCache['stream/consumers'] = {
        arrayBuffer: function(s) { return new Promise(function(resolve) { var bufs = []; s.on('data', function(c) { bufs.push(c); }); s.on('end', function() { var tot = bufs.reduce(function(a,b){return a+b.length;},0); var buf = new Uint8Array(tot); var off=0; bufs.forEach(function(b){ buf.set(b,off); off+=b.length; }); resolve(buf.buffer); }); }); },
        text: function(s) { return new Promise(function(resolve) { var chunks = []; s.on('data', function(c) { chunks.push(c.toString()); }); s.on('end', function() { resolve(chunks.join('')); }); }); },
        json: function(s) { return this.text(s).then(function(t) { return JSON.parse(t); }); },
        blob: function(s) { return this.arrayBuffer(s).then(function(ab) { return new Blob([ab]); }); },
        buffer: function(s) { return this.arrayBuffer(s).then(function(ab) { return Buffer.from(ab); }); }
    };
    globalThis.__requireCache['node:stream/consumers'] = globalThis.__requireCache['stream/consumers'];
}());

// ── util completeness ─────────────────────────────────────────────────────────
(function() {
    var util = globalThis.__requireCache['util'];
    if (!util) return;

    // util.promisify — already in most impls, ensure it handles common Symbol
    if (!util.promisify) {
        util.promisify = function(fn) {
            return function() {
                var args = Array.prototype.slice.call(arguments);
                return new Promise(function(resolve, reject) {
                    args.push(function(err, val) { err ? reject(err) : resolve(val); });
                    fn.apply(this, args);
                });
            };
        };
    }
    util.promisify.custom = Symbol.for('nodejs.util.promisify.custom');

    // util.callbackify
    if (!util.callbackify) {
        util.callbackify = function(fn) {
            return function() {
                var args = Array.prototype.slice.call(arguments);
                var cb = args.pop();
                fn.apply(this, args).then(
                    function(v) { cb(null, v); },
                    function(e) { cb(e); }
                );
            };
        };
    }

    // util.types — used by many packages for runtime type detection.
    // Unconditionally replace so existing stub objects are upgraded.
    {
        var _types = util.types || {};
        util.types = Object.assign({
            isAnyArrayBuffer: function(v) { return v instanceof ArrayBuffer; },
            isArrayBufferView: function(v) { return ArrayBuffer.isView(v); },
            isBigInt64Array: function(v) { return v instanceof BigInt64Array; },
            isBigUint64Array: function(v) { return v instanceof BigUint64Array; },
            isBooleanObject: function(v) { return v instanceof Boolean; },
            isBoxedPrimitive: function(v) { return v instanceof Boolean || v instanceof Number || v instanceof String || v instanceof Symbol || v instanceof BigInt; },
            isDataView: function(v) { return v instanceof DataView; },
            isDate: function(v) { return v instanceof Date; },
            isFloat32Array: function(v) { return v instanceof Float32Array; },
            isFloat64Array: function(v) { return v instanceof Float64Array; },
            isGeneratorFunction: function(v) { var t = Object.prototype.toString.call(v); return t === '[object GeneratorFunction]' || t === '[object AsyncGeneratorFunction]'; },
            isGeneratorObject: function(v) { return typeof v === 'object' && v !== null && typeof v.next === 'function' && typeof v.throw === 'function'; },
            isInt8Array: function(v) { return v instanceof Int8Array; },
            isInt16Array: function(v) { return v instanceof Int16Array; },
            isInt32Array: function(v) { return v instanceof Int32Array; },
            isMap: function(v) { return v instanceof Map; },
            isMapIterator: function(v) { return Object.prototype.toString.call(v) === '[object Map Iterator]'; },
            isModuleNamespaceObject: function(v) { return false; },
            isNativeError: function(v) { return v instanceof Error; },
            isNumberObject: function(v) { return v instanceof Number; },
            isPromise: function(v) { return v && typeof v.then === 'function'; },
            isProxy: function(v) { return false; },
            isRegExp: function(v) { return v instanceof RegExp; },
            isSet: function(v) { return v instanceof Set; },
            isSetIterator: function(v) { return Object.prototype.toString.call(v) === '[object Set Iterator]'; },
            isSharedArrayBuffer: function(v) { return typeof SharedArrayBuffer !== 'undefined' && v instanceof SharedArrayBuffer; },
            isStringObject: function(v) { return v instanceof String; },
            isSymbolObject: function(v) { return v instanceof Symbol; },
            isTypedArray: function(v) { return ArrayBuffer.isView(v) && !(v instanceof DataView); },
            isUint8Array: function(v) { return v instanceof Uint8Array; },
            isUint8ClampedArray: function(v) { return v instanceof Uint8ClampedArray; },
            isUint16Array: function(v) { return v instanceof Uint16Array; },
            isUint32Array: function(v) { return v instanceof Uint32Array; },
            isWeakMap: function(v) { return v instanceof WeakMap; },
            isWeakSet: function(v) { return v instanceof WeakSet; },
            isAsyncFunction: function(v) { return Object.prototype.toString.call(v) === '[object AsyncFunction]'; },
            isArgumentsObject: function(v) { return Object.prototype.toString.call(v) === '[object Arguments]'; },
            isCryptoKey: function(v) { return false; },
            isKeyObject: function(v) { return false; }
        }, _types);
    }

    // util.TextEncoder / util.TextDecoder (mirrored from globalThis)
    if (!util.TextEncoder && typeof TextEncoder !== 'undefined') util.TextEncoder = TextEncoder;
    if (!util.TextDecoder && typeof TextDecoder !== 'undefined') util.TextDecoder = TextDecoder;

    // util.styleText — Node 20+ simple ANSI styling (no-op since we're not a TTY)
    if (!util.styleText) {
        util.styleText = function(format, text) { return text; };
    }

    // util.isDeepStrictEqual — used by assertion libraries
    if (!util.isDeepStrictEqual) {
        util.isDeepStrictEqual = function deepEqual(a, b) {
            if (a === b) return true;
            if (typeof a !== 'object' || a === null || typeof b !== 'object' || b === null) return false;
            var ka = Object.keys(a), kb = Object.keys(b);
            if (ka.length !== kb.length) return false;
            for (var i = 0; i < ka.length; i++) {
                var k = ka[i];
                if (!Object.prototype.hasOwnProperty.call(b, k)) return false;
                if (!deepEqual(a[k], b[k])) return false;
            }
            return true;
        };
    }
}());

// ── constants ─────────────────────────────────────────────────────────────────
// Many packages import 'constants' or use os.constants / fs.constants.
(function() {
    var constants = {
        // errno
        E2BIG: 7, EACCES: 13, EADDRINUSE: 98, EADDRNOTAVAIL: 99,
        EAFNOSUPPORT: 97, EAGAIN: 11, EALREADY: 114, EBADF: 9,
        EBUSY: 16, ECANCELED: 125, ECHILD: 10, ECONNABORTED: 103,
        ECONNREFUSED: 111, ECONNRESET: 104, EDEADLK: 35, EDESTADDRREQ: 89,
        EDOM: 33, EEXIST: 17, EFAULT: 14, EFBIG: 27, EHOSTUNREACH: 113,
        EINPROGRESS: 115, EINTR: 4, EINVAL: 22, EIO: 5, EISCONN: 106,
        EISDIR: 21, ELOOP: 40, EMFILE: 24, EMSGSIZE: 90, ENAMETOOLONG: 36,
        ENETDOWN: 100, ENETRESET: 102, ENETUNREACH: 101, ENFILE: 23,
        ENOBUFS: 105, ENODEV: 19, ENOENT: 2, ENOEXEC: 8, ENOLCK: 37,
        ENOMEM: 12, ENOPROTOOPT: 92, ENOSPC: 28, ENOSYS: 38, ENOTCONN: 107,
        ENOTDIR: 20, ENOTEMPTY: 39, ENOTSOCK: 88, ENOTSUP: 95,
        EOVERFLOW: 75, EPERM: 1, EPIPE: 32, EPROTONOSUPPORT: 93,
        EPROTOTYPE: 91, ERANGE: 34, EROFS: 30, ESPIPE: 29, ESRCH: 3,
        ETIMEDOUT: 110, ETXTBSY: 26, EWOULDBLOCK: 11, EXDEV: 18,
        // signals
        SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGILL: 4, SIGTRAP: 5,
        SIGABRT: 6, SIGIOT: 6, SIGBUS: 7, SIGFPE: 8, SIGKILL: 9,
        SIGUSR1: 10, SIGSEGV: 11, SIGUSR2: 12, SIGPIPE: 13, SIGALRM: 14,
        SIGTERM: 15, SIGCHLD: 17, SIGCONT: 18, SIGSTOP: 19, SIGTSTP: 20,
        SIGTTIN: 21, SIGTTOU: 22, SIGURG: 23, SIGXCPU: 24, SIGXFSZ: 25,
        SIGVTALRM: 26, SIGPROF: 27, SIGWINCH: 28, SIGIO: 29,
        // open flags
        O_RDONLY: 0, O_WRONLY: 1, O_RDWR: 2, O_CREAT: 64, O_EXCL: 128,
        O_NOCTTY: 256, O_TRUNC: 512, O_APPEND: 1024, O_DIRECTORY: 65536,
        O_NOATIME: 262144, O_NOFOLLOW: 131072, O_SYNC: 1052672,
        O_DSYNC: 4096, O_SYMLINK: 0, O_DIRECT: 16384, O_NONBLOCK: 2048,
        // file types
        S_IFMT: 61440, S_IFREG: 32768, S_IFDIR: 16384, S_IFCHR: 8192,
        S_IFBLK: 24576, S_IFIFO: 4096, S_IFLNK: 40960, S_IFSOCK: 49152,
        S_IRWXU: 448, S_IRUSR: 256, S_IWUSR: 128, S_IXUSR: 64,
        S_IRWXG: 56, S_IRGRP: 32, S_IWGRP: 16, S_IXGRP: 8,
        S_IRWXO: 7, S_IROTH: 4, S_IWOTH: 2, S_IXOTH: 1,
        // access
        F_OK: 0, R_OK: 4, W_OK: 2, X_OK: 1,
        // tls
        SSL_OP_ALL: 0, SSL_OP_NO_SSLv2: 0, SSL_OP_NO_SSLv3: 0
    };

    globalThis.__requireCache['constants'] = constants;
    globalThis.__requireCache['node:constants'] = constants;

    // Merge into fs.constants and os.constants if they exist
    var fs = globalThis.__requireCache['fs'];
    if (fs && !fs.constants) fs.constants = constants;
    var osM = globalThis.__requireCache['os'];
    if (osM && !osM.constants) osM.constants = { signals: {}, errno: {}, priority: {} };
}());

// ── process: complete stdin + signals + missing fields ───────────────────────
(function() {
    var p = globalThis.process;
    if (!p) return;

    // stdin as a no-op Readable (CLI tools that read from stdin get an ended stream)
    var stream = globalThis.__requireCache['stream'];
    if (stream && !p.stdin) {
        var stdin = new stream.Readable({ read: function() { this.push(null); } });
        stdin.isTTY = false;
        stdin.fd = 0;
        p.stdin = stdin;
    }

    // execPath / execArgv
    if (!p.execPath) p.execPath = p.argv && p.argv[0] || '3va';
    if (!p.execArgv) p.execArgv = [];
    if (!p.mainModule) p.mainModule = undefined;

    // process.on / once / emit (EventEmitter-like, minimal)
    if (!p._events) {
        p._events = {};
        p.on = p.addListener = function(event, fn) {
            if (!this._events[event]) this._events[event] = [];
            this._events[event].push(fn);
            return this;
        };
        p.once = function(event, fn) {
            var self = this;
            function wrapper() { self.removeListener(event, wrapper); fn.apply(this, arguments); }
            return this.on(event, wrapper);
        };
        p.off = p.removeListener = function(event, fn) {
            if (this._events[event]) {
                this._events[event] = this._events[event].filter(function(f) { return f !== fn; });
            }
            return this;
        };
        p.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            var handlers = this._events[event] || [];
            handlers.forEach(function(fn) { try { fn.apply(null, args); } catch(e) {} });
            return handlers.length > 0;
        };
        p.removeAllListeners = function(event) {
            if (event) delete this._events[event]; else this._events = {};
            return this;
        };
        p.listenerCount = function(event) { return (this._events[event] || []).length; };
        p.listeners = function(event) { return (this._events[event] || []).slice(); };
    }

    // process.abort / process.kill (stubs)
    if (!p.abort) p.abort = function() { process.exit(134); };
    if (!p.kill) p.kill = function(pid, signal) {};

    // process.binding — used by older packages to access internals
    if (!p.binding) p.binding = function(name) {
        if (name === 'constants') return globalThis.__requireCache['constants'] || {};
        return {};
    };

    // process.report (Node 11+)
    if (!p.report) p.report = { writeReport: function() {}, getReport: function() { return '{}'; } };

    // process.allowedNodeEnvironmentFlags
    if (!p.allowedNodeEnvironmentFlags) p.allowedNodeEnvironmentFlags = new Set();

    // process.config — used by node-gyp checks
    if (!p.config) p.config = { variables: { node_root_dir: '', node_prefix: '' } };
}());

// ── reflect-metadata ─────────────────────────────────────────────────────────
// Minimal polyfill for NestJS, TypeORM, tsyringe, routing-controllers.
// Implements: Reflect.metadata, defineMetadata, getMetadata, getOwnMetadata,
//             hasMetadata, hasOwnMetadata, deleteMetadata, getMetadataKeys,
//             getOwnMetadataKeys, decorate.
(function() {
    if (typeof Reflect === 'undefined') globalThis.Reflect = {};
    var _store = typeof WeakMap !== 'undefined' ? new WeakMap() : null;
    var _fallback = [];
    function _getEntry(target) {
        if (_store) {
            if (!_store.has(target)) _store.set(target, Object.create(null));
            return _store.get(target);
        }
        for (var i = 0; i < _fallback.length; i++) { if (_fallback[i][0] === target) return _fallback[i][1]; }
        var e = Object.create(null); _fallback.push([target, e]); return e;
    }
    function _slotKey(prop) { return prop === undefined ? '__$own$__' : '__$' + String(prop) + '$__'; }
    function _getMap(target, prop) { var e = _getEntry(target); var k = _slotKey(prop); return e[k] || null; }
    function _getOrCreate(target, prop) { var e = _getEntry(target); var k = _slotKey(prop); if (!e[k]) e[k] = Object.create(null); return e[k]; }
    function _chain(target, prop, metaKey) {
        var t = target;
        while (t && typeof t === 'object' || typeof t === 'function') {
            var m = _getMap(t, prop);
            if (m && Object.prototype.hasOwnProperty.call(m, metaKey)) return m;
            t = Object.getPrototypeOf ? Object.getPrototypeOf(t) : null;
        }
        return null;
    }
    Reflect.metadata = function(metaKey, metaValue) {
        return function(target, propertyKey) { Reflect.defineMetadata(metaKey, metaValue, target, propertyKey); };
    };
    Reflect.defineMetadata = function(metaKey, metaValue, target, propertyKey) { _getOrCreate(target, propertyKey)[metaKey] = metaValue; };
    Reflect.getOwnMetadata = function(metaKey, target, propertyKey) { var m = _getMap(target, propertyKey); return m ? m[metaKey] : undefined; };
    Reflect.getMetadata = function(metaKey, target, propertyKey) { var m = _chain(target, propertyKey, metaKey); return m ? m[metaKey] : undefined; };
    Reflect.hasOwnMetadata = function(metaKey, target, propertyKey) { var m = _getMap(target, propertyKey); return !!(m && Object.prototype.hasOwnProperty.call(m, metaKey)); };
    Reflect.hasMetadata = function(metaKey, target, propertyKey) { return !!_chain(target, propertyKey, metaKey); };
    Reflect.deleteMetadata = function(metaKey, target, propertyKey) { var m = _getMap(target, propertyKey); if (m) delete m[metaKey]; };
    Reflect.getOwnMetadataKeys = function(target, propertyKey) { var m = _getMap(target, propertyKey); return m ? Object.keys(m) : []; };
    Reflect.getMetadataKeys = function(target, propertyKey) {
        var seen = Object.create(null); var result = []; var t = target;
        while (t && typeof t === 'object' || typeof t === 'function') {
            var m = _getMap(t, propertyKey);
            if (m) Object.keys(m).forEach(function(k) { if (!seen[k]) { seen[k] = true; result.push(k); } });
            t = Object.getPrototypeOf ? Object.getPrototypeOf(t) : null;
        }
        return result;
    };
    Reflect.decorate = function(decorators, target, propertyKey, descriptor) {
        if (propertyKey === undefined) return decorators.reduceRight(function(c, d) { return d(c) || c; }, target);
        return decorators.reduceRight(function(d2, dec) {
            return dec(target, propertyKey, d2) || d2;
        }, descriptor !== undefined ? descriptor : (Object.getOwnPropertyDescriptor ? Object.getOwnPropertyDescriptor(target, propertyKey) : undefined));
    };
    if (globalThis.__requireCache) {
        globalThis.__requireCache['reflect-metadata'] = Reflect;
        globalThis.__requireCache['node:reflect-metadata'] = Reflect;
    }
}());

// ── Additional module aliases ─────────────────────────────────────────────────
// Some packages do `require('node:X')` even when bare `require('X')` is standard.
// Make sure all known modules are accessible with both bare and node: prefix.
(function() {
    var BARE = [
        'assert','buffer','child_process','cluster','console','constants',
        'crypto','dgram','diagnostics_channel','dns','domain','events',
        'fs','http','http2','https','module','net','os','path','perf_hooks',
        'process','punycode','querystring','readline','stream','string_decoder',
        'timers','tls','tty','url','util','v8','vm','worker_threads','zlib'
    ];
    BARE.forEach(function(name) {
        var m = globalThis.__requireCache[name];
        if (m !== undefined && globalThis.__requireCache['node:' + name] === undefined) {
            globalThis.__requireCache['node:' + name] = m;
        } else if (m === undefined && globalThis.__requireCache['node:' + name] !== undefined) {
            globalThis.__requireCache[name] = globalThis.__requireCache['node:' + name];
        }
    });
}());

}());
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
    // Strip jsr: specifier prefix — treat as a scoped package in node_modules/
    let path = path.strip_prefix("jsr:").unwrap_or(path);

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
