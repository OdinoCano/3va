pub mod buffer;
pub mod child_process;
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
pub mod pop3;
pub mod process;
pub mod sqlite;
pub mod ssh;
pub mod tcp;
pub mod timers;
pub mod v8_compat;
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
    console::inject_console(scope)?;
    timers::inject_timers(scope, timer_manager)?;

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

    buffer::inject_buffer(scope)?;
    process::inject_process(scope, permissions.clone())?;

    let global_this_setup = "globalThis.global = globalThis; globalThis.GLOBAL = globalThis;";
    let script = v8::Script::compile(
        scope,
        v8::String::new(scope, global_this_setup).unwrap(),
        None,
    )
    .ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _ = script.run(scope);

    fetch::inject_fetch(scope, permissions.clone())?;
    fs::inject_fs(scope, permissions.clone())?;
    tcp::inject_tcp(scope, permissions.clone())?;
    grpc::inject_grpc(scope, permissions.clone())?;
    http_server::inject_http_server(scope, permissions.clone(), firewall)?;
    modules::inject_require(scope, permissions.clone())?;
    websocket::inject_websocket(scope, permissions.clone(), ws_pool)?;
    zlib::inject_zlib(scope)?;
    child_process::inject_child_process(scope, permissions.clone())?;
    crypto::inject_crypto(scope)?;
    ffi::inject_ffi(scope, permissions.clone())?;
    napi::inject_napi(scope, permissions.clone())?;
    worker_threads::inject_worker_threads_native(scope, permissions.clone());
    dgram::inject_dgram(scope, permissions.clone())?;
    sqlite::inject_sqlite(scope)?;
    event_source::inject_event_source(scope);
    imap::inject_imap(scope, permissions.clone());
    irc::inject_irc(scope, permissions.clone());
    ftp::inject_ftp(scope, permissions.clone());
    pop3::inject_pop3(scope, permissions.clone());
    mqtt::inject_mqtt(scope, permissions.clone());
    ssh::inject_ssh(scope, permissions.clone());
    webrtc::inject_webrtc(scope, permissions.clone());

    Ok(())
}
