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
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::{Instant, timeout_at};

use vvva_firewall::{Firewall, FirewallDecision};
use vvva_permissions::PermissionState;

const HARD_MAX_BODY: usize = 100 * 1024 * 1024;

/// A request fully parsed and ready to be handed to JS. The connection task
/// keeps ownership of the (tokio) socket and forwards it back so the next
/// keep-alive request can be parsed on the same stream.
struct ReadyRequest {
    json: String,
}

/// The response produced by JS (`__httpRespond`/`__httpRespondBytes`), sent back
/// to the owning connection task over a oneshot channel.
struct ResponsePayload {
    head: Vec<u8>,
    body: Vec<u8>,
    /// Whether the socket should be kept alive for another request.
    keep_alive: bool,
}

/// Per-connection bookkeeping shared between the connection task and JS.
#[derive(Clone)]
struct ConnMeta {
    /// Whether the most recent request asked to keep the connection alive.
    keep_alive: bool,
    /// Number of requests already served on this TCP connection.
    requests_served: u32,
}

/// Live connections, keyed by `conn_id`. The value pairs the response sender
/// (the connection task is awaiting it) with the per-connection meta.
type ConnMap = Arc<std::sync::Mutex<HashMap<u32, (oneshot::Sender<ResponsePayload>, ConnMeta)>>>;

/// Per-server state: the listener plus a shutdown `Notify` (the accept task
/// waits on it) and the bounded queue of parsed requests for JS.
type Servers = Arc<std::sync::Mutex<HashMap<u32, (Arc<TcpListener>, Arc<Notify>)>>>;

/// Fixed limits for a single TCP connection's lifetime, resolved once from the
/// firewall config so the hot path stays allocation-free.
#[derive(Clone)]
struct ConnLimits {
    header_timeout: Duration,
    body_timeout: Duration,
    keepalive_timeout: Duration,
    max_header_count: usize,
    max_header_bytes: usize,
    max_body_bytes: usize,
    min_body_rate_bps: u32,
}

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
    tokio_stream: tokio::net::TcpStream,
    keep_alive_requested: bool,
}

/// Limits governing a single request parse, grouped so `parse_request` keeps a
/// small, stable signature.
struct ParseLimits {
    header_timeout: std::time::Duration,
    first_line_timeout: std::time::Duration,
    body_timeout: std::time::Duration,
    max_header_count: usize,
    max_header_bytes: usize,
    max_body_bytes: usize,
    min_body_rate_bps: u32,
}

async fn parse_request(
    stream: tokio::net::TcpStream,
    limits: ParseLimits,
) -> std::result::Result<ParsedRequest, String> {
    let mut reader = BufReader::new(stream);

    // A single monotonic deadline bounds the whole header phase (request line
    // + all headers). Using one absolute deadline (instead of re-arming a full
    // `limits.header_timeout` on every line) both closes a Slowloris hole — an
    // attacker can no longer reset the clock by trickling one byte per timeout
    // — and avoids allocating a fresh timer per header line.
    let header_deadline = Instant::now() + limits.header_timeout;
    // For a *reused* (keep-alive) connection the wait for the next request line
    // is bounded by a separate, shorter idle timeout so an idle socket can't be
    // held open indefinitely (Slowloris on keep-alive). For a freshly accepted
    // connection this equals `limits.header_timeout`.
    let first_line_deadline = Instant::now() + limits.first_line_timeout;

    // Request line — timeout protects against connections that never send data.
    let mut request_line = String::new();
    let first_line_n = timeout_at(first_line_deadline, reader.read_line(&mut request_line))
        .await
        .map_err(|_| "timeout: request line not received in time (Slowloris?)")?
        .map_err(|e| e.to_string())?;
    // Clean EOF (client closed the connection without sending a request): treat
    // as a silent connection close, not a parse error, so the accept loop can
    // `continue` without surfacing `server.emit('error')` to JS.
    if first_line_n == 0 {
        return Err("eof: client closed connection before sending request".into());
    }

    let request_line = request_line.trim_end_matches(['\r', '\n']);
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let version = parts.next().unwrap_or("").to_string();

    // Headers — all reads share the single header deadline above.
    let max_body = if limits.max_body_bytes == 0 {
        HARD_MAX_BODY
    } else {
        limits.max_body_bytes
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length: usize = 0;
    let mut total_header_bytes: usize = 0;

    loop {
        let mut line = String::new();
        timeout_at(header_deadline, reader.read_line(&mut line))
            .await
            .map_err(|_| "timeout: headers not received in time (Slowloris?)")?
            .map_err(|e| e.to_string())?;

        total_header_bytes += line.len();
        if total_header_bytes > limits.max_header_bytes {
            return Err(format!(
                "header flood: exceeded {} bytes",
                limits.max_header_bytes
            ));
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if headers.len() >= limits.max_header_count {
            return Err(format!(
                "header flood: more than {} headers",
                limits.max_header_count
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

    // Determine whether the client wants to keep the connection alive. Default
    // follows HTTP/1.1 (keep-alive) vs HTTP/1.0 (close); an explicit
    // `Connection` header overrides that.
    let mut keep_alive_requested = version.to_ascii_uppercase().starts_with("HTTP/1.1");
    for (name, value) in &headers {
        if name == "connection" {
            let v = value.to_ascii_lowercase();
            if v.contains("close") {
                keep_alive_requested = false;
            } else if v.contains("keep-alive") {
                keep_alive_requested = true;
            }
        }
    }

    // Body — two-layer RUDY defense:
    //   1. Total deadline (limits.body_timeout) — connection dies if body never finishes.
    //   2. Minimum rate (limits.min_body_rate_bps) — connection dies if drip rate is too slow,
    //      even when data arrives just fast enough to beat the total deadline.
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let deadline = Instant::now() + limits.body_timeout;
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
            if limits.min_body_rate_bps > 0 {
                let elapsed = body_start.elapsed().as_secs_f64();
                if elapsed > 2.0 {
                    let rate = received as f64 / elapsed;
                    if rate < limits.min_body_rate_bps as f64 {
                        return Err(format!(
                            "body rate too low ({:.0} B/s < {} B/s min): RUDY?",
                            rate, limits.min_body_rate_bps
                        ));
                    }
                }
            }
        }
    }

    // If the buffered reader still holds unread bytes (a pipelined or coalesced
    // next request sitting in the same TCP segment), we cannot safely carry them
    // into the next keep-alive parse — `into_inner` would drop them. Force this
    // connection closed so we never hang the client mid-pipeline; the caller must
    // open a fresh connection for the buffered request.
    if !reader.buffer().is_empty() {
        keep_alive_requested = false;
    }

    let tokio_stream = reader.into_inner();
    Ok(ParsedRequest {
        method,
        path,
        headers,
        body,
        tokio_stream,
        keep_alive_requested,
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

/// Write an HTTP status/headers line + body directly to the socket (used for
/// firewall-issued rejections that never reach JS).
async fn write_reject_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    msg: &'static str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {msg}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{msg}",
        len = msg.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

/// Write a ready response (head + body) produced by JS to the socket.
async fn write_response(
    stream: &mut tokio::net::TcpStream,
    head: &[u8],
    body: &[u8],
) -> std::io::Result<()> {
    stream.write_all(head).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

/// Owns a single TCP connection for its whole lifetime. Parses request after
/// request, enqueues each into the per-server `ready_tx` channel for JS to
/// consume, awaits the response JS produces, writes it, and (for keep-alive)
/// loops. The firewall's `on_connect`/`on_disconnect` are each called exactly
/// once — on connect (in the accept task) and on the single exit path below.
async fn connection_task(
    stream: tokio::net::TcpStream,
    ip: IpAddr,
    ready_tx: mpsc::Sender<ReadyRequest>,
    conns: ConnMap,
    fw: Arc<Option<Arc<Firewall>>>,
    limits: ConnLimits,
    next_conn_id: Arc<std::sync::Mutex<u32>>,
) {
    let mut meta = ConnMeta {
        keep_alive: false,
        requests_served: 0,
    };
    let mut stream = stream;

    loop {
        // First request line on a reused keep-alive socket is bounded by the
        // short idle timeout; a fresh connection uses the full header timeout.
        let first_line_timeout = if meta.requests_served == 0 {
            limits.header_timeout
        } else {
            limits.keepalive_timeout
        };

        let parsed = parse_request(
            stream,
            ParseLimits {
                header_timeout: limits.header_timeout,
                first_line_timeout,
                body_timeout: limits.body_timeout,
                max_header_count: limits.max_header_count,
                max_header_bytes: limits.max_header_bytes,
                max_body_bytes: limits.max_body_bytes,
                min_body_rate_bps: limits.min_body_rate_bps,
            },
        )
        .await;

        let parsed = match parsed {
            Ok(p) => p,
            // Malformed request, idle timeout, or clean client EOF: the
            // connection is dead. Tear it down once via on_disconnect.
            Err(_) => {
                if let Some(firewall) = fw.as_ref().as_ref() {
                    firewall.on_disconnect(ip);
                }
                return;
            }
        };

        // Reclaim the socket for this connection before any further writes.
        stream = parsed.tokio_stream;

        // ── Firewall: per-request rate limit (runs every request) ──
        if let Some(firewall) = fw.as_ref().as_ref() {
            match firewall.check_request(ip) {
                FirewallDecision::Allow => {}
                decision => {
                    let status = decision.http_status();
                    let msg = decision.message();
                    if write_reject_response(&mut stream, status, msg)
                        .await
                        .is_err()
                    {
                        firewall.on_disconnect(ip);
                        return;
                    }
                    firewall.on_disconnect(ip);
                    return;
                }
            }
        }

        meta.requests_served += 1;
        meta.keep_alive = parsed.keep_alive_requested;

        // Allocate a conn_id and register the response sender so JS can reply.
        let conn_id = {
            let mut n = next_conn_id.lock().unwrap();
            let id = *n;
            *n = n.wrapping_add(1);
            id
        };
        let (tx_resp, rx_resp) = oneshot::channel();
        conns
            .lock()
            .unwrap()
            .insert(conn_id, (tx_resp, meta.clone()));

        let hdr_pairs: Vec<String> = parsed
            .headers
            .iter()
            .map(|(k, v)| format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)))
            .collect();
        let body_str = String::from_utf8_lossy(&parsed.body);
        let json = format!(
            "{{\"method\":\"{m}\",\"url\":\"{u}\",\"headers\":{{{h}}},\
             \"body\":\"{b}\",\"conn_id\":{c},\"remoteAddress\":\"{ip}\"}}",
            m = json_escape(&parsed.method),
            u = json_escape(&parsed.path),
            h = hdr_pairs.join(","),
            b = json_escape(&body_str),
            c = conn_id,
            ip = ip,
        );

        // Hand the parsed request to JS. A send failure means the server was
        // closed (no receiver) — tear the connection down.
        if ready_tx.send(ReadyRequest { json }).await.is_err() {
            conns.lock().unwrap().remove(&conn_id);
            if let Some(firewall) = fw.as_ref().as_ref() {
                firewall.on_disconnect(ip);
            }
            return;
        }

        // Block until JS produces a response (or the connection is dropped).
        // Block until JS produces a response (or the connection is dropped). A
        // handler that never calls __httpRespond would otherwise hang this
        // connection task forever, leaking the socket and the firewall's
        // connection count; bound it by the body deadline.
        let payload = match tokio::time::timeout(limits.body_timeout, rx_resp).await {
            Ok(Ok(p)) => p,
            Ok(Err(_)) | Err(_) => {
                if let Some(firewall) = fw.as_ref().as_ref() {
                    firewall.on_disconnect(ip);
                }
                return;
            }
        };

        if write_response(&mut stream, &payload.head, &payload.body)
            .await
            .is_err()
        {
            if let Some(firewall) = fw.as_ref().as_ref() {
                firewall.on_disconnect(ip);
            }
            return;
        }

        // `keep_alive` was decided by __httpRespond (client preference +
        // per-connection request cap) and echoed back here. A closed socket
        // ends the connection; otherwise we parse the next request.
        if !payload.keep_alive {
            if let Some(firewall) = fw.as_ref().as_ref() {
                firewall.on_disconnect(ip);
            }
            return;
        }
    }
}

// ── Injection ──────────────────────────────────────────────────────────────────

pub fn inject_http_server(
    ctx: &Ctx,
    permissions: Arc<PermissionState>,
    firewall: Option<Arc<Firewall>>,
) -> Result<()> {
    // Per-server state:
    //   * The listener plus a shutdown `Notify` (the accept task waits on it).
    //   * A bounded mpsc of "ready" requests, drained by JS via `__httpAcceptAsync`.
    let servers: Servers = Arc::new(Mutex::new(HashMap::new()));
    let ready_rx: Arc<Mutex<HashMap<u32, mpsc::Receiver<ReadyRequest>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Live connections awaiting their JS-produced response, keyed by `conn_id`.
    let conns: ConnMap = Arc::new(Mutex::new(HashMap::new()));
    let next_server_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let next_conn_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
    let fw: Arc<Option<Arc<Firewall>>> = Arc::new(firewall);

    // Resolve firewall limits once (fixed for the process lifetime). Capturing
    // the scalars keeps the hot path allocation-free.
    let (
        header_timeout_ms,
        body_timeout_ms,
        keepalive_timeout_ms,
        max_header_count,
        max_header_bytes,
        max_body_bytes,
        min_body_rate_bps,
        max_requests_per_conn,
    ): (u64, u64, u64, usize, usize, usize, u32, u32) = match fw.as_ref().as_ref() {
        Some(firewall) => {
            let c = &firewall.config;
            (
                c.header_timeout_ms,
                c.body_timeout_ms,
                c.keepalive_timeout_ms,
                c.max_header_count as usize,
                c.max_header_bytes as usize,
                c.max_body_bytes as usize,
                c.min_body_rate_bps,
                c.max_requests_per_conn,
            )
        }
        None => (10_000, 30_000, 5_000, 100, 16_384, 0, 100, 1_000),
    };

    let limits = ConnLimits {
        header_timeout: Duration::from_millis(header_timeout_ms),
        body_timeout: Duration::from_millis(body_timeout_ms),
        keepalive_timeout: Duration::from_millis(keepalive_timeout_ms),
        max_header_count,
        max_header_bytes,
        max_body_bytes,
        min_body_rate_bps,
    };

    // ── __httpListen ──────────────────────────────────────────────────────────
    {
        let perms = permissions.clone();
        let servers = servers.clone();
        let ready_rx = ready_rx.clone();
        let nid = next_server_id.clone();
        let fw = fw.clone();
        let limits = limits.clone();
        let next_conn_id = next_conn_id.clone();
        let conns = conns.clone();
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
                    let tokio_listener = Arc::new(tokio_listener);

                    let id = {
                        let mut n = nid.lock().unwrap();
                        let id = *n;
                        *n = n.wrapping_add(1);
                        id
                    };

                    // Bounded queue of parsed requests; the JS accept loop pulls
                    // from it. 1024 is the headroom before the accept task starts
                    // to apply back-pressure on parsing.
                    let (ready_tx, ready_rx_ch) = mpsc::channel::<ReadyRequest>(1024);
                    let shutdown = Arc::new(Notify::new());

                    // Insert the request queue first, then the server entry. The
                    // lock order is ready_rx → servers everywhere, which avoids a
                    // deadlock with __httpAcceptAsync / __httpClose.
                    ready_rx.lock().unwrap().insert(id, ready_rx_ch);
                    servers
                        .lock()
                        .unwrap()
                        .insert(id, (tokio_listener.clone(), shutdown.clone()));

                    // Start background blocklist/bucket cleanup once.
                    if id == 0
                        && let Some(firewall) = fw.as_ref().as_ref()
                    {
                        vvva_firewall::spawn_cleanup_task(
                            firewall.clone(),
                            std::time::Duration::from_secs(60),
                        );
                    }

                    // ── Accept task: one per server ──
                    // Sole owner of `accept()`. For each connection it runs the
                    // firewall connection gate ONCE, then spawns a connection task
                    // that owns the socket for its whole lifetime. This removes the
                    // single-outstanding serialization that blocked `__httpAcceptAsync`
                    // on an idle keep-alive socket.
                    {
                        let listener = tokio_listener.clone();
                        let shutdown = shutdown.clone();
                        let fw = fw.clone();
                        let ready_tx = ready_tx.clone();
                        let conns = conns.clone();
                        let limits = limits.clone();
                        let next_conn_id = next_conn_id.clone();
                        tokio::spawn(async move {
                            loop {
                                tokio::select! {
                                    _ = shutdown.notified() => break,
                                    accept = listener.accept() => {
                                        let (stream, peer_addr) = match accept {
                                            Ok(v) => v,
                                            Err(_) => break, // listener gone
                                        };
                                        let ip: IpAddr = match peer_addr {
                                            SocketAddr::V4(a) => IpAddr::V4(*a.ip()),
                                            SocketAddr::V6(a) => IpAddr::V6(*a.ip()),
                                        };
                                        // ── Firewall: connection gate (ONCE per TCP conn) ──
                                        if let Some(firewall) = fw.as_ref().as_ref() {
                                            match firewall.check_connection(ip) {
                                                FirewallDecision::Allow => firewall.on_connect(ip),
                                                decision => {
                                                    reject_stream(
                                                        stream,
                                                        decision.http_status(),
                                                        decision.message(),
                                                    );
                                                    continue;
                                                }
                                            }
                                        }
                                        tokio::spawn(connection_task(
                                            stream,
                                            ip,
                                            ready_tx.clone(),
                                            conns.clone(),
                                            fw.clone(),
                                            limits.clone(),
                                            next_conn_id.clone(),
                                        ));
                                    }
                                }
                            }
                            // Drop our sender clone so a stalled __httpAcceptAsync
                            // observes channel closure once in-flight connection
                            // tasks finish, instead of hanging on recv().
                            drop(ready_tx);
                        });
                    }

                    Ok(id)
                },
            ),
        )?;
    }

    // ── __httpAcceptAsync ────────────────────────────────────────────────────
    {
        let servers = servers.clone();
        let ready_rx = ready_rx.clone();
        // Pull one ready request for `server_id`. Routine connection endings
        // (idle timeout, malformed, rate-limited) are handled inside the
        // connection task and never reach here, so this only errors on a real
        // problem: unknown server id, or the server being closed.
        ctx.globals().set(
            "__httpAcceptAsync",
            Function::new(
                ctx.clone(),
                Async(move |server_id: u32| {
                    let servers = servers.clone();
                    let ready_rx = ready_rx.clone();
                    async move {
                        // Take the receiver out of the map so we never hold the lock
                        // across `.recv().await` (which would deadlock with __httpClose).
                        let (mut rx, shutdown) = {
                            let mut map = ready_rx.lock().unwrap();
                            let rx = map.remove(&server_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown server id",
                                )
                            })?;
                            let shutdown = servers
                                .lock()
                                .unwrap()
                                .get(&server_id)
                                .map(|(_, n)| n.clone());
                            (rx, shutdown)
                        };

                        // Wait for a parsed request, or for the server to be closed.
                        let ready = tokio::select! {
                            r = rx.recv() => r,
                            _ = async {
                                match &shutdown {
                                    Some(n) => n.notified().await,
                                    None => std::future::pending::<()>().await,
                                }
                            } => None,
                        };

                        // Put the receiver back only if the server still exists —
                        // __httpClose may have removed it (and notified the accept
                        // task) while we were awaiting, and we must not resurrect it.
                        {
                            let mut map = ready_rx.lock().unwrap();
                            if servers.lock().unwrap().contains_key(&server_id) {
                                map.entry(server_id).or_insert(rx);
                            }
                        }

                        match ready {
                            Some(req) => Ok::<String, rquickjs::Error>(req.json),
                            None => Err(rquickjs::Error::new_from_js_message(
                                "ENOENT",
                                "ENOENT",
                                "server closed or no pending request",
                            )),
                        }
                    }
                }),
            ),
        )?;
    }

    // ── __httpRespond ─────────────────────────────────────────────────────────
    {
        let conns = conns.clone();
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

                    // The connection task owns the socket and the firewall
                    // on_disconnect call; here we only decide keep-alive and ship
                    // the head/body over the oneshot. `close` honours the client's
                    // preference AND the per-connection request cap.
                    let mut guard = conns.lock().unwrap();
                    let (tx, meta) = guard
                        .remove(&conn_id)
                        .ok_or_else(|| js_code_err(&ctx, "ENOENT", "unknown conn_id"))?;
                    drop(guard);

                    let close = !meta.keep_alive || meta.requests_served >= max_requests_per_conn;

                    if close {
                        resp.push_str("Connection: close\r\n");
                    } else {
                        resp.push_str("Connection: keep-alive\r\n");
                    }
                    for (k, v) in &extra {
                        let kl = k.to_lowercase();
                        if kl != "content-length" && kl != "connection" {
                            resp.push_str(&format!("{}: {}\r\n", k, v));
                        }
                    }
                    resp.push_str("\r\n");

                    let payload = ResponsePayload {
                        head: resp.into_bytes(),
                        body: body_bytes.to_vec(),
                        keep_alive: !close,
                    };
                    if tx.send(payload).is_err() {
                        return Err(js_err(
                            &ctx,
                            "client connection closed before response sent".into(),
                        ));
                    }
                    Ok(())
                },
            ),
        )?;
    }

    // ── __httpRespondBytes ────────────────────────────────────────────────────
    {
        let conns = conns.clone();
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

                    let mut guard = conns.lock().unwrap();
                    let (tx, meta) = guard
                        .remove(&conn_id)
                        .ok_or_else(|| js_code_err(&ctx, "ENOENT", "unknown conn_id"))?;
                    drop(guard);

                    let close = !meta.keep_alive || meta.requests_served >= max_requests_per_conn;

                    if close {
                        resp.push_str("Connection: close\r\n");
                    } else {
                        resp.push_str("Connection: keep-alive\r\n");
                    }
                    for (k, v) in &extra {
                        let kl = k.to_lowercase();
                        if kl != "content-length" && kl != "connection" && kl != "transfer-encoding"
                        {
                            resp.push_str(&format!("{}: {}\r\n", k, v));
                        }
                    }
                    resp.push_str("\r\n");

                    let payload = ResponsePayload {
                        head: resp.into_bytes(),
                        body,
                        keep_alive: !close,
                    };
                    if tx.send(payload).is_err() {
                        return Err(js_err(
                            &ctx,
                            "client connection closed before response sent".into(),
                        ));
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
                    // Forget any queued-but-unaccepted requests first (ready_rx
                    // lock), then wake the accept task via its shutdown Notify so
                    // it stops accepting and drops its listener + ready_tx clones.
                    // `notify_one` stores a permit so the task's `notified()`
                    // future finds it on re-poll and breaks (unlike
                    // `notify_waiters`, which would re-arm the wait and hang).
                    // Lock order is ready_rx → servers everywhere to avoid a
                    // deadlock with __httpAcceptAsync.
                    ready_rx.lock().unwrap().remove(&server_id);
                    if let Some((_listener, shutdown)) = servers.lock().unwrap().remove(&server_id)
                    {
                        shutdown.notify_one();
                    }
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
            ParseLimits {
                header_timeout: std::time::Duration::from_secs(5),
                first_line_timeout: std::time::Duration::from_secs(5),
                body_timeout: std::time::Duration::from_secs(5),
                max_header_count: 100,
                max_header_bytes: 16_384,
                max_body_bytes: 0,
                min_body_rate_bps: 0,
            },
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
            ParseLimits {
                header_timeout: std::time::Duration::from_secs(5),
                first_line_timeout: std::time::Duration::from_secs(5),
                body_timeout: std::time::Duration::from_secs(30), // long deadline — rate check must fire first
                max_header_count: 100,
                max_header_bytes: 16_384,
                max_body_bytes: 0,
                min_body_rate_bps: 50, // min 50 B/s
            },
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
            ParseLimits {
                header_timeout: std::time::Duration::from_secs(5),
                first_line_timeout: std::time::Duration::from_secs(5),
                body_timeout: std::time::Duration::from_millis(200), // short deadline
                max_header_count: 100,
                max_header_bytes: 16_384,
                max_body_bytes: 0,
                min_body_rate_bps: 0, // rate check disabled
            },
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
            ParseLimits {
                header_timeout: std::time::Duration::from_secs(5),
                first_line_timeout: std::time::Duration::from_secs(5),
                body_timeout: std::time::Duration::from_secs(5),
                max_header_count: 100,
                max_header_bytes: 16_384,
                max_body_bytes: 0,
                min_body_rate_bps: 100,
            },
        )
        .await;
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.method, "GET");
        assert!(parsed.body.is_empty());
    }
}
