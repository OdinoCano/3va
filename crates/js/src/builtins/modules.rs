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

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static INJECT_MODULES_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn permissions() -> Arc<PermissionState> {
    INJECT_MODULES_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_require not called on this thread")
    })
}

pub fn inject_require(
    scope: &mut ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    INJECT_MODULES_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));
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
                let err = v8::Exception::error(scope, err_str);
                scope.throw_exception(err);
                return;
            }

            match std::fs::read_to_string(&full_path) {
                Ok(source) => {
                    let is_jsx = path_str.ends_with(".tsx") || path_str.ends_with(".jsx");
                    // A required file with real static import/export syntax
                    // must go through ESM→CJS conversion too — same fix as
                    // eval_file() in lib.rs, same reason: new Function(source)
                    // runs this as a plain function body, which V8 rejects if
                    // it contains bare `import`/`export` declarations.
                    let is_esm =
                        path_str.ends_with(".mjs") || crate::esm::source_is_esm(&source, &path_str);
                    let transpiled = if path_str.ends_with(".cjs")
                        || path_str.ends_with(".json")
                        || source.contains("@exodus/bytes")
                    {
                        source
                    } else if is_esm {
                        crate::transpiler::transpile_to_cjs(&source, is_jsx)
                    } else if is_jsx {
                        crate::transpiler::transpile_jsx(&source)
                    } else if path_str.ends_with(".ts")
                        || path_str.ends_with(".mts")
                        || path_str.ends_with(".cts")
                    {
                        crate::transpiler::transpile(&source)
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

    // ── __setCallerScope(name) ────────────────────────────────────────────────
    // Backs the require() wrapper below: right before handing a
    // capability-gated builtin (fs, net, ...) to the module that required it,
    // JS calls this to record "which package's code is executing" so
    // PermissionState::check() can apply package.json["3va"].permissions.<pkg>
    // grants without applying them process-wide. Pass "." to clear back to
    // the app-level scope. See vvva_permissions::scope for the actual storage.
    let set_caller_scope_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut _rv: ReturnValue| {
            let name = args.get(0).to_rust_string_lossy(scope);
            vvva_permissions::set_current_scope(&name);
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__setCallerScope").unwrap().into(),
        set_caller_scope_fn.unwrap().into(),
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

            // Plain blocking std::net::ToSocketAddrs — no tokio involved, so
            // no need for tokio::task::block_in_place (which requires a
            // multi_thread runtime and panics outright on current_thread,
            // e.g. plain `#[tokio::test]`; see fs_watch's __fsWatchNext fix
            // for the same bug pattern).
            let result = {
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
            };

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

            // Run the lookup on a brand-new OS thread with its own runtime,
            // then join() it — not tokio::task::block_in_place +
            // Handle::current().block_on(), nor a fresh runtime's block_on()
            // called directly on THIS thread. Tokio marks any thread that's
            // currently driving a runtime (even if you build a second,
            // unrelated Runtime and call block_on on *that*) and panics with
            // "Cannot start a runtime from within a runtime" — the guard is
            // per-thread, not per-runtime-instance. A plain std::thread has
            // no such marker, so blocking (join) on it from here is safe
            // regardless of what runtime (if any) is driving this thread;
            // see fs_watch's __fsWatchNext fix for the same root bug.
            let hostname_for_thread = hostname.clone();
            let result: Result<hickory_resolver::lookup::Lookup, String> =
                std::thread::spawn(move || {
                    let hostname = hostname_for_thread;
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?;
                    rt.block_on(async {
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
                    })
                })
                .join()
                .unwrap_or_else(|_| Err("dns query thread panicked".to_string()));

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

            // ── assert ───────────────────────────────────────────────────────────
            (function() {
                function AssertionError(opts) {
                    opts = opts || {};
                    var msg = opts.message;
                    if (!msg) {
                        msg = (opts.operator || 'deepStrictEqual') + '(' + safeInspect(opts.actual) + ', ' + safeInspect(opts.expected) + ')';
                    }
                    Error.call(this, msg);
                    this.message = msg;
                    this.name = 'AssertionError';
                    this.actual = opts.actual;
                    this.expected = opts.expected;
                    this.operator = opts.operator;
                    this.generatedMessage = !opts.message;
                    this.code = 'ERR_ASSERTION';
                    if (typeof Error.captureStackTrace === 'function') Error.captureStackTrace(this, opts.stackStartFn || AssertionError);
                }
                AssertionError.prototype = Object.create(Error.prototype);
                AssertionError.prototype.constructor = AssertionError;
                AssertionError.prototype.name = 'AssertionError';

                function safeInspect(v) {
                    try {
                        if (typeof v === 'string') return JSON.stringify(v);
                        if (v instanceof Error) return String(v);
                        if (typeof v === 'object' && v !== null) return JSON.stringify(v);
                        return String(v);
                    } catch (e) { return String(v); }
                }

                function fail(message, actual, expected, operator, stackStartFn) {
                    if (message instanceof Error) throw message;
                    throw new AssertionError({
                        message: typeof message === 'string' ? message : undefined,
                        actual: actual, expected: expected, operator: operator,
                        stackStartFn: stackStartFn,
                    });
                }

                function deepEqualInternal(a, b, strict, seen) {
                    if (Object.is(a, b)) return true;
                    if (a === null || b === null || a === undefined || b === undefined) return false;
                    if (typeof a !== typeof b) return false;
                    if (typeof a !== 'object') {
                        return !strict && a == b;
                    }
                    if (strict && Object.getPrototypeOf(a) !== Object.getPrototypeOf(b)) return false;

                    if (a instanceof Date || b instanceof Date) {
                        return a instanceof Date && b instanceof Date && a.getTime() === b.getTime();
                    }
                    if (a instanceof RegExp || b instanceof RegExp) {
                        return a instanceof RegExp && b instanceof RegExp && a.source === b.source && a.flags === b.flags;
                    }
                    var aTyped = ArrayBuffer.isView(a), bTyped = ArrayBuffer.isView(b);
                    if (aTyped || bTyped) {
                        if (!aTyped || !bTyped || a.length !== b.length) return false;
                        if (strict && a.constructor !== b.constructor) return false;
                        for (var ti = 0; ti < a.length; ti++) if (a[ti] !== b[ti]) return false;
                        return true;
                    }

                    seen = seen || [];
                    for (var si = 0; si < seen.length; si++) {
                        if (seen[si][0] === a && seen[si][1] === b) return true;
                    }
                    seen.push([a, b]);

                    if (Array.isArray(a) || Array.isArray(b)) {
                        if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
                        for (var ai = 0; ai < a.length; ai++) {
                            if (!deepEqualInternal(a[ai], b[ai], strict, seen)) return false;
                        }
                        return true;
                    }

                    if (typeof Map !== 'undefined' && (a instanceof Map || b instanceof Map)) {
                        if (!(a instanceof Map) || !(b instanceof Map) || a.size !== b.size) return false;
                        var mapOk = true;
                        a.forEach(function(v, k) {
                            if (!mapOk) return;
                            if (!b.has(k) || !deepEqualInternal(v, b.get(k), strict, seen)) mapOk = false;
                        });
                        return mapOk;
                    }
                    if (typeof Set !== 'undefined' && (a instanceof Set || b instanceof Set)) {
                        if (!(a instanceof Set) || !(b instanceof Set) || a.size !== b.size) return false;
                        var setOk = true;
                        a.forEach(function(v) { if (!b.has(v)) setOk = false; });
                        return setOk;
                    }

                    var keysA = Object.keys(a), keysB = Object.keys(b);
                    if (keysA.length !== keysB.length) return false;
                    for (var ki = 0; ki < keysA.length; ki++) {
                        var key = keysA[ki];
                        if (!Object.prototype.hasOwnProperty.call(b, key)) return false;
                        if (!deepEqualInternal(a[key], b[key], strict, seen)) return false;
                    }
                    return true;
                }

                function assert(value, message) {
                    if (!value) fail(message, value, true, '==', assert);
                }
                assert.AssertionError = AssertionError;
                assert.ok = assert;
                assert.equal = function(a, b, msg) { if (!(a == b)) fail(msg, a, b, '==', assert.equal); };
                assert.notEqual = function(a, b, msg) { if (a == b) fail(msg, a, b, '!=', assert.notEqual); };
                assert.strictEqual = function(a, b, msg) { if (!Object.is(a, b)) fail(msg, a, b, 'strictEqual', assert.strictEqual); };
                assert.notStrictEqual = function(a, b, msg) { if (Object.is(a, b)) fail(msg, a, b, 'notStrictEqual', assert.notStrictEqual); };
                assert.deepEqual = function(a, b, msg) { if (!deepEqualInternal(a, b, false)) fail(msg, a, b, 'deepEqual', assert.deepEqual); };
                assert.notDeepEqual = function(a, b, msg) { if (deepEqualInternal(a, b, false)) fail(msg, a, b, 'notDeepEqual', assert.notDeepEqual); };
                assert.deepStrictEqual = function(a, b, msg) { if (!deepEqualInternal(a, b, true)) fail(msg, a, b, 'deepStrictEqual', assert.deepStrictEqual); };
                assert.notDeepStrictEqual = function(a, b, msg) { if (deepEqualInternal(a, b, true)) fail(msg, a, b, 'notDeepStrictEqual', assert.notDeepStrictEqual); };
                assert.throws = function(fn, errorOrMsg, msg) {
                    var errorMatcher = typeof errorOrMsg === 'function' || errorOrMsg instanceof RegExp ? errorOrMsg : undefined;
                    if (errorMatcher === undefined && typeof errorOrMsg === 'string') msg = errorOrMsg;
                    try { fn(); }
                    catch (e) {
                        if (typeof errorMatcher === 'function' && !(e instanceof errorMatcher)) {
                            throw new AssertionError({ message: msg, actual: e, expected: errorMatcher, operator: 'throws', stackStartFn: assert.throws });
                        }
                        if (errorMatcher instanceof RegExp && !errorMatcher.test(e.message)) {
                            throw new AssertionError({ message: msg, actual: e.message, expected: errorMatcher, operator: 'throws', stackStartFn: assert.throws });
                        }
                        return;
                    }
                    fail(msg || 'Missing expected exception', undefined, undefined, 'throws', assert.throws);
                };
                assert.doesNotThrow = function(fn, msg) {
                    try { fn(); }
                    catch (e) { fail(msg || ('Got unwanted exception: ' + e.message), undefined, undefined, 'doesNotThrow', assert.doesNotThrow); }
                };
                assert.ifError = function(err) {
                    if (err !== null && err !== undefined) {
                        throw (err instanceof Error ? err : new AssertionError({ message: 'ifError got unwanted exception: ' + safeInspect(err), actual: err, expected: null, operator: 'ifError', stackStartFn: assert.ifError }));
                    }
                };
                assert.match = function(str, regexp, msg) { if (!regexp.test(str)) fail(msg, str, regexp, 'match', assert.match); };
                assert.doesNotMatch = function(str, regexp, msg) { if (regexp.test(str)) fail(msg, str, regexp, 'doesNotMatch', assert.doesNotMatch); };
                assert.rejects = function(fn, errorOrMsg, msg) {
                    var errorMatcher = typeof errorOrMsg === 'function' ? errorOrMsg : undefined;
                    if (errorMatcher === undefined && typeof errorOrMsg === 'string') msg = errorOrMsg;
                    var p = typeof fn === 'function' ? Promise.resolve().then(fn) : Promise.resolve(fn);
                    return p.then(
                        function() { fail(msg || 'Missing expected rejection', undefined, undefined, 'rejects', assert.rejects); },
                        function(e) {
                            if (typeof errorMatcher === 'function' && !(e instanceof errorMatcher)) {
                                throw new AssertionError({ message: msg, actual: e, expected: errorMatcher, operator: 'rejects', stackStartFn: assert.rejects });
                            }
                        }
                    );
                };
                assert.doesNotReject = function(fn, msg) {
                    var p = typeof fn === 'function' ? Promise.resolve().then(fn) : Promise.resolve(fn);
                    return p.catch(function(e) {
                        fail(msg || ('Got unwanted rejection: ' + e.message), undefined, undefined, 'doesNotReject', assert.doesNotReject);
                    });
                };
                assert.fail = function(message) { fail(message, undefined, undefined, 'fail', assert.fail); };

                globalThis.__requireCache['assert'] = assert;
                globalThis.__requireCache['node:assert'] = assert;
                globalThis.__requireCache['assert/strict'] = assert;
                globalThis.__requireCache['node:assert/strict'] = assert;
            })();

            // ── timers / timers/promises ────────────────────────────────────────────
            (function() {
                var timers = {
                    setTimeout: globalThis.setTimeout,
                    clearTimeout: globalThis.clearTimeout,
                    setInterval: globalThis.setInterval,
                    clearInterval: globalThis.clearInterval,
                    setImmediate: globalThis.setImmediate,
                    clearImmediate: globalThis.clearImmediate,
                };
                globalThis.__requireCache['timers'] = timers;
                globalThis.__requireCache['node:timers'] = timers;

                // Whatever reason a signal carries, the rejection here must
                // look like a real AbortError (Node/DOM give it `.name ===
                // 'AbortError'`) — the generic AbortController shim in
                // web_globals.rs just stores `new Error('AbortError')` as
                // the reason, whose `.name` is still the generic 'Error'.
                function abortError(signal) {
                    var err = (signal && signal.reason instanceof Error) ? signal.reason : new Error('The operation was aborted');
                    if (!err.name || err.name === 'Error') err.name = 'AbortError';
                    return err;
                }

                var timersPromises = {
                    setTimeout: function(delay, value, options) {
                        options = options || {};
                        var signal = options.signal;
                        return new Promise(function(resolve, reject) {
                            if (signal && signal.aborted) { reject(abortError(signal)); return; }
                            var id = globalThis.setTimeout(function() { resolve(value); }, delay);
                            if (signal) {
                                signal.addEventListener('abort', function() {
                                    globalThis.clearTimeout(id);
                                    reject(abortError(signal));
                                });
                            }
                        });
                    },
                    setImmediate: function(value, options) {
                        options = options || {};
                        var signal = options.signal;
                        return new Promise(function(resolve, reject) {
                            if (signal && signal.aborted) { reject(abortError(signal)); return; }
                            var id = globalThis.setImmediate(function() { resolve(value); });
                            if (signal) {
                                signal.addEventListener('abort', function() {
                                    globalThis.clearImmediate ? globalThis.clearImmediate(id) : globalThis.clearTimeout(id);
                                    reject(abortError(signal));
                                });
                            }
                        });
                    },
                    setInterval: function(delay, value, options) {
                        options = options || {};
                        var signal = options.signal;
                        var iterator = {
                            next: function() {
                                return new Promise(function(resolve, reject) {
                                    if (signal && signal.aborted) { reject(abortError(signal)); return; }
                                    globalThis.setTimeout(function() { resolve({ value: value, done: false }); }, delay);
                                });
                            },
                            return: function() { return Promise.resolve({ value: undefined, done: true }); },
                        };
                        iterator[Symbol.asyncIterator] = function() { return iterator; };
                        return iterator;
                    },
                };
                globalThis.__requireCache['timers/promises'] = timersPromises;
                globalThis.__requireCache['node:timers/promises'] = timersPromises;
            })();

            // ── dns / dns/promises ───────────────────────────────────────────────────
            // Backed by real native lookups (__dnsLookup: std::net::ToSocketAddrs;
            // __dnsQuery: hickory-resolver) — both already existed but were never
            // assembled into a require()able module.
            (function() {
                function mkErr(raw, hostname, syscall) {
                    var code = 'ENOTFOUND';
                    var m = /^([A-Z]+)[: ]/.exec(raw);
                    if (m) code = m[1];
                    var err = new Error(raw);
                    err.code = code;
                    err.hostname = hostname;
                    err.syscall = syscall || 'queryA';
                    return err;
                }

                function lookup(hostname, options, callback) {
                    if (typeof options === 'function') { callback = options; options = {}; }
                    options = options || {};
                    setTimeout(function() {
                        var raw = __dnsLookup(hostname);
                        var ips;
                        try { ips = JSON.parse(raw); } catch (e) { callback(mkErr(raw, hostname, 'getaddrinfo')); return; }
                        if (options.all) {
                            callback(null, ips.map(function(ip) { return { address: ip, family: ip.indexOf(':') !== -1 ? 6 : 4 }; }));
                        } else {
                            var ip = ips[0];
                            callback(null, ip, ip.indexOf(':') !== -1 ? 6 : 4);
                        }
                    }, 0);
                }

                function resolveFamily(hostname, family, callback) {
                    setTimeout(function() {
                        var raw = __dnsLookup(hostname);
                        var ips;
                        try { ips = JSON.parse(raw); } catch (e) { callback(mkErr(raw, hostname, 'queryA')); return; }
                        var filtered = ips.filter(function(ip) { return (ip.indexOf(':') !== -1) === (family === 6); });
                        if (!filtered.length) { callback(mkErr('ENOTFOUND', hostname, 'queryA')); return; }
                        callback(null, filtered);
                    }, 0);
                }

                function resolveQuery(rrtype) {
                    return function(hostname, callback) {
                        setTimeout(function() {
                            var raw = __dnsQuery(hostname, rrtype);
                            var data;
                            try { data = JSON.parse(raw); } catch (e) { callback(mkErr(raw, hostname, 'query' + rrtype)); return; }
                            if (data === null || (Array.isArray(data) && data.length === 0)) {
                                var err = new Error('query' + rrtype + ' ENODATA ' + hostname);
                                err.code = 'ENODATA';
                                err.hostname = hostname;
                                err.syscall = 'query' + rrtype;
                                callback(err);
                                return;
                            }
                            callback(null, data);
                        }, 0);
                    };
                }

                var resolveMx = resolveQuery('MX');
                var resolveTxt = resolveQuery('TXT');
                var resolveSrv = resolveQuery('SRV');
                var resolveNs = resolveQuery('NS');
                var resolveCname = resolveQuery('CNAME');
                var resolveNaptr = resolveQuery('NAPTR');
                var resolveSoa = resolveQuery('SOA');
                var resolvePtrQuery = resolveQuery('PTR');

                function resolve4(hostname, callback) { resolveFamily(hostname, 4, callback); }
                function resolve6(hostname, callback) { resolveFamily(hostname, 6, callback); }

                function resolve(hostname, rrtype, callback) {
                    if (typeof rrtype === 'function') { callback = rrtype; rrtype = 'A'; }
                    rrtype = rrtype || 'A';
                    var map = {
                        A: resolve4, AAAA: resolve6, MX: resolveMx, TXT: resolveTxt,
                        SRV: resolveSrv, NS: resolveNs, CNAME: resolveCname,
                        NAPTR: resolveNaptr, SOA: resolveSoa, PTR: resolvePtrQuery,
                    };
                    (map[rrtype] || resolve4)(hostname, callback);
                }

                function reverse(ip, callback) {
                    setTimeout(function() {
                        var raw = __dnsQuery(ip, 'PTR');
                        var data;
                        try { data = JSON.parse(raw); } catch (e) { callback(mkErr(raw, ip, 'getHostByAddr')); return; }
                        callback(null, data);
                    }, 0);
                }

                function promisify1(fn) {
                    return function(a, b) {
                        return new Promise(function(resolve, reject) {
                            fn(a, b, function(err, res) { if (err) reject(err); else resolve(res); });
                        });
                    };
                }

                function lookupPromise(hostname, options) {
                    return new Promise(function(resolvePromise, reject) {
                        lookup(hostname, options || {}, function(err, address, family) {
                            if (err) { reject(err); return; }
                            if (Array.isArray(address)) resolvePromise(address);
                            else resolvePromise({ address: address, family: family });
                        });
                    });
                }

                var dns = {
                    lookup: lookup,
                    resolve: resolve,
                    resolve4: resolve4,
                    resolve6: resolve6,
                    resolveMx: resolveMx,
                    resolveTxt: resolveTxt,
                    resolveSrv: resolveSrv,
                    resolveNs: resolveNs,
                    resolveCname: resolveCname,
                    resolveNaptr: resolveNaptr,
                    resolveSoa: resolveSoa,
                    resolvePtr: resolvePtrQuery,
                    reverse: reverse,
                    getServers: function() { return []; },
                    setServers: function() {},
                    lookupService: function(address, port, callback) {
                        setTimeout(function() {
                            var err = new Error('getnameinfo ENOTSUP ' + address);
                            err.code = 'ENOTSUP';
                            err.errno = -86;
                            callback(err);
                        }, 0);
                    },
                    ADDRCONFIG: 0, ALL: 0, V4MAPPED: 0,
                    NODATA: 'ENODATA', FORMERR: 'EFORMERR', SERVFAIL: 'ESERVFAIL',
                    NOTFOUND: 'ENOTFOUND', NOTIMP: 'ENOTIMP', REFUSED: 'EREFUSED',
                };
                dns.promises = {
                    lookup: lookupPromise,
                    resolve: function(hostname, rrtype) { return promisify1(resolve)(hostname, rrtype); },
                    resolve4: promisify1(function(h, _b, cb) { resolve4(h, cb); }),
                    resolve6: promisify1(function(h, _b, cb) { resolve6(h, cb); }),
                    resolveMx: promisify1(function(h, _b, cb) { resolveMx(h, cb); }),
                    resolveTxt: promisify1(function(h, _b, cb) { resolveTxt(h, cb); }),
                    resolveSrv: promisify1(function(h, _b, cb) { resolveSrv(h, cb); }),
                    resolveNs: promisify1(function(h, _b, cb) { resolveNs(h, cb); }),
                    resolveCname: promisify1(function(h, _b, cb) { resolveCname(h, cb); }),
                    resolveNaptr: promisify1(function(h, _b, cb) { resolveNaptr(h, cb); }),
                    resolveSoa: promisify1(function(h, _b, cb) { resolveSoa(h, cb); }),
                    resolvePtr: promisify1(function(h, _b, cb) { resolvePtrQuery(h, cb); }),
                    reverse: promisify1(function(h, _b, cb) { reverse(h, cb); }),
                    getServers: function() { return []; },
                    setServers: function() {},
                };
                globalThis.__requireCache['dns'] = dns;
                globalThis.__requireCache['node:dns'] = dns;
                globalThis.__requireCache['dns/promises'] = dns.promises;
                globalThis.__requireCache['node:dns/promises'] = dns.promises;
            })();

            (function() {
                var os = {
                    hostname: function() { return __osHostname(); },
                    totalmem: function() { return __osTotalMem(); },
                    freemem: function() { return __osFreeMem(); },
                    uptime: function() { return __osUptime(); },
                    cpus: function() { return JSON.parse(__osCpus()); },
                    networkInterfaces: function() { return JSON.parse(__osNetworkInterfaces()); },
                    platform: function() { return __osPlatform(); },
                    arch: function() { return __osArch(); },
                    type: function() {
                        var p = __osPlatform();
                        return p === 'darwin' ? 'Darwin' : (p === 'win32' ? 'Windows_NT' : 'Linux');
                    },
                    release: function() { return '0.0.0'; },
                    homedir: function() {
                        return (globalThis.process && globalThis.process.env &&
                                (globalThis.process.env.HOME || globalThis.process.env.USERPROFILE)) || '/';
                    },
                    tmpdir: function() {
                        return (globalThis.process && globalThis.process.env && globalThis.process.env.TMPDIR) || '/tmp';
                    },
                    endianness: function() { return 'LE'; },
                    EOL: __osPlatform() === 'win32' ? '\r\n' : '\n',
                    constants: { signals: {}, errno: {}, priority: {} },
                };
                globalThis.__requireCache['os'] = os;
                globalThis.__requireCache['node:os'] = os;

                var tty = {
                    isatty: function(fd) { return typeof __isatty === 'function' ? __isatty(fd) : false; },
                    ReadStream: function() {},
                    WriteStream: function() {},
                };
                globalThis.__requireCache['tty'] = tty;
                globalThis.__requireCache['node:tty'] = tty;

                var v8mod = {
                    getHeapStatistics: function() {
                        return {
                            total_heap_size: 0, total_heap_size_executable: 0,
                            total_physical_size: 0, total_available_size: 0,
                            used_heap_size: 0, heap_size_limit: 0,
                            malloced_memory: 0, peak_malloced_memory: 0,
                            does_zap_garbage: 0, number_of_native_contexts: 0,
                            number_of_detached_contexts: 0,
                        };
                    },
                    getHeapSpaceStatistics: function() { return []; },
                    setFlagsFromString: function() {},
                    serialize: function(v) { return Buffer.from(JSON.stringify(v)); },
                    deserialize: function(buf) { return JSON.parse(buf.toString()); },
                };
                globalThis.__requireCache['v8'] = v8mod;
                globalThis.__requireCache['node:v8'] = v8mod;

                // ponytail: sandboxes are plain objects run through `with`,
                // not real isolated V8 contexts — enough for the common
                // "eval config code against a data object" use case; a real
                // per-Context sandbox needs a native v8::Context binding,
                // add one if code needs true isolation (e.g. untrusted input).
                var _vmContexts = new WeakSet();
                function vmRunInSandbox(code, sandbox) {
                    try {
                        return (new Function('__vm_sandbox__', 'with(__vm_sandbox__){ return (' + code + ') }'))(sandbox);
                    } catch (e) {
                        if (e instanceof SyntaxError) {
                            return (new Function('__vm_sandbox__', 'with(__vm_sandbox__){ ' + code + ' }'))(sandbox);
                        }
                        throw e;
                    }
                }
                function VmScript(code) { this.code = code; }
                VmScript.prototype.runInContext = function(sandbox) { return vmRunInSandbox(this.code, sandbox || {}); };
                VmScript.prototype.runInNewContext = function(sandbox) { return vmRunInSandbox(this.code, sandbox || {}); };
                VmScript.prototype.runInThisContext = function() { return (0, eval)(this.code); };

                var vm = {
                    createContext: function(sandbox) {
                        sandbox = sandbox || {};
                        _vmContexts.add(sandbox);
                        return sandbox;
                    },
                    isContext: function(obj) {
                        return typeof obj === 'object' && obj !== null && _vmContexts.has(obj);
                    },
                    runInNewContext: function(code, sandbox) { return vmRunInSandbox(code, sandbox || {}); },
                    runInContext: function(code, sandbox) { return vmRunInSandbox(code, sandbox || {}); },
                    runInThisContext: function(code) { return (0, eval)(code); },
                    Script: VmScript,
                };
                globalThis.__requireCache['vm'] = vm;
                globalThis.__requireCache['node:vm'] = vm;

                var cluster = Object.create(EventEmitter.prototype);
                EventEmitter.call(cluster);
                var _clusterWorkers = {};
                var _clusterNextId = 0;
                function ClusterWorker() {
                    EventEmitter.call(this);
                    this.id = ++_clusterNextId;
                    this.process = { pid: 0 };
                    this.state = 'online';
                    this.exitedAfterDisconnect = false;
                }
                util.inherits(ClusterWorker, EventEmitter);
                ClusterWorker.prototype.send = function() { return true; };
                ClusterWorker.prototype.kill = function() { this.state = 'dead'; };
                ClusterWorker.prototype.disconnect = function() { this.state = 'dead'; };
                ClusterWorker.prototype.isDead = function() { return this.state === 'dead'; };

                cluster.isPrimary = true;
                cluster.isMaster = true;
                cluster.isWorker = false;
                cluster.workers = _clusterWorkers;
                cluster.settings = {};
                cluster.schedulingPolicy = 2;
                cluster.SCHED_NONE = 1;
                cluster.SCHED_RR = 2;
                cluster.fork = function() {
                    var w = new ClusterWorker();
                    _clusterWorkers[w.id] = w;
                    return w;
                };
                cluster.setupPrimary = function(settings) { cluster.settings = settings || {}; };
                cluster.setupMaster = cluster.setupPrimary;
                cluster.disconnect = function(cb) { if (cb) cb(); };
                globalThis.__requireCache['cluster'] = cluster;
                globalThis.__requireCache['node:cluster'] = cluster;
            })();

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

            // ── process.stdout/stderr as real Writable streams, process.env as a
            // real Proxy — process.rs sets plain placeholders (see its comments),
            // this replaces them now that stream.Writable exists.
            if (typeof process !== 'undefined') {
                var __stdoutFd = process.stdout && process.stdout.fd;
                var __stderrFd = process.stderr && process.stderr.fd;
                process.stdout = new Writable({
                    write: function(chunk, encoding, callback) {
                        __stdoutWrite(Buffer.isBuffer(chunk) || chunk instanceof Uint8Array ? Buffer.from(chunk).toString() : String(chunk));
                        callback();
                    }
                });
                process.stdout.fd = __stdoutFd === undefined ? 1 : __stdoutFd;
                process.stdout.isTTY = false;
                process.stdout.columns = 80;
                process.stdout.rows = 24;

                process.stderr = new Writable({
                    write: function(chunk, encoding, callback) {
                        __stderrWrite(Buffer.isBuffer(chunk) || chunk instanceof Uint8Array ? Buffer.from(chunk).toString() : String(chunk));
                        callback();
                    }
                });
                process.stderr.fd = __stderrFd === undefined ? 2 : __stderrFd;
                process.stderr.isTTY = false;

                // process.stdin: a Readable pulling from the real OS stdin
                // (native __stdinRead(), process.rs — non-blocking, backed by
                // a background reader thread so waiting for input never
                // freezes the engine). Paused by default like Node; starts
                // pumping chunks once something adds a 'data' listener or
                // calls .resume(), matching Node's flowing-mode semantics.
                var stdin = new Readable();
                stdin.fd = 0;
                stdin.isTTY = typeof __isatty === 'function' ? __isatty(0) : false;
                stdin._paused = true;
                stdin._ended = false;
                stdin._encoding = null;
                stdin.setEncoding = function(enc) { this._encoding = enc; return this; };
                stdin.pause = function() { this._paused = true; return this; };
                stdin.resume = function() {
                    if (!this._paused) return this;
                    this._paused = false;
                    this._pump();
                    return this;
                };
                stdin._pump = function() {
                    var self = this;
                    if (self._paused || self._ended) return;
                    __stdinRead().then(function(chunk) {
                        if (self._ended) return;
                        if (chunk.length === 0) {
                            self._ended = true;
                            self.emit('end');
                            self.emit('close');
                            return;
                        }
                        var data = self._encoding
                            ? new TextDecoder(self._encoding === 'utf8' ? 'utf-8' : self._encoding).decode(chunk)
                            : (typeof Buffer !== 'undefined' ? Buffer.from(chunk) : chunk);
                        self.emit('data', data);
                        if (!self._paused) self._pump();
                    });
                };
                var stdinOn = stdin.on;
                stdin.on = stdin.addListener = function(ev, fn) {
                    stdinOn.call(this, ev, fn);
                    if (ev === 'data') this.resume();
                    return this;
                };
                process.stdin = stdin;

                var __envTarget = process.env || {};
                process.env = new Proxy(__envTarget, {
                    get: function(target, prop) {
                        if (prop === '__isProxy') return true;
                        return target[prop];
                    },
                    set: function(target, prop, value) { target[prop] = String(value); return true; },
                    has: function(target, prop) { return prop in target; },
                    deleteProperty: function(target, prop) { delete target[prop]; return true; },
                    ownKeys: function(target) { return Object.keys(target); },
                    getOwnPropertyDescriptor: function(target, prop) {
                        return Object.getOwnPropertyDescriptor(target, prop);
                    }
                });
            }

            // ── readline / readline/promises ────────────────────────────────────────
            (function() {
                function Interface(options) {
                    EventEmitter.call(this);
                    options = options || {};
                    this.input = options.input;
                    this.output = options.output;
                    this.terminal = options.terminal !== undefined ? !!options.terminal : !!(this.output && this.output.isTTY);
                    this._prompt = '> ';
                    this._lineBuffer = '';
                    this._closed = false;

                    var self = this;
                    if (this.input) {
                        this.input.on('data', function(chunk) {
                            self._feedChars(typeof chunk === 'string' ? chunk : chunk.toString());
                        });
                        this.input.on('end', function() {
                            if (self._lineBuffer.length) {
                                self.emit('line', self._lineBuffer);
                                self._lineBuffer = '';
                            }
                            self.close();
                        });
                    }
                }
                util.inherits(Interface, EventEmitter);

                Interface.prototype._feedChars = function(str) {
                    this._lineBuffer += str;
                    var lines = this._lineBuffer.split(/\r\n|\r|\n/);
                    this._lineBuffer = lines.pop();
                    for (var i = 0; i < lines.length; i++) this.emit('line', lines[i]);
                };
                Interface.prototype.setPrompt = function(p) { this._prompt = p; return this; };
                Interface.prototype.getPrompt = function() { return this._prompt; };
                Interface.prototype.prompt = function() {
                    if (this.output) this.output.write(this._prompt);
                    return this;
                };
                Interface.prototype.question = function(query, options, callback) {
                    if (typeof options === 'function') { callback = options; options = {}; }
                    if (this.output) this.output.write(query);
                    this.once('line', function(line) { if (callback) callback(line); });
                    return this;
                };
                Interface.prototype.close = function() {
                    if (this._closed) return;
                    this._closed = true;
                    this.emit('close');
                };
                Interface.prototype.pause = function() {
                    if (this.input && this.input.pause) this.input.pause();
                    return this;
                };
                Interface.prototype.resume = function() {
                    if (this.input && this.input.resume) this.input.resume();
                    return this;
                };
                Interface.prototype.write = function(data) { this._feedChars(data); return this; };
                Interface.prototype[Symbol.asyncIterator] = function() {
                    var self = this, buffered = [], waiting = null, done = false;
                    this.on('line', function(line) {
                        if (waiting) { var w = waiting; waiting = null; w({ value: line, done: false }); }
                        else buffered.push(line);
                    });
                    this.on('close', function() {
                        done = true;
                        if (waiting) { var w = waiting; waiting = null; w({ value: undefined, done: true }); }
                    });
                    return {
                        next: function() {
                            if (buffered.length) return Promise.resolve({ value: buffered.shift(), done: false });
                            if (done) return Promise.resolve({ value: undefined, done: true });
                            return new Promise(function(resolve) { waiting = resolve; });
                        },
                        return: function() { self.close(); return Promise.resolve({ value: undefined, done: true }); },
                    };
                };

                function createInterface(options) {
                    return new Interface(options);
                }
                function cursorTo(stream, x, y, cb) { if (typeof y === 'function') cb = y; if (cb) cb(); return true; }
                function moveCursor(stream, dx, dy, cb) { if (typeof dy === 'function') cb = dy; if (cb) cb(); return true; }
                function clearLine(stream, dir, cb) { if (typeof dir === 'function') cb = dir; if (cb) cb(); return true; }
                function clearScreenDown(stream, cb) { if (cb) cb(); return true; }

                var readline = {
                    Interface: Interface,
                    createInterface: createInterface,
                    cursorTo: cursorTo,
                    moveCursor: moveCursor,
                    clearLine: clearLine,
                    clearScreenDown: clearScreenDown,
                    emitKeypressEvents: function() {},
                };
                readline.promises = {
                    Interface: Interface,
                    createInterface: function(options) {
                        var iface = new Interface(options);
                        var origQuestion = iface.question.bind(iface);
                        iface.question = function(query) {
                            return new Promise(function(resolve) { origQuestion(query, resolve); });
                        };
                        return iface;
                    },
                    cursorTo: function() { return Promise.resolve(); },
                    moveCursor: function() { return Promise.resolve(); },
                    clearLine: function() { return Promise.resolve(); },
                    clearScreenDown: function() { return Promise.resolve(); },
                };
                globalThis.__requireCache['readline'] = readline;
                globalThis.__requireCache['node:readline'] = readline;
                globalThis.__requireCache['readline/promises'] = readline.promises;
                globalThis.__requireCache['node:readline/promises'] = readline.promises;
            })();

            // path — a real segment-based normalize/resolve/relative (the
            // old ones were regex approximations that didn't actually
            // collapse ".."/"." — e.g. normalize('/a/b/../c') returned
            // '/a/b/../c' unchanged instead of '/a/c' — and had duplicate
            // `join`/`resolve` keys in this same object literal, so half
            // the intended behavior was silently shadowed).
            (function() {
                function splitNormalize(p, sep, isAbsolute) {
                    var segments = p.split(sep);
                    var out = [];
                    for (var i = 0; i < segments.length; i++) {
                        var seg = segments[i];
                        if (seg === '' || seg === '.') continue;
                        if (seg === '..') {
                            if (out.length && out[out.length - 1] !== '..') out.pop();
                            else if (!isAbsolute) out.push('..');
                        } else {
                            out.push(seg);
                        }
                    }
                    return out;
                }
                function makePath(sep, otherSep) {
                    var altRe = otherSep ? new RegExp('\\' + otherSep, 'g') : null;
                    function toSep(p) { return altRe ? p.replace(altRe, sep) : p; }
                    var p = {
                        sep: sep,
                        delimiter: sep === '\\' ? ';' : ':',
                        isAbsolute: function(path) {
                            path = toSep(String(path));
                            if (sep === '\\') return /^[A-Za-z]:[\\/]/.test(path) || path.charAt(0) === '\\';
                            return path.charAt(0) === '/';
                        },
                        normalize: function(path) {
                            path = toSep(String(path || ''));
                            if (path === '') return '.';
                            var isAbsolute = p.isAbsolute(path);
                            var trailingSlash = path.length > 1 && path.charAt(path.length - 1) === sep;
                            var out = splitNormalize(path, sep, isAbsolute);
                            var result = out.join(sep);
                            if (isAbsolute) result = sep + result;
                            if (trailingSlash && result.charAt(result.length - 1) !== sep) result += sep;
                            if (result === '') result = isAbsolute ? sep : '.';
                            return result;
                        },
                        join: function() {
                            var parts = Array.prototype.slice.call(arguments).filter(function(s) { return s; });
                            return p.normalize(parts.join(sep));
                        },
                        resolve: function() {
                            var args = Array.prototype.slice.call(arguments);
                            var resolvedAbsolute = false;
                            var pathsToProcess = [];
                            for (var i = args.length - 1; i >= 0 && !resolvedAbsolute; i--) {
                                var seg = toSep(String(args[i] || ''));
                                if (!seg) continue;
                                pathsToProcess.unshift(seg);
                                resolvedAbsolute = p.isAbsolute(seg);
                            }
                            if (!resolvedAbsolute) {
                                pathsToProcess.unshift((typeof process !== 'undefined' && process.cwd) ? process.cwd() : sep);
                            }
                            var combined = pathsToProcess.join(sep);
                            var out = splitNormalize(combined, sep, true);
                            return sep + out.join(sep);
                        },
                        relative: function(from, to) {
                            from = toSep(String(from));
                            to = toSep(String(to));
                            var fromParts = splitNormalize(from, sep, p.isAbsolute(from));
                            var toParts = splitNormalize(to, sep, p.isAbsolute(to));
                            var i = 0;
                            while (i < fromParts.length && i < toParts.length && fromParts[i] === toParts[i]) i++;
                            var ups = fromParts.length - i;
                            var result = [];
                            for (var j = 0; j < ups; j++) result.push('..');
                            for (var k = i; k < toParts.length; k++) result.push(toParts[k]);
                            return result.length ? result.join(sep) : '.';
                        },
                        dirname: function(path) {
                            path = toSep(String(path || ''));
                            if (!path) return '.';
                            var isAbsolute = p.isAbsolute(path);
                            var parts = splitNormalize(path, sep, isAbsolute);
                            parts.pop();
                            var result = parts.join(sep);
                            if (isAbsolute) result = sep + result;
                            return result || (isAbsolute ? sep : '.');
                        },
                        basename: function(path, ext) {
                            path = toSep(String(path || ''));
                            if (!path) return '';
                            var parts = path.split(sep).filter(Boolean);
                            var last = parts[parts.length - 1] || '';
                            if (ext && last !== ext && last.slice(-ext.length) === ext) return last.slice(0, -ext.length);
                            return last;
                        },
                        extname: function(path) {
                            path = toSep(String(path || ''));
                            var last = path.split(sep).pop() || '';
                            var idx = last.lastIndexOf('.');
                            return idx <= 0 ? '' : last.slice(idx);
                        },
                        format: function(obj) {
                            obj = obj || {};
                            var dir = obj.dir || obj.root || '';
                            var base = obj.base || ((obj.name || '') + (obj.ext || ''));
                            return dir ? (dir + (dir.charAt(dir.length - 1) === sep ? '' : sep) + base) : base;
                        },
                        parse: function(path) {
                            path = toSep(String(path || ''));
                            var isAbsolute = p.isAbsolute(path);
                            var base = p.basename(path);
                            var ext = p.extname(base);
                            var name = ext ? base.slice(0, -ext.length) : base;
                            var dir = p.dirname(path);
                            return { root: isAbsolute ? sep : '', dir: dir, base: base, ext: ext, name: name };
                        },
                    };
                    return p;
                }
                var posixPath = makePath('/', '\\');
                var win32Path = makePath('\\', '/');
                posixPath.posix = posixPath;
                posixPath.win32 = win32Path;
                win32Path.posix = posixPath;
                win32Path.win32 = win32Path;
                globalThis.__requireCache['path'] = posixPath;
                globalThis.__requireCache['path/posix'] = posixPath;
                globalThis.__requireCache['path/win32'] = win32Path;
            })();

            globalThis.__requireCache['buffer'] = globalThis.Buffer;
            globalThis.__requireCache['stream'] = globalThis.__requireCache['stream'] || Stream;

            // ── url ──────────────────────────────────────────────────────────────
            (function() {
                function pathToFileURL(p) {
                    var path = String(p).replace(/\\/g, '/');
                    var encoded = path.split('/').map(encodeURIComponent).join('/');
                    if (encoded.charAt(0) !== '/') encoded = '/' + encoded;
                    return new globalThis.URL('file://' + encoded);
                }
                function fileURLToPath(u) {
                    var href = (u && u.href) ? u.href : String(u);
                    if (href.indexOf('file://') !== 0) {
                        throw new TypeError('The URL must be of scheme file');
                    }
                    return href.slice('file://'.length).split('/').map(decodeURIComponent).join('/');
                }
                globalThis.__requireCache['url'] = {
                    URL: globalThis.URL,
                    URLSearchParams: globalThis.URLSearchParams,
                    pathToFileURL: pathToFileURL,
                    fileURLToPath: fileURLToPath,
                    format: function(u) { return (u && u.href) ? u.href : String(u); },
                };
            })();

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

            // ── http module ──────────────────────────────────────────────────────────
            var httpAgent = function(options) {
                this.options = options || {};
                this.maxSockets = Infinity;
                this.sockets = {};
                this.requests = {};
            };
            httpAgent.prototype.addRequest = function(req, options) {};
            httpAgent.prototype.createConnection = function(options, cb) {};
            httpAgent.prototype.destroy = function() {};
            httpAgent.prototype.free = function(socket) {};
            httpAgent.prototype.getCurrentStatus = function() {};
            httpAgent.prototype.keepSocketAlive = function(socket) {};
            httpAgent.prototype.reuseSocket = function(socket, req) {};
            httpAgent.prototype.onFREE = function(socket) {};
            httpAgent.prototype.onConnected = function(socket, options) {};
            httpAgent.prototype.onRemoved = function(socket) {};
            var httpGlobalAgent = new httpAgent();
            var HTTP_STATUS_CODES = { '100': 'Continue', '200': 'OK', '201': 'Created', '204': 'No Content', '301': 'Moved Permanently', '302': 'Found', '304': 'Not Modified', '400': 'Bad Request', '401': 'Unauthorized', '403': 'Forbidden', '404': 'Not Found', '405': 'Method Not Allowed', '500': 'Internal Server Error', '502': 'Bad Gateway', '503': 'Service Unavailable' };

            // ── IncomingMessage — request bodies are already fully buffered by the
            // native accept loop (see http_server.rs), so headers/method/url/_body
            // are set synchronously; 'data'/'end' are still emitted async for code
            // that expects to stream a request body.
            var httpIncomingMessage = function(socket) {
                Readable.call(this);
                this.socket = socket || {};
                this.connection = this.socket;
                this.httpVersion = '1.1';
                this.httpVersionMajor = 1;
                this.httpVersionMinor = 1;
                this.method = 'GET';
                this.url = '/';
                this.headers = {};
                this._body = '';
                this.complete = true;
            };
            util.inherits(httpIncomingMessage, Readable);
            httpIncomingMessage.prototype.setTimeout = function() { return this; };
            httpIncomingMessage.prototype.destroy = function() { return this; };

            // ── ServerResponse — buffers writes and flushes as one __httpRespond(Bytes)
            // call on end(), since the native side writes headers+body together.
            var httpServerResponse = function(req) {
                Writable.call(this);
                // Real usage: constructed internally by the accept loop with a
                // req whose `socket._connId` names the native connection to
                // write to. Node API compat also allows `new ServerResponse(req)`
                // standalone (e.g. for unit tests) — _connId stays undefined
                // there, fine as long as .end() is never called on it.
                this._connId = req && req.socket && req.socket._connId;
                this.statusCode = 200;
                this.statusMessage = '';
                this._headers = {};
                this._chunks = [];
                this.finished = false;
                this.headersSent = false;
            };
            util.inherits(httpServerResponse, Writable);
            httpServerResponse.prototype.setHeader = function(name, value) { this._headers[name] = value; return this; };
            httpServerResponse.prototype.getHeader = function(name) { return this._headers[name]; };
            httpServerResponse.prototype.removeHeader = function(name) { delete this._headers[name]; return this; };
            httpServerResponse.prototype.hasHeader = function(name) { return Object.prototype.hasOwnProperty.call(this._headers, name); };
            httpServerResponse.prototype.getHeaders = function() { return this._headers; };
            httpServerResponse.prototype.writeHead = function(statusCode, statusMessage, headers) {
                this.statusCode = statusCode;
                if (typeof statusMessage === 'object' && statusMessage !== null) { headers = statusMessage; statusMessage = undefined; }
                this.statusMessage = statusMessage || HTTP_STATUS_CODES[String(statusCode)] || '';
                if (headers) {
                    for (var k in headers) if (Object.prototype.hasOwnProperty.call(headers, k)) this._headers[k] = headers[k];
                }
                this.headersSent = true;
                return this;
            };
            httpServerResponse.prototype.write = function(chunk, encoding, callback) {
                if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
                if (chunk !== undefined) this._chunks.push(chunk);
                if (typeof callback === 'function') setTimeout(callback, 0);
                return true;
            };
            httpServerResponse.prototype.end = function(chunk, encoding, callback) {
                var self = this;
                if (typeof chunk === 'function') { callback = chunk; chunk = undefined; }
                if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
                if (chunk !== undefined) this._chunks.push(chunk);
                if (this.finished) { if (callback) setTimeout(callback, 0); return this; }
                this.finished = true;

                var headersJson = JSON.stringify(this._headers);
                var isBinary = this._chunks.some(function(c) {
                    return c instanceof Uint8Array || (typeof Buffer !== 'undefined' && Buffer.isBuffer(c));
                });
                if (isBinary) {
                    var parts = this._chunks.map(function(c) {
                        if (typeof c === 'string') return typeof Buffer !== 'undefined' ? Buffer.from(c) : new TextEncoder().encode(c);
                        return c;
                    });
                    var total = parts.reduce(function(n, p) { return n + p.length; }, 0);
                    var merged = new Uint8Array(total);
                    var off = 0;
                    parts.forEach(function(p) { merged.set(p, off); off += p.length; });
                    __httpRespondBytes(this._connId, this.statusCode, this.statusMessage, headersJson, merged);
                } else {
                    __httpRespond(this._connId, this.statusCode, this.statusMessage, headersJson, this._chunks.join(''));
                }
                setTimeout(function() { self.emit('finish'); self.emit('close'); if (typeof callback === 'function') callback(); }, 0);
                return this;
            };
            httpServerResponse.prototype.setTimeout = function() { return this; };
            httpServerResponse.prototype.destroy = function() { return this; };
            httpServerResponse.prototype.flushHeaders = function() {};

            // ── Server — real listener backed by __httpListen/__httpAcceptPoll.
            var httpServer = function(opts, requestListener) {
                EventEmitter.call(this);
                if (typeof opts === 'function') { requestListener = opts; opts = {}; }
                if (requestListener) this.on('request', requestListener);
                this._id = null;
                this._pollTimer = null;
                this.listening = false;
                this._port = 0;
                this._host = '0.0.0.0';
            };
            util.inherits(httpServer, EventEmitter);
            httpServer.prototype.listen = function(port, hostname, backlog, callback) {
                var self = this;
                if (typeof port === 'object' && port !== null) {
                    var opts = port;
                    callback = hostname;
                    port = opts.port || 0;
                    hostname = opts.host || opts.hostname;
                }
                if (typeof hostname === 'function') { callback = hostname; hostname = undefined; }
                if (typeof backlog === 'function') { callback = backlog; }
                hostname = hostname || '0.0.0.0';
                port = port || 0;
                if (typeof callback === 'function') this.once('listening', callback);

                var result = __httpListen(port, hostname);
                if (typeof result !== 'number') {
                    setTimeout(function() { self.emit('error', result); }, 0);
                    return self;
                }
                self._id = result;
                self._port = port;
                self._host = hostname;
                self.listening = true;
                self._pollTimer = setInterval(function() {
                    var raw;
                    while ((raw = __httpAcceptPoll(self._id)) !== null && raw !== undefined) {
                        var r;
                        try { r = JSON.parse(raw); } catch (e) { continue; }
                        var req = new httpIncomingMessage({ remoteAddress: r.remoteAddress, _connId: r.conn_id });
                        req.method = r.method;
                        req.url = r.url;
                        req.headers = r.headers || {};
                        req._body = r.body || '';
                        var res = new httpServerResponse(req);
                        self.emit('request', req, res);
                        self.emit('connection', req.socket);
                        (function(req) {
                            setTimeout(function() {
                                if (req._body) req.emit('data', typeof Buffer !== 'undefined' ? Buffer.from(req._body) : req._body);
                                req.emit('end');
                            }, 0);
                        })(req);
                    }
                }, 5);
                setTimeout(function() { self.emit('listening'); }, 0);
                return self;
            };
            httpServer.prototype.close = function(callback) {
                var self = this;
                if (this._pollTimer) { clearInterval(this._pollTimer); this._pollTimer = null; }
                if (this._id !== null) { __httpClose(this._id); }
                this.listening = false;
                if (typeof callback === 'function') this.once('close', callback);
                setTimeout(function() { self.emit('close'); }, 0);
                return this;
            };
            httpServer.prototype.address = function() {
                if (!this.listening) return null;
                return { port: this._port, address: this._host, family: this._host.indexOf(':') !== -1 ? 'IPv6' : 'IPv4' };
            };
            httpServer.prototype.getConnections = function(cb) { cb(null, 0); };
            httpServer.prototype.ref = function() { return this; };
            httpServer.prototype.unref = function() { return this; };
            httpServer.prototype.listenOnServerHandler = function(socket) {};

            var httpOutgoingMessage = function() { EventEmitter.call(this); };
            util.inherits(httpOutgoingMessage, EventEmitter);
            httpOutgoingMessage.prototype.write = function(chunk, encoding, callback) { return true; };
            httpOutgoingMessage.prototype.end = function(chunk, encoding, callback) {};
            httpOutgoingMessage.prototype.destroy = function(err) {};
            httpOutgoingMessage.prototype.setTimeout = function(msecs, callback) { return this; };

            function createServer(opts, requestListener) {
                return new httpServer(opts, requestListener);
            }

            var httpModule = {
                createServer: createServer,
                globalAgent: httpGlobalAgent,
                Agent: httpAgent,
                Server: httpServer,
                OutgoingMessage: httpOutgoingMessage,
                IncomingMessage: httpIncomingMessage,
                ServerResponse: httpServerResponse,
                request: function(options, callback) { return new httpOutgoingMessage(); },
                get: function(options, callback) { return new httpOutgoingMessage(); },
                ClientRequest: httpOutgoingMessage,
                maxHeaderSize: 16384,
                STATUS_CODES: HTTP_STATUS_CODES
            };
            globalThis.__requireCache['http'] = httpModule;
            globalThis.__requireCache['node:http'] = httpModule;

            // ── https module ─────────────────────────────────────────────────────────
            var httpsModule = {
                createServer: function(opts, requestListener) { return createServer(opts, requestListener); },
                globalAgent: new httpAgent(),
                request: function(options, callback) { return new httpOutgoingMessage(); },
                get: function(options, callback) { return new httpOutgoingMessage(); }
            };
            globalThis.__requireCache['https'] = httpsModule;
            globalThis.__requireCache['node:https'] = httpsModule;

            // ── reflect-metadata polyfill ──────────────────────────────────────────
            (function() {
                var metadataMap = new WeakMap();
                var symbol = Symbol.for('Reflect.metadata');

                function assertFunction(fn) {
                    if (typeof fn !== 'function') throw new TypeError('argument must be a function');
                }

                var ReflectMetadata = {
                    defineMetadata: function(metadataKey, metadataValue, target, targetKey) {
                        assertFunction(target);
                        if (targetKey !== undefined && typeof targetKey !== 'symbol' && typeof targetKey !== 'string') {
                            throw new TypeError('targetKey must be a symbol or string');
                        }
                        var meta = metadataMap.get(target);
                        if (!meta) {
                            meta = {};
                            metadataMap.set(target, meta);
                        }
                        var key = targetKey || symbol;
                        if (!meta[key]) meta[key] = {};
                        meta[key][metadataKey] = metadataValue;
                        return target;
                    },
                    hasMetadata: function(metadataKey, target, targetKey) {
                        assertFunction(target);
                        var meta = metadataMap.get(target);
                        if (!meta) return false;
                        var key = targetKey || symbol;
                        var metadata = meta[key];
                        if (!metadata) return false;
                        if (targetKey === undefined) {
                            for (var k in metadata) { if (k !== symbol && metadata.hasOwnProperty(k)) return true; }
                            return false;
                        }
                        return metadata.hasOwnProperty(metadataKey);
                    },
                    hasOwnMetadata: function(metadataKey, target, targetKey) {
                        assertFunction(target);
                        var meta = metadataMap.get(target);
                        if (!meta) return false;
                        var key = targetKey || symbol;
                        var metadata = meta[key];
                        return metadata ? metadata.hasOwnProperty(metadataKey) : false;
                    },
                    getMetadata: function(metadataKey, target, targetKey) {
                        assertFunction(target);
                        var meta = metadataMap.get(target);
                        if (!meta) return undefined;
                        var key = targetKey || symbol;
                        var metadata = meta[key];
                        if (!metadata && targetKey === undefined) {
                            for (var k in meta) {
                                if (k !== symbol && meta[k].hasOwnProperty(metadataKey)) {
                                    return meta[k][metadataKey];
                                }
                            }
                            return undefined;
                        }
                        return metadata ? metadata[metadataKey] : undefined;
                    },
                    getOwnMetadata: function(metadataKey, target, targetKey) {
                        assertFunction(target);
                        var meta = metadataMap.get(target);
                        if (!meta) return undefined;
                        var key = targetKey || symbol;
                        var metadata = meta[key];
                        return metadata ? metadata[metadataKey] : undefined;
                    },
                    deleteMetadata: function(metadataKey, target, targetKey) {
                        assertFunction(target);
                        var meta = metadataMap.get(target);
                        if (!meta) return false;
                        var key = targetKey || symbol;
                        var metadata = meta[key];
                        if (!metadata) return false;
                        if (targetKey !== undefined) {
                            delete metadata[metadataKey];
                            return true;
                        }
                        var found = false;
                        for (var k in metadata) {
                            if (k !== symbol && metadata.hasOwnProperty(metadataKey)) {
                                found = true;
                            }
                        }
                        return found;
                    },
                    getMetadataKeys: function(target, targetKey) {
                        assertFunction(target);
                        var meta = metadataMap.get(target);
                        if (!meta) return [];
                        var keys = [];
                        if (targetKey === undefined) {
                            for (var k in meta) {
                                if (k !== symbol) {
                                    var metadata = meta[k];
                                    for (var mk in metadata) {
                                        if (metadata.hasOwnProperty(mk) && keys.indexOf(mk) === -1) {
                                            keys.push(mk);
                                        }
                                    }
                                }
                            }
                            return keys;
                        }
                        var metadata = meta[targetKey || symbol];
                        if (!metadata) return [];
                        for (var mk in metadata) {
                            if (metadata.hasOwnProperty(mk) && keys.indexOf(mk) === -1) {
                                keys.push(mk);
                            }
                        }
                        return keys;
                    },
                    getOwnMetadataKeys: function(target, targetKey) {
                        assertFunction(target);
                        var meta = metadataMap.get(target);
                        if (!meta) return [];
                        var keys = [];
                        var key = targetKey || symbol;
                        var metadata = meta[key];
                        if (!metadata) return [];
                        for (var mk in metadata) {
                            if (metadata.hasOwnProperty(mk) && keys.indexOf(mk) === -1) {
                                keys.push(mk);
                            }
                        }
                        return keys;
                    },
                    defineMetadataMetadata: function(metadataKey, metadataValue, target, targetKey) {
                        assertFunction(target);
                        return this.defineMetadata(metadataKey, metadataValue, target, targetKey);
                    },
                    hasMetadataMetadata: function(metadataKey, target, targetKey) {
                        assertFunction(target);
                        var meta = metadataMap.get(target);
                        if (!meta) return false;
                        var key = targetKey || symbol;
                        var metadata = meta[key];
                        return metadata ? metadata.hasOwnProperty(metadataKey) : false;
                    }
                };

                globalThis.__requireCache['reflect-metadata'] = {
                    __esModule: true,
                    default: ReflectMetadata,
                    Reflect: ReflectMetadata
                };
                globalThis.__requireCache['reflect-metadata/Reflect'] = ReflectMetadata;

                Object.defineProperties(Reflect, {
                    defineMetadata: { value: ReflectMetadata.defineMetadata, writable: true, enumerable: true, configurable: true },
                    hasMetadata: { value: ReflectMetadata.hasMetadata, writable: true, enumerable: true, configurable: true },
                    hasOwnMetadata: { value: ReflectMetadata.hasOwnMetadata, writable: true, enumerable: true, configurable: true },
                    getMetadata: { value: ReflectMetadata.getMetadata, writable: true, enumerable: true, configurable: true },
                    getOwnMetadata: { value: ReflectMetadata.getOwnMetadata, writable: true, enumerable: true, configurable: true },
                    deleteMetadata: { value: ReflectMetadata.deleteMetadata, writable: true, enumerable: true, configurable: true },
                    getMetadataKeys: { value: ReflectMetadata.getMetadataKeys, writable: true, enumerable: true, configurable: true },
                    getOwnMetadataKeys: { value: ReflectMetadata.getOwnMetadataKeys, writable: true, enumerable: true, configurable: true },
                    defineMetadataMetadata: { value: ReflectMetadata.defineMetadataMetadata, writable: true, enumerable: true, configurable: true },
                    hasMetadataMetadata: { value: ReflectMetadata.hasMetadataMetadata, writable: true, enumerable: true, configurable: true },
                    metadata: { value: function(metadataKey, metadataValue) { return function(target, targetPropertyKey) { if (targetPropertyKey === undefined) { ReflectMetadata.defineMetadata(metadataKey, metadataValue, target); } else { ReflectMetadata.defineMetadata(metadataKey, metadataValue, target, targetPropertyKey); } }; }, writable: true, enumerable: true, configurable: true }
                });
            })();

            // ── async_hooks — AsyncLocalStorage / AsyncResource ─────────────────────
            // __acsGet/__acsSet (installed by async_context.rs, before this file runs)
            // read/write V8's ContinuationPreservedEmbedderData directly, which V8
            // itself snapshots/restores across await and .then() continuations. Each
            // AsyncLocalStorage instance gets a unique key into a shared frame object
            // stored in that slot, so multiple instances (and nested/concurrent runs)
            // stay independent without any promise/timer monkey-patching.
            var __acsNextId = 1;

            function AsyncLocalStorage() {
                this._id = '__als' + (__acsNextId++);
            }
            AsyncLocalStorage.prototype.run = function(store, fn) {
                var args = Array.prototype.slice.call(arguments, 2);
                var prevFrame = __acsGet();
                var newFrame = Object.assign({}, prevFrame);
                newFrame[this._id] = store;
                __acsSet(newFrame);
                try {
                    return fn.apply(null, args);
                } finally {
                    __acsSet(prevFrame);
                }
            };
            AsyncLocalStorage.prototype.exit = function(fn) {
                var args = Array.prototype.slice.call(arguments, 1);
                var prevFrame = __acsGet();
                var newFrame = Object.assign({}, prevFrame);
                delete newFrame[this._id];
                __acsSet(newFrame);
                try {
                    return fn.apply(null, args);
                } finally {
                    __acsSet(prevFrame);
                }
            };
            AsyncLocalStorage.prototype.enterWith = function(store) {
                var newFrame = Object.assign({}, __acsGet());
                newFrame[this._id] = store;
                __acsSet(newFrame);
            };
            AsyncLocalStorage.prototype.disable = function() {
                var newFrame = Object.assign({}, __acsGet());
                delete newFrame[this._id];
                __acsSet(newFrame);
            };
            AsyncLocalStorage.prototype.getStore = function() {
                var frame = __acsGet();
                if (!frame || typeof frame !== 'object') return undefined;
                return Object.prototype.hasOwnProperty.call(frame, this._id) ? frame[this._id] : undefined;
            };

            function AsyncResource(type) {
                this.type = type;
                this._frame = __acsGet();
            }
            AsyncResource.prototype.runInAsyncScope = function(fn) {
                var args = Array.prototype.slice.call(arguments, 1);
                var prevFrame = __acsGet();
                __acsSet(this._frame);
                try {
                    return fn.apply(null, args);
                } finally {
                    __acsSet(prevFrame);
                }
            };
            AsyncResource.prototype.emitDestroy = function() { return this; };
            AsyncResource.prototype.asyncId = function() { return 0; };
            AsyncResource.prototype.triggerAsyncId = function() { return 0; };
            AsyncResource.bind = function(fn, type) {
                var res = new AsyncResource(type || fn.name || 'bound-anonymous-fn');
                return function() {
                    var args = arguments, self = this;
                    return res.runInAsyncScope(function() { return fn.apply(self, args); });
                };
            };

            globalThis.__requireCache['async_hooks'] = {
                AsyncLocalStorage: AsyncLocalStorage,
                AsyncResource: AsyncResource,
                executionAsyncId: function() { return 0; },
                triggerAsyncId: function() { return 0; },
                executionAsyncResource: function() { return {}; },
                createHook: function() {
                    return { enable: function() { return this; }, disable: function() { return this; } };
                },
            };
            globalThis.__requireCache['node:async_hooks'] = globalThis.__requireCache['async_hooks'];
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
        // Mirrors the Rust-side thread-local scope (vvva_permissions::scope)
        // so wrapped builtins can read/restore the *previous* scope on their
        // own (see __wrapForScope below) without a round-trip native getter.
        globalThis.__currentCallerScope = globalThis.__currentCallerScope || '.';

        function bareName(specifier) {
            return specifier.indexOf('node:') === 0 ? specifier.slice(5) : specifier;
        }

        // Backs transpiled dynamic `import(x)` calls: static_esm_to_cjs (Rust
        // side) only rewrites *declarations* (`import x from 'y'`), since
        // `import(x)` is an expression; the transpiler instead rewrites it to
        // `__importAsync(x)`, resolved here to a Promise of the same object
        // require(x) would return, with `.default` guaranteed present — the
        // async equivalent of the `.default` unwrap static default-imports
        // already get in convert_import() (transpiler.rs).
        function __makeImportAsync(requireFn) {
            return function(specifier) {
                return new Promise(function(resolve, reject) {
                    try {
                        var mod = requireFn(specifier);
                        if (mod && typeof mod === 'object' && mod.default === undefined) {
                            mod = Object.assign({}, mod, { default: mod });
                        }
                        resolve(mod);
                    } catch (e) {
                        reject(e);
                    }
                });
            };
        }

        // Package-level permission scoping (package.json["3va"].permissions.<name>) ──
        // Extracts the innermost node_modules package name from a directory
        // path, e.g. ".../node_modules/express/lib" -> "express",
        // ".../node_modules/@babel/core/lib" -> "@babel/core". Nested deps use
        // the last (innermost) segment: the package whose own code is
        // literally about to call require(), not one of its ancestors.
        // Paths outside any node_modules (the app's own code) map to '.'.
        function __pkgScopeFor(dir) {
            if (!dir || typeof dir !== 'string') return '.';
            var re = /[\/\\]node_modules[\/\\](@[^\/\\]+[\/\\][^\/\\]+|[^\/\\]+)/g;
            var last = null, m;
            while ((m = re.exec(dir)) !== null) { last = m[1]; }
            return last || '.';
        }

        // Only builtins that themselves perform capability-gated native calls
        // (perms().check(...) in the Rust bindings) need scoping — wrapping
        // every required module would be pure overhead for no benefit, since
        // plain JS/data modules never touch PermissionState.
        var __SCOPE_GATED_MODULES = {
            fs: true, 'fs/promises': true, net: true, tls: true,
            dgram: true, child_process: true,
        };

        // Shallow-wraps lowercase-named function properties (heuristic for
        // "plain function", as opposed to a PascalCase constructor/class like
        // net.Socket — wrapping a constructor with a plain closure would
        // break `instanceof` and is skipped) so each call brackets itself
        // with the requesting package's scope. Recurses one level into plain
        // nested objects (covers fs.promises.readFile etc.) without touching
        // arrays or class instances.
        function __wrapForScope(obj, scopeName, depth) {
            if (!obj || typeof obj !== 'object' || Array.isArray(obj) || depth > 1) return obj;
            var out = {};
            for (var key in obj) {
                var val = obj[key];
                if (typeof val === 'function' && key.length > 0
                    && key.charAt(0) === key.charAt(0).toLowerCase()) {
                    out[key] = (function(fn) {
                        return function() {
                            var prevScope = globalThis.__currentCallerScope;
                            globalThis.__currentCallerScope = scopeName;
                            __setCallerScope(scopeName);
                            try {
                                return fn.apply(this, arguments);
                            } finally {
                                globalThis.__currentCallerScope = prevScope;
                                __setCallerScope(prevScope);
                            }
                        };
                    })(val);
                } else if (val && typeof val === 'object' && !Array.isArray(val)) {
                    out[key] = __wrapForScope(val, scopeName, depth + 1);
                } else {
                    out[key] = val;
                }
            }
            return out;
        }

        // Memoized per (module, scope) pair — requireFrom is called every
        // time a module does `require('fs')`, so this avoids re-wrapping on
        // every call.
        var __scopedModuleCache = Object.create(null);
        function __scopedModule(bare, mod, scopeName) {
            if (scopeName === '.' || !Object.prototype.hasOwnProperty.call(__SCOPE_GATED_MODULES, bare)) {
                return mod;
            }
            var cacheKey = bare + ' ' + scopeName;
            if (!Object.prototype.hasOwnProperty.call(__scopedModuleCache, cacheKey)) {
                __scopedModuleCache[cacheKey] = __wrapForScope(mod, scopeName, 0);
            }
            return __scopedModuleCache[cacheKey];
        }

        function requireFrom(specifier, dir) {
            var bare = bareName(specifier);
            if (Object.prototype.hasOwnProperty.call(globalThis.__requireCache, specifier)) {
                return __scopedModule(bare, globalThis.__requireCache[specifier], __pkgScopeFor(dir));
            }
            if (Object.prototype.hasOwnProperty.call(globalThis.__requireCache, bare)) {
                return __scopedModule(bare, globalThis.__requireCache[bare], __pkgScopeFor(dir));
            }
            if (Object.prototype.hasOwnProperty.call(globalThis.__fallbackModules, specifier)) {
                return globalThis.__fallbackModules[specifier];
            }

            var resolved = __requireResolve(specifier, dir);

            if (Object.prototype.hasOwnProperty.call(globalThis.__loadedModules, resolved)) {
                return globalThis.__loadedModules[resolved].exports;
            }

            // require() of a .mjs file is always an error in Node — .mjs
            // marks a file as ESM-only regardless of its actual content, so
            // this must be an extension check, not the source_is_esm()
            // content sniff used elsewhere (a .cjs file with import/export-
            // looking text must NOT hit this, and does not: it's excluded
            // by extension here too).
            if (resolved.slice(-4) === '.mjs') {
                var esmErr = new Error("Must use import to load ES Module: " + resolved);
                esmErr.code = 'ERR_REQUIRE_ESM';
                throw esmErr;
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
            // Backs this module's own transpiled dynamic `import(x)` calls —
            // must resolve relative to *this* module's directory via
            // localRequire, not the entry script's, same as require() above.
            var localImportAsync = __makeImportAsync(localRequire);

            var source = __readFile(resolved);
            try {
                // A required file's own `import.meta.url` must be ITS path,
                // not the entry script's — replace_import_meta() rewrites
                // `import.meta.url` to the bare identifier `__vvva_meta_url__`,
                // so shadowing it as a local parameter here (instead of relying
                // on the single globalThis.__vvva_meta_url__ set once for the
                // entry point) gives each required module its own value.
                var ownMetaUrl = /^[A-Za-z]:[\\/]/.test(resolved)
                    ? 'file:///' + resolved.replace(/\\/g, '/')
                    : 'file://' + resolved;
                var fn = new Function('exports', 'module', 'require', '__filename', '__dirname', '__vvva_meta_url__', '__importAsync', source);
                fn(mod.exports, mod, localRequire, resolved, moduleDir, ownMetaUrl, localImportAsync);
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

        // Entry-point script's own transpiled dynamic `import(x)` calls —
        // required modules get their own per-directory version instead (see
        // localImportAsync in requireFrom), passed as a shadowing parameter.
        globalThis.__importAsync = __makeImportAsync(function(specifier) {
            return requireFrom(specifier, globalThis.__dirname || undefined);
        });
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
    let joined = base.join(path);
    // Only used for bare specifiers (relative/absolute ones never reach this
    // call site — see esm.rs's resolve_esm_from_dir). A bare specifier is
    // only actually *at* `base/specifier` when that file exists; otherwise
    // it's an npm package name and the caller must fall back to walking
    // node_modules instead of treating this non-existent path as resolved.
    if joined.is_file() {
        Ok(joined)
    } else {
        Err(format!("not found: {}", joined.display()))
    }
}

pub fn split_bare_specifier(name: &str) -> (&str, Option<&str>) {
    // Scoped packages ("@scope/name[/subpath]") have the package name span
    // the first *two* path segments, not just up to the first '/' — that
    // previously split "@myorg/helpers" into pkg "@myorg" + subpath
    // "helpers", which doesn't exist in node_modules. Skip past the scope
    // segment before looking for the pkg/subpath boundary.
    let search_from = if name.starts_with('@') {
        match name.find('/') {
            Some(i) => i + 1,
            None => return (name, None),
        }
    } else {
        0
    };
    match name[search_from..].find('/') {
        Some(rel_pos) => {
            let pkg_end = search_from + rel_pos;
            let pkg = &name[..pkg_end];
            let subpath = name.get(pkg_end + 1..).filter(|s| !s.is_empty());
            (pkg, subpath)
        }
        None => (name, None),
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
