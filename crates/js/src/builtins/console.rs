use v8::{Function, FunctionCallbackArguments, PinScope, ReturnValue};

pub fn inject_console(scope: &mut PinScope) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    let console_write = Function::new(
        scope,
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let level = args.get(0).to_rust_string_lossy(scope);
            let msg = args.get(1).to_rust_string_lossy(scope);

            match level.as_str() {
                "warn" => eprintln!("[WARN] {msg}"),
                "error" => eprintln!("[ERROR] {msg}"),
                "info" => println!("[INFO] {msg}"),
                "debug" => println!("[DEBUG] {msg}"),
                _ => println!("{msg}"),
            }
            rv.set(v8::undefined(scope).into());
        },
    );

    global.set(
        scope,
        v8::String::new(scope, "__console_write").unwrap().into(),
        console_write.unwrap().into(),
    );

    let js_polyfill = r#"(function () {
        function fmt(args) {
            var out = [];
            for (var i = 0; i < args.length; i++) {
                var a = args[i];
                if (typeof a === 'string') {
                    out.push(a);
                } else if (a === null) {
                    out.push('null');
                } else if (a === undefined) {
                    out.push('undefined');
                } else if (typeof a === 'object' || Array.isArray(a)) {
                    try { out.push(JSON.stringify(a)); } catch (_) { out.push('[object Object]'); }
                } else {
                    out.push(String(a));
                }
            }
            return out.join(' ');
        }
        var _timers = {};
        var _groupDepth = 0;
        function indent() {
            var s = '';
            for (var i = 0; i < _groupDepth; i++) s += '  ';
            return s;
        }
        globalThis.console = {
            log:   function () { __console_write('log',   indent() + fmt(arguments)); },
            info:  function () { __console_write('info',  indent() + fmt(arguments)); },
            warn:  function () { __console_write('warn',  indent() + fmt(arguments)); },
            error: function () { __console_write('error', indent() + fmt(arguments)); },
            debug: function () { __console_write('debug', indent() + fmt(arguments)); },
            trace: function () {
                var msg = fmt(arguments);
                try { throw new Error(); } catch (e) {
                    msg += '\n' + (e.stack || '');
                }
                __console_write('log', 'Trace: ' + msg);
            },
            dir: function (obj) {
                var s;
                try { s = JSON.stringify(obj, null, 2); } catch (_) { s = String(obj); }
                __console_write('log', s);
            },
            table: function (data) {
                try { __console_write('log', JSON.stringify(data, null, 2)); }
                catch (_) { __console_write('log', String(data)); }
            },
            time: function (label) {
                _timers[label !== undefined ? String(label) : 'default'] = Date.now();
            },
            timeEnd: function (label) {
                var key = label !== undefined ? String(label) : 'default';
                var start = _timers[key];
                if (start === undefined) {
                    __console_write('warn', 'Timer \'' + key + '\' does not exist');
                    return;
                }
                __console_write('log', key + ': ' + (Date.now() - start) + 'ms');
                delete _timers[key];
            },
            timeLog: function (label) {
                var key = label !== undefined ? String(label) : 'default';
                var start = _timers[key];
                if (start === undefined) {
                    __console_write('warn', 'Timer \'' + key + '\' does not exist');
                    return;
                }
                __console_write('log', key + ': ' + (Date.now() - start) + 'ms');
            },
            group: function () {
                if (arguments.length) __console_write('log', indent() + fmt(arguments));
                _groupDepth++;
            },
            groupCollapsed: function () {
                if (arguments.length) __console_write('log', indent() + fmt(arguments));
                _groupDepth++;
            },
            groupEnd: function () {
                if (_groupDepth > 0) _groupDepth--;
            },
            count: function (label) {
                var key = label !== undefined ? String(label) : 'default';
                if (!_timers['__count_' + key]) _timers['__count_' + key] = 0;
                _timers['__count_' + key]++;
                __console_write('log', indent() + key + ': ' + _timers['__count_' + key]);
            },
            countReset: function (label) {
                var key = label !== undefined ? String(label) : 'default';
                _timers['__count_' + key] = 0;
            },
            assert: function (condition) {
                if (!condition) {
                    var args = Array.prototype.slice.call(arguments, 1);
                    var msg = args.length ? fmt(args) : 'Assertion failed';
                    __console_write('error', 'Assertion failed: ' + msg);
                }
            },
            clear: function () {},
        };
    }());"#;

    let script = v8::Script::compile(scope, v8::String::new(scope, js_polyfill).unwrap(), None)
        .ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _ = script.run(scope);

    Ok(())
}
