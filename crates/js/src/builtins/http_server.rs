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
    head: ParsedHead,
    conn_id: u32,
    remote: IpAddr,
    /// Registry id of the streaming body, or 0 when the request has none.
    body_id: u32,
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

/// The timeouts the application configured on the server
/// (`server.headersTimeout`, `server.requestTimeout`, `server.keepAliveTimeout`).
///
/// Node lets the application set them; 3va's firewall has its own limits
/// (the slowloris protection). The application's value wins when it is
/// stricter, and the firewall's stays in force when the application left the
/// timeout disabled (`0`), so the protection cannot be turned off by
/// accident.
#[derive(Debug, Clone, Copy, Default)]
struct ServerTimeouts {
    /// `None`: the firewall's header timeout.
    headers: Option<std::time::Duration>,
    /// `None`: no application-wide request deadline.
    request: Option<std::time::Duration>,
    /// `None`: the firewall's keep-alive timeout.
    keepalive: Option<std::time::Duration>,
}

struct HttpListenCtx {
    perms: Arc<PermissionState>,
    servers: Arc<Mutex<HashMap<u32, Arc<TcpListener>>>>,
    nid: Arc<Mutex<u32>>,
    fw: Arc<Option<Arc<Firewall>>>,
    conns: Arc<Mutex<HashMap<u32, ConnEntry>>>,
    conn_nid: Arc<Mutex<u32>>,
    bodies: BodyRegistry,
    body_nid: Arc<Mutex<u32>>,
    ready: ReadyQueue,
    upgrades: UpgradeRegistry,
    upgrades_q: UpgradeQueue,
    wake: Arc<tokio::sync::Notify>,
    /// The timeouts the application set, per server.
    timeouts: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerTimeouts>>>>>,
}

struct HttpAcceptCtx {
    ready: ReadyQueue,
}

/// The per-server timeout cells the application configures.
struct HttpTimeoutsCtx {
    timeouts: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerTimeouts>>>>>,
}

struct HttpBodyCtx {
    bodies: BodyRegistry,
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
    head: ParsedHead,
    conn_id: u32,
    remote: IpAddr,
    head_bytes: Vec<u8>,
}

type UpgradeQueue = Arc<Mutex<HashMap<u32, std::collections::VecDeque<UpgradeRequest>>>>;
type UpgradeRegistry = Arc<Mutex<HashMap<u32, Arc<UpgradeConn>>>>;

/// Whether `parsed` is an HTTP upgrade request (any `Upgrade:` target — the
/// `ws` library filters for `websocket` itself).
fn is_upgrade_request(parsed: &ParsedHead) -> bool {
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
    /// Whether the connection is closed after this response (the client said
    /// `Connection: close`, or it is an HTTP/1.0 request without keep-alive).
    /// The response head carries the matching `Connection` header, as in Node.
    close_after: std::sync::atomic::AtomicBool,
    /// The HTTP version of the request: an HTTP/1.0 request is answered with
    /// an HTTP/1.0 status line.
    http10: std::sync::atomic::AtomicBool,
}

struct PendingResponse {
    conn_id: u32,
    bytes: Vec<u8>,
}

// ── Streaming request bodies ────────────────────────────────────────────────
//
// The request body is no longer buffered whole in Rust and handed to JS as one
// lossy string (which corrupted non-UTF-8 bytes and, for a large upload, held
// several copies — a 300 MB POST panicked in `V8String::new(...).unwrap()`).
// Instead the connection task reads the socket in bounded chunks into a
// per-request `BodyPipe` and JS pulls them with `__httpReadBody(bodyId, max)`
// as its Readable stream is consumed. The pipe applies backpressure: the
// reader stops once `BODY_PIPE_MAX` bytes are buffered ahead of JS.

/// Most bytes buffered ahead of JS for a single request body. Bounds memory
/// regardless of the declared `Content-Length` (which can be hundreds of MB).
const BODY_PIPE_MAX: usize = 1 << 20;

/// Largest single chunk `__httpReadBody` returns, matching the `net` poll read.
const BODY_READ_MAX: usize = 64 * 1024;

struct BodyState {
    queue: std::collections::VecDeque<Vec<u8>>,
    queued: usize,
    /// Set once the whole declared body has been read (clean EOF).
    eof: bool,
    /// Set when the body read failed (client aborted, framing error, timeout).
    /// Delivered to JS as a stream error after any queued chunks are drained.
    error: Option<String>,
    /// Set when JS has consumed EOF or the pipe is otherwise abandoned; the
    /// producer stops waiting and the entry can be dropped.
    released: bool,
}

/// A request body in transit between the connection task (producer) and the JS
/// `IncomingMessage` stream (consumer).
struct BodyPipe {
    state: Mutex<BodyState>,
    /// Signalled when JS drains the queue, so a producer blocked on
    /// backpressure resumes.
    space: tokio::sync::Notify,
}

impl BodyPipe {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(BodyState {
                queue: std::collections::VecDeque::new(),
                queued: 0,
                eof: false,
                error: None,
                released: false,
            }),
            space: tokio::sync::Notify::new(),
        })
    }

    /// Enqueue `chunk`, waiting for JS to make room when the pipe is full.
    /// Returns without enqueuing if JS released the pipe (stream destroyed or
    /// connection torn down), so the producer stops reading the socket.
    async fn push(&self, chunk: Vec<u8>) {
        if chunk.is_empty() {
            return;
        }
        loop {
            {
                let mut st = self.state.lock().unwrap();
                if st.released {
                    return;
                }
                // Always accept at least one chunk, even when it alone exceeds
                // the cap (a single reads are bounded by the caller).
                if st.queued + chunk.len() <= BODY_PIPE_MAX || st.queue.is_empty() {
                    st.queued += chunk.len();
                    st.queue.push_back(chunk);
                    return;
                }
            }
            self.space.notified().await;
        }
    }

    /// Mark the body as fully read.
    fn finish(&self) {
        self.state.lock().unwrap().eof = true;
    }

    /// Mark the body as failed; the error is delivered to JS.
    fn fail(&self, msg: impl Into<String>) {
        let mut st = self.state.lock().unwrap();
        st.error = Some(msg.into());
        st.eof = true;
    }

    /// Pop up to `max` bytes for JS.
    fn read(&self, max: usize) -> BodyRead {
        let max = max.max(1);
        let mut st = self.state.lock().unwrap();
        let head_len = st.queue.front().map(|f| f.len()).unwrap_or(0);
        if head_len > 0 {
            let out: Vec<u8> = if head_len <= max {
                st.queue.pop_front().unwrap_or_default()
            } else {
                let head: Vec<u8> = st.queue.front().unwrap()[..max].to_vec();
                if let Some(front) = st.queue.front_mut() {
                    front.drain(..max);
                }
                head
            };
            st.queued = st.queued.saturating_sub(out.len());
            drop(st);
            self.space.notify_one();
            return BodyRead::Data(out);
        }
        if let Some(err) = st.error.take() {
            st.released = true;
            return BodyRead::Error(err);
        }
        if st.eof {
            st.released = true;
            return BodyRead::Eof;
        }
        BodyRead::Eagain
    }
}

enum BodyRead {
    Data(Vec<u8>),
    Eof,
    Eagain,
    Error(String),
}

/// Registry of live request bodies, keyed by a per-request id returned to JS.
type BodyRegistry = Arc<Mutex<HashMap<u32, Arc<BodyPipe>>>>;

fn js_err<'s>(scope: &mut PinScope<'s, '_>, msg: &str) -> v8::Local<'s, v8::Value> {
    // `msg` can embed text that reached us from the client (e.g. a parse error
    // string); escape it so it cannot break out of the JS string literal.
    let src = format!("new Error(\"{}\")", json_escape(msg));
    let source = match V8String::new(scope, &src) {
        Some(s) => s,
        None => return v8::undefined(scope).into(),
    };
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
        json_escape(msg),
        json_escape(code)
    );
    let source = match V8String::new(scope, &src) {
        Some(s) => s,
        None => return v8::undefined(scope).into(),
    };
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

/// The `Date` header value Node sends on every response (RFC 1123, UTC).
///
/// The clock is read per response (not cached): a server that runs for days
/// must not send a stale date.
fn http_date_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_http_date(now)
}

/// Format a Unix timestamp as an HTTP date (`Sun, 06 Nov 1994 08:49:37 GMT`).
fn format_http_date(secs: u64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let mut year = 1970i64;
    let mut d = days as i64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let len = if leap { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let month_lengths = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0usize;
    while month < 12 && d >= month_lengths[month] {
        d -= month_lengths[month];
        month += 1;
    }
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[(days % 7) as usize],
        d + 1,
        MONTHS[month],
        year,
        hour,
        min,
        sec
    )
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

/// A request head (request line + headers), with the body framing left to be
/// streamed separately by `pump_body`.
#[derive(Debug)]
pub struct ParsedHead {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    http10: bool,
    framing: BodyFraming,
}

/// How the request body is framed, decided from the headers only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFraming {
    /// No body (`GET`, or `Content-Length: 0`).
    None,
    /// Fixed `Content-Length`.
    Length(usize),
    /// `Transfer-Encoding: chunked` (already validated as the final coding).
    Chunked,
}

/// A fully-parsed request including its buffered body. Kept for unit tests and
/// any caller that wants the whole body at once; the server itself uses
/// `parse_request_head` + `pump_body` so it never holds a large body in memory.
#[allow(dead_code)]
#[derive(Debug)]
pub struct ParsedRequest {
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
pub enum ParseError {
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

/// Parse the request line and headers, deciding the body framing. The body
/// itself is streamed by [`pump_body`] (or read whole by the test-only
/// [`parse_request`] wrapper), so a server never has to buffer a large upload
/// in Rust before handing it to JS.
pub async fn parse_request_head<R>(
    reader: &mut R,
    header_timeout: std::time::Duration,
    max_header_count: usize,
    max_header_bytes: usize,
    max_body_bytes: usize,
) -> Result<ParsedHead, ParseError>
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

    // Count the request line against the header budget so an oversized
    // method/target (client-controlled) can never reach V8 as a string and hit
    // the `V8String::new(...).unwrap()` in `__httpAcceptPoll`.
    let request_line_len = request_line.len();
    if request_line_len > max_header_bytes {
        return Err(ParseError::Silent(format!(
            "request line flood: exceeded {max_header_bytes} bytes"
        )));
    }
    let request_line = request_line.trim_end_matches(['\r', '\n']);
    if request_line.is_empty() {
        // EOF (or a stray blank line at the start of a keep-alive connection):
        // there is no request here. Without this, an empty line parsed as a
        // request with an empty method and reached the app as a phantom request.
        return Err(ParseError::Silent(
            "connection closed before request line".into(),
        ));
    }
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
    let mut total_header_bytes: usize = request_line_len;

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

        // RFC 9112 §5: obsolete line folding (a continuation line starting with
        // SP/HTAB) and whitespace between the field-name and ':' are framing
        // vectors when an upstream proxy unfolds/parses them differently.
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(ParseError::Respond(400, "Bad Request"));
        }
        let Some(colon) = trimmed.find(':') else {
            return Err(ParseError::Respond(400, "Bad Request"));
        };
        let raw_name = &trimmed[..colon];
        if raw_name.is_empty() || raw_name.bytes().any(|b| b == b' ' || b == b'\t') {
            return Err(ParseError::Respond(400, "Bad Request"));
        }
        let name = raw_name.to_ascii_lowercase();
        let value = trimmed[colon + 1..].trim().to_string();
        match name.as_str() {
            "content-length" => {
                // A non-digit or overflowing value is a framing error; treating
                // it as 0 (or as the last of several values) is a request
                // smuggling vector against an upstream proxy (RFC 9112 §6.3).
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(ParseError::Respond(400, "Bad Request"));
                }
                let Ok(parsed) = value.parse::<usize>() else {
                    return Err(ParseError::Respond(413, "Payload Too Large"));
                };
                if has_content_length && parsed != content_length {
                    return Err(ParseError::Respond(400, "Bad Request"));
                }
                // A body larger than the server accepts must be refused, not
                // silently truncated: the unread remainder would be parsed as
                // the next request on the connection.
                if parsed > max_body {
                    return Err(ParseError::Respond(413, "Payload Too Large"));
                }
                content_length = parsed;
                has_content_length = true;
                headers.push((name, content_length.to_string()));
            }
            "transfer-encoding" => {
                let lower = value.to_ascii_lowercase();
                if transfer_encoding
                    .as_deref()
                    .is_some_and(|prev| prev != lower)
                {
                    return Err(ParseError::Respond(400, "Bad Request"));
                }
                transfer_encoding = Some(lower);
                headers.push((name, value));
            }
            _ => headers.push((name, value)),
        }
    }

    // Request smuggling vector: both framing headers present. RFC 9112 says
    // reject; matching llhttp's HPE_UNEXPECTED_CONTENT_LENGTH behaviour.
    if has_content_length && transfer_encoding.is_some() {
        return Err(ParseError::Respond(400, "Bad Request"));
    }

    let framing = if let Some(te) = transfer_encoding {
        // Only "chunked" (as the final coding) can be decoded here; anything
        // else gets 501 Not Implemented per RFC 9112 §6.1.
        let final_coding = te.rsplit(',').next().unwrap_or("").trim();
        if final_coding != "chunked" {
            return Err(ParseError::Respond(501, "Not Implemented"));
        }
        BodyFraming::Chunked
    } else if has_content_length && content_length > 0 {
        BodyFraming::Length(content_length)
    } else {
        BodyFraming::None
    };

    Ok(ParsedHead {
        method,
        path,
        headers,
        http10,
        framing,
    })
}

/// Where a streamed body goes: the per-request `BodyPipe` (production) or a
/// `Vec` (tests / callers that asked for the whole body).
enum BodySink<'a> {
    Pipe(&'a BodyPipe),
    Vec(&'a mut Vec<u8>),
}

async fn body_push(sink: &mut BodySink<'_>, bytes: &[u8]) {
    match sink {
        BodySink::Pipe(p) => p.push(bytes.to_vec()).await,
        BodySink::Vec(v) => v.extend_from_slice(bytes),
    }
}

/// Stream the request body described by `framing` into `sink`, applying the
/// same body deadline and RUDY minimum-rate checks as before. With a
/// `BodySink::Pipe` the producer blocks on backpressure once
/// [`BODY_PIPE_MAX`] bytes are buffered ahead of JS, so memory stays bounded
/// regardless of the declared size.
#[allow(clippy::too_many_arguments)]
async fn pump_body<R>(
    reader: &mut R,
    framing: BodyFraming,
    sink: &mut BodySink<'_>,
    body_timeout: std::time::Duration,
    max_body_bytes: usize,
    min_body_rate_bps: u32,
    max_header_bytes: usize,
) -> Result<(), ParseError>
where
    R: AsyncBufRead + Unpin,
{
    let max_body = if max_body_bytes == 0 {
        HARD_MAX_BODY
    } else {
        max_body_bytes
    };
    match framing {
        BodyFraming::None => Ok(()),
        BodyFraming::Length(len) => {
            pump_length(reader, sink, len, body_timeout, min_body_rate_bps).await
        }
        BodyFraming::Chunked => {
            pump_chunked(
                reader,
                sink,
                body_timeout,
                max_body,
                min_body_rate_bps,
                max_header_bytes,
            )
            .await
        }
    }
}

async fn pump_length<R>(
    reader: &mut R,
    sink: &mut BodySink<'_>,
    content_length: usize,
    body_timeout: std::time::Duration,
    min_body_rate_bps: u32,
) -> Result<(), ParseError>
where
    R: AsyncBufRead + Unpin,
{
    // `Duration::ZERO` means the body deadline is disabled (firewall off).
    let deadline = if body_timeout.is_zero() {
        None
    } else {
        Some(Instant::now() + body_timeout)
    };
    let mut received: usize = 0;
    let body_start = Instant::now();
    let mut buf = vec![0u8; content_length.min(BODY_READ_MAX)];

    while received < content_length {
        if let Some(d) = deadline
            && Instant::now() >= d
        {
            return Err(ParseError::Silent(
                "timeout: body deadline exceeded (RUDY?)".into(),
            ));
        }
        // Bound each individual read so a stalled socket doesn't hang the
        // connection; the overall deadline (when set) is the real cap.
        let chunk_timeout = match deadline {
            None => std::time::Duration::from_secs(1),
            Some(d) => d
                .saturating_duration_since(Instant::now())
                .min(std::time::Duration::from_secs(1)),
        };
        let to_read = (content_length - received).min(buf.len());
        let n = tokio::time::timeout(chunk_timeout, reader.read(&mut buf[..to_read]))
            .await
            .map_err(|_| ParseError::Silent("timeout: body stalled (RUDY?)".into()))?
            .map_err(|e| ParseError::Silent(e.to_string()))?;
        if n == 0 {
            return Err(ParseError::Silent(
                "connection closed before body complete".into(),
            ));
        }
        body_push(sink, &buf[..n]).await;
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
    Ok(())
}

async fn pump_chunked<R>(
    reader: &mut R,
    sink: &mut BodySink<'_>,
    body_timeout: std::time::Duration,
    max_body: usize,
    min_body_rate_bps: u32,
    max_header_bytes: usize,
) -> Result<(), ParseError>
where
    R: AsyncBufRead + Unpin,
{
    let deadline = if body_timeout.is_zero() {
        None
    } else {
        Some(Instant::now() + body_timeout)
    };
    let body_start = Instant::now();
    let mut total: usize = 0;
    let mut buf = vec![0u8; BODY_READ_MAX];

    loop {
        let mut size_line = String::new();
        let read_size = if body_timeout.is_zero() {
            reader
                .read_line(&mut size_line)
                .await
                .map_err(|e| ParseError::Silent(e.to_string()))
        } else {
            tokio::time::timeout(body_timeout, reader.read_line(&mut size_line))
                .await
                .map_err(|_| ParseError::Silent("timeout: chunk size line stalled".into()))?
                .map_err(|e| ParseError::Silent(e.to_string()))
        };
        read_size?;
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
        if chunk_len > max_body || total.saturating_add(chunk_len) > max_body {
            return Err(ParseError::Respond(413, "Payload Too Large"));
        }

        if chunk_len == 0 {
            // Trailer section: read until blank line, discard, bounded.
            let mut trailer_bytes = 0usize;
            loop {
                let mut trailer_line = String::new();
                let read_trailer = if body_timeout.is_zero() {
                    reader
                        .read_line(&mut trailer_line)
                        .await
                        .map_err(|e| ParseError::Silent(e.to_string()))
                } else {
                    tokio::time::timeout(body_timeout, reader.read_line(&mut trailer_line))
                        .await
                        .map_err(|_| ParseError::Silent("timeout: trailers stalled".into()))?
                        .map_err(|e| ParseError::Silent(e.to_string()))
                };
                read_trailer?;
                if trailer_line.is_empty() {
                    return Ok(());
                }
                trailer_bytes += trailer_line.len();
                if trailer_bytes > max_header_bytes {
                    return Err(ParseError::Silent(format!(
                        "trailer flood: exceeded {max_header_bytes} bytes"
                    )));
                }
                if trailer_line.trim_end_matches(['\r', '\n']).is_empty() {
                    return Ok(());
                }
            }
        }

        // Read exactly chunk_len bytes of chunk data, pushing each slice.
        let mut remaining = chunk_len;
        while remaining > 0 {
            if let Some(d) = deadline
                && Instant::now() >= d
            {
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
            body_push(sink, &buf[..n]).await;
            remaining -= n;
            total += n;

            if min_body_rate_bps > 0 {
                let elapsed = body_start.elapsed().as_secs_f64();
                if elapsed > 2.0 {
                    let rate = total as f64 / elapsed;
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
        let read_crlf = if body_timeout.is_zero() {
            reader
                .read_line(&mut crlf)
                .await
                .map_err(|e| ParseError::Silent(e.to_string()))
        } else {
            tokio::time::timeout(body_timeout, reader.read_line(&mut crlf))
                .await
                .map_err(|_| ParseError::Silent("chunk terminator stalled".into()))?
                .map_err(|e| ParseError::Silent(e.to_string()))
        };
        read_crlf?;
        if !crlf.trim_end_matches(['\r', '\n']).is_empty() {
            return Err(ParseError::Respond(400, "Bad Request"));
        }
    }
}

/// Read the whole request (head + body) into memory. Used by unit tests and
/// any caller that wants the body as one value; the server uses
/// `parse_request_head` + `pump_body` so it streams instead.
pub async fn parse_request<R>(
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
    let head = parse_request_head(
        reader,
        header_timeout,
        max_header_count,
        max_header_bytes,
        max_body_bytes,
    )
    .await?;
    let mut body: Vec<u8> = Vec::new();
    {
        let mut sink = BodySink::Vec(&mut body);
        pump_body(
            reader,
            head.framing,
            &mut sink,
            body_timeout,
            max_body_bytes,
            min_body_rate_bps,
            max_header_bytes,
        )
        .await?;
    }
    Ok(ParsedRequest {
        method: head.method,
        path: head.path,
        headers: head.headers,
        body,
        http10: head.http10,
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

/// Human-readable message for a body-read failure, delivered to JS as the
/// request stream's `'error'`.
fn parse_error_message(e: &ParseError) -> String {
    match e {
        ParseError::Respond(status, msg) => format!("HTTP {status} {msg}"),
        ParseError::Silent(m) => m.clone(),
    }
}

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
    bodies: BodyRegistry,
    body_nid: Arc<Mutex<u32>>,
    fw: Arc<Option<Arc<Firewall>>>,
    ready: ReadyQueue,
    upgrades: UpgradeRegistry,
    upgrades_q: UpgradeQueue,
    wake: Arc<tokio::sync::Notify>,
    app_timeouts: Arc<Mutex<ServerTimeouts>>,
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
    conns.lock().unwrap().insert(
        conn_id,
        ConnEntry {
            resp_tx,
            close_after: std::sync::atomic::AtomicBool::new(true),
            http10: std::sync::atomic::AtomicBool::new(false),
        },
    );

    // Connection accounting IP. Normally the peer itself, registered by the
    // accept loop before this task started. When the peer is a trusted reverse
    // proxy, the accept loop deliberately does not count the connection against
    // the proxy's per-IP limit (it fronts every client); instead we defer that
    // accounting until the first request's `X-Forwarded-For` reveals the real
    // client, then register the connection against *that* IP.
    let peer_is_trusted_proxy = fw.as_ref().as_ref().is_some_and(|f| f.is_trusted_proxy(ip));
    let mut account_ip: Option<IpAddr> = if peer_is_trusted_proxy {
        None
    } else {
        Some(ip)
    };
    // Set when a trusted-proxy connection is counted against the global cap
    // only (no `X-Forwarded-For` to attribute it to a client); the matching
    // release at the end uses `on_disconnect_unattributed`.
    let mut account_global_only = false;

    // Requests already served on this TCP connection. The first request gets
    // the full `hdr_timeout`; every reused-connection request afterwards is
    // bounded by the shorter `keepalive_timeout` so an idle keep-alive socket
    // can't be parked open (Slowloris on keep-alive). `max_requests_per_conn`
    // caps how many we serve before closing, regardless of the client's
    // `Connection` header.
    let mut requests_served: u32 = 0;

    // Body id registered for the most recent request on this connection, so it
    // can be released when the connection ends (and the previous one when the
    // next request starts) if JS never drained it.
    let mut my_last_body: Option<u32> = None;

    loop {
        // The timeouts the application set on the server (Node's
        // `headersTimeout`, `requestTimeout` and `keepAliveTimeout`). They
        // are read per request: the application can change them while the
        // server runs.
        let app = *app_timeouts.lock().unwrap();
        let hdr_limit = match app.headers {
            Some(d) if !d.is_zero() => d,
            _ => hdr_timeout,
        };
        let keepalive_limit = match app.keepalive {
            Some(d) if !d.is_zero() => d,
            _ => keepalive_timeout,
        };
        // `requestTimeout` is the deadline for the whole request (head and
        // body). Node answers 408 when it expires.
        let request_deadline = app
            .request
            .filter(|d| !d.is_zero())
            .map(|d| tokio::time::Instant::now() + d);
        // The head has to arrive within the header timeout *and* within the
        // request timeout, whichever is sooner.
        // The first request on a connection gets the header timeout; the
        // reused ones get the (shorter) keep-alive timeout, so an idle
        // keep-alive socket cannot be parked open.
        let hdr_limit = if requests_served == 0 {
            hdr_limit
        } else {
            keepalive_limit
        };
        let hdr_budget = match request_deadline {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(tokio::time::Instant::now());
                if left.is_zero() {
                    let _ = writer
                        .write_all(b"HTTP/1.1 408 Request Timeout\r\nConnection: close\r\n\r\n")
                        .await;
                    let _ = writer.flush().await;
                    break;
                }
                hdr_limit.min(left)
            }
            None => hdr_limit,
        };
        let head = match parse_request_head(
            &mut reader,
            hdr_budget,
            max_hdr_count,
            max_hdr_bytes,
            max_body,
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
            Err(ParseError::Silent(_)) => {
                // The request timeout expired while the head was still coming
                // in: Node answers 408 and closes the connection.
                if request_deadline.is_some_and(|d| tokio::time::Instant::now() >= d) {
                    let _ = writer
                        .write_all(b"HTTP/1.1 408 Request Timeout\r\nConnection: close\r\n\r\n")
                        .await;
                    let _ = writer.flush().await;
                }
                break;
            }
        };

        // Resolve the effective client IP: the peer itself, or — when the
        // peer is a trusted proxy that sent X-Forwarded-For — the forwarded
        // address. Used for both rate-limit accounting and remoteAddress.
        let xff_present = head.headers.iter().any(|(k, _)| k == "x-forwarded-for");
        let req_ip = fw
            .as_ref()
            .as_ref()
            .map(|f| {
                let xff = head
                    .headers
                    .iter()
                    .find(|(k, _)| k == "x-forwarded-for")
                    .map(|(_, v)| v.as_str());
                f.client_ip_from_xff(ip, xff)
            })
            .unwrap_or(ip);

        // Trusted-proxy connections were not counted against the peer at
        // accept time; now that the client IP is known, enforce the per-client
        // connection cap and register the connection against it.
        if account_ip.is_none() {
            if let Some(firewall) = fw.as_ref().as_ref() {
                if !xff_present {
                    // No X-Forwarded-For: the request cannot be attributed to
                    // a client. Count the connection against the global cap
                    // only — never against the proxy's per-IP cap (it fronts
                    // many clients) and never allow it to be auto-blocked (see
                    // `Firewall::check_request`).
                    match firewall.check_connection_behind_proxy(ip) {
                        FirewallDecision::Allow => {
                            firewall.on_connect_unattributed();
                            account_global_only = true;
                        }
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
                } else {
                    match firewall.check_connection(req_ip) {
                        FirewallDecision::Allow => {
                            firewall.on_connect(req_ip);
                            account_ip = Some(req_ip);
                        }
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
            } else {
                account_ip = Some(req_ip);
            }
        }

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
        if is_upgrade_request(&head) {
            let head_bytes = reader.buffer().to_vec();
            drop(reader);
            let conn = Arc::new(UpgradeConn::new());
            upgrades.lock().unwrap().insert(conn_id, conn.clone());
            spawn_upgrade_driver(
                read_half.unsplit(writer),
                account_ip.unwrap_or(ip),
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
                    head,
                    conn_id,
                    remote: req_ip,
                    head_bytes,
                });
            wake.notify_one();
            conns.lock().unwrap().remove(&conn_id);
            // return, not break: the firewall slot now belongs to the driver,
            // which releases it when the upgrade socket closes.
            return;
        }

        requests_served += 1;

        let conn_hdr = head
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
            None => head.http10,
        } || requests_served >= max_requests_per_conn;
        // The response head carries the matching `Connection` header and the
        // version of the request, as in Node.
        if let Some(entry) = conns.lock().unwrap().get(&conn_id) {
            entry
                .close_after
                .store(close_after, std::sync::atomic::Ordering::Relaxed);
            entry
                .http10
                .store(head.http10, std::sync::atomic::Ordering::Relaxed);
        }

        // Register the streaming body (if any) under a per-request id. The
        // previous request's pipe is dropped here: JS has normally consumed it
        // by now, and a read of a released/unknown id is treated as EOF.
        let framing = head.framing;
        let body = match framing {
            BodyFraming::None => None,
            _ => {
                if let Some(prev) = my_last_body.take() {
                    bodies.lock().unwrap().remove(&prev);
                }
                let id = {
                    let mut n = body_nid.lock().unwrap();
                    let v = *n;
                    *n = n.wrapping_add(1).max(1);
                    v
                };
                let pipe = BodyPipe::new();
                bodies.lock().unwrap().insert(id, pipe.clone());
                my_last_body = Some(id);
                Some((id, pipe))
            }
        };
        let body_id = body.as_ref().map(|(id, _)| *id).unwrap_or(0);

        ready
            .lock()
            .unwrap()
            .entry(server_id)
            .or_default()
            .push_back(ReadyRequest {
                head,
                conn_id,
                remote: req_ip,
                body_id,
            });
        wake.notify_one();

        // Read the body into its pipe while waiting for the response. If the
        // response arrives first the body is abandoned and the connection is
        // closed afterwards (an unread body would be misparsed as the next
        // request); the pipe's read then reports EOF.
        let has_body = body.is_some();
        let mut body_done = !has_body;
        let mut body_err: Option<ParseError> = None;
        // The body has to arrive within the request timeout too: the deadline
        // covers the whole request, not only its head.
        let body_budget = match request_deadline {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(tokio::time::Instant::now());
                if left.is_zero() {
                    std::time::Duration::ZERO
                } else {
                    body_timeout.min(left)
                }
            }
            None => body_timeout,
        };
        let resp = {
            let pump_pipe = body.as_ref().map(|(_, p)| p.clone());
            let mut pump = Box::pin(async {
                let pipe = pump_pipe?;
                let mut sink = BodySink::Pipe(&pipe);
                match pump_body(
                    &mut reader,
                    framing,
                    &mut sink,
                    body_budget,
                    max_body,
                    min_body_rate,
                    max_hdr_bytes,
                )
                .await
                {
                    Ok(()) => {
                        pipe.finish();
                        None
                    }
                    Err(e) => {
                        pipe.fail(parse_error_message(&e));
                        Some(e)
                    }
                }
            });
            loop {
                if body_done {
                    // A silent body failure (stall, RUDY, client abort) means
                    // the request can never complete: close now instead of
                    // waiting out the response timeout.
                    if body_err.is_some() {
                        break None;
                    }
                    // Body fully read: bound the wait for JS's response so a
                    // handler that never answers can't pin the connection open.
                    match tokio::time::timeout(RESPONSE_TIMEOUT, resp_rx.recv()).await {
                        Ok(r) => break r,
                        Err(_) => break None,
                    }
                }
                tokio::select! {
                    r = resp_rx.recv() => break r,
                    res = &mut pump => {
                        body_done = true;
                        body_err = res;
                    }
                }
            }
        };

        // A framing error found while streaming (malformed chunked body, 413)
        // is answered here: the request was already handed to JS, but a
        // protocol violation must get its status promptly even if the app
        // ignores the body.
        let had_body_error = body_err.is_some();
        if let Some(ParseError::Respond(status, msg)) = &body_err {
            let resp = format!(
                "HTTP/1.1 {status} {msg}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{msg}",
                len = msg.len()
            );
            let _ = writer.write_all(resp.as_bytes()).await;
            let _ = writer.flush().await;
            break;
        }

        // The request timeout expired while the request was still incomplete:
        // Node answers 408 and closes the connection.
        if !body_done && request_deadline.is_some_and(|d| tokio::time::Instant::now() >= d) {
            let _ = writer
                .write_all(b"HTTP/1.1 408 Request Timeout\r\nConnection: close\r\n\r\n")
                .await;
            let _ = writer.flush().await;
            break;
        }

        let resp = match resp {
            Some(r) if r.conn_id == conn_id => r,
            _ => break,
        };

        if writer.write_all(&resp.bytes).await.is_err() {
            break;
        }
        if writer.flush().await.is_err() {
            break;
        }
        if close_after || !body_done || had_body_error {
            break;
        }
    }

    conns.lock().unwrap().remove(&conn_id);
    if let Some(id) = my_last_body {
        bodies.lock().unwrap().remove(&id);
    }
    // Only release a slot that was actually counted: a trusted-proxy
    // connection that dropped before its first request never registered.
    if let Some(firewall) = fw.as_ref().as_ref() {
        if let Some(account_ip) = account_ip {
            firewall.on_disconnect(account_ip);
        } else if account_global_only {
            firewall.on_disconnect_unattributed();
        }
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
    let bodies: BodyRegistry = Arc::new(Mutex::new(HashMap::new()));
    let next_body_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
    let fw: Arc<Option<Arc<Firewall>>> = Arc::new(firewall);
    let ready: ReadyQueue = Arc::new(Mutex::new(HashMap::new()));
    let upgrades: UpgradeRegistry = Arc::new(Mutex::new(HashMap::new()));
    let upgrades_q: UpgradeQueue = Arc::new(Mutex::new(HashMap::new()));
    let wake = Arc::new(tokio::sync::Notify::new());
    let timeouts: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerTimeouts>>>>> =
        Arc::new(Mutex::new(HashMap::new()));
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
            bodies: bodies.clone(),
            body_nid: next_body_id.clone(),
            ready: ready.clone(),
            upgrades: upgrades.clone(),
            upgrades_q: upgrades_q.clone(),
            wake: wake.clone(),
            nid: next_server_id.clone(),
            fw: fw.clone(),
            timeouts: timeouts.clone(),
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
                                // The timeouts the application sets on this
                                // server (`server.headersTimeout` and friends)
                                // live in a cell the connection tasks read.
                                let server_timeouts =
                                    Arc::new(Mutex::new(ServerTimeouts::default()));
                                ctx.timeouts
                                    .lock()
                                    .unwrap()
                                    .insert(id, server_timeouts.clone());
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
                                            // A trusted reverse proxy fronts many
                                            // clients: don't count its connections
                                            // against a single per-IP cap. Defer to
                                            // the real client IP once parsed.
                                            let trusted_proxy = firewall.is_trusted_proxy(ip);
                                            let decision = if trusted_proxy {
                                                firewall.check_connection_behind_proxy(ip)
                                            } else {
                                                firewall.check_connection(ip)
                                            };
                                            match decision {
                                                FirewallDecision::Allow => {
                                                    if !trusted_proxy {
                                                        firewall.on_connect(ip);
                                                    }
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
                                            if c.enabled {
                                                (
                                                    std::time::Duration::from_millis(
                                                        c.header_timeout_ms,
                                                    ),
                                                    std::time::Duration::from_millis(
                                                        c.body_timeout_ms,
                                                    ),
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
                                                // `enabled: false` disables the
                                                // firewall's protections, including
                                                // the body size cap and body
                                                // deadline (Duration::ZERO =
                                                // disabled; usize::MAX = unlimited).
                                                (
                                                    std::time::Duration::from_millis(
                                                        c.header_timeout_ms,
                                                    ),
                                                    std::time::Duration::ZERO,
                                                    std::time::Duration::from_millis(
                                                        c.keepalive_timeout_ms,
                                                    ),
                                                    c.max_header_count as usize,
                                                    c.max_header_bytes as usize,
                                                    usize::MAX,
                                                    0,
                                                    c.max_requests_per_conn,
                                                )
                                            }
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
                                        let bodies2 = ctx.bodies.clone();
                                        let body_nid2 = ctx.body_nid.clone();
                                        let fw2 = fw.clone();
                                        let ready2 = ready.clone();
                                        let wake2 = wake.clone();
                                        let tls2 = tls.clone();
                                        let upgrades2 = upgrades.clone();
                                        let upgrades_q2 = upgrades_q.clone();
                                        let timeouts2 = server_timeouts.clone();
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
                                                        // Only release the slot if the
                                                        // accept loop actually counted
                                                        // this connection (i.e. the peer
                                                        // is not a deferred trusted proxy).
                                                        if !firewall.is_trusted_proxy(ip) {
                                                            firewall.on_disconnect(ip);
                                                        }
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
                                                    bodies2,
                                                    body_nid2,
                                                    fw2,
                                                    ready2,
                                                    upgrades2,
                                                    upgrades_q2,
                                                    wake2,
                                                    timeouts2,
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
                                                bodies2,
                                                body_nid2,
                                                fw2,
                                                ready2,
                                                upgrades2,
                                                upgrades_q2,
                                                wake2,
                                                timeouts2,
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
                    // [method, url, connId, remoteAddress, bodyId, [k0, v0, k1, v1, ...]]
                    Some(r) => {
                        let p = &r.head;
                        let flat: Vec<v8::Local<v8::Value>> = p
                            .headers
                            .iter()
                            .flat_map(|(k, v)| [k, v])
                            .map(|x| V8String::new(scope, x).unwrap().into())
                            .collect();
                        let headers = v8::Array::new_with_elements(scope, &flat);
                        let items: [v8::Local<v8::Value>; 7] = [
                            V8String::new(scope, &p.method).unwrap().into(),
                            V8String::new(scope, &p.path).unwrap().into(),
                            v8::Integer::new_from_unsigned(scope, r.conn_id).into(),
                            V8String::new(scope, &r.remote.to_string()).unwrap().into(),
                            v8::Integer::new_from_unsigned(scope, r.body_id).into(),
                            headers.into(),
                            // The HTTP version of the request: an HTTP/1.0
                            // request is reported as such, as in Node.
                            V8String::new(scope, if p.http10 { "1.0" } else { "1.1" })
                                .unwrap()
                                .into(),
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

    // ── Streaming request body reader ────────────────────────────────────────
    // JS pulls body chunks from the per-request `BodyPipe` the connection task
    // fills. Returns a Uint8Array, `null` at EOF, a coded Error on failure, or
    // `undefined` when no chunk is ready yet (EAGAIN) — the JS `IncomingMessage`
    // pump retries on the next tick (the same poll shape `net` sockets use).
    {
        let ctx_ptr = native_ctx.leak(HttpBodyCtx {
            bodies: bodies.clone(),
        });
        let external = v8::External::new(scope, ctx_ptr);
        let http_read_body_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpBodyCtx)
                };
                let body_id = args.get(0).uint32_value(scope).unwrap_or(0);
                let max = args
                    .get(1)
                    .uint32_value(scope)
                    .unwrap_or(BODY_READ_MAX as u32)
                    .min(BODY_READ_MAX as u32) as usize;
                let pipe = ctx.bodies.lock().unwrap().get(&body_id).cloned();
                let Some(pipe) = pipe else {
                    // Unknown or already released id: treat as clean EOF so a
                    // stream whose connection ended still finishes.
                    rv.set(v8::null(scope).into());
                    return;
                };
                match pipe.read(max) {
                    BodyRead::Data(bytes) => {
                        let arr = crate::builtins::v8_compat::uint8array_from_bytes(scope, &bytes);
                        rv.set(arr.into());
                    }
                    BodyRead::Eof => {
                        ctx.bodies.lock().unwrap().remove(&body_id);
                        rv.set(v8::null(scope).into());
                    }
                    BodyRead::Eagain => {
                        rv.set(v8::undefined(scope).into());
                    }
                    BodyRead::Error(msg) => {
                        ctx.bodies.lock().unwrap().remove(&body_id);
                        let err = js_code_err(scope, "ECONNRESET", &msg);
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
            V8String::new(scope, "__httpReadBody").unwrap().into(),
            http_read_body_fn.into(),
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
                        let p = &r.head;
                        let flat: Vec<v8::Local<v8::Value>> = p
                            .headers
                            .iter()
                            .flat_map(|(k, v)| [k, v])
                            .map(|x| V8String::new(scope, x).unwrap().into())
                            .collect();
                        let headers = v8::Array::new_with_elements(scope, &flat);
                        let head =
                            crate::builtins::v8_compat::uint8array_from_bytes(scope, &r.head_bytes);
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
                // Whether the connection stays open after this response: the
                // `Connection` header says so, as in Node.
                let close_after = ctx
                    .conns
                    .lock()
                    .unwrap()
                    .get(&conn_id)
                    .map(|c| c.close_after.load(std::sync::atomic::Ordering::Relaxed))
                    .unwrap_or(true);
                let extra = parse_extra_headers(&headers_json);
                // Node's header order (lib/_http_outgoing.js): the headers the
                // application set first, then the ones the server generates.
                let mut resp = format!("HTTP/1.1 {} {}\r\n", status, status_text);
                for (k, v) in &extra {
                    let kl = k.to_lowercase();
                    if kl == "content-length" || kl == "connection" || kl == "date" {
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
                resp.push_str(&format!("Date: {}\r\n", http_date_now()));
                resp.push_str(if close_after {
                    "Connection: close\r\n"
                } else {
                    "Connection: keep-alive\r\n"
                });
                resp.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
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
                // Whether the connection stays open after this response: the
                // `Connection` header says so, as in Node.
                let close_after = ctx
                    .conns
                    .lock()
                    .unwrap()
                    .get(&conn_id)
                    .map(|c| c.close_after.load(std::sync::atomic::Ordering::Relaxed))
                    .unwrap_or(true);

                let extra = parse_extra_headers(&headers_json);
                // `res.sendDate = false` keeps the `Date` header out, as in
                // Node.
                let send_date = args.get(5).boolean_value(scope);
                // `res.removeHeader('Connection')` keeps the server from
                // adding one back, as in Node.
                let send_connection = args.get(6).boolean_value(scope);
                // Node's header order (lib/_http_outgoing.js): the headers the
                // application set first, then the ones the server generates.
                let mut resp = format!("HTTP/1.1 {} {}\r\n", status, status_text);
                for (k, v) in &extra {
                    let kl = k.to_lowercase();
                    if kl == "content-length"
                        || kl == "connection"
                        || kl == "transfer-encoding"
                        || kl == "date"
                    {
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
                if send_date {
                    resp.push_str(&format!("Date: {}\r\n", http_date_now()));
                }
                if send_connection {
                    resp.push_str(if close_after {
                        "Connection: close\r\n"
                    } else {
                        "Connection: keep-alive\r\n"
                    });
                }
                resp.push_str(&format!("Content-Length: {}\r\n", body.len()));
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
        // The timeouts the application set on the server (Node's
        // `headersTimeout`, `requestTimeout` and `keepAliveTimeout`). They
        // are read by the connection tasks, so a change takes effect on the
        // next request without restarting the server.
        let ctx_ptr = native_ctx.leak(HttpTimeoutsCtx {
            timeouts: timeouts.clone(),
        });
        let external = v8::External::new(scope, ctx_ptr);
        let set_timeouts_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let ctx = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const HttpTimeoutsCtx)
                };
                let server_id = args.get(0).uint32_value(scope).unwrap_or(0);
                // A timeout of 0 means "disabled" in Node; the firewall's own
                // limit stays in force in that case.
                let dur = |v: v8::Local<v8::Value>| -> Option<std::time::Duration> {
                    let ms = v.number_value(scope).unwrap_or(0.0);
                    if !ms.is_finite() || ms <= 0.0 {
                        None
                    } else {
                        Some(std::time::Duration::from_millis(ms as u64))
                    }
                };
                let t = ServerTimeouts {
                    headers: dur(args.get(1)),
                    request: dur(args.get(2)),
                    keepalive: dur(args.get(3)),
                };
                if let Some(cell) = ctx.timeouts.lock().unwrap().get(&server_id) {
                    *cell.lock().unwrap() = t;
                }
                rv.set(v8::undefined(scope).into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__httpSetTimeouts").unwrap().into(),
            set_timeouts_fn.into(),
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

    // ── hostile framing (smuggling) ───────────────────────────────────────

    /// One raw request through `parse_request` with the given body cap.
    async fn parse_raw(req: &[u8], max_body: usize) -> Result<ParsedRequest, ParseError> {
        let (listener, mut client) = loopback_pair().await;
        let req = req.to_vec();
        tokio::spawn(async move {
            let _ = client.write_all(&req).await;
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(server_stream);
        parse_request(
            &mut reader,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(5),
            100,
            16_384,
            max_body,
            0,
        )
        .await
    }

    #[tokio::test]
    async fn invalid_content_length_rejected_with_400() {
        let result = parse_raw(
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5x\r\n\r\n",
            0,
        )
        .await;
        assert!(
            matches!(result, Err(ParseError::Respond(400, _))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn content_length_beyond_the_limit_is_refused_not_truncated() {
        // Declares more than the server accepts: it must be refused (413), not
        // silently truncated, or the unread tail reframes as a second request.
        let result = parse_raw(
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 100\r\n\r\n",
            10,
        )
        .await;
        assert!(
            matches!(result, Err(ParseError::Respond(413, _))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn conflicting_duplicate_content_length_rejected_with_400() {
        let result = parse_raw(
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n",
            0,
        )
        .await;
        assert!(
            matches!(result, Err(ParseError::Respond(400, _))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn whitespace_before_colon_rejected_with_400() {
        let result = parse_raw(
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length : 5\r\n\r\n",
            0,
        )
        .await;
        assert!(
            matches!(result, Err(ParseError::Respond(400, _))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn oversized_request_line_is_rejected_before_reaching_v8() {
        // A request target longer than max_header_bytes (16 KiB here) must be
        // refused, not carried to JS as a method/target V8 string.
        let mut req = b"GET /".to_vec();
        req.extend(std::iter::repeat_n(b'a', 20_000));
        req.extend_from_slice(b" HTTP/1.1\r\nHost: x\r\n\r\n");
        let result = parse_raw(&req, 0).await;
        assert!(matches!(result, Err(ParseError::Silent(_))), "{result:?}");
    }

    #[tokio::test]
    async fn obsolete_line_folding_rejected_with_400() {
        let result = parse_raw(
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n Transfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            0,
        )
        .await;
        assert!(
            matches!(result, Err(ParseError::Respond(400, _))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn conflicting_duplicate_transfer_encoding_rejected_with_400() {
        let result = parse_raw(
            b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            0,
        )
        .await;
        assert!(
            matches!(result, Err(ParseError::Respond(400, _))),
            "{result:?}"
        );
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
