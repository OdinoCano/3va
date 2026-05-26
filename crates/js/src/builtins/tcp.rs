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

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use native_tls::TlsStream;
use rquickjs::{Ctx, Function, Result, function::Rest};
use vvva_permissions::{Capability, PermissionState};

// ── Connection type ───────────────────────────────────────────────────────────

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
            TcpConn::Plain(s) => { let _ = s.shutdown(std::net::Shutdown::Both); }
            TcpConn::Tls(s) => { let _ = s.shutdown(); }
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

// ── Injection ─────────────────────────────────────────────────────────────────

pub fn inject_tcp(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let pool: Arc<Mutex<HashMap<u32, TcpConn>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));

    let alloc_id = |pool: &Arc<Mutex<HashMap<u32, TcpConn>>>,
                    nid: &Arc<Mutex<u32>>,
                    conn: TcpConn|
     -> u32 {
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
            Function::new(ctx.clone(), move |ctx: Ctx<'_>, args: Rest<String>| -> Result<u32> {
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
            }),
        )?;
    }

    // __tcpConnectTls(host, port) -> id
    {
        let perms = permissions.clone();
        let pool = pool.clone();
        let nid = next_id.clone();
        ctx.globals().set(
            "__tcpConnectTls",
            Function::new(ctx.clone(), move |ctx: Ctx<'_>, args: Rest<String>| -> Result<u32> {
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
                let tls = connector
                    .connect(&host, tcp)
                    .map_err(|e| js_code_err(&ctx, "ECONNRESET", format!("TLS handshake failed: {}", e)))?;
                tls.get_ref()
                    .set_nonblocking(true)
                    .map_err(|e| js_err(&ctx, e.to_string()))?;

                Ok(alloc_id(&pool, &nid, TcpConn::Tls(tls)))
            }),
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

    // __tcpSetTimeout(id, ms) — sets read/write timeout (0 = non-blocking)
    {
        let pool = pool.clone();
        ctx.globals().set(
            "__tcpSetTimeout",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, id: u32, ms: u32| -> Result<()> {
                    let guard = pool.lock().unwrap();
                    let conn = guard
                        .get(&id)
                        .ok_or_else(|| js_err(&ctx, format!("tcpSetTimeout: unknown socket {}", id)))?;
                    let timeout = if ms == 0 {
                        None
                    } else {
                        Some(Duration::from_millis(ms as u64))
                    };
                    let raw = match conn {
                        TcpConn::Plain(s) => s as &TcpStream,
                        TcpConn::Tls(s) => s.get_ref(),
                    };
                    // Switch to blocking with timeout (overrides non-blocking mode).
                    raw.set_nonblocking(false).ok();
                    raw.set_read_timeout(timeout).ok();
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
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, id: u32| -> Result<()> {
                    if let Some(mut conn) = pool.lock().unwrap().remove(&id) {
                        conn.shutdown();
                    }
                    Ok(())
                },
            ),
        )?;
    }

    Ok(())
}
