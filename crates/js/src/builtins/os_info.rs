use v8::{ContextScope, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue};

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

// `libc::gethostname` is POSIX-only — the `libc` crate doesn't bind it on
// Windows at all (it's not a winsock function), so a single shared
// implementation doesn't compile there. Windows always sets COMPUTERNAME
// for the current session, so there's no need for a raw FFI call at all.
#[cfg(unix)]
fn hostname() -> String {
    unsafe {
        let mut buf = [0u8; 256];
        if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return std::string::String::from_utf8_lossy(&buf[..end]).into_owned();
        }
    }
    "localhost".to_string()
}

#[cfg(windows)]
fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".to_string())
}

#[cfg(target_os = "linux")]
fn meminfo() -> (u64, u64) {
    unsafe {
        let mut info: libc::sysinfo = std::mem::zeroed();
        if libc::sysinfo(&mut info) == 0 {
            let unit = info.mem_unit as u64;
            return (info.totalram as u64 * unit, info.freeram as u64 * unit);
        }
    }
    (0, 0)
}
#[cfg(not(target_os = "linux"))]
fn meminfo() -> (u64, u64) {
    // ponytail: only /proc-backed Linux wired for real numbers; add a
    // sysinfo-crate-based path here if macOS/Windows values are needed.
    (0, 0)
}

#[cfg(target_os = "linux")]
fn uptime_secs() -> f64 {
    unsafe {
        let mut info: libc::sysinfo = std::mem::zeroed();
        if libc::sysinfo(&mut info) == 0 {
            return info.uptime as f64;
        }
    }
    0.0
}
#[cfg(not(target_os = "linux"))]
fn uptime_secs() -> f64 {
    0.0
}

struct CpuInfo {
    model: String,
    speed_mhz: f64,
    user_ms: u64,
    nice_ms: u64,
    sys_ms: u64,
    idle_ms: u64,
    irq_ms: u64,
}

#[cfg(target_os = "linux")]
fn cpus() -> Vec<CpuInfo> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let mut models = Vec::new();
    let mut speeds = Vec::new();
    let mut cur_model = std::string::String::new();
    let mut cur_speed = 0.0f64;
    for line in cpuinfo.lines() {
        if let Some(v) = line.strip_prefix("model name") {
            cur_model = v
                .split_once(':')
                .map(|x| x.1)
                .unwrap_or("")
                .trim()
                .to_string();
        } else if let Some(v) = line.strip_prefix("cpu MHz") {
            cur_speed = v
                .split_once(':')
                .map(|x| x.1)
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0.0);
        } else if line.trim().is_empty() && !cur_model.is_empty() {
            models.push(cur_model.clone());
            speeds.push(cur_speed);
            cur_model.clear();
        }
    }
    if models.is_empty() {
        models.push("unknown".to_string());
        speeds.push(0.0);
    }

    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    let mut times = Vec::new();
    for line in stat.lines() {
        if !line.starts_with("cpu") {
            continue;
        }
        let rest = &line[3..];
        if !rest.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let fields: Vec<u64> = rest
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if fields.len() >= 4 {
            let to_ms = |ticks: u64| (ticks as f64 / clk_tck * 1000.0) as u64;
            times.push((
                to_ms(fields[0]),
                to_ms(fields.get(1).copied().unwrap_or(0)),
                to_ms(fields[2]),
                to_ms(fields[3]),
                to_ms(fields.get(5).copied().unwrap_or(0)),
            ));
        }
    }
    if times.is_empty() {
        times.push((0, 0, 0, 0, 0));
    }

    let n = models.len().max(times.len());
    (0..n)
        .map(|i| {
            let (user, nice, sys, idle, irq) = times[i % times.len()];
            CpuInfo {
                model: models[i % models.len()].clone(),
                speed_mhz: speeds[i % speeds.len()],
                user_ms: user,
                nice_ms: nice,
                sys_ms: sys,
                idle_ms: idle,
                irq_ms: irq,
            }
        })
        .collect()
}
#[cfg(not(target_os = "linux"))]
fn cpus() -> Vec<CpuInfo> {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (0..n)
        .map(|_| CpuInfo {
            model: "unknown".to_string(),
            speed_mhz: 0.0,
            user_ms: 0,
            nice_ms: 0,
            sys_ms: 0,
            idle_ms: 0,
            irq_ms: 0,
        })
        .collect()
}

#[cfg(unix)]
fn network_interfaces_json() -> std::string::String {
    use std::collections::BTreeMap;
    let mut byname: BTreeMap<std::string::String, Vec<std::string::String>> = BTreeMap::new();
    unsafe {
        let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut addrs) != 0 {
            return "{}".to_string();
        }
        let mut cur = addrs;
        while !cur.is_null() {
            let ifa = &*cur;
            if !ifa.ifa_addr.is_null() {
                let family = (*ifa.ifa_addr).sa_family as i32;
                let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                    .to_string_lossy()
                    .into_owned();
                let internal = (ifa.ifa_flags & (libc::IFF_LOOPBACK as u32)) != 0;

                if family == libc::AF_INET {
                    let sin = ifa.ifa_addr as *const libc::sockaddr_in;
                    let ip = std::net::Ipv4Addr::from((*sin).sin_addr.s_addr.to_be());
                    let entry = format!(
                        "{{\"address\":\"{}\",\"family\":\"IPv4\",\"internal\":{}}}",
                        ip, internal
                    );
                    byname.entry(name).or_default().push(entry);
                } else if family == libc::AF_INET6 {
                    let sin6 = ifa.ifa_addr as *const libc::sockaddr_in6;
                    let ip = std::net::Ipv6Addr::from((*sin6).sin6_addr.s6_addr);
                    let entry = format!(
                        "{{\"address\":\"{}\",\"family\":\"IPv6\",\"internal\":{}}}",
                        ip, internal
                    );
                    byname.entry(name).or_default().push(entry);
                }
            }
            cur = ifa.ifa_next;
        }
        libc::freeifaddrs(addrs);
    }
    let body: Vec<std::string::String> = byname
        .into_iter()
        .map(|(name, entries)| format!("\"{}\":[{}]", name.replace('"', ""), entries.join(",")))
        .collect();
    format!("{{{}}}", body.join(","))
}
#[cfg(not(unix))]
fn network_interfaces_json() -> std::string::String {
    "{}".to_string()
}

pub fn inject_os_info(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    set_fn(
        scope,
        global,
        "__osHostname",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let s = v8::String::new(scope, &hostname()).unwrap();
            rv.set(s.into());
        },
    );

    set_fn(
        scope,
        global,
        "__osTotalMem",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let (total, _) = meminfo();
            rv.set(v8::Number::new(scope, total as f64).into());
        },
    );

    set_fn(
        scope,
        global,
        "__osFreeMem",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let (_, free) = meminfo();
            rv.set(v8::Number::new(scope, free as f64).into());
        },
    );

    set_fn(
        scope,
        global,
        "__osUptime",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            rv.set(v8::Number::new(scope, uptime_secs()).into());
        },
    );

    set_fn(
        scope,
        global,
        "__osCpus",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let list = cpus();
            let json = serde_json::to_string(
                &list
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "model": c.model,
                            "speed": c.speed_mhz,
                            "times": {
                                "user": c.user_ms,
                                "nice": c.nice_ms,
                                "sys": c.sys_ms,
                                "idle": c.idle_ms,
                                "irq": c.irq_ms,
                            }
                        })
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_else(|_| "[]".to_string());
            let s = v8::String::new(scope, &json).unwrap();
            rv.set(s.into());
        },
    );

    set_fn(
        scope,
        global,
        "__osNetworkInterfaces",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let json = network_interfaces_json();
            let s = v8::String::new(scope, &json).unwrap();
            rv.set(s.into());
        },
    );

    set_fn(
        scope,
        global,
        "__osPlatform",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let platform = match std::env::consts::OS {
                "macos" => "darwin",
                "windows" => "win32",
                other => other,
            };
            let s = v8::String::new(scope, platform).unwrap();
            rv.set(s.into());
        },
    );

    set_fn(
        scope,
        global,
        "__osArch",
        |scope: &mut PinScope, _args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let arch = match std::env::consts::ARCH {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                other => other,
            };
            let s = v8::String::new(scope, arch).unwrap();
            rv.set(s.into());
        },
    );

    Ok(())
}
