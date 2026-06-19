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
        globalThis.__fallbackModules = {};
        globalThis.module = { exports: {} };
        globalThis.exports = globalThis.module.exports;
        globalThis.__filename = '';
        globalThis.__dirname = '';
        // React Native globals
        globalThis.__DEV__ = false;

        // ponytail: minimal Intl shim — QuickJS lacks Intl but has Date.toLocaleString.
        // Covers Intl.DateTimeFormat (used by @nestjs/common logger) and stubs the rest.
        // Upgrade to full ICU if locale-specific formatting becomes a real requirement.
        if (typeof Intl === 'undefined') {
            globalThis.Intl = {};
        }
        if (typeof Intl.DateTimeFormat !== 'function') {
            Intl.DateTimeFormat = function DateTimeFormat(locale, options) {
                if (!(this instanceof DateTimeFormat)) return new DateTimeFormat(locale, options);
                this._locale = locale;
                this._options = options || {};
            };
            Intl.DateTimeFormat.prototype.format = function(date) {
                return (date instanceof Date ? date : new Date(date)).toLocaleString();
            };
            Intl.DateTimeFormat.prototype.formatToParts = function(date) { return []; };
            Intl.DateTimeFormat.prototype.resolvedOptions = function() {
                return Object.assign({ locale: this._locale || 'en-US', calendar: 'gregory', numberingSystem: 'latn', timeZone: 'UTC' }, this._options);
            };
            Intl.DateTimeFormat.supportedLocalesOf = function() { return []; };
        }
        if (typeof Intl.NumberFormat !== 'function') {
            Intl.NumberFormat = function NumberFormat(locale, options) {
                if (!(this instanceof NumberFormat)) return new NumberFormat(locale, options);
                this._locale = locale; this._options = options || {};
            };
            Intl.NumberFormat.prototype.format = function(n) { return String(n); };
            Intl.NumberFormat.prototype.formatToParts = function(n) { return []; };
            Intl.NumberFormat.prototype.resolvedOptions = function() { return { locale: this._locale || 'en-US' }; };
            Intl.NumberFormat.supportedLocalesOf = function() { return []; };
        }
        if (typeof Intl.Collator !== 'function') {
            Intl.Collator = function Collator(locale, options) {
                if (!(this instanceof Collator)) return new Collator(locale, options);
            };
            Intl.Collator.prototype.compare = function(a, b) { return a < b ? -1 : a > b ? 1 : 0; };
            Intl.Collator.prototype.resolvedOptions = function() { return { locale: 'en-US' }; };
            Intl.Collator.supportedLocalesOf = function() { return []; };
        }
        if (typeof Intl.getCanonicalLocales !== 'function') {
            Intl.getCanonicalLocales = function(locales) {
                return Array.isArray(locales) ? locales : (locales ? [locales] : []);
            };
        }
        if (typeof Intl.supportedValuesOf !== 'function') {
            Intl.supportedValuesOf = function() { return []; };
        }

        // ES2024: ArrayBuffer.prototype.resizable — always false in QuickJS
        if (typeof ArrayBuffer !== 'undefined' && !('resizable' in ArrayBuffer.prototype)) {
            Object.defineProperty(ArrayBuffer.prototype, 'resizable', {
                get: function() { return false; },
                enumerable: false, configurable: true
            });
        }
        // ES2024: SharedArrayBuffer.prototype.growable — always false in QuickJS
        if (typeof SharedArrayBuffer !== 'undefined' && !('growable' in SharedArrayBuffer.prototype)) {
            Object.defineProperty(SharedArrayBuffer.prototype, 'growable', {
                get: function() { return false; },
                enumerable: false, configurable: true
            });
        }
    "#,
    )?;

    // Native __readFile(resolvedAbsPath) -> String
    // Accepts an already-resolved absolute path; does permission check + optional TS transpile.
    let perms = permissions.clone();
    let read_file_fn = Function::new(
        ctx.clone(),
        move |ctx: rquickjs::Ctx<'_>, args: Rest<String>| -> Result<String> {
            let path_str = args.0.into_iter().next().ok_or_else(|| {
                rquickjs::Error::new_from_js("value", "__readFile() needs a path")
            })?;

            let full_path = PathBuf::from(&path_str);

            // Permission check
            if !perms.check(&Capability::FileRead(full_path.clone())) {
                let msg = format!(
                    "Permission denied: --allow-read={} is required",
                    full_path.display()
                );
                let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
                return match ctx
                    .eval::<rquickjs::Value, _>(format!("new Error(\"{}\")", escaped).as_str())
                {
                    Ok(v) => Err(ctx.throw(v)),
                    Err(e) => Err(e),
                };
            }

            // Read the file
            let source = std::fs::read_to_string(&full_path).map_err(|e| {
                let msg = format!("ENOENT: {}: '{}'", e, path_str);
                let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
                match ctx.eval::<rquickjs::Value, _>(format!("new Error(\"{}\")", escaped).as_str())
                {
                    Ok(v) => ctx.throw(v),
                    Err(e2) => e2,
                }
            })?;

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
            } else if path_str.ends_with(".cjs")
                || path_str.ends_with(".json")
                || path_str.contains("@exodus/bytes")
            {
                source
            } else {
                crate::transpiler::transpile_js(&source)
            };

            Ok(source)
        },
    )?;
    globals.set("__readFile", read_file_fn)?;

    // Native __resolvePath(path, basedir?) -> String
    // basedir is used as the base for relative imports (e.g. require('./lib/foo') inside a package).
    // Throws proper Node.js errors (ERR_PACKAGE_PATH_NOT_EXPORTED, MODULE_NOT_FOUND).
    let resolve_fn = Function::new(
        ctx.clone(),
        move |ctx: rquickjs::Ctx<'_>, args: Rest<String>| -> Result<String> {
            let mut it = args.0.into_iter();
            let path_str = it.next().unwrap_or_default();
            let basedir = it.next();
            match resolve_path_from(&path_str, basedir.as_deref()) {
                Ok(p) => Ok(p.to_string_lossy().to_string()),
                Err(msg) => {
                    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
                    let err_val = ctx
                        .eval::<rquickjs::Value, _>(format!("new Error(\"{}\")", escaped).as_str())
                        .map_err(|_| rquickjs::Error::new_from_js("resolve", "error"))?;
                    Err(ctx.throw(err_val))
                }
            }
        },
    )?;
    globals.set("__resolvePath", resolve_fn)?;

    // Native __dnsLookup(hostname) -> Promise<string>
    // Returns a JSON-encoded array of resolved address strings (e.g. '["1.2.3.4"]').
    let dns_fn = Function::new(
        ctx.clone(),
        rquickjs::function::Async(move |hostname: String| async move {
            let result =
                tokio::task::spawn_blocking(move || -> std::result::Result<Vec<String>, String> {
                    use std::net::ToSocketAddrs;
                    let addr_str = format!("{}:0", hostname);
                    match addr_str.to_socket_addrs() {
                        Ok(addrs) => {
                            let ips: Vec<String> = addrs.map(|a| a.ip().to_string()).collect();
                            if ips.is_empty() {
                                Err(format!("ENOTFOUND: {hostname}"))
                            } else {
                                Ok(ips)
                            }
                        }
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await
                .map_err(|e| {
                    rquickjs::Error::new_from_js_message(
                        "dns",
                        "lookup",
                        format!("join error: {e}"),
                    )
                })?;

            match result {
                Ok(ips) => Ok(serde_json::to_string(&ips).unwrap_or_else(|_| "[]".to_string())),
                Err(e) => Err(rquickjs::Error::new_from_js_message("dns", "lookup", e)),
            }
        }),
    )?;
    globals.set("__dnsLookup", dns_fn)?;

    // Register Node.js built-in module shims in the require cache by their bare names.
    // These are looked up before any file resolution, so require('util') etc. always work.
    ctx.eval::<(), _>(r#"
        (function() {
            // ── Error.captureStackTrace polyfill (V8 API, used by depd, express, etc.) ─
            if (typeof Error.captureStackTrace === 'undefined') {
                function mockCallsite() {
                    return {
                        getFileName: function() { return ''; },
                        getLineNumber: function() { return 0; },
                        getColumnNumber: function() { return 0; },
                        isEval: function() { return false; },
                        getThis: function() { return null; },
                        getTypeName: function() { return 'Object'; },
                        getFunctionName: function() { return ''; },
                        getMethodName: function() { return ''; },
                        toString: function() { return ''; }
                    };
                }
                Error.captureStackTrace = function(obj) {
                    obj.stack = [mockCallsite(), mockCallsite(), mockCallsite()];
                };
                Error.stackTraceLimit = 10;
            }

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
                TextDecoder: globalThis.TextDecoder,
                debuglog: function(section) {
                    var enabled = typeof process !== 'undefined' && process.env
                        && (process.env.NODE_DEBUG || '').split(',').indexOf(section) !== -1;
                    if (enabled) {
                        return function() {
                            var args = Array.prototype.slice.call(arguments);
                            args.unshift(section.toUpperCase() + ':');
                            process.stderr.write(args.join(' ') + '\n');
                        };
                    }
                    return function() {};
                }
            };
            globalThis.__requireCache['util'] = util;

            // ── events ────────────────────────────────────────────────────────────
            function EventEmitter() { this._events = Object.create(null); this._maxListeners = 10; }
            // All EventEmitter methods lazily initialize _events so that objects
            // that mix in EventEmitter.prototype (without calling `new EventEmitter()`)
            // work correctly (e.g. express's app function).
            function _ensureEvents(self) { if (!self._events) self._events = Object.create(null); }
            EventEmitter.prototype.on = EventEmitter.prototype.addListener = function(ev, fn) {
                _ensureEvents(this);
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
                _ensureEvents(this);
                if (!this._events[ev]) return this;
                this._events[ev] = this._events[ev].filter(function(f) { return f !== fn && f._orig !== fn; });
                return this;
            };
            EventEmitter.prototype.removeAllListeners = function(ev) {
                _ensureEvents(this);
                if (ev) { delete this._events[ev]; } else { this._events = Object.create(null); } return this;
            };
            EventEmitter.prototype.emit = function(ev) {
                _ensureEvents(this);
                var args = Array.prototype.slice.call(arguments, 1);
                var listeners = (this._events[ev] || []).slice();
                listeners.forEach(function(fn) { fn.apply(null, args); });
                return listeners.length > 0;
            };
            EventEmitter.prototype.listeners = function(ev) {
                _ensureEvents(this);
                return (this._events[ev] || []).map(function(f) { return f._orig || f; });
            };
            EventEmitter.prototype.rawListeners = function(ev) { _ensureEvents(this); return (this._events[ev] || []).slice(); };
            EventEmitter.prototype.listenerCount = function(ev) { _ensureEvents(this); return (this._events[ev] || []).length; };
            EventEmitter.prototype.setMaxListeners = function(n) { this._maxListeners = n; return this; };
            EventEmitter.prototype.getMaxListeners = function() { return this._maxListeners || EventEmitter.defaultMaxListeners; };
            EventEmitter.prototype.eventNames = function() { _ensureEvents(this); return Object.keys(this._events).filter(function(k) { return this._events[k] && this._events[k].length > 0; }, this); };
            EventEmitter.prototype.prependListener = function(ev, fn) {
                _ensureEvents(this);
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
            // Static EventEmitter.once(emitter, event) → Promise  (Node 11.13+)
            EventEmitter.once = function(emitter, ev) {
                return new Promise(function(resolve, reject) {
                    function onErr(e) { emitter.removeListener(ev, onEvt); reject(e); }
                    function onEvt() { emitter.removeListener('error', onErr); resolve(Array.prototype.slice.call(arguments)); }
                    emitter.once(ev, onEvt);
                    if (ev !== 'error') emitter.once('error', onErr);
                });
            };
            // Static EventEmitter.on(emitter, event) → AsyncIterator  (Node 12.16+)
            EventEmitter.on = function(emitter, ev) {
                var buf = [], waiting = null;
                emitter.on(ev, function() {
                    var args = Array.prototype.slice.call(arguments);
                    if (waiting) { var r = waiting; waiting = null; r({ value: args, done: false }); }
                    else buf.push(args);
                });
                return {
                    next: function() {
                        if (buf.length) return Promise.resolve({ value: buf.shift(), done: false });
                        return new Promise(function(res) { waiting = res; });
                    },
                    return: function() { return Promise.resolve({ value: undefined, done: true }); },
                };
            };
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
            Object.defineProperty(Readable, Symbol.hasInstance, {
                value: function(instance) { return !!(instance && instance._readableState); }
            });
            Readable.prototype._read = function(size) {};
            // Override EventEmitter.on: auto-resume stream when 'data' listener added,
            // matching Node.js behavior. Deferred via setTimeout so both 'data' and 'end'
            // listeners are registered before data flows, AND the resume happens in
            // fire_pending() (BEFORE idle()), avoiding the spawner deadlock.
            Readable.prototype.on = Readable.prototype.addListener = function(ev, fn) {
                _ensureEvents(this);
                if (!this._events[ev]) this._events[ev] = [];
                this._events[ev].push(fn);
                if (ev === 'data' && !this._readableState.flowing && !this._readableState._resumeScheduled) {
                    this._readableState._resumeScheduled = true;
                    var self = this;
                    setTimeout(function() {
                        self._readableState._resumeScheduled = false;
                        if (!self._readableState.flowing) self.resume();
                    }, 0);
                }
                return this;
            };
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
                this.on('end', function() { if (!opts || opts.end !== false) dest.end(); });
                dest.emit('pipe', this);
                if (!this._readableState.flowing) this.resume();
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

            // ── Writable (with backpressure) ──────────────────────────────────────
            function Writable(opts) {
                Stream.call(this);
                this.writable = true;
                this._writableState = {
                    objectMode: !!(opts && opts.objectMode),
                    highWaterMark: (opts && opts.highWaterMark) || 16384,
                    length: 0, corked: 0, ended: false, finished: false,
                    needDrain: false, pendingcb: 0, buffered: [],
                };
                if (opts && typeof opts.write === 'function') this._write = opts.write;
                if (opts && typeof opts.final === 'function') this._final = opts.final;
            }
            util.inherits(Writable, Stream);
            Object.defineProperty(Writable, Symbol.hasInstance, {
                value: function(instance) { return !!(instance && instance._writableState); }
            });
            Writable.prototype._write = function(chunk, encoding, callback) { callback(); };
            Writable.prototype._final = function(callback) { callback(); };
            Writable.prototype.write = function(chunk, encoding, cb) {
                if (typeof encoding === 'function') { cb = encoding; encoding = 'utf8'; }
                var state = this._writableState;
                if (state.ended) {
                    var err = new Error('write after end');
                    this.emit('error', err);
                    if (cb) cb(err);
                    return false;
                }
                var chunkLen = 1;
                if (Buffer.isBuffer(chunk)) {
                    chunkLen = chunk.length;
                } else if (typeof chunk === 'string') {
                    chunkLen = new TextEncoder().encode(chunk).length;
                }
                state.length += chunkLen;
                state.pendingcb++;
                var self = this;
                function onflush(err) {
                    state.length = Math.max(0, state.length - chunkLen);
                    state.pendingcb--;
                    if (err) { self.emit('error', err); if (cb) cb(err); return; }
                    if (cb) cb(null);
                    if (state.needDrain && state.length < state.highWaterMark) {
                        state.needDrain = false;
                        self.emit('drain');
                    }
                }
                var ret = state.length < state.highWaterMark;
                if (!ret) state.needDrain = true;
                if (state.corked > 0) {
                    state.buffered.push({ chunk: chunk, encoding: encoding, cb: cb, onflush: onflush, chunkLen: chunkLen });
                    return true;
                }
                this._write(chunk, encoding || 'utf8', onflush);
                return ret;
            };
            Writable.prototype.end = function(chunk, encoding, cb) {
                if (typeof chunk === 'function') { cb = chunk; chunk = null; }
                if (typeof encoding === 'function') { cb = encoding; encoding = null; }
                var self = this;
                var state = this._writableState;
                state.ended = true;
                function finish() {
                    if (state.pendingcb > 0) { setTimeout(finish, 0); return; }
                    state.finished = true;
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
            Writable.prototype.cork = function() { this._writableState.corked++; };
            Writable.prototype.uncork = function() {
                var state = this._writableState;
                if (state.corked > 0) state.corked--;
                if (state.corked === 0 && state.buffered.length > 0) {
                    var buf = state.buffered.splice(0);
                    var self = this;
                    buf.forEach(function(item) {
                        self._write(item.chunk, item.encoding || 'utf8', item.onflush);
                    });
                }
            };

            // ── Transform ─────────────────────────────────────────────────────────
            function Transform(opts) {
                var self = this;
                Readable.call(self, opts);
                self.writable = true;
                self._writableState = {
                    objectMode: !!(opts && opts.objectMode),
                    highWaterMark: (opts && opts.highWaterMark) || 16384,
                    length: 0, corked: 0, ended: false, finished: false,
                    needDrain: false, pendingcb: 0, buffered: [],
                    destroying: false,
                };
                Object.defineProperties(self, {
                    writableLength: { get: function() { return self._writableState.length; } },
                    writableEnded: { get: function() { return self._writableState.ended; } },
                    writableFinished: { get: function() { return self._writableState.finished; } },
                });
                if (opts && typeof opts.transform === 'function') self._transform = opts.transform;
                if (opts && typeof opts.flush === 'function') self._flush = opts.flush;
                if (opts && typeof opts.final === 'function') self._final = opts.final;
            }
            util.inherits(Transform, Readable);
            Transform.prototype._transform = function(chunk, encoding, callback) { this.push(chunk); callback(); };
            Transform.prototype._flush = function(callback) { callback(); };
            Transform.prototype._final = function(callback) { callback(); };
            Transform.prototype.write = function(chunk, encoding, cb) {
                if (typeof encoding === 'function') { cb = encoding; encoding = 'utf8'; }
                var self = this;
                var state = self._writableState;
                if (state.ended) {
                    var err = new Error('write after end');
                    self.emit('error', err);
                    if (cb) cb(err);
                    return false;
                }
                state.pendingcb = (state.pendingcb || 0) + 1;
                self._transform(chunk, encoding || 'utf8', function(err, data) {
                    state.pendingcb--;
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
                var state = self._writableState;
                state.ended = true;
                function doFlush() {
                    self._flush(function(err, data) {
                        if (err) { self.emit('error', err); if (cb) cb(err); return; }
                        if (data != null) self.push(data);
                        self._final(function(err2) {
                            if (err2) { self.emit('error', err2); if (cb) cb(err2); return; }
                            self.push(null);
                            state.finished = true;
                            self.emit('finish');
                            self.emit('close');
                            if (cb) cb(null);
                        });
                    });
                }
                if (chunk != null) { self.write(chunk, encoding, function(err) { if (!err) doFlush(); else if (cb) cb(err); }); }
                else { doFlush(); }
            };
            Transform.prototype._destroy = function(err, callback) {
                if (err) this.emit('error', err);
                this.emit('close');
                if (typeof callback === 'function') callback(err);
            };
            Transform.prototype.destroy = function(err) {
                var state = this._writableState;
                var rState = this._readableState;
                if (state && state.destroying) return this;
                if (state) state.destroying = true;
                if (err) this.emit('error', err);
                if (rState) rState.ended = true;
                this._destroy(err, function() {});
                return this;
            };
            Transform.prototype.cork = function() { this._writableState.corked++; };
            Transform.prototype.uncork = function() {
                var state = this._writableState;
                if (state.corked > 0) state.corked--;
                if (state.corked === 0 && state.buffered.length > 0) {
                    var buf = state.buffered.splice(0);
                    var self = this;
                    buf.forEach(function(item) {
                        self._transform(item.chunk, item.encoding || 'utf8', function(err, data) {
                            if (err) { self.emit('error', err); if (item.cb) item.cb(err); return; }
                            if (data != null) self.push(data);
                            if (item.cb) item.cb(null);
                        });
                    });
                }
            };

            // ── PassThrough ───────────────────────────────────────────────────────
            function PassThrough(opts) { Transform.call(this, opts); }
            util.inherits(PassThrough, Transform);
            PassThrough.prototype._transform = function(chunk, enc, cb) { this.push(chunk); cb(); };

            // ── Duplex ────────────────────────────────────────────────────────────
            function Duplex(opts) {
                var self = this;
                Readable.call(self, opts);
                self.writable = true;
                self._writableState = {
                    objectMode: !!(opts && opts.objectMode),
                    highWaterMark: (opts && opts.highWaterMark) || 16384,
                    length: 0, corked: 0, ended: false, finished: false,
                    needDrain: false, pendingcb: 0, buffered: [],
                    destroying: false,
                };
                Object.defineProperties(self, {
                    writableLength: { get: function() { return self._writableState.length; } },
                    writableEnded: { get: function() { return self._writableState.ended; } },
                    writableFinished: { get: function() { return self._writableState.finished; } },
                    writableCorked: { get: function() { return self._writableState.corked; } },
                    writableHighWaterMark: { get: function() { return self._writableState.highWaterMark; } },
                });
                if (opts && typeof opts.write === 'function') self._write = opts.write;
                if (opts && typeof opts.final === 'function') self._final = opts.final;
                if (opts && typeof opts.destroy === 'function') self._destroy = opts.destroy;
            }
            util.inherits(Duplex, Readable);
            // Writable side — delegate to Writable.prototype for shared implementation
            Duplex.prototype._write = Writable.prototype._write;
            Duplex.prototype._final = Writable.prototype._final;
            Duplex.prototype.write = Writable.prototype.write;
            Duplex.prototype.end = Writable.prototype.end;
            Duplex.prototype.cork = Writable.prototype.cork;
            Duplex.prototype.uncork = Writable.prototype.uncork;
            Duplex.prototype.setDefaultEncoding = Writable.prototype.setDefaultEncoding;
            Duplex.prototype._destroy = function(err, callback) {
                if (err) this.emit('error', err);
                this.emit('close');
                if (typeof callback === 'function') callback(err);
            };
            Duplex.prototype.destroy = function(err) {
                var state = this._writableState;
                if (state && state.destroying) return this;
                if (state) state.destroying = true;
                var self = this;
                if (err) self.emit('error', err);
                // End readable side
                self._readableState.ended = true;
                // Call _destroy
                self._destroy(err, function() {
                    self.emit('close');
                });
                return this;
            };

            // In Node.js, require('stream') IS the Stream constructor function,
            // with Readable/Writable/etc. attached as properties (not a plain object).
            Stream.Stream = Stream;
            Stream.Readable = Readable;
            Stream.Writable = Writable;
            Stream.Transform = Transform;
            Stream.PassThrough = PassThrough;
            Stream.Duplex = Duplex;
            Stream.isReadable = function(s) { return !!(s && s.readable); };
            Stream.isWritable = function(s) { return !!(s && s.writable); };
            Stream.isStream = function(s) { return !!(s && (s.readable || s.writable)); };
            var stream = Stream;
            globalThis.__requireCache['stream'] = stream;
            globalThis.__requireCache['node:stream'] = stream;
            globalThis.__requireCache['stream/web'] = stream;
            globalThis.__requireCache['readable-stream'] = stream;

            // ── path ──────────────────────────────────────────────────────────────
            function makePath(sep, isAbsFn) {
                function normalize(p) {
                    var abs = isAbsFn(p);
                    var hasTrailing = p.length > 1 && p[p.length-1] === sep;
                    var parts = p.split(sep).filter(function(s,i) { return s !== '' || i === 0; });
                    var out = [];
                    for (var i = 0; i < parts.length; i++) {
                        if (parts[i] === '..') { if (out.length > 0 && out[out.length-1] !== '..') out.pop(); else if (!abs) out.push('..'); }
                        else if (parts[i] !== '.') out.push(parts[i]);
                    }
                    var r = out.join(sep);
                    if (abs && r[0] !== sep) r = sep + r;
                    if (hasTrailing && r.length > 1 && r[r.length-1] !== sep) r += sep;
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
                        var resolvedParts = [];
                        for (var i = args.length - 1; i >= -1; i--) {
                            var p = i >= 0 ? String(args[i]) : ((typeof process !== 'undefined' && process.cwd) ? process.cwd() : '/');
                            if (!p) continue;
                            resolvedParts.push(p);
                            if (isAbsFn(p)) break;
                        }
                        resolvedParts.reverse();
                        return normalize(resolvedParts.join(sep));
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
                    if (typeof __osCpusInfo === 'function') {
                        try { return JSON.parse(__osCpusInfo()); } catch(e) {}
                    }
                    var n = __osCpuCount();
                    var arr = [];
                    for (var i = 0; i < n; i++) arr.push({ model: 'Unknown', speed: 0, times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 } });
                    return arr;
                },
                totalmem: function() { return typeof __osMemTotal === 'function' ? __osMemTotal() : 1073741824; },
                freemem:  function() { return typeof __osMemFree  === 'function' ? __osMemFree()  :  536870912; },
                networkInterfaces: function() {
                    if (typeof __osNetworkInterfaces === 'function') {
                        try { return JSON.parse(__osNetworkInterfaces()); } catch(e) {}
                    }
                    return {};
                },
                userInfo: function(opts) {
                    var u = (process && process.env && process.env.USER) || 'user';
                    var h = (process && process.env && process.env.HOME) || '/home/' + u;
                    return { username: u, uid: -1, gid: -1, shell: '/bin/sh', homedir: h };
                },
                release: function() { return typeof __osRelease === 'function' ? __osRelease() : '6.0.0'; },
                version: function() { return typeof __osVersion === 'function' ? __osVersion() : ''; },
                uptime: function() { return typeof __osUptime === 'function' ? __osUptime() : 0; },
                loadavg: function() {
                    return typeof __osLoadAvg === 'function' ? __osLoadAvg() : [0, 0, 0];
                },
                endianness: function() { return 'LE'; },
                availableParallelism: function() { return __osCpuCount(); },
                constants: {
                    signals: { SIGHUP: 1, SIGINT: 2, SIGTERM: 15, SIGKILL: 9, SIGPIPE: 13, SIGCHLD: 17, SIGUSR1: 10, SIGUSR2: 12 },
                    errno: { ENOENT: -2, EACCES: -13, EEXIST: -17, EISDIR: -21, ENOTDIR: -20, ENOTEMPTY: -39, EPERM: -1 },
                    priority: { PRIORITY_LOW: 19, PRIORITY_BELOW_NORMAL: 10, PRIORITY_NORMAL: 0, PRIORITY_ABOVE_NORMAL: -7, PRIORITY_HIGH: -14, PRIORITY_HIGHEST: -20 },
                    dlopen: { RTLD_LAZY: 1, RTLD_NOW: 2, RTLD_GLOBAL: 8, RTLD_LOCAL: 4, RTLD_DEEPBIND: 8 }
                },
                getPriority: function() { return 0; },
                setPriority: function() {},
                machine: function() { return _osArch; },
                devNull: '/dev/null',
            };
            globalThis.__requireCache['os'] = os;
            globalThis.__requireCache['node:os'] = os;

            // ── url ───────────────────────────────────────────────────────────────
            var url = {
                parse: function(s, parseQueryString) {
                    var result;
                    try {
                        var u = new URL(s);
                        result = { protocol: u.protocol, slashes: true, auth: null, host: u.host, port: u.port || null, hostname: u.hostname, hash: u.hash || null, search: u.search || null, pathname: u.pathname, path: u.pathname + (u.search || ''), href: u.href };
                    } catch(e) {
                        // Relative or path-only URL (e.g. '/hello?a=1#x')
                        var href = s || '/';
                        var hash = null, search = null, pathname = href, path = href;
                        var hi = href.indexOf('#');
                        if (hi !== -1) { hash = href.slice(hi); href = href.slice(0, hi); pathname = href; path = href; }
                        var qi = href.indexOf('?');
                        if (qi !== -1) { search = href.slice(qi); pathname = href.slice(0, qi); path = href; }
                        result = { protocol: null, slashes: null, auth: null, host: null, port: null, hostname: null, hash: hash, search: search, pathname: pathname, path: path, href: s || '/' };
                    }
                    if (parseQueryString) {
                        var qs = result.search ? result.search.slice(1) : '';
                        var q = {};
                        if (qs) qs.split('&').forEach(function(p) { var kv = p.split('='); if (kv[0]) q[decodeURIComponent(kv[0])] = kv[1] !== undefined ? decodeURIComponent(kv[1]) : ''; });
                        result.query = q;
                    } else {
                        result.query = result.search ? result.search.slice(1) : null;
                    }
                    return result;
                },
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
                                // Decode binary responses: server base64-encoded them and set binary:true
                                var bodyData = d.binary ? Buffer.from(d.body, 'base64') : d.body;
                                var res = {
                                    statusCode: d.status,
                                    statusMessage: d.statusText,
                                    headers: d.headers,
                                    _body: bodyData,
                                    _listeners: { data: [], end: [], error: [] },
                                    on: function(ev, fn) { this._listeners[ev] = this._listeners[ev] || []; this._listeners[ev].push(fn); return this; },
                                    pipe: function(dest) { if (dest && dest.write) dest.write(bodyData); if (dest && dest.end) dest.end(); return dest; },
                                    resume: function() {}
                                };
                                // deliver body
                                if (cb) cb(res);
                                listeners.response.forEach(function(fn) { fn(res); });
                                setTimeout(function() {
                                    res._listeners.data.forEach(function(fn) { fn(bodyData); });
                                    res._listeners.end.forEach(function(fn) { fn(); });
                                    listeners.data.forEach(function(fn) { fn(bodyData); });
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

                // ── IncomingMessage ─────────────────────────────────────────
                function IncomingMessage(socket) {
                    Readable.call(this);
                    this.socket = socket || null;
                    this.headers = {};
                    this.rawHeaders = [];
                    this.method = 'GET';
                    this.url = '/';
                    this.httpVersion = '1.1';
                    this.statusCode = null;
                    this.statusMessage = null;
                    this._body = '';
                    this._consumed = false;
                }
                util.inherits(IncomingMessage, Readable);
                IncomingMessage.prototype._read = function(size) {
                    if (!this._consumed && this._body !== '') {
                        this._consumed = true;
                        this.push(this._body);
                    }
                    if (this._consumed || this._body === '') {
                        this.push(null);
                    }
                };
                IncomingMessage.prototype.setEncoding = function(enc) { this._encoding = enc; return this; };
                IncomingMessage.prototype.destroy = function(err) { if (err) this.emit('error', err); this.emit('close'); return this; };

                // ── ServerResponse ─────────────────────────────────────────
                function ServerResponse(req) {
                    Writable.call(this);
                    this.req = req || null;
                    this.statusCode = 200;
                    this.statusMessage = '';
                    this._headers = {};
                    this._chunks = [];
                    this.writableEnded = false;
                    this._responded = false;
                    this._connId = null;
                    this.headersSent = false;
                }
                util.inherits(ServerResponse, Writable);
                ServerResponse.prototype._write = function(chunk, encoding, callback) {
                    if (chunk !== undefined && chunk !== null) {
                        if (typeof chunk === 'string') {
                            this._chunks.push(Buffer.from(chunk, encoding || 'utf8'));
                        } else if (chunk instanceof Uint8Array || Buffer.isBuffer(chunk)) {
                            this._chunks.push(Buffer.from(chunk));
                        } else {
                            this._chunks.push(Buffer.from(String(chunk)));
                        }
                    }
                    callback();
                };
                ServerResponse.prototype._final = function(callback) {
                    this._sendResponse();
                    callback();
                };
                ServerResponse.prototype._sendResponse = function() {
                    if (this._responded || this._connId == null) return;
                    this._responded = true;
                    this.headersSent = true;
                    var st = this.statusCode || 200;
                    var stText = this.statusMessage || STATUS_CODES[st] || 'OK';
                    if (!this._headers['Content-Type'] && !this._headers['content-type']) {
                        this._headers['Content-Type'] = 'text/plain';
                    }
                    try {
                        var _bodyBuf = Buffer.concat(this._chunks.length ? this._chunks : [Buffer.alloc(0)]);
                        __httpRespondBytes(this._connId, st, stText, JSON.stringify(this._headers), Array.from(_bodyBuf));
                    }
                    catch(e) { /* connection may have been closed */ }
                };
                ServerResponse.prototype.setHeader = function(k, v) { this._headers[k] = v; return this; };
                ServerResponse.prototype.getHeader = function(k) { return this._headers[k]; };
                ServerResponse.prototype.removeHeader = function(k) { delete this._headers[k]; return this; };
                ServerResponse.prototype.hasHeader = function(k) { return k in this._headers; };
                ServerResponse.prototype.writeHead = function(status, msg, headers) {
                    if (typeof msg === 'object') { headers = msg; msg = ''; }
                    this.statusCode = status;
                    if (msg) this.statusMessage = msg;
                    if (headers) {
                        var self = this;
                        Object.keys(headers).forEach(function(k) { self._headers[k] = headers[k]; });
                    }
                    return this;
                };
                ServerResponse.prototype.end = function(chunk, encoding, cb) {
                    if (typeof chunk === 'function') { cb = chunk; chunk = null; }
                    if (typeof encoding === 'function') { cb = encoding; encoding = null; }
                    var self = this;
                    function finish() {
                        self.writableEnded = true;
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
                    return this;
                };
                ServerResponse.prototype.destroy = function(err) {
                    if (err) this.emit('error', err);
                    this._responded = true;
                    this.emit('close');
                    return this;
                };

                // ── Cross-platform absolute-path detection ──────────────────
                // path.isAbsolute() is POSIX-only and fails for Windows
                // paths like C:\... when running on Windows.
                function _isAbsPath(p) {
                    p = String(p || '');
                    return p.charAt(0) === '/' || p.charAt(0) === '\\' || /^[A-Za-z]:/.test(p);
                }

                // ── MIME lookup (used by sendFile / download) ───────────────
                function _getMime(p) {
                    var ext = (p || '').split('.').pop().toLowerCase();
                    return ({
                        html:'text/html; charset=utf-8', htm:'text/html; charset=utf-8',
                        css:'text/css', js:'application/javascript; charset=utf-8',
                        mjs:'application/javascript; charset=utf-8',
                        cjs:'application/javascript; charset=utf-8',
                        json:'application/json; charset=utf-8',
                        jsonld:'application/ld+json', xml:'application/xml',
                        txt:'text/plain; charset=utf-8', csv:'text/csv',
                        md:'text/markdown', yaml:'text/yaml', yml:'text/yaml',
                        png:'image/png', jpg:'image/jpeg', jpeg:'image/jpeg',
                        gif:'image/gif', svg:'image/svg+xml', ico:'image/x-icon',
                        webp:'image/webp', avif:'image/avif', bmp:'image/bmp',
                        tiff:'image/tiff', tif:'image/tiff',
                        mp3:'audio/mpeg', ogg:'audio/ogg', wav:'audio/wav',
                        flac:'audio/flac', aac:'audio/aac', weba:'audio/webm',
                        mp4:'video/mp4', webm:'video/webm', avi:'video/x-msvideo',
                        mov:'video/quicktime', mkv:'video/x-matroska',
                        woff:'font/woff', woff2:'font/woff2',
                        ttf:'font/ttf', otf:'font/otf',
                        eot:'application/vnd.ms-fontobject',
                        zip:'application/zip', tar:'application/x-tar',
                        gz:'application/gzip', pdf:'application/pdf',
                        wasm:'application/wasm', map:'application/json',
                        webmanifest:'application/manifest+json',
                    })[ext] || 'application/octet-stream';
                }

                // ── res.sendFile(path, [opts], [cb]) ────────────────────────
                ServerResponse.prototype.sendFile = function(filePath, opts, cb) {
                    if (typeof opts === 'function') { cb = opts; opts = {}; }
                    opts = opts || {};
                    var self = this;
                    var path = require('path');
                    var fs = require('fs');
                    var absPath = (opts.root && !_isAbsPath(filePath))
                        ? path.join(opts.root, filePath)
                        : (_isAbsPath(filePath) ? filePath : path.resolve(filePath));
                    fs.stat(absPath, function(err, stat) {
                        if (err) {
                            err.status = 404;
                            if (cb) return cb(err);
                            self.statusCode = 404;
                            self.setHeader('Content-Type', 'text/plain');
                            return self.end('Not Found');
                        }
                        if (!stat.isFile()) {
                            var e = new Error('Not a file: ' + filePath); e.status = 404;
                            if (cb) return cb(e);
                            self.statusCode = 404;
                            self.setHeader('Content-Type', 'text/plain');
                            return self.end('Not Found');
                        }
                        if (!self._headers['Content-Type'] && !self._headers['content-type']) {
                            self.setHeader('Content-Type', _getMime(absPath));
                        }
                        self.setHeader('Content-Length', String(stat.size));
                        if (self.statusCode === 200 || !self.statusCode) self.statusCode = 200;
                        var stream = fs.createReadStream(absPath);
                        stream.on('error', function(e) {
                            if (cb) { cb(e); return; }
                            if (!self._responded) {
                                self.statusCode = 500;
                                self.setHeader('Content-Type', 'text/plain');
                                self.end('Error reading file: ' + (e && e.message || String(e)));
                            }
                        });
                        stream.on('end', function() { if (cb) cb(null); });
                        stream.pipe(self);
                    });
                };

                // ── res.download(path, [filename], [opts], [cb]) ────────────
                ServerResponse.prototype.download = function(filePath, filename, opts, cb) {
                    if (typeof filename === 'function') { cb = filename; filename = null; opts = {}; }
                    else if (typeof opts === 'function') { cb = opts; opts = {}; }
                    var path = require('path');
                    filename = filename || path.basename(String(filePath));
                    this.setHeader('Content-Disposition',
                        'attachment; filename="' + String(filename).replace(/\\/g,'\\\\').replace(/"/g,'\\"') + '"');
                    this.sendFile(filePath, opts || {}, cb);
                };

                // ── res.render(view, [locals], [cb]) ────────────────────────
                // Falls back to simple {{var}} substitution when no engine is registered.
                ServerResponse.prototype.render = function(view, locals, cb) {
                    if (typeof locals === 'function') { cb = locals; locals = {}; }
                    locals = locals || {};
                    var self = this;
                    var path = require('path');
                    var fs = require('fs');
                    var app = self.req && self.req.app;
                    var viewsDir = app && typeof app.get === 'function' ? app.get('views') : null;
                    var viewEngine = app && typeof app.get === 'function' ? app.get('view engine') : null;
                    var ext = path.extname(view) || (viewEngine ? '.' + viewEngine : '.html');
                    var viewPath = _isAbsPath(view) ? view
                        : (viewsDir
                            ? path.join(viewsDir, view + (path.extname(view) ? '' : ext))
                            : path.resolve(view + (path.extname(view) ? '' : ext)));
                    var engineFn = app && app.engines ? app.engines[ext.replace('.', '')] : null;
                    if (!engineFn && app && app._engines) engineFn = app._engines[ext.replace('.', '')];
                    if (engineFn) {
                        try {
                            engineFn(viewPath, locals, function(err, html) {
                                if (err) {
                                    if (cb) return cb(err);
                                    self.statusCode = 500;
                                    return self.end('Error: ' + err.message);
                                }
                                self.setHeader('Content-Type', 'text/html; charset=utf-8');
                                if (cb) cb(null, html); else self.end(html);
                            });
                        } catch(e) { if (cb) cb(e); else self.end('Error: ' + e.message); }
                    } else {
                        fs.readFile(viewPath, 'utf8', function(err, template) {
                            if (err) {
                                if (cb) return cb(err);
                                self.statusCode = 500;
                                return self.end('Template not found: ' + view);
                            }
                            var html = template.replace(/\{\{([\w.]+)\}\}/g, function(_, key) {
                                var val = locals;
                                key.split('.').forEach(function(k) { val = val && val[k]; });
                                return val != null ? String(val) : '';
                            });
                            self.setHeader('Content-Type', 'text/html; charset=utf-8');
                            if (cb) cb(null, html); else self.end(html);
                        });
                    }
                };

                var modObj = {
                    METHODS: ['ACL','BIND','CHECKOUT','CONNECT','COPY','DELETE','GET','HEAD',
                              'LINK','LOCK','M-SEARCH','MERGE','MKACTIVITY','MKCALENDAR','MKCOL',
                              'MOVE','NOTIFY','OPTIONS','PATCH','POST','PROPFIND','PROPPATCH',
                              'PURGE','PUT','REBIND','REPORT','SEARCH','SOURCE','SUBSCRIBE',
                              'TRACE','UNBIND','UNLINK','UNLOCK','UNSUBSCRIBE'],
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
                                port = parseInt(port, 10) || 0;
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
                            removeListener: function(ev, fn) { return server.off(ev, fn); },
                            removeAllListeners: function(ev) {
                                if (ev !== undefined) { server._listeners[ev] = []; }
                                else { server._listeners = {}; }
                                return server;
                            },
                            listeners: function(ev) {
                                return (server._listeners[ev] || []).slice();
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
                            var socket = new Socket({});
                            socket._id = connId;
                            socket.pending = false;
                            socket.connecting = false;
                            socket._connected = true;
                            socket.server = server;
                            socket.remoteAddress = reqData.remoteAddress || '127.0.0.1';
                            socket.remotePort = reqData.remotePort || 0;
                            socket.localAddress = reqData.localAddress || '0.0.0.0';
                            socket.localPort = reqData.port || 0;

                            var req = new IncomingMessage(socket);
                            req.method = reqData.method;
                            req.url = reqData.url;
                            req.headers = reqData.headers || {};
                            req.rawHeaders = (function() {
                                var arr = [];
                                var h = reqData.headers || {};
                                Object.keys(h).forEach(function(k) { arr.push(k, h[k]); });
                                return arr;
                            })();
                            req._body = reqData.body || '';
                            req.socket = socket;

                            var res = new ServerResponse(req);
                            res._connId = connId;

                            try { handler(req, res); } catch(e) {
                                if (!res._responded) {
                                    try { __httpRespond(connId, 500, 'Internal Server Error', JSON.stringify({'Content-Type':'text/plain'}), 'Internal Server Error'); } catch(_) {}
                                    res._responded = true;
                                }
                            }
                        }

                        return server;
                    }
                };

                // ── Export classes on http module ──────────────────────────
                modObj.IncomingMessage = IncomingMessage;
                modObj.ServerResponse = ServerResponse;

                return modObj;
            }

            var httpMod = makeHttpModule('http:');
            var httpsMod = makeHttpModule('https:');
            var _globalAgent = { maxSockets: Infinity, maxFreeSockets: 256, keepAlive: false };
            httpMod.globalAgent = _globalAgent;
            httpsMod.globalAgent = _globalAgent;
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
                Duplex.call(this, opts);
                opts = opts || {};
                this._id = null;
                this._tls = !!(opts.tls);
                this._connected = false;
                this._destroyed = false;
                this._encoding = null;
                this._pollTimer = null;
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
            util.inherits(Socket, Duplex);

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
                            : (typeof Buffer !== 'undefined' ? Buffer.from(chunk) : new Uint8Array(chunk));
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
                            port = parseInt(port, 10) || 0;
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
                        removeListener: function(ev, fn) { return server.off(ev, fn); },
                        removeAllListeners: function(ev) { if (ev !== undefined) { server._listeners[ev] = []; } else { server._listeners = {}; } return server; },
                        listeners: function(ev) { return (server._listeners[ev] || []).slice(); },
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
                    // TLS termination is not supported server-side; delegate to plain HTTP
                    // server so apps guarded with tls.createServer() run in dev/test.
                    return httpMod.createServer(cb || function() {});
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
            var __perfOrigin = Date.now();
            var __perfObj = {
              timeOrigin: __perfOrigin,
              now: function() { return Date.now() - __perfOrigin; },
              mark: function() {},
              measure: function() {},
              getEntriesByName: function() { return []; },
              getEntriesByType: function() { return []; },
              clearMarks: function() {},
              clearMeasures: function() {},
            };
            globalThis.performance = __perfObj;
            globalThis.__requireCache['perf_hooks'] = { performance: __perfObj, PerformanceObserver: function() {} };
            globalThis.__requireCache['node:perf_hooks'] = globalThis.__requireCache['perf_hooks'];

            // ── WeakRef polyfill (QuickJS lacks native WeakRef / FinalizationRegistry) ──
            // Strong-reference fallback: memory isn't reclaimed early but semantics work.
            if (typeof globalThis.WeakRef === 'undefined') {
                globalThis.WeakRef = function WeakRef(target) { this._t = target; };
                globalThis.WeakRef.prototype.deref = function() { return this._t; };
            }
            if (typeof globalThis.FinalizationRegistry === 'undefined') {
                globalThis.FinalizationRegistry = function FinalizationRegistry(cb) { this._cb = cb; };
                globalThis.FinalizationRegistry.prototype.register = function() {};
                globalThis.FinalizationRegistry.prototype.unregister = function() {};
            }

            // ── agent-base / https-proxy-agent (proxy stubs, not supported in sandbox) ──
            // Stored in __fallbackModules so real node_modules modules are preferred.
            function AgentBase() {}
            AgentBase.prototype.addRequest = function() {};
            globalThis.__fallbackModules['agent-base'] = { Agent: AgentBase };
            var HttpsProxyAgentStub = function(opts) { AgentBase.call(this); this.proxy = opts; };
            util.inherits(HttpsProxyAgentStub, AgentBase);
            HttpsProxyAgentStub.prototype.callback = function(req, opts) { return opts; };
            globalThis.__fallbackModules['https-proxy-agent'] = { HttpsProxyAgent: HttpsProxyAgentStub };

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

            function _makeHttp2Server(handler) {
                var serverId = null;
                var _listeners = {};

                var server = {
                    listening: false,
                    _host: '0.0.0.0',
                    _port: 0,
                    address: function() {
                        return serverId !== null ? { address: server._host, port: server._port, family: 'IPv4' } : null;
                    },
                    on: function(ev, fn) {
                        _listeners[ev] = _listeners[ev] || [];
                        _listeners[ev].push(fn);
                        return server;
                    },
                    once: function(ev, fn) {
                        function w() { fn.apply(this, arguments); server.off(ev, w); }
                        return server.on(ev, w);
                    },
                    off: function(ev, fn) {
                        if (_listeners[ev]) _listeners[ev] = _listeners[ev].filter(function(f) { return f !== fn; });
                        return server;
                    },
                    emit: function(ev) {
                        var args = Array.prototype.slice.call(arguments, 1);
                        (_listeners[ev] || []).slice().forEach(function(fn) { fn.apply(server, args); });
                    },
                    listen: function(port, host, cb) {
                        if (typeof port === 'object' && port !== null) {
                            var o = port; port = o.port || 0; host = o.host || '0.0.0.0'; cb = host;
                        }
                        if (typeof host === 'function') { cb = host; host = '0.0.0.0'; }
                        if (typeof port === 'function') { cb = port; port = 0; host = '0.0.0.0'; }
                        host = host || '0.0.0.0';
                        port = parseInt(port, 10) || 0;
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
                };

                if (handler) server.on('request', handler);

                function _acceptNext() {
                    if (serverId === null) return;
                    __httpAcceptAsync(serverId).then(function(reqJson) {
                        _handleRequest(JSON.parse(reqJson));
                        _acceptNext();
                    }).catch(function(e) {
                        if (serverId !== null) { server.emit('error', e); _acceptNext(); }
                    });
                }

                function _handleRequest(reqData) {
                    var connId = reqData.conn_id;

                    // Http2ServerStream — shared object for both stream and compat APIs.
                    function Http2ServerStream() {
                        EventEmitter.call(this);
                        this._connId = connId;
                        this._statusCode = 200;
                        this._resHeaders = {};
                        this._chunks = [];
                        this._responded = false;
                    }
                    util.inherits(Http2ServerStream, EventEmitter);

                    Http2ServerStream.prototype.respond = function(headers, opts) {
                        var h = headers || {};
                        this._statusCode = h[':status'] || 200;
                        var self = this;
                        Object.keys(h).forEach(function(k) {
                            if (k.charAt(0) !== ':') self._resHeaders[k] = h[k];
                        });
                        if (opts && opts.endStream) this._flush('');
                        return this;
                    };
                    Http2ServerStream.prototype.setHeader = function(k, v) { this._resHeaders[k] = v; };
                    Http2ServerStream.prototype.getHeader = function(k) { return this._resHeaders[k]; };
                    Http2ServerStream.prototype.write = function(chunk) {
                        this._chunks.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString());
                        return true;
                    };
                    Http2ServerStream.prototype.end = function(chunk) {
                        if (chunk !== undefined && chunk !== null && chunk !== '') this.write(chunk);
                        this._flush(this._chunks.join(''));
                    };
                    Http2ServerStream.prototype._flush = function(body) {
                        if (this._responded) return;
                        this._responded = true;
                        var st = this._statusCode;
                        try {
                            __httpRespond(this._connId, st, STATUS_CODES[st] || 'OK',
                                JSON.stringify(this._resHeaders), body);
                        } catch(_) {}
                        this.emit('close');
                    };
                    Http2ServerStream.prototype.destroy = function() { this._flush(''); };

                    var stream = new Http2ServerStream();

                    // Pseudo-headers exposed as the `headers` argument in the stream event.
                    var reqHeaders = Object.assign(
                        { ':method': reqData.method, ':path': reqData.url,
                          ':scheme': 'https',
                          ':authority': (reqData.headers || {}).host || server._host + ':' + server._port },
                        reqData.headers || {}
                    );

                    // Deliver request body asynchronously.
                    setTimeout(function() {
                        if (reqData.body) stream.emit('data', reqData.body);
                        stream.emit('end');
                    }, 0);

                    var hasRequestListeners = (_listeners['request'] || []).length > 0;
                    var hasStreamListeners = (_listeners['stream'] || []).length > 0;

                    // Stream API (primary http2 interface).
                    if (hasStreamListeners) {
                        server.emit('stream', stream, reqHeaders);
                    }

                    // Compat request/response API.
                    if (hasRequestListeners) {
                        var req = new EventEmitter();
                        req.headers = reqHeaders;
                        req.method = reqData.method;
                        req.url = reqData.url;
                        req.httpVersion = '2.0';
                        req.stream = stream;
                        setTimeout(function() {
                            if (reqData.body) req.emit('data', reqData.body);
                            req.emit('end');
                        }, 0);

                        var res = new EventEmitter();
                        res.statusCode = 200;
                        res.headersSent = false;
                        res.stream = stream;
                        res.setHeader = function(k, v) { stream.setHeader(k, v); };
                        res.getHeader = function(k) { return stream.getHeader(k); };
                        res.writeHead = function(status, msg, headers) {
                            stream._statusCode = status;
                            if (typeof msg === 'object' && msg) headers = msg;
                            if (headers) Object.assign(stream._resHeaders, headers);
                        };
                        res.write = function(chunk) { return stream.write(chunk); };
                        res.end = function(chunk) { stream.end(chunk); };

                        server.emit('request', req, res);
                    }

                    // If neither handler is registered, fall back to 404 after a tick.
                    if (!hasRequestListeners && !hasStreamListeners) {
                        setTimeout(function() {
                            try { __httpRespond(connId, 501, 'Not Implemented',
                                JSON.stringify({'content-type':'text/plain'}), 'http2 server: no handler registered'); } catch(_) {}
                        }, 0);
                    }
                }

                return server;
            }

            var http2Mod = {
                connect: function(authority, opts, cb) {
                    if (typeof opts === 'function') { cb = opts; opts = {}; }
                    var session = new Http2Session(authority);
                    if (cb) session.once('connect', cb);
                    setTimeout(function() { session.emit('connect', session, {}); }, 0);
                    return session;
                },
                createServer: function(opts, handler) {
                    if (typeof opts === 'function') { handler = opts; opts = {}; }
                    return _makeHttp2Server(handler);
                },
                createSecureServer: function(opts, handler) {
                    if (typeof opts === 'function') { handler = opts; opts = {}; }
                    return _makeHttp2Server(handler);
                },
                constants: HTTP2_CONSTANTS,
                sensitiveHeaders: Symbol('sensitiveHeaders'),
            };

            globalThis.__requireCache['http2'] = http2Mod;
            globalThis.__requireCache['node:http2'] = http2Mod;

            // ── cluster — single-primary emulation ────────────────────────────────
            // 3va is single-process: clustering is emulated so that apps guarded by
            // `if (cluster.isPrimary)` work as single-instance servers. fork() returns
            // a mock Worker that fires 'online' asynchronously, allowing startup code
            // that iterates over CPU cores to complete without crashing.
            (function() {
                function ClusterWorker(id) {
                    EventEmitter.call(this);
                    this.id = id;
                    this.process = { pid: 0 };
                    this.exitedAfterDisconnect = false;
                    this.suicide = false; // legacy alias
                    var self = this;
                    setTimeout(function() {
                        self.emit('online');
                        clusterMod.emit('online', self);
                    }, 0);
                }
                util.inherits(ClusterWorker, EventEmitter);
                ClusterWorker.prototype.send = function(msg, cb) { if (cb) cb(null); return true; };
                ClusterWorker.prototype.isDead = function() { return false; };
                ClusterWorker.prototype.isConnected = function() { return true; };
                ClusterWorker.prototype.kill = function(signal) {
                    signal = signal || 'SIGTERM';
                    delete clusterMod.workers[this.id];
                    this.emit('exit', 0, signal);
                    clusterMod.emit('exit', this, 0, signal);
                };
                ClusterWorker.prototype.disconnect = function() { this.kill('disconnect'); };

                var _nextId = 1;
                var clusterMod = new EventEmitter();
                clusterMod.isMaster  = true; // legacy Node.js name
                clusterMod.isPrimary = true;
                clusterMod.isWorker  = false;
                clusterMod.workers   = {};
                clusterMod.settings  = {};
                clusterMod.schedulingPolicy = 2; // SCHED_RR
                clusterMod.SCHED_NONE = 1;
                clusterMod.SCHED_RR   = 2;

                clusterMod.fork = function(env) {
                    var w = new ClusterWorker(_nextId++);
                    clusterMod.workers[w.id] = w;
                    clusterMod.emit('fork', w);
                    return w;
                };
                clusterMod.setupMaster  = function(s) { Object.assign(clusterMod.settings, s || {}); };
                clusterMod.setupPrimary = clusterMod.setupMaster;
                clusterMod.disconnect   = function(cb) { if (cb) setTimeout(cb, 0); };

                globalThis.__requireCache['cluster']      = clusterMod;
                globalThis.__requireCache['node:cluster'] = clusterMod;
            }());

            // ── debug / ms (common logging utility) ────────────────────────────
            // ── uuid ─────────────────────────────────────────────────────────────
            // uuid v14 is ESM-only; older versions lack v4 or are CJS.
            // Fallback uses crypto.randomUUID() (available in 3va runtime).
            globalThis.__fallbackModules['uuid'] = {
                v4: function() {
                    if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
                        return crypto.randomUUID();
                    }
                    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
                        var r = Math.random() * 16 | 0;
                        return (c === 'x' ? r : (r & 0x3 | 0x8)).toString(16);
                    });
                },
                v1: function() { throw new Error('uuid.v1 not supported in 3va'); },
                v3: function() { throw new Error('uuid.v3 not supported in 3va'); },
                v5: function() { throw new Error('uuid.v5 not supported in 3va'); },
                NIL: '00000000-0000-0000-0000-000000000000',
                validate: function(s) { return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s); },
                version: function(s) { return parseInt(s[14], 16); },
                parse: function(s) { return s.replace(/-/g,'').match(/.{2}/g).map(function(b){return parseInt(b,16);}); },
                stringify: function(arr) { var h='0123456789abcdef',r=''; for(var i=0;i<16;i++){r+=h[arr[i]>>4]+h[arr[i]&15];if(i===3||i===5||i===7||i===9)r+='-';}return r; },
            };

            // ── whatwg-url ────────────────────────────────────────────────────────
            // whatwg-url v16 depends on @exodus/bytes (ESM-only). Use native URL.
            (function() {
                var _URL = globalThis.URL;
                var _USP = globalThis.URLSearchParams;
                globalThis.__fallbackModules['whatwg-url'] = {
                    URL: _URL,
                    URLSearchParams: _USP,
                    parseURL: function(u, base) { try { return new _URL(u, base); } catch(e) { return null; } },
                    basicURLParse: function(u, base) { try { return new _URL(u, base); } catch(e) { return null; } },
                    serializeURL: function(u) { return u ? u.href : ''; },
                    serializeURLOrigin: function(u) { return u ? u.origin : 'null'; },
                    setTheUsername: function(u, v) { u.username = v; },
                    setThePassword: function(u, v) { u.password = v; },
                    cannotHaveAUsernamePasswordPort: function(u) { return !u || u.host === '' || u.scheme === 'file'; },
                };
            })();

            // Stored in __fallbackModules so the real module from node_modules is
            // preferred. The stub lacks .default, .debug, etc., which breaks
            // socket.io / engine.io that rely on them.
            globalThis.__fallbackModules['debug'] = function(ns) { return function() {}; };
            globalThis.__fallbackModules['ms'] = function(val) { return typeof val === 'string' ? 0 : val; };

            // ── proxy-from-env ───────────────────────────────────────────────────
            globalThis.__fallbackModules['proxy-from-env'] = { getProxyForUrl: function() { return null; } };

            // ── tr46 (Unicode IDNA) ───────────────────────────────────────────────
            // QuickJS cannot parse tr46's large supplementary-plane Unicode regexes.
            // This stub handles ASCII hostnames correctly; IDN (non-ASCII) domains
            // are passed through unchanged — sufficient for localhost / IP connections.
            (function() {
              function toASCII(domain, options) {
                if (typeof domain !== 'string') return null;
                var lower = domain.toLowerCase();
                // Reject labels with leading/trailing hyphens or double-hyphen in pos 3-4
                var labels = lower.split('.');
                for (var i = 0; i < labels.length; i++) {
                  var lbl = labels[i];
                  if (lbl.length === 0) return null;
                  if (lbl[0] === '-' || lbl[lbl.length - 1] === '-') return null;
                }
                return lower;
              }
              function toUnicode(domain, options) {
                return { domain: domain, error: false };
              }
              globalThis.__fallbackModules['tr46'] = { toASCII: toASCII, toUnicode: toUnicode };
            })();

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

        // navigator — minimal WinterCG-compatible object (not frozen so stubs can extend it)
        if (!globalThis.navigator) {
            globalThis.navigator = {
                userAgent: '3va/0.1 (QuickJS)',
                language: 'en-US',
                languages: Object.freeze(['en-US', 'en']),
                onLine: true,
                hardwareConcurrency: 1,
                platform: 'Linux x86_64',
                cookieEnabled: false,
                doNotTrack: '1',
            };
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

    // Dynamic import() polyfill: captures the calling module's __dirname so
    // relative specifiers resolve correctly even when called asynchronously.
    ctx.eval::<(), _>(
        r#"
        globalThis.__importAsync = function(specifier) {
            var dir = globalThis.__dirname;
            return new Promise(function(resolve, reject) {
                try {
                    var saved = globalThis.__dirname;
                    globalThis.__dirname = dir;
                    var mod;
                    try { mod = globalThis.require(specifier); }
                    finally { globalThis.__dirname = saved; }
                    // Wrap as a module namespace object
                    if (mod && mod.__esModule) {
                        resolve(mod);
                    } else {
                        var ns = Object.assign({ default: mod }, mod);
                        resolve(ns);
                    }
                } catch(e) {
                    reject(e);
                }
            });
        };
    "#,
    )?;

    // ESM→CJS inline transformer for loading ESM packages via require()
    ctx.eval::<(), _>(r#"
        globalThis.__esmToCjs = function(src) {
            // Replace import.meta.* patterns before line-by-line ESM→CJS conversion.
            // These are syntax errors in QuickJS script mode; we swap them with the
            // named stubs that the require() module wrapper injects into every module.
            src = src
                .replace(/import\.meta\.resolve\(/g, '__vvva_meta_resolve__(')
                .replace(/import\.meta\.glob\(/g,    '__vvva_meta_glob__(')
                .replace(/import\.meta\.hot\b/g,     'undefined')
                .replace(/import\.meta\.vitest\b/g,  'undefined')
                .replace(/import\.meta\.env\b/g,     '__vvva_meta_env__')
                .replace(/import\.meta\.url\b/g,     '__vvva_meta_url__');

            var lines = src.split('\n');
            var out = [];
            // Function/class exports must be deferred to end of file so the
            // assignment runs AFTER the complete function body, not inside it.
            var deferredExports = [];
            var i = 0;

            while (i < lines.length) {
                var line = lines[i];
                var trimmed = line.trim();

                // import { ... } from '...' (multi-line: collect until closing brace found)
                if (/^import\s*\{/.test(trimmed) && !/\}/.test(trimmed.replace(/^import\s*\{\s*/, ''))) {
                    var _importLines = [trimmed];
                    i++;
                    while (i < lines.length) {
                        var _nextImportLine = lines[i].trim();
                        _importLines.push(_nextImportLine);
                        if (/\}\s*from\s+['"][^'"]+['"]/.test(_nextImportLine)) break;
                        if (/\}\s*;?\s*$/.test(_nextImportLine)) break;
                        i++;
                    }
                    var _fullImport = _importLines.join(' ');
                    // Try import { a, b } from 'specifier'
                    var _imRe = _fullImport.match(/^import\s*\{\s*([^}]+)\s*\}\s*from\s+['"]([^'"]+)['"]\s*;?$/);
                    if (_imRe) {
                        var _named = _imRe[1].split(',').map(function(s) {
                            s = s.trim();
                            if (!s) return '';
                            var parts = s.split(/\s+as\s+/);
                            if (parts.length === 2) return parts[0].trim() + ': ' + parts[1].trim();
                            return s;
                        }).filter(function(s) { return s; }).join(', ');
                        out.push('var {' + _named + '} = require(' + JSON.stringify(_imRe[2]) + ');');
                        i++; continue;
                    }
                    // If nothing matched, just join as-is
                    out.push(_importLines.join('\n'));
                    i++; continue;
                }
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
                // Use a two-step assignment: first replace module.exports, then set .default on
                // the new value. The chained form 'module.exports.default = module.exports = X'
                // sets .default on the OLD module.exports (JS evaluates LHS ref before RHS
                // assignment), so .default ends up on the stale empty object, not the exported one.
                if ((m = trimmed.match(/^export\s+default\s+(.*)/))) {
                    out.push('module.exports = ' + m[1]);
                    deferredExports.push('try{var __ed=module.exports;if(__ed!==null&&__ed!==undefined){__ed.default=__ed;module.exports=__ed;}}catch(__e){}');
                    i++; continue;
                }
                // export { ... } from '...' (multi-line: collect until closing brace found)
                if (/^export\s*\{/.test(trimmed) && !/\}/.test(trimmed.replace(/^export\s*\{\s*/, ''))) {
                    var _exportLines = [trimmed];
                    i++;
                    while (i < lines.length) {
                        var _nextLine = lines[i].trim();
                        _exportLines.push(_nextLine);
                        if (/\}.*from\s+['"][^'"]+['"]/.test(_nextLine)) break;
                        if (/\}\s*;?\s*$/.test(_nextLine)) break;
                        i++;
                    }
                    var _fullExport = _exportLines.join(' ');
                    // Try to match re-export: export { a, b } from 'specifier'
                    var _re = _fullExport.match(/^export\s*\{\s*([^}]+)\s*\}\s*from\s+['"]([^'"]+)['"]\s*;?$/);
                    if (_re) {
                        out.push(';(function(){var __re = require(' + JSON.stringify(_re[2]) + ');');
                        _re[1].split(',').forEach(function(s) {
                            s = s.trim();
                            if (!s) return;
                            var parts = s.split(/\s+as\s+/);
                            var local = parts[0].trim(), exported = (parts[1] || parts[0]).trim();
                            out.push('module.exports.' + exported + ' = __re.' + local + ';');
                        });
                        out.push('})();');
                        i++; continue;
                    }
                    // Fallback: export { a, b }
                    var _re2 = _fullExport.match(/^export\s*\{\s*([^}]+)\s*\}\s*;?$/);
                    if (_re2) {
                        _re2[1].split(',').forEach(function(s) {
                            s = s.trim();
                            if (!s) return;
                            var parts = s.split(/\s+as\s+/);
                            var local = parts[0].trim(), exported = (parts[1] || parts[0]).trim();
                            out.push('module.exports.' + exported + ' = ' + local + ';');
                        });
                        i++; continue;
                    }
                    // If nothing matched, just join as-is (likely a syntax error, but don't make it worse)
                    out.push(_exportLines.join('\n'));
                    i++; continue;
                }
                // export {}  (empty export, OXC emits this to mark a file as ESM — no-op)
                if (/^export\s*\{\s*\}\s*(from\s+['"][^'"]+['"]\s*)?;?$/.test(trimmed)) {
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
                    out.push(';(function(){var __re = require(' + JSON.stringify(m[2]) + ');');
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
                    out.push(';(function(){var __re=require(' + JSON.stringify(m[1]) + ');for(var __k in __re){if(__k!=="default")module.exports[__k]=__re[__k];}})();');
                    i++; continue;
                }
                // export const/let/var { a, b } = X  (destructured object export)
                if ((m = trimmed.match(/^export\s+(?:const|let|var)\s+\{([^}]+)\}\s*=/))) {
                    var decl = trimmed.replace(/^export\s+/, '');
                    out.push(decl);
                    m[1].split(',').forEach(function(s) {
                        s = s.trim();
                        if (!s) return;
                        // Handle aliasing: { orig: local } → export local as orig
                        var parts = s.split(/\s*:\s*/);
                        var localName = (parts[1] || parts[0]).trim().split(/\s+/)[0];
                        var exportName = parts[0].trim();
                        if (localName && /^\w+$/.test(localName)) {
                            out.push('module.exports.' + exportName + ' = ' + localName + ';');
                        }
                    });
                    i++; continue;
                }
                // export const/let/var [...] = X  (destructured array export)
                if ((m = trimmed.match(/^export\s+(?:const|let|var)\s+\[([^\]]+)\]\s*=/))) {
                    var decl = trimmed.replace(/^export\s+/, '');
                    out.push(decl);
                    m[1].split(',').forEach(function(s) {
                        s = s.trim();
                        if (s && /^\w+$/.test(s)) {
                            out.push('module.exports.' + s + ' = ' + s + ';');
                        }
                    });
                    i++; continue;
                }
                // export const/let/var/function/class X ...
                if ((m = trimmed.match(/^export\s+(const|let|var|function|class|async\s+function)\s+(\w+)/))) {
                    var decl = trimmed.replace(/^export\s+/, '');
                    var exportName = m[2];
                    var kind = m[1];
                    out.push(decl);
                    // Defer the export when:
                    //  - function/class: hoisted but body must complete first
                    //  - const/let: TDZ — variable may not be initialized until
                    //    the full expression (possibly multi-line IIFE) completes
                    //  - var without initializer (export var X;): the value is
                    //    populated by a later IIFE (TypeScript enum pattern), so
                    //    exporting immediately would capture 'undefined'.
                    var hasInitializer = decl.indexOf('=') >= 0;
                    if (kind === 'function' || kind === 'class' || kind.indexOf('async') >= 0
                        || kind === 'const' || kind === 'let' || !hasInitializer) {
                        deferredExports.push('module.exports.' + exportName + ' = ' + exportName + ';');
                    } else {
                        out.push('module.exports.' + exportName + ' = ' + exportName + ';');
                    }
                    i++; continue;
                }

                out.push(line);
                i++;
            }

            // Flush deferred function/class exports after all declarations are processed.
            // Wrap in try-catch: some properties may be read-only (e.g. defined via
            // Object.defineProperty with only a getter) and the assignment would throw.
            for (var di = 0; di < deferredExports.length; di++) {
                out.push('try{' + deferredExports[di] + '}catch(__de){}');
            }
            out.push('if(typeof module.exports.default==="undefined"&&Object.keys(module.exports).length>0){module.exports.__esModule=true;}');
            // Replace dynamic import() with __importAsync() so QuickJS doesn't
            // try to use the ES module loader (which isn't configured).
            var result = out.join('\n');
            if (result.indexOf('import(') >= 0) {
                result = result.replace(/\bimport\s*\(/g, '__importAsync(');
            }
            return result;
        };
    "#)?;

    // JS-level require() implementation
    // This avoids rquickjs Value<'js> lifetime issues by keeping all evaluation in JS.
    ctx.eval::<(), _>(r#"
        // Save a private ref so moduleRequire can bypass third-party polyfills
        // that overwrite globalThis.require (e.g. Metro/React Native asset loader).
        var __vvva_require;

        globalThis.require = __vvva_require = function(path) {
            // Defensive: if __vvva_require was somehow clobbered, re-bind.
            if (__vvva_require !== globalThis.require) {
                globalThis.require = __vvva_require;
            }
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

            // Resolve relative to the currently-executing file's directory.
            // Wrap in try/catch so fallback stubs are reachable when the
            // package is simply not installed (resolve throws before readFile).
            var resolvedPath;
            try {
                resolvedPath = __resolvePath(path, globalThis.__dirname);
            } catch (resolveErr) {
                if (globalThis.__fallbackModules !== undefined
                    && globalThis.__fallbackModules[path] !== undefined) {
                    return globalThis.__fallbackModules[path];
                }
                throw resolveErr;
            }

            // Check cache by resolved path.
            // If the cached value is a module wrapper, return its live `.exports`.
            if (globalThis.__requireCache[resolvedPath] !== undefined) {
                var _cached = globalThis.__requireCache[resolvedPath];
                if (_cached && _cached.__vvva_cjs_module) return _cached.exports;
                return _cached;
            }

            // Read and (if needed) transpile the file (path is already absolute)
            var source;
            try {
                source = __readFile(resolvedPath);
            } catch (readErr) {
                // Package resolved but file missing — still check fallback stubs
                if (globalThis.__fallbackModules !== undefined
                    && globalThis.__fallbackModules[path] !== undefined) {
                    var fb = globalThis.__fallbackModules[path];
                    globalThis.__requireCache[resolvedPath] = fb;
                    return fb;
                }
                throw readErr;
            }

            // Compute dirname — handle both / (Unix) and \ (Windows) separators
            var dirname = resolvedPath.replace(/[\/\\][^\/\\]*$/, '') || '.';
            var filename = resolvedPath;

            var result;

            // JSON files: parse directly instead of eval-ing as JS
            if (resolvedPath.endsWith('.json')) {
                result = JSON.parse(source);
                globalThis.__requireCache[resolvedPath] = result;
                return result;
            }

            // ESM-only files (.mjs) cannot be loaded via require().
            // ponytail: checks extension only; "type":"module" .js files still slip through.
            // Full fix needs package.json walk — add when a real package triggers it.
            if (resolvedPath.endsWith('.mjs')) {
                throw Object.assign(
                    new Error('ERR_REQUIRE_ESM: require() of ES Module ' + resolvedPath
                        + ' not supported. Use dynamic import() instead.'),
                    { code: 'ERR_REQUIRE_ESM' }
                );
            }

            // Native NAPI addons (.node files): delegate to Rust __napiRequire
            if (resolvedPath.endsWith('.node')) {
                if (typeof globalThis.__napiRequire !== 'function') {
                    throw new Error('NAPI not available: --allow-ffi is required to load .node addons');
                }
                result = globalThis.__napiRequire(resolvedPath);
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

            // Pre-cache a module wrapper with a live getter so circular requires
            // see the current module.exports even after reassignment.
            // Mongoose error classes rely on this: `class Foo extends require('./')`
            // must get the class, not the initial `{}`.
            (function(_mod) {
                globalThis.__requireCache[resolvedPath] = {
                    __vvva_cjs_module: true,
                    get exports() { return _mod.exports; },
                    set exports(v) { _mod.exports = v; }
                };
            })(globalThis.module);

            // Create a module-scoped require that captures this module's dirname.
            // This is critical for lazy getters and callbacks that call require()
            // after the module has finished loading (and __dirname was restored).
            var _capturedDirname = dirname;
            var moduleRequire = (function(capturedDir) {
                function mr(path) {
                    var saved = globalThis.__dirname;
                    globalThis.__dirname = capturedDir;
                    try { return __vvva_require(path); }
                    finally { globalThis.__dirname = saved; }
                }
                mr.resolve = function(path, options) {
                    var base = options && options.paths && options.paths[0] ? options.paths[0] : capturedDir;
                    return __resolvePath(path, base);
                };
                mr.resolve.paths = globalThis.require.resolve.paths;
                mr.cache = globalThis.__requireCache;
                mr.extensions = globalThis.require && globalThis.require.extensions || {};
                mr.main = globalThis.require.main;
                return mr;
            })(_capturedDirname);

            // If the file uses ESM syntax, inline-convert to CJS before wrapping.
            // __esmToCjs also handles the import() → __importAsync() replacement.
            if (/^\s*(import\s|import\{|export\s|export\{|export\s*default)/m.test(source)) {
                source = __esmToCjs(source);
            } else if (source.indexOf('import(') >= 0) {
                // Plain CJS that still uses dynamic import() — patch it here.
                source = source.replace(/\bimport\s*\(/g, '__importAsync(');
            }

            // Execute the module with CJS wrapper, passing the module-scoped require.
            // Preamble: declare import.meta.* stubs scoped to this module's __filename.
            // Each module gets its own __vvva_meta_url__ based on its own path, so
            // frameworks that use import.meta.url for relative asset resolution work
            // correctly regardless of which file is currently executing.
            var _metaPreamble =
                'var __vvva_meta_url__=(function(){' +
                  'try{return require("url").pathToFileURL(__filename).href;}' +
                  'catch(e){return "file:///"+__filename;}' +
                '})();' +
                // Use globalThis.process to avoid QuickJS TDZ errors when a module
                // declares `const process = require("process")` in its body —
                // that `const` is in TDZ for the entire function scope, so any
                // bare `process` reference in the metaPreamble would throw.
                'var __vvva_meta_env__=(function(){var _p=globalThis.process;return(_p&&_p.env)' +
                  '?Object.assign(Object.create(null),' +
                    '{MODE:(_p.env.NODE_ENV)||"production",' +
                    'PROD:_p.env.NODE_ENV!=="development",' +
                    'DEV:_p.env.NODE_ENV==="development",' +
                    'SSR:true,BASE_URL:"/"},_p.env)' +
                  ':{MODE:"production",PROD:true,DEV:false,SSR:true,BASE_URL:"/"};}());' +
                'function __vvva_meta_resolve__(s){return require.resolve(s);}' +
                'function __vvva_meta_glob__(){return {};}';

            var wrapper = 'var module=globalThis.module,exports=globalThis.exports,' +
                'require=moduleRequire,' +
                '__filename=globalThis.__filename,__dirname=globalThis.__dirname;\n' +
                _metaPreamble + '\n' +
                source;
            try { eval(wrapper); }
            catch (e) {
                var _eMsg = (typeof e === 'object' && e !== null) ? (e.message || String(e)) : String(e);
                throw new Error('[3va:' + resolvedPath + '] ' + _eMsg);
            }

            result = globalThis.module.exports;

            // Error location tracking (only on non-frozen objects, non-enumerable to avoid
            // breaking Object.values/keys/entries on the exports object)
            if (result && typeof result === 'object' && Object.isExtensible(result) && result.__vvva_module_path === undefined) {
                Object.defineProperty(result, '__vvva_module_path', { value: resolvedPath, enumerable: false, writable: true, configurable: true });
            }

            // Backward-compat wrap: some packages compiled with TypeScript emit
            //   Object.defineProperty(exports, "__esModule", { value: true });
            //   exports.create = create;
            // but callers (like Express 5) still call the module as a function:
            //   var cd = require('content-disposition'); cd(filename)
            // If the loaded module is not callable but has __esModule:true and a
            // `create` method, wrap it so the default call delegates to `create`.
            if (result && typeof result === 'object' && result.__esModule === true
                && typeof result !== 'function' && typeof result.create === 'function') {
                var _orig = result;
                result = function() { return _orig.create.apply(_orig, arguments); };
                var _keys = Object.keys(_orig);
                for (var _ki = 0; _ki < _keys.length; _ki++) { result[_keys[_ki]] = _orig[_keys[_ki]]; }
            }

            // Restore outer state
            globalThis.module = savedModule;
            globalThis.exports = savedExports;
            globalThis.__filename = savedFilename;
            globalThis.__dirname = savedDirname;

            // The module wrapper is already in the cache with a live getter.
            // No need to re-wrap — the getter already returns the current exports.

            return result;
        };

        // Node.js require.extensions — used by packages to register hooks for custom file types
        globalThis.require.extensions = {};
        globalThis.require.resolve = function(path, options) {
            var basedir = options && options.paths && options.paths[0] ? options.paths[0] : globalThis.__dirname;
            return __resolvePath(path, basedir);
        };
        globalThis.require.resolve.paths = function(name) {
            // Return standard node_modules search paths
            var dir = globalThis.__dirname || process.cwd();
            var paths = [];
            while (true) {
                paths.push(dir + '/node_modules');
                var parent = dir.replace(/\/[^\/]+$/, '');
                if (parent === dir) break;
                dir = parent;
            }
            return paths;
        };
        globalThis.require.cache = globalThis.__requireCache;
        globalThis.require.main = undefined;

        // ── ES2024+ polyfills (missing in QuickJS ES2023) ──────────────────────
        // Array.prototype group methods (ES2024)
        if (typeof Object.groupBy !== 'function') {
            Object.groupBy = function(items, cb) {
                var result = {};
                for (var i = 0; i < items.length; i++) {
                    var key = cb(items[i], i);
                    if (!result[key]) result[key] = [];
                    result[key].push(items[i]);
                }
                return result;
            };
        }
        if (typeof Map.groupBy !== 'function') {
            Map.groupBy = function(items, cb) {
                var result = new Map();
                for (var i = 0; i < items.length; i++) {
                    var key = cb(items[i], i);
                    if (!result.has(key)) result.set(key, []);
                    result.get(key).push(items[i]);
                }
                return result;
            };
        }

        // Promise.withResolvers (ES2024)
        if (typeof Promise.withResolvers !== 'function') {
            Promise.withResolvers = function() {
                var a, b;
                var p = new Promise(function(resolve, reject) { a = resolve; b = reject; });
                return { promise: p, resolve: a, reject: b };
            };
        }

        // Array.prototype.toSorted, toReversed, toSpliced, with (ES2023 — some QuickJS builds miss these)
        if (typeof Array.prototype.toSorted !== 'function') {
            Array.prototype.toSorted = function(compareFn) {
                var copy = this.slice();
                copy.sort(compareFn);
                return copy;
            };
        }
        if (typeof Array.prototype.toReversed !== 'function') {
            Array.prototype.toReversed = function() {
                var copy = this.slice();
                copy.reverse();
                return copy;
            };
        }
        if (typeof Array.prototype.toSpliced !== 'function') {
            Array.prototype.toSpliced = function(start, deleteCount) {
                var args = Array.prototype.slice.call(arguments);
                var copy = this.slice();
                copy.splice.apply(copy, args);
                return copy;
            };
        }
        if (typeof Array.prototype.with !== 'function') {
            Array.prototype.with = function(index, value) {
                var copy = this.slice();
                copy[index] = value;
                return copy;
            };
        }

        // RegExp.escape (ES2025)
        if (typeof RegExp.escape !== 'function') {
            RegExp.escape = function(str) {
                return String(str).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
            };
        }
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
//
// SECURITY NOTE — QuickJS context isolation limits:
//   runInNewContext/runInContext use `with(sandbox)` so unqualified names resolve
//   against sandbox first (matching Node.js semantics where sandbox IS the global).
//   However, `globalThis`, `require`, `process`, `fs`, etc. remain reachable via
//   the scope chain — QuickJS does not support V8-style isolated contexts.
//   For real process-level isolation use `worker_threads` with restricted permissions.
(function() {
    function Script(code, options) {
        this._code = code;
        this._filename = (options && options.filename) || '<anonymous>';
    }

    // Run in the current global context (no sandbox).
    Script.prototype.runInThisContext = function(options) {
        // Try expression first; fall back to statement block so that both
        // `module.exports = ...` (statement) and `1 + 2` (expression) work.
        try {
            // eslint-disable-next-line no-new-func
            return (new Function('return (' + this._code + ')'))();
        } catch(_) {
            // eslint-disable-next-line no-new-func
            return (new Function(this._code))();
        }
    };

    // Run with sandbox variables shadowing the global scope via `with`.
    // Mutations to sandbox-declared names are reflected back on the sandbox object.
    // globalThis / builtin globals are still reachable — see SECURITY NOTE above.
    Script.prototype.runInNewContext = function(sandbox, options) {
        sandbox = (sandbox && typeof sandbox === 'object') ? sandbox : {};
        // Try expression form first (returns the value); fall back to statement form.
        try {
            // eslint-disable-next-line no-new-func
            var fn = new Function('__sb__', 'with(__sb__) { return (' + this._code + '); }');
            return fn.call(sandbox, sandbox);
        } catch(_) {
            // eslint-disable-next-line no-new-func
            var fn2 = new Function('__sb__', 'with(__sb__) {\n' + this._code + '\n}');
            return fn2.call(sandbox, sandbox);
        }
    };

    Script.prototype.runInContext = function(ctx, options) {
        return Script.prototype.runInNewContext.call(this, ctx, options);
    };

    var _ctxTag = typeof Symbol !== 'undefined' ? Symbol('vmContext') : '__vmContext__';

    function createContext(sandbox) {
        sandbox = (sandbox && typeof sandbox === 'object') ? sandbox : {};
        try { Object.defineProperty(sandbox, _ctxTag, { value: true, enumerable: false, writable: false, configurable: false }); } catch(_) {}
        return sandbox;
    }

    function isContext(obj) {
        return typeof obj === 'object' && obj !== null && obj[_ctxTag] === true;
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

    // Reflect real TTY state onto the stream objects so chalk/ink/color libs work.
    var _isattyFn = (typeof __isatty === 'function') ? __isatty : function() { return false; };
    var _stdin1  = new ReadStream(0);  _stdin1.isTTY  = _isattyFn(0);
    var _stdout1 = new WriteStream(1); _stdout1.isTTY = _isattyFn(1);
    var _stderr1 = new WriteStream(2); _stderr1.isTTY = _isattyFn(2);

    var tty = {
        isatty: function(fd) { return _isattyFn(typeof fd === 'number' ? fd : 0); },
        ReadStream: ReadStream,
        WriteStream: WriteStream,
        // Pre-built stream singletons used by process.stdin/stdout/stderr
        _stdin: _stdin1,
        _stdout: _stdout1,
        _stderr: _stderr1
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

// ── repl ──────────────────────────────────────────────────────────────────────
// Minimal stub: repl.start() returns a REPLServer-like EventEmitter.
// Real interactive REPL is implemented in the CLI (3va sandbox).
(function() {
    var EventEmitter = globalThis.__requireCache['events'];
    function REPLServer(opts) {
        if (EventEmitter) EventEmitter.call(this);
        this.context = globalThis;
        this._closed = false;
    }
    if (EventEmitter) {
        REPLServer.prototype = Object.create(EventEmitter.prototype);
        REPLServer.prototype.constructor = REPLServer;
    }
    REPLServer.prototype.close = function() { this._closed = true; this.emit && this.emit('close'); };
    REPLServer.prototype.displayPrompt = function() {};
    REPLServer.prototype.clearBufferedCommand = function() {};
    REPLServer.prototype.defineCommand = function(keyword, cmd) {};
    REPLServer.prototype.setupHistory = function(path, cb) { if (cb) cb(null, this); };

    var repl = {
        start: function(opts) { return new REPLServer(opts); },
        REPLServer: REPLServer,
        REPL_MODE_SLOPPY: 0,
        REPL_MODE_STRICT: 1,
        builtinModules: Object.keys(globalThis.__requireCache || {})
    };
    globalThis.__requireCache['repl'] = repl;
    globalThis.__requireCache['node:repl'] = repl;
}());

// ── wasi ──────────────────────────────────────────────────────────────────────
// Stub: WASI host bindings are not supported; packages that guard with try/catch
// can still load.
(function() {
    function WASI(opts) {
        this.wasiImport = {};
    }
    WASI.prototype.initialize = function(instance) {};
    WASI.prototype.start = function(instance) { return 0; };
    WASI.prototype.getImportObject = function() { return { wasi_snapshot_preview1: {} }; };

    var wasi = { WASI: WASI };
    globalThis.__requireCache['wasi'] = wasi;
    globalThis.__requireCache['node:wasi'] = wasi;
}());

// ── trace_events ──────────────────────────────────────────────────────────────
(function() {
    function Tracing(cats) {
        this.enabled = false;
        this.categories = Array.isArray(cats) ? cats.join(',') : String(cats || '');
    }
    Tracing.prototype.enable = function() { this.enabled = true; };
    Tracing.prototype.disable = function() { this.enabled = false; };

    var traceEvents = {
        createTracing: function(opts) {
            return new Tracing(opts && opts.categories || []);
        },
        getEnabledCategories: function() { return ''; }
    };
    globalThis.__requireCache['trace_events'] = traceEvents;
    globalThis.__requireCache['node:trace_events'] = traceEvents;
}());

// ── dns ───────────────────────────────────────────────────────────────────────
// Backed by native __dnsLookup(hostname) -> Promise<json-array-of-IPs>
(function() {
    var __dnsLookup = globalThis.__dnsLookup;

    function _lookupOne(hostname, family, cb) {
        if (typeof __dnsLookup !== 'function') {
            var err = Object.assign(new Error('DNS not available'),
                { code: 'ENOTSUP', syscall: 'lookup' });
            if (typeof cb === 'function') return setTimeout(function() { cb(err); }, 0);
            throw err;
        }
        __dnsLookup(hostname).then(function(json) {
            var all = JSON.parse(json);
            if (!Array.isArray(all) || all.length === 0) {
                var nf = Object.assign(new Error('getaddrinfo ENOTFOUND ' + hostname),
                    { code: 'ENOTFOUND', errno: -3008, syscall: 'getaddrinfo', hostname: hostname });
                if (typeof cb === 'function') cb(nf);
                return;
            }
            // Filter by family: 4 = IPv4, 6 = IPv6, 0 or undefined = any
            var filtered = all;
            if (family === 4) filtered = all.filter(function(a) { return a.indexOf(':') === -1; });
            else if (family === 6) filtered = all.filter(function(a) { return a.indexOf(':') !== -1; });

            if (filtered.length === 0) {
                filtered = all;
            }

            var addr = filtered[0];
            var fam = addr.indexOf(':') === -1 ? 4 : 6;
            if (typeof cb === 'function') cb(null, addr, fam);
        }).catch(function(err) {
            var dnsErr = err instanceof Error ? err :
                Object.assign(new Error(String(err)), { code: 'EIO', errno: -5, syscall: 'getaddrinfo', hostname: hostname });
            if (typeof cb === 'function') cb(dnsErr);
        });
    }

    function _lookupAll(hostname, family, cb) {
        if (typeof __dnsLookup !== 'function') {
            var err = Object.assign(new Error('DNS not available'),
                { code: 'ENOTSUP', syscall: 'lookup' });
            if (typeof cb === 'function') return setTimeout(function() { cb(err); }, 0);
            throw err;
        }
        __dnsLookup(hostname).then(function(json) {
            var all = JSON.parse(json);
            if (!Array.isArray(all) || all.length === 0) {
                var nf = Object.assign(new Error('getaddrinfo ENOTFOUND ' + hostname),
                    { code: 'ENOTFOUND', errno: -3008, syscall: 'getaddrinfo', hostname: hostname });
                if (typeof cb === 'function') cb(nf);
                return;
            }
            var filtered = all;
            if (family === 4) filtered = all.filter(function(a) { return a.indexOf(':') === -1; });
            else if (family === 6) filtered = all.filter(function(a) { return a.indexOf(':') !== -1; });
            if (filtered.length === 0) filtered = all;
            var result = filtered.map(function(a) {
                return { address: a, family: a.indexOf(':') === -1 ? 4 : 6 };
            });
            if (typeof cb === 'function') cb(null, result);
        }).catch(function(err) {
            var dnsErr = err instanceof Error ? err :
                Object.assign(new Error(String(err)), { code: 'EIO', errno: -5, syscall: 'getaddrinfo', hostname: hostname });
            if (typeof cb === 'function') cb(dnsErr);
        });
    }

    var dns = {
        lookup: function(hostname, options, callback) {
            if (typeof options === 'function') { callback = options; options = 0; }
            if (typeof options === 'number') options = { family: options, all: false };
            options = options || {};
            var family = options.family || 0;
            var all = options.all || false;
            if (all) return _lookupAll(hostname, family, callback);
            return _lookupOne(hostname, family, callback);
        },
        lookupService: function(addr, port, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, addr, 'tcp'); }, 0);
        },
        resolve: function(hostname, rrtype, cb) {
            if (typeof rrtype === 'function') { cb = rrtype; rrtype = 'A'; }
            if (typeof __dnsLookup !== 'function') {
                var err = Object.assign(new Error('DNS not available'), { code: 'ENOTSUP' });
                if (typeof cb === 'function') return setTimeout(function() { cb(err); }, 0);
                throw err;
            }
            __dnsLookup(hostname).then(function(json) {
                var all = JSON.parse(json);
                if (!Array.isArray(all) || all.length === 0) {
                    var nf = Object.assign(new Error('getaddrinfo ENOTFOUND ' + hostname),
                        { code: 'ENOTFOUND', errno: -3008, syscall: 'getaddrinfo' });
                    if (typeof cb === 'function') cb(nf);
                    return;
                }
                if (rrtype === 'A') {
                    var v4 = all.filter(function(a) { return a.indexOf(':') === -1; });
                    if (typeof cb === 'function') cb(null, v4);
                } else if (rrtype === 'AAAA') {
                    var v6 = all.filter(function(a) { return a.indexOf(':') !== -1; });
                    if (typeof cb === 'function') cb(null, v6);
                } else if (rrtype === 'ANY') {
                    if (typeof cb === 'function') cb(null, all);
                } else {
                    var nf2 = Object.assign(new Error('queryA ENOTFOUND ' + hostname),
                        { code: 'ENOTFOUND' });
                    if (typeof cb === 'function') cb(nf2);
                }
            }).catch(function(err) {
                var dnsErr = err instanceof Error ? err :
                    Object.assign(new Error(String(err)), { code: 'EIO', syscall: 'getaddrinfo' });
                if (typeof cb === 'function') cb(dnsErr);
            });
        },
        resolve4: function(hostname, options, cb) {
            if (typeof options === 'function') { cb = options; options = {}; }
            dns.resolve(hostname, 'A', cb);
        },
        resolve6: function(hostname, options, cb) {
            if (typeof options === 'function') { cb = options; options = {}; }
            dns.resolve(hostname, 'AAAA', cb);
        },
        resolveMx: function(hostname, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, []); }, 0);
        },
        resolveTxt: function(hostname, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, []); }, 0);
        },
        resolveSrv: function(hostname, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, []); }, 0);
        },
        resolveNs: function(hostname, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, []); }, 0);
        },
        resolveCname: function(hostname, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, []); }, 0);
        },
        resolveNaptr: function(hostname, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, []); }, 0);
        },
        resolvePtr: function(hostname, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, []); }, 0);
        },
        resolveSoa: function(hostname, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, null); }, 0);
        },
        resolveAny: function(hostname, cb) {
            dns.resolve(hostname, 'ANY', cb);
        },
        reverse: function(ip, cb) {
            if (typeof cb === 'function') setTimeout(function() { cb(null, []); }, 0);
        },
        setServers: function() {},
        getServers: function() { return []; },
        ADDRCONFIG: 0,
        V4MAPPED: 8,
        ALL: 16,
        promises: {
            lookup: function(hostname, options) {
                return new Promise(function(resolve, reject) {
                    dns.lookup(hostname, options, function(err, addr, fam) {
                        if (err) reject(err); else resolve(options && options.all ? addr : { address: addr, family: fam });
                    });
                });
            },
            resolve: function(hostname, rrtype) {
                return new Promise(function(resolve, reject) {
                    dns.resolve(hostname, rrtype || 'A', function(err, result) {
                        if (err) reject(err); else resolve(result);
                    });
                });
            },
            resolve4: function(hostname, options) {
                return new Promise(function(resolve, reject) {
                    dns.resolve4(hostname, options, function(err, result) {
                        if (err) reject(err); else resolve(result);
                    });
                });
            },
            resolve6: function(hostname, options) {
                return new Promise(function(resolve, reject) {
                    dns.resolve6(hostname, options, function(err, result) {
                        if (err) reject(err); else resolve(result);
                    });
                });
            }
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
            var rss = 0;
            try {
                var mu = (typeof process !== 'undefined' && typeof process.memoryUsage === 'function')
                    ? process.memoryUsage() : null;
                if (mu) rss = mu.rss || 0;
            } catch(e) {}
            var total = (typeof __osMemTotal === 'function') ? __osMemTotal() : 536870912;
            return {
                total_heap_size: rss,
                total_heap_size_executable: 0,
                total_physical_size: rss,
                total_available_size: Math.max(0, total - rss),
                used_heap_size: rss,
                heap_size_limit: total,
                malloced_memory: 0,
                peak_malloced_memory: rss,
                does_zap_garbage: 0,
                number_of_native_contexts: 1,
                number_of_detached_contexts: 0
            };
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
    if (osM && !osM.constants) osM.constants = {
        signals: {}, errno: {}, priority: {},
        // dlopen flags — used by Prisma and other NAPI loaders
        dlopen: { RTLD_LAZY: 1, RTLD_NOW: 2, RTLD_GLOBAL: 8, RTLD_LOCAL: 4, RTLD_DEEPBIND: 8 }
    };
}());

// ── process: complete stdin + signals + missing fields ───────────────────────
(function() {
    var p = globalThis.process;
    if (!p) return;

    var _isatty = (typeof __isatty === 'function') ? __isatty : function() { return false; };

    // stdin as a no-op Readable (CLI tools that read from stdin get an ended stream)
    var stream = globalThis.__requireCache['stream'];
    if (stream && !p.stdin) {
        var stdin = new stream.Readable({ read: function() { this.push(null); } });
        stdin.isTTY = _isatty(0);
        stdin.fd = 0;
        p.stdin = stdin;
    }

    // stdout / stderr as Writable streams
    if (stream) {
        if (p.stdout && !(p.stdout instanceof stream.Writable)) {
            var stdout = new stream.Writable({
                write: function(chunk, encoding, cb) {
                    var s = (typeof chunk === 'string' ? chunk : new TextDecoder().decode(chunk));
                    __stdoutWrite(s);
                    cb();
                },
                final: function(cb) { cb(); }
            });
            stdout.fd = 1;
            stdout.isTTY = _isatty(1);
            stdout.columns = 80;
            stdout.rows = 24;
            p.stdout = stdout;
        }
        if (p.stderr && !(p.stderr instanceof stream.Writable)) {
            var stderr = new stream.Writable({
                write: function(chunk, encoding, cb) {
                    var s = (typeof chunk === 'string' ? chunk : new TextDecoder().decode(chunk));
                    __stderrWrite(s);
                    cb();
                },
                final: function(cb) { cb(); }
            });
            stderr.fd = 2;
            stderr.isTTY = _isatty(2);
            stderr.columns = 80;
            stderr.rows = 24;
            p.stderr = stderr;
        }
    }

    // process.env as a dynamic Proxy
    if (typeof Proxy !== 'undefined' && p.env && !p.env.__isProxy) {
        var _envStore = {};
        // Copy existing env vars into the store
        var _envKeys = Object.keys(p.env);
        for (var i = 0; i < _envKeys.length; i++) _envStore[_envKeys[i]] = p.env[_envKeys[i]];
        // Allow write access if --allow-env or individual var permissions
        var _envProxy = new Proxy(_envStore, {
            get: function(target, key) {
                if (key === '__isProxy') return true;
                if (typeof key === 'symbol') return undefined;
                return target[key];
            },
            set: function(target, key, value) {
                if (key === '__isProxy') return false;
                if (typeof key === 'symbol') return false;
                target[key] = String(value);
                return true;
            },
            has: function(target, key) { return key in target; },
            deleteProperty: function(target, key) { return delete target[key]; },
            ownKeys: function(target) { return Object.keys(target); },
            getOwnPropertyDescriptor: function(target, key) {
                if (key in target) return { configurable: true, enumerable: true, value: target[key], writable: true };
                return undefined;
            }
        });
        p.env = _envProxy;
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

// ── Express / HTTP serving utilities ─────────────────────────────────────────
// Polyfills for packages that express, send, serve-static, and related
// middleware depend on.  These are registered BEFORE file-system resolution so
// packages that bundle their own copy still work; they simply override the stub.
(function() {

// ── depd ─────────────────────────────────────────────────────────────────────
(function() {
    function depd(ns) {
        return function deprecate(fn, msg) {
            if (typeof fn !== 'function') return fn;
            return fn;
        };
    }
    depd.function = depd;
    depd.property = depd;
    globalThis.__requireCache['depd'] = depd;
})();

// ── encodeurl ─────────────────────────────────────────────────────────────────
(function() {
    // Encode unsafe characters (non-ASCII + special) but leave already-encoded
    // %XX sequences and the path separator intact.
    var UNENCODED = /[^\x21\x23-\x3b\x3d\x3f-\x5a\x5f\x61-\x7a\x7e]/g;
    function encodeUrl(url) {
        return String(url).replace(UNENCODED, function(c) { return encodeURIComponent(c); });
    }
    globalThis.__requireCache['encodeurl'] = encodeUrl;
})();

// ── escape-html ───────────────────────────────────────────────────────────────
(function() {
    function escapeHtml(str) {
        return String(str)
            .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
            .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
    }
    globalThis.__requireCache['escape-html'] = escapeHtml;
})();

// ── destroy ───────────────────────────────────────────────────────────────────
(function() {
    function destroy(stream, err) {
        if (!stream) return stream;
        if (typeof stream.destroy === 'function') {
            if (err) stream.destroy(err); else stream.destroy();
        } else if (typeof stream.abort === 'function') {
            stream.abort();
        } else if (typeof stream.close === 'function') {
            stream.close();
        }
        return stream;
    }
    globalThis.__requireCache['destroy'] = destroy;
})();

// ── on-finished ───────────────────────────────────────────────────────────────
(function() {
    function isFinished(res) {
        return !!(res.writableEnded || res.finished ||
            (res._writableState && (res._writableState.finished || res._writableState.ended)));
    }
    function onFinished(res, cb) {
        if (isFinished(res)) {
            setTimeout(function() { cb(null, res); }, 0);
        } else {
            function finish() { cb(null, res); }
            function onerr(err) { cb(err); }
            res.once('finish', finish);
            res.once('end', finish);
            res.once('close', finish);
            res.once('error', onerr);
        }
        return res;
    }
    onFinished.isFinished = isFinished;
    globalThis.__requireCache['on-finished'] = onFinished;
})();

// ── on-headers ────────────────────────────────────────────────────────────────
(function() {
    function onHeaders(res, cb) {
        var orig = res.writeHead;
        res.writeHead = function() {
            if (!res._onHeadersCalled) {
                res._onHeadersCalled = true;
                cb.call(res);
            }
            return orig.apply(res, arguments);
        };
    }
    globalThis.__requireCache['on-headers'] = onHeaders;
})();

// ── fresh ─────────────────────────────────────────────────────────────────────
(function() {
    function fresh(reqHeaders, resHeaders) {
        var modifiedSince = reqHeaders['if-modified-since'];
        var noneMatch = reqHeaders['if-none-match'];
        if (!modifiedSince && !noneMatch) return false;
        var cc = reqHeaders['cache-control'] || '';
        if (cc.indexOf('no-cache') !== -1) return false;
        if (noneMatch && noneMatch !== '*') {
            var etag = resHeaders['etag'] || resHeaders['ETag'] || '';
            if (!etag) return false;
            var etagStale = noneMatch.split(',').every(function(match) {
                var m = match.trim();
                return m !== etag && m !== 'W/' + etag && 'W/' + m !== etag;
            });
            if (etagStale) return false;
        }
        if (modifiedSince) {
            var lastModified = resHeaders['last-modified'] || resHeaders['Last-Modified'] || '';
            if (!lastModified) return false;
            var lm = Date.parse(lastModified), ms = Date.parse(modifiedSince);
            if (isNaN(lm) || isNaN(ms) || lm > ms) return false;
        }
        return true;
    }
    globalThis.__requireCache['fresh'] = fresh;
})();

// ── etag ──────────────────────────────────────────────────────────────────────
(function() {
    function etag(entity, options) {
        var weak = options && options.weak;
        var tag;
        if (entity && typeof entity === 'object' && !Buffer.isBuffer(entity)) {
            // stat object — weak etag from mtime + size
            var size = entity.size != null ? entity.size : 0;
            var mtime = entity.mtime ? +new Date(entity.mtime) : 0;
            tag = 'W/"' + size.toString(16) + '-' + mtime.toString(16) + '"';
        } else {
            var bytes;
            if (typeof entity === 'string') {
                bytes = new TextEncoder().encode(entity);
            } else if (Buffer.isBuffer(entity) || entity instanceof Uint8Array) {
                bytes = entity instanceof Uint8Array ? entity : new Uint8Array(entity.buffer, entity.byteOffset, entity.byteLength);
            } else {
                bytes = new TextEncoder().encode(String(entity));
            }
            var len = bytes.length;
            // Simple djb2-like hash for a fast fingerprint
            var h = 5381;
            for (var i = 0; i < len; i++) h = ((h << 5) + h) ^ bytes[i];
            h = h >>> 0;
            tag = (weak ? 'W/' : '') + '"' + len.toString(16) + '-' + h.toString(16) + '"';
        }
        return tag;
    }
    globalThis.__requireCache['etag'] = etag;
})();

// ── parseurl ──────────────────────────────────────────────────────────────────
(function() {
    function parseurl(req) {
        var url = req.url;
        if (!url) return null;
        if (req._parsedUrl && req._parsedUrl.href === url) return req._parsedUrl;
        var parsed;
        try {
            var u = new URL('http://x' + url);
            parsed = { pathname: u.pathname, search: u.search || null,
                query: u.search ? u.search.slice(1) : null, href: url, path: url, hash: null };
        } catch(e) {
            var q = url.indexOf('?');
            if (q === -1) {
                parsed = { pathname: url, search: null, query: null, href: url, path: url, hash: null };
            } else {
                parsed = { pathname: url.slice(0, q), search: url.slice(q),
                    query: url.slice(q + 1), href: url, path: url, hash: null };
            }
        }
        req._parsedUrl = parsed;
        return parsed;
    }
    parseurl.original = parseurl;
    globalThis.__requireCache['parseurl'] = parseurl;
    globalThis.__requireCache['node:parseurl'] = parseurl;
})();

// ── range-parser ──────────────────────────────────────────────────────────────
(function() {
    function rangeParser(size, header, options) {
        if (typeof header !== 'string') return -2;
        var idx = header.indexOf('=');
        if (idx === -1) return -2;
        var type = header.slice(0, idx).trim();
        if (type !== 'bytes') return -2;
        var rangeStrs = header.slice(idx + 1).split(',');
        var ranges = [];
        for (var i = 0; i < rangeStrs.length; i++) {
            var r = rangeStrs[i].trim(), dash = r.indexOf('-');
            if (dash === -1) return -2;
            var startStr = r.slice(0, dash), endStr = r.slice(dash + 1);
            var start, end;
            if (startStr === '') { start = size - parseInt(endStr, 10); end = size - 1; }
            else if (endStr === '') { start = parseInt(startStr, 10); end = size - 1; }
            else { start = parseInt(startStr, 10); end = parseInt(endStr, 10); }
            if (isNaN(start) || isNaN(end) || start > end) continue;
            if (end > size - 1) end = size - 1;
            if (start < 0) continue;
            ranges.push({ start: start, end: end });
        }
        if (ranges.length === 0) return -1;
        ranges.type = type;
        // Combine overlapping ranges if combine option set
        if (options && options.combine) {
            var combined = [ranges[0]];
            for (var j = 1; j < ranges.length; j++) {
                var prev = combined[combined.length - 1];
                if (ranges[j].start <= prev.end + 1) {
                    if (ranges[j].end > prev.end) prev.end = ranges[j].end;
                } else { combined.push(ranges[j]); }
            }
            combined.type = type;
            return combined;
        }
        return ranges;
    }
    globalThis.__requireCache['range-parser'] = rangeParser;
})();

// ── http-errors ───────────────────────────────────────────────────────────────
(function() {
    var HTTP_STATUS = {400:'Bad Request',401:'Unauthorized',402:'Payment Required',
        403:'Forbidden',404:'Not Found',405:'Method Not Allowed',406:'Not Acceptable',
        408:'Request Timeout',409:'Conflict',410:'Gone',411:'Length Required',
        412:'Precondition Failed',413:'Payload Too Large',414:'URI Too Long',
        415:'Unsupported Media Type',416:'Range Not Satisfiable',418:'I\'m a Teapot',
        422:'Unprocessable Entity',423:'Locked',425:'Too Early',428:'Precondition Required',
        429:'Too Many Requests',451:'Unavailable For Legal Reasons',
        500:'Internal Server Error',501:'Not Implemented',502:'Bad Gateway',
        503:'Service Unavailable',504:'Gateway Timeout',505:'HTTP Version Not Supported'};

    function HttpError(status, message, props) {
        this.name = 'HttpError';
        this.status = this.statusCode = status || 500;
        this.message = message || HTTP_STATUS[this.status] || 'Error';
        this.expose = this.status < 500;
        if (props) for (var k in props) if (props.hasOwnProperty(k)) this[k] = props[k];
    }
    HttpError.prototype = Object.create(Error.prototype);
    HttpError.prototype.constructor = HttpError;
    HttpError.prototype.toString = function() { return this.name + ': ' + this.message; };

    function createError() {
        var status = 500, message, props;
        for (var i = 0; i < arguments.length; i++) {
            var a = arguments[i];
            if (typeof a === 'number') { status = a; }
            else if (typeof a === 'string') { message = a; }
            else if (a && typeof a === 'object') {
                if (a instanceof Error) { message = a.message; props = a; }
                else { props = a; if (a.status) status = a.status; if (a.message) message = a.message; }
            }
        }
        var err = new HttpError(status, message, props);
        return err;
    }
    createError.HttpError = HttpError;
    createError.isHttpError = function(v) { return v instanceof HttpError; };
    // Named shorthands
    Object.keys(HTTP_STATUS).forEach(function(code) {
        var name = HTTP_STATUS[code].replace(/\s+/g, '');
        createError[code] = createError[name] = function(msg, p) { return createError(parseInt(code), msg, p); };
    });
    globalThis.__requireCache['http-errors'] = createError;
    globalThis.__requireCache['createerror'] = createError;
})();

// ── mime-db (minimal JSON table) ──────────────────────────────────────────────
(function() {
    var db = {
        'text/html':{'source':'iana','charset':'UTF-8','extensions':['html','htm']},
        'text/css':{'source':'iana','extensions':['css']},
        'application/javascript':{'source':'iana','charset':'UTF-8','extensions':['js','mjs','cjs']},
        'application/json':{'source':'iana','charset':'UTF-8','extensions':['json']},
        'application/ld+json':{'extensions':['jsonld']},
        'application/xml':{'extensions':['xml']},
        'text/plain':{'source':'iana','charset':'UTF-8','extensions':['txt','text']},
        'text/csv':{'extensions':['csv']},
        'text/markdown':{'extensions':['md','markdown']},
        'text/yaml':{'extensions':['yaml','yml']},
        'image/png':{'source':'iana','extensions':['png']},
        'image/jpeg':{'source':'iana','extensions':['jpg','jpeg']},
        'image/gif':{'source':'iana','extensions':['gif']},
        'image/svg+xml':{'source':'iana','extensions':['svg','svgz']},
        'image/x-icon':{'extensions':['ico']},
        'image/webp':{'source':'iana','extensions':['webp']},
        'image/avif':{'extensions':['avif']},
        'image/bmp':{'extensions':['bmp']},
        'image/tiff':{'extensions':['tiff','tif']},
        'audio/mpeg':{'source':'iana','extensions':['mp3','mpga']},
        'audio/ogg':{'extensions':['ogg','oga','opus']},
        'audio/wav':{'extensions':['wav']},
        'audio/flac':{'extensions':['flac']},
        'audio/aac':{'extensions':['aac']},
        'audio/webm':{'extensions':['weba']},
        'video/mp4':{'source':'iana','extensions':['mp4']},
        'video/webm':{'source':'iana','extensions':['webm']},
        'video/x-msvideo':{'extensions':['avi']},
        'video/quicktime':{'extensions':['mov','qt']},
        'video/x-matroska':{'extensions':['mkv']},
        'font/woff':{'source':'iana','extensions':['woff']},
        'font/woff2':{'source':'iana','extensions':['woff2']},
        'font/ttf':{'source':'iana','extensions':['ttf']},
        'font/otf':{'source':'iana','extensions':['otf']},
        'application/vnd.ms-fontobject':{'extensions':['eot']},
        'application/zip':{'source':'iana','extensions':['zip']},
        'application/x-tar':{'extensions':['tar']},
        'application/gzip':{'source':'iana','extensions':['gz']},
        'application/pdf':{'source':'iana','extensions':['pdf']},
        'application/wasm':{'extensions':['wasm']},
        'application/manifest+json':{'extensions':['webmanifest']},
        'application/octet-stream':{'source':'iana','extensions':['bin','dms','lrf','mar','so','dist','distz','pkg','bpk','dump','elc','deploy']},
    };
    globalThis.__requireCache['mime-db'] = db;
})();

// ── mime-types ────────────────────────────────────────────────────────────────
(function() {
    var db = globalThis.__requireCache['mime-db'] || {};
    // Build ext → type and type → ext maps
    var extToType = Object.create(null);
    var typeToExt = Object.create(null);
    Object.keys(db).forEach(function(type) {
        var entry = db[type];
        if (entry.extensions) {
            entry.extensions.forEach(function(ext) {
                extToType[ext] = type;
            });
            if (!typeToExt[type]) typeToExt[type] = entry.extensions[0];
        }
    });
    var mimeTypes = {
        lookup: function(path) {
            if (!path || typeof path !== 'string') return false;
            var ext = path.replace(/^.*[./]/, '').toLowerCase();
            return extToType[ext] || false;
        },
        contentType: function(type) {
            if (!type || typeof type !== 'string') return false;
            var mime = type.indexOf('/') === -1 ? mimeTypes.lookup(type) : type;
            if (!mime) return false;
            if (mime.indexOf('charset') === -1) {
                var entry = db[mime];
                if (entry && entry.charset) mime += '; charset=' + entry.charset.toLowerCase();
            }
            return mime;
        },
        extension: function(type) {
            if (!type || typeof type !== 'string') return false;
            var t = type.split(';')[0].trim().toLowerCase();
            return typeToExt[t] || false;
        },
        charset: function(type) {
            var t = type.split(';')[0].trim().toLowerCase();
            var entry = db[t];
            return (entry && entry.charset) || false;
        },
    };
    globalThis.__requireCache['mime-types'] = mimeTypes;
})();

// ── mime ──────────────────────────────────────────────────────────────────────
(function() {
    var mimeTypes = globalThis.__requireCache['mime-types'];
    function Mime() {
        this._types = Object.create(null);
        this._extensions = Object.create(null);
    }
    Mime.prototype.define = function(typeMap, force) {
        Object.keys(typeMap).forEach(function(type) {
            var exts = typeMap[type];
            exts.forEach(function(ext) {
                if (!force && this._types[ext]) return;
                this._types[ext] = type;
                if (!this._extensions[type]) this._extensions[type] = ext;
            }, this);
        }, this);
        return this;
    };
    Mime.prototype.getType = function(path) {
        if (!path) return null;
        var ext = path.replace(/^.*[./]/, '').toLowerCase();
        return this._types[ext] || null;
    };
    Mime.prototype.getExtension = function(type) {
        var t = (type || '').split(';')[0].trim().toLowerCase();
        return this._extensions[t] || null;
    };
    var mime = new Mime();
    // Seed from mime-types
    if (mimeTypes) {
        var db = globalThis.__requireCache['mime-db'] || {};
        Object.keys(db).forEach(function(type) {
            var entry = db[type];
            if (entry.extensions && entry.extensions.length) {
                var map = {};
                map[type] = entry.extensions;
                mime.define(map, false);
            }
        });
    }
    // Make mime work as both a class and a singleton
    mime.Mime = Mime;
    mime.default_type = 'application/octet-stream';
    globalThis.__requireCache['mime'] = mime;
})();

// ── serve-static (basic fallback) ─────────────────────────────────────────────
// A minimal implementation that covers the common `express.static(root)` case.
// If `serve-static` is installed as an npm package it will override this stub.
(function() {
    var mime = globalThis.__requireCache['mime-types'];
    var encodeUrl = globalThis.__requireCache['encodeurl'];
    var escapeHtml = globalThis.__requireCache['escape-html'];
    var parseurl = globalThis.__requireCache['parseurl'];

    function serveStatic(root, opts) {
        opts = opts || {};
        var index = opts.index !== undefined ? opts.index : 'index.html';
        var dotfiles = opts.dotfiles || 'ignore';
        var fs = require('fs');
        var path = require('path');

        return function staticMiddleware(req, res, next) {
            if (req.method !== 'GET' && req.method !== 'HEAD') return next();
            var parsed = parseurl ? parseurl(req) : { pathname: req.url };
            var urlPath = parsed.pathname || '/';
            // Decode + security check
            try { urlPath = decodeURIComponent(urlPath); } catch(e) { return next(); }
            if (urlPath.indexOf('\0') !== -1) return next();
            // Check dotfiles
            if (dotfiles === 'ignore' && /(^|[/\\])\.[^/\\]/.test(urlPath)) return next();
            var filePath = path.join(root, urlPath);
            // Prevent path traversal
            var relative = path.relative(root, filePath);
            if (relative.startsWith('..') || /^[A-Za-z]:/.test(relative) || relative.charAt(0) === '/') return next();
            fs.stat(filePath, function(err, stat) {
                if (err) return next();
                if (stat.isDirectory()) {
                    if (index === false) return next();
                    var idxPath = path.join(filePath, index);
                    fs.stat(idxPath, function(err2, stat2) {
                        if (err2 || !stat2.isFile()) return next();
                        sendFile(idxPath, stat2, req, res, next);
                    });
                    return;
                }
                if (!stat.isFile()) return next();
                sendFile(filePath, stat, req, res, next);
            });
        };
    }

    function sendFile(filePath, stat, req, res, next) {
        var path = require('path');
        var fs = require('fs');
        var mimeTypes = globalThis.__requireCache['mime-types'];
        var contentType = (mimeTypes && mimeTypes.contentType(path.extname(filePath))) || 'application/octet-stream';
        res.setHeader('Content-Type', contentType);
        res.setHeader('Content-Length', String(stat.size));
        res.setHeader('Last-Modified', stat.mtime.toUTCString());
        if (req.method === 'HEAD') {
            res.statusCode = 200;
            return res.end();
        }
        var stream = fs.createReadStream(filePath);
        stream.on('error', function(e) { next(e); });
        stream.pipe(res);
    }

    // Only register as fallback — don't override if npm package already loaded
    if (!globalThis.__requireCache['serve-static']) {
        globalThis.__requireCache['serve-static'] = serveStatic;
    }
})();

}()); // end Express utilities block

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

// ── React Native / Expo globals ──────────────────────────────────────────────
(function() {

// Detect platform
var platform = 'web';
var userAgent = typeof navigator !== 'undefined' && navigator.userAgent || '';
if (typeof process !== 'undefined' && process.platform === 'win32') platform = 'windows';

// RN platform detection
globalThis.__REACT_NATIVE__ = true;

// Expo OS identifier: 'web' allows server-side Expo packages to use their web
// code paths (avoiding native-only code like registerWebModule calls).
if (typeof process !== 'undefined' && process.env) {
    if (!process.env.EXPO_OS) process.env.EXPO_OS = 'web';
}

// requestAnimationFrame / cancelAnimationFrame
if (typeof globalThis.requestAnimationFrame !== 'function') {
    globalThis.requestAnimationFrame = function(cb) {
        return setTimeout(function() { cb(Date.now()); }, 16);
    };
}
if (typeof globalThis.cancelAnimationFrame !== 'function') {
    globalThis.cancelAnimationFrame = function(id) { clearTimeout(id); };
}

// NativeModules — stub for the RN native bridge
// Packages like expo-* call NativeModules.ExpoXXX.method() which becomes no-ops.
if (typeof globalThis.nativeModuleProxy === 'undefined') {
    globalThis.nativeModuleProxy = {};
}
if (typeof globalThis.NativeModules === 'undefined') {
    var NativeModules = {};
    // Proxy for NativeModules: only explicitly registered modules return a value.
    // Unknown modules return undefined so truthiness checks like
    //   if (NativeModules.EXDevLauncher) {...}
    // correctly skip the block. Registered modules are set via registerNativeModule.
    globalThis.NativeModules = new Proxy(NativeModules, {
        get: function(target, prop) {
            if (prop === '__esModule' || prop === 'then') return undefined;
            if (prop in target) return target[prop];
            // Unknown module: return undefined so callers treat it as absent.
            return undefined;
        },
        set: function(target, prop, value) { target[prop] = value; return true; }
    });
}

// ExpoModules — Expo's native module registry (used by expo-modules-core)
if (typeof globalThis.ExpoModules === 'undefined') {
    globalThis.ExpoModules = new Proxy({}, {
        get: function(target, prop) {
            if (prop === '__esModule' || prop === 'then') return undefined;
            if (prop in target) return target[prop];
            var mod = new Proxy({}, {
                get: function(t, p) {
                    if (p === '__esModule' || p === 'then') return undefined;
                    if (p in t) return t[p];
                    return function() { return undefined; };
                },
                set: function(t, p, v) { t[p] = v; return true; }
            });
            target[prop] = mod;
            return mod;
        },
        set: function(target, prop, value) { target[prop] = value; return true; }
    });
}

// Platform.OS
if (typeof globalThis.Platform === 'undefined') {
    globalThis.Platform = { OS: platform, Version: 1, select: function(obj) { return obj[platform] || obj.default; } };
}

// Asset require — intercept require() for non-JS files
(function() {
    var ASSET_EXTS = ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp',
                      '.svg', '.woff', '.woff2', '.ttf', '.otf', '.eot',
                      '.mp4', '.webm', '.ogg', '.mp3', '.wav', '.m4a',
                      '.aac', '.pdf', '.ico', '.cur'];
    var assetMap = {};
    var assetIdCounter = 1;

    // Patch require to handle asset files
    var origResolve = globalThis.__resolvePath;

    function isAssetExt(p) {
        for (var i = 0; i < ASSET_EXTS.length; i++) {
            if (p.endsWith(ASSET_EXTS[i])) return true;
        }
        return false;
    }

    // Override require's path resolution to detect assets
    var _origRequire = globalThis.require;
    globalThis.require = function(path) {
        var dirname = globalThis.__dirname || '.';
        var resolvedPath;
        try {
            resolvedPath = origResolve(path, dirname);
        } catch (e) {
            // Resolver may fail for bare specifiers like 'zlib' or 'child_process'
            // that are actually built-in modules in the require cache.
            // Fall back to the original require, which checks the cache first.
            return _origRequire(path);
        }

        if (isAssetExt(resolvedPath)) {
            var id = assetMap[resolvedPath];
            if (!id) {
                id = assetIdCounter++;
                assetMap[resolvedPath] = id;
            }
            var asset = {
                __packager_asset: true,
                uri: resolvedPath,
                width: undefined,
                height: undefined,
                httpServerLocation: '/assets',
                scales: [1],
                hash: id,
                name: resolvedPath.split(/[\/\\]/).pop().replace(/\.[^.]+$/, ''),
                type: resolvedPath.split('.').pop(),
                fileSystemLocation: dirname
            };
            return id;
        }

        return _origRequire(path);
    };
    globalThis.require.assetMap = assetMap;
    // Copy over resolve/cache/main/extensions from original require
    if (_origRequire.resolve) globalThis.require.resolve = _origRequire.resolve;
    if (_origRequire.cache) globalThis.require.cache = _origRequire.cache;
    if (_origRequire.main !== undefined) globalThis.require.main = _origRequire.main;
    if (_origRequire.extensions) globalThis.require.extensions = _origRequire.extensions;

    // Also patch __resolvePath for ESM compatibility
    globalThis.__resolvePath = function(path, basedir) {
        var rp = origResolve(path, basedir);
        if (isAssetExt(rp)) {
            // For assets, return the path directly (ESM modules can't import assets yet)
            return rp;
        }
        return rp;
    };
})();

    // react-native polyfill — exposes the most-used RN APIs as a require()-able module.
    // Packages like expo-constants, expo-asset, expo-font, etc. import from 'react-native'.
    (function() {
        var _rnModule = {
            Platform: globalThis.Platform || { OS: 'web', Version: '1', select: function(obj) { return obj.web || obj.default; }, isPad: false, isTV: false },
            NativeModules: globalThis.NativeModules || {},
            TurboModuleRegistry: {
                get: function(name) { return null; },
                getEnforcing: function(name) { return {}; },
            },
            PixelRatio: { get: function() { return 1; }, getFontScale: function() { return 1; }, roundToNearestPixel: function(n) { return n; } },
            Dimensions: { get: function(dim) { return dim === 'window' ? { width: 0, height: 0, scale: 1, fontScale: 1 } : { width: 0, height: 0, scale: 1, fontScale: 1 }; }, addEventListener: function() { return { remove: function() {} }; } },
            StyleSheet: { create: function(s) { return s; }, flatten: function(s) { return s || {}; }, hairlineWidth: 1, absoluteFill: {} },
            AppRegistry: { registerComponent: function() {}, runApplication: function() {} },
            DeviceEventEmitter: { addListener: function() { return { remove: function() {} }; }, emit: function() {}, removeAllListeners: function() {} },
            NativeEventEmitter: function() { this.addListener = function() { return { remove: function() {} }; }; this.emit = function() {}; this.removeAllListeners = function() {}; },
            EventEmitter: function() { this.addListener = function() { return { remove: function() {} }; }; this.emit = function() {}; this.removeAllListeners = function() {}; },
            Alert: { alert: function() {} },
            Linking: { openURL: function() { return Promise.resolve(); }, canOpenURL: function() { return Promise.resolve(false); }, addEventListener: function() { return { remove: function() {} }; } },
            Animated: { Value: function(v) { this._value = v; }, timing: function() { return { start: function(cb) { cb && cb({finished:true}); } }; }, spring: function() { return { start: function(cb) { cb && cb({finished:true}); } }; }, View: 'View', Text: 'Text', Image: 'Image', createAnimatedComponent: function(c) { return c; } },
            View: function() {}, Text: function() {}, Image: function() {}, TextInput: function() {}, ScrollView: function() {}, FlatList: function() {}, SectionList: function() {}, TouchableOpacity: function() {}, TouchableHighlight: function() {}, Pressable: function() {}, Modal: function() {}, ActivityIndicator: function() {}, Switch: function() {}, Slider: function() {},
            findNodeHandle: function() { return null; },
            UIManager: { dispatchViewManagerCommand: function() {}, getViewManagerConfig: function() { return null; }, measure: function() {} },
            I18nManager: { isRTL: false, forceRTL: function() {}, swapLeftAndRightInRTL: function() {} },
            Keyboard: { dismiss: function() {}, addListener: function() { return { remove: function() {} }; } },
            BackHandler: { addEventListener: function() { return { remove: function() {} }; }, exitApp: function() {} },
            AccessibilityInfo: { addEventListener: function() { return { remove: function() {} }; }, isScreenReaderEnabled: function() { return Promise.resolve(false); } },
            InteractionManager: { runAfterInteractions: function(cb) { return typeof cb === 'function' ? cb() : cb && cb.gen && cb.gen(); }, createInteractionHandle: function() { return 0; }, clearInteractionHandle: function() {} },
            LayoutAnimation: { configureNext: function() {}, create: function() {}, Presets: {}, Types: {}, Properties: {} },
            Vibration: { vibrate: function() {}, cancel: function() {} },
            Share: { share: function() { return Promise.resolve({action:'dismissedAction'}); } },
            Clipboard: { getString: function() { return Promise.resolve(''); }, setString: function() {} },
            Settings: { get: function() { return null; }, set: function() {}, watchKeys: function() { return 0; }, clearWatch: function() {} },
            ToastAndroid: { show: function() {} },
            ActionSheetIOS: { showActionSheetWithOptions: function(opts, cb) { cb && cb(0); } },
        };
        _rnModule.__esModule = true;
        _rnModule.default = _rnModule;
        globalThis.__requireCache['react-native'] = _rnModule;
    })();

    // @react-native/assets-registry polyfill
    (function() {
        var _assets = [];
        var _registry = {
            registerAsset: function(asset) { return _assets.push(asset); },
            getAssetByID: function(id) { return _assets[id - 1]; },
        };
        globalThis.__requireCache['@react-native/assets-registry'] = _registry;
        globalThis.__requireCache['@react-native/assets-registry/registry'] = _registry;
    })();

    // expo-modules-core polyfill — register as CJS module
    // This provides ExpoModulesProxy, requireNativeModule, etc.
    (function() {
        // Track which modules are explicitly registered vs proxy-created
        var registeredModules = {};

        function getNativeModule(name) {
            // Only return explicitly registered modules
            if (registeredModules[name]) return registeredModules[name];
            return null;
        }

        function makeModuleProxy(base) {
            return new Proxy(base || {}, {
                get: function(t, p) {
                    if (p === '__esModule' || p === 'then') return undefined;
                    if (p in t) return t[p];
                    return function() { return undefined; };
                },
                set: function(t, p, v) { t[p] = v; return true; }
            });
        }

        function registerNativeModule(name, impl) {
            var mod = makeModuleProxy(impl);
            registeredModules[name] = mod;
            globalThis.ExpoModules[name] = mod;
            globalThis.NativeModules[name] = mod;
        }

        var expoModulesCore = {
            ExpoModulesProxy: globalThis.ExpoModules,
            NativeModulesProxy: globalThis.NativeModules,
            requireNativeModule: function(name) {
                var mod = getNativeModule(name);
                if (!mod) { mod = {}; registerNativeModule(name, mod); }
                return mod;
            },
            requireOptionalNativeModule: function(name) {
                // Return null for all optional modules: in web/server environments
                // native modules are absent. Callers must guard with if (mod) checks.
                // Returning a truthy proxy would make if(mod.someStringProp) true,
                // leading to incorrect JSON.parse calls etc.
                return null;
            },
        EventEmitter: function() { this.addListener = function() { return { remove: function() {} }; }; this.emit = function() {}; this.removeAllListeners = function() {}; },
        // NativeModule / SharedObject / SharedRef are base classes that Expo web modules extend.
        NativeModule: (function() { function NativeModule() {} return NativeModule; })(),
        SharedObject: (function() { function SharedObject() {} return SharedObject; })(),
        SharedRef: (function() { function SharedRef() {} return SharedRef; })(),
        ModuleNotFoundException: function(name) { this.name = 'ModuleNotFoundException'; this.message = name; },
        CodedError: function(code, message) { this.name = 'CodedError'; this.code = code; this.message = message; },
        UnavailabilityError: function(module, property) {
            this.name = 'UnavailabilityError';
            this.message = module + '.' + property + ' is not available on this platform.';
        },
        // registerWebModule: used by Expo web module implementations.
        // On the server side (no window), just return the factory result as-is.
        registerWebModule: function(factory, name) {
            try { return typeof factory === 'function' ? factory() : factory; } catch(e) { return {}; }
        },
        // Platform shim (matches globalThis.Platform)
        Platform: globalThis.Platform || { OS: 'web', select: function(o){return o.web||o.default;} },
        // uuid shim
        uuid: { v4: function() { return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g,function(c){var r=Math.random()*16|0;return(c=='x'?r:(r&0x3|0x8)).toString(16);}); }, v5: function(){return '';} },
    };
    expoModulesCore.ModuleNotFoundException.prototype = Object.create(Error.prototype);
    expoModulesCore.CodedError.prototype = Object.create(Error.prototype);
    expoModulesCore.UnavailabilityError.prototype = Object.create(Error.prototype);

    // Register under names that expo-modules-core might require
    globalThis.__requireCache['expo-modules-core'] = expoModulesCore;
    globalThis.__requireCache['@unimodules/core'] = expoModulesCore;
    globalThis.__requireCache['unimodules-core'] = expoModulesCore;
    globalThis.__requireCache['expo-core'] = expoModulesCore;

    // Register specific native modules that expo packages commonly require
    var commonModules = [
        'ExpoConstants', 'ExpoFileSystem', 'ExpoAsset', 'ExpoFont',
        'ExpoCrypto', 'ExpoRandom', 'ExpoSecureStore', 'ExpoSQLite',
        'ExpoWebBrowser', 'ExpoSplashScreen', 'ExpoUpdates',
        'ExpoApplication', 'ExpoDevice', 'ExpoNotifications',
        'ExpoCamera', 'ExpoMediaLibrary', 'ExpoImageManipulator',
        'ExpoBarCodeScanner', 'ExpoLocation', 'ExpoPermissions',
        'ExpoKeepAwake', 'ExpoHaptics', 'ExpoVideoPlayer',
        'ExpoStatusBar', 'ExpoLocalization', 'ExpoBrightness',
        'ExpoNetwork', 'ExpoScreenCapture', 'ExpoStoreReview'
    ];
    commonModules.forEach(function(name) {
        registerNativeModule(name, {});
    });
})();

}());

}());
    "#)?;

    // ── 3va:test — native mock / spy API (v2.0.0) ─────────────────────────────
    ctx.eval::<(), _>(r#"
(function() {
    // ── mock.fn ───────────────────────────────────────────────────────────────
    function createMockFn(impl_) {
        var calls = [];
        function spy() {
            var args = Array.prototype.slice.call(arguments);
            var result, error;
            try {
                result = impl_ ? impl_.apply(this, args) : undefined;
            } catch(e) {
                error = e;
                throw e;
            } finally {
                calls.push({ arguments: args, result: result, error: error });
            }
            return result;
        }
        spy.mock = {
            get calls() { return calls; },
            resetCalls: function() { calls = []; },
            restore: function() {}
        };
        spy.mockImplementation = function(fn) { impl_ = fn; return spy; };
        spy.mockReturnValue = function(v) { impl_ = function() { return v; }; return spy; };
        spy.mockResolvedValue = function(v) {
            impl_ = function() { return Promise.resolve(v); };
            return spy;
        };
        spy.mockRejectedValue = function(v) {
            impl_ = function() { return Promise.reject(v); };
            return spy;
        };
        return spy;
    }

    // ── mock.method ───────────────────────────────────────────────────────────
    function mockMethod(obj, methodName, impl_) {
        var original = obj[methodName];
        var spy = createMockFn(impl_ || original.bind(obj));
        spy.mock.restore = function() { obj[methodName] = original; };
        obj[methodName] = spy;
        return spy;
    }

    // ── mock.timers ───────────────────────────────────────────────────────────
    var _fakeTimersEnabled = false;
    var _fakeNow = 0;
    var _fakeQueue = [];
    var _realSetTimeout, _realClearTimeout, _realSetInterval, _realClearInterval;

    var fakeTimers = {
        enable: function(opts) {
            if (_fakeTimersEnabled) return;
            _fakeTimersEnabled = true;
            _fakeNow = (opts && opts.now != null) ? opts.now : Date.now();
            _fakeQueue = [];
            var _idCounter = 0;

            _realSetTimeout = globalThis.setTimeout;
            _realClearTimeout = globalThis.clearTimeout;
            _realSetInterval = globalThis.setInterval;
            _realClearInterval = globalThis.clearInterval;

            globalThis.setTimeout = function(fn, delay) {
                var id = ++_idCounter;
                _fakeQueue.push({ id: id, fn: fn, fireAt: _fakeNow + (delay || 0), repeat: false, interval: 0 });
                return id;
            };
            globalThis.clearTimeout = function(id) {
                _fakeQueue = _fakeQueue.filter(function(e) { return e.id !== id; });
            };
            globalThis.setInterval = function(fn, interval) {
                var id = ++_idCounter;
                _fakeQueue.push({ id: id, fn: fn, fireAt: _fakeNow + (interval || 0), repeat: true, interval: interval || 0 });
                return id;
            };
            globalThis.clearInterval = globalThis.clearTimeout;
        },

        tick: function(ms) {
            _fakeNow += ms;
            var toFire = _fakeQueue.filter(function(e) { return e.fireAt <= _fakeNow; });
            toFire.sort(function(a, b) { return a.fireAt - b.fireAt; });
            _fakeQueue = _fakeQueue.filter(function(e) { return e.fireAt > _fakeNow; });
            for (var i = 0; i < toFire.length; i++) {
                var e = toFire[i];
                try { e.fn(); } catch(ex) {}
                if (e.repeat) {
                    e.fireAt = _fakeNow + e.interval;
                    _fakeQueue.push(e);
                }
            }
        },

        reset: function() {
            if (!_fakeTimersEnabled) return;
            _fakeTimersEnabled = false;
            _fakeQueue = [];
            if (_realSetTimeout)   { globalThis.setTimeout   = _realSetTimeout; }
            if (_realClearTimeout) { globalThis.clearTimeout  = _realClearTimeout; }
            if (_realSetInterval)  { globalThis.setInterval   = _realSetInterval; }
            if (_realClearInterval){ globalThis.clearInterval = _realClearInterval; }
        }
    };

    var mock = {
        fn: createMockFn,
        method: mockMethod,
        timers: fakeTimers
    };

    // Expose as require('3va:test')
    var testModule = { mock: mock };
    globalThis.__requireCache['3va:test'] = testModule;
    globalThis.__requireCache['node:test'] = testModule; // alias for Node.js compat
}());

// ── BroadcastChannel ──────────────────────────────────────────────────────────
// In-process pub/sub across same-JS-context channels.
(function() {
    if (typeof globalThis.BroadcastChannel !== 'undefined') return;
    var _channels = Object.create(null);

    function BroadcastChannel(name) {
        this.name = String(name);
        this.onmessage = null;
        this.onmessageerror = null;
        this._closed = false;
        if (!_channels[this.name]) _channels[this.name] = [];
        _channels[this.name].push(this);
    }

    BroadcastChannel.prototype.postMessage = function(data) {
        if (this._closed) throw new DOMException('BroadcastChannel is closed', 'InvalidStateError');
        var name = this.name;
        var self = this;
        setTimeout(function() {
            var list = _channels[name] || [];
            for (var i = 0; i < list.length; i++) {
                if (list[i] === self || list[i]._closed) continue;
                var evt = { data: data, target: list[i], currentTarget: list[i],
                            type: 'message', origin: '', lastEventId: '' };
                if (typeof list[i].onmessage === 'function') list[i].onmessage(evt);
            }
        }, 0);
    };

    BroadcastChannel.prototype.close = function() {
        if (this._closed) return;
        this._closed = true;
        var list = _channels[this.name];
        if (list) {
            var idx = list.indexOf(this);
            if (idx !== -1) list.splice(idx, 1);
        }
    };

    BroadcastChannel.prototype.addEventListener = function(type, fn, opts) {
        if (type === 'message') this.onmessage = fn;
        else if (type === 'messageerror') this.onmessageerror = fn;
    };
    BroadcastChannel.prototype.removeEventListener = function(type, fn) {
        if (type === 'message' && this.onmessage === fn) this.onmessage = null;
        else if (type === 'messageerror' && this.onmessageerror === fn) this.onmessageerror = null;
    };
    BroadcastChannel.prototype.dispatchEvent = function() { return true; };

    globalThis.BroadcastChannel = BroadcastChannel;
}());

// ── EventSource ───────────────────────────────────────────────────────────────
// SSE is not natively supported; stub prevents import crashes.
(function() {
    if (typeof globalThis.EventSource !== 'undefined') return;

    function EventSource(url, opts) {
        this.url = String(url);
        this.readyState = 2; // CLOSED — no SSE support
        this.withCredentials = !!(opts && opts.withCredentials);
        this.onopen = null;
        this.onmessage = null;
        this.onerror = null;
    }
    EventSource.CONNECTING = 0;
    EventSource.OPEN = 1;
    EventSource.CLOSED = 2;
    EventSource.prototype.close = function() { this.readyState = 2; };
    EventSource.prototype.addEventListener = function(type, fn) {
        if (type === 'open') this.onopen = fn;
        else if (type === 'message') this.onmessage = fn;
        else if (type === 'error') this.onerror = fn;
    };
    EventSource.prototype.removeEventListener = function() {};
    EventSource.prototype.dispatchEvent = function() { return true; };

    globalThis.EventSource = EventSource;
}());

// ── navigator stubs ───────────────────────────────────────────────────────────
// Web Navigator APIs that browsers expose; not available in this runtime.
(function() {
    if (typeof globalThis.navigator === 'undefined') globalThis.navigator = {};
    var nav = globalThis.navigator;

    function _unsupported(name) {
        return { requestDevice: function() { return Promise.reject(new DOMException(name + ' not supported', 'NotSupportedError')); } };
    }

    if (!nav.serial)        nav.serial        = _unsupported('serial');
    if (!nav.usb)           nav.usb           = _unsupported('usb');
    if (!nav.bluetooth)     nav.bluetooth     = _unsupported('bluetooth');
    if (!nav.mediaDevices)  nav.mediaDevices  = {
        getUserMedia: function() { return Promise.reject(new DOMException('getUserMedia not supported', 'NotSupportedError')); },
        enumerateDevices: function() { return Promise.resolve([]); },
        getDisplayMedia: function() { return Promise.reject(new DOMException('getDisplayMedia not supported', 'NotSupportedError')); }
    };
    if (!nav.geolocation)   nav.geolocation   = {
        getCurrentPosition: function(ok, err) { if (err) err({ code: 2, message: 'Geolocation not supported' }); },
        watchPosition: function(ok, err) { if (err) err({ code: 2, message: 'Geolocation not supported' }); return 0; },
        clearWatch: function() {}
    };
}());
    "#)?;

    Ok(())
}

/// Split a bare specifier into (package_name, optional_subpath).
/// e.g. "es-errors/eval" → ("es-errors", Some("eval"))
///      "@scope/pkg"      → ("@scope/pkg", None)
///      "@scope/pkg/sub"  → ("@scope/pkg", Some("sub"))
pub fn split_bare_specifier(spec: &str) -> (&str, Option<&str>) {
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

/// Resolve an exports field value to a file path.
/// Handles:
/// - String: "./dist/index.js" → pkg_dir/dist/index.js
/// - Object (conditions): {"require":"./cjs/","import":"./esm/","default":"./dist/"}
/// - Array: ["./a.js", "./b.js"] — try each in order
/// - null: explicitly blocked (returns None)
///
/// `is_esm` controls condition priority per Node.js spec:
/// - CJS require(): "require" wins over "import"
/// - ESM import:   "import" wins over "require"
pub fn resolve_exports_value(
    val: &serde_json::Value,
    pkg_dir: &Path,
    is_esm: bool,
) -> Option<PathBuf> {
    match val {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => {
            let p = pkg_dir.join(s.trim_start_matches("./"));
            let resolved = resolve_file_path(&p);
            if resolved.is_file() {
                Some(resolved)
            } else {
                Some(p)
            }
        }
        serde_json::Value::Object(_) => {
            let conditions: &[&str] = if is_esm {
                &["import", "node", "default", "require", "module"]
            } else {
                &["require", "node", "default", "import", "module"]
            };
            for key in conditions {
                if let Some(child) = val.get(*key)
                    && let Some(path) = resolve_exports_value(child, pkg_dir, is_esm)
                {
                    return Some(path);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(path) = resolve_exports_value(item, pkg_dir, is_esm) {
                    return Some(path);
                }
            }
            None
        }
        _ => None,
    }
}

/// Try to match a subpath against pattern keys in the exports field.
/// Pattern keys contain `*` wildcards, e.g. "./features/*.js": "./src/features/*.js"
pub fn resolve_exports_pattern(
    exports: &serde_json::Map<String, serde_json::Value>,
    export_key: &str,
    pkg_dir: &Path,
) -> Option<Option<PathBuf>> {
    for (pattern, target) in exports {
        if !pattern.contains('*') {
            continue;
        }
        // Split pattern and export_key on *
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() != 2 {
            continue;
        }
        let (prefix, suffix) = (parts[0], parts[1]);
        if export_key.starts_with(prefix) && export_key.ends_with(suffix) {
            let mid = &export_key[prefix.len()..export_key.len() - suffix.len()];
            // Replace * in target with the matched portion
            let target_str = target.as_str()?;
            let replaced = target_str.replace('*', mid);
            let p = pkg_dir.join(replaced.trim_start_matches("./"));
            let resolved = resolve_file_path(&p);
            let result = if resolved.is_file() { resolved } else { p };
            return Some(Some(result));
        }
    }
    None
}

/// Resolve a module path relative to `basedir` (or CWD if None).
/// Returns Err with a Node.js-compatible error message when the path cannot be resolved.
pub fn resolve_path_from(
    path: &str,
    basedir: Option<&str>,
) -> std::result::Result<PathBuf, String> {
    resolve_path_from_inner(path, basedir, false)
}

/// Same as `resolve_path_from` but uses ESM condition priority in exports fields
/// ("import" wins over "require"). Call this from the ESM loader.
pub fn resolve_path_from_esm(
    path: &str,
    basedir: Option<&str>,
) -> std::result::Result<PathBuf, String> {
    resolve_path_from_inner(path, basedir, true)
}

fn resolve_path_from_inner(
    path: &str,
    basedir: Option<&str>,
    is_esm: bool,
) -> std::result::Result<PathBuf, String> {
    // Strip jsr: specifier prefix — treat as a scoped package in node_modules/
    let path = path.strip_prefix("jsr:").unwrap_or(path);

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let base_dir = basedir
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| cwd.clone());

    let raw = if path.starts_with("./") || path.starts_with("../") || path == "." || path == ".." {
        Ok(resolve_file_path(&base_dir.join(path)))
    } else if path.starts_with('/') || Path::new(path).is_absolute() {
        // `Path::is_absolute` additionally catches Windows absolute paths
        // (`C:/...`, `C:\...`, UNC) that would otherwise be misparsed as a
        // bare specifier with package name "C:".
        Ok(resolve_file_path(&PathBuf::from(path)))
    } else {
        // Bare specifier: may have a subpath like 'pkg/subpath' or '@scope/pkg/sub'
        let (pkg_name, subpath) = split_bare_specifier(path);
        resolve_node_module_from(&base_dir, &cwd, pkg_name, subpath, is_esm)
    }?;

    // Normalize to remove redundant `.` and `..` components so that the same
    // file always gets the same cache key regardless of how __dirname accumulated
    // extra `/.` segments during nested require() calls.
    Ok(normalize_path(&raw))
}

/// Normalize a resolved module path by collapsing `.` and `..` components
/// WITHOUT following symlinks (matching Node.js behaviour: symlinks in
/// node_modules are kept as-is so that package deduplication / hoisting works
/// correctly — resolving them would make every package see its own nested
/// copy of shared deps instead of the hoisted top-level copy).
fn normalize_path(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    // If the file doesn't exist we still return the normalized path so the
    // caller can emit a clear ENOENT rather than silently succeeding.
    out
}

/// Convenience wrapper using CWD as the base.
pub fn resolve_path(path: &str) -> std::result::Result<PathBuf, String> {
    resolve_path_from(path, None)
}

/// Try path as-is, then with common extensions appended, then index files.
fn resolve_file_path(base: &Path) -> PathBuf {
    if base.is_file() {
        return base.to_path_buf();
    }
    // If the path is a directory with a package.json, resolve its entry point
    if base.is_dir() && base.join("package.json").is_file() {
        let entry = resolve_node_module_entry(base, false);
        if entry.is_file() {
            return entry;
        }
    }
    // Append extensions as strings to avoid PathBuf::with_extension clobbering
    // filenames that already contain dots (e.g. "Reflect.getPrototypeOf").
    // Web/platform-specific variants (.web.*) are tried first so Expo/React Native
    // packages resolve to their web implementations rather than native ones.
    let base_str = base.to_string_lossy();
    for ext in &[
        "web.js", "web.tsx", "web.ts", "web.mjs", "js", "tsx", "ts", "mjs", "cjs",
    ] {
        let p = PathBuf::from(format!("{}.{}", base_str, ext));
        if p.is_file() {
            return p;
        }
    }
    for index in &[
        "index.web.js",
        "index.web.tsx",
        "index.web.ts",
        "index.js",
        "index.tsx",
        "index.ts",
    ] {
        let p = base.join(index);
        if p.is_file() {
            return p;
        }
    }
    base.to_path_buf()
}

/// Walk up from `start` toward `root` looking for node_modules/<name>.
/// If `subpath` is Some, resolve that file within the package dir, checking
/// the exports field when present.
fn resolve_node_module_from(
    start: &Path,
    root: &Path,
    name: &str,
    subpath: Option<&str>,
    is_esm: bool,
) -> std::result::Result<PathBuf, String> {
    let mut dir = start.to_path_buf();
    let mut visited = std::collections::HashSet::new();
    loop {
        let pkg_dir = dir.join("node_modules").join(name);
        if pkg_dir.is_dir() && visited.insert(pkg_dir.clone()) {
            match resolve_in_pkg_dir(&pkg_dir, subpath, is_esm) {
                Ok(p) if p.is_file() => return Ok(p),
                // Empty/broken package directory — skip it and continue walking up.
                // Prevents EISDIR errors from stale nested node_modules dirs.
                _ => {}
            }
        }
        if dir == *root || !dir.pop() {
            break;
        }
    }
    // Final fallback: root/node_modules/<name>
    let pkg_dir = root.join("node_modules").join(name);
    match resolve_in_pkg_dir(&pkg_dir, subpath, is_esm) {
        Ok(p) if p.is_file() => Ok(p),
        Ok(_) => Err(format!(
            "ENOENT: Package directory '{}' exists but has no valid entry file",
            pkg_dir.display()
        )),
        Err(e) => Err(e),
    }
}

fn resolve_in_pkg_dir(
    pkg_dir: &Path,
    subpath: Option<&str>,
    is_esm: bool,
) -> std::result::Result<PathBuf, String> {
    // Read package.json if it exists
    let pkg_json_content = std::fs::read_to_string(pkg_dir.join("package.json")).ok();
    let pkg_json =
        pkg_json_content.and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());

    if let Some(sub) = subpath {
        if let Some(ref json) = pkg_json
            && let Some(exports) = json.get("exports")
        {
            // Normalize: "./subpath" if bare "subpath"
            let export_key = if sub.starts_with("./") {
                sub.to_string()
            } else {
                format!("./{}", sub)
            };

            // Try exact match first
            if let Some(val) = exports.get(&export_key)
                && let Some(path) = resolve_exports_value(val, pkg_dir, is_esm)
            {
                return Ok(path);
            }

            // Check for null value → explicitly blocked
            if exports.get(&export_key).is_some_and(|v| v.is_null()) {
                return Err(format!(
                    "ERR_PACKAGE_PATH_NOT_EXPORTED: Package \"{}\" contains \"{}\" in its exports but it is null",
                    pkg_dir.display(),
                    export_key
                ));
            }

            // Try pattern match against keys containing '*'
            if exports.is_object()
                && let Some(Some(result)) =
                    resolve_exports_pattern(exports.as_object().unwrap(), &export_key, pkg_dir)
            {
                return Ok(result);
            }

            // Exports field exists → no fallthrough to main/file resolution
            return Err(format!(
                "ERR_PACKAGE_PATH_NOT_EXPORTED: Package \"{}\" does not define an exports entry for \"{}\"",
                pkg_dir.display(),
                export_key
            ));
        }

        // No exports field — check browser field mapping
        if let Ok(content) = std::fs::read_to_string(pkg_dir.join("package.json"))
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(browser) = json.get("browser").and_then(|b| b.as_object())
        {
            let sub_normalized = sub.strip_prefix("./").unwrap_or(sub);
            for (key, val) in browser {
                let key_normalized = key.strip_prefix("./").unwrap_or(key.as_str());
                if key_normalized == sub_normalized
                    || key_normalized == sub
                    || format!("./{key_normalized}") == sub
                {
                    if let Some(replacement) = val.as_str() {
                        let rp = pkg_dir.join(replacement.trim_start_matches("./"));
                        let resolved = resolve_file_path(&rp);
                        if resolved.is_file() {
                            return Ok(resolved);
                        }
                    }
                    break;
                }
            }
        }

        // No exports field — try file resolution
        let p = pkg_dir.join(sub);
        let resolved = resolve_file_path(&p);
        if resolved.is_file() {
            return Ok(resolved);
        }
        // If the path is a directory, look for its package.json entry point
        // (common for packages like @swc/helpers/_/_interop_require_default/)
        if p.is_dir() {
            let entry = resolve_node_module_entry(&p, is_esm);
            if entry.is_file() {
                return Ok(entry);
            }
        }
        Ok(p)
    } else {
        // No subpath — resolve entry point
        Ok(resolve_node_module_entry(pkg_dir, is_esm))
    }
}

/// Given an already-located package directory, find its entry point.
/// `is_esm` selects export condition priority (import > require for ESM).
fn resolve_node_module_entry(pkg_dir: &Path, is_esm: bool) -> PathBuf {
    let pkg_json_path = pkg_dir.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_json_path)
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
    {
        if let Some(exports) = json.get("exports") {
            // Simple string form: "exports": "./dist/index.js"
            if let Some(path_str) = exports.as_str() {
                let p = pkg_dir.join(path_str.trim_start_matches("./"));
                let resolved = resolve_file_path(&p);
                if resolved.is_file() {
                    return resolved;
                }
            }
            // Standard form: exports["."] is the entry condition map
            if let Some(exp_dot) = exports.get(".")
                && let Some(p) = resolve_exports_value(exp_dot, pkg_dir, is_esm)
                && p.is_file()
            {
                return p;
            }
            // Shorthand form: exports itself IS the condition map (no "." key)
            if exports.is_object()
                && !exports.as_object().is_some_and(|m| m.contains_key("."))
                && let Some(p) = resolve_exports_value(exports, pkg_dir, is_esm)
                && p.is_file()
            {
                return p;
            }
        }
        // react-native field: platform-specific entry point override
        if let Some(rn) = json.get("react-native")
            && let Some(rn_str) = rn.as_str()
        {
            let rn_path = pkg_dir.join(rn_str.trim_start_matches("./"));
            let resolved = resolve_file_path(&rn_path);
            if resolved.is_file() {
                return resolved;
            }
        }
        // Fallback to "main" (standard Node.js entry)
        if let Some(main) = json["main"].as_str() {
            let main_path = pkg_dir.join(main);
            let resolved = resolve_file_path(&main_path);
            if resolved.is_file() {
                return resolved;
            }
        }
        // browser field: bundler hint — string form replaces the main entry
        if let Some(browser) = json.get("browser")
            && let Some(browser_str) = browser.as_str()
        {
            let browser_path = pkg_dir.join(browser_str.trim_start_matches("./"));
            let resolved = resolve_file_path(&browser_path);
            if resolved.is_file() {
                return resolved;
            }
        }
    }
    // Fallback: index.js / index.ts (direct check to avoid recursion with resolve_file_path)
    let idx = pkg_dir.join("index.js");
    if idx.is_file() {
        return idx;
    }
    let idx_ts = pkg_dir.join("index.ts");
    if idx_ts.is_file() {
        return idx_ts;
    }
    pkg_dir.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::resolve_exports_value;
    use serde_json::json;
    use std::path::Path;

    // ── Bug 2: resolve_exports_value condition priority ───────────────────────

    #[test]
    fn cjs_mode_prefers_require_over_import() {
        let val = json!({ "require": "./cjs.js", "import": "./esm.js" });
        let p = resolve_exports_value(&val, Path::new("/pkg"), false).unwrap();
        assert!(p.to_string_lossy().ends_with("cjs.js"), "got {:?}", p);
    }

    #[test]
    fn esm_mode_prefers_import_over_require() {
        let val = json!({ "require": "./cjs.js", "import": "./esm.js" });
        let p = resolve_exports_value(&val, Path::new("/pkg"), true).unwrap();
        assert!(p.to_string_lossy().ends_with("esm.js"), "got {:?}", p);
    }

    #[test]
    fn null_export_returns_none_in_both_modes() {
        let val = serde_json::Value::Null;
        assert!(resolve_exports_value(&val, Path::new("/pkg"), false).is_none());
        assert!(resolve_exports_value(&val, Path::new("/pkg"), true).is_none());
    }

    #[test]
    fn string_export_resolves_in_both_modes() {
        let val = json!("./dist/index.js");
        let cjs = resolve_exports_value(&val, Path::new("/pkg"), false).unwrap();
        let esm = resolve_exports_value(&val, Path::new("/pkg"), true).unwrap();
        assert_eq!(cjs, esm);
        assert!(cjs.to_string_lossy().ends_with("index.js"));
    }

    #[test]
    fn fallback_array_tries_in_order() {
        // Array: first non-null string wins regardless of mode.
        let val = json!(["./a.js", "./b.js"]);
        let p = resolve_exports_value(&val, Path::new("/pkg"), false).unwrap();
        assert!(p.to_string_lossy().ends_with("a.js"), "got {:?}", p);
    }

    #[test]
    fn default_condition_used_when_no_specific_match() {
        let val = json!({ "default": "./fallback.js" });
        let cjs = resolve_exports_value(&val, Path::new("/pkg"), false).unwrap();
        let esm = resolve_exports_value(&val, Path::new("/pkg"), true).unwrap();
        assert!(cjs.to_string_lossy().ends_with("fallback.js"));
        assert!(esm.to_string_lossy().ends_with("fallback.js"));
    }

    #[test]
    fn node_condition_beats_default_in_both_modes() {
        let val = json!({ "node": "./node.js", "default": "./default.js" });
        let cjs = resolve_exports_value(&val, Path::new("/pkg"), false).unwrap();
        let esm = resolve_exports_value(&val, Path::new("/pkg"), true).unwrap();
        assert!(
            cjs.to_string_lossy().ends_with("node.js"),
            "cjs got {:?}",
            cjs
        );
        assert!(
            esm.to_string_lossy().ends_with("node.js"),
            "esm got {:?}",
            esm
        );
    }

    #[test]
    fn nested_conditions_resolved_recursively() {
        // "import" → { "node": "./esm-node.js" } — should resolve the nested object.
        let val = json!({ "import": { "node": "./esm-node.js" }, "require": "./cjs.js" });
        let p = resolve_exports_value(&val, Path::new("/pkg"), true).unwrap();
        assert!(p.to_string_lossy().ends_with("esm-node.js"), "got {:?}", p);
    }
}
