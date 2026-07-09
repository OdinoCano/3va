use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use v8::{
    ContextScope, Function, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue, Script,
    String as V8String,
};
use vvva_permissions::{Capability, PermissionState};

fn dns_resolver() -> std::result::Result<&'static hickory_resolver::TokioResolver, String> {
    static RESOLVER: OnceLock<std::result::Result<hickory_resolver::TokioResolver, String>> =
        OnceLock::new();
    RESOLVER
        .get_or_init(|| {
            hickory_resolver::TokioResolver::builder_tokio()
                .and_then(|b| b.build())
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| e.clone())
}

fn dns_lookup_to_json(
    rrtype: &str,
    answers: &[hickory_resolver::proto::rr::Record],
) -> serde_json::Value {
    use hickory_resolver::proto::rr::RData;
    use serde_json::json;

    match rrtype {
        "MX" => serde_json::Value::Array(
            answers
                .iter()
                .filter_map(|r| match &r.data {
                    RData::MX(mx) => Some(json!({
                        "exchange": mx.exchange.to_string().trim_end_matches('.'),
                        "priority": mx.preference,
                    })),
                    _ => None,
                })
                .collect(),
        ),
        "TXT" => serde_json::Value::Array(
            answers
                .iter()
                .filter_map(|r| match &r.data {
                    RData::TXT(txt) => Some(json!(
                        txt.txt_data
                            .iter()
                            .map(|chunk| std::string::String::from_utf8_lossy(chunk).into_owned())
                            .collect::<Vec<_>>()
                    )),
                    _ => None,
                })
                .collect(),
        ),
        "SRV" => serde_json::Value::Array(
            answers
                .iter()
                .filter_map(|r| match &r.data {
                    RData::SRV(srv) => Some(json!({
                        "priority": srv.priority,
                        "weight": srv.weight,
                        "port": srv.port,
                        "name": srv.target.to_string().trim_end_matches('.'),
                    })),
                    _ => None,
                })
                .collect(),
        ),
        "NS" => serde_json::Value::Array(
            answers
                .iter()
                .filter_map(|r| match &r.data {
                    RData::NS(ns) => Some(json!(ns.0.to_string().trim_end_matches('.'))),
                    _ => None,
                })
                .collect(),
        ),
        "CNAME" => serde_json::Value::Array(
            answers
                .iter()
                .filter_map(|r| match &r.data {
                    RData::CNAME(cname) => Some(json!(cname.0.to_string().trim_end_matches('.'))),
                    _ => None,
                })
                .collect(),
        ),
        "NAPTR" => serde_json::Value::Array(
            answers
                .iter()
                .filter_map(|r| match &r.data {
                    RData::NAPTR(n) => Some(json!({
                        "flags": std::string::String::from_utf8_lossy(&n.flags),
                        "service": std::string::String::from_utf8_lossy(&n.services),
                        "regexp": std::string::String::from_utf8_lossy(&n.regexp),
                        "replacement": n.replacement.to_string().trim_end_matches('.'),
                        "order": n.order,
                        "preference": n.preference,
                    })),
                    _ => None,
                })
                .collect(),
        ),
        "PTR" => serde_json::Value::Array(
            answers
                .iter()
                .filter_map(|r| match &r.data {
                    RData::PTR(ptr) => Some(json!(ptr.0.to_string().trim_end_matches('.'))),
                    _ => None,
                })
                .collect(),
        ),
        "SOA" => answers
            .iter()
            .find_map(|r| match &r.data {
                RData::SOA(soa) => Some(json!({
                    "nsname": soa.mname.to_string().trim_end_matches('.'),
                    "hostmaster": soa.rname.to_string().trim_end_matches('.'),
                    "serial": soa.serial,
                    "refresh": soa.refresh,
                    "retry": soa.retry,
                    "expire": soa.expire,
                    "minttl": soa.minimum,
                })),
                _ => None,
            })
            .unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Array(vec![]),
    }
}

static INJECT_MODULES_PERMISSIONS: std::sync::OnceLock<Arc<PermissionState>> =
    std::sync::OnceLock::new();
fn permissions() -> &'static Arc<PermissionState> {
    INJECT_MODULES_PERMISSIONS.get().unwrap()
}

pub fn inject_require(
    scope: &mut ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    INJECT_MODULES_PERMISSIONS.set(permissions_param).ok();
    let context = scope.get_current_context();
    let global = context.global(scope);

    let source_code = r#"
        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__fallbackModules = globalThis.__fallbackModules || {};
        globalThis.module = { exports: {} };
        globalThis.exports = globalThis.module.exports;
        globalThis.__filename = '';
        globalThis.__dirname = '';
        globalThis.__DEV__ = false;

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

        if (typeof ArrayBuffer !== 'undefined' && !('resizable' in ArrayBuffer.prototype)) {
            Object.defineProperty(ArrayBuffer.prototype, 'resizable', {
                get: function() { return false; },
                enumerable: false, configurable: true
            });
        }
        if (typeof SharedArrayBuffer !== 'undefined' && !('growable' in SharedArrayBuffer.prototype)) {
            Object.defineProperty(SharedArrayBuffer.prototype, 'growable', {
                get: function() { return false; },
                enumerable: false, configurable: true
            });
        }
    "#;
    let source = V8String::new(scope, source_code).unwrap();
    let _ = Script::compile(scope, source, None).and_then(|s| s.run(scope));

    let read_file_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let path_arg = args.get(0);
            let path_str = path_arg.to_rust_string_lossy(scope);

            let full_path = PathBuf::from(&path_str);

            if !permissions().check(&Capability::FileRead(full_path.clone())) {
                let msg = format!(
                    "Permission denied: --allow-read={} is required",
                    full_path.display()
                );
                let err_str = V8String::new(scope, &msg).unwrap();
                rv.set(err_str.into());
                return;
            }

            match std::fs::read_to_string(&full_path) {
                Ok(source) => {
                    let transpiled = if path_str.ends_with(".tsx") || path_str.ends_with(".jsx") {
                        crate::transpiler::transpile_jsx(&source)
                    } else if path_str.ends_with(".ts")
                        || path_str.ends_with(".mts")
                        || path_str.ends_with(".cts")
                    {
                        crate::transpiler::transpile(&source)
                    } else if path_str.ends_with(".cjs")
                        || path_str.ends_with(".json")
                        || source.contains("@exodus/bytes")
                    {
                        source
                    } else {
                        crate::transpiler::transpile_js(&source)
                    };
                    let result = V8String::new(scope, &transpiled).unwrap();
                    rv.set(result.into());
                }
                Err(e) => {
                    let msg = format!("ENOENT: {}: '{}'", e, path_str);
                    let err_str = V8String::new(scope, &msg).unwrap();
                    let err = v8::Exception::error(scope, err_str);
                    scope.throw_exception(err);
                }
            }
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__readFile").unwrap().into(),
        read_file_fn.unwrap().into(),
    );

    let require_resolve_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let specifier_arg = args.get(0);
            let specifier = specifier_arg.to_rust_string_lossy(scope);
            let dir_arg = args.get(1);
            let dir = if dir_arg.is_undefined() {
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                dir_arg.to_rust_string_lossy(scope)
            };

            let resolved = crate::esm::resolve_esm_from_dir(&dir, &specifier);
            if !resolved.is_file() {
                let msg = format!("Cannot find module '{}' imported from '{}'", specifier, dir);
                let err_str = V8String::new(scope, &msg).unwrap();
                let err = v8::Exception::error(scope, err_str);
                scope.throw_exception(err);
                return;
            }
            let result = V8String::new(scope, &resolved.to_string_lossy()).unwrap();
            rv.set(result.into());
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__requireResolve").unwrap().into(),
        require_resolve_fn.unwrap().into(),
    );

    let resolve_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let path_arg = args.get(0);
            let path_str = path_arg.to_rust_string_lossy(scope);
            let basedir_arg = args.get(1);
            let basedir = if basedir_arg.is_undefined() {
                None
            } else {
                Some(basedir_arg.to_rust_string_lossy(scope))
            };

            match resolve_path_from(&path_str, basedir.as_deref()) {
                Ok(p) => {
                    let result = V8String::new(scope, &p.to_string_lossy()).unwrap();
                    rv.set(result.into());
                }
                Err(msg) => {
                    let err_str = V8String::new(scope, &msg).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__resolvePath").unwrap().into(),
        resolve_fn.unwrap().into(),
    );

    let dns_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let hostname_arg = args.get(0);
            let hostname = hostname_arg.to_rust_string_lossy(scope);

            let result = tokio::task::block_in_place(|| {
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
            });

            let json_result = match result {
                Ok(ips) => serde_json::to_string(&ips).unwrap_or_else(|_| "[]".to_string()),
                Err(e) => {
                    let err_str = V8String::new(scope, &format!("Error: {}", e)).unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };
            let result_str = V8String::new(scope, &json_result).unwrap();
            rv.set(result_str.into());
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__dnsLookup").unwrap().into(),
        dns_fn.unwrap().into(),
    );

    let dns_query_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let hostname_arg = args.get(0);
            let hostname = hostname_arg.to_rust_string_lossy(scope);
            let rrtype_arg = args.get(1);
            let rrtype = rrtype_arg.to_rust_string_lossy(scope);

            use hickory_resolver::proto::rr::RecordType;

            let record_type = match rrtype.as_str() {
                "MX" => RecordType::MX,
                "TXT" => RecordType::TXT,
                "SRV" => RecordType::SRV,
                "NS" => RecordType::NS,
                "CNAME" => RecordType::CNAME,
                "NAPTR" => RecordType::NAPTR,
                "SOA" => RecordType::SOA,
                "PTR" => RecordType::PTR,
                other => {
                    let err_str =
                        V8String::new(scope, &format!("unsupported record type: {}", other))
                            .unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            let resolver = match dns_resolver() {
                Ok(r) => r,
                Err(e) => {
                    let err_str = V8String::new(scope, &e).unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            let result: Result<hickory_resolver::lookup::Lookup, String> =
                match tokio::runtime::Handle::try_current() {
                    Ok(runtime) => runtime.block_on(async {
                        if record_type == RecordType::PTR {
                            let ip: std::net::IpAddr = match hostname.parse() {
                                Ok(ip) => ip,
                                Err(_) => {
                                    return Err(format!("EINVAL: not an IP address: {}", hostname));
                                }
                            };
                            resolver.reverse_lookup(ip).await.map_err(|e| e.to_string())
                        } else {
                            resolver
                                .lookup(hostname.clone(), record_type)
                                .await
                                .map_err(|e| e.to_string())
                        }
                    }),
                    Err(_) => Err("no runtime".to_string()),
                };

            match result {
                Ok(lookup) => {
                    let json = dns_lookup_to_json(&rrtype, lookup.answers());
                    let json_str =
                        serde_json::to_string(&json).unwrap_or_else(|_| "null".to_string());
                    let result_str = V8String::new(scope, &json_str).unwrap();
                    rv.set(result_str.into());
                }
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("ENOTFOUND {}: {}", hostname, e)).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__dnsQuery").unwrap().into(),
        dns_query_fn.unwrap().into(),
    );

    let js_code = r#"
        (function() {
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
                },
                stripVTControlCharacters: function(str) {
                    return String(str).replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '');
                }
            };
            globalThis.__requireCache['util'] = util;

            function EventEmitter() { this._events = Object.create(null); this._maxListeners = 10; }
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
            EventEmitter.once = function(emitter, ev) {
                return new Promise(function(resolve, reject) {
                    function onErr(e) { emitter.removeListener(ev, onEvt); reject(e); }
                    function onEvt() { emitter.removeListener('error', onErr); resolve(Array.prototype.slice.call(arguments)); }
                    emitter.once(ev, onEvt);
                    if (ev !== 'error') emitter.once('error', onErr);
                });
            };
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

            function Stream() { EventEmitter.call(this); }
            util.inherits(Stream, EventEmitter);
            Stream.prototype.pipe = function() { return this; };
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

            function PassThrough(opts) { Transform.call(this, opts); }
            util.inherits(PassThrough, Transform);
            PassThrough.prototype._transform = function(chunk, enc, cb) { this.push(chunk); cb(); };

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
            Duplex.prototype._write = Writable.prototype._write;
            Duplex.prototype._final = Writable.prototype._final;
            Duplex.prototype.write = Writable.prototype.write;
            Duplex.prototype.end = Writable.prototype.end;
            Duplex.prototype.cork = Writable.prototype.cork;
            Duplex.prototype.uncork = Writable.prototype.uncork;
            Duplex.prototype.setDefaultEncoding = Writable.prototype.setDefaultEncoding;
            Duplex.prototype.destroy = Writable.prototype.destroy;
            Duplex.prototype._destroy = Writable.prototype._destroy;

            globalThis.__requireCache['stream'] = { Readable: Readable, Writable: Writable, Transform: Transform, PassThrough: PassThrough, Duplex: Duplex, Stream: Stream };

            globalThis.__requireCache['path'] = {
                resolve: function() {
                    var args = Array.prototype.slice.call(arguments);
                    if (!args.length) return process.cwd ? process.cwd() : '/';
                    var result = args.reduce(function(acc, seg) {
                        if (!seg) return acc;
                        if (seg.charAt(0) === '/') return seg;
                        return acc ? acc.replace(/\/*$/, '/') + seg : seg;
                    }, '');
                    return result || '/';
                },
                join: function() {
                    return Array.prototype.slice.call(arguments).join('/').replace(/\/+/g, '/');
                },
                dirname: function(p) {
                    if (!p) return '.';
                    var sep = p.indexOf('/') !== -1 ? '/' : '\\';
                    var parts = p.split(sep);
                    if (parts.length === 1) return '.';
                    parts.pop();
                    return parts.join(sep) || (sep === '/' ? '/' : '.');
                },
                basename: function(p, ext) {
                    if (!p) return '.';
                    var sep = p.indexOf('/') !== -1 ? '/' : '\\';
                    var parts = p.split(sep);
                    var last = parts[parts.length - 1];
                    if (ext && last.endsWith(ext)) return last.slice(0, -ext.length);
                    return last || '';
                },
                extname: function(p) {
                    if (!p) return '';
                    var last = p.split(/[\/\\]/).pop() || '';
                    var idx = last.lastIndexOf('.');
                    return idx <= 0 ? '' : last.slice(idx);
                },
                isAbsolute: function(p) { return p && (p.charAt(0) === '/' || p.match(/^[A-Za-z]:/)); },
                normalize: function(p) { return p.replace(/\/+/g, '/').replace(/\/$/, '') || '/'; },
                relative: function(from, to) {
                    var fromParts = from.split('/').filter(Boolean);
                    var toParts = to.split('/').filter(Boolean);
                    var i = 0;
                    while (i < fromParts.length && i < toParts.length && fromParts[i] === toParts[i]) i++;
                    return '../'.repeat(fromParts.length - i) + toParts.slice(i).join('/');
                },
                join: function() { return this.normalize(Array.prototype.slice.call(arguments).join('/')); },
                resolve: function() {
                    var args = Array.prototype.slice.call(arguments);
                    var result = args.reduce(function(acc, seg) {
                        if (!seg) return acc;
                        if (seg.charAt(0) === '/') return seg;
                        return acc ? acc.replace(/\/*$/, '/') + seg : seg;
                    }, '');
                    return result || '.';
                },
            };

            globalThis.__requireCache['buffer'] = globalThis.Buffer;
            globalThis.__requireCache['stream'] = globalThis.__requireCache['stream'] || Stream;

            // ── net / tls — raw TCP sockets, backed by the native __tcp*/__net* ────
            // bindings in builtins/tcp.rs. Those bindings return either a number
            // (success) or an Error/string (failure) rather than throwing, so every
            // call site below checks the type of the result instead of using try/catch.
            function _netErr(v) { return v instanceof Error ? v : new Error(String(v)); }

            function Socket(opts) {
                if (!(this instanceof Socket)) return new Socket(opts);
                opts = opts || {};
                Duplex.call(this, opts);
                this._connId = (typeof opts.fd === 'number') ? opts.fd : null;
                this.connecting = false;
                this.destroyed = false;
                this.remoteAddress = undefined;
                this.remotePort = undefined;
                this._pollTimer = null;
                if (this._connId !== null) this._startPoll();
            }
            util.inherits(Socket, Duplex);

            Socket.prototype._startPoll = function() {
                var self = this;
                if (self._pollTimer || self.destroyed) return;
                self._pollTimer = setInterval(function() {
                    if (self.destroyed || self._connId === null) return;
                    var chunk = __tcpRead(self._connId, 65536);
                    if (chunk instanceof Uint8Array) {
                        self.push(chunk);
                    } else if (chunk instanceof Error && chunk.code === 'EAGAIN') {
                        // no data available yet — keep polling
                    } else if (chunk instanceof Error && chunk.code === 'EOF') {
                        self._stopPoll();
                        self.push(null);
                    } else {
                        self._stopPoll();
                        self.emit('error', _netErr(chunk));
                        self.destroy();
                    }
                }, 5);
            };
            Socket.prototype._stopPoll = function() {
                if (this._pollTimer) { clearInterval(this._pollTimer); this._pollTimer = null; }
            };

            Socket.prototype._write = function(chunk, encoding, cb) {
                if (this._connId === null) { cb(new Error('Socket is not connected')); return; }
                var data = (chunk instanceof Uint8Array) ? chunk : new TextEncoder().encode(String(chunk));
                var result = __tcpWrite(this._connId, data);
                if (result === undefined) cb(); else cb(_netErr(result));
            };

            Socket.prototype.connect = function(port, host, cb) {
                var self = this;
                if (typeof host === 'function') { cb = host; host = 'localhost'; }
                host = host || 'localhost';
                if (typeof cb === 'function') self.once('connect', cb);
                self.connecting = true;
                var result = __tcpConnect(host, port);
                self.connecting = false;
                if (typeof result === 'number') {
                    self._connId = result;
                    self.remoteAddress = host;
                    self.remotePort = port;
                    self._startPoll();
                    setTimeout(function() { self.emit('connect'); }, 0);
                } else {
                    setTimeout(function() { self.emit('error', _netErr(result)); }, 0);
                }
                return self;
            };

            Socket.prototype.setTimeout = function(ms, cb) {
                if (typeof cb === 'function') this.once('timeout', cb);
                return this;
            };
            Socket.prototype.setNoDelay = function() { return this; };
            Socket.prototype.setKeepAlive = function() { return this; };
            Socket.prototype.destroy = function(err) {
                this._stopPoll();
                if (this._connId !== null) { __tcpClose(this._connId); this._connId = null; }
                this.destroyed = true;
                return Duplex.prototype.destroy.call(this, err);
            };
            Socket.prototype.end = function(chunk, encoding, cb) {
                var self = this;
                return Duplex.prototype.end.call(this, chunk, encoding, function(err) {
                    self.destroy();
                    if (cb) cb(err);
                });
            };

            function Server(opts, connListener) {
                if (!(this instanceof Server)) return new Server(opts, connListener);
                EventEmitter.call(this);
                if (typeof opts === 'function') { connListener = opts; opts = {}; }
                if (connListener) this.on('connection', connListener);
                this._listenerId = null;
                this._acceptTimer = null;
                this.listening = false;
            }
            util.inherits(Server, EventEmitter);

            Server.prototype.listen = function(port, host, cb) {
                var self = this;
                if (typeof host === 'function') { cb = host; host = '0.0.0.0'; }
                host = host || '0.0.0.0';
                var result = __netListen(port, host);
                if (typeof result === 'number') {
                    self._listenerId = result;
                    self.listening = true;
                    if (typeof cb === 'function') self.once('listening', cb);
                    self._acceptLoop();
                    setTimeout(function() { self.emit('listening'); }, 0);
                } else {
                    setTimeout(function() { self.emit('error', _netErr(result)); }, 0);
                }
                return self;
            };

            // __netAcceptAsync is a single non-blocking accept attempt (EAGAIN if no
            // pending connection), so this polls it on an interval — same model as
            // Socket._startPoll for reads — instead of blocking the whole engine
            // waiting for the next connection.
            Server.prototype._acceptLoop = function() {
                var self = this;
                self._acceptTimer = setInterval(function() {
                    if (!self.listening || self._listenerId === null) return;
                    var result = __netAcceptAsync(self._listenerId);
                    if (typeof result === 'number') {
                        self.emit('connection', new Socket({ fd: result }));
                    } else if (result instanceof Error && result.code === 'EAGAIN') {
                        // no pending connection — keep polling
                    } else if (self.listening) {
                        self.emit('error', _netErr(result));
                    }
                }, 5);
            };

            Server.prototype.close = function(cb) {
                if (this._acceptTimer) { clearInterval(this._acceptTimer); this._acceptTimer = null; }
                if (this._listenerId !== null) { __netClose(this._listenerId); this._listenerId = null; }
                this.listening = false;
                if (cb) this.once('close', cb);
                var self = this;
                setTimeout(function() { self.emit('close'); }, 0);
                return this;
            };
            Server.prototype.address = function() {
                return this.listening ? { address: '0.0.0.0', port: 0, family: 'IPv4' } : null;
            };

            function netCreateServer(opts, connListener) { return new Server(opts, connListener); }
            function netConnect(port, host, cb) { return new Socket({}).connect(port, host, cb); }

            globalThis.__requireCache['net'] = {
                Socket: Socket,
                Server: Server,
                createServer: netCreateServer,
                connect: netConnect,
                createConnection: netConnect,
                isIP: function(s) { return /^(\d{1,3}\.){3}\d{1,3}$/.test(s) ? 4 : (String(s).indexOf(':') !== -1 ? 6 : 0); },
                isIPv4: function(s) { return /^(\d{1,3}\.){3}\d{1,3}$/.test(s); },
                isIPv6: function(s) { return String(s).indexOf(':') !== -1; },
            };
            globalThis.__requireCache['node:net'] = globalThis.__requireCache['net'];

            function TLSSocket(socket, opts) {
                if (!(this instanceof TLSSocket)) return new TLSSocket(socket, opts);
                opts = opts || {};
                Duplex.call(this, opts);
                this._connId = (socket && typeof socket._connId === 'number') ? socket._connId : null;
                this.encrypted = true;
                this.destroyed = false;
                this._pollTimer = null;
                if (this._connId !== null) this._startPoll();
            }
            util.inherits(TLSSocket, Duplex);
            TLSSocket.prototype._startPoll = Socket.prototype._startPoll;
            TLSSocket.prototype._stopPoll = Socket.prototype._stopPoll;
            TLSSocket.prototype._write = Socket.prototype._write;
            TLSSocket.prototype.destroy = Socket.prototype.destroy;
            TLSSocket.prototype.end = Socket.prototype.end;
            TLSSocket.prototype.setTimeout = Socket.prototype.setTimeout;
            TLSSocket.prototype.setNoDelay = Socket.prototype.setNoDelay;
            TLSSocket.prototype.setKeepAlive = Socket.prototype.setKeepAlive;

            function tlsConnect(port, host, opts, cb) {
                if (typeof host === 'object' && host !== null) { cb = opts; opts = host; host = (opts && opts.host) || 'localhost'; }
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                host = host || (opts && opts.host) || 'localhost';
                var s = new TLSSocket(null, opts || {});
                if (typeof cb === 'function') s.once('secureConnect', cb);
                var result = __tcpConnectTls(host, port);
                if (typeof result === 'number') {
                    s._connId = result;
                    s._startPoll();
                    setTimeout(function() { s.emit('secureConnect'); s.emit('connect'); }, 0);
                } else {
                    setTimeout(function() { s.emit('error', _netErr(result)); }, 0);
                }
                return s;
            }

            globalThis.__requireCache['tls'] = { TLSSocket: TLSSocket, connect: tlsConnect };
            globalThis.__requireCache['node:tls'] = globalThis.__requireCache['tls'];
        })();
    "#;
    let source = V8String::new(scope, js_code).unwrap();
    let _ = Script::compile(scope, source, None).and_then(|s| s.run(scope));

    // ── require() — CommonJS module loader ────────────────────────────────────
    // Built-in modules (fs, process, util, events, ...) are looked up directly
    // in __requireCache, where each builtin's own inject_* registers itself.
    // Everything else is resolved from disk via the native __requireResolve /
    // __readFile bindings (which reuse the same node_modules/package.json-aware
    // resolver as ESM `import`) and executed as a CommonJS module.
    let require_js = r#"
    (function() {
        globalThis.__loadedModules = globalThis.__loadedModules || {};

        function bareName(specifier) {
            return specifier.indexOf('node:') === 0 ? specifier.slice(5) : specifier;
        }

        function requireFrom(specifier, dir) {
            var bare = bareName(specifier);
            if (Object.prototype.hasOwnProperty.call(globalThis.__requireCache, specifier)) {
                return globalThis.__requireCache[specifier];
            }
            if (Object.prototype.hasOwnProperty.call(globalThis.__requireCache, bare)) {
                return globalThis.__requireCache[bare];
            }
            if (Object.prototype.hasOwnProperty.call(globalThis.__fallbackModules, specifier)) {
                return globalThis.__fallbackModules[specifier];
            }

            var resolved = __requireResolve(specifier, dir);

            if (Object.prototype.hasOwnProperty.call(globalThis.__loadedModules, resolved)) {
                return globalThis.__loadedModules[resolved].exports;
            }

            if (resolved.slice(-5) === '.json') {
                var jsonSrc = __readFile(resolved);
                var parsed = JSON.parse(jsonSrc);
                globalThis.__loadedModules[resolved] = { exports: parsed };
                return parsed;
            }

            var moduleDir = resolved.replace(/[\/\\][^\/\\]*$/, '') || '.';
            var mod = { exports: {} };
            globalThis.__loadedModules[resolved] = mod;

            var localRequire = function(id) { return requireFrom(id, moduleDir); };
            localRequire.resolve = function(id) { return __requireResolve(id, moduleDir); };
            localRequire.cache = globalThis.__loadedModules;

            var source = __readFile(resolved);
            try {
                var fn = new Function('exports', 'module', 'require', '__filename', '__dirname', source);
                fn(mod.exports, mod, localRequire, resolved, moduleDir);
            } catch (e) {
                delete globalThis.__loadedModules[resolved];
                throw e;
            }
            return mod.exports;
        }

        globalThis.__vvva_require_from = requireFrom;
        globalThis.__vvva_require_resolve_from = function(specifier, dir) {
            var bare = bareName(specifier);
            if (Object.prototype.hasOwnProperty.call(globalThis.__requireCache, specifier)
                || Object.prototype.hasOwnProperty.call(globalThis.__requireCache, bare)) {
                return specifier;
            }
            return __requireResolve(specifier, dir);
        };

        globalThis.require = function(specifier) {
            return requireFrom(specifier, globalThis.__dirname || undefined);
        };
        globalThis.require.cache = globalThis.__loadedModules;
        globalThis.require.resolve = function(specifier) {
            return globalThis.__vvva_require_resolve_from(specifier, globalThis.__dirname || undefined);
        };
        globalThis.require.main = globalThis.require.main || undefined;
    })();
    "#;
    let require_source = V8String::new(scope, require_js).unwrap();
    let _ = Script::compile(scope, require_source, None).and_then(|s| s.run(scope));

    Ok(())
}

fn resolve_path_from(path: &str, basedir: Option<&str>) -> std::result::Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return Ok(path);
    }
    let base = basedir
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(base.join(path))
}

pub fn resolve_path_from_esm(
    specifier: &str,
    basedir: Option<&str>,
) -> std::result::Result<PathBuf, String> {
    let path = PathBuf::from(specifier);
    if path.is_absolute() {
        return Ok(path);
    }
    let base = basedir
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(base.join(path))
}

pub fn split_bare_specifier(name: &str) -> (&str, Option<&str>) {
    if let Some(pos) = name.find('/') {
        let pkg = &name[..pos];
        let subpath = name[pos + 1..].strip_prefix('/').filter(|s| !s.is_empty());
        (pkg, subpath)
    } else {
        (name, None)
    }
}

pub fn resolve_exports_value(
    val: &serde_json::Value,
    pkg_dir: &Path,
    _is_dir: bool,
) -> Option<PathBuf> {
    if let Some(s) = val.as_str() {
        let path = resolve_path_from(s, Some(&pkg_dir.to_string_lossy()));
        if path.is_ok() {
            return path.ok();
        }
    }
    None
}

pub fn resolve_exports_pattern(
    exports: &serde_json::Map<String, serde_json::Value>,
    subpath: &str,
    pkg_dir: &Path,
) -> Option<Option<PathBuf>> {
    let pattern_key = subpath.replace('*', "x");
    for (key, val) in exports {
        let pattern = key.replace('*', "x");
        if pattern == pattern_key
            && let Some(s) = val.as_str()
        {
            let resolved = pkg_dir.join(s.trim_start_matches("./"));
            return Some(Some(resolved));
        }
    }
    let wildcard_key = pattern_key.replace("x", "*");
    if let Some(val) = exports.get(&wildcard_key)
        && let Some(s) = val.as_str()
    {
        let result = subpath.replace('*', "");
        let resolved = pkg_dir.join(s.replace('*', &result));
        return Some(Some(resolved));
    }
    Some(None)
}
