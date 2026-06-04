pub mod buffer;
pub mod child_process;
pub mod console;
pub mod crypto;
pub mod dgram;
pub mod fetch;
pub mod ffi;
pub mod fs;
pub mod http_server;
pub mod modules;
pub mod napi;
pub mod process;
pub mod tcp;
pub mod timers;
pub mod websocket;
pub mod worker_threads;
pub mod zlib;

use rquickjs::Ctx;
use std::sync::Arc;
use vvva_permissions::PermissionState;

pub use timers::TimerManager;

pub fn inject_all(
    ctx: &Ctx,
    permissions: Arc<PermissionState>,
    timer_manager: Arc<TimerManager>,
) -> rquickjs::Result<()> {
    console::inject_console(ctx)?;
    timers::inject_timers(ctx, timer_manager)?;

    // atob / btoa polyfills — must be injected before buffer.rs, crypto.rs, and
    // any user code that calls Buffer.from(str, 'base64') or WebCrypto JWK imports.
    // QuickJS does not expose these as globals by default.
    ctx.eval::<(), _>(
        r#"
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
    }());
    "#,
    )?;

    buffer::inject_buffer(ctx)?;
    process::inject_process(ctx, permissions.clone())?;
    // Node.js packages expect `global` and `globalThis` to be the same object.
    ctx.eval::<(), _>("globalThis.global = globalThis; globalThis.GLOBAL = globalThis;")?;
    fetch::inject_fetch(ctx, permissions.clone())?;
    fs::inject_fs(ctx, permissions.clone())?;
    tcp::inject_tcp(ctx, permissions.clone())?;
    http_server::inject_http_server(ctx, permissions.clone())?;
    modules::inject_require(ctx, permissions.clone())?;
    websocket::inject_websocket(ctx, permissions.clone())?;
    // These run after inject_require so they can overwrite the placeholder stubs
    zlib::inject_zlib(ctx)?;
    child_process::inject_child_process(ctx, permissions.clone())?;
    crypto::inject_crypto(ctx)?;
    ffi::inject_ffi(ctx, permissions.clone())?;
    napi::inject_napi(ctx, permissions.clone())?;
    // Must run after inject_require so the worker_threads stub is already loaded.
    worker_threads::inject_worker_threads_native(ctx, permissions.clone())?;
    // dgram (UDP) — after inject_require so it populates __requireCache['dgram'].
    dgram::inject_dgram(ctx, permissions)?;
    Ok(())
}
