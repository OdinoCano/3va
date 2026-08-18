pub mod buffer;
pub mod child_process;
pub mod code_cache;
pub mod console;
pub mod crypto;
pub mod dgram;
pub mod event_source;
pub mod fetch;
pub mod ffi;
pub mod fs;
pub mod ftp;
pub mod grpc;
pub mod http_server;
pub mod imap;
pub mod irc;
pub mod modules;
pub mod mqtt;
pub mod napi;
pub mod os_info;
pub mod pop3;
pub mod process;
pub mod source_maps;
pub mod sqlite;
pub mod ssh;
pub mod tcp;
pub mod timers;
pub mod v8_compat;
pub mod vm;
pub mod web_globals;
pub mod webrtc;
pub mod websocket;
pub mod worker_threads;
pub mod zlib;

use std::sync::Arc;
use v8::{ContextScope, HandleScope};
use vvva_firewall::Firewall;
use vvva_permissions::PermissionState;

pub use timers::TimerManager;

pub fn inject_all(
    scope: &mut ContextScope<HandleScope>,
    permissions: Arc<PermissionState>,
    timer_manager: Arc<TimerManager>,
    firewall: Option<Arc<Firewall>>,
    ws_pool: websocket::WsPool,
) -> anyhow::Result<()> {
    let __trace = std::env::var_os("VVVA_STARTUP_TRACE").is_some();
    macro_rules! t {
        ($label:expr, $e:expr) => {{
            let __t = std::time::Instant::now();
            let __r = $e;
            if __trace {
                eprintln!("[inject] {}: {:?}", $label, __t.elapsed());
            }
            __r
        }};
    }

    t!("console", console::inject_console(scope))?;
    t!("timers", timers::inject_timers(scope, timer_manager))?;

    let atob_btoa = r#"
(function() {
    var _b64chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    var _b64map = Object.create(null);
    for (var _i = 0; _i < 64; _i++) _b64map[_b64chars[_i]] = _i;
    _b64map['='] = -1;
    if (typeof globalThis.atob !== 'function') {
        globalThis.atob = function(s) {
            s = String(s).replace(/[\t\n\f\r ]/g, '');
            var out = '', i = 0;
            while (i < s.length) {
                var a = _b64map[s[i++]], b = _b64map[s[i++]];
                var c = _b64map[s[i++]], d = _b64map[s[i++]];
                out += String.fromCharCode((a << 2) | (b >> 4));
                if (c !== -1) out += String.fromCharCode(((b & 0xf) << 4) | (c >> 2));
                if (d !== -1) out += String.fromCharCode(((c & 0x3) << 6) | d);
            }
            return out;
        };
    }
    if (typeof globalThis.btoa !== 'function') {
        globalThis.btoa = function(s) {
            s = String(s);
            var out = '', i = 0, n = s.length;
            while (i < n) {
                var a = s.charCodeAt(i++);
                var b = i < n ? s.charCodeAt(i++) : NaN;
                var c = i < n ? s.charCodeAt(i++) : NaN;
                out += _b64chars[(a >> 2) & 0x3f];
                out += _b64chars[((a << 4) | (isNaN(b) ? 0 : b >> 4)) & 0x3f];
                out += isNaN(b) ? '=' : _b64chars[((b << 2) | (isNaN(c) ? 0 : c >> 6)) & 0x3f];
                out += isNaN(c) ? '=' : _b64chars[c & 0x3f];
            }
            return out;
        };
    }
})();
"#;

    let script = v8::Script::compile(scope, v8::String::new(scope, atob_btoa).unwrap(), None)
        .ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _ = script.run(scope);

    let require_cache_init = "globalThis.__requireCache = globalThis.__requireCache || {}; globalThis.__loadedModules = globalThis.__loadedModules || {}; globalThis.__fallbackModules = globalThis.__fallbackModules || {};";
    let script = v8::Script::compile(
        scope,
        v8::String::new(scope, require_cache_init).unwrap(),
        None,
    )
    .ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _ = script.run(scope);

    t!("buffer", buffer::inject_buffer(scope))?;
    t!(
        "process",
        process::inject_process(scope, permissions.clone())
    )?;

    let global_this_setup = "globalThis.global = globalThis; globalThis.GLOBAL = globalThis;";
    let script = v8::Script::compile(
        scope,
        v8::String::new(scope, global_this_setup).unwrap(),
        None,
    )
    .ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _ = script.run(scope);

    t!("web_globals", web_globals::inject_web_globals(scope))?;
    t!("fetch", fetch::inject_fetch(scope, permissions.clone()))?;
    t!("fs", fs::inject_fs(scope, permissions.clone()))?;
    t!("tcp", tcp::inject_tcp(scope, permissions.clone()))?;
    t!("grpc", grpc::inject_grpc(scope, permissions.clone()))?;
    t!(
        "http_server",
        http_server::inject_http_server(scope, permissions.clone(), firewall)
    )?;
    t!(
        "http2_server",
        http_server::inject_http2_server(scope, permissions.clone())
    )?;
    t!("os_info", os_info::inject_os_info(scope))?;
    t!(
        "require",
        modules::inject_require(scope, permissions.clone())
    )?;
    t!(
        "websocket",
        websocket::inject_websocket(scope, permissions.clone(), ws_pool)
    )?;
    t!("zlib", zlib::inject_zlib(scope))?;
    t!(
        "child_process",
        child_process::inject_child_process(scope, permissions.clone())
    )?;
    t!("crypto", crypto::inject_crypto(scope))?;
    t!("ffi", ffi::inject_ffi(scope, permissions.clone()))?;
    t!("napi", napi::inject_napi(scope, permissions.clone()))?;
    t!("source_maps", source_maps::inject_source_maps(scope))?;
    t!("vm", vm::inject_vm(scope))?;
    t!(
        "worker_threads",
        worker_threads::inject_worker_threads_native(scope, permissions.clone())
    );
    t!("dgram", dgram::inject_dgram(scope, permissions.clone()))?;
    t!("sqlite", sqlite::inject_sqlite(scope))?;
    t!("event_source", event_source::inject_event_source(scope));
    t!("imap", imap::inject_imap(scope, permissions.clone()));
    t!("irc", irc::inject_irc(scope, permissions.clone()));
    t!("ftp", ftp::inject_ftp(scope, permissions.clone()));
    t!("pop3", pop3::inject_pop3(scope, permissions.clone()));
    t!("mqtt", mqtt::inject_mqtt(scope, permissions.clone()));
    t!("ssh", ssh::inject_ssh(scope, permissions.clone()));
    t!("webrtc", webrtc::inject_webrtc(scope, permissions.clone()));

    Ok(())
}
