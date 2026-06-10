use rquickjs::{Ctx, Function, Result};

pub fn inject_console(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();

    // Single native sink: level ("log"|"info"|"warn"|"error"|"debug") + formatted message.
    // All multi-arg formatting and type coercion happens in the JS polyfill below.
    globals.set(
        "__console_write",
        Function::new(ctx.clone(), |level: String, msg: String| {
            match level.as_str() {
                "warn" => eprintln!("[WARN] {msg}"),
                "error" => eprintln!("[ERROR] {msg}"),
                "info" => println!("[INFO] {msg}"),
                "debug" => println!("[DEBUG] {msg}"),
                _ => println!("{msg}"),
            }
        })?,
    )?;

    // JS polyfill: handles variadic args, type coercion, and object serialization exactly
    // as Node.js console does (strings pass through, everything else gets JSON.stringify).
    ctx.eval::<(), _>(
        r#"(function () {
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
        }());"#,
    )?;

    Ok(())
}
