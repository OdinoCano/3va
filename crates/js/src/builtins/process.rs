use rquickjs::{Array, Ctx, Function, Object, Result, function::Rest};
use std::sync::Arc;
use vvva_permissions::{Capability, PermissionState};

// Re-export std::ffi::OsString so hostname::get() works without adding a dep.
// We use the `hostname` crate if available, otherwise fall back to /etc/hostname.
mod hostname {
    pub fn get() -> std::io::Result<std::ffi::OsString> {
        #[cfg(unix)]
        {
            let mut buf = vec![0u8; 256];
            let rc = unsafe {
                unsafe extern "C" {
                    fn gethostname(name: *mut u8, len: usize) -> i32;
                }
                gethostname(buf.as_mut_ptr(), buf.len())
            };
            if rc == 0 {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                return Ok(std::ffi::OsString::from(
                    String::from_utf8_lossy(&buf[..end]).into_owned(),
                ));
            }
        }
        std::fs::read_to_string("/etc/hostname")
            .map(|s| std::ffi::OsString::from(s.trim().to_string()))
    }
}

fn mem_total_bytes() -> u64 {
    parse_meminfo_kb("MemTotal") * 1024
}
fn mem_free_bytes() -> u64 {
    parse_meminfo_kb("MemAvailable").max(parse_meminfo_kb("MemFree")) * 1024
}

fn parse_meminfo_kb(key: &str) -> u64 {
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
        let needle = format!("{}:", key);
        for line in s.lines() {
            if line.starts_with(&needle) {
                return line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
        }
    }
    0
}

fn cpu_count() -> u32 {
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string("/proc/cpuinfo") {
        let count = s.lines().filter(|l| l.starts_with("processor")).count();
        if count > 0 {
            return count as u32;
        }
    }
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

fn os_release() -> String {
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/osrelease") {
        return s.trim().to_string();
    }
    #[cfg(target_os = "macos")]
    if let Ok(s) = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
    {
        if let Ok(v) = String::from_utf8(s.stdout) {
            return v.trim().to_string();
        }
    }
    "1.0.0".to_string()
}

fn load_avg() -> Vec<f64> {
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string("/proc/loadavg") {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() >= 3 {
            return parts[..3].iter().filter_map(|v| v.parse().ok()).collect();
        }
    }
    vec![0.0, 0.0, 0.0]
}

fn uptime_secs() -> f64 {
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string("/proc/uptime") {
        return s
            .split_whitespace()
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
    }
    0.0
}

/// Returns resident set size in bytes on Linux/macOS, 0 on other platforms.
fn rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            for line in s.lines() {
                if line.starts_with("VmRSS:") {
                    let kb: u64 = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    return kb * 1024;
                }
            }
        }
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

/// Returns (user_µs, sys_µs) CPU times via /proc/self/stat on Linux, 0 elsewhere.
/// Assumes 100 clock ticks/sec (standard Linux HZ=100).
fn cpu_times_us() -> (u64, u64) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/stat") {
            let fields: Vec<&str> = s.split_whitespace().collect();
            if fields.len() > 15 {
                let utime: u64 = fields[13].parse().unwrap_or(0);
                let stime: u64 = fields[14].parse().unwrap_or(0);
                // 100 ticks/sec → 10_000 µs/tick
                return (utime * 10_000, stime * 10_000);
            }
        }
        (0, 0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        (0, 0)
    }
}

pub fn inject_process(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // --- process.exit(code?) ---
    globals.set(
        "__processExit",
        Function::new(ctx.clone(), |args: Rest<i32>| -> () {
            let code = args.0.into_iter().next().unwrap_or(0);
            std::process::exit(code);
        })?,
    )?;

    // --- process object built via native Rust APIs (no format-string injection risk) ---
    let process = Object::new(ctx.clone())?;

    // Strings and numbers
    process.set("version", "3va/0.1.0")?;
    process.set("pid", std::process::id())?;

    let versions = Object::new(ctx.clone())?;
    versions.set("3va", "0.1.0")?;
    // Expose fake Node.js-compatible version strings so packages checking
    // process.versions.node / process.versions.v8 don't crash.
    versions.set("node", "20.0.0")?;
    versions.set("v8", "11.3.244.8-node.20")?;
    versions.set("uv", "1.44.2")?;
    versions.set("zlib", "1.2.13")?;
    versions.set("openssl", "3.0.0")?;
    versions.set("modules", "115")?;
    process.set("versions", versions)?;

    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "unknown"
    };
    process.set("platform", platform)?;

    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    };
    process.set("arch", arch)?;

    // argv[0] = path to runtime binary (Node-compatible convention).
    // argv[1] = script path and argv[2+] = script args are set by eval_file/eval_file_with_args.
    let argv = Array::new(ctx.clone())?;
    let bin = std::env::args().next().unwrap_or_else(|| "3va".to_string());
    argv.set(0usize, bin)?;
    process.set("argv", argv)?;

    // exit(): delegate to the native __processExit binding
    ctx.eval::<(), _>("globalThis.__processExit = __processExit;")?;
    process.set(
        "exit",
        Function::new(ctx.clone(), |args: Rest<i32>| -> () {
            let code = args.0.into_iter().next().unwrap_or(0);
            std::process::exit(code);
        })?,
    )?;

    // cwd(): return the process working directory
    let cwd_str = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("/"))
        .to_string_lossy()
        .to_string();
    let cwd_str_clone = cwd_str.clone();
    process.set(
        "cwd",
        Function::new(ctx.clone(), move || cwd_str_clone.clone())?,
    )?;

    // chdir(): no-op stub — sandboxed runtime doesn't change working dir
    process.set("chdir", Function::new(ctx.clone(), |_: Rest<String>| {})?)?;

    // Native write helpers for stdout/stderr (used by JS Writable streams)
    globals.set(
        "__stdoutWrite",
        Function::new(ctx.clone(), |msg: String| print!("{msg}"))?,
    )?;
    globals.set(
        "__stderrWrite",
        Function::new(ctx.clone(), |msg: String| eprint!("{msg}"))?,
    )?;

    // Temporary stdout/stderr (replaced by Writable instances in modules.rs)
    let stdout_plain = Object::new(ctx.clone())?;
    stdout_plain.set(
        "write",
        Function::new(ctx.clone(), |msg: String| print!("{msg}"))?,
    )?;
    stdout_plain.set("fd", 1i32)?;
    stdout_plain.set("isTTY", false)?;
    process.set("stdout", stdout_plain)?;

    let stderr_plain = Object::new(ctx.clone())?;
    stderr_plain.set(
        "write",
        Function::new(ctx.clone(), |msg: String| eprint!("{msg}"))?,
    )?;
    stderr_plain.set("fd", 2i32)?;
    stderr_plain.set("isTTY", false)?;
    process.set("stderr", stderr_plain)?;

    // env: expose variables that pass permission check (replaced by Proxy in modules.rs)
    let env_obj = Object::new(ctx.clone())?;
    for (key, val) in std::env::vars() {
        if permissions.check(&Capability::EnvVar(key.clone())) {
            env_obj.set(key, val)?;
        }
    }
    process.set("env", env_obj)?;

    // memoryUsage(): real RSS on Linux
    process.set("memoryUsage", Function::new(ctx.clone(), rss_bytes)?)?;

    // Native helpers for os module
    globals.set(
        "__osHostname",
        Function::new(ctx.clone(), || {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "localhost".to_string())
        })?,
    )?;
    globals.set("__osCpuCount", Function::new(ctx.clone(), cpu_count)?)?;
    globals.set("__osMemTotal", Function::new(ctx.clone(), mem_total_bytes)?)?;
    globals.set("__osMemFree", Function::new(ctx.clone(), mem_free_bytes)?)?;
    globals.set("__osUptime", Function::new(ctx.clone(), uptime_secs)?)?;
    globals.set("__osRelease", Function::new(ctx.clone(), os_release)?)?;
    globals.set("__osLoadAvg", Function::new(ctx.clone(), load_avg)?)?;

    // cpuUsage(): returns "user,sys" microseconds — JS wrapper parses to {user, system}
    process.set(
        "cpuUsage",
        Function::new(ctx.clone(), || -> String {
            let (user, sys) = cpu_times_us();
            format!("{user},{sys}")
        })?,
    )?;

    globals.set("process", process)?;

    // hrtime, nextTick, setImmediate, signal handlers — implemented in JS after process is on globalThis.
    ctx.eval::<(), _>(
        r#"(function () {
            var _epoch = Date.now();
            process.hrtime = function (prev) {
                var ms = Date.now() - _epoch;
                var s  = Math.floor(ms / 1000);
                var ns = (ms % 1000) * 1000000;
                if (prev) { s -= prev[0]; ns -= prev[1]; if (ns < 0) { s -= 1; ns += 1000000000; } }
                return [s, ns];
            };
            process.hrtime.bigint = function() { return BigInt(Date.now()) * BigInt(1000000); };

            // nextTick: schedule callback in a microtask (closest to Node's behaviour in QuickJS)
            var _nextTickQueue = [];
            var _nextTickScheduled = false;
            process.nextTick = function(cb) {
                var args = Array.prototype.slice.call(arguments, 1);
                _nextTickQueue.push({ fn: cb, args: args });
                if (!_nextTickScheduled) {
                    _nextTickScheduled = true;
                    Promise.resolve().then(function() {
                        _nextTickScheduled = false;
                        var queue = _nextTickQueue.splice(0);
                        for (var i = 0; i < queue.length; i++) {
                            try { queue[i].fn.apply(null, queue[i].args); } catch(e) {}
                        }
                    });
                }
            };

            // setImmediate / clearImmediate
            if (typeof setImmediate === 'undefined') {
                globalThis.setImmediate = function(cb) {
                    var args = Array.prototype.slice.call(arguments, 1);
                    return setTimeout(function() { cb.apply(null, args); }, 0);
                };
                globalThis.clearImmediate = clearTimeout;
            }

            // EventEmitter-style signal handling on process
            var _sigListeners = {};
            process._sigListeners = _sigListeners;

            process.on = process.addListener = function(ev, fn) {
                if (!_sigListeners[ev]) _sigListeners[ev] = [];
                _sigListeners[ev].push(fn);
                return process;
            };
            process.once = function(ev, fn) {
                function w() { process.removeListener(ev, w); fn.apply(null, arguments); }
                w._orig = fn;
                return process.on(ev, w);
            };
            process.removeListener = process.off = function(ev, fn) {
                if (!_sigListeners[ev]) return process;
                _sigListeners[ev] = _sigListeners[ev].filter(function(f) { return f !== fn && f._orig !== fn; });
                return process;
            };
            process.removeAllListeners = function(ev) {
                if (ev) { delete _sigListeners[ev]; } else { for (var k in _sigListeners) delete _sigListeners[k]; }
                return process;
            };
            process.emit = function(ev) {
                var args = Array.prototype.slice.call(arguments, 1);
                var fns = (_sigListeners[ev] || []).slice();
                fns.forEach(function(f) { try { f.apply(null, args); } catch(e) {} });
                return fns.length > 0;
            };
            process.listenerCount = function(ev) { return (_sigListeners[ev] || []).length; };
            process.listeners = function(ev) {
                return (_sigListeners[ev] || []).map(function(f) { return f._orig || f; });
            };
            process.rawListeners = function(ev) { return (_sigListeners[ev] || []).slice(); };
            process.eventNames = function() {
                return Object.keys(_sigListeners).filter(function(k) { return _sigListeners[k] && _sigListeners[k].length > 0; });
            };
            process.prependListener = function(ev, fn) {
                if (!_sigListeners[ev]) _sigListeners[ev] = [];
                _sigListeners[ev].unshift(fn);
                return process;
            };
            process.prependOnceListener = function(ev, fn) {
                function w() { process.removeListener(ev, w); fn.apply(null, arguments); }
                w._orig = fn;
                return process.prependListener(ev, w);
            };

            // Wrap native memoryUsage to return a proper object
            var _nativeMemUsage = process.memoryUsage;
            process.memoryUsage = function() {
                var rss = typeof _nativeMemUsage === 'function' ? _nativeMemUsage() : 0;
                return { rss: rss, heapTotal: rss, heapUsed: Math.floor(rss * 0.7), external: 0, arrayBuffers: 0 };
            };
            process.memoryUsage.rss = function() {
                return typeof _nativeMemUsage === 'function' ? _nativeMemUsage() : 0;
            };

            // Wrap native cpuUsage to return a proper object
            var _nativeCpuUsage = process.cpuUsage;
            process.cpuUsage = function(prev) {
                var raw = typeof _nativeCpuUsage === 'function' ? _nativeCpuUsage() : '0,0';
                var parts = typeof raw === 'string' ? raw.split(',') : ['0','0'];
                var user = parseInt(parts[0]) || 0;
                var sys  = parseInt(parts[1]) || 0;
                if (prev) { user -= (prev.user || 0); sys -= (prev.system || 0); }
                return { user: user, system: sys };
            };

            // process.uptime(): seconds since process started
            var _startMs = Date.now();
            process.uptime = function() { return (Date.now() - _startMs) / 1000; };

            // process.title
            if (!process.title) process.title = 'node';

            // process.execPath / process.execArgv
            if (!process.execPath) process.execPath = process.argv && process.argv[0] || '3va';
            if (!process.execArgv) process.execArgv = [];

            // process.abort() — terminate immediately
            process.abort = function() { process.exit(1); };

            // process.kill(pid, signal) — noop for other pids, SIGTERM on self
            process.kill = function(pid, _sig) {
                if (pid === process.pid) process.exit(0);
            };

            // process.send — IPC stub (not in a cluster/fork context)
            process.send = null;
            process.connected = false;
            process.disconnect = function() {};

            // process.binding — legacy native addon bridge, stub
            process.binding = function(name) {
                throw new Error('process.binding(\'' + name + '\') is not supported in 3va');
            };

            // process.report — diagnostic reports stub
            process.report = {
                writeReport: function() { return ''; },
                getReport: function() { return {}; },
                filename: '', directory: '', signal: 'SIGUSR2',
                reportOnFatalError: false, reportOnSignal: false, reportOnUncaughtException: false,
                compact: false,
            };

            // process.allowedNodeEnvironmentFlags
            process.allowedNodeEnvironmentFlags = new Set();

            // process.setUncaughtExceptionCaptureCallback
            var _uncaughtCb = null;
            process.setUncaughtExceptionCaptureCallback = function(fn) { _uncaughtCb = fn; };
            process.hasUncaughtExceptionCaptureCallback = function() { return _uncaughtCb !== null; };
        }());"#,
    )?;

    Ok(())
}
