// Tests for http.createServer() — real HTTP/1.1 listener.
//
// The port is bound synchronously (__httpListen) so it is ready immediately after
// eval_to_string returns.  The accept loop uses an async Promise (__httpAcceptAsync),
// so the engine event loop must run concurrently while the HTTP client runs.
//
// Pattern: use an async tokio TcpStream as the client and drive the JS engine
// event loop via `loop { e.idle().await }` in a tokio::select! alongside the client.
//
// Run: cargo test -p vvva_js --test http_server

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use vvva_firewall::{Firewall, FirewallConfig};
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

async fn engine_with_firewall(config: FirewallConfig) -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    let fw = Firewall::new(config);
    JsEngine::new_with_firewall(Arc::new(perms), fw)
        .await
        .unwrap()
}

/// Like `engine_with_firewall`, but also returns the `Firewall` handle so tests
/// can inspect live connection accounting (e.g. `active_connection_count`).
async fn engine_with_firewall_arc(config: FirewallConfig) -> (JsEngine, Arc<Firewall>) {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    let fw = Firewall::new(config);
    let e = JsEngine::new_with_firewall(Arc::new(perms), fw.clone())
        .await
        .unwrap();
    (e, fw)
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

/// Poll until the JS server has bound to `port`, without opening any connection.
///
/// Try to bind to the same port. If binding succeeds the server hasn't taken
/// it yet — drop the listener and retry. If binding fails ("address already in
/// use") the server is listening. This avoids the flaky fixed sleep AND never
/// sends a spurious request that would corrupt request-count assertions in the
/// firewall tests.
async fn wait_for_port(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err() {
            return; // port taken → server is up
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("port {port} never became ready within 5 s");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn raw_http(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap_or_else(|e| panic!("connect to port {}: {}", port, e));

    let req = if body.is_empty() {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            method = method,
            path = path,
        )
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            method = method,
            path = path,
            len = body.len(),
            body = body,
        )
    };

    let result = tokio::time::timeout(Duration::from_secs(5), async {
        stream
            .write_all(req.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        let mut resp = String::new();
        let mut buf = vec![0u8; 4096];
        loop {
            let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            resp.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        Ok::<_, String>(resp)
    })
    .await;

    match result {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            eprintln!("raw_http error: {}", e);
            String::new()
        }
        Err(_) => {
            eprintln!("raw_http timeout on port {}", port);
            String::new()
        }
    }
}

fn response_status(resp: &str) -> u16 {
    resp.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn response_body(resp: &str) -> &str {
    resp.split("\r\n\r\n").nth(1).unwrap_or("")
}

/// Drive the JS event loop forever (for use in tokio::select! left branch).
/// Never returns — cancelled by tokio::select! when the right branch completes.
async fn drive_forever(e: &JsEngine) -> ! {
    loop {
        e.idle().await;
        tokio::task::yield_now().await;
    }
}

/// Drive the JS event loop until the client future completes.
async fn drive_until<T>(e: &JsEngine, client: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(client);
    tokio::select! {
        _ = drive_forever(e) => unreachable!("engine event loop terminated unexpectedly"),
        result = &mut client => result,
    }
}

/// Send a raw HTTP/1.1 request with an explicit `Content-Length` that may differ
/// from the actual body bytes sent.  Used to test server-side cap behaviour
/// without actually transmitting hundreds of megabytes.
///
/// The connection is closed after the body bytes are written, so if the server
/// tries to read more than `body.len()` bytes it will see an EOF error rather
/// than blocking indefinitely.
async fn raw_http_with_claimed_length(port: u16, claimed_content_length: usize, body: &str) {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap_or_else(|e| panic!("connect: {e}"));

    let req = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {claimed_content_length}\r\nConnection: close\r\n\r\n{body}"
    );
    let _ = stream.write_all(req.as_bytes()).await;
    // Intentionally drop — server sees EOF before claimed_content_length bytes arrive.
    drop(stream);
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn server_responds_200() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{
            res.writeHead(200, {{ 'Content-Type': 'text/plain' }});
            res.end('hello');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    // Give engine a moment to start the accept loop.
    wait_for_port(port).await;

    let resp = drive_until(&e, raw_http(port, "GET", "/", "")).await;

    assert_eq!(response_status(&resp), 200, "full response:\n{}", resp);
    assert_eq!(response_body(&resp), "hello");
}

#[tokio::test]
async fn server_reads_method_and_url() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__lastReq = '';
        var _server = http.createServer(function(req, res) {{
            globalThis.__lastReq = req.method + ' ' + req.url;
            res.end('ok');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    drive_until(&e, raw_http(port, "POST", "/test-path", "")).await;

    let result = e.eval_to_string("globalThis.__lastReq").await.unwrap();
    assert_eq!(result, "POST /test-path");
}

#[tokio::test]
async fn server_reads_request_body() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__lastBody = '';
        var _server = http.createServer(function(req, res) {{
            globalThis.__lastBody = req._body;
            res.end('received');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    drive_until(&e, raw_http(port, "POST", "/", "hello body")).await;

    let result = e.eval_to_string("globalThis.__lastBody").await.unwrap();
    assert_eq!(result, "hello body");
}

#[tokio::test]
async fn server_responds_with_custom_status() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{
            res.writeHead(404, {{ 'Content-Type': 'text/plain' }});
            res.end('not found');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    let resp = drive_until(&e, raw_http(port, "GET", "/missing", "")).await;

    assert_eq!(response_status(&resp), 404);
    assert_eq!(response_body(&resp), "not found");
}

#[tokio::test]
async fn server_handles_multiple_requests() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__reqCount = 0;
        var _server = http.createServer(function(req, res) {{
            globalThis.__reqCount++;
            res.end('req ' + globalThis.__reqCount);
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    for _ in 0..3u32 {
        drive_until(&e, raw_http(port, "GET", "/", "")).await;
    }

    let count = e
        .eval_to_string("String(globalThis.__reqCount)")
        .await
        .unwrap();
    assert_eq!(count, "3");
}

/// Verify that `req.headers['content-length']` in JS reflects the bytes that
/// were actually allocated and read — not the raw value from the request header.
///
/// Before the fix, the header string forwarded to JS was taken verbatim from the
/// incoming request even when the allocation was capped at 100 MiB.  After the
/// fix the forwarded value equals the effective (capped) `content_length`.
#[tokio::test]
async fn content_length_header_matches_allocated_body_bytes() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__clHeader = null;
        var _server = http.createServer(function(req, res) {{
            globalThis.__clHeader = req.headers['content-length'];
            res.end('ok');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    let body = "hello world";
    drive_until(&e, raw_http(port, "POST", "/", body)).await;

    // JS must see the exact Content-Length that was sent (no capping occurs here
    // since the body is well below the 100 MiB limit).
    let cl = e
        .eval_to_string("String(globalThis.__clHeader)")
        .await
        .unwrap();
    assert_eq!(cl, body.len().to_string());
}

/// Verify the server remains responsive after a client sends a wildly over-sized
/// Content-Length and then closes the connection without sending the body.
///
/// The 100 MiB allocation cap means the server tries to `read_exact` at most
/// 100 MiB; when the client closes early it gets an I/O error on that request
/// but must NOT crash or hang — subsequent requests must succeed.
#[tokio::test]
async fn server_survives_oversized_content_length_with_early_close() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__okCount = 0;
        var _server = http.createServer(function(req, res) {{
            globalThis.__okCount++;
            res.end('ok');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    // Send a request claiming 200 MiB but providing 0 bytes then closing —
    // the server should handle the EOF gracefully without panicking.
    let oversized = 200 * 1024 * 1024usize; // 200 MiB — beyond the 100 MiB cap.
    drive_until(&e, raw_http_with_claimed_length(port, oversized, "")).await;

    // Allow the server to process the (failed) request and reset.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A legitimate follow-up request must still succeed.
    let resp = drive_until(&e, raw_http(port, "GET", "/health", "")).await;
    assert_eq!(response_status(&resp), 200);
}

// ── Firewall tests ─────────────────────────────────────────────────────────────

/// Verify that every accepted request contains the `remoteAddress` of the client
/// in `req.socket.remoteAddress` (populated from the `remoteAddress` JSON field).
#[tokio::test]
async fn request_exposes_remote_address() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__remoteAddr = '';
        var _server = http.createServer(function(req, res) {{
            globalThis.__remoteAddr = req.socket.remoteAddress;
            res.end('ok');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;
    drive_until(&e, raw_http(port, "GET", "/", "")).await;

    let addr = e.eval_to_string("globalThis.__remoteAddr").await.unwrap();
    assert_eq!(addr, "127.0.0.1");
}

/// Verify that once a client exhausts its token-bucket burst, subsequent requests
/// receive HTTP 429 Too Many Requests without crashing the server.
///
/// Config: burst=2, rps=1. Requests 1-2 are allowed; request 3 is rate-limited.
#[tokio::test]
async fn firewall_rate_limits_after_burst_exhausted() {
    let port = free_port();
    let e = engine_with_firewall(FirewallConfig {
        rate_limit_rps: 1,
        rate_limit_burst: 2,
        auto_block_threshold: 100, // don't auto-block during this test
        ..FirewallConfig::default()
    })
    .await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('ok'); }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    // First two requests consume the burst — must succeed.
    let r1 = drive_until(&e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&r1), 200, "request 1 should be allowed");

    let r2 = drive_until(&e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&r2), 200, "request 2 should be allowed");

    // Third request with no time to refill → rate limited.
    let r3 = drive_until(&e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(
        response_status(&r3),
        429,
        "request 3 should be rate limited\nfull response:\n{}",
        r3
    );
}

/// Verify that after enough rate-limit violations the IP is auto-blocked and
/// subsequent connection attempts receive HTTP 403 Forbidden.
///
/// Config: burst=2, rps=1, threshold=3.
/// Requests 1-2 → 200; requests 3-4 → 429 (violations 1-2); request 5 → 403 (auto-blocked).
#[tokio::test]
async fn firewall_auto_blocks_after_threshold() {
    let port = free_port();
    let e = engine_with_firewall(FirewallConfig {
        rate_limit_rps: 1,
        rate_limit_burst: 2,
        auto_block_threshold: 3,
        block_duration_secs: 60,
        ..FirewallConfig::default()
    })
    .await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('ok'); }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    let mut statuses = Vec::new();
    for _ in 0..5 {
        let resp = drive_until(&e, raw_http(port, "GET", "/", "")).await;
        statuses.push(response_status(&resp));
    }

    assert_eq!(statuses[0], 200, "req 1 should be allowed");
    assert_eq!(statuses[1], 200, "req 2 should be allowed");
    assert_eq!(statuses[2], 429, "req 3 should be rate limited");
    assert_eq!(statuses[3], 429, "req 4 should be rate limited");
    assert_eq!(
        statuses[4], 403,
        "req 5 should be blocked (auto-blocked after threshold)"
    );
}

/// Verify the server drops a request that sends more headers than `max_header_count`
/// and continues accepting subsequent valid requests.
#[tokio::test]
async fn firewall_rejects_header_flood_and_continues() {
    let port = free_port();
    let e = engine_with_firewall(FirewallConfig {
        max_header_count: 5,
        ..FirewallConfig::default()
    })
    .await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__okCount = 0;
        var _server = http.createServer(function(req, res) {{
            globalThis.__okCount++;
            res.end('ok');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    // Send a request with 10 headers (exceeds limit of 5) — server should drop it.
    drive_until(&e, async move {
        if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            let mut req = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n".to_string();
            for i in 0..10 {
                req.push_str(&format!("X-Flood-{i}: value\r\n"));
            }
            req.push_str("\r\n");
            let _ = stream.write_all(req.as_bytes()).await;
            // Read whatever comes back (may be empty — server drops the connection).
            let mut buf = vec![0u8; 256];
            let _ = tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf)).await;
        }
    })
    .await;

    // Allow the server event loop to recover.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // A normal request must still succeed.
    let resp = drive_until(&e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(
        response_status(&resp),
        200,
        "server must accept requests after header flood"
    );

    let count = e
        .eval_to_string("String(globalThis.__okCount)")
        .await
        .unwrap();
    assert_eq!(count, "1", "only the valid request should have reached JS");
}

/// Verify that a slow connection (Slowloris: sends the request line then stalls)
/// is timed out and the server recovers to serve subsequent requests normally.
#[tokio::test]
async fn firewall_slowloris_timeout_and_recovery() {
    let port = free_port();
    let e = engine_with_firewall(FirewallConfig {
        header_timeout_ms: 300, // very tight: 300 ms
        ..FirewallConfig::default()
    })
    .await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__okCount = 0;
        var _server = http.createServer(function(req, res) {{
            globalThis.__okCount++;
            res.end('ok');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    // Simulate Slowloris: connect and send only the request line, then stall.
    // Never send the blank line that ends the headers, so the server's read_line
    // call will time out after header_timeout_ms.
    drive_until(&e, async move {
        if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", port)).await {
            // Send the request line but never the header-terminating \r\n.
            let _ = stream
                .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n")
                .await;
            // Hold the connection open past the timeout.
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await;

    // A normal request after the timeout must succeed.
    let resp = drive_until(&e, raw_http(port, "GET", "/ok", "")).await;
    assert_eq!(
        response_status(&resp),
        200,
        "server must recover after Slowloris timeout"
    );

    let count = e
        .eval_to_string("String(globalThis.__okCount)")
        .await
        .unwrap();
    assert_eq!(count, "1");
}

// ── keep-alive tests ─────────────────────────────────────────────────────────────

/// Read exactly one full HTTP/1.1 response from `stream` (headers + body, sized by
/// `Content-Length`) without consuming any bytes that belong to a subsequent
/// keep-alive request on the same connection.
async fn read_one_response(stream: &mut TcpStream) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut header_end: Option<usize> = None;
    loop {
        let n = stream.read(&mut tmp).await.expect("read response headers");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = Some(pos + 4);
            break;
        }
        assert!(buf.len() < (1 << 20), "response headers grew too large");
    }
    let header_end = match header_end {
        Some(h) => h,
        None => return String::from_utf8_lossy(&buf).to_string(),
    };
    let header_str = String::from_utf8_lossy(&buf[..header_end]);
    let content_length: usize = header_str
        .lines()
        .find_map(|l| {
            if l.to_ascii_lowercase().starts_with("content-length:") {
                l.split_once(':')?.1.trim().parse().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut tmp).await.expect("read response body");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    String::from_utf8_lossy(&buf[..header_end + content_length]).to_string()
}

/// Send a single request on an already-open `stream` and read the response.
/// `conn_header` lets the caller request keep-alive or close.
async fn request_on(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    body: &str,
    conn_header: &str,
) -> String {
    let req = if body.is_empty() {
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{conn_header}\r\n\r\n")
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {len}\r\n{conn_header}\r\n\r\n{body}",
            len = body.len(),
        )
    };
    stream
        .write_all(req.as_bytes())
        .await
        .expect("write request");
    stream.flush().await.expect("flush request");
    read_one_response(stream).await
}

/// Two requests over a single keep-alive connection must each get a correct
/// response, with the socket staying open in between.
#[tokio::test]
async fn keepalive_serves_two_requests_on_one_connection() {
    let port = free_port();
    let e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('resp ' + req.url); }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    let (r1, r2) = drive_until(&e, async move {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        let r1 = request_on(&mut stream, "GET", "/one", "", "Connection: keep-alive").await;
        let r2 = request_on(&mut stream, "GET", "/two", "", "Connection: keep-alive").await;
        let _ = stream.shutdown().await;
        (r1, r2)
    })
    .await;

    assert_eq!(response_status(&r1), 200, "full r1:\n{r1}");
    assert_eq!(response_body(&r1), "resp /one");
    assert_eq!(response_status(&r2), 200, "full r2:\n{r2}");
    assert_eq!(response_body(&r2), "resp /two");
}

/// When the client sends `Connection: close`, the server responds once and then
/// closes the socket (the next read returns EOF).
#[tokio::test]
async fn client_connection_close_closes_socket() {
    let port = free_port();
    let e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('bye'); }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    let (r1, n) = drive_until(&e, async move {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        let r1 = request_on(&mut stream, "GET", "/", "", "Connection: close").await;
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
        (r1, n)
    })
    .await;

    assert_eq!(response_status(&r1), 200, "full r1:\n{r1}");
    match n {
        Ok(Ok(0)) => {}
        other => panic!("expected EOF after Connection: close, got {:?}", other),
    }
}

/// A keep-alive connection left idle past `keepalive_timeout_ms` must be closed
/// silently by the server (Slowloris-on-idle mitigation), not held open forever.
#[tokio::test]
async fn idle_keepalive_connection_is_closed_by_server() {
    let port = free_port();
    let e = engine_with_firewall(FirewallConfig {
        keepalive_timeout_ms: 300,
        max_requests_per_conn: 1000,
        ..FirewallConfig::default()
    })
    .await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('ok'); }});
        _server.on('error', function() {{}});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    let (r1, n) = drive_until(&e, async move {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        let r1 = request_on(&mut stream, "GET", "/", "", "Connection: keep-alive").await;
        // Send no further requests; the server must drop the idle socket after
        // keepalive_timeout_ms (300 ms) rather than holding it open.
        tokio::time::sleep(Duration::from_millis(800)).await;
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
        (r1, n)
    })
    .await;

    assert_eq!(response_status(&r1), 200, "full r1:\n{r1}");
    match n {
        Ok(Ok(0)) => {}
        other => panic!(
            "expected server to close idle keep-alive connection, got {:?}",
            other
        ),
    }
}

/// A client that pipelines two requests in a single write: the server cannot carry
/// the coalesced second request across the keep-alive boundary, so it answers the
/// first and forces `Connection: close` (dropping the second), never hanging the
/// client. This is the safe fallback required because pipelined bytes would
/// otherwise be lost in `reader.into_inner()`.
#[tokio::test]
async fn pipelined_requests_force_connection_close() {
    let port = free_port();
    let e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('resp ' + req.url); }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    let (r1, n) = drive_until(&e, async move {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        let pipeline = "GET /one HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: keep-alive\r\n\r\n\
             GET /two HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: keep-alive\r\n\r\n"
            .to_string();
        stream.write_all(pipeline.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let r1 = read_one_response(&mut stream).await;
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
        (r1, n)
    })
    .await;

    assert_eq!(response_status(&r1), 200, "full r1:\n{r1}");
    assert_eq!(response_body(&r1), "resp /one");
    assert!(
        r1.contains("Connection: close"),
        "pipelined connection must be forced closed:\n{r1}"
    );
    match n {
        Ok(Ok(0)) => {}
        other => panic!(
            "expected EOF after forced close on pipelined request, got {:?}",
            other
        ),
    }
}

/// A server whose `'error'` handler re-throws must survive a keep-alive client
/// that disconnects after its first response without sending a second request.
/// Previously the EOF/read on the next accept surfaced `server.emit('error')`,
/// which this handler re-threw and crashed the server.
#[tokio::test]
async fn rethrowing_error_handler_survives_keepalive_client_close() {
    let port = free_port();
    let e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('ok'); }});
        _server.on('error', function(e) {{ throw e; }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    // One keep-alive request, read the response, then close the socket without a
    // second request.
    drive_until(&e, async move {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        let r1 = request_on(&mut stream, "GET", "/", "", "Connection: keep-alive").await;
        let _ = stream.shutdown().await;
        r1
    })
    .await;

    // Let the event loop process the now-closed connection.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The server must still accept new connections and respond normally.
    let resp = drive_until(&e, raw_http(port, "GET", "/after", "")).await;
    assert_eq!(
        response_status(&resp),
        200,
        "server must survive client close and keep serving:\n{resp}"
    );
}

/// A keep-alive connection left idle must NOT head-of-line block a brand-new
/// connection. With the old single-outstanding accept loop, `__httpAcceptAsync`
/// blocked up to `keepalive_timeout_ms` reading the idle socket before it could
/// serve the second connection. The new design parses every connection on its
/// own task and queues ready requests, so a new client is served immediately.
#[tokio::test]
async fn head_of_line_idle_keepalive_does_not_block_others() {
    let port = free_port();
    let e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('ok'); }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    let (resp, elapsed) = drive_until(&e, async move {
        // Connection A: one keep-alive request, then deliberately left idle
        // (no second request sent).
        let mut a = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        let _ = request_on(&mut a, "GET", "/a", "", "Connection: keep-alive").await;

        // Connection B: a fresh request that must be served promptly even though
        // A's socket is idle in keep-alive.
        let start = tokio::time::Instant::now();
        let mut b = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        let resp = request_on(&mut b, "GET", "/b", "", "Connection: close").await;
        (resp, start.elapsed())
    })
    .await;

    assert_eq!(response_status(&resp), 200, "full resp:\n{resp}");
    assert!(
        elapsed < Duration::from_millis(500),
        "B was blocked by idle A (head-of-line): took {elapsed:?}",
    );
}

/// 50 connections opened simultaneously, each issuing a single request, must
/// all get a 200 — and the whole batch must complete quickly. This exercises
/// the accept task + one-connection-task-per-socket model: connections are
/// parsed concurrently instead of being serialized behind one `__httpAcceptAsync`.
#[tokio::test]
async fn many_simultaneous_connections_all_served() {
    let port = free_port();
    let e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('ok'); }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    let (statuses, elapsed) = drive_until(&e, async move {
        let start = tokio::time::Instant::now();
        let mut handles = Vec::new();
        for _ in 0..50u32 {
            handles.push(tokio::spawn(async move {
                let mut s = TcpStream::connect(format!("127.0.0.1:{port}"))
                    .await
                    .unwrap();
                request_on(&mut s, "GET", "/", "", "Connection: close").await
            }));
        }
        let mut statuses = Vec::new();
        for h in handles {
            statuses.push(h.await.unwrap());
        }
        (statuses, start.elapsed())
    })
    .await;

    assert_eq!(statuses.len(), 50, "expected 50 responses");
    for s in &statuses {
        assert_eq!(response_status(s), 200, "full resp:\n{s}");
    }
    assert!(
        elapsed < Duration::from_secs(2),
        "wall time too high (serialized accept?): {elapsed:?}",
    );
}

/// After N keep-alive requests over a single connection that is then closed,
/// the firewall's active connection count must return to 0 — proving
/// `on_connect` and `on_disconnect` are each called exactly once per TCP
/// connection (no leak, no double-accounting) under the new task model.
#[tokio::test]
async fn firewall_connection_count_balances_to_zero() {
    let port = free_port();
    let (e, fw) = engine_with_firewall_arc(FirewallConfig {
        keepalive_timeout_ms: 5_000,
        max_requests_per_conn: 1_000,
        ..FirewallConfig::default()
    })
    .await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('ok'); }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    let n = 5u32;
    drive_until(&e, async move {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        for i in 0..n {
            // Keep-alive for all but the last request; close on the final one.
            let conn = if i + 1 < n {
                "Connection: keep-alive"
            } else {
                "Connection: close"
            };
            let resp = request_on(&mut stream, "GET", "/", "", conn).await;
            assert_eq!(response_status(&resp), 200, "full resp:\n{resp}");
        }
        let _ = stream.shutdown().await;
        n
    })
    .await;

    // Wait (briefly) for the server to tear the connection down.
    let mut waited = Duration::from_millis(0);
    while fw.active_connection_count() != 0 && waited < Duration::from_secs(2) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        waited += Duration::from_millis(50);
    }
    assert_eq!(
        fw.active_connection_count(),
        0,
        "connection leaked: firewall still tracks an open connection",
    );
}

/// Closing a server must stop its accept task so the listener is released: a
/// client connecting afterwards must NOT receive a 200 (it gets refused/EOF),
/// and the same port must be re-bindable without EADDRINUSE.
#[tokio::test]
async fn close_releases_listener_and_allows_relisten() {
    let port = free_port();
    let e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{ res.end('ok'); }});
        _server.listen({port}, '127.0.0.1');
        globalThis.__closeServer = function() {{ _server.close(); }};
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    // Close the server from JS.
    e.eval_to_string("globalThis.__closeServer()")
        .await
        .unwrap();
    // Give the accept task time to observe the shutdown and drop the listener.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A client connecting now must NOT be served. Either the OS refuses the
    // connection or the socket is gone (immediate EOF), never a 200 response.
    let outcome = drive_until(&e, async move {
        match TcpStream::connect(format!("127.0.0.1:{port}")).await {
            Ok(mut s) => {
                let mut buf = [0u8; 1];
                match tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf)).await {
                    Ok(Ok(0)) => "eof",  // socket closed, no response
                    Ok(Ok(_)) => "data", // unexpected response bytes
                    Ok(Err(_)) => "readerr",
                    Err(_) => "timeout",
                }
            }
            Err(_) => "refused",
        }
    })
    .await;
    assert!(
        outcome == "refused" || outcome == "eof",
        "expected connection refused/EOF after close, got {outcome}",
    );

    // Re-listening on the same port must succeed (no EADDRINUSE).
    let relisten = e
        .eval_to_string(&format!(
            r#"
        var http2 = require('http');
        var s2 = http2.createServer(function(req, res) {{ res.end('again'); }});
        s2.listen({port}, '127.0.0.1');
        'relistened'
        "#,
            port = port,
        ))
        .await;
    assert!(
        relisten.is_ok(),
        "re-listen on same port failed: {relisten:?}"
    );
}
