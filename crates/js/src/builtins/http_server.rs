//! HTTP/1.1 server backend for `http.createServer()`.
//!
//! Security hardening (v2.1):
//!   - Header read timeout      → Slowloris protection
//!   - Body read timeout        → RUDY protection (total deadline)
//!   - Min body receive rate    → RUDY protection (drip rate)
//!   - Max header count + total header bytes
//!   - Per-IP token-bucket rate limiting (vvva_firewall)
//!   - Auto-block IPs that exceed violation threshold
//!   - Per-IP and total connection caps
//!   - Client IP forwarded to JS in every request object

use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpStream as StdTcpStream};
use std::sync::{Arc, Mutex};

use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::time::Instant;

use vvva_firewall::{Firewall, FirewallDecision};
use vvva_permissions::PermissionState;

const HARD_MAX_BODY: usize = 100 * 1024 * 1024;

/// Bind a TCP listener, enabling `SO_REUSEPORT` when `VVVA_CLUSTER` is set so
/// `3va start --instances N` can run N processes load-balanced by the kernel
/// on the same port (mirrors Node's `cluster` module, which does the same at
/// the libuv layer). Off by default: without cluster mode, two accidental
/// binds to the same port should still fail loudly with EADDRINUSE.
#[cfg(unix)]
fn bind_listener(addr: &str) -> std::io::Result<std::net::TcpListener> {
    if std::env::var_os("VVVA_CLUSTER").is_none() {
        return std::net::TcpListener::bind(addr);
    }
    let sockaddr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| std::io::Error::other(format!("invalid address {addr}: {e}")))?;
    let socket = socket2::Socket::new(
        socket2::Domain::for_address(sockaddr),
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    socket.bind(&sockaddr.into())?;
    socket.listen(1024)?;
    Ok(socket.into())
}

#[cfg(not(unix))]
fn bind_listener(addr: &str) -> std::io::Result<std::net::TcpListener> {
    std::net::TcpListener::bind(addr)
}

struct ConnEntry {
    stream: StdTcpStream,
}

fn js_err(ctx: &Ctx<'_>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value<'_>, _>(format!("new Error(\"{}\")", escaped)) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

fn js_code_err(ctx: &Ctx<'_>, code: &str, msg: &str) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    let src = format!(
        "(function(){{var e=new Error(\"{msg}\");e.code=\"{code}\";return e;}})()",
        msg = escaped,
        code = code,
    );
    match ctx.eval::<rquickjs::Value<'_>, _>(src) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn parse_extra_headers(headers_json: &str) -> Vec<(String, String)> {
    serde_json::from_str(headers_json)
        .ok()
        .and_then(|v: serde_json::Value| {
            v.as_object().map(|obj| {
                obj.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
        })
        .unwrap_or_default()
}

// ── Request parser ─────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ParsedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    std_stream: StdTcpStream,
}

async fn parse_request(
    stream: tokio::net::TcpStream,
    header_timeout: std::time::Duration,
    body_timeout: std::time::Duration,
    max_header_count: usize,
    max_header_bytes: usize,
    max_body_bytes: usize,
    min_body_rate_bps: u32,
) -> std::result::Result<ParsedRequest, String> {
    let mut reader = BufReader::new(stream);

    // Request line — timeout protects against connections that never send data.
    let mut request_line = String::new();
    tokio::time::timeout(header_timeout, reader.read_line(&mut request_line))
        .await
        .map_err(|_| "timeout: request line not received in time (Slowloris?)")?
        .map_err(|e| e.to_string())?;

    let request_line = request_line.trim_end_matches(['\r', '\n']);
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    // Headers — each read_line call is independently timed.
    let max_body = if max_body_bytes == 0 {
        HARD_MAX_BODY
    } else {
        max_body_bytes
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length: usize = 0;
    let mut total_header_bytes: usize = 0;

    loop {
        let mut line = String::new();
        tokio::time::timeout(header_timeout, reader.read_line(&mut line))
            .await
            .map_err(|_| "timeout: headers not received in time (Slowloris?)")?
            .map_err(|e| e.to_string())?;

        total_header_bytes += line.len();
        if total_header_bytes > max_header_bytes {
            return Err(format!("header flood: exceeded {} bytes", max_header_bytes));
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if headers.len() >= max_header_count {
            return Err(format!(
                "header flood: more than {} headers",
                max_header_count
            ));
        }

        if let Some(colon) = trimmed.find(':') {
            let name = trimmed[..colon].trim().to_lowercase();
            let value = trimmed[colon + 1..].trim().to_string();
            if name == "content-length" {
                content_length = value.parse::<usize>().unwrap_or(0).min(max_body);
                headers.push((name, content_length.to_string()));
            } else {
                headers.push((name, value));
            }
        }
    }

    // Body — two-layer RUDY defense:
    //   1. Total deadline (body_timeout) — connection dies if body never finishes.
    //   2. Minimum rate (min_body_rate_bps) — connection dies if drip rate is too slow,
    //      even when data arrives just fast enough to beat the total deadline.
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let deadline = Instant::now() + body_timeout;
        let mut received: usize = 0;
        let body_start = Instant::now();

        while received < content_length {
            if Instant::now() >= deadline {
                return Err("timeout: body deadline exceeded (RUDY?)".into());
            }
            // Read one chunk with a 1-second window so we can check rate frequently.
            let remaining_deadline = deadline.saturating_duration_since(Instant::now());
            let chunk_timeout = remaining_deadline.min(std::time::Duration::from_secs(1));
            let n = tokio::time::timeout(chunk_timeout, reader.read(&mut body[received..]))
                .await
                .map_err(|_| "timeout: body stalled (RUDY?)")?
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("connection closed before body complete".into());
            }
            received += n;

            // Rate check: after a short grace window, enforce minimum bytes/sec.
            if min_body_rate_bps > 0 {
                let elapsed = body_start.elapsed().as_secs_f64();
                if elapsed > 2.0 {
                    let rate = received as f64 / elapsed;
                    if rate < min_body_rate_bps as f64 {
                        return Err(format!(
                            "body rate too low ({:.0} B/s < {} B/s min): RUDY?",
                            rate, min_body_rate_bps
                        ));
                    }
                }
            }
        }
    }

    let std_stream = reader.into_inner().into_std().map_err(|e| e.to_string())?;
    Ok(ParsedRequest {
        method,
        path,
        headers,
        body,
        std_stream,
    })
}

/// Send a firewall-rejection response before a conn_id is allocated.
fn reject_stream(stream: tokio::net::TcpStream, status: u16, msg: &'static str) {
    let response = format!(
        "HTTP/1.1 {status} {msg}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{msg}",
        len = msg.len()
    );
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut s = stream;
        let _ = s.write_all(response.as_bytes()).await;
    });
}

// ── Injection ──────────────────────────────────────────────────────────────────

pub fn inject_http_server(
    ctx: &Ctx,
    permissions: Arc<PermissionState>,
    firewall: Option<Arc<Firewall>>,
) -> Result<()> {
    let servers: Arc<Mutex<HashMap<u32, Arc<TcpListener>>>> = Arc::new(Mutex::new(HashMap::new()));
    let conns: Arc<Mutex<HashMap<u32, ConnEntry>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_server_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let next_conn_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
    let fw: Arc<Option<Arc<Firewall>>> = Arc::new(firewall);

    // ── __httpListen ──────────────────────────────────────────────────────────
    {
        let perms = permissions.clone();
        let servers = servers.clone();
        let nid = next_server_id.clone();
        let fw = fw.clone();
        ctx.globals().set(
            "__httpListen",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, port: u16, host: String| -> Result<u32> {
                    if !perms.check_bind(&host) {
                        return Err(js_code_err(
                            &ctx,
                            "EACCES",
                            &format!("Network access denied. Run with --allow-net={}", host),
                        ));
                    }

                    let std_listener = bind_listener(&format!("{}:{}", host, port))
                        .map_err(|e| js_code_err(&ctx, "EADDRINUSE", &e.to_string()))?;
                    std_listener
                        .set_nonblocking(true)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    let tokio_listener = TcpListener::from_std(std_listener)
                        .map_err(|e| js_err(&ctx, format!("TcpListener::from_std: {}", e)))?;

                    let id = {
                        let mut n = nid.lock().unwrap();
                        let id = *n;
                        *n = n.wrapping_add(1);
                        id
                    };
                    servers.lock().unwrap().insert(id, Arc::new(tokio_listener));

                    // Start background blocklist/bucket cleanup once.
                    if id == 0
                        && let Some(firewall) = fw.as_ref().as_ref()
                    {
                        vvva_firewall::spawn_cleanup_task(
                            firewall.clone(),
                            std::time::Duration::from_secs(60),
                        );
                    }
                    Ok(id)
                },
            ),
        )?;
    }

    // ── __httpAcceptAsync ────────────────────────────────────────────────────
    {
        let servers = servers.clone();
        let conns = conns.clone();
        let nid = next_conn_id.clone();
        let fw = fw.clone();
        ctx.globals().set(
            "__httpAcceptAsync",
            Function::new(ctx.clone(), Async(move |server_id: u32| {
                let servers = servers.clone();
                let conns   = conns.clone();
                let nid     = nid.clone();
                let fw      = fw.clone();
                async move {
                    let listener = {
                        let guard = servers.lock().unwrap();
                        guard.get(&server_id).cloned()
                    };
                    let listener = listener.ok_or_else(|| {
                        rquickjs::Error::new_from_js_message("ENOENT", "ENOENT", "unknown server id")
                    })?;

                    loop {
                        let (stream, peer_addr) = listener.accept().await.map_err(|e| {
                            rquickjs::Error::new_from_js_message("ECONNRESET", "ECONNRESET", e.to_string())
                        })?;

                        let ip: IpAddr = match peer_addr {
                            SocketAddr::V4(a) => IpAddr::V4(*a.ip()),
                            SocketAddr::V6(a) => IpAddr::V6(*a.ip()),
                        };

                        // ── Firewall: connection gate ─────────────────────
                        if let Some(firewall) = fw.as_ref().as_ref() {
                            match firewall.check_connection(ip) {
                                FirewallDecision::Allow => { firewall.on_connect(ip); }
                                decision => {
                                    reject_stream(stream, decision.http_status(), decision.message());
                                    continue;
                                }
                            }
                        }

                        // ── Parser limits from firewall config ────────────
                        let (hdr_timeout, body_timeout, max_hdr_count, max_hdr_bytes, max_body, min_body_rate) =
                            if let Some(firewall) = fw.as_ref().as_ref() {
                                let c = &firewall.config;
                                (
                                    std::time::Duration::from_millis(c.header_timeout_ms),
                                    std::time::Duration::from_millis(c.body_timeout_ms),
                                    c.max_header_count as usize,
                                    c.max_header_bytes as usize,
                                    c.max_body_bytes as usize,
                                    c.min_body_rate_bps,
                                )
                            } else {
                                (
                                    std::time::Duration::from_secs(10),
                                    std::time::Duration::from_secs(30),
                                    100, 16_384, 0, 100,
                                )
                            };

                        let parsed = parse_request(
                            stream, hdr_timeout, body_timeout,
                            max_hdr_count, max_hdr_bytes, max_body, min_body_rate,
                        ).await;

                        let parsed = match parsed {
                            Ok(p) => p,
                            Err(_) => {
                                if let Some(firewall) = fw.as_ref().as_ref() {
                                    firewall.on_disconnect(ip);
                                }
                                continue; // drop connection, accept next
                            }
                        };

                        // ── Firewall: per-request rate limit ──────────────
                        if let Some(firewall) = fw.as_ref().as_ref() {
                            match firewall.check_request(ip) {
                                FirewallDecision::Allow => {}
                                decision => {
                                    // Write rejection into the already-parsed stream.
                                    let resp = format!(
                                        "HTTP/1.1 {s} {m}\r\nContent-Length: {l}\r\nConnection: close\r\n\r\n{m}",
                                        s = decision.http_status(), m = decision.message(),
                                        l = decision.message().len(),
                                    );
                                    let _ = { let mut s = parsed.std_stream; s.write_all(resp.as_bytes()) };
                                    firewall.on_disconnect(ip);
                                    continue;
                                }
                            }
                        }

                        // ── Allocate conn_id and return to JS ─────────────
                        let conn_id = {
                            let mut n = nid.lock().unwrap();
                            let id = *n; *n = n.wrapping_add(1); id
                        };
                        conns.lock().unwrap().insert(conn_id, ConnEntry { stream: parsed.std_stream });

                        let hdr_pairs: Vec<String> = parsed.headers.iter()
                            .map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)))
                            .collect();
                        let body_str = String::from_utf8_lossy(&parsed.body);
                        let json = format!(
                            "{{\"method\":\"{m}\",\"url\":\"{u}\",\"headers\":{{{h}}},\
                             \"body\":\"{b}\",\"conn_id\":{c},\"remoteAddress\":\"{ip}\"}}",
                            m  = json_escape(&parsed.method),
                            u  = json_escape(&parsed.path),
                            h  = hdr_pairs.join(","),
                            b  = json_escape(&body_str),
                            c  = conn_id,
                            ip = ip,
                        );

                        return Ok::<String, rquickjs::Error>(json);
                    }
                }
            })),
        )?;
    }

    // ── __httpRespond ─────────────────────────────────────────────────────────
    {
        let conns = conns.clone();
        let fw = fw.clone();
        ctx.globals().set(
            "__httpRespond",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      conn_id: u32,
                      status: u32,
                      status_text: String,
                      headers_json: String,
                      body: String|
                      -> Result<()> {
                    let body_bytes = body.as_bytes();
                    let extra = parse_extra_headers(&headers_json);
                    let mut resp = format!("HTTP/1.1 {} {}\r\n", status, status_text);
                    resp.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
                    resp.push_str("Connection: close\r\n");
                    for (k, v) in &extra {
                        let kl = k.to_lowercase();
                        if kl != "content-length" && kl != "connection" {
                            resp.push_str(&format!("{}: {}\r\n", k, v));
                        }
                    }
                    resp.push_str("\r\n");

                    let mut guard = conns.lock().unwrap();
                    let conn = guard
                        .get_mut(&conn_id)
                        .ok_or_else(|| js_code_err(&ctx, "ENOENT", "unknown conn_id"))?;
                    conn.stream
                        .write_all(resp.as_bytes())
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    conn.stream
                        .write_all(body_bytes)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    conn.stream.flush().ok();
                    drop(guard);

                    if let Some(entry) = conns.lock().unwrap().remove(&conn_id)
                        && let Some(firewall) = fw.as_ref().as_ref()
                        && let Ok(peer) = entry.stream.peer_addr()
                    {
                        firewall.on_disconnect(peer.ip());
                    }
                    Ok(())
                },
            ),
        )?;
    }

    // ── __httpRespondBytes ────────────────────────────────────────────────────
    {
        let conns = conns.clone();
        let fw = fw.clone();
        ctx.globals().set(
            "__httpRespondBytes",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      conn_id: u32,
                      status: u32,
                      status_text: String,
                      headers_json: String,
                      body: Vec<u8>|
                      -> Result<()> {
                    let extra = parse_extra_headers(&headers_json);
                    let mut resp = format!("HTTP/1.1 {} {}\r\n", status, status_text);
                    resp.push_str(&format!("Content-Length: {}\r\n", body.len()));
                    resp.push_str("Connection: close\r\n");
                    for (k, v) in &extra {
                        let kl = k.to_lowercase();
                        if kl != "content-length" && kl != "connection" && kl != "transfer-encoding"
                        {
                            resp.push_str(&format!("{}: {}\r\n", k, v));
                        }
                    }
                    resp.push_str("\r\n");

                    let mut guard = conns.lock().unwrap();
                    let conn = guard
                        .get_mut(&conn_id)
                        .ok_or_else(|| js_code_err(&ctx, "ENOENT", "unknown conn_id"))?;
                    conn.stream
                        .write_all(resp.as_bytes())
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    conn.stream
                        .write_all(&body)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    conn.stream.flush().ok();
                    drop(guard);

                    if let Some(entry) = conns.lock().unwrap().remove(&conn_id)
                        && let Some(firewall) = fw.as_ref().as_ref()
                        && let Ok(peer) = entry.stream.peer_addr()
                    {
                        firewall.on_disconnect(peer.ip());
                    }
                    Ok(())
                },
            ),
        )?;
    }

    // ── __httpClose ───────────────────────────────────────────────────────────
    {
        ctx.globals().set(
            "__httpClose",
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, server_id: u32| -> Result<()> {
                    servers.lock().unwrap().remove(&server_id);
                    Ok(())
                },
            ),
        )?;
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    // Bind a local listener, return (listener, client_stream).
    async fn loopback_pair() -> (TcpListener, tokio::net::TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        (listener, client)
    }

    #[tokio::test]
    async fn normal_post_body_accepted() {
        let (listener, mut client) = loopback_pair().await;
        let body = b"hello=world";
        let req = format!(
            "POST /test HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        tokio::spawn(async move {
            client.write_all(req.as_bytes()).await.unwrap();
            client.write_all(body).await.unwrap();
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let result = parse_request(
            server_stream,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            100,
            16_384,
            0,
            0,
        )
        .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().body, body);
    }

    #[tokio::test]
    async fn rudy_slow_drip_rejected_by_rate_check() {
        let (listener, mut client) = loopback_pair().await;
        // Declare 200 bytes but drip 1 byte every ~500ms → ~2 B/s, below 50 B/s min.
        let req = "POST /slow HTTP/1.1\r\nContent-Length: 200\r\n\r\n";
        tokio::spawn(async move {
            client.write_all(req.as_bytes()).await.unwrap();
            for _ in 0..200u8 {
                client.write_all(b"x").await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let result = parse_request(
            server_stream,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(30), // long deadline — rate check must fire first
            100,
            16_384,
            0,
            50, // min 50 B/s
        )
        .await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("rate too low") || msg.contains("stalled") || msg.contains("RUDY"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn body_total_deadline_fires_when_rate_check_disabled() {
        let (listener, mut client) = loopback_pair().await;
        // Declare 1000 bytes but never send them.
        let req = "POST /hang HTTP/1.1\r\nContent-Length: 1000\r\n\r\n";
        tokio::spawn(async move {
            client.write_all(req.as_bytes()).await.unwrap();
            // no body bytes sent
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let result = parse_request(
            server_stream,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(200), // short deadline
            100,
            16_384,
            0,
            0, // rate check disabled
        )
        .await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("timeout") || msg.contains("stalled") || msg.contains("deadline"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn get_request_no_body_accepted() {
        let (listener, mut client) = loopback_pair().await;
        tokio::spawn(async move {
            client
                .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await
                .unwrap();
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let result = parse_request(
            server_stream,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            100,
            16_384,
            0,
            100,
        )
        .await;
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.method, "GET");
        assert!(parsed.body.is_empty());
    }
}
