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
async fn drive_forever(e: &mut JsEngine) -> ! {
    loop {
        e.idle().await;
        // Timers (setInterval, used by http's __httpAcceptPoll loop) only
        // fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
}

/// Drive the JS event loop until the client future completes.
async fn drive_until<T>(e: &mut JsEngine, client: impl std::future::Future<Output = T>) -> T {
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
    let mut e = engine_with_net().await;

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

    let resp = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;

    assert_eq!(response_status(&resp), 200, "full response:\n{}", resp);
    assert_eq!(response_body(&resp), "hello");
}

#[tokio::test]
async fn server_reads_method_and_url() {
    let port = free_port();
    let mut e = engine_with_net().await;

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

    drive_until(&mut e, raw_http(port, "POST", "/test-path", "")).await;

    let result = e.eval_to_string("globalThis.__lastReq").await.unwrap();
    assert_eq!(result, "POST /test-path");
}

#[tokio::test]
async fn server_reads_request_body() {
    let port = free_port();
    let mut e = engine_with_net().await;

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

    drive_until(&mut e, raw_http(port, "POST", "/", "hello body")).await;

    let result = e.eval_to_string("globalThis.__lastBody").await.unwrap();
    assert_eq!(result, "hello body");
}

#[tokio::test]
async fn server_responds_with_custom_status() {
    let port = free_port();
    let mut e = engine_with_net().await;

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

    let resp = drive_until(&mut e, raw_http(port, "GET", "/missing", "")).await;

    assert_eq!(response_status(&resp), 404);
    assert_eq!(response_body(&resp), "not found");
}

#[tokio::test]
async fn server_handles_multiple_requests() {
    let port = free_port();
    let mut e = engine_with_net().await;

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
        drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
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
    let mut e = engine_with_net().await;

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
    drive_until(&mut e, raw_http(port, "POST", "/", body)).await;

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
    let mut e = engine_with_net().await;

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
    drive_until(&mut e, raw_http_with_claimed_length(port, oversized, "")).await;

    // Allow the server to process the (failed) request and reset.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A legitimate follow-up request must still succeed.
    let resp = drive_until(&mut e, raw_http(port, "GET", "/health", "")).await;
    assert_eq!(response_status(&resp), 200);
}

// ── Firewall tests ─────────────────────────────────────────────────────────────

/// Verify that every accepted request contains the `remoteAddress` of the client
/// in `req.socket.remoteAddress` (populated from the `remoteAddress` JSON field).
#[tokio::test]
async fn request_exposes_remote_address() {
    let port = free_port();
    let mut e = engine_with_net().await;

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
    drive_until(&mut e, raw_http(port, "GET", "/", "")).await;

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
    let mut e = engine_with_firewall(FirewallConfig {
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
    let r1 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&r1), 200, "request 1 should be allowed");

    let r2 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&r2), 200, "request 2 should be allowed");

    // Third request with no time to refill → rate limited.
    let r3 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
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
    let mut e = engine_with_firewall(FirewallConfig {
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
        let resp = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
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
    let mut e = engine_with_firewall(FirewallConfig {
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
    drive_until(&mut e, async move {
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
    let resp = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
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
    let mut e = engine_with_firewall(FirewallConfig {
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
    drive_until(&mut e, async move {
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
    let resp = drive_until(&mut e, raw_http(port, "GET", "/ok", "")).await;
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
