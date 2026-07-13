//! HTTP/1.1 server backend for `http.createServer()`.

use crate::builtins::v8_compat::uint8array_to_vec;
use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, SocketAddr, TcpStream as StdTcpStream};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::time::Instant;
use v8::{FunctionCallbackArguments, PinScope, ReturnValue, Script, String as V8String};

use vvva_firewall::{Firewall, FirewallDecision};
use vvva_permissions::PermissionState;

const HARD_MAX_BODY: usize = 100 * 1024 * 1024;

/// Requests that finished parsing, waiting to be picked up by JS, keyed by
/// server id. Populated by the background accept task spawned in
/// `__httpListen`; drained by the non-blocking `__httpAcceptPoll`.
type ReadyQueue = Arc<Mutex<HashMap<u32, std::collections::VecDeque<String>>>>;

struct HttpListenCtx {
    perms: Arc<PermissionState>,
    servers: Arc<Mutex<HashMap<u32, Arc<TcpListener>>>>,
    nid: Arc<Mutex<u32>>,
    fw: Arc<Option<Arc<Firewall>>>,
    conns: Arc<Mutex<HashMap<u32, ConnEntry>>>,
    conn_nid: Arc<Mutex<u32>>,
    ready: ReadyQueue,
}

struct HttpAcceptCtx {
    ready: ReadyQueue,
}

struct HttpRespondCtx {
    conns: Arc<Mutex<HashMap<u32, ConnEntry>>>,
    fw: Arc<Option<Arc<Firewall>>>,
}

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

    let mut request_line = String::new();
    tokio::time::timeout(header_timeout, reader.read_line(&mut request_line))
        .await
        .map_err(|_| "timeout: request line not received in time (Slowloris?)")?
        .map_err(|e| e.to_string())?;

    let request_line = request_line.trim_end_matches(['\r', '\n']);
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

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

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let deadline = Instant::now() + body_timeout;
        let mut received: usize = 0;
        let body_start = Instant::now();

        while received < content_length {
            if Instant::now() >= deadline {
                return Err("timeout: body deadline exceeded (RUDY?)".into());
            }
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

pub fn inject_http_server(
    scope: &mut PinScope,
    permissions: Arc<PermissionState>,
    firewall: Option<Arc<Firewall>>,
) -> anyhow::Result<()> {
    let servers: Arc<Mutex<HashMap<u32, Arc<TcpListener>>>> = Arc::new(Mutex::new(HashMap::new()));
    let conns: Arc<Mutex<HashMap<u32, ConnEntry>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_server_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let next_conn_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
    let fw: Arc<Option<Arc<Firewall>>> = Arc::new(firewall);
    let ready: ReadyQueue = Arc::new(Mutex::new(HashMap::new()));
    let context = scope.get_current_context();
    let global = context.global(scope);

    {
        let ctx_ptr = Box::leak(Box::new(HttpListenCtx {
            perms: permissions.clone(),
            servers: servers.clone(),
            conns: conns.clone(),
            conn_nid: next_conn_id.clone(),
            ready: ready.clone(),
            nid: next_server_id.clone(),
            fw: fw.clone(),
        })) as *mut HttpListenCtx as *mut std::ffi::c_void;
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

                if !ctx.perms.check_bind(&host) {
                    let err = js_code_err(
                        scope,
                        "EACCES",
                        &format!("Network access denied. Run with --allow-net={}", host),
                    );
                    rv.set(err);
                    return;
                }

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
                                            max_hdr_count,
                                            max_hdr_bytes,
                                            max_body,
                                            min_body_rate,
                                        ) = if let Some(firewall) = fw.as_ref().as_ref() {
                                            let c = &firewall.config;
                                            (
                                                std::time::Duration::from_millis(
                                                    c.header_timeout_ms,
                                                ),
                                                std::time::Duration::from_millis(
                                                    c.body_timeout_ms,
                                                ),
                                                c.max_header_count as usize,
                                                c.max_header_bytes as usize,
                                                c.max_body_bytes as usize,
                                                c.min_body_rate_bps,
                                            )
                                        } else {
                                            (
                                                std::time::Duration::from_secs(10),
                                                std::time::Duration::from_secs(30),
                                                100,
                                                16_384,
                                                0,
                                                100,
                                            )
                                        };

                                        let parsed = parse_request(
                                            stream,
                                            hdr_timeout,
                                            body_timeout,
                                            max_hdr_count,
                                            max_hdr_bytes,
                                            max_body,
                                            min_body_rate,
                                        )
                                        .await;

                                        let parsed = match parsed {
                                            Ok(p) => p,
                                            Err(_) => {
                                                if let Some(firewall) = fw.as_ref().as_ref() {
                                                    firewall.on_disconnect(ip);
                                                }
                                                continue;
                                            }
                                        };

                                        if let Some(firewall) = fw.as_ref().as_ref() {
                                            match firewall.check_request(ip) {
                                                FirewallDecision::Allow => {}
                                                decision => {
                                                    let resp = format!(
                                                        "HTTP/1.1 {s} {m}\r\nContent-Length: {l}\r\nConnection: close\r\n\r\n{m}",
                                                        s = decision.http_status(),
                                                        m = decision.message(),
                                                        l = decision.message().len(),
                                                    );
                                                    let _ = {
                                                        let mut s = parsed.std_stream;
                                                        s.write_all(resp.as_bytes())
                                                    };
                                                    firewall.on_disconnect(ip);
                                                    continue;
                                                }
                                            }
                                        }

                                        let conn_id = {
                                            let mut n = conn_nid.lock().unwrap();
                                            let cid = *n;
                                            *n = n.wrapping_add(1);
                                            cid
                                        };
                                        conns.lock().unwrap().insert(
                                            conn_id,
                                            ConnEntry {
                                                stream: parsed.std_stream,
                                            },
                                        );

                                        let hdr_pairs: Vec<String> = parsed
                                            .headers
                                            .iter()
                                            .map(|(k, v)| {
                                                format!(
                                                    "\"{}\":\"{}\"",
                                                    json_escape(k),
                                                    json_escape(v)
                                                )
                                            })
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

                                        ready.lock().unwrap().entry(id).or_default().push_back(json);
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
        let ctx_ptr = Box::leak(Box::new(HttpAcceptCtx {
            ready: ready.clone(),
        })) as *mut HttpAcceptCtx as *mut std::ffi::c_void;
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
                    Some(json) => {
                        let result_str = V8String::new(scope, &json).unwrap();
                        rv.set(result_str.into());
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

    {
        let ctx_ptr = Box::leak(Box::new(HttpRespondCtx {
            conns: conns.clone(),
            fw: fw.clone(),
        })) as *mut HttpRespondCtx as *mut std::ffi::c_void;
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
                resp.push_str("Connection: close\r\n");
                for (k, v) in &extra {
                    let kl = k.to_lowercase();
                    if kl != "content-length" && kl != "connection" {
                        resp.push_str(&format!("{}: {}\r\n", k, v));
                    }
                }
                resp.push_str("\r\n");

                let mut guard = ctx.conns.lock().unwrap();
                match guard.get_mut(&conn_id) {
                    Some(conn) => {
                        if let Err(_e) = conn.stream.write_all(resp.as_bytes()) {
                            rv.set(v8::undefined(scope).into());
                            return;
                        }
                        if let Err(_e) = conn.stream.write_all(body_bytes) {
                            rv.set(v8::undefined(scope).into());
                            return;
                        }
                        let _ = conn.stream.flush();
                    }
                    None => {
                        let err = js_code_err(scope, "ENOENT", "unknown conn_id");
                        rv.set(err);
                        return;
                    }
                }
                drop(guard);

                if let Some(entry) = ctx.conns.lock().unwrap().remove(&conn_id)
                    && let Some(firewall) = ctx.fw.as_ref().as_ref()
                    && let Ok(peer) = entry.stream.peer_addr()
                {
                    firewall.on_disconnect(peer.ip());
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
        let ctx_ptr = Box::leak(Box::new(HttpRespondCtx {
            conns: conns.clone(),
            fw: fw.clone(),
        })) as *mut HttpRespondCtx as *mut std::ffi::c_void;
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
                resp.push_str("Connection: close\r\n");
                for (k, v) in &extra {
                    let kl = k.to_lowercase();
                    if kl != "content-length" && kl != "connection" && kl != "transfer-encoding" {
                        resp.push_str(&format!("{}: {}\r\n", k, v));
                    }
                }
                resp.push_str("\r\n");

                let mut guard = ctx.conns.lock().unwrap();
                match guard.get_mut(&conn_id) {
                    Some(conn) => {
                        if let Err(_e) = conn.stream.write_all(resp.as_bytes()) {
                            rv.set(v8::undefined(scope).into());
                            return;
                        }
                        if let Err(_e) = conn.stream.write_all(&body) {
                            rv.set(v8::undefined(scope).into());
                            return;
                        }
                        let _ = conn.stream.flush();
                    }
                    None => {
                        let err = js_code_err(scope, "ENOENT", "unknown conn_id");
                        rv.set(err);
                        return;
                    }
                }
                drop(guard);

                if let Some(entry) = ctx.conns.lock().unwrap().remove(&conn_id)
                    && let Some(firewall) = ctx.fw.as_ref().as_ref()
                    && let Ok(peer) = entry.stream.peer_addr()
                {
                    firewall.on_disconnect(peer.ip());
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
        let servers_ptr = Box::leak(Box::new(servers.clone()))
            as *const Arc<Mutex<HashMap<u32, Arc<TcpListener>>>>
            as *mut std::ffi::c_void;
        let external = v8::External::new(scope, servers_ptr);
        let http_close_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let servers = unsafe {
                    let ptr = args.data().cast::<v8::External>().value();
                    &*(ptr as *const Arc<Mutex<HashMap<u32, Arc<TcpListener>>>>)
                };
                let server_id_arg = args.get(0);
                let server_id = server_id_arg.uint32_value(scope).unwrap_or(0);
                servers.lock().unwrap().remove(&server_id);
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
            std::time::Duration::from_secs(30),
            100,
            16_384,
            0,
            50,
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
        let req = "POST /hang HTTP/1.1\r\nContent-Length: 1000\r\n\r\n";
        tokio::spawn(async move {
            client.write_all(req.as_bytes()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        });
        let (server_stream, _) = listener.accept().await.unwrap();
        let result = parse_request(
            server_stream,
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(200),
            100,
            16_384,
            0,
            0,
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
