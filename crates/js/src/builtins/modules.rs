use rquickjs::{Ctx, Function, Result, function::Rest};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use vvva_permissions::{Capability, PermissionState};

/// Inject CommonJS `require()`, `module`, `exports`, `__filename`, `__dirname` globals.
///
/// Strategy: inject a native `__readFile(path) -> String` function that handles
/// permission checks and file I/O. The JS-side `require()` wrapper handles the
/// module caching, wrapping, and evaluation — avoiding rquickjs `Value<'js>` lifetime
/// issues in closures.
pub fn inject_require(ctx: &Ctx, permissions: Rc<RefCell<PermissionState>>) -> Result<()> {
    let globals = ctx.globals();

    // Initialize module cache and CommonJS globals
    ctx.eval::<(), _>(
        r#"
        globalThis.__requireCache = {};
        globalThis.module = { exports: {} };
        globalThis.exports = globalThis.module.exports;
        globalThis.__filename = '';
        globalThis.__dirname = '';
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
        {
            let p = perms.borrow();
            if !p.check(&Capability::FileRead(full_path.clone())) {
                return Err(rquickjs::Error::new_from_js(
                    "permission",
                    "permission denied",
                ));
            }
        }

        // Read the file
        let source = std::fs::read_to_string(&full_path)
            .map_err(|_| rquickjs::Error::new_from_js("io", "file not found"))?;

        // Transpile if TypeScript
        let source = if path_str.ends_with(".ts") || path_str.ends_with(".tsx") {
            crate::transpiler::transpile(&source)
        } else {
            source
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
                URL: typeof URL !== 'undefined' ? URL : function(u) { this.href = u; },
                URLSearchParams: typeof URLSearchParams !== 'undefined' ? URLSearchParams : function() {}
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

            // ── http / https (minimal stubs) ──────────────────────────────────────
            var httpStub = {
                request: function(opts, cb) { var r = new EventEmitter(); r.end = function() {}; r.write = function() {}; return r; },
                get: function(url, cb) { return httpStub.request(url, cb); },
                STATUS_CODES: { 200: 'OK', 404: 'Not Found', 500: 'Internal Server Error' },
                createServer: function() { return { listen: function() {} }; },
                Agent: function() {}
            };
            globalThis.__requireCache['http'] = httpStub;
            globalThis.__requireCache['https'] = httpStub;
            globalThis.__requireCache['node:http'] = httpStub;
            globalThis.__requireCache['node:https'] = httpStub;

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

            // ── zlib (minimal stub) ───────────────────────────────────────────────
            var zlib = {
                gzip: function(buf, cb) { cb(null, buf); },
                gunzip: function(buf, cb) { cb(null, buf); },
                inflate: function(buf, cb) { cb(null, buf); },
                deflate: function(buf, cb) { cb(null, buf); },
                createGzip: function() { return new Transform(); },
                createGunzip: function() { return new Transform(); },
                createDeflate: function() { return new Transform(); },
                createInflate: function() { return new Transform(); },
                constants: {}
            };
            globalThis.__requireCache['zlib'] = zlib;
            globalThis.__requireCache['node:zlib'] = zlib;

            // ── net / tls (stubs) ─────────────────────────────────────────────────
            var netStub = { createConnection: function() { return new EventEmitter(); }, createServer: function() { return { listen: function() {} }; }, Socket: EventEmitter };
            globalThis.__requireCache['net'] = netStub;
            globalThis.__requireCache['tls'] = netStub;
            globalThis.__requireCache['node:net'] = netStub;
            globalThis.__requireCache['node:tls'] = netStub;

            // ── child_process (stub) ──────────────────────────────────────────────
            globalThis.__requireCache['child_process'] = { exec: function(cmd, cb) { if (cb) cb(null, '', ''); }, spawn: function() { return new EventEmitter(); }, execSync: function() { return ''; } };

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

            // ── http2 (stub) ──────────────────────────────────────────────────────
            globalThis.__requireCache['http2'] = { connect: function() { return new EventEmitter(); }, constants: {}, createServer: function() { return { listen: function() {} }; } };
            globalThis.__requireCache['node:http2'] = globalThis.__requireCache['http2'];

            // ── debug (common logging utility) ───────────────────────────────────
            globalThis.__requireCache['debug'] = function(ns) { return function() {}; };
            globalThis.__requireCache['ms'] = function(val) { return typeof val === 'string' ? 0 : val; };

            // ── proxy-from-env ───────────────────────────────────────────────────
            globalThis.__requireCache['proxy-from-env'] = { getProxyForUrl: function() { return null; } };
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
            if (/^\s*(import\s|import\{|export\s|export\{|export\s*default)/.test(source)) {
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
fn resolve_file_path(base: &PathBuf) -> PathBuf {
    if base.is_file() {
        return base.clone();
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
    base.clone()
}

/// Walk up from `start` toward `root` looking for node_modules/<name>.
/// If `subpath` is Some, resolve that file within the package dir instead of using the entry point.
fn resolve_node_module_from(
    start: &PathBuf,
    root: &PathBuf,
    name: &str,
    subpath: Option<&str>,
) -> PathBuf {
    let mut dir = start.clone();
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

fn resolve_in_pkg_dir(pkg_dir: &PathBuf, subpath: Option<&str>) -> PathBuf {
    if let Some(sub) = subpath {
        let p = pkg_dir.join(sub);
        let resolved = resolve_file_path(&p);
        if resolved.is_file() {
            return resolved;
        }
        p
    } else {
        let resolved = resolve_node_module_entry(pkg_dir);
        if resolved.is_file() {
            return resolved;
        }
        pkg_dir.clone()
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
fn resolve_node_module_entry(pkg_dir: &PathBuf) -> PathBuf {
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

/// Resolve a bare module specifier from node_modules, reading package.json for entry point.
fn resolve_node_module(cwd: &PathBuf, name: &str) -> PathBuf {
    let (pkg_name, subpath) = split_bare_specifier(name);
    let pkg_dir = cwd.join("node_modules").join(pkg_name);
    if pkg_dir.is_dir() {
        resolve_in_pkg_dir(&pkg_dir, subpath)
    } else {
        pkg_dir
    }
}
