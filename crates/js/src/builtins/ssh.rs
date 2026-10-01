// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! SSH/SFTP client built-in module
//!
//! Provides: `require('ssh2')` with `Client` class, backed by real SSH via
//! `russh` (client protocol, password auth, exec channels) and `russh-sftp`
//! (SFTP subsystem: readdir/open/mkdir/rmdir/unlink/rename/stat/read/write).
//!
//! Native functions:
//! - `__sshCreate()` -> id
//! - `__sshConnect(id, host, port, username, password, hostFingerprint, knownHosts, hasHostVerifier)` -> Promise<envelope>
//! - `__sshVerifyPoll(opId)` -> `{verificationId, fingerprint}` | null
//! - `__sshVerifyReply(verificationId, accept)` -> bool
//! - `__sshExec(id, command)` -> Promise<envelope {stdout, stderr, code}>
//! - `__sshSftp(id)` -> Promise<envelope {sftpId}>
//! - `__sftpReaddir(id, path)` -> Promise<envelope [entries]>
//! - `__sftpReadFile(id, path)` -> Promise<envelope [bytes]>
//! - `__sftpWriteFile(id, path, bytes)` -> Promise<envelope>
//! - `__sftpMkdir(id, path)` / `__sftpRmdir` / `__sftpUnlink` -> Promise<envelope>
//! - `__sftpRename(id, oldPath, newPath)` -> Promise<envelope>
//! - `__sftpStat(id, path)` -> Promise<envelope {size, mtime, mode}>
//! - `__sshClose(id)`

use russh::ChannelMsg;
use russh::client::{self, Handle};
use russh::keys::HashAlg;
use russh_sftp::client::SftpSession;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

use hmac::{Hmac, Mac};
use sha1::Sha1;

use base64::Engine;

type SshId = u32;
type SftpId = u32;

// Host-key verification policy for one `connect()`, supplied by the JS glue
// from the `connect()` options and read by `SshHandler::check_server_key`.
// Without one of `fingerprint`, `known_hosts` or `verifier` the connect fails
// closed (`EHOSTUNVERIFY`), unless `--allow-insecure` was passed or the host
// is loopback — the same trust model as the plaintext-protocol policy.
struct HostKeyPolicy {
    fingerprint: Option<String>,
    known_hosts: Option<String>,
    verifier: bool,
}

struct SshHandler {
    host: String,
    port: u16,
    policy: HostKeyPolicy,
    // op id of the connect that owns this handshake; used to route a
    // `hostVerifier` decision request back to the JS glue that started it.
    op_id: u32,
}

impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let pubkey = key.public_key();
        match host_key_check(
            &self.host,
            self.port,
            &self.policy,
            vvva_permissions::insecure_ssh_allowed(&self.host),
            &pubkey,
        ) {
            HostKeyCheck::Accepted => Ok(true),
            HostKeyCheck::Rejected => Ok(false),
            // A `hostVerifier` callback decides: the handshake parks here until
            // the JS glue answers; the reply is routed by verification id so
            // several connects can verify concurrently.
            HostKeyCheck::NeedsVerifier(fingerprint) => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let verification_id = next_verification_id();
                pending_verifications().lock().unwrap().insert(
                    verification_id,
                    PendingVerification {
                        op_id: self.op_id,
                        fingerprint,
                        reply: tx,
                    },
                );
                // A verifier that never answers must not hang the connect forever.
                let decision = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .unwrap_or(false);
                pending_verifications()
                    .lock()
                    .unwrap()
                    .remove(&verification_id);
                Ok(decision)
            }
        }
    }
}

// The outcome of the deterministic part of host-key verification. The
// `NeedsVerifier` case hands back to JS; every other case is decided here.
enum HostKeyCheck {
    Accepted,
    Rejected,
    NeedsVerifier(String),
}

// Loopback and `--allow-insecure` trust the first key presented, like the
// plaintext-protocol policy trusts the first hop on this machine. Otherwise
// the key must match `hostFingerprint`, a `knownHosts` entry, or the
// `hostVerifier` callback; with none of those, verification fails closed.
fn host_key_check(
    host: &str,
    port: u16,
    policy: &HostKeyPolicy,
    insecure_allowed: bool,
    pubkey: &russh::keys::ssh_key::PublicKey,
) -> HostKeyCheck {
    if insecure_allowed {
        return HostKeyCheck::Accepted;
    }
    let fingerprint = pubkey.fingerprint(HashAlg::Sha256).to_string();
    if let Some(expected) = &policy.fingerprint {
        return if fingerprint_matches(expected, &fingerprint) {
            HostKeyCheck::Accepted
        } else {
            HostKeyCheck::Rejected
        };
    }
    if let Some(content) = &policy.known_hosts {
        // A changed recorded key (Err) is refused too, as OpenSSH does.
        return if known_hosts_check(host, port, content, pubkey).unwrap_or(false) {
            HostKeyCheck::Accepted
        } else {
            HostKeyCheck::Rejected
        };
    }
    if policy.verifier {
        HostKeyCheck::NeedsVerifier(fingerprint)
    } else {
        HostKeyCheck::Rejected
    }
}

// A `hostVerifier` decision the JS glue must make before the handshake can
// continue. `check_server_key` registers one and awaits `reply`; the JS glue
// learns about it through `__sshVerifyPoll` and answers via `__sshVerifyReply`.
struct PendingVerification {
    op_id: u32,
    fingerprint: String,
    reply: tokio::sync::oneshot::Sender<bool>,
}

static PENDING_VERIFICATIONS: OnceLock<Mutex<HashMap<u32, PendingVerification>>> = OnceLock::new();

fn pending_verifications() -> &'static Mutex<HashMap<u32, PendingVerification>> {
    PENDING_VERIFICATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_verification_id() -> u32 {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// True when `expected` equals `actual` (both are `SHA256:<base64>`), ignoring
// case, or when `expected` is the bare base64 of `actual` without the prefix.
fn fingerprint_matches(expected: &str, actual: &str) -> bool {
    let expected = expected.trim().to_ascii_lowercase();
    let actual = actual.to_ascii_lowercase();
    expected == actual
        || actual
            .split_once(':')
            .is_some_and(|(_, b64)| expected == b64)
}

// known_hosts matching over in-memory content (the JS glue reads the file
// through the fs permission model and passes it here, so a script can't use
// this module as a file oracle). Mirrors OpenSSH: comma-separated host
// patterns, `[host]:port` for non-22 ports, and hashed `|1|salt|hash` lines.
// Returns Ok(true) on a match, Ok(false) when the host has no usable entry,
// and Err(()) when a recorded key of the same type differs ("key changed").
fn known_hosts_check(
    host: &str,
    port: u16,
    content: &str,
    pubkey: &russh::keys::ssh_key::PublicKey,
) -> Result<bool, ()> {
    let host_port = if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    };
    for raw_line in content.lines() {
        let line = raw_line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let hosts = fields.next().unwrap_or_default();
        let _key_type = fields.next();
        let key_b64 = fields.next();
        let Some(key_b64) = key_b64 else { continue };
        if !known_hosts_hostname_matches(&host_port, hosts) {
            continue;
        }
        let Ok(recorded) = russh::keys::parse_public_key_base64(key_b64) else {
            continue;
        };
        if pubkey.algorithm() == recorded.algorithm() && *pubkey == recorded {
            return Ok(true);
        }
        if pubkey.algorithm() == recorded.algorithm() {
            return Err(());
        }
    }
    Ok(false)
}

fn known_hosts_hostname_matches(host: &str, pattern: &str) -> bool {
    for entry in pattern.split(',') {
        if let Some(rest) = entry.strip_prefix("|1|") {
            // Hashed host line: `|1|<salt b64>|<sha1-b64>` where the hash is
            // HMAC-SHA1 keyed by the salt over the hostname.
            let mut parts = rest.split('|');
            let (Some(salt), Some(hash)) = (parts.next(), parts.next()) else {
                continue;
            };
            let (Ok(salt), Ok(hash)) = (
                base64::engine::general_purpose::STANDARD.decode(salt.as_bytes()),
                base64::engine::general_purpose::STANDARD.decode(hash.as_bytes()),
            ) else {
                continue;
            };
            if let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(&salt) {
                mac.update(host.as_bytes());
                if mac.verify_slice(&hash).is_ok() {
                    return true;
                }
            }
        } else if host == entry {
            return true;
        }
    }
    false
}

struct SshConn {
    handle: Handle<SshHandler>,
    // russh spawns a background task that drives the connection (framing,
    // keepalives, channel dispatch) onto whatever runtime `client::connect`
    // ran on. Every op used to build its OWN throwaway `Runtime::new()` and
    // drop it when done — which killed that driver task the moment connect()
    // returned, so every later exec/sftp call on the same connection failed
    // with a channel send error (the driver task was gone). Keeping the
    // connect-time runtime alive for the connection's whole lifetime, and
    // reusing it for every later op, is what actually keeps the session
    // alive between calls.
    runtime: Arc<tokio::runtime::Runtime>,
}

struct SftpConn {
    sftp: SftpSession,
    // Same reasoning as SshConn::runtime — an SFTP session's stream is a
    // channel over the same SSH connection, driven by that connection's
    // background task.
    runtime: Arc<tokio::runtime::Runtime>,
}

static SSH_REGISTRY: OnceLock<Mutex<HashMap<SshId, Arc<SshConn>>>> = OnceLock::new();
static SFTP_REGISTRY: OnceLock<Mutex<HashMap<SftpId, Arc<SftpConn>>>> = OnceLock::new();

fn ssh_registry() -> &'static Mutex<HashMap<SshId, Arc<SshConn>>> {
    SSH_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sftp_registry() -> &'static Mutex<HashMap<SftpId, Arc<SftpConn>>> {
    SFTP_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_ssh_id() -> SshId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// Every __ssh*/__sftp* operation below spawns a background OS thread with
// its own throwaway tokio runtime to do real blocking network I/O (SSH
// connect, exec, SFTP calls) — necessary since none of this is safe to
// await on the V8 callback thread. Each op gets an id here, the spawned
// thread stores its JSON envelope result keyed by that id when done, and
// `__sshOpPoll` (a plain, non-blocking native function) drains it. This
// used to be missing entirely: the native functions returned `undefined`
// while the spawned thread's result went nowhere but an eprintln!, so
// every `.then()` on them threw "Cannot read properties of undefined
// (reading 'then')" instead of ever settling.
static SSH_OPS: OnceLock<Mutex<HashMap<u32, String>>> = OnceLock::new();
fn ssh_ops() -> &'static Mutex<HashMap<u32, String>> {
    SSH_OPS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn next_op_id() -> u32 {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn next_sftp_id() -> SftpId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn get_ssh(id: SshId) -> Option<Arc<SshConn>> {
    ssh_registry().lock().unwrap().get(&id).cloned()
}

fn get_sftp(id: SftpId) -> Option<Arc<SftpConn>> {
    sftp_registry().lock().unwrap().get(&id).cloned()
}

fn ok_envelope(data: serde_json::Value) -> String {
    json!({"ok": true, "data": data}).to_string()
}

fn err_envelope(code: &str, message: impl std::fmt::Display) -> String {
    json!({"ok": false, "code": code, "message": message.to_string()}).to_string()
}

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static INJECT_SSH_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn permissions() -> Arc<PermissionState> {
    INJECT_SSH_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_ssh not called on this thread")
    })
}

pub fn inject_ssh(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    permissions_param: Arc<PermissionState>,
) {
    let context = scope.get_current_context();
    let global = context.global(scope);
    INJECT_SSH_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));

    let create_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              _args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = next_ssh_id();
            rv.set(v8::Number::new(_scope, id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshCreate").unwrap().into(),
        create_fn.into(),
    );

    let connect_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SshId;
            let host = args.get(1).to_rust_string_lossy(_scope);
            let port = super::port_from_js(args.get(2).uint32_value(_scope), 22);
            let username = args.get(3).to_rust_string_lossy(_scope);
            let password = args.get(4).to_rust_string_lossy(_scope);
            // Host-key verification policy (see HostKeyPolicy). The JS glue
            // already resolved `knownHosts` to file content through the fs
            // permission model before calling here.
            let fingerprint = args.get(5).to_rust_string_lossy(_scope);
            let known_hosts = args.get(6).to_rust_string_lossy(_scope);
            let has_verifier = args.get(7).uint32_value(_scope).unwrap_or(0) != 0;
            let policy = HostKeyPolicy {
                fingerprint: (!fingerprint.is_empty()).then_some(fingerprint),
                known_hosts: (!known_hosts.is_empty()).then_some(known_hosts),
                verifier: has_verifier,
            };
            // Read on this (callback) thread, where the thread-local is
            // actually populated, and move the clone into the spawned
            // thread — permissions() itself would panic if called from
            // there (see PermissionState's doc comment above).
            let perms_for_thread = permissions();
            // Same reasoning for the per-package permission scope
            // (vvva_permissions::scope): it's a thread-local too, set by the
            // require() wrapper on the JS engine's own thread. A freshly
            // spawned std::thread::spawn thread never had it set, so
            // perms_for_thread.check() below would silently evaluate against
            // ROOT_SCOPE instead of whichever package actually called
            // ssh.connect() — capture it here and re-apply it on the new
            // thread before the check.
            let scope_for_thread = vvva_permissions::current_scope();
            let op_id = next_op_id();

            std::thread::spawn(move || {
                vvva_permissions::set_current_scope(&scope_for_thread);
                // Kept alive for the connection's lifetime via SshConn — see
                // its doc comment. Not dropped at the end of this thread.
                let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
                let rt_for_conn = rt.clone();
                let result: String = rt.block_on(async {
                    if !perms_for_thread.check(&Capability::Network(vvva_permissions::authority(
                        &host, port,
                    ))) {
                        return err_envelope(
                            "EACCES",
                            format!("Network access denied. Run with --allow-net={}", host),
                        );
                    }

                    // Dial only vetted addresses, resolved once (VULN-18).
                    let addrs = match perms_for_thread.vetted_addrs(&host, port) {
                        Ok(a) => a,
                        Err(e) => return err_envelope("EACCES", e),
                    };
                    let config = Arc::new(client::Config::default());
                    let mut handle = match client::connect(
                        config,
                        &addrs[..],
                        SshHandler {
                            host: host.clone(),
                            port,
                            policy,
                            op_id,
                        },
                    )
                    .await
                    {
                        Ok(h) => h,
                        Err(e) => {
                            // check_server_key returned false: no recorded key
                            // matched and no explicit verification option was
                            // given, so the host key is unverified (MITM).
                            if matches!(e, russh::Error::UnknownKey) {
                                return err_envelope(
                                    "EHOSTUNVERIFY",
                                    "Host key verification failed. Pass hostFingerprint, \
                                     knownHosts or hostVerifier to connect(), or run with \
                                     --allow-insecure",
                                );
                            }
                            return err_envelope("ECONNREFUSED", e);
                        }
                    };

                    match handle.authenticate_password(&username, &password).await {
                        Ok(auth) if auth.success() => {
                            ssh_registry().lock().unwrap().insert(
                                id,
                                Arc::new(SshConn {
                                    handle,
                                    runtime: rt_for_conn,
                                }),
                            );
                            ok_envelope(serde_json::Value::Null)
                        }
                        Ok(_) => err_envelope("EAUTH", "authentication failed"),
                        Err(e) => err_envelope("EAUTH", e),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshConnect").unwrap().into(),
        connect_fn.into(),
    );

    let verify_poll_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let op_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let guard = pending_verifications().lock().unwrap();
            for (verification_id, pv) in guard.iter() {
                if pv.op_id == op_id {
                    let payload = json!({
                        "verificationId": verification_id,
                        "fingerprint": pv.fingerprint,
                    })
                    .to_string();
                    rv.set(v8::String::new(_scope, &payload).unwrap().into());
                    return;
                }
            }
            rv.set(v8::null(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshVerifyPoll").unwrap().into(),
        verify_poll_fn.into(),
    );

    let verify_reply_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let verification_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let accept = args.get(1).uint32_value(_scope).unwrap_or(0) != 0;
            let replied = match pending_verifications()
                .lock()
                .unwrap()
                .remove(&verification_id)
            {
                Some(pv) => pv.reply.send(accept).is_ok(),
                None => false,
            };
            rv.set(v8::Boolean::new(_scope, replied).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshVerifyReply").unwrap().into(),
        verify_reply_fn.into(),
    );

    let exec_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SshId;
            let command = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_ssh(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SSH ID"));
                        return;
                    }
                };
                // Reuse the connection's own runtime — see SshConn::runtime's
                // doc comment; a fresh Runtime::new() here would drop out
                // from under the connection's background driver task on the
                // *previous* connect() call, breaking every op after the
                // first.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    let mut channel = match conn.handle.channel_open_session().await {
                        Ok(ch) => ch,
                        Err(e) => {
                            return err_envelope(
                                "EIO",
                                format!("channel_open_session failed: {}", e),
                            );
                        }
                    };

                    if let Err(e) = channel.exec(true, command.as_bytes()).await {
                        return err_envelope("EIO", format!("exec failed: {}", e));
                    }

                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    let mut code = None;
                    loop {
                        match channel.wait().await {
                            Some(ChannelMsg::Data { data }) => stdout.extend_from_slice(&data),
                            Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                                stderr.extend_from_slice(&data)
                            }
                            Some(ChannelMsg::ExitStatus { exit_status }) => {
                                code = Some(exit_status)
                            }
                            Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                            _ => {}
                        }
                    }

                    ok_envelope(json!({
                        "stdout": String::from_utf8_lossy(&stdout),
                        "stderr": String::from_utf8_lossy(&stderr),
                        "code": code.unwrap_or(0),
                    }))
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshExec").unwrap().into(),
        exec_fn.into(),
    );

    let sftp_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SshId;
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_ssh(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SSH ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let rt_for_sftp = rt.clone();
                let result: String = rt.block_on(async move {
                    let channel = match conn.handle.channel_open_session().await {
                        Ok(ch) => ch,
                        Err(e) => {
                            return err_envelope(
                                "EIO",
                                format!("channel_open_session failed: {}", e),
                            );
                        }
                    };
                    if let Err(e) = channel.request_subsystem(true, "sftp").await {
                        return err_envelope(
                            "EIO",
                            format!("sftp subsystem request failed: {}", e),
                        );
                    }
                    let sftp = match SftpSession::new(channel.into_stream()).await {
                        Ok(s) => s,
                        Err(e) => {
                            return err_envelope("EIO", format!("sftp session failed: {}", e));
                        }
                    };

                    let sftp_id = next_sftp_id();
                    sftp_registry().lock().unwrap().insert(
                        sftp_id,
                        Arc::new(SftpConn {
                            sftp,
                            runtime: rt_for_sftp,
                        }),
                    );
                    ok_envelope(json!({ "sftpId": sftp_id }))
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshSftp").unwrap().into(),
        sftp_fn.into(),
    );

    let readdir_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.read_dir(&path).await {
                        Ok(rd) => {
                            let entries: Vec<serde_json::Value> = rd
                                .map(|entry| {
                                    let meta = entry.metadata();
                                    json!({
                                        "filename": entry.file_name(),
                                        "longname": entry.file_name(),
                                        "attrs": {
                                            "size": meta.len(),
                                            "mtime": meta.mtime.unwrap_or(0),
                                            "atime": meta.atime.unwrap_or(0),
                                            "mode": meta.permissions.unwrap_or(0),
                                        }
                                    })
                                })
                                .collect();
                            ok_envelope(serde_json::Value::Array(entries))
                        }
                        Err(e) => err_envelope("EIO", format!("readdir failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpReaddir").unwrap().into(),
        readdir_fn.into(),
    );

    let read_file_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.read(&path).await {
                        Ok(bytes) => ok_envelope(json!(bytes)),
                        Err(e) => err_envelope("EIO", format!("read failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpReadFile").unwrap().into(),
        read_file_fn.into(),
    );

    let write_file_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let data = {
                let maybe_uint8 = v8::Local::<v8::Uint8Array>::try_from(args.get(2)).ok();
                if let Some(uint8) = maybe_uint8 {
                    let len = uint8.byte_length();
                    let mut data = vec![0u8; len];
                    uint8.copy_contents(&mut data);
                    data
                } else {
                    vec![]
                }
            };

            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.write(&path, &data).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("write failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpWriteFile").unwrap().into(),
        write_file_fn.into(),
    );

    let mkdir_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.create_dir(&path).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("mkdir failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpMkdir").unwrap().into(),
        mkdir_fn.into(),
    );

    let rmdir_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.remove_dir(&path).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("rmdir failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpRmdir").unwrap().into(),
        rmdir_fn.into(),
    );

    let unlink_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.remove_file(&path).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("unlink failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpUnlink").unwrap().into(),
        unlink_fn.into(),
    );

    let rename_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let old_path = args.get(1).to_rust_string_lossy(_scope);
            let new_path = args.get(2).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.rename(&old_path, &new_path).await {
                        Ok(_) => ok_envelope(serde_json::Value::Null),
                        Err(e) => err_envelope("EIO", format!("rename failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpRename").unwrap().into(),
        rename_fn.into(),
    );

    let stat_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SftpId;
            let path = args.get(1).to_rust_string_lossy(_scope);
            let op_id = next_op_id();

            std::thread::spawn(move || {
                let conn = match get_sftp(id) {
                    Some(c) => c,
                    None => {
                        ssh_ops()
                            .lock()
                            .unwrap()
                            .insert(op_id, err_envelope("ENOTCONN", "Invalid SFTP ID"));
                        return;
                    }
                };
                // Reuse the connection's runtime — see SshConn::runtime.
                let rt = conn.runtime.clone();
                let result: String = rt.block_on(async move {
                    match conn.sftp.metadata(&path).await {
                        Ok(attrs) => ok_envelope(json!({
                            "size": attrs.len(),
                            "mtime": attrs.mtime.unwrap_or(0),
                            "mode": attrs.permissions.unwrap_or(0),
                        })),
                        Err(e) => err_envelope("EIO", format!("stat failed: {}", e)),
                    }
                });
                ssh_ops().lock().unwrap().insert(op_id, result);
            });

            rv.set(v8::Number::new(_scope, op_id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sftpStat").unwrap().into(),
        stat_fn.into(),
    );

    let op_poll_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let op_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            match ssh_ops().lock().unwrap().remove(&op_id) {
                Some(json) => rv.set(v8::String::new(_scope, &json).unwrap().into()),
                None => rv.set(v8::null(_scope).into()),
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshOpPoll").unwrap().into(),
        op_poll_fn.into(),
    );

    let ssh_close_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = args.get(0).uint32_value(_scope).unwrap_or(0) as SshId;
            ssh_registry().lock().unwrap().remove(&id);
            rv.set(v8::Boolean::new(_scope, true).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__sshClose").unwrap().into(),
        ssh_close_fn.into(),
    );

    let js_code = r#"
    (function() {
        // Every __ssh*/__sftp* native function below is synchronous: it
        // starts a background thread and returns an op id immediately,
        // rather than a Promise (a native function cannot hand back a real
        // V8 Promise here). This polls __sshOpPoll(opId) until the
        // background thread's result is ready, wrapped as a Promise so the
        // call sites read the same as before.
        function _pollOp(startFn, args, onVerify) {
            var opId = startFn.apply(null, args);
            return new Promise(function(resolve) {
                (function check() {
                    // A hostVerifier callback is pending: __sshVerifyPoll
                    // hands us the fingerprint, we ask the callback (which may
                    // be async) and report the decision back to the handshake.
                    var v = __sshVerifyPoll(opId);
                    if (v !== null && v !== undefined) {
                        var pv = JSON.parse(v);
                        var decision;
                        try {
                            decision = onVerify ? onVerify(pv.fingerprint) : false;
                        } catch (e) {
                            __sshVerifyReply(pv.verificationId, 0);
                            resolve(JSON.stringify({
                                ok: false,
                                code: 'EHOSTUNVERIFY',
                                message: 'hostVerifier threw: ' + (e && e.message || e)
                            }));
                            return;
                        }
                        Promise.resolve(decision).then(function(accept) {
                            __sshVerifyReply(pv.verificationId, accept ? 1 : 0);
                            setTimeout(check, 5);
                        }, function(e) {
                            __sshVerifyReply(pv.verificationId, 0);
                            resolve(JSON.stringify({
                                ok: false,
                                code: 'EHOSTUNVERIFY',
                                message: 'hostVerifier rejected: ' + (e && e.message || e)
                            }));
                        });
                        return;
                    }
                    var r = __sshOpPoll(opId);
                    if (r === null || r === undefined) { setTimeout(check, 5); return; }
                    resolve(r);
                })();
            });
        }

        function _unwrap(json) {
            var env = JSON.parse(json);
            if (env.ok) return { error: null, data: env.data };
            var err = new Error(env.message);
            err.code = env.code;
            return { error: err, data: null };
        }

        function Client(options) {
            this._id = null;
            this._connected = false;
            this._handlers = {};
            this._opts = options || {};
        }

        Client.prototype.connect = function(options) {
            var self = this;
            options = options || {};
            var host = options.host || 'localhost';
            var port = options.port || 22;
            var username = options.username || 'root';
            var password = options.password || '';

            // Host-key verification: without one of these the connect fails
            // closed (EHOSTUNVERIFY) unless the host is loopback or the
            // process ran with --allow-insecure. knownHosts is read through
            // the fs permission model (it needs --allow-read for the file).
            var fingerprint = options.hostFingerprint || '';
            var knownHosts = '';
            if (typeof options.knownHosts === 'string' && options.knownHosts) {
                try {
                    knownHosts = require('fs').readFileSync(options.knownHosts, 'utf8');
                } catch (e) {
                    var err = new Error('Failed to read knownHosts: ' + e.message);
                    err.code = 'EHOSTUNVERIFY';
                    self.emit('error', err);
                    return this;
                }
            }
            var verifier = typeof options.hostVerifier === 'function'
                ? options.hostVerifier
                : null;

            this._id = __sshCreate();
            _pollOp(
                __sshConnect,
                [this._id, host, port, username, password,
                 fingerprint, knownHosts, verifier ? 1 : 0],
                function(fp) { return verifier ? verifier(fp) : false; }
            ).then(function(json) {
                var r = _unwrap(json);
                if (r.error) { self.emit('error', r.error); return; }
                self._connected = true;
                self.emit('ready');
            }).catch(function(err) { self.emit('error', err); });

            return this;
        };

        Client.prototype.exec = function(command, callback) {
            var self = this;
            if (!this._connected) {
                var err = Object.assign(new Error('Not connected'), { code: 'ENOTCONN' });
                if (callback) callback(err, null);
                return this;
            }
            _pollOp(__sshExec, [this._id, command]).then(function(json) {
                var r = _unwrap(json);
                if (r.error) { if (callback) callback(r.error, null); return; }
                var ch = new (require('events').EventEmitter)();
                ch.stdout = new (require('events').EventEmitter)();
                ch.stderr = new (require('events').EventEmitter)();
                if (callback) callback(null, ch);
                setTimeout(function() {
                    ch.stdout.emit('data', Buffer.from(r.data.stdout));
                    if (r.data.stderr) ch.stderr.emit('data', Buffer.from(r.data.stderr));
                    ch.emit('close', r.data.code);
                    ch.emit('exit', r.data.code);
                }, 0);
            }).catch(function(err) { if (callback) callback(err, null); });
            return this;
        };

        Client.prototype.shell = function(options, callback) {
            var sh = { on: function() { return this; }, stdin: { write: function() { return this; } } };
            if (typeof options === 'function') options(null, sh);
            else if (callback) callback(null, sh);
            return sh;
        };

        Client.prototype.sftp = function(callback) {
            var self = this;
            if (!this._connected) {
                var err = Object.assign(new Error('Not connected'), { code: 'ENOTCONN' });
                if (callback) callback(err, null);
                return;
            }
            _pollOp(__sshSftp, [this._id]).then(function(json) {
                var r = _unwrap(json);
                if (r.error) { if (callback) callback(r.error, null); return; }
                if (callback) callback(null, new SftpWrapper(r.data.sftpId));
            }).catch(function(err) { if (callback) callback(err, null); });
        };

        Client.prototype.end = function() {
            if (this._id !== null) {
                __sshClose(this._id);
                this._connected = false;
                this._id = null;
            }
        };

        Client.prototype.disconnect = Client.prototype.end;

        Client.prototype.on = Client.prototype.addListener = function(event, listener) {
            this._handlers[event] = this._handlers[event] || [];
            this._handlers[event].push(listener);
            return this;
        };

        Client.prototype.off = Client.prototype.removeListener = function(event, listener) {
            if (this._handlers[event] && listener) {
                var idx = this._handlers[event].indexOf(listener);
                if (idx >= 0) this._handlers[event].splice(idx, 1);
            }
            return this;
        };

        Client.prototype.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            (this._handlers[event] || []).forEach(function(h) { h.apply(null, args); });
        };

        Client.prototype.readFile = function(path, options, callback) {
            if (typeof options === 'function') { callback = options; }
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.readFile(path, callback);
            });
        };

        Client.prototype.writeFile = function(path, data, options, callback) {
            if (typeof options === 'function') { callback = options; }
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.writeFile(path, data, callback);
            });
        };

        Client.prototype.stat = function(path, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err, null); return; }
                sftp.stat(path, callback);
            });
        };

        Client.prototype.mkdir = function(path, attrs, callback) {
            if (typeof attrs === 'function') { callback = attrs; }
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.mkdir(path, callback);
            });
        };

        Client.prototype.rmdir = function(path, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.rmdir(path, callback);
            });
        };

        Client.prototype.unlink = function(path, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.unlink(path, callback);
            });
        };

        Client.prototype.rename = function(from, to, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err); return; }
                sftp.rename(from, to, callback);
            });
        };

        Client.prototype.readdir = function(path, callback) {
            this.sftp(function(err, sftp) {
                if (err) { if (callback) callback(err, []); return; }
                sftp.readdir(path, callback);
            });
        };

        function SftpWrapper(sftpId) {
            this._sftpId = sftpId;
        }

        SftpWrapper.prototype.readdir = function(path, callback) {
            _pollOp(__sftpReaddir, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error, r.error ? [] : r.data);
            }).catch(function(err) { if (callback) callback(err, []); });
        };

        SftpWrapper.prototype.readFile = function(path, options, callback) {
            if (typeof options === 'function') { callback = options; }
            _pollOp(__sftpReadFile, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (r.error) { if (callback) callback(r.error, null); return; }
                if (callback) callback(null, Buffer.from(r.data));
            }).catch(function(err) { if (callback) callback(err, null); });
        };

        SftpWrapper.prototype.writeFile = function(path, data, options, callback) {
            if (typeof options === 'function') { callback = options; }
            var bytes = typeof data === 'string'
                ? Array.from(new TextEncoder().encode(data))
                : Array.from(data instanceof Uint8Array ? data : new Uint8Array(data));
            _pollOp(__sftpWriteFile, [this._sftpId, path, bytes]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.mkdir = function(path, callback) {
            _pollOp(__sftpMkdir, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.rmdir = function(path, callback) {
            _pollOp(__sftpRmdir, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.unlink = function(path, callback) {
            _pollOp(__sftpUnlink, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.rename = function(from, to, callback) {
            _pollOp(__sftpRename, [this._sftpId, from, to]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error);
            }).catch(function(err) { if (callback) callback(err); });
        };

        SftpWrapper.prototype.stat = function(path, callback) {
            _pollOp(__sftpStat, [this._sftpId, path]).then(function(json) {
                var r = _unwrap(json);
                if (callback) callback(r.error, r.error ? null : r.data);
            }).catch(function(err) { if (callback) callback(err, null); });
        };

        SftpWrapper.prototype.lstat = SftpWrapper.prototype.stat;

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['ssh2'] = { Client: Client };
        globalThis.__requireCache['node:ssh2'] = { Client: Client };
        globalThis.ssh = { Client: Client };
    })();
    "#;

    let _ = crate::builtins::code_cache::bootstrap_js(scope, "ssh", js_code);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two distinct Ed25519 public keys, straight from russh's own known_hosts
    // test fixtures.
    fn key_a() -> russh::keys::ssh_key::PublicKey {
        russh::keys::parse_public_key_base64(
            "AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ",
        )
        .unwrap()
    }

    fn key_b() -> russh::keys::ssh_key::PublicKey {
        russh::keys::parse_public_key_base64(
            "AAAAC3NzaC1lZDI1NTE5AAAAIA6rWI3G1sz07DnfFlrouTcysQlj2P+jpNSOEWD9OJ3X",
        )
        .unwrap()
    }

    fn policy(
        fingerprint: Option<&str>,
        known_hosts: Option<&str>,
        verifier: bool,
    ) -> HostKeyPolicy {
        HostKeyPolicy {
            fingerprint: fingerprint.map(str::to_string),
            known_hosts: known_hosts.map(str::to_string),
            verifier,
        }
    }

    fn key_a_fingerprint() -> String {
        key_a().fingerprint(HashAlg::Sha256).to_string()
    }

    #[test]
    fn fingerprint_matches_exact_and_bare_base64() {
        let fp = key_a_fingerprint();
        assert!(fingerprint_matches(&fp, &fp));
        // bare base64 without the "SHA256:" prefix
        let bare = fp.split_once(':').unwrap().1;
        assert!(fingerprint_matches(bare, &fp));
        // case-insensitive
        assert!(fingerprint_matches(&fp.to_lowercase(), &fp));
        // a different key's fingerprint must not match
        assert!(!fingerprint_matches(
            &key_b().fingerprint(HashAlg::Sha256).to_string(),
            &fp
        ));
    }

    #[test]
    fn host_key_check_accepts_loopback_and_allow_insecure() {
        let p = policy(None, None, false);
        // insecure_allowed = loopback exemption or --allow-insecure
        assert!(matches!(
            host_key_check("example.com", 22, &p, true, &key_a()),
            HostKeyCheck::Accepted
        ));
    }

    #[test]
    fn host_key_check_fails_closed_without_options() {
        let p = policy(None, None, false);
        assert!(matches!(
            host_key_check("example.com", 22, &p, false, &key_a()),
            HostKeyCheck::Rejected
        ));
    }

    #[test]
    fn host_key_check_fingerprint_match_and_mismatch() {
        let fp = key_a_fingerprint();
        let ok = policy(Some(&fp), None, false);
        assert!(matches!(
            host_key_check("example.com", 22, &ok, false, &key_a()),
            HostKeyCheck::Accepted
        ));
        let wrong = policy(
            Some(&key_b().fingerprint(HashAlg::Sha256).to_string()),
            None,
            false,
        );
        assert!(matches!(
            host_key_check("example.com", 22, &wrong, false, &key_a()),
            HostKeyCheck::Rejected
        ));
    }

    #[test]
    fn host_key_check_known_hosts_match_no_entry_and_changed() {
        use russh::keys::PublicKeyBase64;
        let content = format!(
            "# comment\nexample.com ssh-ed25519 {}\n",
            key_a().public_key_base64()
        );
        let p = policy(None, Some(&content), false);
        assert!(matches!(
            host_key_check("example.com", 22, &p, false, &key_a()),
            HostKeyCheck::Accepted
        ));

        // A different host has no entry: refused.
        assert!(matches!(
            host_key_check("other.example.com", 22, &p, false, &key_a()),
            HostKeyCheck::Rejected
        ));

        // The recorded key is a different key of the same type: refused.
        assert!(matches!(
            host_key_check("example.com", 22, &p, false, &key_b()),
            HostKeyCheck::Rejected
        ));
    }

    #[test]
    fn host_key_check_known_hosts_non_default_port() {
        use russh::keys::PublicKeyBase64;
        let content = format!(
            "[example.com]:2222 ssh-ed25519 {}\n",
            key_a().public_key_base64()
        );
        let p = policy(None, Some(&content), false);
        assert!(matches!(
            host_key_check("example.com", 2222, &p, false, &key_a()),
            HostKeyCheck::Accepted
        ));
        assert!(matches!(
            host_key_check("example.com", 22, &p, false, &key_a()),
            HostKeyCheck::Rejected
        ));
    }

    #[test]
    fn host_key_check_known_hosts_hashed_entry() {
        // |1|salt|hash line from russh's known_hosts tests (host "example.com").
        let content = "|1|O33ESRMWPVkMYIwJ1Uw+n877jTo=|nuuC5vEqXlEZ/8BXQR7m619W6Ak= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF\n";
        let key = russh::keys::parse_public_key_base64(
            "AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF",
        )
        .unwrap();
        let p = policy(None, Some(content), false);
        assert!(matches!(
            host_key_check("example.com", 22, &p, false, &key),
            HostKeyCheck::Accepted
        ));
        assert!(matches!(
            host_key_check("other.example.com", 22, &p, false, &key),
            HostKeyCheck::Rejected
        ));
    }

    #[test]
    fn host_key_check_verifier_defers_to_js() {
        let p = policy(None, None, true);
        assert!(matches!(
            host_key_check("example.com", 22, &p, false, &key_a()),
            HostKeyCheck::NeedsVerifier(_)
        ));
    }
}
