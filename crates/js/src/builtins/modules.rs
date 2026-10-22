// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use v8::{
    ContextScope, Function, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue,
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
        // node:console — set early so it's available before js_code's IIFE runs.
        (function() {
            var _con = globalThis.console || { log: function(){}, error: function(){}, warn: function(){} };
            globalThis.__requireCache['console'] = globalThis.__requireCache['console'] || _con;
            globalThis.__requireCache['node:console'] = _con;
        })();
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

        Error.prepareStackTrace = function(err, stack) {
            var lines = [err.toString()];
            for (var i = 0; i < stack.length; i++) {
                var frame = stack[i];
                var fileName = frame.getFileName() || '';
                var line = frame.getLineNumber();
                var col = frame.getColumnNumber();
                try {
                    var mapJson = __getSourceMap(fileName);
                    if (mapJson) {
                        var result = __applySourceMap(fileName, line, col);
                        if (result) {
                            try {
                                var mapped = JSON.parse(result);
                                if (mapped.source) fileName = mapped.source;
                                if (mapped.line) line = mapped.line;
                                if (mapped.col) col = mapped.col;
                            } catch(e) {}
                        }
                    }
                } catch(e) {}
                // V8 knows the receiver's type of a frame; Node's own stack
                // format uses it for the frames without a name
                // (`Object.<anonymous>`, the frame of a CommonJS module).
                var name = frame.getFunctionName();
                if (!name) {
                    var typeName = (typeof frame.getTypeName === 'function') ? frame.getTypeName() : '';
                    name = typeName ? (typeName + '.<anonymous>') : '<anonymous>';
                }
                lines.push('    at ' + name + ' (' + fileName + ':' + line + ':' + col + ')');
            }
            return lines.join('\n');
        };
    "#;
    crate::builtins::code_cache::bootstrap_js(scope, "require-intro", source_code)?;

    // __vvvaCompileModule(source, filename) — compile a CommonJS module the
    // way Node does: the file's own text is the body of a function with the
    // module's parameters, and the script origin carries the file name. A
    // stack trace then points at the file with the line and column of the
    // source, and the frame of the module is `Object.<anonymous>`, as in Node.
    let compile_module_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let source = args.get(0).to_rust_string_lossy(scope);
            let filename = args.get(1).to_rust_string_lossy(scope);
            let params_csv = args.get(2).to_rust_string_lossy(scope);
            let script_id = args.get(3).int32_value(scope).unwrap_or(-1);
            let param_handles: Vec<v8::Local<v8::String>> = params_csv
                .split(',')
                .filter(|p| !p.is_empty())
                .map(|p| v8::String::new(scope, p).unwrap())
                .collect();
            let resource: v8::Local<v8::Value> = v8::String::new(scope, &filename).unwrap().into();
            let origin = v8::ScriptOrigin::new(
                scope, resource, 0, 0, false, script_id, None, false, false, false, None,
            );
            let source_str = v8::String::new(scope, &source).unwrap();
            let mut source_obj = v8::script_compiler::Source::new(source_str, Some(&origin));
            if let Some(f) = v8::script_compiler::compile_function(
                scope,
                &mut source_obj,
                &param_handles,
                &[],
                v8::script_compiler::CompileOptions::EagerCompile,
                v8::script_compiler::NoCacheReason::NoReason,
            ) {
                rv.set(f.into());
            }
            // When the compilation fails, the error is pending in the isolate:
            // leaving the return value unset propagates it to the caller.
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__vvvaCompileModule").unwrap().into(),
        compile_module_fn.into(),
    );

    // __vvvaTranspileJs(source) — the fallback for a `.js` file that is not
    // plain JavaScript (JSX, Flow, …): the loader compiles the file as it is
    // first, so a stack trace points at the file's own lines, and only
    // transpiles it when V8 cannot parse it.
    let transpile_js_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let source = args.get(0).to_rust_string_lossy(scope);
            let transpiled = crate::transpiler::transpile_js(&source);
            rv.set(V8String::new(scope, &transpiled).unwrap().into());
        },
    )
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__vvvaTranspileJs").unwrap().into(),
        transpile_js_fn.into(),
    );

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
                Ok(mut source) => {
                    // Strip shebang so V8 doesn't see it as a syntax error.
                    if source.starts_with("#!") {
                        if let Some(nl) = source.find('\n') {
                            source = source[nl + 1..].to_string();
                        } else {
                            source = String::new();
                        }
                    }
                    let is_jsx = path_str.ends_with(".tsx") || path_str.ends_with(".jsx");
                    // A required file with real static import/export syntax
                    // must go through ESM→CJS conversion too — same fix as
                    // eval_file() in lib.rs, same reason: new Function(source)
                    // runs this as a plain function body, which V8 rejects if
                    // it contains bare `import`/`export` declarations.
                    let is_esm =
                        path_str.ends_with(".mjs") || crate::esm::source_is_esm(&source, &path_str);
                    let _zod_core_debug = path_str.contains("zod") && path_str.contains("core.js");
                    if _zod_core_debug {
                        eprintln!("[ZOD-CORE] path={} is_esm={}", path_str, is_esm);
                    }
                    if path_str.contains("micromark-util-character") {
                        eprintln!(
                            "[MMARK] loaded path={} is_esm={} first_export={:?}",
                            path_str,
                            is_esm,
                            source.lines().find(|l| l.trim().starts_with("export "))
                        );
                    }
                    let _mmark_debug = path_str.contains("micromark-util-character");
                    let transpiled = if path_str.ends_with(".cjs")
                        || path_str.ends_with(".json")
                        || source.contains("@exodus/bytes")
                        || path_str.contains("/dist/cjs/")
                        || path_str.contains("/cjs/dist/")
                    {
                        source
                    } else if is_esm {
                        // Marker (same line, so stack-trace lines don't shift): the JS loader
                        // evaluates static imports first for ES modules.
                        format!(
                            "/*__3va_esm__*/ {}",
                            crate::transpiler::transpile_to_cjs(&source, is_jsx)
                        )
                    } else if is_jsx {
                        crate::transpiler::transpile_jsx(&source)
                    } else if path_str.ends_with(".ts")
                        || path_str.ends_with(".mts")
                        || path_str.ends_with(".cts")
                    {
                        crate::transpiler::transpile(&source)
                    } else {
                        // A plain JavaScript file is used as it is: the
                        // transpiler re-prints the code (and reformats it), and
                        // then the line and column of a stack trace would not
                        // be the ones of the file. If the file turns out not to
                        // be plain JavaScript (JSX, Flow, …), the loader
                        // transpiles it and compiles that instead.
                        source
                    };
                    if _zod_core_debug {
                        let has_bare = transpiled.lines().any(|l| l.trim().starts_with("export "));
                        eprintln!(
                            "[ZOD-CORE] transpiled len={} has_bare_export={}",
                            transpiled.len(),
                            has_bare
                        );
                    }
                    if _mmark_debug {
                        eprintln!(
                            "[MMARK] transpiled len={} has_export={}",
                            transpiled.len(),
                            transpiled.contains("\nexport ")
                        );
                    }
                    if std::env::var("DEBUG_TRANSPILE").is_ok() {
                        let has_bare = transpiled.lines().any(|l| l.trim().starts_with("export "));
                        if has_bare {
                            let ex: Vec<_> = transpiled
                                .lines()
                                .filter(|l| l.trim().starts_with("export "))
                                .take(2)
                                .collect();
                            eprintln!("[DEBUG] Bare: {} — {:?}", path_str, ex);
                        }
                        if path_str.contains("chunks/config.js") {
                            eprintln!("[DEBUG] Wrote chunks/config.js: {}", path_str);
                        }
                        if path_str.contains("property-information/lib/html.js") {
                            eprintln!(
                                "[DEBUG] Wrote html.js transpiled to /tmp/html_transpiled.js"
                            );
                        }
                        if path_str.contains("micromark-util-character/index.js") {
                            eprintln!(
                                "[DEBUG] Wrote micromark-util-character transpiled, has_export={}",
                                transpiled.contains("export ")
                            );
                        }
                    }

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

    // ── __drainMicrotasks() ───────────────────────────────────────────────────
    // Lets the synchronous require() loader run a top-level-await module's
    // microtask-only awaits (e.g. `await import(...)`) to completion before
    // handing its exports to the importer. No-op when already inside a microtask.
    let drain_fn = Function::new(
        scope,
        |scope: &mut PinScope<'_, '_>, _args: FunctionCallbackArguments, _rv: ReturnValue| {
            scope.perform_microtask_checkpoint();
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__drainMicrotasks").unwrap().into(),
        drain_fn.unwrap().into(),
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

    // ── __SCOPE_GATED_MODULES ────────────────────────────────────────────────
    // Authoritative list (kept here in Rust, next to the capability checks
    // they gate) of builtin modules the require() wrapper must scope per
    // package. A capability-gated builtin missing from this set runs its
    // permission checks in the root scope, silently ignoring the package's
    // deny-* rules — the VULN-03 bypass. The wrapper itself still skips
    // PascalCase constructors (net.Socket, http.Server, ...) to preserve
    // `instanceof`, so modules exposing only such constructors (ssh2, imap,
    // ftp, pop3, irc) are listed for completeness but wrap to a no-op.
    const SCOPE_GATED_MODULES: &[&str] = &[
        // filesystem
        "fs",
        "fs/promises",
        // network
        "net",
        "tls",
        "dgram",
        "mqtt",
        "http",
        "http2",
        "https",
        "dns",
        "dns/promises",
        // network via protocol clients (PascalCase-only exports are skipped)
        "ssh2",
        "imap",
        "ftp",
        "pop3",
        "irc",
        "webrtc",
        // process
        "child_process",
        "worker_threads",
        // native code loading
        "ffi",
    ];
    let gated_map = SCOPE_GATED_MODULES
        .iter()
        .map(|m| format!("{}: true", serde_json::to_string(m).unwrap()))
        .collect::<Vec<_>>()
        .join(",");
    let gated_script = format!("globalThis.__SCOPE_GATED_MODULES = {{{}}};", gated_map);
    let _ = crate::builtins::code_cache::bootstrap_js(scope, "scope-gated-modules", &gated_script);

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

            // For node: builtins, skip filesystem resolution and return the
            // specifier itself as a sentinel.  requireFrom's fast-path below
            // catches it and looks up __requireCache / returns an empty stub.
            if specifier.starts_with("node:") {
                let result = V8String::new(scope, &specifier).unwrap();
                rv.set(result.into());
                return;
            }
            // file:// URLs come from Vite's fetchModule (pathToFileURL). Strip
            // the scheme so the path resolver handles them as absolute paths.
            let specifier_stripped;
            let specifier = if specifier.starts_with("file:///") {
                specifier_stripped = specifier[7..].to_string(); // "file://" → ""
                &specifier_stripped
            } else if specifier.starts_with("file://") {
                specifier_stripped = specifier[6..].to_string();
                &specifier_stripped
            } else {
                &specifier
            };
            let resolved = crate::esm::resolve_esm_from_dir(&dir, specifier);
            if !resolved.is_file() {
                // Bare node.js built-in names (without node: prefix) also
                // return as sentinel so requireFrom can find them in cache.
                const NODE_BUILTINS: &[&str] = &[
                    "assert",
                    "async_hooks",
                    "buffer",
                    "child_process",
                    "cluster",
                    "console",
                    "crypto",
                    "dgram",
                    "diagnostics_channel",
                    "dns",
                    "domain",
                    "events",
                    "fs",
                    "http",
                    "http2",
                    "https",
                    "inspector",
                    "module",
                    "net",
                    "os",
                    "path",
                    "perf_hooks",
                    "process",
                    "punycode",
                    "querystring",
                    "readline",
                    "repl",
                    "stream",
                    "string_decoder",
                    "sys",
                    "timers",
                    "tls",
                    "tty",
                    "url",
                    "util",
                    "v8",
                    "vm",
                    "wasi",
                    "worker_threads",
                    "zlib",
                ];
                let bare = specifier.strip_prefix("node:").unwrap_or(specifier);
                // Also match sub-path builtins like timers/promises, stream/consumers
                let root = bare.split('/').next().unwrap_or(bare);
                if NODE_BUILTINS.contains(&root) {
                    let result = V8String::new(scope, specifier).unwrap();
                    rv.set(result.into());
                    return;
                }
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

            // DNS resolution is a network operation: it must be gated by the
            // same Capability::Network check as any other outbound activity,
            // or a package with an empty grant set can exfiltrate via
            // resolveTxt / tunnel via resolve4 (VULN-06).
            if !permissions().check(&Capability::Network(hostname.clone())) {
                let msg = format!(
                    "EACCES: Network access denied. Run with --allow-net={}",
                    hostname
                );
                let err_str = V8String::new(scope, &msg).unwrap();
                rv.set(err_str.into());
                return;
            }

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

            // Same capability gate as __dnsLookup: record queries leak
            // whatever the caller can encode in a name (VULN-06).
            if !permissions().check(&Capability::Network(hostname.clone())) {
                let msg = format!(
                    "EACCES: Network access denied. Run with --allow-net={}",
                    hostname
                );
                let err_str = V8String::new(scope, &msg).unwrap();
                rv.set(err_str.into());
                return;
            }

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
                            // Short option group: -f, -abc (booleans), -n 3, -n3 (value is the rest).
                            for (var si = 1; si < arg.length; si++) {
                                var sch = arg[si];
                                var lname = Object.keys(options).filter(function(k) { return options[k].short === sch; })[0];
                                if (lname === undefined && strict) throw new TypeError('Unknown option \'-' + sch + '\'');
                                var sopt = lname !== undefined ? options[lname] : null;
                                var skey = lname !== undefined ? lname : sch;
                                var sval = true, inl = false;
                                if (sopt && (sopt.type || 'string') !== 'boolean') {
                                    if (si + 1 < arg.length) { sval = arg.slice(si + 1); inl = true; }
                                    else sval = (i + 1 < argv.length && argv[i + 1].slice(0, 1) !== '-') ? argv[++i] : '';
                                    si = arg.length;
                                }
                                if (sopt && sopt.multiple) { if (!Array.isArray(values[skey])) values[skey] = []; values[skey].push(sval); }
                                else values[skey] = sval;
                                tokens.push({ kind: 'option', name: skey, rawName: '-' + sch, value: sval === true ? undefined : sval, inlineValue: inl });
                            }
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
                types: (function() {
                    var t = {
                        isRegExp: function(v) { return v instanceof RegExp; },
                        isDate: function(v) { return v instanceof Date; },
                        isNativeError: function(v) { return v instanceof Error; },
                        isMap: function(v) { return v instanceof Map; },
                        isSet: function(v) { return v instanceof Set; },
                        isPromise: function(v) { return v && typeof v === 'object' && typeof v.then === 'function'; },
                        isProxy: function(v) { return false; },
                        isArrayBuffer: function(v) { return v instanceof ArrayBuffer; },
                        isAnyArrayBuffer: function(v) { return v instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && v instanceof SharedArrayBuffer); },
                        isSharedArrayBuffer: function(v) { return typeof SharedArrayBuffer !== 'undefined' && v instanceof SharedArrayBuffer; },
                        isDataView: function(v) { return v instanceof DataView; },
                        isTypedArray: function(v) { return ArrayBuffer.isView(v) && !(v instanceof DataView); },
                        isUint8Array: function(v) { return v instanceof Uint8Array; },
                        isUint16Array: function(v) { return v instanceof Uint16Array; },
                        isUint32Array: function(v) { return v instanceof Uint32Array; },
                        isInt8Array: function(v) { return v instanceof Int8Array; },
                        isInt16Array: function(v) { return v instanceof Int16Array; },
                        isInt32Array: function(v) { return v instanceof Int32Array; },
                        isFloat32Array: function(v) { return v instanceof Float32Array; },
                        isFloat64Array: function(v) { return v instanceof Float64Array; },
                        isBigInt64Array: function(v) { return v instanceof BigInt64Array; },
                        isBigUint64Array: function(v) { return v instanceof BigUint64Array; },
                        isMapIterator: function(v) { return false; },
                        isSetIterator: function(v) { return false; },
                        isGeneratorObject: function(v) { return false; },
                        isAsyncFunction: function(v) { return false; },
                        isGeneratorFunction: function(v) { return false; },
                        isArgumentsObject: function(v) { return false; },
                        isBoxedPrimitive: function(v) { return v instanceof Number || v instanceof String || v instanceof Boolean || v instanceof Symbol || v instanceof BigInt; },
                        isWeakMap: function(v) { return v instanceof WeakMap; },
                        isWeakSet: function(v) { return v instanceof WeakSet; },
                        isExternal: function(v) { return false; },
                        isModuleNamespaceObject: function(v) { return false; },
                    };
                    return t;
                })(),
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
                },
                styleText: function(format, text) {
                    var codes = {
                        reset: [0, 0], bold: [1, 22], dim: [2, 22], italic: [3, 23],
                        underline: [4, 24], overline: [53, 55], inverse: [7, 27],
                        strikethrough: [9, 29], hidden: [8, 28],
                        black: [30, 39], red: [31, 39], green: [32, 39], yellow: [33, 39],
                        blue: [34, 39], magenta: [35, 39], cyan: [36, 39], white: [37, 39],
                        gray: [90, 39], grey: [90, 39],
                        bgBlack: [40, 49], bgRed: [41, 49], bgGreen: [42, 49], bgYellow: [43, 49],
                        bgBlue: [44, 49], bgMagenta: [45, 49], bgCyan: [46, 49], bgWhite: [47, 49]
                    };
                    var formats = Array.isArray(format) ? format : [format];
                    var open = '', close = '';
                    for (var i = formats.length - 1; i >= 0; i--) {
                        var c = codes[formats[i]];
                        if (c) { open = '\x1b[' + c[0] + 'm' + open; close = close + '\x1b[' + c[1] + 'm'; }
                    }
                    return open + String(text) + close;
                },
                getCallSites: function(frameCountOrOptions) {
                    // Node's real getCallSites() returns real file/line info per
                    // frame; this engine doesn't set a script's ScriptOrigin when
                    // compiling, so V8's CallSite objects come back with empty
                    // getFileName()/getLineNumber() here. Still returns real,
                    // correctly-shaped frame objects (functionName included) —
                    // enough that callers relying on getCallSites() existing and
                    // returning an array of the right length (e.g. test harnesses'
                    // mustNotCall()) don't crash, even though scriptName/lineNumber
                    // read as empty/0 instead of the real location.
                    var frameCount = typeof frameCountOrOptions === 'number' ? frameCountOrOptions : 10;
                    var orig = Error.prepareStackTrace;
                    var holder = {};
                    Error.prepareStackTrace = function(_, stack) { return stack; };
                    Error.captureStackTrace(holder, util.getCallSites);
                    var stack = holder.stack;
                    Error.prepareStackTrace = orig;
                    return (stack || []).slice(0, frameCount).map(function(cs) {
                        return {
                            functionName: cs.getFunctionName() || '',
                            scriptName: cs.getFileName() || '',
                            lineNumber: cs.getLineNumber() || 0,
                            column: cs.getColumnNumber() || 0,
                        };
                    });
                }
            };
            globalThis.__requireCache['util'] = util;
            globalThis.__requireCache['node:util'] = util;
            globalThis.__requireCache['util/types'] = util.types;
            globalThis.__requireCache['node:util/types'] = util.types;

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
                // Node: emitting 'error' with no listener throws, so the
                // runtime reports it as an uncaught exception. A silent no-op
                // hid every error nobody listened for (and made
                // `uncaughtException`-based recovery impossible).
                if (ev === 'error') {
                    var errListeners = this._events['error'];
                    if (!errListeners || errListeners.length === 0) {
                        var er = args[0];
                        if (!(er instanceof Error)) {
                            er = new Error('Unhandled error. (' + String(er) + ')');
                            er.code = 'ERR_UNHANDLED_ERROR';
                            er.context = args[0];
                        }
                        throw er;
                    }
                }
                var listeners = (this._events[ev] || []).slice();
                listeners.forEach(function(fn) { fn.apply(this, args); }, this);
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
            // Node's `events.listenerCount(emitter, type)` takes an
            // EventEmitter *or* an EventTarget: `listenerCount(signal,
            // 'abort')` is how libraries size the bookkeeping they keep for
            // an AbortSignal (test-http-agent-abort-controller). 3va's
            // AbortSignal is an EventTarget whose listeners live in
            // `_listeners`; 'abort' is the only event it supports.
            EventEmitter.listenerCount = function(emitter, ev) {
                if (emitter && typeof emitter.listenerCount === 'function') return emitter.listenerCount(ev);
                if (emitter && typeof emitter.addEventListener === 'function') {
                    if (ev === 'abort' && Array.isArray(emitter._listeners)) return emitter._listeners.length;
                    return 0;
                }
                var e = new TypeError('The "emitter" argument must be an instance of EventEmitter or EventTarget. Received ' + String(emitter));
                e.code = 'ERR_INVALID_ARG_TYPE';
                throw e;
            };
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
            globalThis.__requireCache['node:events'] = EventEmitter;

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

            // ── diagnostics_channel ─────────────────────────────────────────────────
            (function() {
                var _channels = Object.create(null);
                function Channel(name) {
                    this.name = name;
                    this._subscribers = [];
                }
                Object.defineProperty(Channel.prototype, 'hasSubscribers', {
                    get: function() { return this._subscribers.length > 0; }
                });
                Channel.prototype.subscribe = function(fn) { this._subscribers.push(fn); };
                Channel.prototype.unsubscribe = function(fn) {
                    var i = this._subscribers.indexOf(fn);
                    if (i >= 0) this._subscribers.splice(i, 1);
                    return i >= 0;
                };
                Channel.prototype.publish = function(data) {
                    for (var i = 0; i < this._subscribers.length; i++) {
                        try { this._subscribers[i](data, this.name); } catch(e) {}
                    }
                };
                Channel.prototype.bindStore = function() {};
                Channel.prototype.unbindStore = function() {};
                Channel.prototype.runStores = function(ctx, fn) { return fn(); };
                function channel(name) {
                    if (!_channels[name]) _channels[name] = new Channel(name);
                    return _channels[name];
                }
                function hasSubscribers(name) { return !!(_channels[name] && _channels[name].hasSubscribers); }
                function subscribe(name, fn) { channel(name).subscribe(fn); }
                function unsubscribe(name, fn) { return channel(name).unsubscribe(fn); }
                function tracingChannel(prefix) {
                    return {
                        start: channel(prefix + '.start'),
                        end: channel(prefix + '.end'),
                        asyncStart: channel(prefix + '.asyncStart'),
                        asyncEnd: channel(prefix + '.asyncEnd'),
                        error: channel(prefix + '.error'),
                    };
                }
                var dc = { channel: channel, hasSubscribers: hasSubscribers, subscribe: subscribe, unsubscribe: unsubscribe, tracingChannel: tracingChannel, Channel: Channel };
                globalThis.__requireCache['diagnostics_channel'] = dc;
                globalThis.__requireCache['node:diagnostics_channel'] = dc;
            })();

            // ── http2 compat (real h2 backend) ──────────────────────────────
            (function() {
                var constants = {
                    NGHTTP2_SESSION_SERVER: 0, NGHTTP2_SESSION_CLIENT: 1,
                    NGHTTP2_STREAM_STATE_IDLE: 1, NGHTTP2_STREAM_STATE_OPEN: 2,
                    NGHTTP2_STREAM_STATE_RESERVED_LOCAL: 3, NGHTTP2_STREAM_STATE_RESERVED_REMOTE: 4,
                    NGHTTP2_STREAM_STATE_HALF_CLOSED_LOCAL: 5, NGHTTP2_STREAM_STATE_HALF_CLOSED_REMOTE: 6,
                    NGHTTP2_STREAM_STATE_CLOSED: 7,
                    NGHTTP2_NO_ERROR: 0, NGHTTP2_PROTOCOL_ERROR: 1, NGHTTP2_INTERNAL_ERROR: 2,
                    NGHTTP2_FLOW_CONTROL_ERROR: 3, NGHTTP2_SETTINGS_TIMEOUT: 4,
                    NGHTTP2_STREAM_CLOSED: 5, NGHTTP2_FRAME_SIZE_ERROR: 6,
                    NGHTTP2_REFUSED_STREAM: 7, NGHTTP2_CANCEL: 8,
                    NGHTTP2_COMPRESSION_ERROR: 9, NGHTTP2_CONNECT_ERROR: 10,
                    NGHTTP2_ENHANCE_YOUR_CALM: 11, NGHTTP2_INADEQUATE_SECURITY: 12,
                    NGHTTP2_HTTP_1_1_REQUIRED: 13,
                    HTTP2_HEADER_STATUS: ':status', HTTP2_HEADER_METHOD: ':method',
                    HTTP2_HEADER_AUTHORITY: ':authority', HTTP2_HEADER_SCHEME: ':scheme',
                    HTTP2_HEADER_PATH: ':path', HTTP2_HEADER_CONTENT_LENGTH: 'content-length',
                    HTTP2_HEADER_CONTENT_TYPE: 'content-type', HTTP2_HEADER_ACCEPT: 'accept',
                    HTTP2_HEADER_HOST: 'host', HTTP2_HEADER_CONNECTION: 'connection',
                    HTTP2_HEADER_UPGRADE: 'upgrade', HTTP2_HEADER_SERVER: 'server',
                    HTTP2_HEADER_DATE: 'date', HTTP2_HEADER_TRANSFER_ENCODING: 'transfer-encoding',
                    HTTP2_METHOD_GET: 'GET', HTTP2_METHOD_POST: 'POST',
                    HTTP2_METHOD_PUT: 'PUT', HTTP2_METHOD_DELETE: 'DELETE',
                    HTTP2_METHOD_PATCH: 'PATCH', HTTP2_METHOD_HEAD: 'HEAD',
                    HTTP2_METHOD_OPTIONS: 'OPTIONS'
                };

                function EventEmitter() {
                    this._events = {};
                    this._maxListeners = 10;
                }
                EventEmitter.prototype.on = function(e, fn) {
                    if (!this._events[e]) this._events[e] = [];
                    this._events[e].push(fn);
                    return this;
                };
                EventEmitter.prototype.once = function(e, fn) {
                    var self = this;
                    function wrapped() { self.removeListener(e, wrapped); fn.apply(this, arguments); }
                    wrapped._original = fn;
                    return this.on(e, wrapped);
                };
                EventEmitter.prototype.emit = function(e) {
                    var args = Array.prototype.slice.call(arguments, 1);
                    var fns = this._events[e] || [];
                    for (var i = 0; i < fns.length; i++) fns[i].apply(this, args);
                    return fns.length > 0;
                };
                EventEmitter.prototype.removeListener = function(e, fn) {
                    var fns = this._events[e]; if (!fns) return this;
                    this._events[e] = fns.filter(function(f) { return f !== fn && f._original !== fn; });
                    return this;
                };
                EventEmitter.prototype.setMaxListeners = function(n) { this._maxListeners = n; return this; };
                EventEmitter.prototype.getMaxListeners = function() { return this._maxListeners; };
                EventEmitter.prototype.listeners = function(e) { return this._events[e] || []; };
                EventEmitter.prototype.listenerCount = function(e) { return (this._events[e] || []).length; };

                function Http2Server(options, onRequest) {
                    EventEmitter.call(this);
                    this._options = options || {};
                    this._serverId = null;
                    this._pollTimer = null;
                    this._listening = false;
                    if (typeof onRequest === 'function') this.on('stream', onRequest);
                }
                Http2Server.prototype = Object.create(EventEmitter.prototype);
                Http2Server.prototype.listen = function(port, host, cb) {
                    var self = this;
                    if (typeof port === 'function') { cb = port; port = 0; host = '0.0.0.0'; }
                    if (typeof host === 'function') { cb = host; host = '0.0.0.0'; }
                    port = port || 0;
                    host = host || '0.0.0.0';
                    var result = __h2Listen(port, host);
                    if (result instanceof Error) {
                        if (cb) setTimeout(function() { cb(result); }, 0);
                        return this;
                    }
                    this._serverId = result;
                    this._listening = true;
                    this._pollTimer = setInterval(function() {
                        var info = __h2AcceptPoll(self._serverId);
                        if (info !== null) {
                            try {
                                var parsed = JSON.parse(info);
                                parsed._serverId = self._serverId;
                                var stream = new Http2ServerStream(parsed);
                                self.emit('stream', stream, parsed.headers);
                            } catch(e) {}
                        }
                    }, 5);
                    this.emit('listening');
                    if (cb) setTimeout(cb, 0);
                    return this;
                };
                Http2Server.prototype.address = function() {
                    if (this._serverId === null) return null;
                    var port = __h2ServerPort(this._serverId);
                    return { port: port, family: 'IPv4', address: '0.0.0.0' };
                };
                Http2Server.prototype.close = function(cb) {
                    if (this._serverId !== null) {
                        __h2ServerClose(this._serverId);
                        this._serverId = null;
                    }
                    if (this._pollTimer) { clearInterval(this._pollTimer); this._pollTimer = null; }
                    this._listening = false;
                    this.emit('close');
                    if (cb) setTimeout(cb, 0);
                    return this;
                };
                // ponytail: ref/unref are no-ops — 3va's event loop does not release the process per handle; see docs/05-js-engine/07-inventario-stubs.md §7.2.
                Http2Server.prototype.ref = function() { return this; };
                Http2Server.prototype.unref = function() { return this; };

                function Http2ServerStream(info) {
                    EventEmitter.call(this);
                    this._serverId = info._serverId;
                    this._streamId = info.streamId;
                    this.headers = info.headers || {};
                    this.method = info.method;
                    this.url = info.path;
                    this._responded = false;
                    this._ended = false;
                }
                Http2ServerStream.prototype = Object.create(EventEmitter.prototype);
                Http2ServerStream.prototype.respond = function(headers) {
                    if (this._responded) return;
                    this._responded = true;
                    this._headers = headers;
                    __h2StreamRespond(this._serverId, this._streamId, JSON.stringify(headers));
                };
                Http2ServerStream.prototype.write = function(data) {
                    if (!this._responded) this.respond({ ':status': 200 });
                    __h2StreamWrite(this._serverId, this._streamId, data);
                };
                Http2ServerStream.prototype.end = function(data) {
                    if (!this._responded) this.respond({ ':status': 200 });
                    if (data !== undefined && data !== null) {
                        __h2StreamEnd(this._serverId, this._streamId, data);
                    } else {
                        __h2StreamEnd(this._serverId, this._streamId);
                    }
                    this._ended = true;
                    this.emit('end');
                };
                Http2ServerStream.prototype.close = function() {
                    if (!this._ended) this.end();
                };
                Http2ServerStream.prototype.pushStream = function(headers, cb) {
                    if (cb) cb(new Error('push streams not supported'));
                };

                function Http2ClientSession(clientId) {
                    EventEmitter.call(this);
                    this._clientId = clientId;
                    this._pollTimer = null;
                    this._closed = false;
                    this._startPolling();
                }
                Http2ClientSession.prototype = Object.create(EventEmitter.prototype);
                Http2ClientSession.prototype._startPolling = function() {
                    var self = this;
                    this._pollTimer = setInterval(function() {
                        if (self._closed) { clearInterval(self._pollTimer); return; }
                        var info = __h2ClientPoll(self._clientId);
                        if (info !== null) {
                            try {
                                var parsed = JSON.parse(info);
                                self.emit(parsed.type, parsed);
                            } catch(e) {}
                        }
                    }, 5);
                };
                Http2ClientSession.prototype.request = function(headers) {
                    var streamId = __h2ClientRequest(this._clientId, JSON.stringify(headers));
                    if (streamId === -1) throw new Error('failed to create stream');
                    return new Http2ClientStream(this._clientId, streamId);
                };
                Http2ClientSession.prototype.close = function(cb) {
                    this._closed = true;
                    if (this._pollTimer) { clearInterval(this._pollTimer); this._pollTimer = null; }
                    __h2ClientClose(this._clientId);
                    this.emit('close');
                    if (cb) setTimeout(cb, 0);
                };
                Http2ClientSession.prototype.destroy = function() { this.close(); };
                Http2ClientSession.prototype.settings = function() {};
                Http2ClientSession.prototype.ref = function() { return this; };
                Http2ClientSession.prototype.unref = function() { return this; };
                Http2ClientSession.prototype.ping = function(cb) { if (cb) cb(null, Buffer.alloc(8)); };

                function Http2ClientStream(clientId, streamId) {
                    EventEmitter.call(this);
                    this._clientId = clientId;
                    this._streamId = streamId;
                    this._ended = false;
                }
                Http2ClientStream.prototype = Object.create(EventEmitter.prototype);
                Http2ClientStream.prototype.write = function(data) {
                    __h2ClientStreamWrite(this._clientId, this._streamId, data);
                };
                Http2ClientStream.prototype.end = function(data) {
                    if (data !== undefined && data !== null) {
                        __h2ClientStreamEnd(this._clientId, this._streamId, data);
                    } else {
                        __h2ClientStreamEnd(this._clientId, this._streamId);
                    }
                    this._ended = true;
                };
                Http2ClientStream.prototype.close = function() {
                    if (!this._ended) this.end();
                };
                Http2ClientStream.prototype.respond = function() {};

                var http2 = {
                    constants: constants,
                    sensitiveHeaders: Symbol('sensitiveHeaders'),
                    createServer: function(options, onRequest) {
                        if (typeof options === 'function') { onRequest = options; options = {}; }
                        return new Http2Server(options || {}, onRequest);
                    },
                    createSecureServer: function(options, onRequest) {
                        if (typeof options === 'function') { onRequest = options; options = {}; }
                        return new Http2Server(options || {}, onRequest);
                    },
                    connect: function(url, options, listener) {
                        if (typeof options === 'function') { listener = options; options = {}; }
                        var parsed = typeof url === 'string' ? new URL(url) : url;
                        var authority = parsed.host || (parsed.hostname + ':' + (parsed.port || 80));
                        var clientId = __h2Connect(authority);
                        var session = new Http2ClientSession(clientId);
                        if (listener) setTimeout(function() { listener(session); }, 0);
                        return session;
                    },
                    getDefaultSettings: function() { return { enablePush: true, enableConnectProtocol: false }; },
                    getPackedSettings: function() { return Buffer.alloc(0); },
                    getUnpackedSettings: function() { return {}; },
                    performServerHandshake: function() { return false; }
                };
                globalThis.__requireCache['http2'] = http2;
                globalThis.__requireCache['node:http2'] = http2;
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
                // `require('node:timers').promises` is the same object, as in
                // Node (`timers.promises === require('node:timers/promises')`).
                timers.promises = timersPromises;
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
                    // The native lookup runs on a later tick, by which the
                    // require()-boundary scope wrapper has reverted. Capture
                    // the caller's scope now and re-apply it around the
                    // deferred native call so package deny-* rules still gate
                    // it (same pattern as fs streams' __streamScope).
                    var __scope = (typeof globalThis.__currentCallerScope === 'string')
                        ? globalThis.__currentCallerScope : '.';
                    setTimeout(function() {
                        var __prev = globalThis.__currentCallerScope;
                        globalThis.__currentCallerScope = __scope;
                        if (typeof __setCallerScope === 'function') __setCallerScope(__scope);
                        try {
                            var raw = __dnsLookup(hostname);
                            var ips;
                            try { ips = JSON.parse(raw); } catch (e) { callback(mkErr(raw, hostname, 'getaddrinfo')); return; }
                            if (options.all) {
                                callback(null, ips.map(function(ip) { return { address: ip, family: ip.indexOf(':') !== -1 ? 6 : 4 }; }));
                            } else {
                                var ip = ips[0];
                                callback(null, ip, ip.indexOf(':') !== -1 ? 6 : 4);
                            }
                        } finally {
                            globalThis.__currentCallerScope = __prev;
                            if (typeof __setCallerScope === 'function') __setCallerScope(__prev);
                        }
                    }, 0);
                }

                function resolveFamily(hostname, family, callback) {
                    var __scope = (typeof globalThis.__currentCallerScope === 'string')
                        ? globalThis.__currentCallerScope : '.';
                    setTimeout(function() {
                        var __prev = globalThis.__currentCallerScope;
                        globalThis.__currentCallerScope = __scope;
                        if (typeof __setCallerScope === 'function') __setCallerScope(__scope);
                        try {
                            var raw = __dnsLookup(hostname);
                            var ips;
                            try { ips = JSON.parse(raw); } catch (e) { callback(mkErr(raw, hostname, 'queryA')); return; }
                            var filtered = ips.filter(function(ip) { return (ip.indexOf(':') !== -1) === (family === 6); });
                            if (!filtered.length) { callback(mkErr('ENOTFOUND', hostname, 'queryA')); return; }
                            callback(null, filtered);
                        } finally {
                            globalThis.__currentCallerScope = __prev;
                            if (typeof __setCallerScope === 'function') __setCallerScope(__prev);
                        }
                    }, 0);
                }

                function resolveQuery(rrtype) {
                    return function(hostname, callback) {
                        var __scope = (typeof globalThis.__currentCallerScope === 'string')
                            ? globalThis.__currentCallerScope : '.';
                        setTimeout(function() {
                            var __prev = globalThis.__currentCallerScope;
                            globalThis.__currentCallerScope = __scope;
                            if (typeof __setCallerScope === 'function') __setCallerScope(__scope);
                            try {
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
                            } finally {
                                globalThis.__currentCallerScope = __prev;
                                if (typeof __setCallerScope === 'function') __setCallerScope(__prev);
                            }
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
                    var __scope = (typeof globalThis.__currentCallerScope === 'string')
                        ? globalThis.__currentCallerScope : '.';
                    setTimeout(function() {
                        var __prev = globalThis.__currentCallerScope;
                        globalThis.__currentCallerScope = __scope;
                        if (typeof __setCallerScope === 'function') __setCallerScope(__scope);
                        try {
                            var raw = __dnsQuery(ip, 'PTR');
                            var data;
                            try { data = JSON.parse(raw); } catch (e) { callback(mkErr(raw, ip, 'getHostByAddr')); return; }
                            callback(null, data);
                        } finally {
                            globalThis.__currentCallerScope = __prev;
                            if (typeof __setCallerScope === 'function') __setCallerScope(__prev);
                        }
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
                    // 1/5/15-minute load averages, as in Node. Backed by
                    // /proc/loadavg on Linux (see process.rs); other platforms
                    // return [0, 0, 0] rather than throwing.
                    loadavg: function() { return __osLoadAvg(); },
                    cpus: function() { return JSON.parse(__osCpus()); },
                    availableParallelism: function() {
                        var list = JSON.parse(__osCpus());
                        return (list && list.length) || 1;
                    },
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
                    constants: (function() {
                        // Node's `os.constants`: the POSIX signal numbers, with
                        // `signals` frozen (writing to it throws a TypeError in
                        // strict mode, like in Node).
                        var signals = {
                            SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGILL: 4, SIGTRAP: 5,
                            SIGABRT: 6, SIGIOT: 6, SIGBUS: 7, SIGFPE: 8, SIGKILL: 9,
                            SIGUSR1: 10, SIGSEGV: 11, SIGUSR2: 12, SIGPIPE: 13,
                            SIGALRM: 14, SIGTERM: 15, SIGCHLD: 17, SIGSTKFLT: 16,
                            SIGCONT: 18, SIGSTOP: 19, SIGTSTP: 20, SIGTTIN: 21,
                            SIGTTOU: 22, SIGURG: 23, SIGXCPU: 24, SIGXFSZ: 25,
                            SIGVTALRM: 26, SIGPROF: 27, SIGWINCH: 28, SIGIO: 29,
                            SIGPOLL: 29, SIGPWR: 30, SIGSYS: 31
                        };
                        Object.freeze(signals);
                        return { signals: signals, errno: {}, priority: {} };
                    })(),
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
                var _vmContexts = {};
                var _vmContextId = 0;

                function VmScript(code) { this.code = code; }
                VmScript.prototype.runInContext = function(sandbox) {
                    if (!sandbox || !sandbox.__contextId__) return undefined;
                    var result = __vmRunInContextById(sandbox.__contextId__, this.code);
                    if (result && result.error) throw new Error(result.error);
                    // sync sandbox changes back
                    var globals = __vmGetContextGlobals(sandbox.__contextId__);
                    if (globals) {
                        try {
                            var updated = JSON.parse(globals);
                            Object.keys(updated).forEach(function(k) { sandbox[k] = updated[k]; });
                        } catch(e) {}
                    }
                    return result ? JSON.parse(result).value : undefined;
                };
                VmScript.prototype.runInNewContext = function(sandbox) { return this.runInContext(sandbox || {}); };
                VmScript.prototype.runInThisContext = function() { return (0, eval)(this.code); };

                var vm = {
                    createContext: function(sandbox) {
                        sandbox = sandbox || {};
                        var id = ++_vmContextId;
                        sandbox.__isVmContext__ = true;
                        sandbox.__contextId__ = id;
                        var json = JSON.stringify(sandbox);
                        __vmCreateContext(id, json);
                        _vmContexts[id] = sandbox;
                        return sandbox;
                    },
                    isContext: function(obj) {
                        return obj && obj.__isVmContext__ === true;
                    },
                    runInNewContext: function(code, sandbox) {
                        var ctx = vm.createContext(sandbox || {});
                        var result = __vmRunInContextById(ctx.__contextId__, code);
                        if (result && result.error) throw new Error(result.error);
                        // sync sandbox changes back
                        var globals = __vmGetContextGlobals(ctx.__contextId__);
                        if (globals) {
                            try {
                                var updated = JSON.parse(globals);
                                Object.keys(updated).forEach(function(k) { ctx[k] = updated[k]; });
                            } catch(e) {}
                        }
                        return result ? JSON.parse(result).value : undefined;
                    },
                    runInContext: function(code, sandbox) {
                        if (!sandbox || !sandbox.__contextId__) return undefined;
                        var result = __vmRunInContextById(sandbox.__contextId__, code);
                        if (result && result.error) throw new Error(result.error);
                        // sync sandbox changes back
                        var globals = __vmGetContextGlobals(sandbox.__contextId__);
                        if (globals) {
                            try {
                                var updated = JSON.parse(globals);
                                Object.keys(updated).forEach(function(k) { sandbox[k] = updated[k]; });
                            } catch(e) {}
                        }
                        return result ? JSON.parse(result).value : undefined;
                    },
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
                    this.id = 0;
                    this.process = { pid: 0 };
                    this.state = 'online';
                    this.exitedAfterDisconnect = false;
                    this._pollInterval = null;
                }
                util.inherits(ClusterWorker, EventEmitter);
                ClusterWorker.prototype.send = function(msg) {
                    return __clusterSend(this.id, JSON.stringify(msg));
                };
                ClusterWorker.prototype.kill = function(sig) {
                    this.state = 'dead';
                    if (this._pollInterval) { clearInterval(this._pollInterval); this._pollInterval = null; }
                    __clusterKill(this.id, sig || 'SIGTERM');
                    this.emit('exit', 0, sig || 'SIGTERM');
                    cluster.emit('exit', this, 0, sig || 'SIGTERM');
                };
                ClusterWorker.prototype.disconnect = function() { this.kill(); };
                ClusterWorker.prototype.isDead = function() { return this.state === 'dead'; };

                function _clusterEnvIsWorker() {
                    return !!(typeof process !== 'undefined' && process.env && process.env.CLUSTER_WORKER);
                }
                var _clusterWorkerHandle = null;
                // Only sets the worker handle; the per-run `fork-ipc` bootstrap
                // already installs process.send/stdin polling from VVVA_FORKED.
                function _clusterEnsureWorker() {
                    if (cluster._workerReady || !_clusterEnvIsWorker()) return;
                    cluster._workerReady = true;
                    _clusterWorkerHandle = {
                        id: parseInt(process.env.NODE_WORKER_ID || '0'),
                        send: function(msg) { process.stdout.write(JSON.stringify(msg) + '\n'); }
                    };
                }
                // Runtime getters, not values baked into the builtins snapshot:
                // the snapshot is recorded with an empty environment, so an eager
                // `cluster.isPrimary = !process.env.CLUSTER_WORKER` froze `true`
                // for every process and made cluster workers re-fork forever.
                Object.defineProperty(cluster, 'isPrimary', {
                    get: function() { return !_clusterEnvIsWorker(); },
                    enumerable: true, configurable: true,
                });
                Object.defineProperty(cluster, 'isMaster', {
                    get: function() { return cluster.isPrimary; },
                    enumerable: true, configurable: true,
                });
                Object.defineProperty(cluster, 'isWorker', {
                    get: function() { _clusterEnsureWorker(); return _clusterEnvIsWorker(); },
                    enumerable: true, configurable: true,
                });
                Object.defineProperty(cluster, 'worker', {
                    get: function() { _clusterEnsureWorker(); return _clusterWorkerHandle; },
                    enumerable: true, configurable: true,
                });
                cluster.workers = _clusterWorkers;
                cluster.settings = {};
                cluster.schedulingPolicy = 2;
                cluster.SCHED_NONE = 1;
                cluster.SCHED_RR = 2;

                _clusterEnsureWorker();

                cluster.fork = function(env) {
                    if (!cluster.isPrimary) return null;
                    var w = new ClusterWorker();
                    w.id = ++_clusterNextId;
                    _clusterWorkers[w.id] = w;
                    var scriptPath = process.argv[1] || cluster.settings.exec || '';
                    var result = __clusterFork(scriptPath, w.id, JSON.stringify(env || {}), JSON.stringify([]), true);
                    if (result === -1) {
                        delete _clusterWorkers[w.id];
                        return null;
                    }
                    w.process.pid = result;
                    var pollInterval = setInterval(function() {
                        try {
                            var msg = __clusterPoll(w.id);
                            if (msg !== null) {
                                var parsed = JSON.parse(msg);
                                w.emit('message', parsed);
                                cluster.emit('message', w, parsed);
                            }
                        } catch(e) {}
                    }, 10);
                    w._pollInterval = pollInterval;
                    cluster.emit('fork', w);
                    return w;
                };
                cluster.setupPrimary = function(settings) { cluster.settings = settings || {}; };
                cluster.setupMaster = cluster.setupPrimary;
                cluster.disconnect = function(cb) {
                    var ids = Object.keys(_clusterWorkers);
                    ids.forEach(function(id) { _clusterWorkers[id].kill(); });
                    if (cb) setTimeout(cb, 0);
                };
                globalThis.__requireCache['cluster'] = cluster;
                globalThis.__requireCache['node:cluster'] = cluster;
            })();

            // ── StringDecoder ─────────────────────────────────────────────────────
            // Incremental, chunk-boundary-safe decoding (Node's string_decoder).
            // One long-lived instance per stream holds back an incomplete
            // trailing multi-byte UTF-8 sequence and prepends it to the next
            // chunk, so sequences split across read boundaries are reassembled
            // instead of being corrupted into U+FFFD. Defined at top level (not
            // inside the string_decoder IIFE further down) so the stream
            // classes below can route setEncoding() through it; the IIFE only
            // registers it in the require cache.
            function StringDecoder(encoding) {
                this.encoding = (encoding || 'utf8').toLowerCase().replace(/[-_]/g, '');
                if (this.encoding === 'utf8') this.encoding = 'utf8';
                this._incomplete = null;
            }
            StringDecoder.prototype.write = function(buf) {
                if (typeof buf === 'string') return buf;
                if (!(buf instanceof Buffer)) buf = Buffer.from(buf);
                if (buf.length === 0) return '';
                if (this.encoding === 'utf8') {
                    if (this._incomplete) {
                        buf = Buffer.concat([this._incomplete, buf]);
                        this._incomplete = null;
                    }
                    // Walk back over the continuation bytes of the final
                    // sequence; if that sequence is truncated at the end of
                    // this chunk, hold it back for the next write().
                    var i = buf.length;
                    while (i > 0 && (buf[i - 1] & 0xC0) === 0x80) i--;
                    var complete = buf.length;
                    if (i === 0) {
                        // Entire chunk is continuation bytes of a sequence
                        // started in a previous chunk.
                        complete = 0;
                    } else if ((buf[i - 1] & 0x80) !== 0) {
                        var b = buf[i - 1];
                        var bytesNeeded = (b & 0xE0) === 0xC0 ? 2 : (b & 0xF0) === 0xE0 ? 3 : (b & 0xF8) === 0xF0 ? 4 : 0;
                        if (bytesNeeded === 0) {
                            // Invalid lead byte: decode lossily, hold nothing.
                            complete = i;
                        } else if (i - 1 + bytesNeeded > buf.length) {
                            // Final sequence truncated at the chunk boundary.
                            complete = i - 1;
                        }
                    }
                    if (complete < buf.length) {
                        this._incomplete = buf.slice(complete);
                        if (complete === 0) return '';
                    }
                    return buf.slice(0, complete).toString('utf8');
                }
                return buf.toString(this.encoding);
            };
            StringDecoder.prototype.end = function(buf) {
                var ret = '';
                if (buf) ret = this.write(buf);
                if (this._incomplete) {
                    ret += this._incomplete.toString('utf8');
                    this._incomplete = null;
                }
                return ret;
            };

            function Stream() { EventEmitter.call(this); }
            util.inherits(Stream, EventEmitter);
            // Generic pipe for plain `Stream` subclasses (combined-stream /
            // form-data inherit from Stream directly, not Readable). This used
            // to be `function() { return this; }` — a no-op — so
            // `formData.pipe(req)` attached no 'data' listener, the multipart
            // body was emitted to nobody and the process exited 0 with the
            // upload still pending. Readable overrides this with the
            // backpressure-aware version below.
            Stream.prototype.pipe = function(dest, opts) {
                var src = this;
                function ondata(chunk) {
                    if (!dest || typeof dest.write !== 'function') return;
                    if (dest.write(chunk) === false && typeof src.pause === 'function') src.pause();
                }
                function onend() {
                    if (!opts || opts.end !== false) {
                        if (dest && typeof dest.end === 'function') dest.end();
                    }
                }
                function onerror(err) {
                    if (dest && typeof dest.destroy === 'function') dest.destroy(err);
                }
                function ondrain() { if (typeof src.resume === 'function') src.resume(); }
                src.on('data', ondata);
                src.once('end', onend);
                src.once('error', onerror);
                if (dest && typeof dest.on === 'function') dest.on('drain', ondrain);
                if (dest && typeof dest.emit === 'function') dest.emit('pipe', src);
                if (typeof src.resume === 'function' && !(src._readableState && src._readableState.flowing)) src.resume();
                return dest;
            };
            function Readable(opts) {
                Stream.call(this);
                this.readable = true;
                this._readableState = { objectMode: !!(opts && opts.objectMode), highWaterMark: (opts && opts.highWaterMark) || 16384, length: 0, buffer: [], ended: false, _endEmitted: false, flowing: false, discard: false };
                if (opts && typeof opts.read === 'function') this._read = opts.read;
                // Node: { encoding: 'utf8' } in the constructor is equivalent
                // to calling setEncoding() — 'data' events carry decoded
                // strings, with multi-byte sequences split across chunk
                // boundaries reassembled by the decoder instead of being
                // corrupted at each boundary.
                if (opts && opts.encoding && opts.encoding !== 'buffer') this.setEncoding(opts.encoding);
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
                var state = this._readableState;
                // Attaching a 'data' listener switches the stream to flowing
                // mode and flushes anything buffered while it was paused.
                // Synchronous (Node resumes on the next tick) so a push right
                // after the attach is delivered in the same turn, which is the
                // ordering almost all code — and the stream tests — rely on.
                if (ev === 'data') {
                    // A consumer is reading, so nothing is dropped.
                    state.discard = false;
                    if (!state.flowing) state.flowing = true;
                    this._flushIfFlowing();
                    this._maybeEnd();
                    // Also schedule a real resume(): streams that start I/O
                    // from resume() (fs.ReadStream._read, UpgradeSocket's poll)
                    // need it, and it must not run re-entrantly inside `on`.
                    if (!state._resumeScheduled) {
                        state._resumeScheduled = true;
                        var self = this;
                        setTimeout(function() {
                            state._resumeScheduled = false;
                            self.resume();
                        }, 0);
                    }
                }
                return this;
            };
            Readable.prototype.read = function(n) {
                if (!this._readableState.flowing) this._read(n || 0);
                var buf = this._readableState.buffer.splice(0);
                this._readableState.length = 0;
                this._maybeEnd();
                if (!buf.length) return null;
                // With setEncoding() the buffer holds decoded strings.
                if (typeof buf[0] === 'string') return buf.join('');
                return Buffer.concat ? Buffer.concat(buf) : buf[0];
            };
            // Byte length of a buffered chunk (a count in objectMode). Used to
            // enforce highWaterMark in bytes, the way Node does — counting
            // buffered *chunks* made the watermark unreachable with 64 KiB chunks.
            function _chunkBytes(chunk) {
                if (typeof chunk === 'string') return chunk.length;
                return (chunk && typeof chunk.length === 'number') ? chunk.length : 1;
            }
            // Deliver buffered chunks to 'data' listeners while the stream is
            // flowing, stopping the moment a listener pauses it again.
            Readable.prototype._flushIfFlowing = function() {
                var state = this._readableState;
                while (state.buffer.length && state.flowing) {
                    // Re-check each iteration: a `once('data')` consumer (the
                    // async iterator) removes itself after the first chunk, and
                    // emitting the rest to nobody would silently drop them.
                    var hasDataL = this._events && this._events['data'] && this._events['data'].length > 0;
                    if (!hasDataL) break;
                    var chunk = state.buffer.shift();
                    state.length = Math.max(0, state.length - _chunkBytes(chunk));
                    this.emit('data', chunk);
                }
            };
            // Emit 'end' only once the buffer is empty. Emitting it while data
            // was still buffered (a fast producer calling push(null) before a
            // paused consumer resumed) made small bodies vanish: the pending
            // 'end' resolved a consumer's `for await` before its 'data' did.
            Readable.prototype._maybeEnd = function() {
                var state = this._readableState;
                if (state.ended && !state._endEmitted && state.buffer.length === 0) {
                    state._endEmitted = true;
                    this.emit('end');
                    this.emit('readable');
                }
            };
            Readable.prototype.push = function(chunk) {
                var state = this._readableState;
                if (chunk === null) {
                    // Flush any incomplete trailing multi-byte sequence the
                    // decoder has been holding back as a final 'data' event.
                    if (this._decoder) {
                        var tail = this._decoder.end();
                        this._decoder = null;
                        if (tail) {
                            var hasDataT = this._events && this._events['data'] && this._events['data'].length > 0;
                            if (state.flowing && hasDataT) this.emit('data', tail);
                            else { state.buffer.push(tail); state.length += _chunkBytes(tail); }
                        }
                    }
                    state.ended = true;
                    this._flushIfFlowing();
                    this._maybeEnd();
                    this.emit('readable');
                } else {
                    // Honor setEncoding(): decode byte chunks through the
                    // stream's long-lived StringDecoder so sequences split
                    // across chunks are reassembled, never re-decoded
                    // per-chunk (which would corrupt boundary-straddling
                    // multi-byte characters into U+FFFD).
                    if (this._decoder && !state.objectMode && typeof chunk !== 'string') {
                        chunk = this._decoder.write(chunk);
                    }
                    // Node only emits 'data' while flowing. Emitting while
                    // paused (as this did) meant `pause()` did not actually
                    // stop delivery, which corrupted uploads in libraries that
                    // pause the request while they flush to disk (formidable).
                    var hasDataL = this._events && this._events['data'] && this._events['data'].length > 0;
                    if (state.flowing && hasDataL) {
                        this.emit('data', chunk);
                    } else if (state.flowing && state.discard) {
                        // Resumed with nobody listening: drop the chunk and
                        // keep the buffer empty, as Node's flow() does (the
                        // async iterator and pipe() attach 'data', so they
                        // never take this branch and keep their backpressure).
                    } else {
                        state.buffer.push(chunk);
                        state.length += state.objectMode ? 1 : _chunkBytes(chunk);
                        if (this._events && this._events['readable'] && this._events['readable'].length) {
                            this.emit('readable');
                        }
                    }
                }
                return !state.ended;
            };
            Readable.prototype.unshift = function(chunk) { if (chunk != null) this.emit('data', chunk); return this; };
            Readable.prototype.pipe = function(dest, opts) {
                var self = this;
                function onDrain() { if (!self._readableState.flowing) self.resume(); }
                this.on('data', function(c) {
                    if (dest.write(c) === false) {
                        // `write()` can return false *and* have already
                        // emitted 'drain' synchronously (a sync `_write`). Only
                        // pause if the destination is still over its watermark,
                        // or the pause would outlive the drain and stall.
                        var dst = dest._writableState;
                        if (self.pause && dst && dst.length >= dst.highWaterMark) self.pause();
                    }
                });
                this.on('end', function() { if (!opts || opts.end !== false) dest.end(); });
                // Now that pause() actually stops 'data', resume when the
                // destination drains — otherwise a slow destination stalls the
                // pipe after the first backpressured write (Node does this too).
                if (dest && typeof dest.on === 'function') dest.on('drain', onDrain);
                dest.emit('pipe', this);
                if (!this._readableState.flowing) this.resume();
                return dest;
            };
            Readable.prototype.unpipe = function(dest) { return this; };
            Readable.prototype.resume = function() {
                var state = this._readableState;
                state.flowing = true;
                state._resumeScheduled = false;
                // Node's flow(): a resumed stream with no 'data' listener
                // drains to nobody, so the buffer must not pile up. It used to
                // stay full and `_maybeEnd` (which requires an empty buffer)
                // never emitted 'end' — the `res.resume()` then 'end' idiom
                // that the http client uses. `on('data')` clears this.
                if (!(this._events && this._events['data'] && this._events['data'].length)) {
                    state.discard = true;
                    state.buffer.length = 0;
                    state.length = 0;
                }
                this._flushIfFlowing();
                this._maybeEnd();
                this._read(0);
                return this;
            };
            Readable.prototype.pause = function() { this._readableState.flowing = false; return this; };
            // setEncoding() creates the long-lived decoder used by push():
            // storing the value alone is not enough — consumers that collect
            // 'data' chunks as strings rely on Node's semantics where each
            // multi-byte sequence split across chunks is reassembled.
            Readable.prototype.setEncoding = function(enc) {
                this._encoding = enc;
                this._decoder = (enc && enc !== 'buffer') ? new StringDecoder(enc) : null;
                return this;
            };
            // Node's stream destroy (lib/internal/streams/destroy.js): the
            // stream is destroyed synchronously (`destroyed` and `errored`
            // are set before destroy() returns), and 'error'/'close' are
            // emitted on later ticks, with `closed` set just before 'close'.
            // An 'error' with no listener is the process's uncaught exception
            // in Node, not a silent no-op (test-http-client-incomingmessage-
            // destroy relies on it, and so does any app that installs an
            // `uncaughtException` handler instead of an error listener).
            function __destroyStream(self, err) {
                if (self.destroyed) {
                    if (err && self.listenerCount('error') > 0) self.emit('error', err);
                    return self;
                }
                self.destroyed = true;
                self.errored = err === undefined ? null : err;
                self.closed = false;
                if (self._readableState) self._readableState.destroyed = true;
                if (self._writableState) self._writableState.destroyed = true;
                var next = (typeof process !== 'undefined' && typeof process.nextTick === 'function')
                    ? process.nextTick : function(f) { queueMicrotask(f); };
                // Two separate ticks, as in Node: an unhandled 'error' must
                // not keep 'close' from being emitted.
                next(function() {
                    if (!err) return;
                    // emit() itself throws when nobody listens, as in Node.
                    self.emit('error', err);
                });
                next(function() {
                    self.closed = true;
                    self.emit('close');
                });
                return self;
            }
            Readable.prototype.destroy = function(err) { return __destroyStream(this, err); };
            Readable.prototype[Symbol.asyncIterator] = function() {
                var self = this;
                return {
                    next: function() {
                        // Re-check live state on every call instead of caching a
                        // `done` flag: a stale 'end' listener left over from an
                        // earlier pending next() used to flip a shared flag and
                        // make a buffered final chunk invisible (the async
                        // generator dropped the last element).
                        if (self._readableState.buffer.length) {
                            var _v = self._readableState.buffer.shift();
                            self._readableState.length = Math.max(0, self._readableState.length - _chunkBytes(_v));
                            self._maybeEnd();
                            return Promise.resolve({ value: _v, done: false });
                        }
                        if (self._readableState.ended)
                            return Promise.resolve({ done: true });
                        return new Promise(function(resolve) {
                            function cleanup() {
                                self.removeListener('data', onData);
                                self.removeListener('end', onEnd);
                            }
                            function onData(chunk) { cleanup(); resolve({ value: chunk, done: false }); }
                            function onEnd() { cleanup(); resolve({ done: true }); }
                            self.once('data', onData);
                            self.once('end', onEnd);
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
                    err.code = 'ERR_STREAM_WRITE_AFTER_END';
                    // Node reports the error on a later tick (via destroy),
                    // so `write()` still returns false synchronously and the
                    // uncaught exception only fires once the loop runs.
                    var self = this;
                    var next = (typeof process !== 'undefined' && typeof process.nextTick === 'function')
                        ? process.nextTick : function(f) { queueMicrotask(f); };
                    next(function() { self.emit('error', err); });
                    if (cb) cb(err);
                    return false;
                }
                var chunkLen = 1;
                if (chunk instanceof Uint8Array) {
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
                        // Deferred: with a synchronous `_write` (net.Socket, fs,
                        // custom Writables) onflush runs *inside* write(), before
                        // the caller can attach its 'drain' listener — emitting
                        // now lost the event and a caller waiting on it stalled.
                        // Node emits 'drain' on the next tick for the same reason.
                        var selfD = self;
                        (typeof queueMicrotask === 'function' ? queueMicrotask : function(f) { setTimeout(f, 0); })(function() { selfD.emit('drain'); });
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
                if (state.ended || state.finished) { if (cb) cb(); return this; }
                function finish() {
                    if (state.pendingcb > 0) { setTimeout(finish, 0); return; }
                    state.finished = true;
                    self._final(function(err) {
                        if (err) self.emit('error', err);
                        self.emit('finish');
                        // A socket must stay open after end() until its read
                        // side is done (see Socket._maybeClose), so it opts out
                        // of the auto-'close' that a plain Writable emits.
                        if (!self._noAutoClose) self.emit('close');
                        if (cb) cb(err || null);
                    });
                }
                // The chunk passed to end() is written *before* the stream is
                // marked ended. Doing it after hit the write-after-end guard:
                // `PassThrough.end(buf)` and every `socket.end(data)` dropped
                // both the chunk and the final flush, and the process exited 0
                // with the work still pending.
                if (chunk != null) {
                    this.write(chunk, encoding, function(err) {
                        if (err) { if (cb) cb(err); return; }
                        state.ended = true;
                        finish();
                    });
                } else {
                    state.ended = true;
                    finish();
                }
                return this;
            };
            Writable.prototype.destroy = function(err) { return __destroyStream(this, err); };
            // TODO(2.13.0): implement — setDefaultEncoding must set the encoding used by write(string); see docs/05-js-engine/07-inventario-stubs.md §7.4.
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
                if (state.ended || state.finished) { if (cb) cb(); return this; }
                function doFlush() {
                    self._flush(function(err, data) {
                        if (err) { self.emit('error', err); if (cb) cb(err); return; }
                        if (data != null) self.push(data);
                        self._final(function(err2) {
                            if (err2) { self.emit('error', err2); if (cb) cb(err2); return; }
                            state.finished = true;
                            self.push(null);
                            self.emit('finish');
                            if (!self._noAutoClose) self.emit('close');
                            if (cb) cb(null);
                        });
                    });
                }
                // Same ordering fix as Writable.end: write the final chunk while
                // the stream still accepts writes. PassThrough.end(buf) used to
                // lose the chunk entirely.
                if (chunk != null) {
                    self.write(chunk, encoding, function(err) {
                        if (err) { if (cb) cb(err); return; }
                        state.ended = true;
                        doFlush();
                    });
                } else {
                    state.ended = true;
                    doFlush();
                }
                return this;
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

            // Node.js: require('stream') returns the Stream class itself with sub-classes as properties
            Stream.Readable = Readable;
            Stream.Writable = Writable;
            Stream.Transform = Transform;
            Stream.PassThrough = PassThrough;
            Stream.Duplex = Duplex;
            Stream.Stream = Stream;
            Stream.EventEmitter = EventEmitter;
            Stream.isDisturbed = function(s) {
                return !!(s && s._readableState && (s._readableState.disturbed || s._readableState.ended || s._readableState.aborted));
            };
            Stream.isErrored = function(s) {
                return !!(s && s._readableState && s._readableState.errored != null);
            };
            Stream.isReadable = function(s) {
                return !!(s && typeof s.read === 'function' && s._readableState && !s._readableState.ended && !s._readableState.errored);
            };

            // ── Readable.from / fromWeb / toWeb ──────────────────────────────
            // Standard bridges between Node streams and the Web Streams API
            // (Readable.from since Node 12.3; fromWeb/toWeb since Node 17).
            // Without them a `fetch()` response body or any async iterable is
            // unusable with the rest of the stream module.
            Readable.from = function(iterable, opts) {
                if (iterable == null) throw new TypeError('Readable.from: iterable is required');
                var r = new Readable(opts || {});
                var objectMode = !!(opts && opts.objectMode);
                var it;
                if (typeof iterable[Symbol.asyncIterator] === 'function') {
                    it = iterable[Symbol.asyncIterator]();
                } else if (typeof iterable[Symbol.iterator] === 'function') {
                    it = iterable[Symbol.iterator]();
                } else {
                    throw new TypeError('Readable.from: iterable must implement Symbol.iterator or Symbol.asyncIterator');
                }
                (async function() {
                    try {
                        while (true) {
                            var res = await it.next();
                            if (!res || res.done) break;
                            var chunk = res.value;
                            if (!objectMode && typeof chunk !== 'string' && !Buffer.isBuffer(chunk)) {
                                chunk = chunk instanceof Uint8Array ? Buffer.from(chunk) : Buffer.from(String(chunk));
                            }
                            r.push(chunk);
                        }
                        r.push(null);
                    } catch (e) {
                        r.destroy(e);
                    }
                })();
                return r;
            };

            Readable.fromWeb = function(webStream, opts) {
                if (webStream == null || typeof webStream.getReader !== 'function') {
                    throw new TypeError('Readable.fromWeb: expected a web ReadableStream');
                }
                var r = new Readable(opts || {});
                var objectMode = !!(opts && opts.objectMode);
                var reader = webStream.getReader();
                (async function() {
                    try {
                        while (true) {
                            var res = await reader.read();
                            if (!res || res.done) break;
                            var chunk = res.value;
                            if (!objectMode && chunk != null && typeof chunk !== 'string' && !Buffer.isBuffer(chunk)) {
                                chunk = Buffer.from(chunk);
                            }
                            r.push(chunk);
                        }
                        r.push(null);
                    } catch (e) {
                        r.destroy(e);
                    }
                })();
                return r;
            };

            Readable.toWeb = function(nodeStream) {
                if (nodeStream == null || typeof nodeStream[Symbol.asyncIterator] !== 'function') {
                    throw new TypeError('Readable.toWeb: expected a Node readable stream');
                }
                var it = nodeStream[Symbol.asyncIterator]();
                return new ReadableStream({
                    pull: async function(controller) {
                        try {
                            var res = await it.next();
                            if (res.done) controller.close();
                            else controller.enqueue(res.value);
                        } catch (e) {
                            controller.error(e);
                        }
                    },
                    cancel: function(reason) {
                        if (typeof nodeStream.destroy === 'function') nodeStream.destroy(reason);
                    },
                });
            };

            // ── pipeline() ───────────────────────────────────────────────────
            // Shared by stream.pipeline (callback) and stream/promises.pipeline.
            // Accepts Node streams *and* AsyncIterables, plus async generator
            // functions / async functions as transforms, exactly like Node ≥13:
            // a function stage is called with the previous stage's async
            // iterable and whatever it returns becomes the next stage.
            function pipelineDrain(w) {
                return new Promise(function(resolve) {
                    // A synchronous `_write` (fs, sockets, custom Writables) can
                    // return `false` from `write()` and already be drained by
                    // the time we look — the `'drain'` it emitted ran before we
                    // could attach a listener. Without this check the pipeline
                    // waited for a drain that had already happened and stalled
                    // after the first chunk.
                    var st = w && w._writableState;
                    if (!st || st.length < st.highWaterMark) { resolve(); return; }
                    w.once('drain', resolve);
                });
            }
            function pipelineEnd(w) {
                return new Promise(function(resolve, reject) {
                    var settled = false;
                    function ok() { if (!settled) { settled = true; cleanup(); resolve(); } }
                    function err(e) { if (!settled) { settled = true; cleanup(); reject(e); } }
                    function cleanup() {
                        w.removeListener('finish', ok);
                        w.removeListener('close', ok);
                        w.removeListener('error', err);
                    }
                    w.once('finish', ok);
                    w.once('close', ok);
                    w.once('error', err);
                    w.end();
                });
            }
            function pipelineToAsyncIterable(x) {
                if (x == null) throw new TypeError('pipeline: stage is null or undefined');
                if (typeof x[Symbol.asyncIterator] === 'function') return x;
                if (typeof x[Symbol.iterator] === 'function') {
                    var it = x[Symbol.iterator]();
                    var o = {};
                    o[Symbol.asyncIterator] = function() {
                        return { next: function() { return Promise.resolve(it.next()); } };
                    };
                    return o;
                }
                throw new TypeError('pipeline: stage is not an async iterable');
            }
            // Feed `source` into a Duplex/Transform and expose its readable side
            // as the next async iterable.
            function pipelineThrough(source, transform) {
                var o = {};
                o[Symbol.asyncIterator] = function() {
                    // Attach an error handler before any write: a transform that
                    // fails mid-stream emits 'error' synchronously from
                    // `_transform`'s callback, and without a listener (in 3va
                    // emit('error') with none is dropped) the failure vanished
                    // and the pipeline reported success.
                    var rejectError;
                    var errorPromise = new Promise(function(_res, rej) { rejectError = rej; });
                    function onErr(e) { rejectError(e); }
                    errorPromise.catch(function() {});
                    transform.once('error', onErr);
                    var wiring = (async function() {
                        try {
                            for await (var c of source) {
                                if (transform.write(c) === false) await pipelineDrain(transform);
                            }
                            await pipelineEnd(transform);
                        } catch (e) {
                            if (typeof transform.destroy === 'function') transform.destroy(e);
                            throw e;
                        }
                    })();
                    wiring.catch(function() {});
                    var it = transform[Symbol.asyncIterator]();
                    return {
                        next: function() {
                            return Promise.race([
                                it.next(),
                                errorPromise.then(function() { return { done: true }; }, function(e) { throw e; })
                            ]);
                        },
                        return: async function() {
                            transform.removeListener('error', onErr);
                            if (typeof it.return === 'function') await it.return();
                            await wiring.catch(function() {});
                            return { done: true };
                        },
                    };
                };
                return o;
            }
            function pipelineImpl(streams) {
                return new Promise(function(resolve, reject) {
                    if (!streams || streams.length < 2) {
                        reject(new Error('pipeline: need at least 2 streams'));
                        return;
                    }
                    (async function() {
                        var last = streams.length - 1;
                        var current = typeof streams[0] === 'function'
                            ? pipelineToAsyncIterable(streams[0]())
                            : pipelineToAsyncIterable(streams[0]);
                        for (var i = 1; i < last; i++) {
                            var st = streams[i];
                            if (typeof st === 'function') {
                                current = pipelineToAsyncIterable(st(current));
                            } else if (st && typeof st.write === 'function'
                                       && typeof st[Symbol.asyncIterator] === 'function') {
                                current = pipelineThrough(current, st);
                            } else {
                                current = pipelineToAsyncIterable(st);
                            }
                        }
                        var dest = streams[last];
                        if (typeof dest === 'function') {
                            var out = dest(current);
                            if (out && typeof out.then === 'function') await out;
                            else if (out && typeof out[Symbol.asyncIterator] === 'function') {
                                for await (var _d of pipelineToAsyncIterable(out)) { /* drain */ }
                            }
                        } else if (dest && typeof dest.write === 'function') {
                            for await (var chunk of current) {
                                if (dest.write(chunk) === false) await pipelineDrain(dest);
                            }
                            await pipelineEnd(dest);
                        } else {
                            for await (var _e of pipelineToAsyncIterable(dest)) { /* drain */ }
                        }
                    })().then(resolve, reject);
                });
            }
            // callback-based pipeline: pipeline(src, ...transforms, dst, cb) returns dst
            Stream.pipeline = function() {
                var args = Array.prototype.slice.call(arguments);
                var cb = typeof args[args.length - 1] === 'function' ? args.pop() : function(err) { if (err) console.error('[3va-pipeline] error:', err); };
                var ret = args.length ? args[args.length - 1] : new PassThrough();
                pipelineImpl(args).then(function() { cb(null); }, function(err) { cb(err); });
                return ret;
            };
            Stream.finished = function(stream, options, cb) {
                if (typeof options === 'function') { cb = options; options = {}; }
                options = options || {};
                cb = cb || function() {};
                var noop = function() {};
                if (!stream || typeof stream.on !== 'function') {
                    queueMicrotask(function() { cb(); });
                    return noop;
                }
                var settled = false;
                function onEnd() { done(null); }
                function onFinish() { done(null); }
                function onError(err) { done(err); }
                // Node's eos (which finished() is built on) also completes on
                // 'close': a stream destroyed before it finished is done as far
                // as the caller is concerned, and http responses emit 'close'
                // right after 'finish'.
                function onClose() { done(null); }
                function cleanup() {
                    stream.removeListener('end', onEnd);
                    stream.removeListener('finish', onFinish);
                    stream.removeListener('error', onError);
                    stream.removeListener('close', onClose);
                }
                // A Duplex emits both 'end' and 'finish'; without the guard the
                // callback ran twice (Node calls it exactly once).
                function done(err) {
                    if (settled) return;
                    settled = true;
                    cleanup();
                    // Node documents finished()'s callback as asynchronous, and
                    // code that emits 'finish' and then 'close' in a row counts
                    // on the callback running after both (it used to run inside
                    // the 'finish' emit, so the second one saw a stream that had
                    // not closed yet).
                    queueMicrotask(function() { cb(err); });
                }
                // Already over: still call back, but on a later tick, like Node.
                if (stream.readableEnded || stream.writableFinished || stream.destroyed || stream.finished === true) {
                    queueMicrotask(function() { cb(null); });
                    return noop;
                }
                stream.once('end', onEnd);
                stream.once('finish', onFinish);
                stream.once('error', onError);
                stream.once('close', onClose);
                return function() { done(new Error('aborted')); };
            };
            globalThis.__requireCache['stream'] = Stream;
            globalThis.__requireCache['node:stream'] = Stream;

            // fs.ReadStream / fs.WriteStream are defined in the fs module, which
            // is injected before the stream module exists, so they can't inherit
            // from Readable/Writable at definition time. Wire up their prototypes
            // here (still before any app code runs) so packages like graceful-fs
            // that do `new fs.WriteStream(...)` or `fs.WriteStream.apply(this,..)`
            // get working Writable/Readable instances.
            (function() {
                var _fs = globalThis.__requireCache['fs'];
                if (_fs && _fs.ReadStream) {
                    _fs.ReadStream.prototype = Object.create(Readable.prototype);
                    _fs.ReadStream.prototype.constructor = _fs.ReadStream;
                }
                if (_fs && _fs.WriteStream) {
                    _fs.WriteStream.prototype = Object.create(Writable.prototype);
                    _fs.WriteStream.prototype.constructor = _fs.WriteStream;
                }
            })();

            (function() {
                var streamPromises = {
                    pipeline: function() {
                        return pipelineImpl(Array.prototype.slice.call(arguments));
                    },
                    finished: function(stream, options) {
                        return new Promise(function(resolve, reject) {
                            if (!stream || typeof stream.on !== 'function') { resolve(); return; }
                            stream.on('end', resolve);
                            stream.on('finish', resolve);
                            stream.on('error', reject);
                            stream.on('close', function() {
                                if (!stream.destroyed) resolve();
                            });
                        });
                    }
                };
                globalThis.__requireCache['stream/promises'] = streamPromises;
                globalThis.__requireCache['node:stream/promises'] = streamPromises;
            })();

            // node:console — just the global console object.
            (function() {
                var consoleStub = globalThis.console || {};
                globalThis.__requireCache['console'] = globalThis.__requireCache['console'] || consoleStub;
                globalThis.__requireCache['node:console'] = consoleStub;
            })();

            // node:stream/consumers — text()/json()/buffer()/blob() helpers.
            (function() {
                var streamConsumers = {
                    text: function(stream) {
                        return new Promise(function(resolve, reject) {
                            var chunks = [];
                            stream.on('data', function(c) { chunks.push(typeof c === 'string' ? c : Buffer.from(c).toString()); });
                            stream.on('end', function() { resolve(chunks.join('')); });
                            stream.on('error', reject);
                        });
                    },
                    json: function(stream) {
                        return streamConsumers.text(stream).then(JSON.parse);
                    },
                    buffer: function(stream) {
                        return new Promise(function(resolve, reject) {
                            var chunks = [];
                            stream.on('data', function(c) { chunks.push(Buffer.isBuffer(c) ? c : Buffer.from(c)); });
                            stream.on('end', function() { resolve(Buffer.concat(chunks)); });
                            stream.on('error', reject);
                        });
                    },
                    blob: function(stream) {
                        return streamConsumers.buffer(stream).then(function(b) { return new Blob([b]); });
                    },
                    arrayBuffer: function(stream) {
                        return streamConsumers.buffer(stream).then(function(b) { return b.buffer; });
                    },
                };
                globalThis.__requireCache['stream/consumers'] = streamConsumers;
                globalThis.__requireCache['node:stream/consumers'] = streamConsumers;
            })();

            // node:stream/web — Web Streams API; 3va exposes these as globals already.
            (function() {
                var streamWeb = {
                    ReadableStream: globalThis.ReadableStream,
                    WritableStream: globalThis.WritableStream,
                    TransformStream: globalThis.TransformStream,
                    ReadableStreamDefaultReader: globalThis.ReadableStreamDefaultReader,
                    ReadableStreamBYOBReader: globalThis.ReadableStreamBYOBReader,
                    ReadableStreamDefaultController: globalThis.ReadableStreamDefaultController,
                    ReadableByteStreamController: globalThis.ReadableByteStreamController,
                    WritableStreamDefaultWriter: globalThis.WritableStreamDefaultWriter,
                    WritableStreamDefaultController: globalThis.WritableStreamDefaultController,
                    TransformStreamDefaultController: globalThis.TransformStreamDefaultController,
                    ByteLengthQueuingStrategy: globalThis.ByteLengthQueuingStrategy,
                    CountQueuingStrategy: globalThis.CountQueuingStrategy,
                    TextEncoderStream: globalThis.TextEncoderStream,
                    TextDecoderStream: globalThis.TextDecoderStream,
                };
                globalThis.__requireCache['stream/web'] = streamWeb;
                globalThis.__requireCache['node:stream/web'] = streamWeb;
            })();

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
                // Asked when read, not while loading: whether stdin is a
                // terminal belongs to this run (startup snapshot).
                Object.defineProperty(stdin, 'isTTY', {
                    configurable: true, enumerable: true,
                    get: function() { return typeof __isatty === 'function' ? __isatty(0) : false; },
                    set: function(v) { Object.defineProperty(stdin, 'isTTY', { value: v, writable: true, configurable: true, enumerable: true }); },
                });
                stdin._paused = true;
                stdin._ended = false;
                stdin._encoding = null;
                stdin._decoder = null;
                // Encoding is honored through a long-lived StringDecoder, not
                // a fresh TextDecoder per chunk: a fresh decoder replaces any
                // multi-byte sequence split across two reads with U+FFFD.
                stdin.setEncoding = function(enc) {
                    this._encoding = enc;
                    this._decoder = (enc && enc !== 'buffer') ? new StringDecoder(enc) : null;
                    return this;
                };
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
                            if (self._decoder) {
                                var tail = self._decoder.end();
                                self._decoder = null;
                                if (tail) self.emit('data', tail);
                            }
                            self.emit('end');
                            self.emit('close');
                            return;
                        }
                        var data = self._decoder
                            ? self._decoder.write(chunk)
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
                // Expose the raw (unproxied) env object so one-time init
                // spreads (e.g. `__vvva_meta_env__`'s Object.assign over
                // process.env) copy values without firing the read-audit
                // `get` trap for every variable — otherwise `3va permissions
                // learn` would report the entire host environment instead of
                // only the variables the script actually read.
                globalThis.__vvva_env_raw__ = __envTarget;
                process.env = new Proxy(__envTarget, {
                    get: function(target, prop) {
                        if (prop === '__isProxy') return true;
                        // Record a real read of this variable (used by `3va
                        // permissions learn`). __envTarget already holds only
                        // the permitted variables, so this re-check changes
                        // nothing about what callers can see — it only makes
                        // the capability check visible to the audit log.
                        if (typeof prop === 'string') {
                            if (prop in target) {
                                if (typeof __envAudit === 'function') __envAudit(prop);
                            } else if (typeof __envRequest === 'function') {
                                // Not granted (or not set): ask the permission layer,
                                // which may prompt, audits the denial, and lets
                                // `3va run` report the hidden read at exit.
                                var granted = __envRequest(prop);
                                if (granted !== undefined) { target[prop] = granted; return granted; }
                            }
                        }
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
                    // Node.js accepts createInterface(stream) or createInterface({input:stream,...})
                    if (options && typeof options === 'object' && typeof options.on === 'function' && !options.input)
                        options = { input: options };
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
                            // Raw string split on the last separator — like Node,
                            // NOT the "." / ".." resolving splitNormalize() used by
                            // join/resolve/normalize. A trailing "." is a literal
                            // path segment here (dirname("/a/b/.") === "/a/b"), not
                            // something to collapse away first — collapsing it
                            // before popping the last segment strips one level too
                            // many (this broke Metro's entry-point resolution,
                            // which represents "the project root itself" as
                            // projectRoot + "/.").
                            var isAbsolute = p.isAbsolute(path);
                            var end = path.length;
                            while (end > 1 && path.charAt(end - 1) === sep) end--;
                            var trimmed = path.slice(0, end);
                            var idx = trimmed.lastIndexOf(sep);
                            if (idx === -1) return isAbsolute ? sep : '.';
                            if (idx === 0) return sep;
                            return trimmed.slice(0, idx);
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
                globalThis.__requireCache['node:path'] = posixPath;
                globalThis.__requireCache['path/posix'] = posixPath;
                globalThis.__requireCache['node:path/posix'] = posixPath;
                globalThis.__requireCache['path/win32'] = win32Path;
                globalThis.__requireCache['node:path/win32'] = win32Path;
            })();

            // ── module (node:module) ─────────────────────────────────────────────
            // Next.js monkey-patches module.prototype.require and module._resolveFilename,
            // so module must be a constructor-like function with .prototype.require.
            (function() {
                function Module(id, parent) {
                    this.id = id || '';
                    this.filename = id || '';
                    this.parent = parent || null;
                    this.children = [];
                    this.exports = {};
                    this.loaded = false;
                }
                Module.prototype.require = function(id) {
                    var dir = (this.filename || '').replace(/[/\\][^/\\]*$/, '') || globalThis.__dirname || '';
                    return globalThis.__vvva_require_from
                        ? globalThis.__vvva_require_from(id, dir)
                        : globalThis.require(id);
                };
                Module._resolveFilename = function(request, parent, isMain, options) {
                    var dir = (parent && parent.filename) ? parent.filename.replace(/[/\\][^/\\]*$/, '') : (globalThis.__dirname || '');
                    return __requireResolve(request, dir);
                };
                Module._cache = globalThis.__loadedModules || {};
                Module._extensions = {};
                Module._load = function(request, parent, isMain) {
                    return Module.prototype.require.call(parent || { filename: '' }, request);
                };
                Module.createRequire = function(filename) {
                    var fsPath = (typeof filename === 'string')
                        ? filename.replace(/^file:\/\/\/?/, '/')
                        : (globalThis.__dirname || '');
                    var dir = fsPath.replace(/[/\\][^/\\]*$/, '') || globalThis.__dirname || '';
                    var scopedRequire = function(id) {
                        return globalThis.__vvva_require_from
                            ? globalThis.__vvva_require_from(id, dir)
                            : globalThis.require(id);
                    };
                    scopedRequire.resolve = function(id) {
                        return __requireResolve(id, dir);
                    };
                    scopedRequire.resolve.paths = function() { return [dir]; };
                    scopedRequire.cache = globalThis.__loadedModules || {};
                    return scopedRequire;
                };
                Module.builtinModules = [
                    'assert','async_hooks','buffer','child_process','cluster',
                    'console','crypto','dgram','diagnostics_channel','dns',
                    'domain','events','fs','http','http2','https','inspector',
                    'module','net','os','path','perf_hooks','process','punycode',
                    'querystring','readline','repl','stream','string_decoder',
                    'sys','timers','tls','tty','url','util','v8','vm','wasi',
                    'worker_threads','zlib',
                ];
                Module.isBuiltin = function(id) {
                    var bare = id.startsWith('node:') ? id.slice(5) : id;
                    return Module.builtinModules.indexOf(bare) !== -1;
                };
                Module.syncBuiltinESMExports = function() {};
                Module.register = function() {};
                Module.wrap = function(script) { return '(function (exports, require, module, __filename, __dirname) { ' + script + '\n});'; };
                Module.wrapper = ['(function (exports, require, module, __filename, __dirname) { ', '\n});'];
                globalThis.__requireCache['module'] = Module;
                globalThis.__requireCache['node:module'] = Module;
            })();

            // Node.js require('buffer') returns { Buffer, SlowBuffer, Blob, File, constants, ...staticMethods }
            (function() {
                var B = globalThis.Buffer;
                var bufMod = { Buffer: B, SlowBuffer: B, Blob: globalThis.Blob, File: globalThis.File, atob: globalThis.atob, btoa: globalThis.btoa, constants: { MAX_LENGTH: 2147483647, MAX_STRING_LENGTH: 1073741823 } };
                ['from','alloc','allocUnsafe','allocUnsafeSlow','isBuffer','isEncoding','byteLength','concat','compare'].forEach(function(k) {
                    if (B[k]) bufMod[k] = B[k].bind(B);
                });
                globalThis.__requireCache['buffer'] = bufMod;
                globalThis.__requireCache['node:buffer'] = bufMod;
            })();
            globalThis.__requireCache['stream'] = globalThis.__requireCache['stream'] || Stream;

            // ── constants (deprecated, used by webpack) ─────────────────────────
            globalThis.__requireCache['constants'] = globalThis.__requireCache['constants'] || {};
            globalThis.__requireCache['node:constants'] = globalThis.__requireCache['constants'];

            // ── querystring ──────────────────────────────────────────────────────
            (function() {
                function encode(obj, sep, eq) {
                    sep = sep || '&'; eq = eq || '=';
                    return Object.keys(obj || {}).map(function(k) {
                        var v = obj[k];
                        if (Array.isArray(v)) return v.map(function(vi) { return encodeURIComponent(k) + eq + encodeURIComponent(vi); }).join(sep);
                        return encodeURIComponent(k) + eq + encodeURIComponent(v == null ? '' : v);
                    }).join(sep);
                }
                function decode(str, sep, eq) {
                    sep = sep || '&'; eq = eq || '=';
                    var obj = {};
                    String(str || '').split(sep).forEach(function(pair) {
                        if (!pair) return;
                        var idx = pair.indexOf(eq);
                        var k = idx < 0 ? pair : pair.slice(0, idx);
                        var v = idx < 0 ? '' : pair.slice(idx + 1);
                        k = decodeURIComponent(k.replace(/\+/g, ' '));
                        v = decodeURIComponent(v.replace(/\+/g, ' '));
                        if (obj[k] === undefined) obj[k] = v;
                        else if (Array.isArray(obj[k])) obj[k].push(v);
                        else obj[k] = [obj[k], v];
                    });
                    return obj;
                }
                var qs = { stringify: encode, parse: decode, encode: encode, decode: decode, escape: encodeURIComponent, unescape: decodeURIComponent };
                globalThis.__requireCache['querystring'] = qs;
                globalThis.__requireCache['node:querystring'] = qs;
            })();

            // ── perf_hooks ───────────────────────────────────────────────────────
            (function() {
                var _nativePerf = (typeof globalThis.performance !== 'undefined') ? globalThis.performance : null;
                // Wrap native performance in a plain object so we can add missing methods
                var perf = {
                    now: _nativePerf ? function() { return _nativePerf.now(); } : function() { return Date.now(); },
                    timeOrigin: _nativePerf ? _nativePerf.timeOrigin : 0,
                    mark: _nativePerf && _nativePerf.mark ? function() { return _nativePerf.mark.apply(_nativePerf, arguments); } : function() {},
                    measure: _nativePerf && _nativePerf.measure ? function() { return _nativePerf.measure.apply(_nativePerf, arguments); } : function() {},
                    clearMarks: _nativePerf && _nativePerf.clearMarks ? function() { return _nativePerf.clearMarks.apply(_nativePerf, arguments); } : function() {},
                    clearMeasures: _nativePerf && _nativePerf.clearMeasures ? function() { return _nativePerf.clearMeasures.apply(_nativePerf, arguments); } : function() {},
                    getEntries: _nativePerf && _nativePerf.getEntries ? function() { return _nativePerf.getEntries.apply(_nativePerf, arguments); } : function() { return []; },
                    getEntriesByName: _nativePerf && _nativePerf.getEntriesByName ? function() { return _nativePerf.getEntriesByName.apply(_nativePerf, arguments); } : function() { return []; },
                    getEntriesByType: _nativePerf && _nativePerf.getEntriesByType ? function() { return _nativePerf.getEntriesByType.apply(_nativePerf, arguments); } : function() { return []; },
                    markResourceTiming: function() {},
                    eventLoopUtilization: function() { return { idle: 0, active: 0, utilization: 0 }; },
                    nodeTiming: {},
                    toJSON: function() { return { now: this.now(), timeOrigin: this.timeOrigin }; }
                };
                globalThis.performance = perf;
                var perfHooks = { performance: perf, PerformanceObserver: globalThis.PerformanceObserver || function() {}, constants: {} };
                globalThis.__requireCache['perf_hooks'] = perfHooks;
                globalThis.__requireCache['node:perf_hooks'] = perfHooks;
            })();

            // ── whatwg-url ───────────────────────────────────────────────────────
            // ponytail: wraps V8 native URL/URLSearchParams; only URL+URLSearchParams
            // are used by mongodb-connection-string-url; add more if needed
            (function() {
                var whatwgUrl = {
                    URL: globalThis.URL,
                    URLSearchParams: globalThis.URLSearchParams,
                    parseURL: function(url) { try { return new globalThis.URL(url); } catch(e) { return null; } },
                    serializeURL: function(u) { return u ? u.href : ''; },
                    basicURLParse: function(url, base) { try { return new globalThis.URL(url, base); } catch(e) { return null; } },
                    serializeURLOrigin: function(u) { return u ? u.origin : ''; },
                };
                globalThis.__requireCache['whatwg-url'] = whatwgUrl;
            })();

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
                function urlParse(urlStr, parseQueryString, slashesDenoteHost) {
                    try {
                        var s = String(urlStr);
                        // WHATWG URL requires an absolute base; legacy url.parse()
                        // also accepts relative URLs like "/foo?bar=1". Fall back to
                        // a dummy base and null out the host fields for relative input.
                        var relative = !/^[a-zA-Z][a-zA-Z0-9+.\-]*:/.test(s);
                        var u = new globalThis.URL(s, relative ? 'http://x/' : undefined);
                        var obj = {
                            href: relative ? s : u.href,
                            protocol: relative ? null : u.protocol,
                            slashes: !relative && (u.protocol === 'http:' || u.protocol === 'https:' || u.protocol === 'ftp:'),
                            auth: u.username ? (u.password ? u.username + ':' + u.password : u.username) : null,
                            host: relative ? null : u.host,
                            port: relative ? null : (u.port || null),
                            hostname: relative ? null : u.hostname,
                            hash: u.hash || null,
                            search: u.search || null,
                            query: parseQueryString ? Object.fromEntries(u.searchParams) : (u.search ? u.search.slice(1) : null),
                            pathname: u.pathname,
                            path: u.pathname + (u.search || ''),
                        };
                        return obj;
                    } catch(e) { return { href: urlStr, protocol: null, host: null, hostname: null, pathname: urlStr, path: urlStr, query: null, hash: null }; }
                }
                function urlFormat(obj) {
                    if (typeof obj === 'string') return obj;
                    if (obj && obj.href) return obj.href;
                    // Node normalizes a protocol missing its trailing ':' (e.g.
                    // { protocol: 'http' }, as react-native's own CLI passes) —
                    // without this, the result is missing the colon entirely
                    // ('http//host:port/'), which `new URL()` then rejects.
                    var proto = obj.protocol || '';
                    if (proto && proto.charAt(proto.length - 1) !== ':') proto += ':';
                    var s = proto + '//';
                    if (obj.auth) s += obj.auth + '@';
                    s += (obj.host || obj.hostname || '');
                    if (obj.port) s += ':' + obj.port;
                    s += (obj.pathname || '/');
                    if (obj.search) s += obj.search;
                    else if (obj.query && typeof obj.query === 'object') s += '?' + Object.entries(obj.query).map(function(kv) { return encodeURIComponent(kv[0]) + '=' + encodeURIComponent(kv[1]); }).join('&');
                    if (obj.hash) s += obj.hash;
                    return s;
                }
                function urlResolve(from, to) {
                    try { return new globalThis.URL(to, from).href; } catch(e) { return to; }
                }
                globalThis.__requireCache['url'] = {
                    URL: globalThis.URL,
                    URLSearchParams: globalThis.URLSearchParams,
                    pathToFileURL: pathToFileURL,
                    fileURLToPath: fileURLToPath,
                    parse: urlParse,
                    format: urlFormat,
                    resolve: urlResolve,
                };
                globalThis.__requireCache['node:url'] = globalThis.__requireCache['url'];
            })();

            // ── net / tls — raw TCP sockets, backed by the native __tcp*/__net* ────
            // bindings in builtins/tcp.rs. Those bindings return either a number
            // (success) or an Error/string (failure) rather than throwing, so every
            // call site below checks the type of the result instead of using try/catch.
            function _netErr(v) { return v instanceof Error ? v : new Error(String(v)); }

            // Node's AbortError (internal/errors.js): name 'AbortError', code
            // ABORT_ERR, and the signal's reason as `cause`.
            function __abortError(signal) {
                var reason = signal && signal.reason;
                var e;
                if (reason instanceof Error && reason.name === 'AbortError') e = reason;
                else {
                    e = new Error(reason && reason.message ? reason.message : 'The operation was aborted');
                    e.name = 'AbortError';
                }
                if (!e.code) e.code = 'ABORT_ERR';
                e.cause = reason;
                return e;
            }
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
                // Node's default is `allowHalfOpen: false`: when the peer sends
                // FIN (read EOF) the writable side ends too and the socket
                // closes. With `true`, the socket stays writable after the peer
                // half-closes.
                this._allowHalfOpen = !!(opts.allowHalfOpen);
                this._readEnded = false;
                this._writableEnded = false;
                this._shutdownWrite = false;
                // Suppresses the `'close'` that Writable.end would otherwise
                // emit while the read side is still open: a socket closes once,
                // after both directions are done (see _maybeClose).
                this._noAutoClose = true;
                if (this._connId !== null) this._startPoll();
            }
            util.inherits(Socket, Duplex);

            Socket.prototype._startPoll = function() {
                var self = this;
                if (self._pollTimer || self.destroyed) return;
                // Read is polled, not event-driven: a blocking read would
                // stall the event loop, so a JS timer asks the non-blocking
                // native for data. A fixed interval is the worst of both
                // worlds (a 5 ms idle poll also costs a 5 ms wait per read on
                // a busy socket), so the interval is adaptive: 1 ms while the
                // socket is actively exchanging data, backing off to 5 ms
                // after a stretch of idle polls.
                var idleTicks = 0;
                var step = function() {
                    if (self.destroyed || self._connId === null) return;
                    var chunk = __tcpRead(self._connId, 65536);
                    if (chunk instanceof Uint8Array) {
                        idleTicks = 0;
                        self.push(Buffer.from(chunk));
                    } else if (chunk instanceof Error && chunk.code === 'EAGAIN') {
                        idleTicks += 1;
                        // no data available yet — keep polling
                    } else if (chunk instanceof Error && chunk.code === 'EOF') {
                        self._stopPoll();
                        self._readEnded = true;
                        self.push(null);
                        // allowHalfOpen === false (Node's default): the peer is
                        // done, so end our side and close. With true we keep the
                        // write side open until the application ends it.
                        if (!self._allowHalfOpen) { self.destroy(); }
                        else { self._maybeClose(); }
                        return;
                    } else {
                        self._stopPoll();
                        self.emit('error', _netErr(chunk));
                        self.destroy();
                        return;
                    }
                    // A 'data' listener may have destroyed the socket; do not
                    // re-arm then (the timer could never be cancelled).
                    if (self.destroyed || self._connId === null) { self._pollTimer = null; return; }
                    self._pollTimer = setTimeout(step, idleTicks < 8 ? 1 : 5);
                };
                // A chain of timeouts, not intervals: clearing an interval from
                // inside its own callback leaks the callback entry in the engine.
                self._pollTimer = setTimeout(step, 5);
            };
            Socket.prototype._stopPoll = function() {
                if (this._pollTimer) { clearTimeout(this._pollTimer); this._pollTimer = null; }
            };

            Socket.prototype._write = function(chunk, encoding, cb) {
                if (this._connId === null) { cb(new Error('Socket is not connected')); return; }
                var data = (chunk instanceof Uint8Array) ? chunk : new TextEncoder().encode(String(chunk));
                var result = __tcpWrite(this._connId, data);
                if (result === undefined) cb(); else cb(_netErr(result));
            };

            Socket.prototype.connect = function(port, host, cb) {
                var self = this;
                // Support options object form: connect({host, port}[, cb])
                if (port && typeof port === 'object') {
                    if (typeof host === 'function') cb = host;
                    var opts = port;
                    port = opts.port;
                    host = opts.host || 'localhost';
                } else {
                    if (typeof host === 'function') { cb = host; host = 'localhost'; }
                    host = host || 'localhost';
                }
                if (typeof cb === 'function') self.once('connect', cb);
                // Node's Socket.prototype.connect honours `options.signal`:
                // an already-aborted signal fails the connection on the next
                // tick and a later abort destroys it with an AbortError. Net
                // code (and http.Agent.createConnection, which forwards the
                // request's signal) relies on it.
                var signal = (opts && opts.signal) || null;
                if (signal) {
                    if (signal.aborted) {
                        setTimeout(function() { self.destroy(__abortError(signal)); }, 0);
                        return self;
                    }
                    var onAbort = function() {
                        if (self._abortSignal !== signal) return;
                        self._abortSignal = null;
                        self._abortListener = null;
                        self.destroy(__abortError(signal));
                    };
                    self._abortSignal = signal;
                    self._abortListener = onAbort;
                    if (typeof signal.addEventListener === 'function') signal.addEventListener('abort', onAbort);
                }
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
                if (typeof ms === 'function') { cb = ms; ms = 0; }
                if (typeof cb === 'function') this.once('timeout', cb);
                // Node keeps the current timeout in `socket.timeout` and
                // fires the 'timeout' event when it expires.
                this.timeout = ms;
                if (this._timeoutTimer) { clearTimeout(this._timeoutTimer); this._timeoutTimer = null; }
                if (ms > 0) {
                    var self = this;
                    this._timeoutTimer = setTimeout(function() {
                        self._timeoutTimer = null;
                        self.emit('timeout');
                    }, ms);
                }
                return this;
            };
            Socket.prototype.setNoDelay = function(on) {
                // Real toggle now (TCP_NODELAY was stuck on anyway, see tcp.rs);
                // a truthy or absent argument enables it.
                if (this._connId !== null) __tcpSetNoDelay(this._connId, on === false ? 0 : 1);
                return this;
            };
            // ponytail: no-op by design — TCP socket options (SO_KEEPALIVE / TCP_NODELAY) are performance hints, not semantics; see docs/05-js-engine/07-inventario-stubs.md §7.2.
            Socket.prototype.setKeepAlive = function() { return this; };
            // ponytail: no-ops — pg-pool/pg call socket.ref()/unref() on idle clients; unref() does not
            // yet release the event loop (a pool with idle clients keeps the process alive until pool.end()).
            Socket.prototype.ref = function() { return this; };
            Socket.prototype.unref = function() { return this; };
            Socket.prototype.destroy = function(err) {
                this._stopPoll();
                if (this._abortSignal && this._abortListener) {
                    var sig = this._abortSignal, fn = this._abortListener;
                    this._abortSignal = null;
                    this._abortListener = null;
                    if (sig && typeof sig.removeEventListener === 'function') sig.removeEventListener('abort', fn);
                }
                if (this._timeoutTimer) { clearTimeout(this._timeoutTimer); this._timeoutTimer = null; }
                if (this._connId !== null) { __tcpClose(this._connId); this._connId = null; }
                // __destroyStream sets `destroyed`/`errored` and schedules
                // 'error'/'close'; setting it here first made destroy(err)
                // drop the error (the guard in __destroyStream saw it as an
                // already-destroyed stream with no listener yet).
                return Duplex.prototype.destroy.call(this, err);
            };
            // Close once the read side has ended *and* the write side is done
            // (or the socket is not allowed to stay half-open). Closing merely
            // because end() was called would drop every byte the peer sends
            // afterwards.
            Socket.prototype._maybeClose = function() {
                if (this.destroyed) return;
                if (this._readEnded && (this._writableEnded || this._allowHalfOpen === false)) this.destroy();
            };
            Socket.prototype.end = function(chunk, encoding, cb) {
                if (typeof chunk === 'function') { cb = chunk; chunk = null; }
                if (typeof encoding === 'function') { cb = encoding; encoding = null; }
                var self = this;
                if (self._writableEnded) { if (cb) cb(); return this; }
                self._writableEnded = true;
                return Duplex.prototype.end.call(this, chunk, encoding, function(err) {
                    if (err) { if (cb) cb(err); return; }
                    // Half-close: the write side is flushed, send FIN (or a TLS
                    // close_notify) and keep reading. Destroying here — as this
                    // did — lost every byte a server sent after the client's
                    // end(), which is the usual net echo pattern.
                    if (!self.destroyed && self._connId !== null && !self._shutdownWrite) {
                        self._shutdownWrite = true;
                        __tcpShutdownWrite(self._connId);
                    }
                    self._maybeClose();
                    if (cb) cb(err);
                });
            };

            // Duplex socket handed to `server.on('upgrade', (req, socket, head))`
            // for an HTTP `Upgrade` (WebSocket). Backed by the http layer's own
            // queue bridge (__upgradeRead/__upgradeWrite/__upgradeClose), with
            // the same pollable shape as net.Socket — the `ws` library drives
            // the 101 handshake and WebSocket frames over it.
            function UpgradeSocket(opts) {
                if (!(this instanceof UpgradeSocket)) return new UpgradeSocket(opts);
                opts = opts || {};
                Duplex.call(this, opts);
                this._connId = (typeof opts.fd === 'number') ? opts.fd : null;
                this.destroyed = false;
                this.connecting = false;
                this.remoteAddress = undefined;
                this.remotePort = undefined;
                this._pollTimer = null;
                this._unshifted = null;
                // Born paused, like a Node socket: nothing is read until a
                // consumer attaches a 'data' listener or calls resume() (see
                // below). Reading from the start lost whatever arrived before
                // the application got around to calling ws.handleUpgrade().
            }
            util.inherits(UpgradeSocket, Duplex);

            // resume()/pause() start and stop the native read poll. Paused, the
            // bytes stay in the native queue, which stops reading from the
            // network at its high-water mark: real backpressure.
            UpgradeSocket.prototype.resume = function() {
                Duplex.prototype.resume.call(this);
                if (this._connId !== null) this._startPoll();
                return this;
            };
            UpgradeSocket.prototype.pause = function() {
                Duplex.prototype.pause.call(this);
                this._stopPoll();
                return this;
            };
            // The shared Readable.unshift emits 'data' on the spot, which loses
            // the bytes when no listener is attached yet — and `ws` unshifts the
            // upgrade `head` just before attaching its listener. Keep them and
            // deliver them first, in order, once reading starts.
            UpgradeSocket.prototype.unshift = function(chunk) {
                if (chunk == null || chunk.length === 0) return this;
                (this._unshifted = this._unshifted || []).unshift(chunk);
                return this;
            };

            // Reads are polled: 1 ms while data is flowing, 5 ms once idle. A
            // chain of timeouts, not an interval that is cleared and recreated
            // on every step: the engine keeps the callback of a cleared
            // interval, so recreating one every millisecond leaked a closure
            // per step, and re-arming after a 'data' listener destroyed the
            // socket (ws does, on a bad frame) left a timer nobody could
            // cancel — the process then never exited.
            UpgradeSocket.prototype._startPoll = function() {
                var self = this;
                if (self._pollTimer || self.destroyed) return;
                var idleTicks = 0;
                var step = function() {
                    self._pollTimer = null;
                    if (self.destroyed || self._connId === null) return;
                    if (self._unshifted) {
                        var pending = self._unshifted;
                        self._unshifted = null;
                        for (var pi = 0; pi < pending.length; pi++) self.push(Buffer.from(pending[pi]));
                        if (self.destroyed || self._connId === null) return;
                    }
                    var chunk = __upgradeRead(self._connId, 65536);
                    if (chunk instanceof Uint8Array) {
                        idleTicks = 0;
                        self.push(Buffer.from(chunk));
                    } else if (chunk instanceof Error && chunk.code === 'EAGAIN') {
                        idleTicks += 1;
                    } else if (chunk instanceof Error && chunk.code === 'EOF') {
                        self.push(null);
                        return;
                    } else {
                        self.emit('error', chunk instanceof Error ? chunk : new Error(String(chunk)));
                        self.destroy();
                        return;
                    }
                    // A 'data' listener may have destroyed the socket.
                    if (self.destroyed || self._connId === null) return;
                    self._pollTimer = setTimeout(step, idleTicks < 8 ? 1 : 5);
                };
                self._pollTimer = setTimeout(step, 1);
            };
            UpgradeSocket.prototype._stopPoll = function() {
                if (this._pollTimer) { clearTimeout(this._pollTimer); this._pollTimer = null; }
            };

            UpgradeSocket.prototype._write = function(chunk, encoding, cb) {
                if (this._connId === null) { cb(new Error('Upgrade socket is not connected')); return; }
                var data = (chunk instanceof Uint8Array) ? chunk : new TextEncoder().encode(String(chunk));
                var result = __upgradeWrite(this._connId, data);
                if (result === undefined) cb(); else cb(_netErr(result));
            };

            UpgradeSocket.prototype.destroy = function(err) {
                this._stopPoll();
                if (this._connId !== null) { __upgradeClose(this._connId); this._connId = null; }
                return Duplex.prototype.destroy.call(this, err);
            };
            UpgradeSocket.prototype.end = function(chunk, encoding, cb) {
                var self = this;
                return Duplex.prototype.end.call(this, chunk, encoding, function(err) {
                    self.destroy();
                    if (cb) cb(err);
                });
            };
            // ponytail: no-op by design — see docs/05-js-engine/07-inventario-stubs.md §7.2.
            UpgradeSocket.prototype.setNoDelay = function() { return this; };
            UpgradeSocket.prototype.setTimeout = function(ms, cb) {
                if (typeof cb === 'function') this.once('timeout', cb);
                return this;
            };
            UpgradeSocket.prototype.setKeepAlive = function() { return this; };
            UpgradeSocket.prototype.address = function() {
                return { address: this.remoteAddress || '0.0.0.0', port: this.remotePort || 0, family: this.remoteAddress && this.remoteAddress.indexOf(':') !== -1 ? 'IPv6' : 'IPv4' };
            };
            UpgradeSocket.prototype.ref = function() { return this; };
            UpgradeSocket.prototype.unref = function() { return this; };

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

            Server.prototype.listen = function(port, host, backlog, cb) {
                var self = this;
                // Real Node accepts listen(port, host, backlog, callback) —
                // Metro's own earlyPortCheck calls it exactly that way
                // (backlog explicitly undefined, callback as the 4th arg).
                // Without shifting here, a 4-arg call silently drops the real
                // callback (this function used to only declare 3 params), so
                // the 'listening' event never fires and any caller awaiting
                // it — like earlyPortCheck's Promise — hangs forever.
                if (typeof backlog === 'function') { cb = backlog; backlog = undefined; }
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

            Server.prototype.setTimeout = function(ms, cb) { if (cb) this.once('timeout', cb); return this; };
            Server.prototype.keepAliveTimeout = 5000;
            Server.prototype.requestTimeout = 0;
            Server.prototype.headersTimeout = 60000;
            Server.prototype.maxHeadersCount = null;
            Server.prototype.maxConnections = 0;
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
            // ponytail: ref/unref are no-ops — see docs/05-js-engine/07-inventario-stubs.md §7.2.
            Server.prototype.unref = function() { return this; };
            Server.prototype.ref = function() { return this; };
            Server.prototype.getConnections = function(cb) { if (cb) cb(null, 0); };

            function netCreateServer(opts, connListener) { return new Server(opts, connListener); }
            function netConnect(port, host, cb) {
                // Options object form keeps `allowHalfOpen` (and the other
                // Socket options) — the old code always built `new Socket({})`,
                // so `net.connect({ port, allowHalfOpen: true })` silently lost
                // the flag.
                if (port && typeof port === 'object') {
                    var sock = new Socket(port);
                    sock._3vaSocket = true;
                    return sock.connect(port, host);
                }
                var s = new Socket({});
                s._3vaSocket = true;
                return s.connect(port, host, cb);
            }

            var __autoSelectFamilyAttemptTimeout = 250;
            globalThis.__requireCache['net'] = {
                Socket: Socket,
                Server: Server,
                createServer: netCreateServer,
                connect: netConnect,
                createConnection: netConnect,
                isIP: function(s) { return /^(\d{1,3}\.){3}\d{1,3}$/.test(s) ? 4 : (String(s).indexOf(':') !== -1 ? 6 : 0); },
                isIPv4: function(s) { return /^(\d{1,3}\.){3}\d{1,3}$/.test(s); },
                isIPv6: function(s) { return String(s).indexOf(':') !== -1; },
                // 3va has no Happy-Eyeballs dual-stack connection racing (the
                // feature this timeout actually governs in real Node) — this
                // is just a stored value so code that reads/writes it (like
                // test/common's startup tuning) doesn't crash, not a real
                // attempt-timeout that changes connection behavior.
                getDefaultAutoSelectFamily: function() { return true; },
                setDefaultAutoSelectFamily: function() {},
                getDefaultAutoSelectFamilyAttemptTimeout: function() { return __autoSelectFamilyAttemptTimeout; },
                setDefaultAutoSelectFamilyAttemptTimeout: function(ms) {
                    __autoSelectFamilyAttemptTimeout = Math.max(1, Number(ms) || 0);
                },
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
            TLSSocket.prototype.ref = Socket.prototype.ref;
            TLSSocket.prototype.unref = Socket.prototype.unref;

            function tlsConnect(port, host, opts, cb) {
                if (typeof host === 'object' && host !== null) { cb = opts; opts = host; host = (opts && opts.host) || 'localhost'; }
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                host = host || (opts && opts.host) || 'localhost';
                var s = new TLSSocket(null, opts || {});
                s._3vaSocket = true;
                if (typeof cb === 'function') s.once('secureConnect', cb);
                var tlsOpts = {
                    ca: opts && opts.ca !== undefined ? opts.ca : undefined,
                    cert: opts && opts.cert !== undefined ? opts.cert : undefined,
                    key: opts && opts.key !== undefined ? opts.key : undefined,
                    passphrase: opts && opts.passphrase !== undefined ? opts.passphrase : undefined,
                    rejectUnauthorized: opts && opts.rejectUnauthorized !== false,
                };
                var result = __tcpConnectTls(host, port, JSON.stringify(tlsOpts));
                if (typeof result === 'number') {
                    s._connId = result;
                    s._startPoll();
                    setTimeout(function() { s.emit('secureConnect'); s.emit('connect'); }, 0);
                } else {
                    setTimeout(function() { s.emit('error', _netErr(result)); }, 0);
                }
                return s;
            }

            // Real in-handshake hybrid PQ-TLS (RFC 10024 X25519MLKEM768) — see
            // docs/10-security/06-pq-tls-hybrid-design.md. Client-only; the
            // negotiated group falls back to plain X25519 against a server that
            // doesn't support the hybrid group, so this never fails a connection
            // that plain tls.connect() would have succeeded at.
            function tlsPqConnect(port, host, opts, cb) {
                if (typeof host === 'object' && host !== null) { cb = opts; opts = host; host = (opts && opts.host) || 'localhost'; }
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                host = host || (opts && opts.host) || 'localhost';
                var s = new TLSSocket(null, opts || {});
                var result = __pqTlsConnect(host, port, (opts && opts.ca) || null);
                if (typeof result === 'string') {
                    var parsed = JSON.parse(result);
                    s._connId = parsed.connId;
                    s.pqNegotiated = parsed.pqNegotiated;
                    s.negotiatedGroup = parsed.group;
                    s._startPoll();
                    if (typeof cb === 'function') s.once('secureConnect', cb);
                    setTimeout(function() { s.emit('secureConnect'); s.emit('connect'); }, 0);
                } else {
                    if (typeof cb === 'function') s.once('secureConnect', cb);
                    setTimeout(function() { s.emit('error', _netErr(result)); }, 0);
                }
                return s;
            }

            globalThis.__requireCache['tls'] = {
                TLSSocket: TLSSocket,
                connect: tlsConnect,
                pqConnect: tlsPqConnect,
                createSecureContext: function(options) {
                    options = options || {};
                    return {
                        context: {
                            ca: options.ca || null,
                            cert: options.cert || null,
                            key: options.key || null,
                            passphrase: options.passphrase || null,
                            rejectUnauthorized: options.rejectUnauthorized !== false,
                            servername: options.servername || null,
                            minVersion: options.minVersion || 'TLSv1.2',
                            maxVersion: options.maxVersion || 'TLSv1.3',
                            ciphers: options.ciphers || null,
                            honorCipherOrder: options.honorCipherOrder || false,
                            secureOptions: options.secureOptions || 0,
                        },
                        getCiphers: function() { return []; },
                    };
                },
                getCiphers: function() { return []; },
                DEFAULT_ECDH_CURVE: 'auto',
                DEFAULT_MAX_VERSION: 'TLSv1.3',
                DEFAULT_MIN_VERSION: 'TLSv1.2',
            };
            globalThis.__requireCache['node:tls'] = globalThis.__requireCache['tls'];

            // ── http module ──────────────────────────────────────────────────────────
            var httpAgent = function(options) {
                // Node allows `http.Agent()` without `new`.
                if (!(this instanceof httpAgent)) return new httpAgent(options);
                EventEmitter.call(this);
                this.options = options || {};
                this.maxSockets = Infinity;
                this.maxFreeSockets = 256;
                this.scheduling = 'lifo';
                this.sockets = {};
                this.freeSockets = {};
                this.requests = {};
                this.keepAlive = !!(options && options.keepAlive);
                this.keepAliveMsecs = (options && options.keepAliveMsecs) || 1000;
                this.options.keepAlive = this.keepAlive;
                this.options.keepAliveMsecs = this.keepAliveMsecs;
                this.maxCachedSessions = this.options.maxCachedSessions || 100;
            };
            httpAgent.prototype = Object.create(EventEmitter.prototype);
            httpAgent.prototype.constructor = httpAgent;
            httpAgent.prototype.addRequest = function(req, options) {};
            // Node's lib/_http_agent.js: the name of the pool a request goes
            // to. It is the key `agent.sockets` and `agent.freeSockets` are
            // indexed by, and the value `agent.getName()` returns.
            httpAgent.prototype.getName = function(options) {
                options = options || {};
                var name = options.host || 'localhost';
                name += ':';
                if (options.port) name += options.port;
                name += ':';
                if (options.localAddress) name += options.localAddress;
                if (options.family === 4 || options.family === 6) name += ':' + options.family;
                if (options.socketPath) name += ':' + options.socketPath;
                return name;
            };
            // The socket a request uses. Node's default is a plain TCP
            // connection (TLS for https). An agent that overrides this is how
            // the proxy agents work (https-proxy-agent, http-proxy-agent,
            // tunnel, global-agent): the request has to go through it and not
            // straight to the origin, or the proxy is bypassed.
            httpAgent.prototype.createConnection = function(options, cb) {
                options = options || {};
                var socket = options.secure
                    ? tlsConnect(options.port, options.host, {
                        servername: options.servername || options.host,
                        ca: options.ca,
                        rejectUnauthorized: options.rejectUnauthorized,
                    })
                    : netConnect({ host: options.host, port: options.port, signal: options.signal });
                if (typeof cb === 'function') socket.once('connect', function() { cb(null, socket); });
                return socket;
            };
            httpAgent.prototype.destroy = function() {
                // Node closes every socket the agent holds: the free ones and
                // the ones a request is using. Without this a keep-alive
                // socket stays open and the process never exits.
                var self = this;
                var close = function(map) {
                    Object.keys(map || {}).forEach(function(k) {
                        var list = map[k] || [];
                        for (var i = 0; i < list.length; i++) {
                            var s = list[i];
                            if (s && !s.destroyed && typeof s.destroy === 'function') s.destroy();
                        }
                        map[k] = [];
                    });
                };
                close(self.sockets);
                close(self.freeSockets);
            };
            httpAgent.prototype.free = function(socket, options) {};
            httpAgent.prototype.getCurrentStatus = function() { return { activeSockets: 0, freeSocketCount: 0, requestCount: 0 }; };
            httpAgent.prototype.keepSocketAlive = function(socket) {};
            httpAgent.prototype.reuseSocket = function(socket, req) {};
            httpAgent.prototype.onFREE = function(socket) {};
            httpAgent.prototype.onConnected = function(socket, options) {};
            httpAgent.prototype.onRemoved = function(socket) {};
            // Node keeps the sockets of the agent in `agent.sockets` and the
            // reusable ones in `agent.freeSockets`; the total is what
            // `totalSocketCount` reports.
            Object.defineProperty(httpAgent.prototype, 'totalSocketCount', {
                get: function() {
                    var self = this;
                    var count = 0;
                    Object.keys(self.sockets || {}).forEach(function(k) {
                        count += (self.sockets[k] || []).length;
                    });
                    Object.keys(self.freeSockets || {}).forEach(function(k) {
                        count += (self.freeSockets[k] || []).length;
                    });
                    return count;
                },
                configurable: true,
            });
            var httpGlobalAgent = new httpAgent();
            var HTTP_STATUS_CODES = {
                '100': "Continue",
                '101': "Switching Protocols",
                '102': "Processing",
                '103': "Early Hints",
                '200': "OK",
                '201': "Created",
                '202': "Accepted",
                '203': "Non-Authoritative Information",
                '204': "No Content",
                '205': "Reset Content",
                '206': "Partial Content",
                '207': "Multi-Status",
                '208': "Already Reported",
                '226': "IM Used",
                '300': "Multiple Choices",
                '301': "Moved Permanently",
                '302': "Found",
                '303': "See Other",
                '304': "Not Modified",
                '305': "Use Proxy",
                '307': "Temporary Redirect",
                '308': "Permanent Redirect",
                '400': "Bad Request",
                '401': "Unauthorized",
                '402': "Payment Required",
                '403': "Forbidden",
                '404': "Not Found",
                '405': "Method Not Allowed",
                '406': "Not Acceptable",
                '407': "Proxy Authentication Required",
                '408': "Request Timeout",
                '409': "Conflict",
                '410': "Gone",
                '411': "Length Required",
                '412': "Precondition Failed",
                '413': "Payload Too Large",
                '414': "URI Too Long",
                '415': "Unsupported Media Type",
                '416': "Range Not Satisfiable",
                '417': "Expectation Failed",
                '418': "I'm a Teapot",
                '421': "Misdirected Request",
                '422': "Unprocessable Entity",
                '423': "Locked",
                '424': "Failed Dependency",
                '425': "Too Early",
                '426': "Upgrade Required",
                '428': "Precondition Required",
                '429': "Too Many Requests",
                '431': "Request Header Fields Too Large",
                '451': "Unavailable For Legal Reasons",
                '500': "Internal Server Error",
                '501': "Not Implemented",
                '502': "Bad Gateway",
                '503': "Service Unavailable",
                '504': "Gateway Timeout",
                '505': "HTTP Version Not Supported",
                '506': "Variant Also Negotiates",
                '507': "Insufficient Storage",
                '508': "Loop Detected",
                '509': "Bandwidth Limit Exceeded",
                '510': "Not Extended",
                '511': "Network Authentication Required",
            };

            // ── IncomingMessage — the request body streams from the native
            // connection task (see http_server.rs): headers/method/url are set
            // synchronously when the head is dispatched, and the body is pulled
            // chunk by chunk from `__httpReadBody` only once something consumes
            // the stream. The producer keeps at most 1 MiB buffered ahead, so a
            // large upload never grows memory with its size.
            var httpIncomingMessage = function(socket) {
                // Node allows `http.IncomingMessage(socket)` without `new`.
                if (!(this instanceof httpIncomingMessage)) return new httpIncomingMessage(socket);
                Readable.call(this);
                this.socket = socket || {};
                this.connection = this.socket;
                this.httpVersion = '1.1';
                this.httpVersionMajor = 1;
                this.httpVersionMinor = 1;
                this.method = 'GET';
                this.url = '/';
                this.headers = {};
                this.rawHeaders = [];
                this.complete = false;
                this.aborted = false;
                this._headerLines = [];
            };
            util.inherits(httpIncomingMessage, Readable);
            // The body is delivered when something starts consuming the stream
            // ('data'/'readable'/'end' listener, resume(), read(), pipe(),
            // for-await), not right after the handler returns: middleware that
            // awaits before express.json() attaches its listeners late and used
            // to miss it. Chunks go through push() so setEncoding() is honored.
            // A consumed request reports `readable: false` / `readableEnded:
            // true` before `'end'` (on-finished reads these inside an 'end'
            // handler to tell a consumed request, e.g. the 400 for malformed
            // JSON), and `'end'` listeners added after the end still get it once.
            var kIncomingBody = typeof Symbol === 'function' ? Symbol('3va.incomingBody') : '__3vaIncomingBody';
            function finishIncoming(req, st) {
                if (st.done) return;
                st.done = true;
                req.complete = true;
                req.readable = false;
                req.readableEnded = true;
                req.push(null);
            }
            // The native body reader reports "the client went away" as a coded
            // error. Node turns that into IncomingMessage.destroy(ConnReset):
            // 'aborted' first, then 'error' only if the application listens
            // (the server's socket absorbs it otherwise) and 'close'.
            function abortIncomingRequest(req, err) {
                if (req.destroyed) return;
                var e = err;
                if (!e || e.code === 'ECONNRESET') e = __connReset('aborted');
                req.destroy(e);
            }
            function pumpIncoming(req, st) {
                var state = req._readableState;
                function schedule() {
                    if (st.pumpScheduled || st.done) return;
                    st.pumpScheduled = true;
                    queueMicrotask(function() { st.pumpScheduled = false; step(); });
                }
                function scheduleRetry() {
                    if (st.pumpScheduled || st.done) return;
                    st.pumpScheduled = true;
                    setTimeout(function() { st.pumpScheduled = false; step(); }, 1);
                }
                function step() {
                    if (st.done) return;
                    var hasConsumer = state.flowing
                        || (req._events && req._events['data'] && req._events['data'].length > 0)
                        || (req._events && req._events['readable'] && req._events['readable'].length > 0);
                    if (state.length >= state.highWaterMark) {
                        // Backpressure: wait for the consumer to drain. With no
                        // consumer at all (only an 'end' listener), discard so
                        // the body still reaches 'end' without unbounded memory.
                        if (hasConsumer) { st.waiting = true; return; }
                        state.buffer.length = 0;
                        state.length = 0;
                    }
                    st.waiting = false;
                    var chunk;
                    try {
                        chunk = __httpReadBody(st.bodyId, 65536);
                    } catch (e) {
                        // The native reader reported a body error (stall,
                        // RUDY, abort): the request is aborted, not completed.
                        abortIncomingRequest(req, e);
                        return;
                    }
                    if (chunk === null) { finishIncoming(req, st); return; }
                    if (chunk instanceof Error) {
                        abortIncomingRequest(req, chunk);
                        return;
                    }
                    if (chunk === undefined) { st.waiting = true; scheduleRetry(); return; }
                    req.push(typeof Buffer !== 'undefined' ? Buffer.from(chunk) : chunk);
                    if (state.length >= state.highWaterMark) { st.waiting = true; return; }
                    // Yield between chunks so an async consumer (for await,
                    // pipeline) applies backpressure instead of us racing ahead.
                    schedule();
                }
                // Resuming after the consumer drained the buffer: run on the
                // current turn unless a continuation is already queued.
                st.step = function() { if (!st.pumpScheduled) { st.waiting = false; step(); } };
                step();
            }
            function deliverIncomingBody(req) {
                var st = req[kIncomingBody];
                if (!st || st.started) return;
                st.started = true;
                if (!st.bodyId) { finishIncoming(req, st); return; }
                pumpIncoming(req, st);
            }
            httpIncomingMessage.prototype.on = httpIncomingMessage.prototype.addListener = function(ev, fn) {
                var st = this[kIncomingBody];
                if (st && ev === 'end' && st.done) { var self = this; queueMicrotask(function() { fn.call(self); }); return this; }
                var r = Readable.prototype.on.call(this, ev, fn);
                if (st && (ev === 'data' || ev === 'readable' || ev === 'end')) {
                    deliverIncomingBody(this);
                    // A consumer (e.g. `for await`) that drained the buffer and
                    // re-attached its listener must also restart a pump paused
                    // on backpressure — nothing else will.
                    if (st.waiting && st.step) st.step();
                }
                return r;
            };
            httpIncomingMessage.prototype.resume = function() {
                var st = this[kIncomingBody];
                deliverIncomingBody(this);
                Readable.prototype.resume.call(this);
                if (st && st.waiting && st.step) st.step();
                return this;
            };
            httpIncomingMessage.prototype.read = function(n) {
                var st = this[kIncomingBody];
                deliverIncomingBody(this);
                var r = Readable.prototype.read.call(this, n);
                if (st && st.waiting && st.step) st.step();
                return r;
            };
            // Node's `IncomingMessage.setTimeout` (lib/_http_incoming.js):
            // the callback listens on the message and the socket's inactivity
            // timer is re-armed.
            httpIncomingMessage.prototype.setTimeout = function(ms, cb) {
                if (typeof cb === 'function') this.on('timeout', cb);
                this.timeout = ms;
                if (this.socket && typeof this.socket.setTimeout === 'function') this.socket.setTimeout(ms);
                return this;
            };
            // Node's `IncomingMessage._destroy` (lib/_http_incoming.js): an
            // incomplete message is `aborted` and emits 'aborted' before
            // 'error'/'close'. On the server, the error only surfaces as an
            // 'error' event when someone listens for it (the socket absorbs
            // it otherwise), so `req.destroy(err)` does not crash the process
            // — test-http-server-incomingmessage-destroy relies on that.
            httpIncomingMessage.prototype.destroy = function(err) {
                if (this.destroyed) return this;
                this.destroyed = true;
                this.errored = err === undefined ? null : err;
                this.closed = false;
                if (!this.readableEnded || !this.complete) {
                    this.aborted = true;
                    this.emit('aborted');
                }
                var st = this[kIncomingBody];
                if (st) { st.done = true; this[kIncomingBody] = null; }
                if (this.socket && typeof this.socket.destroy === 'function' && !this.socket.destroyed) {
                    this.socket.destroy(err);
                }
                var self = this;
                var next = (typeof process !== 'undefined' && typeof process.nextTick === 'function')
                    ? process.nextTick : function(f) { queueMicrotask(f); };
                if (err) {
                    next(function() {
                        if (self.listenerCount('error') > 0) self.emit('error', err);
                    });
                }
                next(function() {
                    self.closed = true;
                    self.emit('close');
                });
                return this;
            };
            // Node's `_addHeaderLine` (lib/_http_incoming.js): the raw
            // `name: value` line a header arrived in. Node's IncomingMessage
            // has no `getHeader`/`setHeader`/... — those belong to
            // OutgoingMessage — so nothing else is added here.
            httpIncomingMessage.prototype._addHeaderLine = function(field, value, dest) {
                if (this._headerLines === undefined) this._headerLines = [];
                this._headerLines.push(field, value);
                if (dest && typeof dest === 'object') {
                    var lk = String(field).toLowerCase();
                    var current = dest[lk];
                    if (current === undefined) dest[lk] = value;
                    else if (Array.isArray(current)) current.push(value);
                    else dest[lk] = [current, value];
                }
                return this;
            };

            // ── ServerResponse — buffers writes and flushes as one __httpRespond(Bytes)
            // call on end(), since the native side writes headers+body together.
            var httpServerResponse = function(req) {
                // Node allows `http.ServerResponse(req)` without `new`.
                if (!(this instanceof httpServerResponse)) return new httpServerResponse(req);
                Writable.call(this);
                // Real usage: constructed internally by the accept loop with a
                // req whose `socket._connId` names the native connection to
                // write to. Node API compat also allows `new ServerResponse(req)`
                // standalone (e.g. for unit tests) — _connId stays undefined
                // there, fine as long as .end() is never called on it.
                this.req = req;
                this._connId = req && req.socket && req.socket._connId;
                this.socket = req && req.socket;
                this.connection = this.socket;
                this.statusCode = 200;
                this.statusMessage = '';
                this._headers = {};
                this._chunks = [];
                this.finished = false;
                this.headersSent = false;
                // Node sends a `Date` header unless the application turns it
                // off (`res.sendDate = false`).
                this.sendDate = true;
                // Set when the application removes the `Connection` header:
                // the server must not add one back.
                this._removedConnection = false;
            };
            util.inherits(httpServerResponse, Writable);
            // Header validation — same rules as Node's lib/_http_outgoing.js:
            // names must be HTTP tokens, values must not carry CR/LF or other
            // control bytes (HTTP response splitting / header injection).
            var __isValidHeaderToken = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;
            var __validateHeaderName = function(name) {
                if (typeof name !== 'string' || name.length === 0 || !__isValidHeaderToken.test(name)) {
                    var ne = new TypeError('Header name must be a valid HTTP token [' + JSON.stringify(String(name)) + ']');
                    ne.code = 'ERR_INVALID_HTTP_TOKEN';
                    throw ne;
                }
            };
            var __validateHeaderValue = function(name, value) {
                var s = String(value);
                for (var i = 0; i < s.length; i++) {
                    var c = s.charCodeAt(i);
                    // Reject CRLF and every other control byte except tab (0x09),
                    // plus DEL (0x7F). Mirrors Node's checkInvalidHeaderChar.
                    if ((c <= 0x1F && c !== 0x09) || c === 0x7F) {
                        var ve = new TypeError('Invalid character in header content [' + JSON.stringify(String(name).toLowerCase()) + ']');
                        ve.code = 'ERR_INVALID_CHAR';
                        throw ve;
                    }
                }
            };
            httpServerResponse.prototype.setHeader = function(name, value) {
                __validateHeaderName(name);
                __validateHeaderValue(name, value);
                this._headers[name] = value;
                return this;
            };
            httpServerResponse.prototype.getHeader = function(name) { return this._headers[name]; };
            httpServerResponse.prototype.removeHeader = function(name) {
                delete this._headers[name];
                // Node: removing the `Connection` header keeps the server
                // from adding one back (lib/_http_outgoing.js).
                if (String(name).toLowerCase() === 'connection') this._removedConnection = true;
                return this;
            };
            httpServerResponse.prototype.hasHeader = function(name) { return Object.prototype.hasOwnProperty.call(this._headers, name); };
            httpServerResponse.prototype.getHeaders = function() { return this._headers; };
            // Node's REASON_PHRASE (lib/internal/validators.js): a reason
            // phrase may carry tab, printable ASCII and bytes 0x80-0xff, and
            // nothing else. Anything with CR/LF (or another control byte)
            // would split the response in two.
            var __isValidReasonPhrase = /^[\t\x20-\x7e\x80-\xff]*$/;
            var __validateReasonPhrase = function(reason) {
                if (typeof reason === 'string' && reason.length > 0 && !__isValidReasonPhrase.test(reason)) {
                    var re = new TypeError('Invalid character in statusMessage');
                    re.code = 'ERR_INVALID_CHAR';
                    throw re;
                }
            };
            httpServerResponse.prototype.writeHead = function(statusCode, statusMessage, headers) {
                // Node's signature: the second argument is the reason phrase
                // only when it is a string; otherwise it is the headers
                // argument (`writeHead(200, {..})`, `writeHead(200, ['a','b'])`).
                if (typeof statusMessage !== 'string' && statusMessage !== undefined) {
                    headers = statusMessage;
                    statusMessage = undefined;
                }
                // Node records the status line before validating the headers:
                // when a header is rejected, statusCode and statusMessage are
                // already set and headersSent is still false.
                this.statusCode = statusCode;
                this.statusMessage = statusMessage === undefined
                    ? (HTTP_STATUS_CODES[String(statusCode)] || '')
                    : statusMessage;
                __validateReasonPhrase(this.statusMessage);
                // Node accepts three shapes for the headers: an object, a flat
                // array of alternating names and values, and an array of
                // [name, value] pairs.
                var pairs = null;
                if (Array.isArray(headers)) {
                    pairs = [];
                    if (headers.length && Array.isArray(headers[0])) {
                        for (var pi = 0; pi < headers.length; pi++) {
                            var p = headers[pi];
                            if (!Array.isArray(p) || p.length !== 2) {
                                var pe = new TypeError('Invalid header value: ' + String(p));
                                pe.code = 'ERR_INVALID_ARG_VALUE';
                                throw pe;
                            }
                            pairs.push([String(p[0]), p[1]]);
                        }
                    } else {
                        if (headers.length % 2 !== 0) {
                            var oe = new TypeError('Invalid header value: ' + JSON.stringify(headers));
                            oe.code = 'ERR_INVALID_ARG_VALUE';
                            throw oe;
                        }
                        for (var ai = 0; ai < headers.length; ai += 2) {
                            pairs.push([String(headers[ai]), headers[ai + 1]]);
                        }
                    }
                }
                // Validate the headers before recording any of them, so a
                // rejected header leaves the header list untouched.
                if (pairs) {
                    for (var q = 0; q < pairs.length; q++) {
                        __validateHeaderName(pairs[q][0]);
                        __validateHeaderValue(pairs[q][0], pairs[q][1]);
                    }
                }
                if (headers && !pairs) {
                    for (var k in headers) if (Object.prototype.hasOwnProperty.call(headers, k)) {
                        __validateHeaderName(k);
                        __validateHeaderValue(k, headers[k]);
                    }
                }
                if (pairs) {
                    for (var p2 = 0; p2 < pairs.length; p2++) {
                        var lk = pairs[p2][0].toLowerCase();
                        var value = pairs[p2][1];
                        if (this._headers[lk] === undefined) this._headers[lk] = value;
                        else if (Array.isArray(this._headers[lk])) this._headers[lk].push(value);
                        else this._headers[lk] = [this._headers[lk], value];
                    }
                }
                if (headers && !pairs) {
                    for (var k2 in headers) if (Object.prototype.hasOwnProperty.call(headers, k2)) this._headers[k2] = headers[k2];
                }
                this.headersSent = true;
                this._header = true;
                return this;
            };
            // Node compat: compression middleware (next/dist/compiled/compression)
            // calls `this._implicitHeader()` when writing a response without an
            // explicit writeHead. Node's ServerResponse implementation is just
            // `this.writeHead(this.statusCode)` — the actual bytes are only flushed
            // at end(), so recording the head here is sufficient.
            httpServerResponse.prototype._implicitHeader = function() { this.writeHead(this.statusCode); };
            // Bytes of a chunk, for the writable high-water mark below. A
            // string's byte length is what counts, not its character length.
            var __httpBytes = function(c) {
                if (typeof c === 'string') return typeof Buffer !== 'undefined' ? Buffer.byteLength(c) : c.length;
                return (c && typeof c.length === 'number') ? c.length : 0;
            };
            httpServerResponse.prototype.write = function(chunk, encoding, callback) {
                if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
                if (this.finished) {
                    var err = new Error('write after end');
                    err.code = 'ERR_STREAM_WRITE_AFTER_END';
                    if (typeof callback === 'function') queueMicrotask(function() { callback(err); });
                    this.emit('error', err);
                    return false;
                }
                if (chunk !== undefined && chunk !== null) {
                    this._chunks.push(chunk);
                    this._buffered = (this._buffered || 0) + __httpBytes(chunk);
                    if (typeof callback === 'function') {
                        (this._writeCallbacks || (this._writeCallbacks = [])).push(callback);
                    }
                } else if (typeof callback === 'function') {
                    queueMicrotask(callback);
                }
                // Writable's high-water mark (16 KiB by default). The response
                // is flushed as one write at end(), so everything written so
                // far is still pending here: `while (res.write(buf))` has to
                // see the false once the mark is passed, or the loop never
                // stops (test-http-outgoing-finish.js ran out of memory).
                return (this._buffered || 0) < (this.writableHighWaterMark || 16384);
            };
            httpServerResponse.prototype.end = function(chunk, encoding, callback) {
                var self = this;
                if (typeof chunk === 'function') { callback = chunk; chunk = undefined; }
                if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
                if (chunk !== undefined) this._chunks.push(chunk);
                if (this.finished) { if (callback) setTimeout(callback, 0); return this; }
                this.finished = true;

                var headersJson = JSON.stringify(this._headers);
                // Node sends the standard reason phrase when none was set.
                if (!this.statusMessage) this.statusMessage = HTTP_STATUS_CODES[String(this.statusCode)] || '';
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
                    __httpRespondBytes(this._connId, this.statusCode, this.statusMessage, headersJson, merged, this.sendDate !== false, !this._removedConnection, this._closeAfter === true);
                } else {
                    // Route through __httpRespondBytes (JS-side TextEncoder,
                    // not the native __httpRespond(string) path) even for a
                    // plain string body. __httpRespond converts the V8
                    // string to a Rust String via to_rust_string_lossy(),
                    // which for a large multi-megabyte string containing
                    // multi-byte UTF-8 sequences (e.g. Metro serving a JS
                    // bundle as a single ~4M-character string) was silently
                    // corrupting individual characters — not consistently,
                    // and not in a way curl-from-localhost's own single-shot
                    // fetch always reproduced, which is what made it look
                    // connection/cache-dependent. TextEncoder is a
                    // spec'd, well-exercised primitive with no such issue at
                    // this scale, so encoding here in JS and always calling
                    // the byte-safe responder sidesteps the native
                    // conversion entirely rather than depending on finding
                    // the exact defect in it.
                    var __finalS = this._chunks.join('');
                    if (typeof Buffer !== 'undefined') {
                        __httpRespondBytes(this._connId, this.statusCode, this.statusMessage, headersJson, Buffer.from(__finalS, 'utf-8'), this.sendDate !== false, !this._removedConnection, this._closeAfter === true);
                    } else {
                        __httpRespondBytes(this._connId, this.statusCode, this.statusMessage, headersJson, new TextEncoder().encode(__finalS), this.sendDate !== false, !this._removedConnection, this._closeAfter === true);
                    }
                }
                // A microtask, not a 0 ms timer: it still runs after the handler
                // returns, without a timer round-trip per response.
                queueMicrotask(function() {
                    // Node calls the write callbacks before 'finish', and
                    // 'drain' once a backpressured response has been flushed.
                    var writeCbs = self._writeCallbacks;
                    if (writeCbs) {
                        self._writeCallbacks = null;
                        for (var i = 0; i < writeCbs.length; i++) writeCbs[i]();
                    }
                    // `_buffered` is 0 for the usual "write nothing, just
                    // end(body)" response: skip the writableHighWaterMark
                    // getter there, this runs once per response.
                    var buffered = self._buffered || 0;
                    if (buffered) {
                        self._buffered = 0;
                        if (buffered >= (self.writableHighWaterMark || 16384)) self.emit('drain');
                    }
                    self.emit('finish');
                    self.emit('close');
                    httpConnResponseDone(self);
                    if (typeof callback === 'function') callback();
                });
                return this;
            };
            // Node's `OutgoingMessage.setTimeout` (lib/_http_outgoing.js):
            // the callback listens on the response and the socket's inactivity
            // timer is re-armed, so an unanswered response can time out.
            httpServerResponse.prototype.setTimeout = function(ms, cb) {
                if (typeof cb === 'function') this.on('timeout', cb);
                this.timeout = ms;
                if (this.socket && typeof this.socket.setTimeout === 'function') this.socket.setTimeout(ms);
                return this;
            };
            httpServerResponse.prototype.destroy = function(err) {
                if (this.socket && typeof this.socket.destroy === 'function') this.socket.destroy(err);
                return this;
            };
            // ponytail: no-op by design — the head and the body are written
            // together in end(); see docs/05-js-engine/07-inventario-stubs.md §7.3.
            httpServerResponse.prototype.flushHeaders = function() {};
            // Node's ServerResponse has these (lib/_http_server.js): the
            // response can be handed a socket it did not come from
            // (`assignSocket`), give it back (`detachSocket`), and can carry
            // trailers. Without them, code that moves a response between
            // sockets (a proxy, a middleware that rewrites the socket)
            // crashes with "not a function".
            httpServerResponse.prototype.assignSocket = function(socket) {
                this.socket = socket;
                this.connection = socket;
                this._connId = socket && socket._connId;
                if (socket && typeof socket.on === 'function') {
                    var self = this;
                    // The listener is kept so `detachSocket` can take it back
                    // out (it is the only one this response added).
                    this._onSocketClose = function() { self.emit('close'); };
                    socket.on('close', this._onSocketClose);
                }
                return this;
            };
            httpServerResponse.prototype.detachSocket = function(socket) {
                if (socket && typeof socket.removeListener === 'function' && this._onSocketClose) {
                    socket.removeListener('close', this._onSocketClose);
                }
                this._onSocketClose = null;
                if (this.socket === socket) { this.socket = null; this.connection = null; }
                return this;
            };
            // ponytail: no-op by design — the response is not chunked, so it
            // has no trailer section; see docs/05-js-engine/07-inventario-stubs.md §7.3.
            httpServerResponse.prototype.addTrailers = function(headers) {
                if (!this._trailers) this._trailers = {};
                var self = this;
                Object.keys(headers || {}).forEach(function(k) { self._trailers[k] = headers[k]; });
                return this;
            };
            httpServerResponse.prototype.getHeaderNames = function() {
                return Object.keys(this._headers || {});
            };
            // The 1xx informational responses Node can send before the final
            // one: `writeEarlyHints` (103), `writeProcessing` (102) and
            // `writeInformation` (any 1xx). They are written to the socket
            // before the response, as in Node.
            httpServerResponse.prototype.writeEarlyHints = function(hints, cb) {
                if (typeof hints === 'function') { cb = hints; hints = {}; }
                this._writeInformational(103, 'Early Hints', hints || {});
                if (typeof cb === 'function') setTimeout(cb, 0);
                return this;
            };
            httpServerResponse.prototype.writeProcessing = function() {
                this._writeInformational(102, 'Processing', {});
                return this;
            };
            httpServerResponse.prototype.writeInformation = function(statusCode, headers, cb) {
                if (typeof headers === 'function') { cb = headers; headers = {}; }
                var code = parseInt(statusCode, 10) || 100;
                this._writeInformational(code, HTTP_STATUS_CODES[String(code)] || '', headers || {});
                if (typeof cb === 'function') setTimeout(cb, 0);
                return this;
            };
            httpServerResponse.prototype._writeInformational = function(statusCode, statusText, headers) {
                if (this._connId === undefined || this._connId === null) return;
                if (typeof __httpRespondInformational !== 'function') return;
                __httpRespondInformational(this._connId, statusCode, statusText, JSON.stringify(headers || {}));
            };

            // Per-connection socket, as seen by the request/response objects.
            // Node hands the application the real net.Socket, whose timeout
            // is an inactivity timer: when it fires, 'timeout' goes to the
            // current request (if it has not completed), the current
            // response, and the server; the socket is only destroyed when
            // nobody listened (lib/_http_server.js: socketOnTimeout). The
            // TCP connection itself lives in the native connection task, so
            // `destroy()` asks it to close through __httpCloseConn.
            var httpConnSocket = function(connId, remote, server) {
                EventEmitter.call(this);
                this._connId = connId;
                this.remoteAddress = remote;
                this.remotePort = undefined;
                // `on-finished` (body-parser, express.json()) treats a request
                // whose socket is not `readable` as already finished and skips
                // parsing the body: req.body stayed undefined. A live
                // connection is readable/writable.
                this.readable = true;
                this.writable = true;
                this.destroyed = false;
                this._server = server;
                this._req = null;
                this._resps = [];
                this._timer = null;
                this._timeoutMs = 0;
            };
            util.inherits(httpConnSocket, EventEmitter);
            // ponytail: like UpgradeSocket (docs/05-js-engine/07-inventario-stubs.md
            // §7.2), the local side of an accepted HTTP connection is not tracked, so
            // report the peer we do know.
            httpConnSocket.prototype.address = function() {
                return { address: this.remoteAddress || '0.0.0.0', port: this.remotePort || 0, family: this.remoteAddress && this.remoteAddress.indexOf(':') !== -1 ? 'IPv6' : 'IPv4' };
            };
            httpConnSocket.prototype._arm = function(ms) {
                var self = this;
                if (this._timer) { clearTimeout(this._timer); this._timer = null; }
                if (!ms || this.destroyed) return this;
                this._timer = setTimeout(function() {
                    self._timer = null;
                    if (self.destroyed) return;
                    var req = self._req, res = null, i;
                    if (req && req.complete) req = null;
                    for (i = 0; i < self._resps.length; i++) {
                        if (!self._resps[i].finished) { res = self._resps[i]; break; }
                    }
                    var handled = false;
                    if (req) handled = req.emit('timeout', self) || handled;
                    if (res) handled = res.emit('timeout', self) || handled;
                    handled = self._server.emit('timeout', self) || handled;
                    if (!handled) self.destroy();
                }, ms);
                return this;
            };
            // `socket.setTimeout(msecs, cb)`: re-arms the inactivity timer.
            // A request or response passes its own listener; the event is
            // emitted by `_arm` on whichever of them is still incomplete.
            httpConnSocket.prototype.setTimeout = function(ms, cb) {
                if (typeof cb === 'function') this.once('timeout', cb);
                this._timeoutMs = ms | 0;
                return this._arm(this._timeoutMs);
            };
            // ponytail: no-op by design — the TCP options belong to the native
            // connection task, as with UpgradeSocket (docs/05-js-engine/07-inventario-stubs.md §7.2).
            httpConnSocket.prototype.setNoDelay = function() { return this; };
            httpConnSocket.prototype.setKeepAlive = function() { return this; };
            httpConnSocket.prototype.ref = function() { return this; };
            httpConnSocket.prototype.unref = function() { return this; };
            httpConnSocket.prototype.destroy = function(err) {
                if (this.destroyed) return this;
                this.destroyed = true;
                if (this._timer) { clearTimeout(this._timer); this._timer = null; }
                if (this._server && this._server._conns[this._connId] === this) {
                    delete this._server._conns[this._connId];
                }
                if (typeof __httpCloseConn === 'function') __httpCloseConn(this._connId);
                this.emit('close');
                // Node's HTTP server owns its sockets: an error destroying one
                // is absorbed by the server (socketOnError), not thrown at the
                // application. Only an explicit listener sees it.
                if (err && this.listenerCount('error') > 0) this.emit('error', err);
                return this;
            };
            // A response that finished frees its slot; if the connection
            // stays open, the socket's inactivity timeout becomes the
            // server's keep-alive timeout, as `resetSocketTimeout` does in
            // Node (lib/_http_server.js).
            var httpConnResponseDone = function(res) {
                var sock = res.socket;
                if (!sock || !sock._resps) return;
                var i = sock._resps.indexOf(res);
                if (i !== -1) sock._resps.splice(i, 1);
                var server = sock._server;
                // `Connection: close`: the native task closes the connection
                // right after this response, so drop the inactivity timer with
                // it (a spurious 'timeout' would otherwise fire later).
                if (res._closeAfter) {
                    sock._timeoutMs = 0;
                    sock._arm(0);
                    return;
                }
                // The native keep-alive timeout already closes an idle
                // connection; the JS timer is only needed when the
                // application listens for 'timeout' (otherwise this would
                // allocate a timer on every response, on the hot path).
                if (server && server.keepAliveTimeout && server.listenerCount('timeout') > 0) {
                    var ms = server.keepAliveTimeout + (server.keepAliveTimeoutBuffer || 0);
                    sock._timeoutMs = ms;
                    sock._arm(ms);
                }
            };

            // ── Server — real listener backed by __httpListen/__httpAcceptPoll.
            var httpServer = function(opts, requestListener) {
                // Node allows `http.Server(fn)` without `new`.
                if (!(this instanceof httpServer)) return new httpServer(opts, requestListener);
                EventEmitter.call(this);
                if (typeof opts === 'function') { requestListener = opts; opts = {}; }
                opts = opts || {};
                if (requestListener) this.on('request', requestListener);
                this._id = null;
                this._pollTimer = null;
                this.listening = false;
                this._port = 0;
                this._host = '0.0.0.0';
                // Node's `server.timeout` (socket inactivity timeout, 0 = off)
                // and the live connections, keyed by native connection id, so
                // `server.setTimeout`, `req.setTimeout` and `res.setTimeout`
                // have a socket to fire on and `close()` can drop idle ones.
                this.timeout = 0;
                this._conns = {};
                // Node's server timeouts (lib/_http_server.js). `requestTimeout`
                // is the deadline for the whole request, `headersTimeout` for
                // its head, and `keepAliveTimeout` for an idle keep-alive
                // connection.
                var V = globalThis.__validators;
                // The limits 3va really enforces: the firewall's slowloris
                // protection. The application can tighten them, never loosen
                // them, and it is told when its value is looser than the
                // limit in force (the INFO-F guard of the security audit).
                var limits = (typeof __httpServerLimits === 'function') ? __httpServerLimits() : null;
                var timeout = function(name, value) {
                    if (typeof value !== 'number') throw V.invalidArgType(name, 'of type number', value);
                    if (!(value >= 0) || value > Number.MAX_SAFE_INTEGER) {
                        throw V.outOfRange(name, value, 0, Number.MAX_SAFE_INTEGER);
                    }
                    return value;
                };
                var requestTimeout = opts.requestTimeout !== undefined
                    ? timeout('requestTimeout', opts.requestTimeout) : 300000;
                var headersTimeout = opts.headersTimeout !== undefined
                    ? timeout('headersTimeout', opts.headersTimeout)
                    : Math.min(requestTimeout, 60000);
                if (requestTimeout > 0 && headersTimeout > requestTimeout) {
                    var e = new RangeError('The value of "headersTimeout" is out of range. ' +
                        'It must be <= requestTimeout. Received ' + headersTimeout);
                    e.code = 'ERR_OUT_OF_RANGE';
                    throw e;
                }
                var keepAliveTimeout = opts.keepAliveTimeout !== undefined
                    ? timeout('keepAliveTimeout', opts.keepAliveTimeout) : 5000;
                // Node keeps the socket `keepAliveTimeoutBuffer` ms longer than
                // the advertised `keepAliveTimeout` (default 1 s), so a client
                // that reuses the connection right at the deadline is not reset.
                this.keepAliveTimeoutBuffer = opts.keepAliveTimeoutBuffer !== undefined
                    ? timeout('keepAliveTimeoutBuffer', opts.keepAliveTimeoutBuffer) : 1000;
                var self = this;
                // What Node reports.
                var reported = {
                    headersTimeout: headersTimeout,
                    requestTimeout: requestTimeout,
                    keepAliveTimeout: keepAliveTimeout,
                };
                // Only the values the application set reach the native side:
                // Node's defaults are not 3va's limits, and the firewall's
                // own (stricter) limits must not be replaced by them.
                var applied = {
                    headersTimeout: opts.headersTimeout !== undefined ? headersTimeout : undefined,
                    requestTimeout: opts.requestTimeout !== undefined ? requestTimeout : undefined,
                    keepAliveTimeout: opts.keepAliveTimeout !== undefined ? keepAliveTimeout : undefined,
                };
                var warnLooser = function(name, value) {
                    var limit = limits && limits[name];
                    if (!limit) return;
                    if (value > limit || value === 0) {
                        console.warn('3va: http.Server.' + name + ' is ' + value + ' ms, but the built-in ' +
                            'firewall limits it to ' + limit + ' ms; the effective limit is ' + limit + ' ms.');
                    }
                };
                var pushTimeouts = function() {
                    if (self._id === null) return;
                    if (typeof __httpSetTimeouts === 'function') {
                        __httpSetTimeouts(self._id, applied.headersTimeout || 0,
                            applied.requestTimeout || 0, applied.keepAliveTimeout || 0);
                    }
                };
                ['headersTimeout', 'requestTimeout', 'keepAliveTimeout'].forEach(function(name) {
                    Object.defineProperty(self, name, {
                        get: function() { return reported[name]; },
                        set: function(v) {
                            reported[name] = timeout(name, v);
                            applied[name] = reported[name];
                            warnLooser(name, reported[name]);
                            pushTimeouts();
                        },
                        enumerable: true,
                        configurable: true,
                    });
                });
                if (opts.headersTimeout !== undefined) warnLooser('headersTimeout', headersTimeout);
                if (opts.keepAliveTimeout !== undefined) warnLooser('keepAliveTimeout', keepAliveTimeout);
                this._pushTimeouts = pushTimeouts;
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

                var result;
                if (self._tlsMissing) {
                    // An https server with no key/cert must never fall back
                    // to serving plaintext on its port.
                    result = new TypeError('https server: `key` and `cert` are required to listen (refusing to serve plaintext)');
                    result.code = 'ERR_MISSING_ARGS';
                } else {
                    result = self._tls
                        ? __httpListen(port, hostname, self._tls.cert, self._tls.key)
                        : __httpListen(port, hostname);
                }
                if (typeof result !== 'number') {
                    // Node semantics: with no 'error' listener a failed listen
                    // (e.g. EACCES without --allow-net) is an uncaught exception,
                    // not a silent exit 0.
                    setTimeout(function() {
                        if (self.listenerCount('error') > 0) self.emit('error', result);
                        else throw result;
                    }, 0);
                    return self;
                }
                self._id = result;
                self._port = __httpServerPort(result);
                self._host = __httpServerAddress(result) || hostname;
                self.listening = true;
                // The timeouts the application configured reach the
                // connection tasks now that the server has an id.
                if (typeof self._pushTimeouts === 'function') self._pushTimeouts();
                // Called by the event loop (http_server.rs dispatch_ready) as
                // soon as a connection task queues a parsed request — no
                // polling interval, so no latency floor between arrivals.
__httpSetHandler(self._id, function() {
                    try {
                    // At most 64 per call: the event loop runs microtasks
                    // between batches, so a burst doesn't keep hundreds of
                    // requests' objects alive at once (they'd be promoted to
                    // the old generation). The rest are dispatched next turn.
                    var raw, _n = 0;
                    while (_n++ < 64 && (raw = __httpAcceptPoll(self._id)) !== null && raw !== undefined) {
                        // raw = [method, url, connId, remoteAddress, body, flatHeaders]
                        var _sock = self._conns[raw[2]];
                        if (!_sock) {
                            _sock = new httpConnSocket(raw[2], raw[3], self);
                            self._conns[raw[2]] = _sock;
                            if (self._tls) _sock.encrypted = true;
                            // Node: one 'connection' per TCP connection, before
                            // its first 'request'.
                            self.emit('connection', _sock);
                        } else if (raw[3]) {
                            _sock.remoteAddress = raw[3];
                        }
                        var req = new httpIncomingMessage(_sock);
                        req.method = raw[0];
                        req.url = raw[1];
                        // The HTTP version of the request: an HTTP/1.0
                        // request is reported as such, as in Node.
                        // The version comes as a number (10 or 11) so the
                        // hot path does not build a string per request.
                        req.httpVersion = raw[6] === 10 ? '1.0' : '1.1';
                        req.httpVersionMajor = 1;
                        req.httpVersionMinor = raw[6] === 10 ? 0 : 1;
                        // Last duplicate wins, as the JSON object this replaced did.
                        var _flat = raw[5], _rh = {};
                        for (var _i = 0; _i < _flat.length; _i += 2) _rh[_flat[_i]] = _flat[_i + 1];
                        req.headers = _rh;
                        var _rk = Object.keys(_rh), _raw = new Array(_rk.length * 2);
                        for (var _j = 0; _j < _rk.length; _j++) { _raw[2 * _j] = _rk[_j]; _raw[2 * _j + 1] = _rh[_rk[_j]]; }
                        req.rawHeaders = _raw;
                        // Body state lives under a Symbol, never `req._body`: body-parser 1.x
                        // (Express 4) uses `req._body` as its "already parsed" flag. Set before
                        // the 'request' event so the handler's own listeners see it. The body
                        // streams from the native pipe named by `raw[4]` (0 = no body).
                        req[kIncomingBody] = { bodyId: raw[4] | 0, started: false, done: false, waiting: false };
                        var res = new httpServerResponse(req);
                        res._closeAfter = raw[7] === true;
                        _sock._req = req;
                        _sock._resps.push(res);
                        // Node's `resetSocketTimeout`: a new request on the
                        // connection restores the server's socket timeout.
                        _sock._timeoutMs = self.timeout || 0;
                        _sock._arm(_sock._timeoutMs);
                        self.emit('request', req, res);
                        // The raw body stays in this closure, never on `req`: body-parser 1.x
                        // (Express 4) uses `req._body` as its "already parsed" flag, so a
                        // property of that name made express.json() skip every request.
                    }
                    // HTTP `Upgrade` (WebSocket): hand the raw duplex socket
                    // to the `'upgrade'` listener, exactly like Node — the
                    // `ws` library performs the 101 handshake and drives the
                    // frames itself. Upgrades and requests share this handler
                    // call so one event-loop wake serves both.
                    var _u, _un = 0;
                    while (_un++ < 64 && (_u = __httpUpgradePoll(self._id)) !== null && _u !== undefined) {
                        // _u = [connId, remoteAddress, head(Uint8Array), method, url, flatHeaders]
                        var _usock = new UpgradeSocket({ fd: _u[0] });
                        _usock.remoteAddress = _u[1];
                        if (self._tls) _usock.encrypted = true;
                        var _ureq = new httpIncomingMessage(_usock);
                        _ureq.method = _u[3];
                        _ureq.url = _u[4];
                        var _uflat = _u[5], _urh = {};
                        for (var _ui = 0; _ui < _uflat.length; _ui += 2) _urh[_uflat[_ui]] = _uflat[_ui + 1];
                        _ureq.headers = _urh;
                        var _rk2 = Object.keys(_urh), _ura = new Array(_rk2.length * 2);
                        for (var _uj = 0; _uj < _rk2.length; _uj++) { _ura[2 * _uj] = _rk2[_uj]; _ura[2 * _uj + 1] = _urh[_rk2[_uj]]; }
                        _ureq.rawHeaders = _ura;
                        var _uhead = _u[2] instanceof Uint8Array ? _u[2] : new Uint8Array(_u[2] || 0);
                        if (self.listenerCount('upgrade') > 0) {
                            self.emit('upgrade', _ureq, _usock, _uhead);
                        } else {
                            // No upgrade listener: close the raw socket so it
                            // can't sit open holding a firewall slot.
                            _usock.destroy();
                        }
                    }
                    } catch(e) { console.error('[3va-http] request handler threw:', e && (e.stack || e.message || e)); }
                });
                setTimeout(function() { self.emit('listening'); }, 0);
                return self;
            };
            httpServer.prototype.close = function(callback) {
                var self = this;
                if (this._pollTimer) { clearInterval(this._pollTimer); this._pollTimer = null; }
                if (this._id !== null) { __httpClose(this._id); }
                this.listening = false;
                // Node 19+: closing the server drops its idle keep-alive
                // connections; a connection with a response in flight is left
                // to finish (the native task closes it then).
                var ids = Object.keys(this._conns);
                for (var i = 0; i < ids.length; i++) {
                    var c = this._conns[ids[i]];
                    if (c._resps.length === 0) c.destroy();
                }
                if (typeof callback === 'function') this.once('close', callback);
                setTimeout(function() { self.emit('close'); }, 0);
                return this;
            };
            httpServer.prototype.address = function() {
                if (!this.listening) return null;
                return { port: this._port, address: this._host, family: this._host.indexOf(':') !== -1 ? 'IPv6' : 'IPv4' };
            };
            httpServer.prototype.getConnections = function(cb) { cb(null, 0); };
            httpServer.prototype.setTimeout = function(ms, cb) {
                // Node (lib/_http_server.js): remember the value for every new
                // connection and register the callback on the server; the
                // per-socket timer is what actually fires.
                this.timeout = ms;
                if (typeof cb === 'function') this.on('timeout', cb);
                return this;
            };
            httpServer.prototype.keepAliveTimeout = 5000;
            // The native server takes its limits from the firewall config
            // (header timeout 10 s by default), not from these properties.
            // Report the real value and say so when code sets one, instead
            // of silently accepting e.g. a slowloris `headersTimeout` (INFO-F).
            [['maxHeadersCount', null], ['maxConnections', 0]].forEach(function(p) {
                var warned = false;
                Object.defineProperty(httpServer.prototype, p[0], {
                    get: function() { return p[1]; },
                    set: function() {
                        if (!warned) {
                            warned = true;
                            console.warn('3va: http.Server.' + p[0] + ' has no effect; ' +
                                'connection limits come from the built-in firewall config.');
                        }
                    },
                    configurable: true,
                });
            });
            // ponytail: ref/unref/timeout are no-ops — see docs/05-js-engine/07-inventario-stubs.md §7.2-7.3.
            httpServer.prototype.ref = function() { return this; };
            httpServer.prototype.unref = function() { return this; };
            httpServer.prototype.listenOnServerHandler = function(socket) {};

            var httpOutgoingMessage = function() {
                // Node allows `http.OutgoingMessage()` without `new`.
                if (!(this instanceof httpOutgoingMessage)) return new httpOutgoingMessage();
                EventEmitter.call(this);
            };
            util.inherits(httpOutgoingMessage, EventEmitter);
            httpOutgoingMessage.prototype.write = function(chunk, encoding, callback) { return true; };
            httpOutgoingMessage.prototype.end = function(chunk, encoding, callback) {};
            httpOutgoingMessage.prototype.destroy = function(err) {};
            httpOutgoingMessage.prototype.setTimeout = function(msecs, callback) { return this; };

            // ── Client-side IncomingMessage ──────────────────────────────────
            // A real Readable: status line, headers and the de-framed body
            // (Content-Length, chunked, or read-to-EOF). The server's
            // IncomingMessage is tied to the native request-body reader; a
            // client response is parsed from the socket here.
            function ClientIncomingMessage(socket) {
                Readable.call(this);
                this.socket = socket;
                this.connection = socket;
                this.httpVersion = '1.1';
                this.httpVersionMajor = 1;
                this.httpVersionMinor = 1;
                this.statusCode = 0;
                this.statusMessage = '';
                this.headers = {};
                this.rawHeaders = [];
                this.complete = false;
                this.aborted = false;
                // The Readable destroy state Node exposes on a message.
                this.destroyed = false;
                this.errored = null;
                this.closed = false;
                this.req = null;
            }
            util.inherits(ClientIncomingMessage, Readable);
            ClientIncomingMessage.prototype.setTimeout = function(ms, cb) { if (typeof cb === 'function') this.once('timeout', cb); return this; };
            ClientIncomingMessage.prototype.destroy = function(err) {
                if (this.destroyed) return this;
                this.destroyed = true;
                this.errored = err === undefined ? null : err;
                this.closed = false;
                // Node's IncomingMessage._destroy: what is not fully read is
                // an abort, and 'aborted' goes out before 'error'/'close'.
                if (!this.readableEnded || !this.complete) {
                    this.aborted = true;
                    this.emit('aborted');
                }
                if (this.aborted && this.socket && !this.socket.destroyed
                    && typeof this.socket.destroy === 'function') {
                    // A live socket carries the error to the request (as in
                    // Node), which is where an unhandled one reaches
                    // `uncaughtException`.
                    this.socket.destroy(err);
                }
                var self = this;
                var next = (typeof process !== 'undefined' && typeof process.nextTick === 'function')
                    ? process.nextTick : function(f) { queueMicrotask(f); };
                if (err) {
                    next(function() {
                        if (self.listenerCount('error') > 0) self.emit('error', err);
                    });
                }
                next(function() {
                    self.closed = true;
                    self.emit('close');
                });
                return this;
            };

            // Parse one response head (status line + headers). Header names are
            // lower-cased; duplicates are joined with ", " except set-cookie,
            // which stays an array, as in Node.
            //
            // The headers Node calls "no-repeat" keep their first value: a
            // repeated `content-length` or `location` is not a list, and the
            // first one is the one the client acts on.
            var __NO_REPEAT_HEADERS = {
                'age': 1, 'authorization': 1, 'content-length': 1, 'content-type': 1,
                'etag': 1, 'expires': 1, 'from': 1, 'host': 1, 'if-modified-since': 1,
                'if-unmodified-since': 1, 'last-modified': 1, 'location': 1,
                'max-forwards': 1, 'proxy-authorization': 1, 'referer': 1,
                'retry-after': 1, 'server': 1, 'user-agent': 1,
            };
            function __parseResponseHead(head) {
                var lines = head.split('\r\n');
                var m = /^HTTP\/(\d+)\.(\d+)\s+(\d{3})\s?(.*)$/.exec(lines[0] || '');
                var out = { major: 1, minor: 1, statusCode: 0, statusMessage: '', headers: {}, rawHeaders: [] };
                if (m) {
                    out.major = parseInt(m[1], 10);
                    out.minor = parseInt(m[2], 10);
                    out.statusCode = parseInt(m[3], 10);
                    out.statusMessage = m[4] || '';
                }
                var lastName = null;
                for (var i = 1; i < lines.length; i++) {
                    var line = lines[i];
                    if (line === '') continue;
                    if (line.charCodeAt(0) === 9 || line.charCodeAt(0) === 32) {
                        if (lastName) out.headers[lastName] += ' ' + line.trim();
                        continue;
                    }
                    var ci = line.indexOf(':');
                    if (ci <= 0) continue;
                    var name = line.slice(0, ci).trim();
                    var value = line.slice(ci + 1).trim();
                    var lk = name.toLowerCase();
                    out.rawHeaders.push(name, value);
                    lastName = lk;
                    // Node keeps `set-cookie` in an array even when there is
                    // only one: the header can be repeated and a string would
                    // lose the difference between "one cookie" and "several".
                    if (lk === 'set-cookie') {
                        if (out.headers[lk] === undefined) out.headers[lk] = [value];
                        else out.headers[lk] = [].concat(out.headers[lk], value);
                    } else if (out.headers[lk] === undefined) {
                        out.headers[lk] = value;
                    } else if (__NO_REPEAT_HEADERS[lk]) {
                        // The first value wins for the headers Node does not
                        // repeat.
                    } else {
                        out.headers[lk] = out.headers[lk] + ', ' + value;
                    }
                }
                return out;
            }

            function __reqChunkToBuffer(chunk, encoding) {
                if (Buffer.isBuffer(chunk)) return chunk;
                if (chunk instanceof Uint8Array) return Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength);
                if (typeof chunk === 'string') return Buffer.from(chunk, encoding || 'utf8');
                return Buffer.from(String(chunk), encoding || 'utf8');
            }

            // Node's ConnResetException (lib/internal/errors.js): an
            // ECONNRESET whose message says what happened ('aborted' when the
            // peer went away mid-response, 'socket hang up' when no response
            // ever arrived).
            function __connReset(message) {
                var e = new Error(message);
                e.code = 'ECONNRESET';
                return e;
            }

            // ClientRequest — a real HTTP client request: string and binary
            // bodies, several write() + end(), streaming (pipe), chunked vs
            // Content-Length framing, a de-framing response parser, half-close,
            // timeouts, abort and keep-alive socket reuse.
            // defaultProtocol is 'https:' when called through the https module:
            // those requests must go over TLS, never a plain TCP socket.
            function ClientRequest(options, callback, defaultProtocol) {
                // Node allows `http.ClientRequest(options)` without `new`.
                if (!(this instanceof ClientRequest)) return new ClientRequest(options, callback, defaultProtocol);
                EventEmitter.call(this);
                if (typeof options === 'string') {
                    var u = new URL(options);
                    options = { protocol: u.protocol, hostname: u.hostname, port: parseInt(u.port) || (u.protocol === 'https:' ? 443 : 80), path: u.pathname + u.search };
                }
                options = options || {};
                this._secure = (options.protocol || defaultProtocol || 'http:') === 'https:';
                this._method = (options.method || 'GET').toUpperCase();
                this._path = options.path || '/';
                this._host = options.hostname || options.host || 'localhost';
                this._port = parseInt(options.port) || (this._secure ? 443 : 80);
                this.method = this._method;
                this.path = this._path;
                this.host = this._host;
                this._headers = {};
                this._headerNames = {};
                this._headerSent = false;
                this._socket = null;
                this._connected = false;
                this._ended = false;
                this._aborted = false;
                this._body = [];
                this._bodyLen = 0;
                this._encodeChunked = false;
                this._chunkTerminated = false;
                this._response = null;
                this._responseDone = false;
                this._heardResponse = false;
                // The request state Node exposes to applications (and what
                // `test-http-destroyed-socket-write2` checks after a reset).
                this.res = null;
                this.outputData = [];
                this.aborted = false;
                this.closed = false;
                this.errored = null;
                this._hadError = false;
                this._omitHost = options.setHost === false;
                // The TLS options of the request (Node's `https.request`
                // options). They reach the connection factory, so the
                // default connection and a proxy agent both honour them.
                this._ca = options.ca;
                this._cert = options.cert;
                this._key = options.key;
                this._passphrase = options.passphrase;
                this._servername = options.servername;
                this._rejectUnauthorized = options.rejectUnauthorized !== false;
                // The OutgoingMessage state Node exposes on a request.
                this.destroyed = false;
                this.writableEnded = false;
                this.writableFinished = false;
                this.finished = false;
                this.headersSent = false;
                this._parserState = null;
                this._leftover = null;
                this._agent = options.agent === undefined ? httpGlobalAgent : (options.agent === false ? null : options.agent);
                // Node: `options.createConnection` replaces the agent's own
                // factory for this request.
                this._createConnection = typeof options.createConnection === 'function' ? options.createConnection : null;
                this._keepAliveKey = null;
                this._timeoutMs = options.timeout || 0;
                this._timeoutTimer = null;
                if (options.headers) {
                    var oh = options.headers;
                    Object.keys(oh).forEach(function(k) {
                        this._headers[k.toLowerCase()] = oh[k];
                        this._headerNames[k.toLowerCase()] = k;
                    }, this);
                }
                if (options.auth) this.setHeader('Authorization', 'Basic ' + Buffer.from(String(options.auth)).toString('base64'));
                if (typeof callback === 'function') this.once('response', callback);
                var self = this;
                if (options.signal) {
                    var sig = options.signal;
                    var onAbort = function() {
                        var e = new Error('The operation was aborted');
                        e.code = 'ABORT_ERR';
                        e.name = 'AbortError';
                        self.destroy(e);
                    };
                    if (sig.aborted) setTimeout(onAbort, 0);
                    else if (typeof sig.addEventListener === 'function') sig.addEventListener('abort', onAbort);
                }
            }
            util.inherits(ClientRequest, EventEmitter);
            ClientRequest.prototype.setHeader = function(k, v) {
                var lk = String(k).toLowerCase();
                if (this._headerSent) {
                    var e = new Error('Cannot set headers after they are sent to the client');
                    e.code = 'ERR_HTTP_HEADERS_SENT';
                    throw e;
                }
                if (v === undefined) { delete this._headers[lk]; delete this._headerNames[lk]; return this; }
                this._headers[lk] = v;
                this._headerNames[lk] = String(k);
                return this;
            };
            ClientRequest.prototype.getHeader = function(k) { return this._headers[String(k).toLowerCase()]; };
            ClientRequest.prototype.getHeaders = function() {
                var out = {};
                for (var k in this._headers) out[this._headerNames[k] || k] = this._headers[k];
                return out;
            };
            ClientRequest.prototype.removeHeader = function(k) {
                var lk = String(k).toLowerCase();
                delete this._headers[lk]; delete this._headerNames[lk];
                return this;
            };
            // ponytail: no-op by design — TCP socket options are hints; see docs/05-js-engine/07-inventario-stubs.md §7.2.
            ClientRequest.prototype.setNoDelay = function() { return this; };
            ClientRequest.prototype.setSocketKeepAlive = function() { return this; };
            ClientRequest.prototype.setTimeout = function(ms, cb) {
                if (typeof ms === 'function') { cb = ms; ms = 0; }
                this._timeoutMs = ms;
                if (typeof cb === 'function') this.once('timeout', cb);
                // Node hands the timeout to the socket; the socket's
                // 'timeout' event is what the request reports.
                if (this._socket && typeof this._socket.setTimeout === 'function') {
                    this._socket.setTimeout(ms);
                } else {
                    var self = this;
                    this.once('socket', function(sock) {
                        if (sock && typeof sock.setTimeout === 'function') sock.setTimeout(self._timeoutMs);
                    });
                }
                return this;
            };
            ClientRequest.prototype._clearTimer = function() {
                if (this._timeoutTimer) { clearTimeout(this._timeoutTimer); this._timeoutTimer = null; }
            };
            // The 'timeout' event fires once, no matter whether the deadline
            // came from the request's own timer or from the socket's.
            ClientRequest.prototype._emitTimeout = function() {
                if (this._timeoutFired) return;
                this._timeoutFired = true;
                this.emit('timeout');
            };
            ClientRequest.prototype._armTimeout = function() {
                var self = this;
                if (self._timeoutMs > 0 && !self._timeoutTimer && !self._responseDone) {
                    self._timeoutTimer = setTimeout(function() {
                        self._timeoutTimer = null;
                        self._emitTimeout();
                    }, self._timeoutMs);
                }
            };
            ClientRequest.prototype._fail = function(err) {
                if (this._aborted || this._responseDone) return;
                this._aborted = true;
                this._hadError = true;
                this._clearTimer();
                this.emit('error', err);
                if (this._socket && !this._socket.destroyed) this._socket.destroy();
            };
            // A socket 'error' (ECONNRESET/EPIPE) once the response has begun
            // is the server tearing the connection down — a `Connection:
            // close` server that resets right after its final bytes — not the
            // request's own error. The response path finishes it ('end' on a
            // graceful close, 'close' on a reset), exactly like Node's
            // socketCloseListener. Before any response it is still a request
            // error ('socket hang up').
            ClientRequest.prototype._socketError = function(err) {
                if (this._heardResponse && this._response && !this._response.complete) {
                    this._onSocketEnd();
                    return;
                }
                this._fail(err);
            };
            ClientRequest.prototype.write = function(chunk, encoding, cb) {
                if (typeof encoding === 'function') { cb = encoding; encoding = 'utf8'; }
                if (this._ended) {
                    var err = new Error('write after end');
                    err.code = 'ERR_STREAM_WRITE_AFTER_END';
                    this.emit('error', err);
                    if (cb) cb(err);
                    return false;
                }
                if (chunk === undefined || chunk === null) { if (cb) cb(); return true; }
                var buf = __reqChunkToBuffer(chunk, encoding);
                // An empty write is a no-op, as in Node: it must not send the
                // head (with Transfer-Encoding: chunked) nor a premature `0`
                // chunk. `req.write('')` followed by `end()` otherwise framed
                // the body as two empty chunked terminators, which a Node
                // server rejects with 400 (the express differential).
                if (buf.length === 0) { if (cb) queueMicrotask(cb); return true; }
                this._bodyLen += buf.length;
                // Node's OutgoingMessage: the head goes out on the first
                // write(), not at end(). A streamed upload (or a proxy that
                // wants the request in flight before the body is complete)
                // otherwise never reaches the server: this is why
                // `req.write('hello')` without end() used to hang.
                if (!this._headerSent) {
                    this._ensureConnected();
                    if (this._connected) this._sendHead();
                }
                var flushed = true;
                if (this._headerSent && this._socket) {
                    flushed = this._sendChunk(buf);
                } else {
                    this._body.push(buf);
                    this._pendingLen = (this._pendingLen || 0) + buf.length;
                }
                if (cb) cb();
                // Same contract as any Writable: false once the pending bytes
                // pass the high-water mark (the body is only flushed at
                // end(), so a `while (req.write(buf))` loop must stop here).
                return flushed && (this._pendingLen || 0) < (this.writableHighWaterMark || 16384);
            };
            ClientRequest.prototype._sendChunk = function(buf) {
                var socket = this._socket;
                if (!socket || socket.destroyed) return false;
                if (this._encodeChunked) {
                    return socket.write(Buffer.concat([Buffer.from(buf.length.toString(16) + '\r\n'), buf, Buffer.from('\r\n')]));
                }
                return socket.write(buf);
            };
            ClientRequest.prototype._flushBody = function() {
                var body = this._body;
                this._body = [];
                this._pendingLen = 0;
                for (var i = 0; i < body.length; i++) this._sendChunk(body[i]);
            };
            ClientRequest.prototype.end = function(chunk, encoding, cb) {
                if (typeof chunk === 'function') { cb = chunk; chunk = null; }
                if (typeof encoding === 'function') { cb = encoding; encoding = null; }
                if (this._ended) {
                    if (typeof cb === 'function') {
                        if (this._finishEmitted) queueMicrotask(cb);
                        else this.once('finish', cb);
                    }
                    return this;
                }
                this._ended = true;
                this.writableEnded = true;
                this.finished = true;
                if (chunk !== undefined && chunk !== null) {
                    var buf = __reqChunkToBuffer(chunk, encoding);
                    this._bodyLen += buf.length;
                    // The head may already be out (flushHeaders, or a streaming
                    // request that already wrote): send the final chunk framed,
                    // instead of leaving it queued in _body forever.
                    if (this._headerSent && this._socket) this._sendChunk(buf);
                    else this._body.push(buf);
                }
                this._ensureConnected();
                // Node calls end()'s callback from 'finish', after the last
                // byte of the request is out; it used to run synchronously,
                // before 'finish' had been emitted.
                if (typeof cb === 'function') this.once('finish', cb);
                this._maybeFinishRequest();
                return this;
            };
            ClientRequest.prototype.flushHeaders = function() {
                this._ensureConnected();
                if (this._connected) this._sendHead();
                return this;
            };
            // Node's ClientRequest.abort (lib/_http_client.js): mark the
            // request aborted, emit 'abort' on a later tick and destroy it.
            // The socket's 'close' then reports the response as aborted (or,
            // with no response, 'socket hang up' on the request).
            ClientRequest.prototype.abort = function() {
                if (this.aborted) return this;
                this.aborted = true;
                var self = this;
                (typeof process !== 'undefined' && typeof process.nextTick === 'function'
                    ? process.nextTick : function(f) { queueMicrotask(f); })(function() { self.emit('abort'); });
                return this.destroy();
            };
            // Node's ClientRequest.destroy: nothing left to cancel once the
            // request was fully sent and the response fully received; tearing
            // the socket down then would emit an error on a socket that is
            // already back in the agent (lib/_http_client.js).
            ClientRequest.prototype.destroy = function(err) {
                if (this.destroyed) {
                    if (err && this.listenerCount('error') > 0) this.emit('error', err);
                    return this;
                }
                this.destroyed = true;
                this.errored = err === undefined ? null : err;
                this._clearTimer();
                if (this._response && typeof this._response._dump === 'function') this._response._dump();
                if (this.writableFinished && this._response && this._response.complete) return this;
                if (this._socket && !this._socket.destroyed) this._socket.destroy(err);
                else if (err) this.emit('error', err);
                return this;
            };
            ClientRequest.prototype._ensureConnected = function() {
                if (this._socket || this.destroyed) return;
                var self = this;
                this._keepAliveKey = (self._secure ? 'https:' : 'http:') + '//' + self._host + ':' + self._port;
                var reused = (self._agent && typeof self._agent._getFreeSocket === 'function')
                    ? self._agent._getFreeSocket(self._keepAliveKey) : null;
                if (reused) {
                    self._socket = reused;
                    self._connected = true;
                    self._socketErrorHandler = function(err) { self._socketError(err); };
                    reused.on('error', self._socketErrorHandler);
                    var agent = self._agent;
                    if (agent && agent.sockets) {
                        var list = agent.sockets[self._keepAliveKey];
                        if (!list) { list = []; agent.sockets[self._keepAliveKey] = list; }
                        list.push(reused);
                    }
                    self._attachSocket(reused);
                    self.emit('socket', reused);
                    self._sendHead();
                    self._flushBody();
                    self._maybeFinishRequest();
                    self._armTimeout();
                    return;
                }
                // Node's lib/_http_client.js: the socket comes from
                // `options.createConnection` when the caller gave one, from
                // the agent's `createConnection` when there is an agent, and
                // from a plain connection otherwise. An agent that replaces
                // it is how the proxy agents work, so the request has to go
                // through it and not straight to the origin.
                // The options the factory gets. The TLS ones come from the
                // request, so an agent (or a proxy agent) can build the
                // connection the application asked for.
                var options = {
                    host: self._host,
                    port: self._port,
                    path: self._path,
                    method: self._method,
                    secure: self._secure,
                    servername: self._servername || self._host,
                    ca: self._ca,
                    cert: self._cert,
                    key: self._key,
                    passphrase: self._passphrase,
                    rejectUnauthorized: self._rejectUnauthorized,
                };
                var factory = null;
                if (typeof self._createConnection === 'function') {
                    factory = self._createConnection;
                } else if (self._agent && typeof self._agent.createConnection === 'function') {
                    var agent = self._agent;
                    factory = function(opts, cb) { return agent.createConnection(opts, cb); };
                }
                if (factory) {
                    // `once`: the factory may both return the socket and call
                    // the callback, and only the first one counts.
                    var settled = false;
                    var oncreate = function(err, socket) {
                        if (settled) return;
                        settled = true;
                        if (err) { self._fail(err); return; }
                        if (!socket) return;
                        self._useSocket(socket);
                    };
                    var created = null;
                    try {
                        created = factory(options, oncreate);
                    } catch (e) {
                        this._fail(e);
                        return;
                    }
                    if (created) oncreate(null, created);
                    return;
                }
                var socket = self._secure
                    ? tlsConnect(self._port, self._host, { servername: self._host })
                    : netConnect(self._port, self._host);
                self._useSocket(socket);
            };
            // Take the socket the connection produced and start the request on
            // it. A socket 3va opened waits for its 'connect' event (a write
            // before the connection is up is lost); a socket the application
            // brought (a Duplex from `createConnection`) is used right away,
            // as in Node.
            ClientRequest.prototype._useSocket = function(socket) {
                var self = this;
                this._socket = socket;
                this._socketErrorHandler = function(err) { self._socketError(err); };
                socket.on('error', this._socketErrorHandler);
                // The agent keeps the socket it handed out, so `agent.destroy()`
                // can close it and `agent.totalSocketCount` can count it.
                var agent = this._agent;
                if (agent && agent.sockets) {
                    var list = agent.sockets[this._keepAliveKey];
                    if (!list) { list = []; agent.sockets[this._keepAliveKey] = list; }
                    list.push(socket);
                }
                var go = function() {
                    if (self.destroyed) return;
                    self._connected = true;
                    self.emit('socket', socket);
                    self._attachSocket(socket);
                    self._sendHead();
                    self._flushBody();
                    self._maybeFinishRequest();
                    self._armTimeout();
                };
                // The socket reports its own timeout; the request turns it
                // into its 'timeout' event, as in Node.
                if (this._timeoutMs > 0 && typeof socket.setTimeout === 'function') {
                    socket.setTimeout(this._timeoutMs);
                    socket.once('timeout', function() { self._emitTimeout(); });
                }
                if (socket._3vaSocket && socket._connId === null) socket.once('connect', go);
                else go();
            };
            ClientRequest.prototype._maybeFinishRequest = function() {
                if (!this._ended || !this._headerSent) return;
                if (this._encodeChunked && !this._chunkTerminated && this._socket && !this._socket.destroyed) {
                    this._chunkTerminated = true;
                    this._socket.write('0\r\n\r\n');
                }
                // The request has been fully written: Node's ClientRequest is a
                // Writable, so 'finish' is emitted (once) at this point and the
                // end() callback runs after it.
                if (!this._finishEmitted) {
                    this._finishEmitted = true;
                    var self = this;
                    queueMicrotask(function() { self.emit('finish'); });
                }
            };
            ClientRequest.prototype._sendHead = function() {
                if (this._headerSent) return;
                var socket = this._socket;
                if (!socket) return;
                this._headerSent = true;
                var headers = this._headers;
                var haveCL = headers['content-length'] !== undefined;
                var haveTE = headers['transfer-encoding'] !== undefined;
                // Node decides the framing when the head goes out: a body that
                // is already complete gets Content-Length; anything still being
                // written (or flushHeaders() before end()) gets chunked. Without
                // this the body was written with no framing header at all, so
                // Node's server never read it as the request body.
                if (!haveCL && !haveTE) {
                    if (this._ended) {
                        headers['content-length'] = String(this._bodyLen);
                        this._headerNames['content-length'] = 'Content-Length';
                    } else {
                        headers['transfer-encoding'] = 'chunked';
                        this._headerNames['transfer-encoding'] = 'Transfer-Encoding';
                        this._encodeChunked = true;
                    }
                }
                var lines = [this._method + ' ' + this._path + ' HTTP/1.1'];
                if (headers['host'] === undefined && !this._omitHost) {
                    var hostHdr = this._host + ((this._port !== 80 && this._port !== 443) ? ':' + this._port : '');
                    lines.push('Host: ' + hostHdr);
                }
                if (headers['connection'] === undefined) {
                    lines.push('Connection: ' + (this._agent && this._agent.keepAlive ? 'keep-alive' : 'close'));
                }
                var self = this;
                Object.keys(headers).forEach(function(lk) {
                    var name = self._headerNames[lk] || lk;
                    var val = headers[lk];
                    if (Array.isArray(val)) { for (var i = 0; i < val.length; i++) lines.push(name + ': ' + val[i]); }
                    else lines.push(name + ': ' + val);
                });
                lines.push('', '');
                socket.write(lines.join('\r\n'));
            };
            ClientRequest.prototype._attachSocket = function(socket) {
                var self = this;
                if (socket._detachHttpParser) socket._detachHttpParser();
                var st = {
                    headBuf: Buffer.alloc(0), headersDone: false,
                    mode: null, remaining: 0,
                    cstate: 'size', csize: 0, pending: Buffer.alloc(0),
                    res: null,
                };
                self._parserState = st;
                function onData(chunk) {
                    if (self._parserState !== st) return;
                    if (!Buffer.isBuffer(chunk)) chunk = Buffer.from(chunk);
                    if (!st.headersDone) {
                        st.headBuf = Buffer.concat([st.headBuf, chunk]);
                        var idx = st.headBuf.indexOf('\r\n\r\n');
                        if (idx < 0) return;
                        var head = st.headBuf.slice(0, idx).toString('latin1');
                        var rest = st.headBuf.slice(idx + 4);
                        st.headersDone = true;
                        self._onHeaders(__parseResponseHead(head), rest);
                        return;
                    }
                    self._consumeBody(chunk);
                }
                function onEnd() { self._onSocketEnd(); }
                function onClose() { self._onSocketClose(); }
                st.onData = onData;
                st.onEnd = onEnd;
                st.onClose = onClose;
                socket.on('data', onData);
                socket.once('end', onEnd);
                socket.once('close', onClose);
                socket._httpParserState = st;
            };
            Socket.prototype._detachHttpParser = function() {
                var st = this._httpParserState;
                if (!st) return;
                this.removeListener('data', st.onData);
                this.removeListener('end', st.onEnd);
                if (st.onClose) this.removeListener('close', st.onClose);
                this._httpParserState = null;
            };
            // Node's socketCloseListener (lib/_http_client.js): a connection
            // that goes away with the response still incomplete is an abort
            // ('aborted' on the response, then 'close' on the request); a
            // connection that closes before any response is 'socket hang up'.
            ClientRequest.prototype._onSocketClose = function() {
                var st = this._parserState;
                if (!st) return;   // detached: the response completed or the socket was released
                this._parserState = null;
                var res = this._response;
                if (res && !res.complete) {
                    res.destroy(__connReset('aborted'));
                } else if (!this._heardResponse && !this._hadError) {
                    this._fail(__connReset('socket hang up'));
                }
                if (this.outputData) this.outputData.length = 0;
                if (!this._closed) {
                    this._closed = true;
                    this.closed = true;
                    this.emit('close');
                }
            };
            ClientRequest.prototype._onHeaders = function(parsed, rest) {
                var self = this;
                var socket = self._socket;
                var res = new ClientIncomingMessage(socket);
                res.httpVersionMajor = parsed.major;
                res.httpVersionMinor = parsed.minor;
                res.httpVersion = parsed.major + '.' + parsed.minor;
                res.statusCode = parsed.statusCode;
                res.statusMessage = parsed.statusMessage;
                res.headers = parsed.headers;
                res.rawHeaders = parsed.rawHeaders;
                self._response = res;
                self.res = res;
                res.req = self;

                if (parsed.statusCode === 101) {
                    self._clearTimer();
                    self._heardResponse = true;
                    if (socket._detachHttpParser) socket._detachHttpParser();
                    self._parserState = null;
                    res.upgrade = true;
                    self.emit('upgrade', res, socket, rest);
                    return;
                }
                // 1xx informational (other than 101): keep reading the real head.
                if (parsed.statusCode >= 100 && parsed.statusCode < 200) {
                    self._parserState.headersDone = false;
                    self._parserState.headBuf = rest || Buffer.alloc(0);
                    self._parserState.onData(Buffer.alloc(0));
                    return;
                }
                self._heardResponse = true;
                self._clearTimer();
                var te = res.headers['transfer-encoding'];
                var cl = res.headers['content-length'];
                var noBody = self._method === 'HEAD' || parsed.statusCode === 204 || parsed.statusCode === 304;
                if (noBody) self._parserState.mode = 'none';
                else if (te && /(^|,)\s*chunked\s*(,|$)/i.test(String(te))) { self._parserState.mode = 'chunked'; self._parserState.cstate = 'size'; }
                else if (cl !== undefined && /^\d+$/.test(String(cl).trim())) { self._parserState.mode = 'length'; self._parserState.remaining = parseInt(String(cl).trim(), 10); }
                else self._parserState.mode = 'eof';
                self.emit('response', res);
                if (rest && rest.length) self._consumeBody(rest);
                else self._checkResponseComplete();
            };
            ClientRequest.prototype._checkResponseComplete = function() {
                var st = this._parserState;
                if (!st || !this._response) return;
                if (st.mode === 'none' || (st.mode === 'length' && st.remaining <= 0)) this._finishResponse();
            };
            ClientRequest.prototype._consumeBody = function(chunk) {
                var st = this._parserState;
                var res = this._response;
                if (!st || !res) return;
                if (st.mode === 'none') return;
                if (st.mode === 'length') {
                    if (st.remaining <= 0) { this._finishResponse(); return; }
                    if (chunk.length > st.remaining) {
                        var take = st.remaining;
                        res.push(chunk.slice(0, take));
                        st.remaining = 0;
                        this._leftover = chunk.slice(take);
                        this._finishResponse();
                        return;
                    }
                    st.remaining -= chunk.length;
                    res.push(chunk);
                    if (st.remaining <= 0) this._finishResponse();
                    return;
                }
                if (st.mode === 'eof') { res.push(chunk); return; }
                if (st.mode === 'chunked') this._feedChunked(chunk, res);
            };
            ClientRequest.prototype._feedChunked = function(chunk, res) {
                var st = this._parserState;
                st.pending = st.pending.length ? Buffer.concat([st.pending, chunk]) : chunk;
                var guard = 0;
                while (guard++ < 1000000) {
                    if (st.cstate === 'size') {
                        var nl = st.pending.indexOf('\r\n');
                        if (nl < 0) return;
                        var line = st.pending.slice(0, nl).toString('latin1');
                        var semi = line.indexOf(';');
                        if (semi >= 0) line = line.slice(0, semi);
                        var size = parseInt(line.trim(), 16);
                        if (!(size >= 0)) { this._fail(new Error('Invalid chunk size')); return; }
                        st.pending = st.pending.slice(nl + 2);
                        if (size === 0) { st.cstate = 'trailers'; }
                        else { st.csize = size; st.cstate = 'data'; }
                    } else if (st.cstate === 'data') {
                        if (st.pending.length < st.csize) return;
                        res.push(st.pending.slice(0, st.csize));
                        st.pending = st.pending.slice(st.csize);
                        st.cstate = 'data_crlf';
                    } else if (st.cstate === 'data_crlf') {
                        if (st.pending.length < 2) return;
                        st.pending = st.pending.slice(2);
                        st.cstate = 'size';
                    } else if (st.cstate === 'trailers') {
                        if (st.pending.length >= 2 && st.pending[0] === 13 && st.pending[1] === 10) {
                            st.pending = st.pending.slice(2);
                            st.cstate = 'done';
                        } else {
                            var nlnl = st.pending.indexOf('\r\n\r\n');
                            if (nlnl < 0) return;
                            st.pending = st.pending.slice(nlnl + 4);
                            st.cstate = 'done';
                        }
                    } else {
                        this._leftover = st.pending;
                        st.pending = Buffer.alloc(0);
                        this._finishResponse();
                        return;
                    }
                    if (st.cstate === 'done') {
                        this._leftover = st.pending;
                        st.pending = Buffer.alloc(0);
                        this._finishResponse();
                        return;
                    }
                }
            };
            ClientRequest.prototype._finishResponse = function() {
                var res = this._response;
                if (!res || res.complete) return;
                res.complete = true;
                res.readableEnded = true;
                this._responseDone = true;
                this._clearTimer();
                var socket = this._socket;
                if (socket) {
                    if (this._socketErrorHandler) { socket.removeListener('error', this._socketErrorHandler); this._socketErrorHandler = null; }
                    if (socket._detachHttpParser) socket._detachHttpParser();
                    var connHdr = String(res.headers['connection'] || '').toLowerCase();
                    var keepAlive = res.httpVersionMinor === 0 ? connHdr === 'keep-alive' : connHdr !== 'close';
                    keepAlive = keepAlive && !this.destroyed && !this._aborted && !socket.destroyed;
                    // Release the socket *before* ending the response: the next
                    // request is usually created from this stream's 'end'
                    // handler, and returning it after that made the agent hand
                    // out a fresh connection instead of reusing this one.
                    var agent = this._agent;
                    if (agent && agent.sockets && agent.sockets[this._keepAliveKey]) {
                        var inUse = agent.sockets[this._keepAliveKey];
                        var at = inUse.indexOf(socket);
                        if (at >= 0) inUse.splice(at, 1);
                    }
                    if (keepAlive && agent && typeof agent._returnSocket === 'function') {
                        agent._returnSocket(this._keepAliveKey, socket);
                    } else if (!socket.destroyed) {
                        socket.destroy();
                    }
                }
                res.push(null);
                this.closed = true;
                this._closed = true;
                this.emit('close');
            };
            ClientRequest.prototype._onSocketEnd = function() {
                var st = this._parserState;
                if (!this._heardResponse) {
                    if (!this._aborted) this._fail(__connReset('socket hang up'));
                    return;
                }
                var res = this._response;
                if (!res || res.complete) return;
                if (st && st.mode === 'chunked' && st.cstate !== 'done') {
                    this._fail(new Error('Premature close'));
                    return;
                }
                this._finishResponse();
            };

            httpAgent.prototype._getFreeSocket = function(key) {
                if (!this.keepAlive) return null;
                var list = this.freeSockets[key];
                while (list && list.length) {
                    var s = list.pop();
                    if (s && !s.destroyed && !(s._readableState && s._readableState.ended) && !(s._writableState && s._writableState.ended)) return s;
                    if (s && !s.destroyed) s.destroy();
                }
                return null;
            };
            httpAgent.prototype._returnSocket = function(key, socket) {
                if (!this.keepAlive || !socket || socket.destroyed) { if (socket && !socket.destroyed) socket.destroy(); return; }
                var list = (this.freeSockets[key] = this.freeSockets[key] || []);
                if (list.length >= (this.maxFreeSockets || 256)) { socket.destroy(); return; }
                list.push(socket);
            };

            // Node's `http.request(url[, options][, callback])`: the URL can
            // come first, and the options are merged with it.
            function __mergeUrlOptions(url, extra) {
                var u = (url instanceof URL) ? url : new URL(String(url));
                var opts = {};
                if (extra) { for (var k in extra) opts[k] = extra[k]; }
                opts.protocol = u.protocol;
                opts.hostname = u.hostname;
                opts.host = u.host;
                opts.port = u.port || (u.protocol === 'https:' ? 443 : 80);
                opts.path = (u.pathname || '/') + (u.search || '');
                if (u.hash) opts.hash = u.hash;
                if (u.username || u.password) {
                    opts.auth = decodeURIComponent(u.username) + ':' + decodeURIComponent(u.password);
                }
                return opts;
            }
            function httpRequest(options, callback, extra) {
                // http.request(url, options, callback)
                if (typeof options === 'string' || (typeof URL !== 'undefined' && options instanceof URL)) {
                    var cb = (typeof callback === 'function') ? callback : (typeof extra === 'function' ? extra : null);
                    var opts = (typeof callback === 'function') ? {} : (callback || {});
                    return new ClientRequest(__mergeUrlOptions(options, opts), cb);
                }
                return new ClientRequest(options, callback);
            }
            function httpGet(options, callback, extra) {
                var req = httpRequest(options, callback, extra);
                req.end();
                return req;
            }
            function httpsRequest(options, callback, extra) {
                if (typeof options === 'string' || (typeof URL !== 'undefined' && options instanceof URL)) {
                    var cb = (typeof callback === 'function') ? callback : (typeof extra === 'function' ? extra : null);
                    var opts = (typeof callback === 'function') ? {} : (callback || {});
                    return new ClientRequest(__mergeUrlOptions(options, opts), cb, 'https:');
                }
                return new ClientRequest(options, callback, 'https:');
            }
            function httpsGet(options, callback, extra) {
                var req = httpsRequest(options, callback, extra);
                req.end();
                return req;
            }

            function createServer(opts, requestListener) {
                return new httpServer(opts, requestListener);
            }

            // Node's `http.METHODS` is the list llhttp knows, sorted.
            var HTTP_METHODS = [
                'ACL','BIND','CHECKOUT','CONNECT','COPY','DELETE',
                'GET','HEAD','LINK','LOCK','M-SEARCH','MERGE',
                'MKACTIVITY','MKCALENDAR','MKCOL','MOVE','NOTIFY',
                'OPTIONS','PATCH','POST','PRI','PROPFIND','PROPPATCH',
                'PURGE','PUT','REBIND','REPORT','SEARCH','SOURCE',
                'SUBSCRIBE','TRACE','UNBIND','UNLINK','UNLOCK','UNSUBSCRIBE'
            ].sort();

            var httpModule = {
                createServer: createServer,
                globalAgent: httpGlobalAgent,
                Agent: httpAgent,
                Server: httpServer,
                OutgoingMessage: httpOutgoingMessage,
                IncomingMessage: httpIncomingMessage,
                ServerResponse: httpServerResponse,
                request: httpRequest,
                get: httpGet,
                ClientRequest: ClientRequest,
                maxHeaderSize: 16384,
                STATUS_CODES: HTTP_STATUS_CODES,
                METHODS: HTTP_METHODS
            };
            globalThis.__requireCache['http'] = httpModule;
            globalThis.__requireCache['node:http'] = httpModule;

            // ── https module ─────────────────────────────────────────────────────────
            var httpsModule = {
                // A real TLS listener (http_server.rs). This used to return a
                // plain HTTP server and ignore `key`/`cert`: the app believed
                // it was serving TLS while the port spoke cleartext. Options
                // that can't be honoured are refused, never silently dropped.
                createServer: function(opts, requestListener) {
                    if (typeof opts === 'function') { requestListener = opts; opts = {}; }
                    opts = opts || {};
                    var pem = function(v, what) {
                        if (Array.isArray(v)) v = v.map(function(x) { return pem(x, what); }).join('\n');
                        if (typeof Buffer !== 'undefined' && Buffer.isBuffer(v)) v = v.toString('utf8');
                        else if (v instanceof Uint8Array) v = new TextDecoder().decode(v);
                        if (typeof v !== 'string' || v.indexOf('-----BEGIN') === -1) {
                            var e = new TypeError('https.createServer: `' + what + '` must be a PEM string or Buffer');
                            e.code = 'ERR_INVALID_ARG_VALUE';
                            throw e;
                        }
                        return v;
                    };
                    ['pfx', 'passphrase', 'requestCert', 'ca', 'SNICallback', 'crl'].forEach(function(k) {
                        if (opts[k] !== undefined && opts[k] !== false && opts[k] !== null) {
                            var e = new Error('https.createServer: the `' + k + '` option is not supported by 3va yet');
                            e.code = 'ERR_FEATURE_UNAVAILABLE_ON_PLATFORM';
                            throw e;
                        }
                    });
                    var server = createServer(opts, requestListener);
                    var absent = function(v) { return v === undefined || v === null; };
                    if (absent(opts.key) || absent(opts.cert)) {
                        // Like Node, creating the server doesn't need a
                        // certificate; listening does. Instead of listening
                        // in cleartext, listen() fails (see httpServer.listen).
                        server._tlsMissing = true;
                    } else {
                        server._tls = { key: pem(opts.key, 'key'), cert: pem(opts.cert, 'cert') };
                    }
                    return server;
                },
                globalAgent: new httpAgent(),
                Agent: httpAgent,
                Server: Server,
                request: httpsRequest,
                get: httpsGet,
                ClientRequest: ClientRequest
            };
            globalThis.__requireCache['https'] = httpsModule;
            globalThis.__requireCache['node:https'] = httpsModule;

            // ── legacy `_http_*` / `_stream_*` modules ───────────────────────────
            // Node still ships these deprecated aliases and they are public
            // API (plenty of old code requires them), so they alias what 3va
            // already implements instead of duplicating an implementation.
            // `HTTPParser` is not aliased: 3va has no equivalent to expose.
            (function() {
                // Node's lib/_http_common.js: a header field name must be an
                // HTTP token; a header field value must not carry control
                // characters (except HTAB) nor DEL. The lenient variant
                // (`--insecure-http-parser`) only rejects NUL, CR, LF and
                // characters above U+00FF, per the Fetch specification.
                var httpTokenRegex = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
                var headerCharRegex = /[^\t\x20-\x7e\x80-\xff]/;
                var lenientHeaderCharRegex = /[\x00\r\n]|[\u0100-\uffff]/;
                var _checkIsHttpToken = function(val) { return httpTokenRegex.test(val); };
                var _checkInvalidHeaderChar = function(val, insecureHTTPParser) {
                    if (insecureHTTPParser) return lenientHeaderCharRegex.test(val);
                    return headerCharRegex.test(val);
                };
                globalThis.__requireCache['_http_common'] = {
                    methods: HTTP_METHODS,
                    _checkIsHttpToken: _checkIsHttpToken,
                    _checkInvalidHeaderChar: _checkInvalidHeaderChar
                };
                globalThis.__requireCache['_http_server'] = {
                    Server: httpModule.Server,
                    ServerResponse: httpModule.ServerResponse,
                    STATUS_CODES: HTTP_STATUS_CODES,
                    kConnectionsCheckingInterval: 30000
                };
                globalThis.__requireCache['_http_agent'] = {
                    Agent: httpModule.Agent,
                    globalAgent: httpModule.globalAgent
                };
                globalThis.__requireCache['_http_client'] = {
                    ClientRequest: httpModule.ClientRequest
                };
                globalThis.__requireCache['_http_incoming'] = {
                    IncomingMessage: httpModule.IncomingMessage
                };
                globalThis.__requireCache['_http_outgoing'] = {
                    OutgoingMessage: httpModule.OutgoingMessage,
                    kHighWaterMark: 16384
                };

                // Node's lib/_stream_*.js export the class itself, so
                // `require('_stream_readable') === stream.Readable`.
                var Stream = globalThis.__requireCache['stream'];
                globalThis.__requireCache['_stream_readable'] = Stream.Readable;
                globalThis.__requireCache['_stream_writable'] = Stream.Writable;
                globalThis.__requireCache['_stream_duplex'] = Stream.Duplex;
                globalThis.__requireCache['_stream_transform'] = Stream.Transform;
                globalThis.__requireCache['_stream_passthrough'] = Stream.PassThrough;
            })();

            // ── reflect-metadata polyfill ──────────────────────────────────────────
            (function() {
                var metadataMap = new WeakMap();
                var symbol = Symbol.for('Reflect.metadata');

                function assertFunction(fn) {
                    if (fn === null || fn === undefined || (typeof fn !== 'function' && typeof fn !== 'object')) throw new TypeError('argument must be a function');
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
            AsyncResource.prototype.runInAsyncScope = function(fn, thisArg) {
                var args = Array.prototype.slice.call(arguments, 2);
                var prevFrame = __acsGet();
                __acsSet(this._frame);
                try {
                    return fn.apply(thisArg || null, args);
                } finally {
                    __acsSet(prevFrame);
                }
            };
            // ponytail: no-op by design — 3va has no AsyncResource destroy hooks to run; see docs/05-js-engine/07-inventario-stubs.md §7.5.
            AsyncResource.prototype.emitDestroy = function() { return this; };
            AsyncResource.prototype.asyncId = function() { return 0; };
            AsyncResource.prototype.triggerAsyncId = function() { return 0; };
            AsyncResource.prototype.bind = function(fn) {
                var self = this;
                return function() {
                    var args = arguments, thisArg = this;
                    return self.runInAsyncScope(fn, thisArg, ...args);
                };
            };
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
                    // ponytail: no-op by design — diagnostics_channel enable/disable has no state to change; see docs/05-js-engine/07-inventario-stubs.md §7.5.
                    return { enable: function() { return this; }, disable: function() { return this; } };
                },
            };
            globalThis.__requireCache['node:async_hooks'] = globalThis.__requireCache['async_hooks'];

        // Stub for native Rollup binary — 3va can't load .node addons.
        // Hash functions use FNV-1a; parse/parseAsync throw so callers fall back
        // to the JS-only path if one exists, or Vite skips parsing at dev time.
        (function() {
            function fnv1a(s) {
                var h = 2166136261;
                var buf = typeof s === 'string' ? s : String(s);
                for (var i = 0; i < buf.length; i++) {
                    h ^= buf.charCodeAt(i);
                    h = (h * 16777619) >>> 0;
                }
                return h;
            }
            var rollupNativeStub = {
                parse: function() { throw new Error('Native Rollup parse unavailable in 3va'); },
                parseAsync: function() { return Promise.reject(new Error('Native Rollup parse unavailable in 3va')); },
                xxhashBase64Url: function(s) { return btoa(String(fnv1a(s))).replace(/=+$/,''); },
                xxhashBase36: function(s) { return fnv1a(s).toString(36); },
                xxhashBase16: function(s) { return fnv1a(s).toString(16); },
            };
            globalThis.__requireCache['@rollup/rollup-linux-x64-gnu'] = rollupNativeStub;
            globalThis.__requireCache['@rollup/rollup-linux-x64-musl'] = rollupNativeStub;
            globalThis.__requireCache['@rollup/rollup-linux-arm64-gnu'] = rollupNativeStub;
            globalThis.__requireCache['@rollup/rollup-darwin-x64'] = rollupNativeStub;
            globalThis.__requireCache['@rollup/rollup-darwin-arm64'] = rollupNativeStub;
            globalThis.__requireCache['@rollup/rollup-win32-x64-msvc'] = rollupNativeStub;
        })();

        // Stub for rollup/parseAst — Vite uses parseAstAsync in ssrTransform to
        // rewrite import/export for the module runner. Use @babel/parser (installed
        // as a dep of @babel/types which ships with Astro) with the estree plugin
        // so it outputs ESTree-compatible nodes that Vite's ssrTransform expects.
        (function() {
            var _babelParser = null;
            function getBabelParser() {
                if (!_babelParser) {
                    _babelParser = globalThis.require('@babel/parser');
                }
                return _babelParser;
            }
            var parseAstStub = {
                parseAst: function(code, opts) {
                    var parser = getBabelParser();
                    var result = parser.parse(code, {
                        plugins: ['estree'],
                        sourceType: 'module',
                        allowReturnOutsideFunction: !!(opts && opts.allowReturnOutsideFunction),
                        allowImportExportEverywhere: true,
                        errorRecovery: true,
                    });
                    // @babel/parser returns { type:"File", program:{type:"Program",body:[...]} }
                    // Vite's ssrTransform expects the Program node directly (ast.body iterable).
                    return (result && result.program) ? result.program : result;
                },
                parseAstAsync: function(code, opts) {
                    try {
                        return Promise.resolve(parseAstStub.parseAst(code, opts));
                    } catch (e) {
                        return Promise.reject(e);
                    }
                },
            };
            globalThis.__requireCache['rollup/parseAst'] = parseAstStub;
        })();

        (function() {
            // StringDecoder itself is defined at top level (see the stream
            // section) so Readable/stdin can share it; here it is only
            // exposed as the string_decoder module.
            globalThis.__requireCache['string_decoder'] = { StringDecoder: StringDecoder };
            globalThis.__requireCache['node:string_decoder'] = globalThis.__requireCache['string_decoder'];
        })();

        (function() {
            if (globalThis.__requireCache['util']) {
                globalThis.__requireCache['sys'] = globalThis.__requireCache['util'];
                globalThis.__requireCache['node:sys'] = globalThis.__requireCache['util'];
            }
        })();

        (function() {
            var MAX_LENGTH = 40;
            var PUNYCODE_BASE = 36;
            var TMIN = 1;
            var TMAX = 26;
            var SKEW = 38;
            var DAMP = 700;
            var INITIAL_BIAS = 72;
            var INITIAL_N = 128;
            var DELIMITER = '-';
            function adapt(delta, numPoints, firstTime) {
                delta = firstTime ? Math.floor(delta / DAMP) : delta >> 1;
                delta += Math.floor(delta / numPoints);
                var k = 0;
                while (delta > ((PUNYCODE_BASE - TMIN) * TMAX) >> 1) {
                    delta = Math.floor(delta / (PUNYCODE_BASE - TMIN));
                    k += PUNYCODE_BASE;
                }
                return k + Math.floor(((PUNYCODE_BASE - TMIN + 1) * delta) / (delta + SKEW));
            }
            function digitToBasic(digit) {
                return digit < 26 ? digit + 97 : digit - 26 + 22;
            }
            function basicToDigit(codePoint) {
                if (codePoint >= 48 && codePoint < 58) return codePoint - 22;
                if (codePoint >= 65 && codePoint < 91) return codePoint - 65;
                if (codePoint >= 97 && codePoint < 123) return codePoint - 97;
                return PUNYCODE_BASE;
            }
            function encode(input) {
                var output = [];
                var inputArray = Array.from(input);
                var inputLength = inputArray.length;
                var n = INITIAL_N;
                var delta = 0;
                var bias = INITIAL_BIAS;
                for (var i = 0; i < inputLength; i++) {
                    if (inputArray[i].charCodeAt(0) < 128) output.push(inputArray[i]);
                }
                var basicLength = output.length;
                var handledCPCount = basicLength;
                if (basicLength > 0) output.push(DELIMITER);
                while (handledCPCount < inputLength) {
                    var m = Infinity;
                    for (var i = 0; i < inputLength; i++) {
                        var cp = inputArray[i].charCodeAt(0);
                        if (cp >= n && cp < m) m = cp;
                    }
                    if (m - n > Math.floor((2147483647 - delta) / (handledCPCount + 1))) throw new RangeError('Overflow');
                    delta += (m - n) * (handledCPCount + 1);
                    n = m;
                    for (var i = 0; i < inputLength; i++) {
                        var cp = inputArray[i].charCodeAt(0);
                        if (cp < n && ++delta > 2147483647) throw new RangeError('Overflow');
                        if (cp === n) {
                            var q = delta;
                            for (var k = PUNYCODE_BASE; ; k += PUNYCODE_BASE) {
                                var t = k <= bias ? TMIN : (k >= bias + TMAX ? TMAX : k - bias);
                                if (q < t) break;
                                output.push(String.fromCharCode(digitToBasic(t + (q - t) % (PUNYCODE_BASE - t))));
                                q = Math.floor((q - t) / (PUNYCODE_BASE - t));
                            }
                            output.push(String.fromCharCode(digitToBasic(q)));
                            bias = adapt(delta, handledCPCount + 1, handledCPCount === basicLength);
                            delta = 0;
                            handledCPCount++;
                        }
                    }
                    delta++;
                    n++;
                }
                return output.join('');
            }
            function decode(input) {
                var output = [];
                var inputLength = input.length;
                var i = 0;
                var n = INITIAL_N;
                var bias = INITIAL_BIAS;
                var basic = input.lastIndexOf(DELIMITER);
                if (basic < 0) basic = 0;
                for (var j = 0; j < basic; j++) output.push(input[j]);
                var index = basic > 0 ? basic + 1 : 0;
                while (index < inputLength) {
                    var oldi = i;
                    var w = 1;
                    for (var k = PUNYCODE_BASE; ; k += PUNYCODE_BASE) {
                        if (index >= inputLength) throw new RangeError('Invalid input');
                        var digit = basicToDigit(input.charCodeAt(index++));
                        if (digit >= PUNYCODE_BASE) throw new RangeError('Invalid input');
                        if (digit > Math.floor((2147483647 - i) / w)) throw new RangeError('Overflow');
                        i += digit * w;
                        var t = k <= bias ? TMIN : (k >= bias + TMAX ? TMAX : k - bias);
                        if (digit < t) break;
                        if (w > Math.floor(2147483647 / (PUNYCODE_BASE - t))) throw new RangeError('Overflow');
                        w *= PUNYCODE_BASE - t;
                    }
                    bias = adapt(i - oldi, output.length + 1, oldi === 0);
                    if (Math.floor(i / (output.length + 1)) > 2147483647 - n) throw new RangeError('Overflow');
                    n += Math.floor(i / (output.length + 1));
                    i %= output.length + 1;
                    output.splice(i, 0, String.fromCharCode(n));
                    i++;
                }
                return output.join('');
            }
            function toASCII(domain) {
                return domain.split('.').map(function(label) {
                    if (/^[a-zA-Z0-9-]+$/.test(label)) return label;
                    return 'xn--' + encode(label);
                }).join('.');
            }
            function toUnicode(domain) {
                return domain.split('.').map(function(label) {
                    if (label.indexOf('xn--') === 0) return decode(label.slice(4));
                    return label;
                }).join('.');
            }
            globalThis.__requireCache['punycode'] = {
                encode: encode, decode: decode, toASCII: toASCII, toUnicode: toUnicode,
                ucs2: { decode: function(s) { return Array.from(s).map(function(c) { return c.charCodeAt(0); }); },
                        encode: function(arr) { return arr.map(function(c) { return String.fromCharCode(c); }).join(''); } },
                version: '2.3.1'
            };
            globalThis.__requireCache['node:punycode'] = globalThis.__requireCache['punycode'];
        })();

        (function() {
            var EE = globalThis.__requireCache['events'] || globalThis.__requireCache['node:events'];
            function Domain() {
                if (!(this instanceof Domain)) return new Domain();
                if (EE && typeof EE === 'function') EE.call(this);
                else if (EE && EE.EventEmitter) EE.EventEmitter.call(this);
                this.members = [];
            }
            if (EE && typeof EE === 'function') Domain.prototype = Object.create(EE.prototype);
            else if (EE && EE.EventEmitter) Domain.prototype = Object.create(EE.EventEmitter.prototype);
            Domain.prototype.run = function(fn) { try { return fn(); } catch(e) { this.emit('error', e); } };
            Domain.prototype.bind = function(fn) { var self = this; return function() { try { return fn.apply(this, arguments); } catch(e) { self.emit('error', e); } }; };
            Domain.prototype.intercept = function(fn) { var self = this; return function(err) { if (err) { self.emit('error', err); } else { try { return fn.apply(this, Array.prototype.slice.call(arguments, 1)); } catch(e) { self.emit('error', e); } } }; };
            Domain.prototype.add = function(emitter) { if (this.members.indexOf(emitter) === -1) this.members.push(emitter); };
            Domain.prototype.remove = function(emitter) { var i = this.members.indexOf(emitter); if (i !== -1) this.members.splice(i, 1); };
            Domain.prototype.enter = function() { process.domain = this; };
            Domain.prototype.exit = function() { process.domain = null; };
            Domain.prototype.dispose = function() { this.members = []; process.domain = null; };
            if (!process.domain) process.domain = null;
            globalThis.__requireCache['domain'] = { create: function() { return new Domain(); }, Domain: Domain };
            globalThis.__requireCache['node:domain'] = globalThis.__requireCache['domain'];
        })();
        })();
    "#;
    crate::builtins::code_cache::bootstrap_js(scope, "require-core", js_code)?;

    // ── require() — CommonJS module loader ────────────────────────────────────
    // Built-in modules (fs, process, util, events, ...) are looked up directly
    // in __requireCache, where each builtin's own inject_* registers itself.
    // Everything else is resolved from disk via the native __requireResolve /
    // __readFile bindings (which reuse the same node_modules/package.json-aware
    // resolver as ESM `import`) and executed as a CommonJS module.
    let require_js = r#"
    (function() {
        globalThis.__loadedModules = globalThis.__loadedModules || {};
        // `import()` must return a promise whose constructor is the intrinsic
        // %Promise% even after user code replaces `globalThis.Promise`
        // (test262 language/expressions/dynamic-import/returns-promise.js), so
        // capture the real constructor before any script gets to reassign it.
        var __RealPromise = globalThis.Promise;
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
        // Node 22+ requires an ES module synchronously unless its graph has
        // top-level await (ERR_REQUIRE_ASYNC_MODULE). `import()` goes through
        // the same loader and *may* wait, so the loader reads this depth: a
        // plain require() that finds a pending `__tla` throws, an import()
        // waits for it.
        globalThis.__vvvaImportDepth = globalThis.__vvvaImportDepth || 0;
        function __makeImportAsync(requireFn) {
            return function(specifier) {
                return new __RealPromise(function(resolve, reject) {
                    try {
                        globalThis.__vvvaImportDepth++;
                        var mod;
                        try {
                            mod = requireFn(specifier);
                        } finally {
                            globalThis.__vvvaImportDepth--;
                        }
                        // A module with top-level await may still be evaluating
                        // (real async work): wait for it, then re-read its exports.
                        var rec = null;
                        try { rec = globalThis.__loadedModules[requireFn.resolve(specifier)]; } catch (_) {}
                        if (rec && (rec.__tla || rec.__deps)) {
                            __waitTla(rec).then(function() { finish(rec.exports); }, reject);
                            return;
                        }
                        finish(mod);
                    } catch (e) {
                        reject(e);
                    }
                    function finish(mod) {
                        // ESM interop: CJS modules need a synthetic .default export so that
                        // `import foo from 'cjs-pkg'` and module-runner's importedNames
                        // check finds 'default'. Works for both object exports and function
                        // exports (e.g. picomatch exports a function).
                        if (mod != null && !('default' in Object(mod))) {
                            if (typeof mod === 'function' || typeof mod === 'object') {
                                var wrapped = Object.assign(Object.create(null), mod);
                                wrapped.default = mod;
                                mod = wrapped;
                            } else {
                                mod = { default: mod };
                            }
                        }
                        resolve(mod);
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
            if (!last) return '.';
            // The entry file passed to `3va run`/`3va start` is the program
            // the user explicitly chose to execute, even when it physically
            // lives under node_modules/<pkg> (a real npm/yarn `.bin` symlink
            // target, e.g. node_modules/react-native/cli.js). Treat its own
            // package the same as app code ('.') — every *other* dependency
            // it requires is still scoped normally.
            if (globalThis.__vvva_entry_scope && last === globalThis.__vvva_entry_scope) return '.';
            return last;
        }

        // Only builtins that themselves perform capability-gated native calls
        // (perms().check(...) in the Rust bindings) need scoping — wrapping
        // every required module would be pure overhead for no benefit, since
        // plain JS/data modules never touch PermissionState.
        // The set is injected by Rust (`__SCOPE_GATED_MODULES`) so a gated
        // builtin can never be forgotten here: the list lives next to the
        // permission checks it gates. ssh2/imap/ftp/pop3/irc are listed but
        // their exports are PascalCase-only, which the wrapper below
        // deliberately skips to avoid breaking `instanceof` — same documented
        // gap as `net.Socket`.
        var __SCOPE_GATED_MODULES = globalThis.__SCOPE_GATED_MODULES || {};

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
                        var w = function() {
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
                        for (var p in fn) {
                            if (Object.prototype.hasOwnProperty.call(fn, p)) { w[p] = fn[p]; }
                        }
                        return w;
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
            var cacheKey = bare + '\u0000' + scopeName;
            if (!Object.prototype.hasOwnProperty.call(__scopedModuleCache, cacheKey)) {
                __scopedModuleCache[cacheKey] = __wrapForScope(mod, scopeName, 0);
            }
            return __scopedModuleCache[cacheKey];
        }

        // Modules currently executing, so a required module with a pending
        // top-level await is recorded as a dependency of whoever required it;
        // import() then waits for the whole subgraph (async module graph order).
        var __loadingStack = [];
        // Still waiting on its own top-level await, or in an import cycle with a module that is.
        function __tlaPending(rec, seen) {
            seen = seen || [];
            if (!rec || seen.indexOf(rec) !== -1) return false;
            seen.push(rec);
            if (rec.__tla && !rec.__tlaDone) return true;
            return (rec.__cycle || []).some(function(c) { return __tlaPending(c, seen); });
        }
        function __noteTlaDep(rec) {
            var cur = __loadingStack[__loadingStack.length - 1];
            if (cur && rec && rec !== cur && __tlaPending(rec)) {
                (cur.__deps = cur.__deps || []).push(rec);
            }
        }
        function __waitTla(rec, seen) {
            seen = seen || [];
            if (seen.indexOf(rec) !== -1) return __RealPromise.resolve();
            seen.push(rec);
            var all = [rec.__tla || __RealPromise.resolve()];
            (rec.__deps || []).forEach(function(d) { all.push(__waitTla(d, seen)); });
            (rec.__cycle || []).forEach(function(c) { all.push(__waitTla(c, seen)); });
            return __RealPromise.all(all);
        }

        // Specifiers of the top-level `[var x =] require("m");` lines of a transpiled ES module.
        function __esmTopRequires(src) {
            var out = [], re = /^(?:\/\*__3va_esm__\*\/\s*)?(?:var\s+[^=\n]+=\s*)?require\("([^"\n]+)"\);?[ \t]*$/gm, m;
            while ((m = re.exec(src)) !== null) out.push(m[1]);
            return out;
        }

        function requireFrom(specifier, dir) {
            // Normalize file:// URLs: strip scheme, host, and query/fragment
            if (specifier && specifier.indexOf('file://') === 0) {
                var stripped = specifier.slice('file://'.length);
                // Remove Windows triple-slash prefix (file:///C:/ → C:/)
                if (stripped.indexOf('/') === 0 && stripped[2] === ':') stripped = stripped.slice(1);
                // Strip ?query and #fragment
                var qi = stripped.indexOf('?'); if (qi !== -1) stripped = stripped.slice(0, qi);
                var hi = stripped.indexOf('#'); if (hi !== -1) stripped = stripped.slice(0, hi);
                // Decode percent-encoded characters (e.g. %40 → @ in pnpm paths)
                try { stripped = decodeURIComponent(stripped); } catch(e) {}
                specifier = stripped;
            }
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

            var __NATIVE_REMAP = {
                'node-argon2': 'argon2-wasm-pro',
                'argon2': 'argon2-wasm-pro',
                'canvas': 'canvas-wasm',
                'sharp': 'jimp',
                'sqlite3': '__3va_node_sqlite',
                'better-sqlite3': '__3va_node_sqlite',
                'fsevents': '__3va_stub_fsevents',
                '@parcel/watcher': '__3va_stub_parcel_watcher',
                'cpu-features': '__3va_stub_cpu_features'
            };
            var __STUB_CACHE = {
                '__3va_stub_fsevents': { subscribe: function(){ return function(){}; }, unsubscribe: function(){} },
                '__3va_stub_parcel_watcher': { subscribe: function(){ return { unsubscribe: function(){} }; }, unsubscribe: function(){} },
                '__3va_stub_cpu_features': {}
            };
            if (specifier[0] !== '.' && specifier[0] !== '/' && Object.prototype.hasOwnProperty.call(__NATIVE_REMAP, specifier)) {
                var remapped = __NATIVE_REMAP[specifier];
                var scope = __pkgScopeFor(dir);
                if (typeof process !== 'undefined' && process.emitWarning) {
                    process.emitWarning("Remapped '" + specifier + "' → '" + (remapped === '__3va_node_sqlite' ? 'node:sqlite' : remapped) + "' (native addon not supported)");
                }
                if (remapped === '__3va_node_sqlite') {
                    return __scopedModule(bare, requireFrom('node:sqlite', dir), scope);
                }
                if (Object.prototype.hasOwnProperty.call(__STUB_CACHE, remapped)) {
                    return __scopedModule(bare, __STUB_CACHE[remapped], scope);
                }
                return __scopedModule(bare, requireFrom(remapped, dir), scope);
            }

            if (/\.node$/.test(specifier)) {
                var resolved_node = __requireResolve(specifier, dir);
                if (resolved_node && resolved_node.slice(-5) === '.node') {
                    return __napiLoad(resolved_node);
                }
                var err = new Error('Cannot find native addon: ' + specifier);
                err.code = 'ERR_DLOPEN_FAILED';
                throw err;
            }

            var resolved = __requireResolve(specifier, dir);

            // node: builtins (or bare node names) return themselves as sentinel.
            // Guard: absolute paths (starts with /) are never sentinels — they're
            // real files that resolve_esm_from_dir returns unchanged, so
            // resolved===specifier is a coincidence, not a sentinel signal.
            var isAbsoluteOrRelative = specifier[0] === '/' || specifier[0] === '.';
            if (resolved && (resolved.indexOf('node:') === 0 || (!isAbsoluteOrRelative && resolved === specifier))) {
                var nbCached = globalThis.__requireCache[resolved]
                    || globalThis.__requireCache['node:' + resolved]
                    || globalThis.__requireCache[bare];
                if (nbCached !== undefined) return __scopedModule(bare, nbCached, __pkgScopeFor(dir));
                return {};
            }

            if (Object.prototype.hasOwnProperty.call(globalThis.__loadedModules, resolved)) {
                var cachedRec = globalThis.__loadedModules[resolved];
                // Import cycle: cachedRec is still being loaded further up the stack, so every module
                // above it is in the cycle with it and must finish only after it (async cycles).
                var cyc = __loadingStack.indexOf(cachedRec);
                if (cyc !== -1) {
                    for (var ci = cyc + 1; ci < __loadingStack.length; ci++) {
                        (__loadingStack[ci].__cycle = __loadingStack[ci].__cycle || []).push(cachedRec);
                    }
                }
                __noteTlaDep(cachedRec);
                return cachedRec.exports;
            }


            if (resolved.slice(-5) === '.json') {
                var jsonSrc = __readFile(resolved);
                var parsed = JSON.parse(jsonSrc);
                globalThis.__loadedModules[resolved] = { id: resolved, filename: resolved, exports: parsed, children: [], parent: null, loaded: true };
                return parsed;
            }

            if (resolved.slice(-5) === '.node') {
                return __napiLoad(resolved);
            }

            var moduleDir = resolved.replace(/[\/\\][^\/\\]*$/, '') || '.';
            var mod = { id: resolved, filename: resolved, path: moduleDir, exports: {}, children: [], parent: null, loaded: false };
            globalThis.__loadedModules[resolved] = mod;

            var localRequire = function(id) { return requireFrom(id, moduleDir); };
            localRequire.resolve = function(id) { return __requireResolve(id, moduleDir); };
            localRequire.cache = globalThis.__loadedModules;
            localRequire.extensions = globalThis.require.extensions || {};
            // Backs this module's own transpiled dynamic `import(x)` calls —
            // must resolve relative to *this* module's directory via
            // localRequire, not the entry script's, same as require() above.
            var localImportAsync = __makeImportAsync(localRequire);

            // `3va test --coverage` installs instrumented copies of the project's sources.
            var source = (globalThis.__3va_covSources && globalThis.__3va_covSources[resolved]) || __readFile(resolved);
            // The ESM marker is metadata for the loader, not code: take it out
            // before compiling so the columns of the first line are the ones
            // of the transpiled source — and of its source map.
            var __isEsm = source.indexOf('/*__3va_esm__*/') === 0;
            if (__isEsm) source = source.slice(16);
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
                var __vvva_import_meta__ = {url: ownMetaUrl, env: globalThis.__vvva_meta_env__, hot: undefined, glob: globalThis.__vvva_meta_glob__, resolve: globalThis.__vvva_meta_resolve__, require: localRequire, dirname: moduleDir, filename: resolved, main: false};
                // Compiled natively, with the file's name in the script origin:
                // a stack trace then points at the file with the right line
                // and column, as in Node. A file that is not plain JavaScript
                // (JSX, Flow, …) is transpiled and compiled again.
                var fn;
                try {
                    fn = globalThis.__vvvaCompileModule(
                        source, resolved,
                        'exports,module,require,__filename,__dirname,__vvva_meta_url__,__importAsync,__vvva_import_meta__',
                        -1);
                } catch (e) {
                    fn = globalThis.__vvvaCompileModule(
                        globalThis.__vvvaTranspileJs(source), resolved,
                        'exports,module,require,__filename,__dirname,__vvva_meta_url__,__importAsync,__vvva_import_meta__',
                        -1);
                }
                // ES modules: evaluate static dependencies first (like ESM linking). If one of
                // them is still waiting on a top-level await, this module's body must not run
                // yet: defer it, and expose a promise (mod.__tla) that importers wait on.
                var esmDeps = __isEsm ? __esmTopRequires(source) : [];
                if (esmDeps.length) {
                    __loadingStack.push(mod);
                    try {
                        var prevDepth = globalThis.__vvvaImportDepth || 0;
                        globalThis.__vvvaImportDepth = prevDepth + 1;
                        try { esmDeps.forEach(function(d) { localRequire(d); }); }
                        finally { globalThis.__vvvaImportDepth = prevDepth; }
                    } finally { __loadingStack.pop(); }
                }
                var runBody = function() {
                    __loadingStack.push(mod);
                    try {
                        fn(mod.exports, mod, localRequire, resolved, moduleDir, ownMetaUrl, localImportAsync, __vvva_import_meta__);
                    } finally {
                        __loadingStack.pop();
                    }
                };
                if (mod.__deps && mod.__deps.length) {
                    if (globalThis.__vvvaImportDepth === 0) {
                        throw __requireAsyncModuleError(resolved);
                    }
                    var innerTla, finalTla;
                    Object.defineProperty(mod, '__tla', { configurable: true, get: function() { return finalTla; }, set: function(v) { innerTla = v; } });
                    finalTla = __RealPromise.all(mod.__deps.map(function(d) { return __waitTla(d, [mod]); }))
                        .then(function() { runBody(); return innerTla; })
                        .then(function() { mod.loaded = true; mod.__tlaDone = true; }, function(e) { delete globalThis.__loadedModules[resolved]; throw e; });
                    __noteTlaDep(mod);
                    return mod.exports;
                }
                runBody();
                __noteTlaDep(mod);
                if (mod.__tla) {
                    // Node: require() cannot be used on an ESM graph with
                    // top-level await; import() waits for it instead.
                    if (globalThis.__vvvaImportDepth === 0) {
                        throw __requireAsyncModuleError(resolved);
                    }
                    // Top-level await: run microtask-only awaits to completion so a
                    // static importer sees full exports. Real async work (timers, I/O)
                    // stays pending; importers defer and import() waits on mod.__tla.
                    var tlaState = 0, tlaErr;
                    mod.__tla.then(function() { tlaState = 1; mod.__tlaDone = true; }, function(e) { tlaState = 2; tlaErr = e; });
                    __drainMicrotasks();
                    if (tlaState === 2) throw tlaErr;
                }
                mod.loaded = true;
            } catch (e) {
                delete globalThis.__loadedModules[resolved];
                e.message = (e.message || String(e)) + ' (in ' + resolved + ')';
                throw e;
            }
            return mod.exports;
        }

        function __requireAsyncModuleError(resolved) {
            var err = new Error(
                'require() cannot be used on an ESM graph with top-level await. ' +
                'Use import() instead.\n  From ' + resolved +
                ' \n  Requiring ' + resolved + ' ');
            err.code = 'ERR_REQUIRE_ASYNC_MODULE';
            return err;
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
        globalThis.require.extensions = globalThis.require.extensions || {
            '.js': function(mod, filename) { mod.exports = requireFrom(filename, filename.replace(/[\/\\][^\/\\]*$/, '')); },
            '.json': function(mod, filename) { mod.exports = JSON.parse(__readFile(filename)); },
            '.node': function(mod, filename) { mod.exports = __napiLoad(filename); }
        };

        // Entry-point script's own transpiled dynamic `import(x)` calls —
        // required modules get their own per-directory version instead (see
        // localImportAsync in requireFrom), passed as a shadowing parameter.
        var __entryRequire = function(specifier) {
            return requireFrom(specifier, globalThis.__dirname || undefined);
        };
        __entryRequire.resolve = function(specifier) {
            return __requireResolve(specifier, globalThis.__dirname || undefined);
        };
        globalThis.__importAsync = __makeImportAsync(__entryRequire);

        // Backs native dynamic import() resolved by V8's host callback: the
        // callback passes the importing script's path (its resource name), so a
        // required module's `import('./x')` resolves from *its* directory, not
        // the entry's. The entry-scoped `__importAsync` above stays for scripts
        // whose resource name is not a path.
        globalThis.__importAsyncFrom = function(specifier, fromPath) {
            var dir = '.';
            if (typeof fromPath === 'string' && fromPath.length) {
                var i = Math.max(fromPath.lastIndexOf('/'), fromPath.lastIndexOf('\\'));
                dir = i >= 0 ? fromPath.slice(0, i) : '.';
            } else {
                // No resource name (e.g. `eval`): fall back to the entry dir.
                dir = globalThis.__dirname || '.';
            }
            var req = function(s) { return requireFrom(s, dir); };
            req.resolve = function(s) { return __requireResolve(s, dir); };
            return __makeImportAsync(req)(specifier);
        };

        // Entry file (ES module): load its static imports first; if any is still waiting on a
        // top-level await, return a promise the entry body must wait for (null otherwise).
        globalThis.__esmEntryPreload = function(specs) {
            var entry = {};
            __loadingStack.push(entry);
            var prevDepth = globalThis.__vvvaImportDepth || 0;
            globalThis.__vvvaImportDepth = prevDepth + 1;
            try { specs.forEach(function(sp) { __entryRequire(sp); }); }
            finally {
                globalThis.__vvvaImportDepth = prevDepth;
                __loadingStack.pop();
            }
            if (entry.__deps && entry.__deps.length) {
                return __RealPromise.all(entry.__deps.map(function(d) { return __waitTla(d); }));
            }
            return null;
        };

    })();
    "#;
    crate::builtins::code_cache::bootstrap_js(scope, "require-tail", require_js)?;
    // Per run: whether this process is a forked child is in its environment.
    crate::builtins::code_cache::bootstrap_js_per_run(
        scope,
        "fork-ipc",
        r#"(function() {
        // When this process is a forked child (child_process.fork or cluster.fork),
        // set up process.send and stdin IPC polling immediately — not gated on
        // require('cluster'), so child scripts that never require cluster still get IPC.
        if (process.env && process.env.VVVA_FORKED === '1' && typeof process.send !== 'function') {
            process.send = function(msg) { process.stdout.write(JSON.stringify(msg) + '\n'); };
            process.connected = true;
            var __forkIpcBuf = '';
            var __forkIpcInterval = setInterval(function() {
                try {
                    var chunk = __stdinReadPoll();
                    if (chunk && chunk.length > 0) {
                        __forkIpcBuf += Buffer.from(chunk).toString('utf8');
                        var lines = __forkIpcBuf.split('\n');
                        __forkIpcBuf = lines.pop();
                        for (var i = 0; i < lines.length; i++) {
                            var line = lines[i].trim();
                            if (!line) continue;
                            try { process.emit('message', JSON.parse(line)); } catch(e) {}
                        }
                    }
                } catch(e) {}
            }, 10);
        }
    })();"#,
    )?;

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
    // Node.js package `imports` field: `#specifier` → look up in nearest package.json
    if specifier.starts_with('#') {
        let base = basedir
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        if let Some(resolved) = resolve_package_imports(specifier, &base) {
            return Ok(resolved);
        }
        return Err(format!("package import not resolved: {}", specifier));
    }

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

/// Resolve a `#`-prefixed package-internal import via the `imports` field in
/// the nearest `package.json` ancestor of `start_dir`.
fn resolve_package_imports(specifier: &str, start_dir: &std::path::Path) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let pkg_json = dir.join("package.json");
        // Resolution metadata only: the JSON is parsed for the "imports" map
        // and never returned to the script. The module this resolves to is
        // loaded later through the capability-gated `__readFile`, so this
        // stays ungated like Node's resolver (no --allow-read for import).
        if pkg_json.is_file()
            && let Ok(content) = std::fs::read_to_string(&pkg_json)
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(imports) = json.get("imports")
            && let Some(val) = imports.get(specifier)
            && let Some(s) = crate::esm::resolve_condition(val)
        {
            let resolved = dir.join(s.trim_start_matches("./"));
            if resolved.is_file() {
                return Some(resolved);
            }
        }
        match dir.parent() {
            Some(p) if p != dir => dir = p.to_path_buf(),
            _ => return None,
        }
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
    // Array: try each element in order
    if let Some(arr) = val.as_array() {
        for item in arr {
            if let Some(path) = resolve_exports_value(item, pkg_dir, _is_dir) {
                return Some(path);
            }
        }
        return None;
    }
    // If the value is a conditions object { "require": ..., "import": ..., "default": ... },
    // resolve using the same priority as resolve_condition in esm.rs: require > node > default
    if val.is_object()
        && let Some(s) = crate::esm::resolve_condition(val)
    {
        let path = resolve_path_from(&s, Some(&pkg_dir.to_string_lossy()));
        if path.is_ok() {
            return path.ok();
        }
        // Also try joining with pkg_dir directly
        let joined = pkg_dir.join(s.trim_start_matches("./"));
        if joined.is_file() {
            return Some(joined);
        }
    }
    None
}

/// Substitute the wildcard capture into a JSON value (string or conditions object).
fn substitute_exports_wildcard(
    val: &serde_json::Value,
    capture: &str,
    pkg_dir: &Path,
) -> Option<PathBuf> {
    match val {
        serde_json::Value::String(s) => {
            let s2 = s.replace('*', capture);
            let p = pkg_dir.join(s2.trim_start_matches("./"));
            if p.is_file() {
                Some(p)
            } else {
                let j = PathBuf::from(format!("{}.json", p.display()));
                if j.is_file() { Some(j) } else { Some(p) }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(p) = substitute_exports_wildcard(item, capture, pkg_dir)
                    && p.is_file()
                {
                    return Some(p);
                }
            }
            None
        }
        serde_json::Value::Object(obj) => {
            for key in &["require", "node", "default", "import", "module"] {
                if let Some(v) = obj.get(*key)
                    && let Some(p) = substitute_exports_wildcard(v, capture, pkg_dir)
                {
                    if p.is_file() {
                        return Some(p);
                    }
                    // Return even if not a file (fallback)
                    return Some(p);
                }
            }
            None
        }
        _ => None,
    }
}

/// Node's `patternKeyCompare`: sorts candidate wildcard keys so the most
/// specific one is tried first. `baseLength` is the length of the key up to
/// and including its `*` — a longer literal prefix wins; ties go to the
/// longer overall key (e.g. `"./src/*.js"` beats `"./src/*"` for the same
/// prefix, because the former's literal `.js` suffix is more specific).
/// Without this, iterating `exports` in whatever order `serde_json::Map`
/// happens to give (alphabetical, since it's a `BTreeMap` without the
/// `preserve_order` feature) can match a broader pattern first and — since
/// the old code accepted the first structural match without checking the
/// substituted path actually exists — silently produce a wrong path (e.g.
/// double-appending `.js`) instead of falling through to the correct key.
fn pattern_key_base_len(key: &str) -> Option<usize> {
    key.find('*').map(|i| i + 1)
}

pub fn resolve_exports_pattern(
    exports: &serde_json::Map<String, serde_json::Value>,
    subpath: &str,
    pkg_dir: &Path,
) -> Option<Option<PathBuf>> {
    // Node.js exports wildcard matching: key may contain exactly one '*'
    // e.g. "./drivers/*" with value {"require": "./drivers/*.cjs", "import": "./drivers/*.mjs"}
    let mut candidates: Vec<(&str, &serde_json::Value)> = Vec::new();
    for (key, val) in exports {
        let Some(star) = key.find('*') else { continue };
        let prefix = &key[..star];
        let suffix = &key[star + 1..];
        let n = subpath.len();
        let plen = prefix.len();
        let slen = suffix.len();
        if n < plen + slen {
            continue;
        }
        if !subpath.starts_with(prefix) {
            continue;
        }
        if !suffix.is_empty() && !subpath.ends_with(suffix) {
            continue;
        }
        candidates.push((key, val));
    }
    candidates.sort_by(|(a, _), (b, _)| {
        let base_a = pattern_key_base_len(a).unwrap_or(a.len());
        let base_b = pattern_key_base_len(b).unwrap_or(b.len());
        base_b.cmp(&base_a).then(b.len().cmp(&a.len()))
    });
    for (key, val) in candidates {
        let star = key.find('*').unwrap();
        let prefix = &key[..star];
        let suffix = &key[star + 1..];
        let capture = &subpath[prefix.len()..subpath.len() - suffix.len()];
        if let Some(resolved) = substitute_exports_wildcard(val, capture, pkg_dir) {
            return Some(Some(resolved));
        }
    }
    Some(None)
}
