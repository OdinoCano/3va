// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

pub mod buffer;
pub mod child_process;
pub mod code_cache;
pub mod console;
pub mod crypto;
#[cfg(feature = "fips")]
mod crypto_fips;
#[cfg(not(feature = "fips"))]
mod crypto_rc;
pub mod dgram;
pub mod event_source;
pub mod fetch;
pub mod ffi;
pub mod fs;
pub mod ftp;
pub mod grpc;
pub mod http_server;
pub mod imap;
pub mod intl;
pub mod irc;
pub mod modules;
pub mod mqtt;
pub mod napi;
pub mod os_info;
pub mod pop3;
pub mod process;
pub(crate) mod secure_fs;
pub mod source_maps;
pub mod sqlite;
pub mod ssh;
pub mod tcp;
pub mod timers;
pub mod tls;
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

/// Owns the heap allocations backing V8 `External` callback data.
///
/// V8 function callbacks need a raw pointer that stays valid for as long as
/// the callback itself can be invoked (i.e. the engine's lifetime), which
/// native code usually gets via `Box::leak`. But a real `Box::leak` is never
/// freed even after the owning `JsEngine` (and its isolate) are dropped —
/// harmless for a real process, which creates one engine and exits, but it
/// accumulates across the many engines a single test binary creates and
/// drops, which is what trips AddressSanitizer's leak checker in CI.
/// Stashing the boxes here instead ties their lifetime to the `JsEngine`
/// (see its `native_ctx` field) so they're freed once the engine — and every
/// callback that could still dereference them — is gone. The heap address
/// handed to V8 stays stable because moving a `Box` moves only the pointer,
/// not the pointee.
#[derive(Default)]
pub struct NativeCtxRegistry(Vec<Box<dyn std::any::Any>>);

impl NativeCtxRegistry {
    pub fn leak<T: 'static>(&mut self, value: T) -> *mut std::ffi::c_void {
        let mut boxed: Box<T> = Box::new(value);
        let ptr = boxed.as_mut() as *mut T as *mut std::ffi::c_void;
        self.0.push(boxed);
        ptr
    }
}

pub fn inject_all(
    scope: &mut ContextScope<HandleScope>,
    permissions: Arc<PermissionState>,
    timer_manager: Arc<TimerManager>,
    firewall: Option<Arc<Firewall>>,
    ws_pool: websocket::WsPool,
    native_ctx: &mut NativeCtxRegistry,
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

    tls::init()?;
    t!("console", console::inject_console(scope))?;
    t!(
        "timers",
        timers::inject_timers(scope, timer_manager, native_ctx)
    )?;

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

    code_cache::bootstrap_js(scope, "atob-btoa", atob_btoa)?;

    let require_cache_init = "globalThis.__requireCache = globalThis.__requireCache || {}; globalThis.__loadedModules = globalThis.__loadedModules || {}; globalThis.__fallbackModules = globalThis.__fallbackModules || {};";
    code_cache::bootstrap_js(scope, "require-cache-init", require_cache_init)?;

    t!("buffer", buffer::inject_buffer(scope))?;
    t!(
        "process",
        process::inject_process(scope, permissions.clone())
    )?;

    let global_this_setup = "globalThis.global = globalThis; globalThis.GLOBAL = globalThis;";
    code_cache::bootstrap_js(scope, "global-this", global_this_setup)?;

    t!("web_globals", web_globals::inject_web_globals(scope))?;
    t!("intl", intl::inject_intl(scope))?;
    t!("fetch", fetch::inject_fetch(scope, permissions.clone()))?;
    t!("fs", fs::inject_fs(scope, permissions.clone()))?;
    t!("tcp", tcp::inject_tcp(scope, permissions.clone()))?;
    t!("grpc", grpc::inject_grpc(scope, permissions.clone()))?;
    t!(
        "http_server",
        http_server::inject_http_server(scope, permissions.clone(), firewall, native_ctx)
    )?;
    t!(
        "http2_server",
        http_server::inject_http2_server(scope, permissions.clone(), native_ctx)
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
        child_process::inject_child_process(scope, permissions.clone(), native_ctx)
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
    t!("sqlite", sqlite::inject_sqlite(scope, permissions.clone()))?;
    t!(
        "event_source",
        event_source::inject_event_source(scope, permissions.clone())
    );
    // Protocol clients most scripts never touch are installed on first use
    // (require() or their global), not on every start.
    t!("lazy", install_lazy_modules(scope, permissions.clone())?);

    Ok(())
}

/// Modules installed on first access instead of at startup: name, the
/// `__requireCache` keys, and the globals (public and the module's own
/// native `__*` functions) that trigger it.
const LAZY_MODULES: &[(&str, &[&str], &[&str])] = &[
    (
        "imap",
        &["imap", "node:imap"],
        &[
            "imap",
            "__imapAddFlags",
            "__imapAppend",
            "__imapCapability",
            "__imapClose",
            "__imapConnect",
            "__imapCopy",
            "__imapCreate",
            "__imapCreateMailbox",
            "__imapDeleteMailbox",
            "__imapDisconnect",
            "__imapExpunge",
            "__imapFetch",
            "__imapFetchBody",
            "__imapListMailboxes",
            "__imapLogin",
            "__imapLogout",
            "__imapMove",
            "__imapRemoveFlags",
            "__imapRenameMailbox",
            "__imapSearch",
            "__imapSelect",
            "__imapSetFlags",
            "__imapStatus",
            "__imapSubscribe",
        ],
    ),
    (
        "irc",
        &["irc", "node:irc"],
        &[
            "irc",
            "__ircClose",
            "__ircConnect",
            "__ircCreate",
            "__ircRead",
            "__ircSend",
        ],
    ),
    (
        "ftp",
        &["ftp", "node:ftp"],
        &[
            "ftp",
            "__ftpClose",
            "__ftpConnect",
            "__ftpCreate",
            "__ftpDataClose",
            "__ftpDataConnect",
            "__ftpDataRead",
            "__ftpDataWrite",
            "__ftpRead",
            "__ftpSend",
        ],
    ),
    (
        "pop3",
        &["pop3", "node:pop3"],
        &[
            "pop3",
            "__pop3Close",
            "__pop3Connect",
            "__pop3Create",
            "__pop3Read",
            "__pop3Send",
        ],
    ),
    (
        "mqtt",
        &["mqtt", "node:mqtt"],
        &[
            "mqtt",
            "__mqttClose",
            "__mqttConnect",
            "__mqttCreate",
            "__mqttDisconnect",
            "__mqttIsConnected",
            "__mqttRead",
            "__mqttSend",
        ],
    ),
    (
        "ssh",
        &["ssh2", "node:ssh2"],
        &[
            "ssh",
            "__sftpMkdir",
            "__sftpReadFile",
            "__sftpReaddir",
            "__sftpRename",
            "__sftpRmdir",
            "__sftpStat",
            "__sftpUnlink",
            "__sftpWriteFile",
            "__sshClose",
            "__sshConnect",
            "__sshCreate",
            "__sshExec",
            "__sshOpPoll",
            "__sshSftp",
        ],
    ),
    (
        "webrtc",
        &["webrtc"],
        &[
            "RTCPeerConnection",
            "RTCSessionDescription",
            "RTCIceCandidate",
            "RTCDataChannel",
            "__rtcAddIceCandidate",
            "__rtcClosePeerConnection",
            "__rtcCreateAnswer",
            "__rtcCreateDataChannel",
            "__rtcCreateOffer",
            "__rtcCreatePeerConnection",
            "__rtcDataChannelClose",
            "__rtcDataChannelSend",
            "__rtcGetConnectionState",
            "__rtcSetLocalDescription",
            "__rtcSetRemoteDescription",
        ],
    ),
];

/// Isolate slot: what a lazily installed module's injector needs.
struct LazyModules {
    permissions: Arc<PermissionState>,
    native_ctx: NativeCtxRegistry,
}

fn install_lazy_modules(
    scope: &mut ContextScope<HandleScope>,
    permissions: Arc<PermissionState>,
) -> anyhow::Result<()> {
    scope.set_slot(LazyModules {
        permissions,
        native_ctx: NativeCtxRegistry::default(),
    });
    let global = scope.get_current_context().global(scope);
    let f = v8::Function::new(scope, lazy_inject)
        .ok_or_else(|| anyhow::anyhow!("failed to create __lazyInject"))?;
    let key = v8::String::new(scope, "__lazyInject").unwrap();
    global.set(scope, key.into(), f.into());

    let table: Vec<String> = LAZY_MODULES
        .iter()
        .map(|(name, keys, globals)| format!("[{name:?}, {keys:?}, {globals:?}]"))
        .collect();
    // Each trigger is an accessor that removes every trigger of its module,
    // installs the module (which assigns the real values), then reads the
    // real value back. Assigning to a trigger before first use just replaces
    // it, like assigning to the plain property it stands in for.
    let src = format!(
        r#"(function () {{
            var inject = globalThis.__lazyInject;
            delete globalThis.__lazyInject;
            [{table}].forEach(function (m) {{
                var targets = m[1].map(function (k) {{ return [globalThis.__requireCache, k]; }})
                    .concat(m[2].map(function (k) {{ return [globalThis, k]; }}));
                function load() {{
                    targets.forEach(function (t) {{ delete t[0][t[1]]; }});
                    inject(m[0]);
                }}
                targets.forEach(function (t) {{
                    Object.defineProperty(t[0], t[1], {{
                        configurable: true, enumerable: true,
                        get: function () {{ load(); return t[0][t[1]]; }},
                        set: function (v) {{
                            Object.defineProperty(t[0], t[1], {{ value: v, writable: true, configurable: true, enumerable: true }});
                        }},
                    }});
                }});
            }});
        }})();"#,
        table = table.join(",")
    );
    code_cache::bootstrap_js(scope, "lazy-modules", &src)?;
    Ok(())
}

fn lazy_inject(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let name = args.get(0).to_rust_string_lossy(scope);
    let Some(mut lazy) = scope.remove_slot::<LazyModules>() else {
        return;
    };
    let context = scope.get_current_context();
    {
        let scope = &mut v8::ContextScope::new(scope, context);
        let perms = lazy.permissions.clone();
        match name.as_str() {
            "imap" => imap::inject_imap(scope, perms),
            "irc" => irc::inject_irc(scope, perms, &mut lazy.native_ctx),
            "ftp" => ftp::inject_ftp(scope, perms),
            "pop3" => pop3::inject_pop3(scope, perms, &mut lazy.native_ctx),
            "mqtt" => mqtt::inject_mqtt(scope, perms),
            "ssh" => ssh::inject_ssh(scope, perms),
            "webrtc" => webrtc::inject_webrtc(scope, perms),
            _ => {}
        }
        // SSH (russh on ring, curve25519/chacha) and WebRTC (DTLS/SRTP in
        // pure Rust) cannot run on the FIPS module, so the fips build
        // refuses them.
        if tls::FIPS && (name == "ssh" || name == "webrtc") {
            let deny = "globalThis.__sshCreate = globalThis.__rtcCreatePeerConnection = function () { \
                throw new Error('ERR_CRYPTO_FIPS_FORCED: SSH and WebRTC are unavailable in the FIPS build of 3va'); };";
            let source = v8::String::new(scope, deny).unwrap();
            if let Some(script) = v8::Script::compile(scope, source, None) {
                let _ = script.run(scope);
            }
        }
    }
    scope.set_slot(lazy);
}

/// A port number from JS as `u16`. Out-of-range values become 0 (which the OS
/// refuses) instead of wrapping onto another port: `70000 as u16` is 4464, a
/// port the permission check never saw (VULN-17).
pub(crate) fn port_from_js(v: Option<u32>, default: u16) -> u16 {
    v.map_or(default, |p| u16::try_from(p).unwrap_or(0))
}

#[cfg(test)]
mod port_tests {
    #[test]
    fn out_of_range_ports_do_not_wrap() {
        assert_eq!(super::port_from_js(Some(70000), 80), 0);
        assert_eq!(super::port_from_js(Some(443), 80), 443);
        assert_eq!(super::port_from_js(None, 80), 80);
    }
}
