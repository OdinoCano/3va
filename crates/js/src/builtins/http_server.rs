// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! HTTP/1.1 server backend for `http.createServer()`.

use crate::builtins::NativeCtxRegistry;
use crate::builtins::v8_compat::uint8array_to_vec;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::Instant;
use v8::{FunctionCallbackArguments, PinScope, ReturnValue, Script, String as V8String};

use vvva_firewall::{Firewall, FirewallDecision};
use vvva_permissions::PermissionState;

const HARD_MAX_BODY: usize = 100 * 1024 * 1024;

/// Count of currently-open `http.createServer().listen()` / http2 listeners
/// (HTTP/1 and HTTP/2 both bump this — there's no need to tell them apart,
/// only whether *any* server is listening). Like libuv's active-handle
/// count: an open listener has to keep `run_event_loop` alive while idle,
/// since "waiting for the next connection" isn't a pending timer or task.
static HTTP_ACTIVE_LISTENERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub fn has_active_listeners() -> bool {
    HTTP_ACTIVE_LISTENERS.load(std::sync::atomic::Ordering::Relaxed) > 0
}

/// Requests that finished parsing, waiting to be picked up by JS, keyed by
/// server id. Populated by the background accept task spawned in
/// `__httpListen`; drained by the non-blocking `__httpAcceptPoll`.
type ReadyQueue = Arc<Mutex<HashMap<u32, std::collections::VecDeque<ReadyRequest>>>>;

/// A parsed request waiting for JS. Handed over as plain values (see
/// `__httpAcceptPoll`) rather than a JSON string JS would have to re-parse.
struct ReadyRequest {
    parsed: ParsedRequest,
    conn_id: u32,
    remote: IpAddr,
}

/// Per-isolate dispatch state (an isolate slot): connection tasks push into
/// `ready` and signal `wake`; `run_event_loop` waits on `wake` instead of
/// polling on a timer, then `dispatch_ready` calls each server's JS handler.
/// Each handler runs with the package deny scopes captured when it was
/// registered, the same way timers re-apply theirs (VULN-03).
struct HttpDispatch {
    ready: ReadyQueue,
    upgrades: UpgradeQueue,
    wake: Arc<tokio::sync::Notify>,
    handlers: HashMap<u32, (v8::Global<v8::Function>, Vec<String>)>,
}

/// The notifier `run_event_loop` waits on, if this isolate has an HTTP server
/// backend.
pub fn wake_handle(isolate: &v8::Isolate) -> Option<Arc<tokio::sync::Notify>> {
    isolate.get_slot::<HttpDispatch>().map(|d| d.wake.clone())
}

/// Whether any server with a JS handler still has requests queued (the
/// handler takes a bounded batch per call).
pub fn has_ready(isolate: &v8::Isolate) -> bool {
    isolate.get_slot::<HttpDispatch>().is_some_and(|d| {
        let ready = d.ready.lock().unwrap();
        let upgrades = d.upgrades.lock().unwrap();
        d.handlers
            .keys()
            .any(|id| ready.get(id).is_some_and(|q| !q.is_empty()))
            || d.handlers
                .keys()
                .any(|id| upgrades.get(id).is_some_and(|q| !q.is_empty()))
    })
}

/// Calls the JS handler of every server with parsed requests waiting.
/// Returns whether any handler ran.
pub fn dispatch_ready(scope: &mut v8::ContextScope<v8::HandleScope>) -> anyhow::Result<bool> {
    let pending: Vec<(v8::Global<v8::Function>, Vec<String>)> = {
        let Some(d) = scope.get_slot::<HttpDispatch>() else {
            return Ok(false);
        };
        let ready = d.ready.lock().unwrap();
        let upgrades = d.upgrades.lock().unwrap();
        d.handlers
            .iter()
            .filter(|(id, _)| {
                ready.get(id).is_some_and(|q| !q.is_empty())
                    || upgrades.get(id).is_some_and(|q| !q.is_empty())
            })
            .map(|(_, h)| h.clone())
            .collect()
    };
    let ran = !pending.is_empty();
    let mut first_error: Option<String> = None;
    // Each handler call takes a batch of up to 64 requests; microtasks run
    // between batches (so each batch's objects die young), up to 16 rounds
    // before returning to the event loop so timers still get their turn.
    for _ in 0..16 {
        for (handler, deny_scopes) in &pending {
            v8::tc_scope!(let try_catch, scope);
            let f = v8::Local::new(try_catch, handler);
            let recv: v8::Local<v8::Value> = v8::undefined(try_catch).into();
            let prev = vvva_permissions::set_inherited_scopes(deny_scopes.clone());
            let ok = f.call(try_catch, recv, &[]).is_some();
            vvva_permissions::set_inherited_scopes(prev);
            if !ok {
                let text = try_catch
                    .stack_trace()
                    .or_else(|| try_catch.exception())
                    .map(|e| e.to_rust_string_lossy(try_catch))
                    .unwrap_or_else(|| "unknown error".to_string());
                first_error.get_or_insert(text);
            }
        }
        scope.perform_microtask_checkpoint();
        if first_error.is_some() || !has_ready(scope) {
            break;
        }
    }
    match first_error {
        Some(text) => anyhow::bail!("Uncaught exception: {text}"),
        None => Ok(ran),
    }
}

struct HttpListenCtx {
    perms: Arc<PermissionState>,
    servers: Arc<Mutex<HashMap<u32, Arc<TcpListener>>>>,
    nid: Arc<Mutex<u32>>,
    fw: Arc<Option<Arc<Firewall>>>,
    conns: Arc<Mutex<HashMap<u32, ConnEntry>>>,
    conn_nid: Arc<Mutex<u32>>,
    ready: ReadyQueue,
    upgrades: UpgradeRegistry,
    upgrades_q: UpgradeQueue,
    wake: Arc<tokio::sync::Notify>,
}

struct HttpAcceptCtx {
    ready: ReadyQueue,
}

struct HttpUpgradeCtx {
    upgrades: UpgradeRegistry,
    upgrades_q: UpgradeQueue,
}

struct HttpRespondCtx {
    conns: Arc<Mutex<HashMap<u32, ConnEntry>>>,
}

// ── HTTP `Upgrade` (WebSocket) ──────────────────────────────────────────────
//
// A request with `Connection: upgrade` + an `Upgrade:` header is handed to JS
// as `server.on('upgrade', (req, socket, head))`, exactly like Node: the http
// layer does not answer it (the `ws` npm client/server library performs the
// 101 handshake and drives the frames over the raw socket). The raw duplex
// socket is bridged to JS by a per-connection driver task that owns the
// stream and pushes read chunks into / drains writes from shared queues, so
// JS sees the same pollable socket shape the `net` module uses. The firewall
// already admitted the connection and request; the driver releases the
// per-IP slot on close. A server-side frame codec / message-size limit is
// the `ws` library's job in this design (its `maxPayload`), not the http
// layer's.

/// One bridged upgrade socket, shared between its driver task (tokio thread)
/// and the JS natives (V8 thread).
struct UpgradeConn {
    read_q: Mutex<std::collections::VecDeque<UpgradeRead>>,
    write_q: Mutex<std::collections::VecDeque<Vec<u8>>>,
    write_wake: tokio::sync::Notify,
    data_wake: tokio::sync::Notify,
    closed: std::sync::atomic::AtomicBool,
    /// Set when the connection is dropped without flushing what JS queued
    /// (the write queue overflowed because the peer isn't reading).
    aborted: std::sync::atomic::AtomicBool,
    /// Bytes currently held in `read_q` / `write_q`.
    read_bytes: std::sync::atomic::AtomicUsize,
    write_bytes: std::sync::atomic::AtomicUsize,
    /// Signalled when JS drains `read_q`, so a paused driver resumes reading.
    drained: tokio::sync::Notify,
}

/// Most bytes the driver reads ahead of JS. At this point it stops reading
/// the socket, so TCP flow control pushes back on the peer instead of the
/// process buffering whatever a fast client sends while JS is busy.
const UPGRADE_READ_HIGH_WATER: usize = 1024 * 1024;

/// Most bytes JS may have queued for the driver to write. A peer that stops
/// reading while the app keeps sending (a chat broadcast to a stalled
/// client) would otherwise grow this without bound; past the limit the
/// connection is dropped, as WebSocket servers do for slow consumers.
const UPGRADE_WRITE_HARD_LIMIT: usize = 32 * 1024 * 1024;

/// A write that makes no progress for this long means the peer is gone or
/// stalled: the connection is closed so it can't hold its firewall slot.
const UPGRADE_WRITE_STALL: std::time::Duration = std::time::Duration::from_secs(60);

impl UpgradeConn {
    fn new() -> Self {
        Self {
            read_q: Mutex::new(std::collections::VecDeque::new()),
            write_q: Mutex::new(std::collections::VecDeque::new()),
            write_wake: tokio::sync::Notify::new(),
            data_wake: tokio::sync::Notify::new(),
            closed: std::sync::atomic::AtomicBool::new(false),
            aborted: std::sync::atomic::AtomicBool::new(false),
            read_bytes: std::sync::atomic::AtomicUsize::new(0),
            write_bytes: std::sync::atomic::AtomicUsize::new(0),
            drained: tokio::sync::Notify::new(),
        }
    }
}

enum UpgradeTake {
    Data(Vec<u8>),
    Eof,
    Empty,
}

/// Takes up to `max` bytes from the front of the read queue. When JS asks for
/// less than the chunk holds, the rest stays queued, in order (it used to be
/// dropped), and the driver is woken if it had paused for lack of room.
fn take_upgrade_read(c: &UpgradeConn, max: usize) -> UpgradeTake {
    use std::sync::atomic::Ordering::Relaxed;
    let mut q = c.read_q.lock().unwrap();
    match q.pop_front() {
        Some(UpgradeRead::Data(mut bytes)) => {
            if bytes.len() > max {
                let rest = bytes.split_off(max);
                q.push_front(UpgradeRead::Data(rest));
            }
            drop(q);
            c.read_bytes.fetch_sub(bytes.len(), Relaxed);
            c.drained.notify_one();
            UpgradeTake::Data(bytes)
        }
        Some(UpgradeRead::Eof) => UpgradeTake::Eof,
        None if c.closed.load(Relaxed) => UpgradeTake::Eof,
        None => UpgradeTake::Empty,
    }
}

/// Queues `bytes` for the driver to write. Returns false (and aborts the
/// connection) when the peer has not read for so long that the queue would
/// pass [`UPGRADE_WRITE_HARD_LIMIT`].
fn queue_upgrade_write(conn: &UpgradeConn, bytes: Vec<u8>) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let n = bytes.len();
    if conn.write_bytes.fetch_add(n, Relaxed) + n > UPGRADE_WRITE_HARD_LIMIT {
        conn.write_bytes.fetch_sub(n, Relaxed);
        conn.aborted.store(true, Relaxed);
        conn.closed.store(true, Relaxed);
        conn.write_wake.notify_one();
        return false;
    }
    conn.write_q.lock().unwrap().push_back(bytes);
    conn.write_wake.notify_one();
    true
}

enum UpgradeRead {
    Data(Vec<u8>),
    Eof,
}

/// An upgrade request waiting for JS, like `ReadyRequest` but for the
/// `upgrade` event. Carries the bytes already read past the headers (`head`)
/// so the first thing the WS library sees is exactly what the client sent.
struct UpgradeRequest {
    parsed: ParsedRequest,
    conn_id: u32,
    remote: IpAddr,
    head: Vec<u8>,
}

type UpgradeQueue = Arc<Mutex<HashMap<u32, std::collections::VecDeque<UpgradeRequest>>>>;
type UpgradeRegistry = Arc<Mutex<HashMap<u32, Arc<UpgradeConn>>>>;

/// Whether `parsed` is an HTTP upgrade request (any `Upgrade:` target — the
/// `ws` library filters for `websocket` itself).
fn is_upgrade_request(parsed: &ParsedRequest) -> bool {
    let conn_upgrade = parsed.headers.iter().any(|(k, v)| {
        k == "connection"
            && v.to_ascii_lowercase()
                .split(',')
                .any(|tok| tok.trim() == "upgrade")
    });
    conn_upgrade && parsed.headers.iter().any(|(k, _)| k == "upgrade")
}

/// The task that owns an upgraded stream for its lifetime: pushes every byte
/// read into `read_q`, drains JS-side writes from `write_q`, and on
/// EOF/error/close pushes `Eof` and releases the firewall slot. The bytes the
/// http parser had already read past the headers (`head`) are NOT replayed
/// here: they travel in the `upgrade` event, as in Node, and the `ws` library
/// unshifts them itself. Runs until the peer or JS closes the socket.
fn spawn_upgrade_driver<S>(
    mut stream: S,
    ip: IpAddr,
    conn_id: u32,
    conn: Arc<UpgradeConn>,
    upgrades: UpgradeRegistry,
    fw: Arc<Option<Arc<Firewall>>>,
) -> tokio::task::JoinHandle<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use std::sync::atomic::Ordering::Relaxed;
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;

        let push_eof = || {
            conn.read_q.lock().unwrap().push_back(UpgradeRead::Eof);
            conn.data_wake.notify_one();
        };

        let mut buf = [0u8; 16_384];
        'io: loop {
            // Flush what JS wrote first — also when it has already asked to
            // close: `socket.end(data)` is write-then-close, and the last
            // frame (a WebSocket close echo) must not be lost to the close.
            if conn.aborted.load(Relaxed) {
                conn.write_q.lock().unwrap().clear();
                break;
            }
            loop {
                let next = conn.write_q.lock().unwrap().pop_front();
                let Some(bytes) = next else { break };
                let len = bytes.len();
                let written =
                    tokio::time::timeout(UPGRADE_WRITE_STALL, stream.write_all(&bytes)).await;
                conn.write_bytes.fetch_sub(len, Relaxed);
                if !matches!(written, Ok(Ok(()))) {
                    push_eof();
                    conn.closed.store(true, Relaxed);
                    break 'io;
                }
            }
            if conn.closed.load(Relaxed) {
                break;
            }
            // Past the high-water mark stop reading until JS drains: TCP
            // flow control then slows the peer instead of us buffering it.
            let paused = conn.read_bytes.load(Relaxed) >= UPGRADE_READ_HIGH_WATER;
            tokio::select! {
                _ = conn.write_wake.notified() => {}
                _ = conn.drained.notified(), if paused => {}
                r = stream.read(&mut buf), if !paused => match r {
                    Ok(0) | Err(_) => {
                        push_eof();
                        break;
                    }
                    Ok(n) => {
                        conn.read_bytes.fetch_add(n, Relaxed);
                        conn.read_q
                            .lock()
                            .unwrap()
                            .push_back(UpgradeRead::Data(buf[..n].to_vec()));
                        conn.data_wake.notify_one();
                    }
                },
            }
        }

        // Send the FIN (and a TLS close_notify) rather than just dropping.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), stream.shutdown()).await;
        upgrades.lock().unwrap().remove(&conn_id);
        if let Some(firewall) = fw.as_ref().as_ref() {
            firewall.on_disconnect(ip);
        }
    })
}

/// Binds a listening socket like `std::net::TcpListener::bind` (same name
/// resolution, first address that binds wins) but with a 1024 backlog: std
/// hardcodes 128, so a burst of more simultaneous connects than that had
/// SYNs dropped and waited out a 1 s retransmit (Node uses 511).
pub(crate) fn bind_listener(addr: &str) -> std::io::Result<std::net::TcpListener> {
    use std::net::ToSocketAddrs;
    let mut last_err = None;
    for sockaddr in addr.to_socket_addrs()? {
        match bind_one(sockaddr) {
            Ok(l) => return Ok(l),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("could not resolve to any addresses: {addr}"),
        )
    }))
}

fn bind_one(sockaddr: SocketAddr) -> std::io::Result<std::net::TcpListener> {
    let socket = socket2::Socket::new(
        socket2::Domain::for_address(sockaddr),
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    // What std sets on Unix, so a restarted server can rebind at once.
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    if std::env::var_os("VVVA_CLUSTER").is_some() {
        socket.set_reuse_port(true)?;
    }
    socket.bind(&sockaddr.into())?;
    socket.listen(1024)?;
    Ok(socket.into())
}

struct ConnEntry {
    resp_tx: mpsc::Sender<PendingResponse>,
}

struct PendingResponse {
    conn_id: u32,
    bytes: Vec<u8>,
}

fn js_err<'s>(scope: &mut PinScope<'s, '_>, msg: &str) -> v8::Local<'s, v8::Value> {
    let src = format!("new Error(\"{}\")", msg);
    let source = V8String::new(scope, &src).unwrap();
    Script::compile(scope, source, None)
        .and_then(|s| s.run(scope))
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn js_code_err<'s>(
    scope: &mut PinScope<'s, '_>,
    code: &str,
    msg: &str,
) -> v8::Local<'s, v8::Value> {
    let src = format!(
        "(function(){{var e=new Error(\"{}\");e.code=\"{}\";return e;}})()",
        msg, code
    );
    let source = V8String::new(scope, &src).unwrap();
    Script::compile(scope, source, None)
        .and_then(|s| s.run(scope))
        .unwrap_or_else(|| v8::undefined(scope).into())
}

/// Build a `TypeError` carrying a Node-style `.code` without compiling a
/// script (nested `Script::run` inside a native callback can fail silently
/// and yield `undefined`). Used for exceptions thrown from native fns.
fn js_type_error_with_code<'s>(
    scope: &mut PinScope<'s, '_>,
    code: &str,
    msg: &str,
) -> v8::Local<'s, v8::Value> {
    let msg_str = V8String::new(scope, msg).unwrap();
    let err = v8::Exception::type_error(scope, msg_str);
    if let Ok(obj) = v8::Local::<v8::Object>::try_from(err) {
        let key = V8String::new(scope, "code").unwrap();
        let val = V8String::new(scope, code).unwrap();
        obj.set(scope, key.into(), val.into());
    }
    err
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
                // Like Node: numbers are stringified and an array value
                // (e.g. several Set-Cookie) becomes one header line each.
                // They used to be sent as empty values.
                let text = |v: &serde_json::Value| match v {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                };
                obj.iter()
                    .flat_map(|(k, v)| match v {
                        serde_json::Value::Array(items) => items
                            .iter()
                            .map(|i| (k.clone(), text(i)))
                            .collect::<Vec<_>>(),
                        _ => vec![(k.clone(), text(v))],
                    })
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Header-name validity, matching the JS-side check in modules.rs (RFC 7230
/// token charset). The JS layer (`setHeader`/`writeHead`) already rejects
/// invalid names; this is the last line of defense before bytes hit the wire,
/// covering headers placed into `_headers` without going through those APIs.
fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            matches!(
                b,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            ) || b.is_ascii_alphanumeric()
        })
}

/// Header-value validity: no CR/LF or other control bytes except tab (HTTP
/// response splitting). Bytes >= 0x80 pass through (latin1/UTF-8 payloads).
fn is_valid_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b == b'\t' || !(0x00..=0x1F).contains(&b) && b != 0x7F)
}

#[derive(Debug)]
struct ParsedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    http10: bool,
}

/// Why request parsing gave up. `Respond` is a client-side protocol violation
/// that warrants a status line before closing (e.g. CL+TE smuggling, bad chunk
/// framing); `Silent` covers timeouts/EOF/IO where the existing behaviour —
/// dropping the connection without a response — is kept.
#[derive(Debug)]
enum ParseError {
    Respond(u16, &'static str),
    // Message kept for tests/diagnostics; production drops the connection
    // without logging, hence dead_code.
    #[allow(dead_code)]
    Silent(String),
}

impl From<String> for ParseError {
    fn from(msg: String) -> Self {
        ParseError::Silent(msg)
    }
}

/// Decode a `Transfer-Encoding: chunked` body: hex size [+ ;extensions] CRLF,
/// data CRLF, repeated until a 0-size chunk, then the trailer section (read
/// and discarded, bounded by `max_header_bytes`). Applies the same body
/// deadline and RUDY minimum-rate checks as the Content-Length path.
async fn read_chunked_body<R>(
    reader: &mut R,
    body_timeout: std::time::Duration,
    max_body: usize,
    min_body_rate_bps: u32,
    max_header_bytes: usize,
) -> Result<Vec<u8>, ParseError>
where
    R: AsyncBufRead + Unpin,
{
    let mut body: Vec<u8> = Vec::new();
    let deadline = Instant::now() + body_timeout;
    let body_start = Instant::now();

    loop {
        let mut size_line = String::new();
        tokio::time::timeout(body_timeout, reader.read_line(&mut size_line))
            .await
            .map_err(|_| ParseError::Silent("timeout: chunk size line stalled".into()))?
            .map_err(|e| ParseError::Silent(e.to_string()))?;
        if size_line.is_empty() {
            return Err(ParseError::Silent(
                "connection closed mid-chunked-body".into(),
            ));
        }
        let size_str = size_line.trim_end_matches(['\r', '\n']);
        // Chunk extensions (RFC 9112 §7.1.1) are ignored.
        let size_str = size_str.split(';').next().unwrap_or("").trim();
        let chunk_len = usize::from_str_radix(size_str, 16)
            .map_err(|_| ParseError::Respond(400, "Bad Request"))?;
        if chunk_len > max_body || body.len().saturating_add(chunk_len) > max_body {
            return Err(ParseError::Respond(413, "Payload Too Large"));
        }

        if chunk_len == 0 {
            // Trailer section: read until blank line, discard, bounded.
            let mut trailer_bytes = 0usize;
            loop {
                let mut trailer_line = String::new();
                tokio::time::timeout(body_timeout, reader.read_line(&mut trailer_line))
                    .await
                    .map_err(|_| ParseError::Silent("timeout: trailers stalled".into()))?
                    .map_err(|e| ParseError::Silent(e.to_string()))?;
                if trailer_line.is_empty() {
                    return Ok(body);
                }
                trailer_bytes += trailer_line.len();
                if trailer_bytes > max_header_bytes {
                    return Err(ParseError::Silent(format!(
                        "trailer flood: exceeded {max_header_bytes} bytes"
                    )));
                }
                if trailer_line.trim_end_matches(['\r', '\n']).is_empty() {
                    return Ok(body);
                }
            }
        }

        // Read exactly chunk_len bytes of chunk data.
        let mut remaining = chunk_len;
        let mut buf = vec![0u8; chunk_len.min(16 * 1024)];
        while remaining > 0 {
            if Instant::now() >= deadline {
                return Err(ParseError::Silent(
                    "timeout: body deadline exceeded (RUDY?)".into(),
                ));
            }
            let to_read = remaining.min(buf.len());
            let n = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                reader.read(&mut buf[..to_read]),
            )
            .await
            .map_err(|_| ParseError::Silent("timeout: body stalled (RUDY?)".into()))?
            .map_err(|e| ParseError::Silent(e.to_string()))?;
            if n == 0 {
                return Err(ParseError::Silent(
                    "connection closed before body complete".into(),
                ));
            }
            body.extend_from_slice(&buf[..n]);
            remaining -= n;

            if min_body_rate_bps > 0 {
                let elapsed = body_start.elapsed().as_secs_f64();
                if elapsed > 2.0 {
                    let rate = body.len() as f64 / elapsed;
                    if rate < min_body_rate_bps as f64 {
                        return Err(ParseError::Silent(format!(
                            "body rate too low ({rate:.0} B/s < {min_body_rate_bps} B/s min): RUDY?"
                        )));
                    }
                }
            }
        }

        // Chunk data must be followed by CRLF; consume it via one line read so
        // buffering stays consistent with the next size line.
        let mut crlf = String::new();
        tokio::time::timeout(body_timeout, reader.read_line(&mut crlf))
            .await
            .map_err(|_| ParseError::Silent("timeout: chunk terminator stalled".into()))?
            .map_err(|e| ParseError::Silent(e.to_string()))?;
        if !crlf.trim_end_matches(['\r', '\n']).is_empty() {
            return Err(ParseError::Respond(400, "Bad Request"));
        }
    }
}

async fn parse_request<R>(
    reader: &mut R,
    header_timeout: std::time::Duration,
    body_timeout: std::time::Duration,
    max_header_count: usize,
    max_header_bytes: usize,
    max_body_bytes: usize,
    min_body_rate_bps: u32,
) -> Result<ParsedRequest, ParseError>
where
    R: AsyncBufRead + Unpin,
{
    let mut request_line = String::new();
    tokio::time::timeout(header_timeout, reader.read_line(&mut request_line))
        .await
        .map_err(|_| {
            ParseError::Silent("timeout: request line not received in time (Slowloris?)".into())
        })?
        .map_err(|e| ParseError::Silent(e.to_string()))?;

    let request_line = request_line.trim_end_matches(['\r', '\n']);
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let version = parts.next().unwrap_or("HTTP/1.1").to_string();
    let http10 = version == "HTTP/1.0";

    let max_body = if max_body_bytes == 0 {
        HARD_MAX_BODY
    } else {
        max_body_bytes
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length: usize = 0;
    let mut has_content_length = false;
    let mut transfer_encoding: Option<String> = None;
    let mut total_header_bytes: usize = 0;

    loop {
        let mut line = String::new();
        tokio::time::timeout(header_timeout, reader.read_line(&mut line))
            .await
            .map_err(|_| {
                ParseError::Silent("timeout: headers not received in time (Slowloris?)".into())
            })?
            .map_err(|e| ParseError::Silent(e.to_string()))?;

        total_header_bytes += line.len();
        if total_header_bytes > max_header_bytes {
            return Err(ParseError::Silent(format!(
                "header flood: exceeded {} bytes",
                max_header_bytes
            )));
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if headers.len() >= max_header_count {
            return Err(ParseError::Silent(format!(
                "header flood: more than {} headers",
                max_header_count
            )));
        }

        if let Some(colon) = trimmed.find(':') {
            let name = trimmed[..colon].trim().to_lowercase();
            let value = trimmed[colon + 1..].trim().to_string();
            match name.as_str() {
                "content-length" => {
                    content_length = value.parse::<usize>().unwrap_or(0).min(max_body);
                    has_content_length = true;
                    headers.push((name, content_length.to_string()));
                }
                "transfer-encoding" => {
                    transfer_encoding = Some(value.to_lowercase());
                    headers.push((name, value));
                }
                _ => headers.push((name, value)),
            }
        }
    }

    // Request smuggling vector: both framing headers present. RFC 9112 says
    // reject; matching llhttp's HPE_UNEXPECTED_CONTENT_LENGTH behaviour.
    if has_content_length && transfer_encoding.is_some() {
        return Err(ParseError::Respond(400, "Bad Request"));
    }

    let body = if let Some(te) = transfer_encoding {
        // Only "chunked" (as the final coding) can be decoded here; anything
        // else gets 501 Not Implemented per RFC 9112 §6.1.
        let final_coding = te.rsplit(',').next().unwrap_or("").trim();
        if final_coding != "chunked" {
            return Err(ParseError::Respond(501, "Not Implemented"));
        }
        read_chunked_body(
            reader,
            body_timeout,
            max_body,
            min_body_rate_bps,
            max_header_bytes,
        )
        .await?
    } else if has_content_length && content_length > 0 {
        let mut body = vec![0u8; content_length];
        let deadline = Instant::now() + body_timeout;
        let mut received: usize = 0;
        let body_start = Instant::now();

        while received < content_length {
            if Instant::now() >= deadline {
                return Err(ParseError::Silent(
                    "timeout: body deadline exceeded (RUDY?)".into(),
                ));
            }
            let remaining_deadline = deadline.saturating_duration_since(Instant::now());
            let chunk_timeout = remaining_deadline.min(std::time::Duration::from_secs(1));
            let n = tokio::time::timeout(chunk_timeout, reader.read(&mut body[received..]))
                .await
                .map_err(|_| ParseError::Silent("timeout: body stalled (RUDY?)".into()))?
                .map_err(|e| ParseError::Silent(e.to_string()))?;
            if n == 0 {
                return Err(ParseError::Silent(
                    "connection closed before body complete".into(),
                ));
            }
            received += n;

            if min_body_rate_bps > 0 {
                let elapsed = body_start.elapsed().as_secs_f64();
                if elapsed > 2.0 {
                    let rate = received as f64 / elapsed;
                    if rate < min_body_rate_bps as f64 {
                        return Err(ParseError::Silent(format!(
                            "body rate too low ({:.0} B/s < {} B/s min): RUDY?",
                            rate, min_body_rate_bps
                        )));
                    }
                }
            }
        }
        body
    } else {
        Vec::new()
    };

    Ok(ParsedRequest {
        method,
        path,
        headers,
        body,
        http10,
    })
}

fn reject_stream(stream: tokio::net::TcpStream, status: u16, msg: &'static str) {
    let response = format!(
        "HTTP/1.1 {status} {msg}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{msg}",
        len = msg.len()
    );
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut s = stream;
        let _ = s.write_all(response.as_bytes()).await;
        // A blocked client may already have sent request bytes the accept loop
        // never read. Dropping the socket with that data still unread makes
        // Linux send an RST instead of a clean FIN, so the client's read loop
        // gets "connection reset by peer" and discards the 403/503 it was just
        // sent. Drain briefly so the close is a graceful FIN.
        let mut buf = [0u8; 512];
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), async {
            loop {
                match s.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        })
        .await;
    });
}

/// Builds the TLS acceptor for an `https.createServer({ key, cert })`
/// listener from PEM text: the runtime's crypto provider (the FIPS module in
/// a `fips` build), rustls' safe default protocol versions (TLS 1.2 and
/// 1.3), no client authentication, HTTP/1.1 over ALPN.
fn tls_acceptor(cert_pem: &str, key_pem: &str) -> Result<tokio_rustls::TlsAcceptor, String> {
    let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("invalid certificate PEM: {e}"))?;
    if certs.is_empty() {
        return Err("no certificate found in `cert`".to_string());
    }
    let key = rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem.as_bytes()))
        .map_err(|e| format!("invalid private key PEM: {e}"))?
        .ok_or_else(|| "no private key found in `key`".to_string())?;
    let mut config = rustls::ServerConfig::builder_with_provider(super::tls::provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS protocol versions: {e}"))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("certificate/key rejected: {e}"))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

/// Idle timeout while waiting for JS to enqueue a response via
/// `__httpRespond`. Guards against a request handler that never responds:
/// without it a stalled handler would pin the connection open forever,
/// defeating the firewall's connection-exhaustion protection.
const RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Drives a single accepted connection: parses requests in a loop (HTTP/1.1
/// keep-alive), drops each parsed request into `ready` for JS, waits for
/// `__httpRespond` to push a response through the per-connection channel, and
/// writes it. Exits when the client closes the connection, a parse timeout
/// fires (Slowloris/RUDY), a response times out, or the stream errors — the
/// `conns` entry and firewall connection accounting are cleaned up on exit.
#[allow(clippy::too_many_arguments)]
async fn handle_connection<S>(
    stream: S,
    ip: IpAddr,
    server_id: u32,
    hdr_timeout: std::time::Duration,
    body_timeout: std::time::Duration,
    keepalive_timeout: std::time::Duration,
    max_hdr_count: usize,
    max_hdr_bytes: usize,
    max_body: usize,
    min_body_rate: u32,
    max_requests_per_conn: u32,
    conns: Arc<Mutex<HashMap<u32, ConnEntry>>>,
    conn_nid: Arc<Mutex<u32>>,
    fw: Arc<Option<Arc<Firewall>>>,
    ready: ReadyQueue,
    upgrades: UpgradeRegistry,
    upgrades_q: UpgradeQueue,
    wake: Arc<tokio::sync::Notify>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut read_half, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(&mut read_half);

    let conn_id = {
        let mut n = conn_nid.lock().unwrap();
        let cid = *n;
        *n = n.wrapping_add(1);
        cid
    };

    let (resp_tx, mut resp_rx) = mpsc::channel::<PendingResponse>(16);
    conns.lock().unwrap().insert(conn_id, ConnEntry { resp_tx });

    // Requests already served on this TCP connection. The first request gets
    // the full `hdr_timeout`; every reused-connection request afterwards is
    // bounded by the shorter `keepalive_timeout` so an idle keep-alive socket
    // can't be parked open (Slowloris on keep-alive). `max_requests_per_conn`
    // caps how many we serve before closing, regardless of the client's
    // `Connection` header.
    let mut requests_served: u32 = 0;

    loop {
        let hdr_deadline = if requests_served == 0 {
            hdr_timeout
        } else {
            keepalive_timeout
        };
        let parsed = match parse_request(
            &mut reader,
            hdr_deadline,
            body_timeout,
            max_hdr_count,
            max_hdr_bytes,
            max_body,
            min_body_rate,
        )
        .await
        {
            Ok(p) => p,
            Err(ParseError::Respond(status, msg)) => {
                let resp = format!(
                    "HTTP/1.1 {status} {msg}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{msg}",
                    len = msg.len()
                );
                let _ = writer.write_all(resp.as_bytes()).await;
                let _ = writer.flush().await;
                break;
            }
            Err(ParseError::Silent(_)) => break,
        };

        // Resolve the effective client IP: the peer itself, or — when the
        // peer is a trusted proxy that sent X-Forwarded-For — the forwarded
        // address. Used for both rate-limit accounting and remoteAddress.
        let req_ip = fw
            .as_ref()
            .as_ref()
            .map(|f| {
                let xff = parsed
                    .headers
                    .iter()
                    .find(|(k, _)| k == "x-forwarded-for")
                    .map(|(_, v)| v.as_str());
                f.client_ip_from_xff(ip, xff)
            })
            .unwrap_or(ip);

        if let Some(firewall) = fw.as_ref().as_ref() {
            match firewall.check_request(req_ip) {
                FirewallDecision::Allow => {}
                decision => {
                    let resp = format!(
                        "HTTP/1.1 {s} {m}\r\nContent-Length: {l}\r\nConnection: close\r\n\r\n{m}",
                        s = decision.http_status(),
                        m = decision.message(),
                        l = decision.message().len(),
                    );
                    let _ = writer.write_all(resp.as_bytes()).await;
                    break;
                }
            }
        }

        // HTTP `Upgrade` (WebSocket): hand the raw duplex socket to JS instead
        // of answering the request — `ws` (npm) performs the 101 handshake and
        // drives the frames itself. The bytes already read past the headers go
        // out as `head` so the client's first WS frame arrives intact.
        if is_upgrade_request(&parsed) {
            let head = reader.buffer().to_vec();
            drop(reader);
            let conn = Arc::new(UpgradeConn::new());
            upgrades.lock().unwrap().insert(conn_id, conn.clone());
            spawn_upgrade_driver(
                read_half.unsplit(writer),
                ip,
                conn_id,
                conn,
                upgrades,
                fw.clone(),
            );
            upgrades_q
                .lock()
                .unwrap()
                .entry(server_id)
                .or_default()
                .push_back(UpgradeRequest {
                    parsed,
                    conn_id,
                    remote: req_ip,
                    head,
                });
            wake.notify_one();
            conns.lock().unwrap().remove(&conn_id);
            // return, not break: the firewall slot now belongs to the driver,
            // which releases it when the upgrade socket closes.
            return;
        }

        requests_served += 1;

        let conn_hdr = parsed
            .headers
            .iter()
            .find(|(k, _)| k == "connection")
            .map(|(_, v)| v.to_lowercase());
        // HTTP/1.1 defaults to keep-alive unless the client says close; HTTP/1.0
        // defaults to close unless the client explicitly asks to keep the
        // connection alive. The per-connection request cap forces a close
        // regardless of what the client asked for.
        let close_after = match conn_hdr.as_deref() {
            Some(v) => v == "close",
            None => parsed.http10,
        } || requests_served >= max_requests_per_conn;

        ready
            .lock()
            .unwrap()
            .entry(server_id)
            .or_default()
            .push_back(ReadyRequest {
                parsed,
                conn_id,
                remote: req_ip,
            });
        wake.notify_one();

        let resp = match tokio::time::timeout(RESPONSE_TIMEOUT, resp_rx.recv()).await {
            Ok(Some(r)) if r.conn_id == conn_id => r,
            _ => break,
        };

        if writer.write_all(&resp.bytes).await.is_err() {
            break;
        }
        if writer.flush().await.is_err() {
            break;
        }
        if close_after {
            break;
        }
    }

    conns.lock().unwrap().remove(&conn_id);
    if let Some(firewall) = fw.as_ref().as_ref() {
        firewall.on_disconnect(ip);
    }
}

pub fn inject_http_server(
    scope: &mut PinScope,
    permissions: Arc<PermissionState>,
    firewall: Option<Arc<Firewall>>,
    native_ctx: &mut NativeCtxRegistry,
) -> anyhow::Result<()> {
    let servers: Arc<Mutex<HashMap<u32, Arc<TcpListener>>>> = Arc::new(Mutex::new(HashMap::new()));
    let conns: Arc<Mutex<HashMap<u32, ConnEntry>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_server_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let next_conn_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
    let fw: Arc<Option<Arc<Firewall>>> = Arc::new(firewall);
    let ready: ReadyQueue = Arc::new(Mutex::new(HashMap::new()));
    let upgrades: UpgradeRegistry = Arc::new(Mutex::new(HashMap::new()));
    let upgrades_q: UpgradeQueue = Arc::new(Mutex::new(HashMap::new()));
    let wake = Arc::new(tokio::sync::Notify::new());
    scope.set_slot(HttpDispatch {
        ready: ready.clone(),
        upgrades: upgrades_q.clone(),
        wake: wake.clone(),
        handlers: HashMap::new(),
    });
    let context = scope.get_current_context();
    let global = context.global(scope);

    {
        let ctx_ptr = native_ctx.leak(HttpListenCtx {
            perms: permissions.clone(),
            servers: servers.clone(),
            conns: conns.clone(),
            conn_nid: next_conn_id.clone(),
            ready: ready.clone(),
            upgrades: upgrades.clone(),
            upgrades_q: upgrades_q.clone(),
            wake: wake.clone(),
            nid: next_server_id.clone(),
            fw: fw.clone(),
        });
        let external = v8::External::new(scope, ctx_ptr);
        let http_listen_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpListenCtx)
                };
                let port_arg = args.get(0);
                let port: u16 = port_arg.uint32_value(scope).unwrap_or(0) as u16;
                let host_arg = args.get(1);
                let host = host_arg.to_rust_string_lossy(scope);
                // Optional (cert PEM, key PEM): an https server. Both or
                // neither — the JS side validates the options.
                let tls = if args.get(2).is_string() && args.get(3).is_string() {
                    let cert = args.get(2).to_rust_string_lossy(scope);
                    let key = args.get(3).to_rust_string_lossy(scope);
                    match tls_acceptor(&cert, &key) {
                        Ok(a) => Some(a),
                        Err(e) => {
                            let err = js_code_err(scope, "ERR_TLS_INVALID_CONTEXT", &e);
                            rv.set(err);
                            return;
                        }
                    }
                } else {
                    None
                };

                let Some(host) = ctx.perms.bind_host(&host) else {
                    let err = js_code_err(
                        scope,
                        "EACCES",
                        &format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                };

                match bind_listener(&format!("{}:{}", host, port)) {
                    Ok(std_listener) => {
                        if let Err(e) = std_listener.set_nonblocking(true) {
                            let err = js_err(scope, &e.to_string());
                            rv.set(err);
                            return;
                        }
                        match TcpListener::from_std(std_listener) {
                            Ok(tokio_listener) => {
                                let id = {
                                    let mut n = ctx.nid.lock().unwrap();
                                    let id = *n;
                                    *n = n.wrapping_add(1);
                                    id
                                };
                                let listener = Arc::new(tokio_listener);
                                ctx.servers.lock().unwrap().insert(id, listener.clone());
                                HTTP_ACTIVE_LISTENERS
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                                if id == 0
                                    && let Some(firewall) = ctx.fw.as_ref().as_ref()
                                {
                                    vvva_firewall::spawn_cleanup_task(
                                        firewall.clone(),
                                        std::time::Duration::from_secs(60),
                                    );
                                }

                                // Background accept loop: runs for the listener's
                                // lifetime, parsing each request (with real
                                // timeouts/firewall checks — hence async, not a
                                // blocking std accept) and dropping the finished
                                // JSON into `ready[id]` for the non-blocking
                                // `__httpAcceptPoll` to pick up. This avoids
                                // calling `Handle::block_on` from inside a V8
                                // callback that is itself already running inside
                                // a tokio task, which would panic.
                                let conns = ctx.conns.clone();
                                let conn_nid = ctx.conn_nid.clone();
                                let fw = ctx.fw.clone();
                                let ready = ctx.ready.clone();
                                let upgrades = ctx.upgrades.clone();
                                let upgrades_q = ctx.upgrades_q.clone();
                                let wake = ctx.wake.clone();
                                tokio::spawn(async move {
                                    loop {
                                        let (stream, peer_addr) = match listener.accept().await {
                                            Ok(v) => v,
                                            Err(_) => break,
                                        };

                                        let ip: IpAddr = match peer_addr {
                                            SocketAddr::V4(a) => IpAddr::V4(*a.ip()),
                                            SocketAddr::V6(a) => IpAddr::V6(*a.ip()),
                                        };

                                        if let Some(firewall) = fw.as_ref().as_ref() {
                                            match firewall.check_connection(ip) {
                                                FirewallDecision::Allow => {
                                                    firewall.on_connect(ip);
                                                }
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

                                        let (
                                            hdr_timeout,
                                            body_timeout,
                                            keepalive_timeout,
                                            max_hdr_count,
                                            max_hdr_bytes,
                                            max_body,
                                            min_body_rate,
                                            max_requests_per_conn,
                                        ) = if let Some(firewall) = fw.as_ref().as_ref() {
                                            let c = &firewall.config;
                                            (
                                                std::time::Duration::from_millis(
                                                    c.header_timeout_ms,
                                                ),
                                                std::time::Duration::from_millis(c.body_timeout_ms),
                                                std::time::Duration::from_millis(
                                                    c.keepalive_timeout_ms,
                                                ),
                                                c.max_header_count as usize,
                                                c.max_header_bytes as usize,
                                                c.max_body_bytes as usize,
                                                c.min_body_rate_bps,
                                                c.max_requests_per_conn,
                                            )
                                        } else {
                                            (
                                                std::time::Duration::from_secs(10),
                                                std::time::Duration::from_secs(30),
                                                std::time::Duration::from_secs(5),
                                                100,
                                                16_384,
                                                0,
                                                100,
                                                1_000,
                                            )
                                        };

                                        let conns2 = conns.clone();
                                        let conn_nid2 = conn_nid.clone();
                                        let fw2 = fw.clone();
                                        let ready2 = ready.clone();
                                        let wake2 = wake.clone();
                                        let tls2 = tls.clone();
                                        let upgrades2 = upgrades.clone();
                                        let upgrades_q2 = upgrades_q.clone();
                                        tokio::spawn(async move {
                                            // TLS: handshake first, bounded by
                                            // the header timeout so a client
                                            // that never finishes it can't park
                                            // the connection (and its firewall
                                            // slot) open.
                                            if let Some(acceptor) = tls2 {
                                                let handshake = tokio::time::timeout(
                                                    hdr_timeout,
                                                    acceptor.accept(stream),
                                                )
                                                .await;
                                                let Ok(Ok(tls_stream)) = handshake else {
                                                    if let Some(firewall) = fw2.as_ref().as_ref() {
                                                        firewall.on_disconnect(ip);
                                                    }
                                                    return;
                                                };
                                                handle_connection(
                                                    tls_stream,
                                                    ip,
                                                    id,
                                                    hdr_timeout,
                                                    body_timeout,
                                                    keepalive_timeout,
                                                    max_hdr_count,
                                                    max_hdr_bytes,
                                                    max_body,
                                                    min_body_rate,
                                                    max_requests_per_conn,
                                                    conns2,
                                                    conn_nid2,
                                                    fw2,
                                                    ready2,
                                                    upgrades2,
                                                    upgrades_q2,
                                                    wake2,
                                                )
                                                .await;
                                                return;
                                            }
                                            handle_connection(
                                                stream,
                                                ip,
                                                id,
                                                hdr_timeout,
                                                body_timeout,
                                                keepalive_timeout,
                                                max_hdr_count,
                                                max_hdr_bytes,
                                                max_body,
                                                min_body_rate,
                                                max_requests_per_conn,
                                                conns2,
                                                conn_nid2,
                                                fw2,
                                                ready2,
                                                upgrades2,
                                                upgrades_q2,
                                                wake2,
                                            )
                                            .await;
                                        });
                                    }
                                });

                                rv.set(v8::Integer::new_from_unsigned(scope, id).into());
                            }
                            Err(e) => {
                                let err = js_err(scope, &format!("TcpListener::from_std: {}", e));
                                rv.set(err);
                            }
                        }
                    }
                    Err(e) => {
                        let err = js_code_err(scope, "EADDRINUSE", &e.to_string());
                        rv.set(err);
                    }
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpListen").unwrap().into(),
            http_listen_fn.into(),
        );
    }

    {
        let ctx_ptr = native_ctx.leak(HttpAcceptCtx {
            ready: ready.clone(),
        });
        let external = v8::External::new(scope, ctx_ptr);
        // Non-blocking: pops one ready request's JSON for `server_id`, or
        // returns null if none has finished parsing yet. The actual accept +
        // parse work happens in the background task spawned by __httpListen;
        // this just drains its output queue. JS polls this on an interval,
        // the same pattern dgram.rs uses for __udpRecv.
        let http_accept_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpAcceptCtx)
                };
                let server_id_arg = args.get(0);
                let server_id = server_id_arg.uint32_value(scope).unwrap_or(0);

                let popped = ctx
                    .ready
                    .lock()
                    .unwrap()
                    .get_mut(&server_id)
                    .and_then(|q| q.pop_front());

                match popped {
                    // [method, url, connId, remoteAddress, body, [k0, v0, k1, v1, ...]]
                    Some(r) => {
                        let p = &r.parsed;
                        let flat: Vec<v8::Local<v8::Value>> = p
                            .headers
                            .iter()
                            .flat_map(|(k, v)| [k, v])
                            .map(|x| V8String::new(scope, x).unwrap().into())
                            .collect();
                        let headers = v8::Array::new_with_elements(scope, &flat);
                        let body = String::from_utf8_lossy(&p.body);
                        let items: [v8::Local<v8::Value>; 6] = [
                            V8String::new(scope, &p.method).unwrap().into(),
                            V8String::new(scope, &p.path).unwrap().into(),
                            v8::Integer::new_from_unsigned(scope, r.conn_id).into(),
                            V8String::new(scope, &r.remote.to_string()).unwrap().into(),
                            V8String::new(scope, &body).unwrap().into(),
                            headers.into(),
                        ];
                        rv.set(v8::Array::new_with_elements(scope, &items).into());
                    }
                    None => rv.set(v8::null(scope).into()),
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpAcceptPoll").unwrap().into(),
            http_accept_fn.into(),
        );
    }

    // ── Upgrade (WebSocket) bridge ───────────────────────────────────────────
    // The upgrade socket lives in a shared registry between the V8 thread (the
    // natives below) and the driver task on a tokio thread. Read/write are
    // queue-based, exactly like the `net` module's pollable sockets.
    {
        let ctx_ptr = native_ctx.leak(HttpUpgradeCtx {
            upgrades: upgrades.clone(),
            upgrades_q: upgrades_q.clone(),
        });
        let external = v8::External::new(scope, ctx_ptr);

        let http_upgrade_poll_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpUpgradeCtx)
                };
                let server_id = args.get(0).uint32_value(scope).unwrap_or(0);
                let popped = ctx
                    .upgrades_q
                    .lock()
                    .unwrap()
                    .get_mut(&server_id)
                    .and_then(|q| q.pop_front());

                match popped {
                    // [connId, remoteAddress, head(Uint8Array), method, url, [k0,v0,...]]
                    Some(r) => {
                        let p = &r.parsed;
                        let flat: Vec<v8::Local<v8::Value>> = p
                            .headers
                            .iter()
                            .flat_map(|(k, v)| [k, v])
                            .map(|x| V8String::new(scope, x).unwrap().into())
                            .collect();
                        let headers = v8::Array::new_with_elements(scope, &flat);
                        let head =
                            crate::builtins::v8_compat::uint8array_from_bytes(scope, &r.head);
                        let items: [v8::Local<v8::Value>; 6] = [
                            v8::Integer::new_from_unsigned(scope, r.conn_id).into(),
                            V8String::new(scope, &r.remote.to_string()).unwrap().into(),
                            head.into(),
                            V8String::new(scope, &p.method).unwrap().into(),
                            V8String::new(scope, &p.path).unwrap().into(),
                            headers.into(),
                        ];
                        rv.set(v8::Array::new_with_elements(scope, &items).into());
                    }
                    None => rv.set(v8::null(scope).into()),
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpUpgradePoll").unwrap().into(),
            http_upgrade_poll_fn.into(),
        );

        let upgrade_read_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpUpgradeCtx)
                };
                let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);
                let max_bytes: usize = args
                    .get(1)
                    .uint32_value(scope)
                    .unwrap_or(16_384)
                    .min(65_536) as usize;

                let conn = ctx.upgrades.lock().unwrap().get(&conn_id).cloned();
                match conn {
                    None => {
                        let err = js_code_err(scope, "EOF", "upgrade socket closed");
                        rv.set(err);
                    }
                    Some(c) => match take_upgrade_read(&c, max_bytes) {
                        UpgradeTake::Data(bytes) => {
                            let result =
                                crate::builtins::v8_compat::uint8array_from_bytes(scope, &bytes);
                            rv.set(result.into());
                        }
                        UpgradeTake::Eof => {
                            let err = js_code_err(scope, "EOF", "connection closed");
                            rv.set(err);
                        }
                        UpgradeTake::Empty => {
                            let err = js_code_err(scope, "EAGAIN", "no data available");
                            rv.set(err);
                        }
                    },
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__upgradeRead").unwrap().into(),
            upgrade_read_fn.into(),
        );

        let upgrade_write_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpUpgradeCtx)
                };
                let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);
                let bytes = crate::builtins::v8_compat::js_value_to_bytes(scope, args.get(1));
                let conn = ctx.upgrades.lock().unwrap().get(&conn_id).cloned();
                match conn {
                    Some(c) if !c.closed.load(std::sync::atomic::Ordering::Relaxed) => {
                        if queue_upgrade_write(&c, bytes) {
                            rv.set(v8::undefined(scope).into());
                        } else {
                            let err = js_code_err(
                                scope,
                                "EPIPE",
                                "upgrade socket write buffer is full (the peer is not reading)",
                            );
                            rv.set(err);
                        }
                    }
                    _ => {
                        let err = js_code_err(scope, "EPIPE", "upgrade socket closed");
                        rv.set(err);
                    }
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__upgradeWrite").unwrap().into(),
            upgrade_write_fn.into(),
        );

        let upgrade_close_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpUpgradeCtx)
                };
                let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);
                if let Some(c) = ctx.upgrades.lock().unwrap().remove(&conn_id) {
                    c.closed.store(true, std::sync::atomic::Ordering::Relaxed);
                    c.write_wake.notify_one();
                }
                rv.set(v8::undefined(scope).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__upgradeClose").unwrap().into(),
            upgrade_close_fn.into(),
        );
    }

    {
        let ctx_ptr = native_ctx.leak(HttpRespondCtx {
            conns: conns.clone(),
        });
        let external = v8::External::new(scope, ctx_ptr);
        let http_respond_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpRespondCtx)
                };
                let conn_id_arg = args.get(0);
                let conn_id = conn_id_arg.uint32_value(scope).unwrap_or(0);
                let status_arg = args.get(1);
                let status: u32 = status_arg.uint32_value(scope).unwrap_or(200);
                let status_text_arg = args.get(2);
                let status_text = status_text_arg.to_rust_string_lossy(scope);
                let headers_json_arg = args.get(3);
                let headers_json = headers_json_arg.to_rust_string_lossy(scope);
                let body_arg = args.get(4);
                let body = body_arg.to_rust_string_lossy(scope);

                let body_bytes = body.as_bytes();
                let extra = parse_extra_headers(&headers_json);
                let mut resp = format!("HTTP/1.1 {} {}\r\n", status, status_text);
                resp.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
                for (k, v) in &extra {
                    let kl = k.to_lowercase();
                    if kl == "content-length" || kl == "connection" {
                        continue;
                    }
                    if !is_valid_header_name(k) || !is_valid_header_value(v) {
                        let err = js_type_error_with_code(
                            scope,
                            "ERR_INVALID_CHAR",
                            &format!("Invalid character in header content [\"{kl}\"]"),
                        );
                        scope.throw_exception(err);
                        return;
                    }
                    resp.push_str(&format!("{}: {}\r\n", k, v));
                }
                resp.push_str("\r\n");

                let mut bytes = resp.into_bytes();
                bytes.extend_from_slice(body_bytes);

                let guard = ctx.conns.lock().unwrap();
                match guard.get(&conn_id) {
                    Some(conn) => {
                        let ok = conn.resp_tx.try_send(PendingResponse { conn_id, bytes });
                        drop(guard);
                        if ok.is_err() {
                            let err = js_code_err(scope, "ENOENT", "connection closed");
                            rv.set(err);
                            return;
                        }
                    }
                    None => {
                        let err = js_code_err(scope, "ENOENT", "unknown conn_id");
                        rv.set(err);
                        return;
                    }
                }
                rv.set(v8::undefined(scope).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpRespond").unwrap().into(),
            http_respond_fn.into(),
        );
    }

    {
        let ctx_ptr = native_ctx.leak(HttpRespondCtx {
            conns: conns.clone(),
        });
        let external = v8::External::new(scope, ctx_ptr);
        let http_respond_bytes_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpRespondCtx)
                };
                let conn_id_arg = args.get(0);
                let conn_id = conn_id_arg.uint32_value(scope).unwrap_or(0);
                let status_arg = args.get(1);
                let status: u32 = status_arg.uint32_value(scope).unwrap_or(200);
                let status_text_arg = args.get(2);
                let status_text = status_text_arg.to_rust_string_lossy(scope);
                let headers_json_arg = args.get(3);
                let headers_json = headers_json_arg.to_rust_string_lossy(scope);
                let body_arg = args.get(4);
                let body: Vec<u8> = if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(body_arg)
                {
                    uint8array_to_vec(scope, arr)
                } else {
                    vec![]
                };

                let extra = parse_extra_headers(&headers_json);
                let mut resp = format!("HTTP/1.1 {} {}\r\n", status, status_text);
                resp.push_str(&format!("Content-Length: {}\r\n", body.len()));
                for (k, v) in &extra {
                    let kl = k.to_lowercase();
                    if kl == "content-length" || kl == "connection" || kl == "transfer-encoding" {
                        continue;
                    }
                    if !is_valid_header_name(k) || !is_valid_header_value(v) {
                        let err = js_type_error_with_code(
                            scope,
                            "ERR_INVALID_CHAR",
                            &format!("Invalid character in header content [\"{kl}\"]"),
                        );
                        scope.throw_exception(err);
                        return;
                    }
                    resp.push_str(&format!("{}: {}\r\n", k, v));
                }
                resp.push_str("\r\n");

                let mut bytes = resp.into_bytes();
                bytes.extend_from_slice(&body);

                let guard = ctx.conns.lock().unwrap();
                match guard.get(&conn_id) {
                    Some(conn) => {
                        let ok = conn.resp_tx.try_send(PendingResponse { conn_id, bytes });
                        drop(guard);
                        if ok.is_err() {
                            let err = js_code_err(scope, "ENOENT", "connection closed");
                            rv.set(err);
                            return;
                        }
                    }
                    None => {
                        let err = js_code_err(scope, "ENOENT", "unknown conn_id");
                        rv.set(err);
                        return;
                    }
                }
                rv.set(v8::undefined(scope).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpRespondBytes").unwrap().into(),
            http_respond_bytes_fn.into(),
        );
    }

    {
        // Registers the function `dispatch_ready` calls when requests for
        // `server_id` are waiting. Scopes are captured here, at listen time.
        let set_handler_fn = v8::Function::new(
            scope,
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let server_id = args.get(0).uint32_value(scope).unwrap_or(0);
                let Ok(f) = v8::Local::<v8::Function>::try_from(args.get(1)) else {
                    let err = js_type_error_with_code(
                        scope,
                        "ERR_INVALID_ARG_TYPE",
                        "handler must be a function",
                    );
                    scope.throw_exception(err);
                    return;
                };
                let handler = v8::Global::new(scope, f);
                let scopes = vvva_permissions::deny_scopes();
                if let Some(d) = scope.get_slot_mut::<HttpDispatch>() {
                    d.handlers.insert(server_id, (handler, scopes));
                }
                rv.set(v8::undefined(scope).into());
            },
        )
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpSetHandler").unwrap().into(),
            set_handler_fn.into(),
        );
    }

    {
        let servers_ptr = native_ctx.leak(servers.clone());
        let external = v8::External::new(scope, servers_ptr);
        let http_close_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const Arc<Mutex<HashMap<u32, Arc<TcpListener>>>>)
                };
                let server_id_arg = args.get(0);
                let server_id = server_id_arg.uint32_value(scope).unwrap_or(0);
                if servers.lock().unwrap().remove(&server_id).is_some() {
                    HTTP_ACTIVE_LISTENERS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
                if let Some(d) = scope.get_slot_mut::<HttpDispatch>() {
                    d.handlers.remove(&server_id);
                }
                rv.set(v8::undefined(scope).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpClose").unwrap().into(),
            http_close_fn.into(),
        );
    }

    {
        let servers_ptr = native_ctx.leak(servers.clone());
        let external = v8::External::new(scope, servers_ptr);
        let http_server_port_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const Arc<Mutex<HashMap<u32, Arc<TcpListener>>>>)
                };
                let server_id = args.get(0).uint32_value(scope).unwrap_or(0);
                let port = servers
                    .lock()
                    .unwrap()
                    .get(&server_id)
                    .and_then(|l| l.local_addr().ok())
                    .map(|a| a.port() as u32)
                    .unwrap_or(0);
                rv.set(v8::Integer::new_from_unsigned(scope, port).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpServerPort").unwrap().into(),
            http_server_port_fn.into(),
        );
    }

    {
        // The address actually bound, which can differ from the one asked
        // for: a wildcard listen without an all-interfaces grant is kept on
        // loopback (see PermissionState::bind_host).
        let servers_ptr = native_ctx.leak(servers.clone());
        let external = v8::External::new(scope, servers_ptr);
        let http_server_address_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const Arc<Mutex<HashMap<u32, Arc<TcpListener>>>>)
                };
                let server_id = args.get(0).uint32_value(scope).unwrap_or(0);
                let ip = servers
                    .lock()
                    .unwrap()
                    .get(&server_id)
                    .and_then(|l| l.local_addr().ok())
                    .map(|a| a.ip().to_string())
                    .unwrap_or_default();
                rv.set(V8String::new(scope, &ip).unwrap().into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpServerAddress").unwrap().into(),
            http_server_address_fn.into(),
        );
    }

    Ok(())
}

// ── HTTP/2 server bindings ─────────────────────────────────────────────────

use std::collections::VecDeque;

struct H2ServerState {
    listener: Option<Arc<TcpListener>>,
    stream_queue: VecDeque<String>,
    streams: HashMap<u32, H2StreamEntry>,
    next_stream_id: u32,
}

struct H2StreamEntry {
    send_response: Option<h2::server::SendResponse<bytes::Bytes>>,
    send_stream: Option<h2::SendStream<bytes::Bytes>>,
}

type H2Servers = Arc<Mutex<HashMap<u32, Arc<Mutex<H2ServerState>>>>>;

struct H2ClientState {
    send_request: Option<h2::client::SendRequest<bytes::Bytes>>,
    streams: HashMap<u32, H2ClientStreamEntry>,
    next_stream_id: u32,
    response_queue: VecDeque<String>,
    /// The thread running the h2 connection driver (its own current-thread
    /// runtime). `h2` only transmits what this driver polls; spawning it on
    /// a runtime that is torn down when `__h2Connect` returns orphaned the
    /// connection, so every `session.request()` failed with "failed to
    /// create stream". Keeping the driver on its own thread lets the client
    /// actually send and receive until the connection closes, at which point
    /// the thread exits by itself.
    conn_thread: Option<std::thread::JoinHandle<()>>,
}

struct H2ClientStreamEntry {
    send_stream: Option<h2::SendStream<bytes::Bytes>>,
}

type H2Clients = Arc<Mutex<HashMap<u32, Arc<Mutex<H2ClientState>>>>>;

pub fn inject_http2_server(
    scope: &mut PinScope,
    permissions: Arc<PermissionState>,
    native_ctx: &mut NativeCtxRegistry,
) -> anyhow::Result<()> {
    let h2_servers: H2Servers = Arc::new(Mutex::new(HashMap::new()));
    let h2_clients: H2Clients = Arc::new(Mutex::new(HashMap::new()));
    let h2_next_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let h2_next_client_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let context = scope.get_current_context();
    let global = context.global(scope);

    // __h2Listen(port, host) → serverId
    {
        let servers = h2_servers.clone();
        let nid = h2_next_id.clone();
        let ctx_ptr = native_ctx.leak((permissions.clone(), servers, nid));
        let external = v8::External::new(scope, ctx_ptr);
        let listen_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let (perms, servers, nid) = unsafe {
                    &*(args.data().cast::<v8::External>().value()
                        as *const (Arc<PermissionState>, H2Servers, Arc<Mutex<u32>>))
                };
                let port = args.get(0).uint32_value(scope).unwrap_or(0) as u16;
                let host = args.get(1).to_rust_string_lossy(scope);
                let Some(host) = perms.bind_host(&host) else {
                    let err = js_code_err(scope, "EACCES",
                        &format!("Network access denied. Run with --allow-net={}", host));
                    rv.set(err);
                    return;
                };
                match bind_listener(&format!("{}:{}", host, port)) {
                    Ok(std_listener) => {
                        if let Err(e) = std_listener.set_nonblocking(true) {
                            rv.set(js_err(scope, &e.to_string()));
                            return;
                        }
                        match TcpListener::from_std(std_listener) {
                            Ok(listener) => {
                                let listener = Arc::new(listener);
                                let id = { let mut n = nid.lock().unwrap(); let id = *n; *n += 1; id };
                                let state = Arc::new(Mutex::new(H2ServerState {
                                    listener: Some(listener.clone()),
                                    stream_queue: VecDeque::new(),
                                    streams: HashMap::new(),
                                    next_stream_id: 1,
                                }));
                                servers.lock().unwrap().insert(id, state.clone());
                                HTTP_ACTIVE_LISTENERS
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                                let state2 = state.clone();
                                tokio::spawn(async move {
                                    loop {
                                        let (tcp_stream, _) = match listener.accept().await {
                                            Ok(v) => v,
                                            Err(_) => break,
                                        };
                                        let state3 = state2.clone();
                                        tokio::spawn(async move {
                                            let h2_result = h2::server::handshake(tcp_stream).await;
                                            if let Ok(mut conn) = h2_result {
                                                loop {
                                                    match conn.accept().await {
                                                        Some(Ok((request, send_response))) => {
                                                            let mut st = state3.lock().unwrap();
                                                            let sid = st.next_stream_id;
                                                            st.next_stream_id += 1;
                                                            let headers: Vec<(String, String)> = request
                                                                .headers()
                                                                .iter()
                                                                .map(|(k, v)| (
                                                                    k.as_str().to_string(),
                                                                    v.to_str().unwrap_or("").to_string(),
                                                                ))
                                                                .collect();
                                                            let method = request.method().to_string();
                                                            let path = request.uri().to_string();
                                                            let hdr_json: String = {
                                                                let mut parts: Vec<String> = Vec::new();
                                                                for (k, v) in &headers {
                                                                    parts.push(format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)));
                                                                }
                                                                format!("{{{}}}", parts.join(","))
                                                            };
                                                            let info = format!(
                                                                r#"{{"streamId":{},"method":"{}","path":"{}","headers":{}}}"#,
                                                                sid, json_escape(&method), json_escape(&path), hdr_json
                                                            );
                                                            st.streams.insert(sid, H2StreamEntry {
                                                                send_response: Some(send_response),
                                                                send_stream: None,
                                                            });
                                                            st.stream_queue.push_back(info);
                                                            drop(st);
                                                            drop(request);
                                                        }
                                                        Some(Err(_)) => break,
                                                        None => break,
                                                    }
                                                }
                                            }
                                        });
                                    }
                                });
                                rv.set(v8::Integer::new_from_unsigned(scope, id).into());
                            }
                            Err(e) => { rv.set(js_err(scope, &e.to_string())); }
                        }
                    }
                    Err(e) => { rv.set(js_code_err(scope, "EADDRINUSE", &e.to_string())); }
                }
            },
        ).data(external.into()).build(scope).unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2Listen").unwrap().into(),
            listen_fn.into(),
        );
    }

    // __h2AcceptPoll(serverId) → JSON or null
    {
        let servers = h2_servers.clone();
        let ctx_ptr = native_ctx.leak(servers);
        let external = v8::External::new(scope, ctx_ptr);
        let poll_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Servers) };
                let sid = args.get(0).uint32_value(scope).unwrap_or(0);
                let popped = servers
                    .lock()
                    .unwrap()
                    .get(&sid)
                    .and_then(|s| s.lock().unwrap().stream_queue.pop_front());
                match popped {
                    Some(json) => rv.set(V8String::new(scope, &json).unwrap().into()),
                    None => rv.set(v8::null(scope).into()),
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2AcceptPoll").unwrap().into(),
            poll_fn.into(),
        );
    }

    // __h2ServerPort(serverId) → port
    {
        let servers = h2_servers.clone();
        let ctx_ptr = native_ctx.leak(servers);
        let external = v8::External::new(scope, ctx_ptr);
        let port_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Servers) };
                let sid = args.get(0).uint32_value(scope).unwrap_or(0);
                let port = servers
                    .lock()
                    .unwrap()
                    .get(&sid)
                    .and_then(|s| {
                        s.lock()
                            .unwrap()
                            .listener
                            .as_ref()
                            .and_then(|l| l.local_addr().ok())
                            .map(|a| a.port() as u32)
                    })
                    .unwrap_or(0);
                rv.set(v8::Integer::new_from_unsigned(scope, port).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2ServerPort").unwrap().into(),
            port_fn.into(),
        );
    }

    // __h2ServerClose(serverId)
    {
        let servers = h2_servers.clone();
        let ctx_ptr = native_ctx.leak(servers);
        let external = v8::External::new(scope, ctx_ptr);
        let close_fn = v8::Function::builder(
            |_scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Servers) };
                let sid = args.get(0).uint32_value(_scope).unwrap_or(0);
                if let Some(state) = servers.lock().unwrap().remove(&sid) {
                    state.lock().unwrap().listener.take();
                    HTTP_ACTIVE_LISTENERS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
                rv.set(v8::undefined(_scope).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2ServerClose").unwrap().into(),
            close_fn.into(),
        );
    }

    // __h2StreamRespond(serverId, streamId, headers_json) → bool
    {
        let servers = h2_servers.clone();
        let ctx_ptr = native_ctx.leak(servers);
        let external = v8::External::new(scope, ctx_ptr);
        let respond_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Servers) };
                let sid = args.get(0).uint32_value(scope).unwrap_or(0);
                let stream_id = args.get(1).uint32_value(scope).unwrap_or(0);
                let headers_json = args.get(2).to_rust_string_lossy(scope);

                let mut builder = http::response::Builder::new();
                if let Some(obj) = serde_json::from_str::<serde_json::Value>(&headers_json)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
                {
                    for (k, v) in &obj {
                        let vs = v.as_str().unwrap_or("");
                        if k == ":status" {
                            if let Ok(status) = vs.parse::<u16>() {
                                builder = builder.status(status);
                            }
                        } else {
                            builder = builder.header(k.as_str(), vs);
                        }
                    }
                }
                let response = builder
                    .body(())
                    .unwrap_or_else(|_| http::Response::builder().status(200).body(()).unwrap());

                let servers_guard = servers.lock().unwrap();
                if let Some(state) = servers_guard.get(&sid) {
                    let mut st = state.lock().unwrap();
                    if let Some(entry) = st.streams.get_mut(&stream_id) {
                        if let Some(mut send_response) = entry.send_response.take() {
                            match send_response.send_response(response, false) {
                                Ok(send_stream) => {
                                    entry.send_stream = Some(send_stream);
                                    rv.set(v8::Boolean::new(scope, true).into());
                                }
                                Err(_) => {
                                    rv.set(v8::Boolean::new(scope, false).into());
                                }
                            }
                        } else {
                            rv.set(v8::Boolean::new(scope, false).into());
                        }
                    } else {
                        rv.set(v8::Boolean::new(scope, false).into());
                    }
                } else {
                    rv.set(v8::Boolean::new(scope, false).into());
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2StreamRespond").unwrap().into(),
            respond_fn.into(),
        );
    }

    // __h2StreamWrite(serverId, streamId, data) → bool
    {
        let servers = h2_servers.clone();
        let ctx_ptr = native_ctx.leak(servers);
        let external = v8::External::new(scope, ctx_ptr);
        let write_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Servers) };
                let sid = args.get(0).uint32_value(scope).unwrap_or(0);
                let stream_id = args.get(1).uint32_value(scope).unwrap_or(0);
                let data_arg = args.get(2);
                let data: Vec<u8> = if let Ok(s) = v8::Local::<v8::String>::try_from(data_arg) {
                    s.to_rust_string_lossy(scope).into_bytes()
                } else if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                    uint8array_to_vec(scope, arr)
                } else {
                    vec![]
                };

                let servers_guard = servers.lock().unwrap();
                if let Some(state) = servers_guard.get(&sid) {
                    let mut st = state.lock().unwrap();
                    if let Some(entry) = st.streams.get_mut(&stream_id) {
                        if let Some(ref mut send_stream) = entry.send_stream {
                            match send_stream.send_data(bytes::Bytes::from(data), false) {
                                Ok(_) => rv.set(v8::Boolean::new(scope, true).into()),
                                Err(_) => rv.set(v8::Boolean::new(scope, false).into()),
                            }
                        } else {
                            rv.set(v8::Boolean::new(scope, false).into());
                        }
                    } else {
                        rv.set(v8::Boolean::new(scope, false).into());
                    }
                } else {
                    rv.set(v8::Boolean::new(scope, false).into());
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2StreamWrite").unwrap().into(),
            write_fn.into(),
        );
    }

    // __h2StreamEnd(serverId, streamId, data?) → bool
    {
        let servers = h2_servers.clone();
        let ctx_ptr = native_ctx.leak(servers);
        let external = v8::External::new(scope, ctx_ptr);
        let end_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Servers) };
                let sid = args.get(0).uint32_value(scope).unwrap_or(0);
                let stream_id = args.get(1).uint32_value(scope).unwrap_or(0);
                let data_arg = args.get(2);
                let data: Vec<u8> = if data_arg.is_undefined() || data_arg.is_null() {
                    vec![]
                } else if let Ok(s) = v8::Local::<v8::String>::try_from(data_arg) {
                    s.to_rust_string_lossy(scope).into_bytes()
                } else if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                    uint8array_to_vec(scope, arr)
                } else {
                    vec![]
                };

                let servers_guard = servers.lock().unwrap();
                if let Some(state) = servers_guard.get(&sid) {
                    let mut st = state.lock().unwrap();
                    if let Some(entry) = st.streams.get_mut(&stream_id) {
                        if let Some(ref mut send_stream) = entry.send_stream {
                            match send_stream.send_data(bytes::Bytes::from(data), true) {
                                Ok(_) => rv.set(v8::Boolean::new(scope, true).into()),
                                Err(_) => rv.set(v8::Boolean::new(scope, false).into()),
                            }
                        } else {
                            rv.set(v8::Boolean::new(scope, false).into());
                        }
                    } else {
                        rv.set(v8::Boolean::new(scope, false).into());
                    }
                } else {
                    rv.set(v8::Boolean::new(scope, false).into());
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2StreamEnd").unwrap().into(),
            end_fn.into(),
        );
    }

    // __h2Connect(authority) → clientId or -1
    {
        let clients = h2_clients.clone();
        let nid = h2_next_client_id.clone();
        let perms2 = permissions.clone();
        let ctx_ptr = native_ctx.leak((perms2, clients, nid));
        let external = v8::External::new(scope, ctx_ptr);
        let connect_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let (perms, clients, nid) = unsafe {
                    &*(args.data().cast::<v8::External>().value()
                        as *const (Arc<PermissionState>, H2Clients, Arc<Mutex<u32>>))
                };
                let authority = args.get(0).to_rust_string_lossy(scope);
                let host = authority
                    .split(':')
                    .next()
                    .unwrap_or(&authority)
                    .to_string();
                // The whole authority, so a port-scoped grant applies (VULN-17).
                if !perms.check(&vvva_permissions::Capability::Network(authority.clone())) {
                    let err = js_code_err(
                        scope,
                        "EACCES",
                        &format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }
                let addr = if authority.contains(':') {
                    authority.clone()
                } else {
                    format!("{}:80", authority)
                };
                // Dial only vetted addresses, resolved once (VULN-18).
                let vetted = addr
                    .rsplit_once(':')
                    .and_then(|(h, p)| Some((h, p.parse::<u16>().ok()?)))
                    .ok_or_else(|| std::io::Error::other(format!("invalid authority {addr}")))
                    .and_then(|(h, p)| perms.vetted_addrs(h, p));
                let addr = match vetted {
                    Ok(a) => a,
                    Err(e) => {
                        let err = js_code_err(scope, "EACCES", &e.to_string());
                        rv.set(err);
                        return;
                    }
                };
                let id = {
                    let mut n = nid.lock().unwrap();
                    let id = *n;
                    *n += 1;
                    id
                };

                // ponytail: synchronous handshake on a thread with its own
                // runtime, so the V8 thread isn't blocked on tokio. The h2
                // `Connection` captures the runtime handle while it is
                // handshaking, so the handshake AND the driver must run on
                // the same runtime for the connection's lifetime — driving
                // `conn` from a different runtime fails immediately with
                // "A Tokio 1.x context was found, but it is being shutdown."
                // The send_request half is handed back over a channel once
                // the handshake completes; the thread then keeps polling the
                // connection until it closes, at which point it exits.
                let clients2 = clients.clone();
                let (send_request_tx, send_request_rx) = std::sync::mpsc::channel();
                let conn_thread = std::thread::spawn(move || {
                    let rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            let _ = send_request_tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    rt.block_on(async move {
                        let tcp = match tokio::net::TcpStream::connect(&addr[..]).await {
                            Ok(tcp) => tcp,
                            Err(e) => {
                                let _ = send_request_tx.send(Err(e.to_string()));
                                return;
                            }
                        };
                        let (send_request, conn) = match h2::client::handshake(tcp).await {
                            Ok(parts) => parts,
                            Err(e) => {
                                let _ = send_request_tx.send(Err(e.to_string()));
                                return;
                            }
                        };
                        if send_request_tx.send(Ok(send_request)).is_err() {
                            return;
                        }
                        // Drive the connection; returning ends this thread.
                        let _ = conn.await;
                    });
                });
                let handshake_result = match send_request_rx.recv() {
                    Ok(Ok(send_request)) => Ok(send_request),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err("h2 conn thread panicked".to_string()),
                };

                match handshake_result {
                    Ok(send_request) => {
                        let state = Arc::new(Mutex::new(H2ClientState {
                            send_request: Some(send_request),
                            streams: HashMap::new(),
                            next_stream_id: 1,
                            response_queue: VecDeque::new(),
                            conn_thread: Some(conn_thread),
                        }));
                        clients2.lock().unwrap().insert(id, state);
                        rv.set(v8::Integer::new_from_unsigned(scope, id).into());
                    }
                    Err(_) => rv.set(v8::Integer::new(scope, -1).into()),
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2Connect").unwrap().into(),
            connect_fn.into(),
        );
    }

    // __h2ClientReady(clientId) → bool (check if handshake completed)
    {
        let clients = h2_clients.clone();
        let ctx_ptr = native_ctx.leak(clients);
        let external = v8::External::new(scope, ctx_ptr);
        let ready_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let clients =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Clients) };
                let cid = args.get(0).uint32_value(scope).unwrap_or(0);
                let ready = clients
                    .lock()
                    .unwrap()
                    .get(&cid)
                    .map(|s| s.lock().unwrap().send_request.is_some())
                    .unwrap_or(false);
                rv.set(v8::Boolean::new(scope, ready).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2ClientReady").unwrap().into(),
            ready_fn.into(),
        );
    }

    // __h2ClientRequest(clientId, headers_json) → streamId or -1
    {
        let clients = h2_clients.clone();
        let ctx_ptr = native_ctx.leak(clients);
        let external = v8::External::new(scope, ctx_ptr);
        let req_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let clients = unsafe {
                    &*(args.data().cast::<v8::External>().value() as *const H2Clients)
                };
                let cid = args.get(0).uint32_value(scope).unwrap_or(0);
                let headers_json = args.get(1).to_rust_string_lossy(scope);

                let clients_guard = clients.lock().unwrap();
                if let Some(state) = clients_guard.get(&cid) {
                    let mut st = state.lock().unwrap();
                    if let Some(ref mut send_request) = st.send_request {
                        let mut builder = http::request::Builder::new();
                        let end_stream = false;
                        if let Some(obj) = serde_json::from_str::<serde_json::Value>(&headers_json)
                            .ok()
                            .and_then(|v| v.as_object().cloned())
                        {
                            for (k, v) in &obj {
                                let vs = v.as_str().unwrap_or("");
                                match k.as_str() {
                                    ":method" => { builder = builder.method(vs); }
                                    ":path" => { builder = builder.uri(vs); }
                                    ":authority" | ":scheme" => {
                                        builder = builder.header(k.as_str(), vs);
                                    }
                                    _ => { builder = builder.header(k.as_str(), vs); }
                                }
                            }
                        }
                        let request = builder.body(()).unwrap_or_else(|_| {
                            http::Request::builder().method("GET").uri("/").body(()).unwrap()
                        });
                        match send_request.send_request(request, end_stream) {
                            Ok((response, send_stream)) => {
                                let stream_id = st.next_stream_id;
                                st.next_stream_id += 1;
                                st.streams.insert(stream_id, H2ClientStreamEntry {
                                    send_stream: Some(send_stream),
                                });
                                drop(st);
                                drop(clients_guard);
                                let clients3 = clients.clone();
                                let cid2 = cid;
                                let sid2 = stream_id;
                                tokio::spawn(async move {
                                    if let Ok(resp) = response.await {
                                            let status = resp.status().as_u16();
                                            let hdrs: Vec<(String, String)> = resp.headers().iter()
                                                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                                                .collect();
                                            let hdr_json: String = {
                                                let mut parts: Vec<String> = Vec::new();
                                                for (k, v) in &hdrs {
                                                    parts.push(format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)));
                                                }
                                                format!("{{{}}}", parts.join(","))
                                            };
                                            let info = format!(
                                                r#"{{"type":"response","streamId":{},"status":{},"headers":{}}}"#,
                                                sid2, status, hdr_json
                                            );
                                            if let Some(state) = clients3.lock().unwrap().get(&cid2) {
                                                state.lock().unwrap().response_queue.push_back(info);
                                            }
                                            let mut body = resp.into_body();
                                            while let Some(chunk) = body.data().await {
                                                match chunk {
                                                    Ok(data) => {
                                                        let info = format!(
                                                            r#"{{"type":"data","streamId":{},"data":"{}"}}"#,
                                                            sid2, json_escape(&String::from_utf8_lossy(&data))
                                                        );
                                                        if let Some(state) = clients3.lock().unwrap().get(&cid2) {
                                                            state.lock().unwrap().response_queue.push_back(info);
                                                        }
                                                    }
                                                    Err(_) => break,
                                                }
                                            }
                                            let info = format!(
                                                r#"{{"type":"end","streamId":{}}}"#,
                                                sid2
                                            );
                                            if let Some(state) = clients3.lock().unwrap().get(&cid2) {
                                                state.lock().unwrap().response_queue.push_back(info);
                                            }
                                    }
                                });
                                rv.set(v8::Integer::new_from_unsigned(scope, stream_id).into());
                            }
                            Err(_) => {
                                rv.set(v8::Integer::new(scope, -1).into());
                            }
                        }
                    } else {
                        rv.set(v8::Integer::new(scope, -1).into());
                    }
                } else {
                    rv.set(v8::Integer::new(scope, -1).into());
                }
            },
        ).data(external.into()).build(scope).unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2ClientRequest").unwrap().into(),
            req_fn.into(),
        );
    }

    // __h2ClientStreamWrite(clientId, streamId, data) → bool
    {
        let clients = h2_clients.clone();
        let ctx_ptr = native_ctx.leak(clients);
        let external = v8::External::new(scope, ctx_ptr);
        let write_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let clients =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Clients) };
                let cid = args.get(0).uint32_value(scope).unwrap_or(0);
                let stream_id = args.get(1).uint32_value(scope).unwrap_or(0);
                let data_arg = args.get(2);
                let data: Vec<u8> = if let Ok(s) = v8::Local::<v8::String>::try_from(data_arg) {
                    s.to_rust_string_lossy(scope).into_bytes()
                } else if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                    uint8array_to_vec(scope, arr)
                } else {
                    vec![]
                };
                let clients_guard = clients.lock().unwrap();
                if let Some(state) = clients_guard.get(&cid) {
                    let mut st = state.lock().unwrap();
                    if let Some(entry) = st.streams.get_mut(&stream_id) {
                        if let Some(ref mut ss) = entry.send_stream {
                            match ss.send_data(bytes::Bytes::from(data), false) {
                                Ok(_) => rv.set(v8::Boolean::new(scope, true).into()),
                                Err(_) => rv.set(v8::Boolean::new(scope, false).into()),
                            }
                        } else {
                            rv.set(v8::Boolean::new(scope, false).into());
                        }
                    } else {
                        rv.set(v8::Boolean::new(scope, false).into());
                    }
                } else {
                    rv.set(v8::Boolean::new(scope, false).into());
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2ClientStreamWrite")
                .unwrap()
                .into(),
            write_fn.into(),
        );
    }

    // __h2ClientStreamEnd(clientId, streamId, data?) → bool
    {
        let clients = h2_clients.clone();
        let ctx_ptr = native_ctx.leak(clients);
        let external = v8::External::new(scope, ctx_ptr);
        let end_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let clients =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Clients) };
                let cid = args.get(0).uint32_value(scope).unwrap_or(0);
                let stream_id = args.get(1).uint32_value(scope).unwrap_or(0);
                let data_arg = args.get(2);
                let data: Vec<u8> = if data_arg.is_undefined() || data_arg.is_null() {
                    vec![]
                } else if let Ok(s) = v8::Local::<v8::String>::try_from(data_arg) {
                    s.to_rust_string_lossy(scope).into_bytes()
                } else if let Ok(arr) = v8::Local::<v8::Uint8Array>::try_from(data_arg) {
                    uint8array_to_vec(scope, arr)
                } else {
                    vec![]
                };
                let clients_guard = clients.lock().unwrap();
                if let Some(state) = clients_guard.get(&cid) {
                    let mut st = state.lock().unwrap();
                    if let Some(entry) = st.streams.get_mut(&stream_id) {
                        if let Some(ref mut ss) = entry.send_stream {
                            match ss.send_data(bytes::Bytes::from(data), true) {
                                Ok(_) => rv.set(v8::Boolean::new(scope, true).into()),
                                Err(_) => rv.set(v8::Boolean::new(scope, false).into()),
                            }
                        } else {
                            rv.set(v8::Boolean::new(scope, false).into());
                        }
                    } else {
                        rv.set(v8::Boolean::new(scope, false).into());
                    }
                } else {
                    rv.set(v8::Boolean::new(scope, false).into());
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2ClientStreamEnd").unwrap().into(),
            end_fn.into(),
        );
    }

    // __h2ClientPoll(clientId) → JSON or null
    {
        let clients = h2_clients.clone();
        let ctx_ptr = native_ctx.leak(clients);
        let external = v8::External::new(scope, ctx_ptr);
        let poll_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let clients =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Clients) };
                let cid = args.get(0).uint32_value(scope).unwrap_or(0);
                let popped = clients
                    .lock()
                    .unwrap()
                    .get(&cid)
                    .and_then(|s| s.lock().unwrap().response_queue.pop_front());
                match popped {
                    Some(json) => rv.set(V8String::new(scope, &json).unwrap().into()),
                    None => rv.set(v8::null(scope).into()),
                }
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2ClientPoll").unwrap().into(),
            poll_fn.into(),
        );
    }

    // __h2ClientClose(clientId)
    {
        let clients = h2_clients.clone();
        let ctx_ptr = native_ctx.leak(clients);
        let external = v8::External::new(scope, ctx_ptr);
        let close_fn = v8::Function::builder(
            |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let clients =
                    unsafe { &*(args.data().cast::<v8::External>().value() as *const H2Clients) };
                let cid = args.get(0).uint32_value(scope).unwrap_or(0);
                if let Some(state) = clients.lock().unwrap().remove(&cid) {
                    // Take the driver thread handle so it is dropped here (the
                    // thread itself exits once the connection closes). The
                    // field otherwise reads as unused.
                    if let Ok(mut st) = state.lock() {
                        st.conn_thread.take();
                    }
                }
                rv.set(v8::undefined(scope).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__h2ClientClose").unwrap().into(),
            close_fn.into(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

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
        let mut reader = BufReader::new(server_stream);
        let result = parse_request(
            &mut reader,
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
        let req = "POST /slow HTTP/1.1\r\nContent-Length: 200\r\n\r\n";
        tokio::spawn(async move {
            client.write_all(req.as_bytes()).await.unwrap();
            for _ in 0..200u8 {
                client.write_all(b"x").await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(server_stream);
        let result = parse_request(
            &mut reader,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(30),
            100,
            16_384,
            0,
            50,
        )
        .await;
        assert!(matches!(result, Err(ParseError::Silent(_))));
        let ParseError::Silent(msg) = result.unwrap_err() else {
            unreachable!()
        };
        assert!(
            msg.contains("rate too low") || msg.contains("stalled") || msg.contains("RUDY"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn body_total_deadline_fires_when_rate_check_disabled() {
        let (listener, mut client) = loopback_pair().await;
        let req = "POST /hang HTTP/1.1\r\nContent-Length: 1000\r\n\r\n";
        tokio::spawn(async move {
            client.write_all(req.as_bytes()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(server_stream);
        let result = parse_request(
            &mut reader,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(200),
            100,
            16_384,
            0,
            0,
        )
        .await;
        assert!(matches!(result, Err(ParseError::Silent(_))));
        let ParseError::Silent(msg) = result.unwrap_err() else {
            unreachable!()
        };
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
        let mut reader = BufReader::new(server_stream);
        let result = parse_request(
            &mut reader,
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

    #[tokio::test]
    async fn chunked_body_decoded() {
        let (listener, mut client) = loopback_pair().await;
        tokio::spawn(async move {
            client
                .write_all(
                    b"POST /chunked HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n\
                      5\r\nhello\r\n6\r\n world\r\n0\r\nX-Trailer: v\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(server_stream);
        let parsed = parse_request(
            &mut reader,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            100,
            16_384,
            0,
            0,
        )
        .await
        .expect("chunked request must parse");
        assert_eq!(parsed.body, b"hello world");
    }

    #[tokio::test]
    async fn multi_chunk_body_with_large_chunk_decoded() {
        let (listener, mut client) = loopback_pair().await;
        let big_chunk = vec![b'a'; 40_000]; // larger than one 16 KiB read buffer
        let mut expected = big_chunk.clone();
        tokio::spawn(async move {
            let head =
                b"POST /m HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: CHUNKED\r\n\r\n".to_vec();
            client.write_all(&head).await.unwrap();
            client
                .write_all(format!("{:x}\r\n", big_chunk.len()).as_bytes())
                .await
                .unwrap();
            client.write_all(&big_chunk).await.unwrap();
            client.write_all(b"\r\n1A\r\n").await.unwrap();
            client.write_all(&[b'b'; 26]).await.unwrap();
            client.write_all(b"\r\n0\r\n\r\n").await.unwrap();
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(server_stream);
        let parsed = parse_request(
            &mut reader,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            100,
            16_384,
            0,
            0,
        )
        .await
        .expect("multi-chunk request must parse");
        expected.extend_from_slice(&[b'b'; 26]);
        assert_eq!(parsed.body.len(), expected.len());
        assert_eq!(parsed.body, expected);
    }

    #[tokio::test]
    async fn content_length_plus_transfer_encoding_rejected_as_smuggling() {
        let (listener, mut client) = loopback_pair().await;
        tokio::spawn(async move {
            client
                .write_all(
                    b"POST /smuggle HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\
                      Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(server_stream);
        let result = parse_request(
            &mut reader,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            100,
            16_384,
            0,
            0,
        )
        .await;
        assert!(
            matches!(result, Err(ParseError::Respond(400, "Bad Request"))),
            "CL+TE must be rejected with 400, got {result:?}"
        );
    }

    #[tokio::test]
    async fn malformed_chunk_size_rejected_with_400() {
        let (listener, mut client) = loopback_pair().await;
        tokio::spawn(async move {
            client
                .write_all(
                    b"POST /bad HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n\
                      zz\r\nhello\r\n0\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(server_stream);
        let result = parse_request(
            &mut reader,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            100,
            16_384,
            0,
            0,
        )
        .await;
        assert!(matches!(result, Err(ParseError::Respond(400, _))));
    }

    #[tokio::test]
    async fn unsupported_transfer_coding_rejected_with_501() {
        let (listener, mut client) = loopback_pair().await;
        tokio::spawn(async move {
            client
                .write_all(b"POST /gzip HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip\r\n\r\n...")
                .await
                .unwrap();
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(server_stream);
        let result = parse_request(
            &mut reader,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            100,
            16_384,
            0,
            0,
        )
        .await;
        assert!(matches!(result, Err(ParseError::Respond(501, _))));
    }

    // ── upgrade bridge ────────────────────────────────────────────────────

    fn test_upgrade() -> (Arc<UpgradeConn>, UpgradeRegistry) {
        let conn = Arc::new(UpgradeConn::new());
        let registry: UpgradeRegistry = Arc::new(Mutex::new(HashMap::new()));
        registry.lock().unwrap().insert(7, conn.clone());
        (conn, registry)
    }

    fn loopback_ip() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    #[tokio::test]
    async fn upgrade_driver_stops_reading_when_js_does_not_drain() {
        use std::sync::atomic::Ordering::Relaxed;
        let (server_end, mut peer) = tokio::io::duplex(64 * 1024);
        let (conn, registry) = test_upgrade();
        spawn_upgrade_driver(
            server_end,
            loopback_ip(),
            7,
            conn.clone(),
            registry,
            Arc::new(None),
        );
        // A fast peer: 16 MiB, while JS never reads.
        let flood = tokio::spawn(async move {
            let chunk = vec![7u8; 64 * 1024];
            for _ in 0..256 {
                if peer.write_all(&chunk).await.is_err() {
                    return false;
                }
            }
            true
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let queued = conn.read_bytes.load(Relaxed);
        assert!(
            queued <= UPGRADE_READ_HIGH_WATER + 16 * 1024,
            "read ahead of JS must stop at the high-water mark, queued {queued} bytes"
        );
        assert!(
            !flood.is_finished(),
            "the peer's write must be held back by TCP flow control, not absorbed"
        );
        // JS starts reading: everything arrives, in the end.
        let drain = async {
            let mut total = 0usize;
            while total < 256 * 64 * 1024 {
                match take_upgrade_read(&conn, 65_536) {
                    UpgradeTake::Data(b) => total += b.len(),
                    _ => tokio::time::sleep(std::time::Duration::from_millis(1)).await,
                }
            }
            total
        };
        let total = tokio::time::timeout(std::time::Duration::from_secs(20), drain)
            .await
            .expect("draining the queue must let the peer finish");
        assert_eq!(total, 256 * 64 * 1024);
        assert!(flood.await.unwrap());
    }

    #[tokio::test]
    async fn upgrade_driver_flushes_writes_queued_before_close() {
        use tokio::io::AsyncReadExt;
        let (server_end, mut peer) = tokio::io::duplex(64 * 1024);
        let (conn, registry) = test_upgrade();
        let driver = spawn_upgrade_driver(
            server_end,
            loopback_ip(),
            7,
            conn.clone(),
            registry,
            Arc::new(None),
        );
        // What `socket.end(frame)` does: write, then close right away.
        assert!(queue_upgrade_write(&conn, b"closing frame".to_vec()));
        conn.closed
            .store(true, std::sync::atomic::Ordering::Relaxed);
        conn.write_wake.notify_one();

        let mut got = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            peer.read_to_end(&mut got),
        )
        .await
        .expect("the driver must close the connection")
        .unwrap();
        assert_eq!(
            got, b"closing frame",
            "the last write must not be lost to the close"
        );
        driver.await.unwrap();
    }

    #[tokio::test]
    async fn upgrade_write_overflow_drops_the_connection_instead_of_buffering() {
        use std::sync::atomic::Ordering::Relaxed;
        let (server_end, _peer) = tokio::io::duplex(1024); // a peer that never reads
        let (conn, registry) = test_upgrade();
        let driver = spawn_upgrade_driver(
            server_end,
            loopback_ip(),
            7,
            conn.clone(),
            registry.clone(),
            Arc::new(None),
        );
        let chunk = vec![1u8; 1024 * 1024];
        let mut accepted = 0usize;
        while queue_upgrade_write(&conn, chunk.clone()) {
            accepted += chunk.len();
            assert!(
                accepted <= UPGRADE_WRITE_HARD_LIMIT,
                "queue grew past its limit"
            );
        }
        assert!(conn.aborted.load(Relaxed) && conn.closed.load(Relaxed));
        tokio::time::timeout(std::time::Duration::from_secs(5), driver)
            .await
            .expect("an aborted connection must not wait for a peer that never reads")
            .unwrap();
        assert!(
            registry.lock().unwrap().is_empty(),
            "the registry entry is released"
        );
    }

    #[test]
    fn upgrade_read_keeps_the_rest_of_a_chunk_that_js_did_not_ask_for() {
        use std::sync::atomic::Ordering::Relaxed;
        let conn = UpgradeConn::new();
        conn.read_bytes.store(10, Relaxed);
        conn.read_q
            .lock()
            .unwrap()
            .push_back(UpgradeRead::Data(b"0123456789".to_vec()));
        let take = |max| match take_upgrade_read(&conn, max) {
            UpgradeTake::Data(b) => b,
            _ => panic!("expected data"),
        };
        assert_eq!(take(4), b"0123");
        assert_eq!(conn.read_bytes.load(Relaxed), 6);
        assert_eq!(take(4), b"4567");
        assert_eq!(take(4), b"89");
        assert_eq!(conn.read_bytes.load(Relaxed), 0);
        assert!(matches!(take_upgrade_read(&conn, 4), UpgradeTake::Empty));
    }
}
