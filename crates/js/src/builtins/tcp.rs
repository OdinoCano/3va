//! Raw TCP and TLS socket backend for the `net` and `tls` Node.js modules.
//!
//! Architecture mirrors `websocket.rs`: connections are held in a
//! `Arc<Mutex<HashMap<u32, TcpConn>>>` pool.  The JS side polls for data via
//! `__tcpRead`; writes and closes are synchronous.
//!
//! Read semantics:
//!   - Returns `Vec<u8>` with data when bytes are available.
//!   - Throws an error with `message == "EAGAIN"` when no data is ready.
//!   - Throws an error with `message == "EOF"` when the peer has closed.

use native_tls::TlsStream;
use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result, function::Rest};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use vvva_crypto;
use vvva_permissions::{Capability, PermissionState};

// ── Connection type ───────────────────────────────────────────────────────────

#[allow(clippy::large_enum_variant)]
enum TcpConn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl TcpConn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            TcpConn::Plain(s) => s.read(buf),
            TcpConn::Tls(s) => s.read(buf),
        }
    }
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            TcpConn::Plain(s) => s.write_all(data),
            TcpConn::Tls(s) => s.write_all(data),
        }
    }
    fn shutdown(&mut self) {
        match self {
            TcpConn::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            TcpConn::Tls(s) => {
                let _ = s.shutdown();
            }
        }
    }
}

// ── Error helpers ─────────────────────────────────────────────────────────────

fn js_err(ctx: &Ctx<'_>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value<'_>, _>(format!("new Error(\"{}\")", escaped)) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

fn js_code_err(ctx: &Ctx<'_>, code: &str, msg: String) -> rquickjs::Error {
    let escaped_msg = msg.replace('\\', "\\\\").replace('"', "\\\"");
    let src = format!(
        "(function(){{var e=new Error(\"{msg}\");e.code=\"{code}\";return e;}})()",
        msg = escaped_msg,
        code = code
    );
    match ctx.eval::<rquickjs::Value<'_>, _>(src) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

// ── PQ-TLS blocking helper ────────────────────────────────────────────────────

/// Perform the full TCP connect → TLS handshake → ML-KEM-768 key exchange on
/// the calling (blocking) thread.  Returns the ready-to-use TLS stream and the
/// hex-encoded 32-byte shared secret, or an error message string.
///
/// Callers **must** run this inside `tokio::task::spawn_blocking` so the JS
/// event loop is not stalled during network I/O.
fn pq_tls_connect_blocking(
    host: &str,
    port: u16,
) -> std::result::Result<(TlsStream<TcpStream>, String), String> {
    // Classical TLS handshake (blocking).
    let connector = native_tls::TlsConnector::new().map_err(|e| format!("TLS init: {e}"))?;
    let tcp =
        TcpStream::connect(format!("{host}:{port}")).map_err(|e| format!("ECONNREFUSED: {e}"))?;
    let mut tls = connector
        .connect(host, tcp)
        .map_err(|e| format!("TLS handshake failed: {e}"))?;

    // ML-KEM-768 ephemeral key exchange: client initiates.
    let kp = vvva_crypto::kem::MlKemKeypair::generate();
    let ek_bytes = kp.encapsulation_key_bytes();

    // Send [4-byte big-endian length][encapsulation key].
    let ek_len = (ek_bytes.len() as u32).to_be_bytes();
    tls.write_all(&ek_len)
        .and_then(|_| tls.write_all(&ek_bytes))
        .map_err(|e| format!("PQ TLS send ek: {e}"))?;

    // Receive [4-byte big-endian length][ML-KEM-768 ciphertext = 1088 B].
    let mut ct_len_buf = [0u8; 4];
    tls.read_exact(&mut ct_len_buf)
        .map_err(|e| format!("PQ TLS recv ct len: {e}"))?;
    let ct_len = u32::from_be_bytes(ct_len_buf) as usize;
    if ct_len != 1088 {
        return Err(format!("PQ TLS: invalid ciphertext length {ct_len}"));
    }
    let mut ct_bytes = vec![0u8; ct_len];
    tls.read_exact(&mut ct_bytes)
        .map_err(|e| format!("PQ TLS recv ct: {e}"))?;

    // Decode raw bytes directly — no hex round-trip needed.
    let ct = vvva_crypto::MlKemCiphertext::from_bytes(&ct_bytes)
        .map_err(|e| format!("PQ TLS ct decode: {e}"))?;
    let ss = vvva_crypto::decapsulate(&kp.dk, &ct);
    let ss_hex = hex::encode(ss.0);

    // Switch to non-blocking for subsequent reads through the connection pool.
    tls.get_ref()
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;

    Ok((tls, ss_hex))
}

// ── Injection ─────────────────────────────────────────────────────────────────

pub fn inject_tcp(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let pool: Arc<Mutex<HashMap<u32, TcpConn>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));

    let alloc_id =
        |pool: &Arc<Mutex<HashMap<u32, TcpConn>>>, nid: &Arc<Mutex<u32>>, conn: TcpConn| -> u32 {
            let id = {
                let mut n = nid.lock().unwrap();
                let id = *n;
                *n = n.wrapping_add(1);
                id
            };
            pool.lock().unwrap().insert(id, conn);
            id
        };

    // __tcpConnect(host, port) -> id
    {
        let perms = permissions.clone();
        let pool = pool.clone();
        let nid = next_id.clone();
        ctx.globals().set(
            "__tcpConnect",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, args: Rest<String>| -> Result<u32> {
                    let mut it = args.0.into_iter();
                    let host = it.next().unwrap_or_default();
                    let port: u16 = it
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| js_err(&ctx, "tcpConnect: invalid port".into()))?;

                    if !perms.check(&Capability::Network(host.clone())) {
                        return Err(js_code_err(
                            &ctx,
                            "EACCES",
                            format!("Network access denied. Run with --allow-net={}", host),
                        ));
                    }

                    let stream = TcpStream::connect(format!("{}:{}", host, port))
                        .map_err(|e| js_code_err(&ctx, "ECONNREFUSED", e.to_string()))?;
                    stream
                        .set_nonblocking(true)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;

                    Ok(alloc_id(&pool, &nid, TcpConn::Plain(stream)))
                },
            ),
        )?;
    }

    // __tcpConnectTls(host, port) -> id
    {
        let perms = permissions.clone();
        let pool = pool.clone();
        let nid = next_id.clone();
        ctx.globals().set(
            "__tcpConnectTls",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, args: Rest<String>| -> Result<u32> {
                    let mut it = args.0.into_iter();
                    let host = it.next().unwrap_or_default();
                    let port: u16 = it
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| js_err(&ctx, "tcpConnectTls: invalid port".into()))?;

                    if !perms.check(&Capability::Network(host.clone())) {
                        return Err(js_code_err(
                            &ctx,
                            "EACCES",
                            format!("Network access denied. Run with --allow-net={}", host),
                        ));
                    }

                    let connector = native_tls::TlsConnector::new()
                        .map_err(|e| js_err(&ctx, format!("TLS init failed: {}", e)))?;
                    let tcp = TcpStream::connect(format!("{}:{}", host, port))
                        .map_err(|e| js_code_err(&ctx, "ECONNREFUSED", e.to_string()))?;
                    // TLS handshake is blocking — keep the stream blocking during handshake,
                    // then switch to non-blocking for data reads.
                    let tls = connector.connect(&host, tcp).map_err(|e| {
                        js_code_err(&ctx, "ECONNRESET", format!("TLS handshake failed: {}", e))
                    })?;
                    tls.get_ref()
                        .set_nonblocking(true)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;

                    Ok(alloc_id(&pool, &nid, TcpConn::Tls(tls)))
                },
            ),
        )?;
    }

    // __tcpWrite(id, data) -> undefined | throws
    {
        let pool = pool.clone();
        ctx.globals().set(
            "__tcpWrite",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, id: u32, data: Vec<u8>| -> Result<()> {
                    let mut guard = pool.lock().unwrap();
                    let conn = guard
                        .get_mut(&id)
                        .ok_or_else(|| js_err(&ctx, format!("tcpWrite: unknown socket {}", id)))?;
                    conn.write_all(&data)
                        .map_err(|e| js_code_err(&ctx, "EPIPE", e.to_string()))
                },
            ),
        )?;
    }

    // __tcpRead(id, maxBytes) -> Vec<u8> | throws EAGAIN | throws EOF
    {
        let pool = pool.clone();
        ctx.globals().set(
            "__tcpRead",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, id: u32, max_bytes: u32| -> Result<Vec<u8>> {
                    let max = (max_bytes as usize).min(65536);
                    let mut buf = vec![0u8; max];
                    let mut guard = pool.lock().unwrap();
                    let conn = guard
                        .get_mut(&id)
                        .ok_or_else(|| js_err(&ctx, format!("tcpRead: unknown socket {}", id)))?;

                    match conn.read(&mut buf) {
                        Ok(0) => Err(js_code_err(&ctx, "EOF", "connection closed".into())),
                        Ok(n) => {
                            buf.truncate(n);
                            Ok(buf)
                        }
                        Err(ref e)
                            if e.kind() == io::ErrorKind::WouldBlock
                                || e.kind() == io::ErrorKind::TimedOut =>
                        {
                            Err(js_code_err(&ctx, "EAGAIN", "no data available".into()))
                        }
                        Err(e) => Err(js_code_err(&ctx, "EIO", e.to_string())),
                    }
                },
            ),
        )?;
    }

    // __tcpSetTimeout(id, ms) — no-op: sockets must stay non-blocking so the
    // JS event loop poll (_startPoll) doesn't block Tokio. Socket-level timeouts
    // are handled at the JS layer via setTimeout() in Socket.prototype.setTimeout.
    {
        let _pool = pool.clone();
        ctx.globals().set(
            "__tcpSetTimeout",
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, _id: u32, _ms: u32| -> Result<()> {
                    // Intentionally no-op: switching to blocking mode would cause
                    // __tcpRead to block the Tokio thread and freeze all JS timers.
                    Ok(())
                },
            ),
        )?;
    }

    // __tcpClose(id)
    {
        let pool = pool.clone();
        ctx.globals().set(
            "__tcpClose",
            Function::new(ctx.clone(), move |_ctx: Ctx<'_>, id: u32| -> Result<()> {
                if let Some(mut conn) = pool.lock().unwrap().remove(&id) {
                    conn.shutdown();
                }
                Ok(())
            }),
        )?;
    }

    // ── Raw TCP server ─────────────────────────────────────────────────────────
    // __netListen(port, host) → server_id   (sync; port bound immediately)
    // __netAcceptAsync(server_id) → Promise<conn_id>  (awaits next connection,
    //   then inserts the accepted stream into the shared tcp pool so __tcpRead /
    //   __tcpWrite / __tcpClose work on it without any extra plumbing)
    // __netClose(server_id) → void  (drops the listener)

    let listeners: Arc<Mutex<HashMap<u32, Arc<tokio::net::TcpListener>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let next_listener_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));

    {
        let perms = permissions.clone();
        let listeners = listeners.clone();
        let nid = next_listener_id.clone();
        ctx.globals().set(
            "__netListen",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, port: u16, host: String| -> Result<u32> {
                    if !perms.check(&Capability::Network(host.clone())) {
                        return Err(js_code_err(
                            &ctx,
                            "EACCES",
                            format!("Network access denied. Run with --allow-net={}", host),
                        ));
                    }

                    let std_l = std::net::TcpListener::bind(format!("{}:{}", host, port))
                        .map_err(|e| js_code_err(&ctx, "EADDRINUSE", e.to_string()))?;
                    std_l
                        .set_nonblocking(true)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    let tokio_l = tokio::net::TcpListener::from_std(std_l)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;

                    let id = {
                        let mut n = nid.lock().unwrap();
                        let id = *n;
                        *n = n.wrapping_add(1);
                        id
                    };
                    listeners.lock().unwrap().insert(id, Arc::new(tokio_l));
                    Ok(id)
                },
            ),
        )?;
    }

    {
        let listeners = listeners.clone();
        let pool = pool.clone();
        let next_id = next_id.clone();
        ctx.globals().set(
            "__netAcceptAsync",
            Function::new(
                ctx.clone(),
                Async(move |server_id: u32| {
                    let listeners = listeners.clone();
                    let pool = pool.clone();
                    let next_id = next_id.clone();
                    async move {
                        let listener = {
                            let g = listeners.lock().unwrap();
                            g.get(&server_id).cloned()
                        };
                        let listener = listener.ok_or_else(|| {
                            rquickjs::Error::new_from_js_message(
                                "ENOENT",
                                "ENOENT",
                                "unknown server id".to_string(),
                            )
                        })?;

                        let (tokio_stream, _addr) = listener.accept().await.map_err(|e| {
                            rquickjs::Error::new_from_js_message(
                                "ECONNRESET",
                                "ECONNRESET",
                                e.to_string(),
                            )
                        })?;

                        // Convert to std for the existing non-blocking pool
                        let std_stream = tokio_stream.into_std().map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;
                        std_stream.set_nonblocking(true).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        let conn_id = {
                            let mut n = next_id.lock().unwrap();
                            let id = *n;
                            *n = n.wrapping_add(1);
                            id
                        };
                        pool.lock()
                            .unwrap()
                            .insert(conn_id, TcpConn::Plain(std_stream));
                        Ok::<u32, rquickjs::Error>(conn_id)
                    }
                }),
            ),
        )?;
    }

    {
        let listeners = listeners.clone();
        ctx.globals().set(
            "__netClose",
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, server_id: u32| -> Result<()> {
                    listeners.lock().unwrap().remove(&server_id);
                    Ok(())
                },
            ),
        )?;
    }

    // ── Hybrid PQ-TLS connect ─────────────────────────────────────────────────
    //
    // __pqTlsConnect(host, port) → { connId: number, pqSharedSecret: hex }
    //
    // Establishes a classical TLS connection then performs an ML-KEM-768
    // ephemeral key encapsulation exchange over the secured channel:
    //   client → server: [4-byte length][ML-KEM encapsulation key]
    //   client ← server: [4-byte length][ML-KEM ciphertext]
    //   Both sides derive a 32-byte PQ shared secret.
    //
    // The resulting `pqSharedSecret` can be combined with the TLS session key
    // (e.g. via HKDF) to achieve hybrid classical+PQ forward secrecy.
    //
    // All blocking I/O (TCP connect, TLS handshake, ML-KEM round-trip) runs in
    // a `spawn_blocking` thread so the JS event loop is never stalled.
    {
        let pool = pool.clone();
        let nid = next_id.clone();
        let perms = permissions.clone();
        ctx.globals().set(
            "__pqTlsConnect",
            Function::new(
                ctx.clone(),
                Async(move |host: String, port: u16| {
                    let perms = perms.clone();
                    let pool = pool.clone();
                    let nid = nid.clone();
                    async move {
                        if !perms.check(&Capability::Network(host.clone())) {
                            return Err(rquickjs::Error::new_from_js_message(
                                "EACCES",
                                "EACCES",
                                format!("Network access denied. Run with --allow-net={}", host),
                            ));
                        }

                        // All blocking I/O runs off the JS thread.
                        let (tls, ss_hex) = tokio::task::spawn_blocking(move || {
                            pq_tls_connect_blocking(&host, port)
                        })
                        .await
                        .map_err(|e| {
                            rquickjs::Error::new_from_js_message(
                                "EIO",
                                "EIO",
                                format!("PQ TLS task panicked: {e}"),
                            )
                        })?
                        .map_err(|e| {
                            rquickjs::Error::new_from_js_message("ECONNRESET", "ECONNRESET", e)
                        })?;

                        let conn_id = alloc_id(&pool, &nid, TcpConn::Tls(tls));
                        Ok::<String, rquickjs::Error>(
                            serde_json::json!({ "connId": conn_id, "pqSharedSecret": ss_hex })
                                .to_string(),
                        )
                    }
                }),
            ),
        )?;
    }

    Ok(())
}
