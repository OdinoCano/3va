use std::sync::Arc;
use v8::{ContextScope, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue};
use vvva_permissions::{Capability, PermissionState};

fn set_fn(
    scope: &mut ContextScope<HandleScope>,
    obj: v8::Local<v8::Object>,
    name: &str,
    f: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let func = v8::Function::new(scope, f).unwrap();
    let key = v8::String::new(scope, name).unwrap().into();
    obj.set(scope, key, func.into());
}

/// Chunks read from OS stdin by the background reader thread. `None` marks
/// EOF (the thread stops after pushing it); `Some(bytes)` is real data.
struct StdinState {
    queue: std::sync::Mutex<std::collections::VecDeque<Vec<u8>>>,
    started: std::sync::atomic::AtomicBool,
    // Separate from the queue, and sticky: once the reader thread hits EOF it
    // exits for good (POSIX stdin doesn't "un-EOF"), so every poll from then
    // on — even from a different JsEngine/test in the same process — must
    // keep reporting EOF immediately. If EOF were only a one-shot item
    // pushed into the queue, the first caller to drain it would leave every
    // later caller polling a permanently-empty queue forever (this was a
    // real hang: two tests using process.stdin in the same test binary).
    eof: std::sync::atomic::AtomicBool,
}
fn stdin_state() -> &'static StdinState {
    static STATE: std::sync::OnceLock<StdinState> = std::sync::OnceLock::new();
    STATE.get_or_init(|| StdinState {
        queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
        started: std::sync::atomic::AtomicBool::new(false),
        eof: std::sync::atomic::AtomicBool::new(false),
    })
}
/// Spawns the (single, process-wide) blocking stdin reader thread on first
/// call; a no-op afterwards. stdin is one shared OS resource regardless of
/// how many JsEngines exist, so a single background reader is correct even
/// across multiple engines/tests in the same process.
fn ensure_stdin_thread() {
    let state = stdin_state();
    if state
        .started
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    std::thread::spawn(|| {
        use std::io::Read;
        loop {
            let mut buf = vec![0u8; 4096];
            match std::io::stdin().lock().read(&mut buf) {
                Ok(0) | Err(_) => {
                    stdin_state()
                        .eof
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    break;
                }
                Ok(n) => {
                    buf.truncate(n);
                    stdin_state().queue.lock().unwrap().push_back(buf);
                }
            }
        }
    });
}

fn set_str(
    scope: &mut ContextScope<HandleScope>,
    obj: v8::Local<v8::Object>,
    name: &str,
    value: &str,
) {
    let key = v8::String::new(scope, name).unwrap().into();
    let val = v8::String::new(scope, value).unwrap().into();
    obj.set(scope, key, val);
}

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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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
    #[allow(unused_mut)] // mut is needed inside #[cfg(target_os = "linux")] blocks
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

fn os_version() -> String {
    #[cfg(target_os = "linux")]
    if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/version") {
        return s.trim().to_string();
    }
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("uname").arg("-v").output() {
        if let Ok(v) = String::from_utf8(out.stdout) {
            return v.trim().to_string();
        }
    }
    String::new()
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

pub fn inject_process(
    scope: &mut ContextScope<HandleScope>,
    permissions: Arc<PermissionState>,
) -> anyhow::Result<()> {
    let permissions: &'static Arc<PermissionState> = Box::leak(Box::new(permissions));
    let context = scope.get_current_context();
    let globals = context.global(scope);

    // --- process.exit(code?) ---
    set_fn(
        scope,
        globals,
        "__processExit",
        |_scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let code = args.get(0).int32_value(_scope).unwrap_or(0);
            std::process::exit(code);
        },
    );

    // --- __isatty(fd) — real TTY detection via std::io::IsTerminal ---
    set_fn(
        scope,
        globals,
        "__isatty",
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            use std::io::IsTerminal;
            let fd = args.get(0).int32_value(scope).unwrap_or(0);
            let is_tty = match fd {
                0 => std::io::stdin().is_terminal(),
                1 => std::io::stdout().is_terminal(),
                2 => std::io::stderr().is_terminal(),
                _ => false,
            };
            rv.set(v8::Boolean::new(scope, is_tty).into());
        },
    );

    // --- __stdinReadPoll() -> Uint8Array | null — non-blocking poll of the
    // next chunk read from OS stdin. A real (blocking) OS read only ever
    // happens on a dedicated background thread (spawned lazily, once), so
    // polling this from a V8 callback never blocks the engine — waiting for
    // real interactive input would otherwise freeze all JS execution. Returns
    // null while no chunk is ready yet, an empty Uint8Array at EOF, or the
    // chunk's bytes. The JS-level `__stdinRead()` (see readline's IIFE below)
    // wraps this in a Promise via setInterval, same pattern as fs_watch's
    // __fsWatchNext / dgram's __udpRecv.
    set_fn(
        scope,
        globals,
        "__stdinReadPoll",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            ensure_stdin_thread();
            let popped = stdin_state().queue.lock().unwrap().pop_front();
            match popped {
                Some(data) => {
                    rv.set(crate::builtins::v8_compat::uint8array_from_bytes(scope, &data).into())
                }
                None if stdin_state().eof.load(std::sync::atomic::Ordering::SeqCst) => {
                    rv.set(crate::builtins::v8_compat::uint8array_from_bytes(scope, &[]).into())
                }
                None => rv.set(v8::null(scope).into()),
            }
        },
    );
    {
        let src = v8::String::new(
            scope,
            r#"(function() {
                globalThis.__stdinRead = function() {
                    return new Promise(function(resolve) {
                        (function check() {
                            var r = __stdinReadPoll();
                            if (r === null || r === undefined) { setTimeout(check, 5); return; }
                            resolve(r);
                        })();
                    });
                };
            })();"#,
        )
        .unwrap();
        let _ = v8::Script::compile(scope, src, None).and_then(|s| s.run(scope));
    }

    // --- process object built via native Rust APIs (no format-string injection risk) ---
    let process = v8::Object::new(scope);

    // Strings and numbers
    set_str(scope, process, "version", "3va/2.4.0");
    {
        let key = v8::String::new(scope, "pid").unwrap().into();
        let val = v8::Integer::new_from_unsigned(scope, std::process::id()).into();
        process.set(scope, key, val);
    }

    let versions = v8::Object::new(scope);
    set_str(scope, versions, "3va", "2.4.0");
    // Expose fake Node.js-compatible version strings so packages checking
    // process.versions.node / process.versions.v8 don't crash.
    set_str(scope, versions, "node", "20.0.0");
    set_str(scope, versions, "v8", "11.3.244.8-node.20");
    set_str(scope, versions, "uv", "1.44.2");
    set_str(scope, versions, "zlib", "1.2.13");
    set_str(scope, versions, "openssl", "3.0.0");
    set_str(scope, versions, "modules", "115");
    {
        let key = v8::String::new(scope, "versions").unwrap().into();
        process.set(scope, key, versions.into());
    }

    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "unknown"
    };
    set_str(scope, process, "platform", platform);

    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    };
    set_str(scope, process, "arch", arch);

    // argv[0] = path to runtime binary (Node-compatible convention).
    // argv[1] = script path and argv[2+] = script args are set by eval_file/eval_file_with_args.
    let argv = v8::Array::new(scope, 1);
    let bin = std::env::args().next().unwrap_or_else(|| "3va".to_string());
    {
        let bin_val = v8::String::new(scope, &bin).unwrap().into();
        argv.set_index(scope, 0, bin_val);
    }
    {
        let key = v8::String::new(scope, "argv").unwrap().into();
        process.set(scope, key, argv.into());
    }

    // exit(): delegate to the native __processExit binding
    {
        let src = v8::String::new(scope, "globalThis.__processExit = __processExit;").unwrap();
        let script = v8::Script::compile(scope, src, None).unwrap();
        let _ = script.run(scope);
    }
    set_fn(
        scope,
        process,
        "exit",
        |_scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let code = args.get(0).int32_value(_scope).unwrap_or(0);
            std::process::exit(code);
        },
    );

    // cwd(): return the process working directory (dynamic, reflects chdir)
    set_fn(
        scope,
        process,
        "cwd",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let cwd = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("/"))
                .to_string_lossy()
                .to_string();
            rv.set(v8::String::new(scope, &cwd).unwrap().into());
        },
    );

    // chdir(): actually change the working directory
    set_fn(
        scope,
        process,
        "chdir",
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path = args.get(0).to_rust_string_lossy(scope);
            if let Err(e) = std::env::set_current_dir(&path) {
                let msg = v8::String::new(scope, &format!("ENOENT: {e}")).unwrap();
                let err = v8::Exception::error(scope, msg);
                rv.set(err);
            }
        },
    );

    // Native write helpers for stdout/stderr (used by JS Writable streams)
    set_fn(
        scope,
        globals,
        "__stdoutWrite",
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            print!("{}", args.get(0).to_rust_string_lossy(scope));
        },
    );
    set_fn(
        scope,
        globals,
        "__stderrWrite",
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            eprint!("{}", args.get(0).to_rust_string_lossy(scope));
        },
    );

    // Temporary stdout/stderr (replaced by Writable instances in modules.rs)
    let stdout_plain = v8::Object::new(scope);
    set_fn(
        scope,
        stdout_plain,
        "write",
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            print!("{}", args.get(0).to_rust_string_lossy(scope));
        },
    );
    {
        let key = v8::String::new(scope, "fd").unwrap().into();
        let val = v8::Integer::new(scope, 1).into();
        stdout_plain.set(scope, key, val);
    }
    {
        let key = v8::String::new(scope, "isTTY").unwrap().into();
        let val = v8::Boolean::new(scope, false).into();
        stdout_plain.set(scope, key, val);
    }
    {
        let key = v8::String::new(scope, "stdout").unwrap().into();
        process.set(scope, key, stdout_plain.into());
    }

    let stderr_plain = v8::Object::new(scope);
    set_fn(
        scope,
        stderr_plain,
        "write",
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            eprint!("{}", args.get(0).to_rust_string_lossy(scope));
        },
    );
    {
        let key = v8::String::new(scope, "fd").unwrap().into();
        let val = v8::Integer::new(scope, 2).into();
        stderr_plain.set(scope, key, val);
    }
    {
        let key = v8::String::new(scope, "isTTY").unwrap().into();
        let val = v8::Boolean::new(scope, false).into();
        stderr_plain.set(scope, key, val);
    }
    {
        let key = v8::String::new(scope, "stderr").unwrap().into();
        process.set(scope, key, stderr_plain.into());
    }

    // env: expose variables that pass permission check (replaced by Proxy in modules.rs)
    let env_obj = v8::Object::new(scope);
    for (key, val) in std::env::vars() {
        if permissions.check(&Capability::EnvVar(key.clone())) {
            set_str(scope, env_obj, &key, &val);
        }
    }
    {
        let key = v8::String::new(scope, "env").unwrap().into();
        process.set(scope, key, env_obj.into());
    }

    // memoryUsage(): real RSS on Linux
    set_fn(
        scope,
        process,
        "memoryUsage",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::Integer::new_from_unsigned(scope, rss_bytes() as u32).into());
        },
    );

    // Native helpers for os module
    set_fn(
        scope,
        globals,
        "__osHostname",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let name = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "localhost".to_string());
            rv.set(v8::String::new(scope, &name).unwrap().into());
        },
    );
    set_fn(
        scope,
        globals,
        "__osCpuCount",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::Integer::new_from_unsigned(scope, cpu_count()).into());
        },
    );
    set_fn(
        scope,
        globals,
        "__osCpusInfo",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::String::new(scope, &cpus_info_json()).unwrap().into());
        },
    );
    set_fn(
        scope,
        globals,
        "__osNetworkInterfaces",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(
                v8::String::new(scope, &network_interfaces_json())
                    .unwrap()
                    .into(),
            );
        },
    );
    set_fn(
        scope,
        globals,
        "__osMemTotal",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::Number::new(scope, mem_total_bytes() as f64).into());
        },
    );
    set_fn(
        scope,
        globals,
        "__osMemFree",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::Number::new(scope, mem_free_bytes() as f64).into());
        },
    );
    set_fn(
        scope,
        globals,
        "__osUptime",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::Number::new(scope, uptime_secs()).into());
        },
    );
    set_fn(
        scope,
        globals,
        "__osRelease",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::String::new(scope, &os_release()).unwrap().into());
        },
    );
    set_fn(
        scope,
        globals,
        "__osVersion",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::String::new(scope, &os_version()).unwrap().into());
        },
    );
    set_fn(
        scope,
        globals,
        "__osLoadAvg",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let avg = load_avg();
            let arr = v8::Array::new(scope, avg.len() as i32);
            for (i, v) in avg.iter().enumerate() {
                let val = v8::Number::new(scope, *v).into();
                arr.set_index(scope, i as u32, val);
            }
            rv.set(arr.into());
        },
    );

    // cpuUsage(): returns "user,sys" microseconds — JS wrapper parses to {user, system}
    set_fn(
        scope,
        process,
        "cpuUsage",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let (user, sys) = cpu_times_us();
            rv.set(
                v8::String::new(scope, &format!("{user},{sys}"))
                    .unwrap()
                    .into(),
            );
        },
    );

    {
        let key = v8::String::new(scope, "process").unwrap().into();
        globals.set(scope, key, process.into());
        let js_src = "if (globalThis.__requireCache) { globalThis.__requireCache['process'] = globalThis.process; globalThis.__requireCache['node:process'] = globalThis.process; }";
        let source = v8::String::new(scope, js_src).unwrap();
        if let Some(script) = v8::Script::compile(scope, source, None) {
            let _ = script.run(scope);
        }
    }

    // hrtime, nextTick, setImmediate, signal handlers — implemented in JS after process is on globalThis.
    {
        let js_src = r#"(function () {
            var _epoch = Date.now();
            process.hrtime = function (prev) {
                var ms = Date.now() - _epoch;
                var s  = Math.floor(ms / 1000);
                var ns = (ms % 1000) * 1000000;
                if (prev) { s -= prev[0]; ns -= prev[1]; if (ns < 0) { s -= 1; ns += 1000000000; } }
                return [s, ns];
            };
            process.hrtime.bigint = function() { return BigInt(Date.now()) * BigInt(1000000); };

            // nextTick: use Promise.resolve().then() so callbacks run as microtasks,
            // BEFORE timers and I/O — matching Node.js semantics.
            // A sentinel setTimeout(noop, 0) keeps the event loop alive while nextTick
            // callbacks are pending (since microtasks alone don't satisfy has_pending()).
            var _nextTickQueue = [];
            var _nextTickKeepalive = false;
            process.nextTick = function(cb) {
                if (typeof cb !== 'function') throw new TypeError('callback is not a function');
                var args = Array.prototype.slice.call(arguments, 1);
                _nextTickQueue.push({ fn: cb, args: args });
                if (!_nextTickKeepalive) {
                    _nextTickKeepalive = true;
                    // keepalive timer so the event loop doesn't exit before microtasks drain
                    var tid = setTimeout(function() { _nextTickKeepalive = false; }, 0);
                }
                // Schedule drain as a microtask (runs before timers and I/O)
                Promise.resolve().then(function() { globalThis.__drainNextTick(); });
            };
            globalThis.__drainNextTick = function() {
                if (_nextTickQueue.length === 0) return;
                var queue = _nextTickQueue.splice(0);
                for (var i = 0; i < queue.length; i++) {
                    try { queue[i].fn.apply(null, queue[i].args); } catch(e) {}
                }
                if (_nextTickQueue.length > 0) globalThis.__drainNextTick();
            };

            // setImmediate / clearImmediate — backed by setTimeout(fn, 0) so the
            // timer manager tracks it and the event loop continues to run.
            var _immediateMap = {};
            var _immediateId = 0;
            globalThis.setImmediate = function(cb) {
                if (typeof cb !== 'function') throw new TypeError('callback is not a function');
                var args = Array.prototype.slice.call(arguments, 1);
                var id = ++_immediateId;
                var timerId = setTimeout(function() {
                    if (_immediateMap[id]) {
                        delete _immediateMap[id];
                        cb.apply(null, args);
                    }
                }, 0);
                _immediateMap[id] = timerId;
                return id;
            };
            globalThis.clearImmediate = function(id) {
                if (_immediateMap[id]) {
                    clearTimeout(_immediateMap[id]);
                    delete _immediateMap[id];
                }
            };
            // Keep drain stubs for backward compat (Rust event loop calls these)
            globalThis.__drainImmediate = function() {};

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

            // process.resourceUsage() — Node 12.6+ getrusage()-style object
            process.resourceUsage = function() {
                var cpu = process.cpuUsage();
                var mem = process.memoryUsage();
                return {
                    userCPUTime: cpu.user, systemCPUTime: cpu.system,
                    maxRSS: Math.floor(mem.rss / 1024),
                    sharedMemorySize: 0, unsharedDataSize: 0, unsharedStackSize: 0,
                    minorPageFault: 0, majorPageFault: 0, swappedOut: 0,
                    fsRead: 0, fsWrite: 0, voluntaryContextSwitches: 0, involuntaryContextSwitches: 0
                };
            };

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
            // process.getBuiltinModule — Node 22+ API to load built-in modules
            process.getBuiltinModule = function(name) {
                try { return globalThis.require(name); } catch(e) { return undefined; }
            };
        }());"#;
        let source = v8::String::new(scope, js_src).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        let _ = script.run(scope);
    }

    // localStorage backing — reads/writes ~/.local/share/3va/localStorage.json
    // No permission gate: this is the user's own browser-like data store.
    set_fn(
        scope,
        globals,
        "__localStorageRead",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let p = ls_path();
            let content = std::fs::read_to_string(&p).unwrap_or_else(|_| "{}".to_string());
            rv.set(v8::String::new(scope, &content).unwrap().into());
        },
    );
    set_fn(
        scope,
        globals,
        "__localStorageSave",
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let json = args.get(0).to_rust_string_lossy(scope);
            let p = ls_path();
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&p, json.as_bytes());
        },
    );

    Ok(())
}

fn ls_path() -> std::path::PathBuf {
    std::env::var("3VA_LOCALSTORAGE_PATH")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".local/share/3va/localStorage.json"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/3va-localStorage.json"))
}
