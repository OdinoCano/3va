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
    #[cfg(target_os = "windows")]
    return windows_mem_total();
    #[allow(unreachable_code)]
    {
        parse_meminfo_kb("MemTotal") * 1024
    }
}

fn mem_free_bytes() -> u64 {
    #[cfg(target_os = "windows")]
    return windows_mem_free();
    #[allow(unreachable_code)]
    {
        parse_meminfo_kb("MemAvailable").max(parse_meminfo_kb("MemFree")) * 1024
    }
}

#[cfg(target_os = "windows")]
fn windows_mem_total() -> u64 {
    windows_memory_status().map(|(total, _)| total).unwrap_or(0)
}

#[cfg(target_os = "windows")]
fn windows_mem_free() -> u64 {
    windows_memory_status().map(|(_, free)| free).unwrap_or(0)
}

#[cfg(target_os = "windows")]
fn windows_memory_status() -> Option<(u64, u64)> {
    #[repr(C)]
    struct MemoryStatusEx {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(lp_buffer: *mut MemoryStatusEx) -> i32;
    }
    let mut s = MemoryStatusEx {
        dw_length: std::mem::size_of::<MemoryStatusEx>() as u32,
        dw_memory_load: 0,
        ull_total_phys: 0,
        ull_avail_phys: 0,
        ull_total_page_file: 0,
        ull_avail_page_file: 0,
        ull_total_virtual: 0,
        ull_avail_virtual: 0,
        ull_avail_extended_virtual: 0,
    };
    if unsafe { GlobalMemoryStatusEx(&mut s) } != 0 {
        Some((s.ull_total_phys, s.ull_avail_phys))
    } else {
        None
    }
}

fn parse_meminfo_kb(_key: &str) -> u64 {
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
        let needle = format!("{}:", _key);
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

/// Returns a JSON array of per-CPU info: [{model, speed, times: {user,nice,sys,idle,irq}}].
/// Reads /proc/cpuinfo (model name + MHz) and /proc/stat (per-CPU jiffies → ms, HZ=100).
fn cpus_info_json() -> String {
    let mut cpus: Vec<serde_json::Value> = Vec::new();

    #[cfg(target_os = "linux")]
    {
        let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();

        // Parse per-CPU model name + MHz. Blocks are separated by blank lines.
        let mut cpu_meta: Vec<(String, u64)> = Vec::new();
        let mut model = String::from("Unknown");
        let mut speed_mhz = 0u64;
        let mut in_block = false;
        for line in cpuinfo.lines() {
            if line.trim().is_empty() {
                if in_block {
                    cpu_meta.push((model.clone(), speed_mhz));
                    model = String::from("Unknown");
                    speed_mhz = 0;
                    in_block = false;
                }
            } else {
                in_block = true;
                if let Some((k, v)) = line.split_once(':') {
                    match k.trim() {
                        "model name" => model = v.trim().to_string(),
                        "cpu MHz" => speed_mhz = v.trim().parse::<f64>().unwrap_or(0.0) as u64,
                        _ => {}
                    }
                }
            }
        }
        if in_block {
            cpu_meta.push((model, speed_mhz));
        }

        // Parse per-CPU times from /proc/stat.
        // Line format: "cpu0 user nice sys idle iowait irq softirq ..."
        // Multiply jiffies by 10 to convert to ms (assuming HZ=100, standard Linux).
        let mut cpu_times: Vec<serde_json::Value> = Vec::new();
        for line in stat.lines() {
            if !line.starts_with("cpu") {
                break;
            }
            let rest = &line[3..];
            // Skip the aggregate "cpu " line (starts with a space after "cpu").
            if rest.is_empty() || rest.starts_with(' ') {
                continue;
            }
            let nums: Vec<u64> = rest
                .split_whitespace()
                .skip(1) // skip the cpu-N index
                .filter_map(|v| v.parse().ok())
                .collect();
            cpu_times.push(serde_json::json!({
                "user": nums.first().copied().unwrap_or(0) * 10,
                "nice": nums.get(1).copied().unwrap_or(0) * 10,
                "sys":  nums.get(2).copied().unwrap_or(0) * 10,
                "idle": nums.get(3).copied().unwrap_or(0) * 10,
                "irq":  nums.get(5).copied().unwrap_or(0) * 10,
            }));
        }

        let n = cpu_meta.len().max(cpu_times.len());
        let dummy_times = serde_json::json!({"user":0,"nice":0,"sys":0,"idle":0,"irq":0});
        for i in 0..n {
            let (m, s) = cpu_meta
                .get(i)
                .cloned()
                .unwrap_or_else(|| ("Unknown".to_string(), 0));
            let t = cpu_times
                .get(i)
                .cloned()
                .unwrap_or_else(|| dummy_times.clone());
            cpus.push(serde_json::json!({ "model": m, "speed": s, "times": t }));
        }
    }

    if cpus.is_empty() {
        let dummy = serde_json::json!({"user":0,"nice":0,"sys":0,"idle":0,"irq":0});
        for _ in 0..cpu_count() {
            cpus.push(serde_json::json!({"model":"Unknown","speed":0,"times":dummy}));
        }
    }

    serde_json::to_string(&cpus).unwrap_or_else(|_| "[]".to_string())
}

#[allow(dead_code)]
fn prefix_to_ipv4_netmask(prefix: u8) -> String {
    let mask: u32 = if prefix == 0 {
        0
    } else if prefix >= 32 {
        u32::MAX
    } else {
        !((1u32 << (32 - prefix)) - 1)
    };
    let b = mask.to_be_bytes();
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

#[allow(dead_code)]
fn prefix_to_ipv6_netmask(prefix: u8) -> String {
    let mut groups = Vec::with_capacity(8);
    let mut remaining = prefix as i32;
    for _ in 0..8 {
        let bits = remaining.clamp(0, 16) as u32;
        let mask: u16 = if bits == 0 {
            0
        } else if bits >= 16 {
            0xffff
        } else {
            (0xffffu32 << (16 - bits)) as u16
        };
        groups.push(format!("{:x}", mask));
        remaining -= 16;
    }
    groups.join(":")
}

#[allow(dead_code)]
fn format_ipv6_hex(hex: &str) -> String {
    if hex.len() != 32 {
        return "::".to_string();
    }
    let groups: Vec<String> = hex
        .as_bytes()
        .chunks(4)
        .map(|c| {
            let s = std::str::from_utf8(c).unwrap_or("0000");
            let trimmed = s.trim_start_matches('0');
            if trimmed.is_empty() { "0" } else { trimmed }.to_string()
        })
        .collect();
    groups.join(":")
}

/// Returns a JSON object mapping interface name → [{address, netmask, family, mac, internal, cidr}].
/// Primary: runs `ip -j addr` (iproute2, available on all modern Linux).
/// Fallback: parses /proc/net/if_inet6 for IPv6-only data.
fn network_interfaces_json() -> String {
    let mut result = serde_json::Map::new();

    #[cfg(target_os = "linux")]
    {
        if let Ok(out) = std::process::Command::new("ip")
            .args(["-j", "addr"])
            .output()
            && out.status.success()
            && let Ok(ifaces) = serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout)
        {
            for iface in &ifaces {
                let name = match iface.get("ifname").and_then(|v| v.as_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let internal = iface
                    .get("flags")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().any(|v| v.as_str() == Some("LOOPBACK")))
                    .unwrap_or(false);
                let mac = std::fs::read_to_string(format!("/sys/class/net/{name}/address"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| "00:00:00:00:00:00".to_string());

                let mut addrs: Vec<serde_json::Value> = Vec::new();
                if let Some(infos) = iface.get("addr_info").and_then(|v| v.as_array()) {
                    for addr in infos {
                        let family = addr.get("family").and_then(|v| v.as_str()).unwrap_or("");
                        let local = match addr.get("local").and_then(|v| v.as_str()) {
                            Some(l) => l.to_string(),
                            None => continue,
                        };
                        let prefix =
                            addr.get("prefixlen").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                        match family {
                            "inet" => addrs.push(serde_json::json!({
                                "address":  local,
                                "netmask":  prefix_to_ipv4_netmask(prefix),
                                "family":   "IPv4",
                                "mac":      mac,
                                "internal": internal,
                                "cidr":     format!("{local}/{prefix}"),
                            })),
                            "inet6" => {
                                let scope_id =
                                    addr.get("scope_id").and_then(|v| v.as_u64()).unwrap_or(0);
                                addrs.push(serde_json::json!({
                                    "address":  local,
                                    "netmask":  prefix_to_ipv6_netmask(prefix),
                                    "family":   "IPv6",
                                    "mac":      mac,
                                    "internal": internal,
                                    "cidr":     format!("{local}/{prefix}"),
                                    "scopeid":  scope_id,
                                }));
                            }
                            _ => {}
                        }
                    }
                }
                if !addrs.is_empty() {
                    result.insert(name, serde_json::Value::Array(addrs));
                }
            }
            return serde_json::to_string(&serde_json::Value::Object(result))
                .unwrap_or_else(|_| "{}".to_string());
        }

        // Fallback: parse /proc/net/if_inet6 for IPv6 addresses.
        // Line format: "addr32hex ifindex prefixlen(hex) scope(hex) flags(hex) ifname"
        if let Ok(content) = std::fs::read_to_string("/proc/net/if_inet6") {
            for line in content.lines() {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() < 6 {
                    continue;
                }
                let prefix = u8::from_str_radix(fields[2], 16).unwrap_or(0);
                let scope = u8::from_str_radix(fields[3], 16).unwrap_or(0);
                let iface_name = fields[5];
                let addr = format_ipv6_hex(fields[0]);
                let mac = std::fs::read_to_string(format!("/sys/class/net/{iface_name}/address"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| "00:00:00:00:00:00".to_string());
                let entry = serde_json::json!({
                    "address":  addr,
                    "netmask":  prefix_to_ipv6_netmask(prefix),
                    "family":   "IPv6",
                    "mac":      mac,
                    "internal": scope == 0x10,
                    "cidr":     format!("{addr}/{prefix}"),
                    "scopeid":  scope as u64,
                });
                result
                    .entry(iface_name.to_string())
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()))
                    .as_array_mut()
                    .unwrap()
                    .push(entry);
            }
        }
    }

    serde_json::to_string(&serde_json::Value::Object(result)).unwrap_or_else(|_| "{}".to_string())
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
    process.set("version", "3va/2.0.0")?;
    process.set("pid", std::process::id())?;

    let versions = Object::new(ctx.clone())?;
    versions.set("3va", "2.0.0")?;
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
    globals.set("__osCpusInfo", Function::new(ctx.clone(), cpus_info_json)?)?;
    globals.set(
        "__osNetworkInterfaces",
        Function::new(ctx.clone(), network_interfaces_json)?,
    )?;
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

            // nextTick: drained by __drainNextTick() called from the Rust event loop
            // BEFORE Promise microtasks, matching Node.js semantics.
            var _nextTickQueue = [];
            process.nextTick = function(cb) {
                if (typeof cb !== 'function') throw new TypeError('callback is not a function');
                _nextTickQueue.push({ fn: cb, args: Array.prototype.slice.call(arguments, 1) });
            };
            globalThis.__drainNextTick = function() {
                if (_nextTickQueue.length === 0) return;
                var queue = _nextTickQueue.splice(0);
                for (var i = 0; i < queue.length; i++) {
                    try { queue[i].fn.apply(null, queue[i].args); } catch(e) {}
                }
                // Drain any nested nextTick calls made during the drain
                if (_nextTickQueue.length > 0) globalThis.__drainNextTick();
            };

            // setImmediate / clearImmediate — real queue drained by __drainImmediate
            // in the Rust event loop, NOT via setTimeout(0).
            // Unconditionally override any previous timer-based setImmediate.
            var _immediateQueue = [];
            var _immediateId = 0;
            globalThis.setImmediate = function(cb) {
                if (typeof cb !== 'function') throw new TypeError('callback is not a function');
                var args = Array.prototype.slice.call(arguments, 1);
                var id = ++_immediateId;
                _immediateQueue.push({ id: id, fn: cb, args: args });
                return id;
            };
            globalThis.clearImmediate = function(id) {
                _immediateQueue = _immediateQueue.filter(function(item) { return item.id !== id; });
            };
            globalThis.__drainImmediate = function() {
                if (_immediateQueue.length === 0) return;
                var queue = _immediateQueue.splice(0);
                for (var i = 0; i < queue.length; i++) {
                    try { queue[i].fn.apply(null, queue[i].args); } catch(e) {}
                }
                // Drain any nested setImmediate calls
                if (_immediateQueue.length > 0) globalThis.__drainImmediate();
            };

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

            // process.emitWarning — used by Node.js built-ins and drivers (e.g. MongoDB)
            var _warnListeners = [];
            process.emitWarning = function(message, options) {
                var code = (options && typeof options === 'object') ? options.code : options;
                var w = new Error(typeof message === 'string' ? message : String(message));
                w.name = 'Warning';
                if (code) w.code = code;
                if (typeof process.emit === 'function') process.emit('warning', w);
                return w;
            };

            // process.dlopen — used by Prisma and other NAPI loaders instead of require()
            // Mirrors Node.js: process.dlopen(module, filename[, flags])
            // After the call, module.exports contains the native addon's exports.
            process.dlopen = function(mod, filename, flags) {
                if (typeof globalThis.__napiRequire !== 'function') {
                    throw new Error('process.dlopen requires --allow-ffi');
                }
                var exports = globalThis.__napiRequire(filename);
                if (mod && typeof mod === 'object') {
                    mod.exports = exports || {};
                }
            };
        }());"#,
    )?;

    Ok(())
}
