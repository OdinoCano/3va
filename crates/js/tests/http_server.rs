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

/// Verify HTTP/1.1 keep-alive: multiple requests on a single TCP connection
/// are served without the client sending `Connection: close`. Each response is
/// delimited by Content-Length, and the connection stays open across requests.
#[tokio::test]
async fn server_keep_alive_multiple_requests_same_connection() {
    let port = free_port();
    let mut e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__reqCount = 0;
        var _server = http.createServer(function(req, res) {{
            globalThis.__reqCount++;
            res.writeHead(200, {{ 'Content-Type': 'text/plain' }});
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

    // One connection, three sequential requests, no Connection: close.
    let result = drive_until(&mut e, async {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        let mut responses = Vec::new();
        for _ in 0..3u32 {
            let req = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
            stream.write_all(req.as_bytes()).await.unwrap();
            let mut resp = String::new();
            let mut buf = vec![0u8; 4096];
            // Read exactly one response (until the header/body separator is seen
            // and Content-Length bytes have arrived) without waiting for EOF.
            let n = stream.read(&mut buf).await.unwrap();
            resp.push_str(&String::from_utf8_lossy(&buf[..n]));
            responses.push(resp);
        }
        responses
    })
    .await;

    let responses = result;
    assert_eq!(responses.len(), 3);
    for (i, resp) in responses.iter().enumerate() {
        assert_eq!(
            response_status(resp),
            200,
            "response {} should be 200: {:?}",
            i + 1,
            resp
        );
        assert_eq!(response_body(resp), format!("req {}", i + 1));
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

    // A legitimate follow-up request must still succeed. On a heavily loaded
    // machine a single attempt can starve past raw_http's internal timeout even
    // though the server is healthy, so retry before declaring it wedged — a
    // real hang fails every attempt.
    let mut last = String::new();
    for attempt in 0..5 {
        last = drive_until(&mut e, raw_http(port, "GET", "/health", "")).await;
        if response_status(&last) == 200 {
            break;
        }
        eprintln!(
            "recovery attempt {attempt} got status {}",
            response_status(&last)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        response_status(&last),
        200,
        "server must recover after oversized Content-Length with early close\nfull response:\n{}",
        last
    );
}

// ── Header injection (CRLF) rejection ──────────────────────────────────────────

fn standalone_res_js() -> &'static str {
    "var http = require('http'); var res = new http.ServerResponse(new http.IncomingMessage({}));"
}

/// res.setHeader('X-Foo', 'bar\r\nX-Evil: 1') must throw TypeError
/// ERR_INVALID_CHAR instead of letting CR/LF into the response buffer.
#[tokio::test]
async fn set_header_rejects_crlf_injection() {
    let mut e = engine_with_net().await;
    let r = e
        .eval_to_string(&format!(
            r#"
            {setup}
            try {{
                res.setHeader('X-Foo', 'bar\r\nX-Evil: 1');
                'no-throw';
            }} catch (err) {{
                (err instanceof TypeError) + ':' + err.code;
            }}
            "#,
            setup = standalone_res_js()
        ))
        .await
        .unwrap();
    assert_eq!(
        r, "true:ERR_INVALID_CHAR",
        "expected ERR_INVALID_CHAR TypeError"
    );
}

#[tokio::test]
async fn set_header_rejects_bare_cr_lf_and_del() {
    let mut e = engine_with_net().await;
    for evil in ["a\rb", "a\nb", "a\u{0}b", "a\u{7F}b"] {
        let r = e
            .eval_to_string(&format!(
                r#"
                {setup}
                try {{
                    res.setHeader('X-Foo', {evil:?});
                    'no-throw';
                }} catch (err) {{
                    err.code;
                }}
                "#,
                setup = standalone_res_js(),
                evil = evil
            ))
            .await
            .unwrap();
        assert_eq!(r, "ERR_INVALID_CHAR", "value {evil:?} must be rejected");
    }
}

/// Invalid header names (non-token chars incl. CR/LF) are rejected like Node's
/// ERR_INVALID_HTTP_TOKEN.
#[tokio::test]
async fn set_header_rejects_invalid_header_name() {
    let mut e = engine_with_net().await;
    for bad_name in ["X Foo", "X\rFoo", "X\nFoo", "", "X(Foo)"] {
        let r = e
            .eval_to_string(&format!(
                r#"
                {setup}
                try {{
                    res.setHeader({bad:?}, 'value');
                    'no-throw';
                }} catch (err) {{
                    (err instanceof TypeError) + ':' + err.code;
                }}
                "#,
                setup = standalone_res_js(),
                bad = bad_name
            ))
            .await
            .unwrap();
        assert_eq!(
            r, "true:ERR_INVALID_HTTP_TOKEN",
            "name {bad_name:?} must be rejected"
        );
    }
}

/// writeHead(200, {{...}}) with an injected header must throw before any state
/// is recorded.
#[tokio::test]
async fn write_head_rejects_crlf_in_headers() {
    let mut e = engine_with_net().await;
    let r = e
        .eval_to_string(&format!(
            r#"
            {setup}
            try {{
                res.writeHead(200, {{ 'X-Foo': 'bar\r\nSet-Cookie: pwned=1' }});
                'no-throw';
            }} catch (err) {{
                err.code + ':' + res.headersSent + ':' + res.statusCode;
            }}
            "#,
            setup = standalone_res_js()
        ))
        .await
        .unwrap();
    // Validation happens before writeHead records anything.
    assert_eq!(r, "ERR_INVALID_CHAR:false:200");
}

/// Valid headers keep working through both setHeader and writeHead.
#[tokio::test]
async fn valid_headers_still_accepted() {
    let mut e = engine_with_net().await;
    let r = e
        .eval_to_string(&format!(
            r#"
            {setup}
            res.setHeader('X-Foo', 'bar');
            var viaWriteHead = true;
            try {{
                res.writeHead(200, {{ 'Content-Type': 'text/plain', 'X-Token': "!#$%&'*+.^_`|~az09-" }});
            }} catch (err) {{ viaWriteHead = err.code; }}
            JSON.stringify([res.getHeader('X-Foo'), viaWriteHead, res.statusMessage]);
            "#,
            setup = standalone_res_js()
        ))
        .await
        .unwrap();
    assert_eq!(r, r#"["bar",true,"OK"]"#);
}

/// Last-line-of-defense: headers smuggled into `_headers` without going through
/// setHeader/writeHead are rejected by the native response writer when the
/// response is flushed.
#[tokio::test]
async fn smuggled_header_caught_by_native_writer() {
    let mut e = engine_with_net().await;
    let r = e
        .eval_to_string(&format!(
            r#"
            {setup}
            res._headers['X-Evil'] = 'a\r\nb';
            try {{
                res.end('x');
                'no-throw';
            }} catch (err) {{
                err.code;
            }}
            "#,
            setup = standalone_res_js()
        ))
        .await
        .unwrap();
    assert_eq!(r, "ERR_INVALID_CHAR");
}

// ── Transfer-Encoding: chunked / request smuggling ────────────────────────────

/// Send a fully raw request string (no automatic framing) and return the
/// complete response.
async fn raw_raw(port: u16, req: &str) -> String {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap_or_else(|e| panic!("connect to port {}: {}", port, e));
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
            eprintln!("raw_raw error: {}", e);
            String::new()
        }
        Err(_) => {
            eprintln!("raw_raw timeout on port {}", port);
            String::new()
        }
    }
}

/// A standard HTTP/1.1 chunked request body must be decoded exactly like the
/// equivalent Content-Length-framed one.
#[tokio::test]
async fn chunked_body_decoded_same_as_content_length() {
    let port = free_port();
    let mut e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__lastBody = '';
        var _server = http.createServer(function(req, res) {{
            globalThis.__lastBody = req._body;
            res.end('len:' + req._body.length);
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    // Chunked: "hello" in two chunks + trailers discarded.
    let chunked_req = "POST /upload HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\n\
                       Connection: close\r\n\r\n3\r\nhel\r\n8\r\nlo world\r\n0\r\nX-Ignore: me\r\n\r\n";
    let resp = drive_until(&mut e, raw_raw(port, chunked_req)).await;
    assert_eq!(response_status(&resp), 200, "full response:\n{}", resp);
    assert_eq!(response_body(&resp), "len:11");

    // Same payload via Content-Length must behave identically.
    let cl_resp = drive_until(&mut e, raw_http(port, "POST", "/upload", "hello world")).await;
    assert_eq!(response_status(&cl_resp), 200);
    assert_eq!(response_body(&cl_resp), "len:11");

    let body = e.eval_to_string("globalThis.__lastBody").await.unwrap();
    assert_eq!(body, "hello world");
}

/// The classic request-smuggling vector: a request carrying BOTH
/// Content-Length and Transfer-Encoding must be rejected with 400.
#[tokio::test]
async fn content_length_plus_transfer_encoding_rejected_400() {
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

    let smuggle = "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 6\r\n\
         Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n0\r\n\r\nGET /\r\n";
    let resp = drive_until(&mut e, raw_raw(port, smuggle)).await;
    assert_eq!(
        response_status(&resp),
        400,
        "CL+TE smuggling attempt must get 400\nfull response:\n{}",
        resp
    );

    // The poisoned request must never have reached JS…
    tokio::time::sleep(Duration::from_millis(150)).await;
    let count = e
        .eval_to_string("String(globalThis.__okCount)")
        .await
        .unwrap();
    assert_eq!(count, "0");

    // …and the server must keep serving clean requests.
    let ok = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&ok), 200);
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

/// Poll `globalThis.__remoteAddr`, driving the event loop while waiting,
/// until the request handler has populated it.
async fn wait_remote_addr(e: &mut JsEngine) -> String {
    for _ in 0..250 {
        let a = e.eval_to_string("globalThis.__remoteAddr").await.unwrap();
        if !a.is_empty() {
            return a;
        }
        drive_until(e, tokio::time::sleep(Duration::from_millis(20))).await;
    }
    panic!("remoteAddress was never set");
}

/// When the direct peer is a trusted proxy, `remoteAddress` reports the
/// client IP from `X-Forwarded-For` and rate-limit accounting uses it.
#[tokio::test]
async fn trusted_proxy_forwards_client_ip_to_remote_address() {
    let port = free_port();
    // The test client connects from 127.0.0.1 — declare it a trusted proxy.
    let mut e = engine_with_firewall(FirewallConfig {
        trusted_proxies: vec!["127.0.0.1".to_string()],
        ..FirewallConfig::default()
    })
    .await;

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

    // Proxy-style request carrying XFF.
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    let req = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Forwarded-For: 203.0.113.9\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();

    let addr = wait_remote_addr(&mut e).await;
    assert_eq!(
        addr, "203.0.113.9",
        "trusted-proxy XFF must become remoteAddress"
    );

    // Same server, request WITHOUT XFF → falls back to the peer address.
    drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    e.eval_to_string("globalThis.__remoteAddr = ''")
        .await
        .unwrap();
    drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    let addr2 = wait_remote_addr(&mut e).await;
    assert_eq!(addr2, "127.0.0.1");
}

/// A direct (untrusted) client cannot spoof `remoteAddress` by sending its
/// own X-Forwarded-For header when no proxies are trusted.
#[tokio::test]
async fn untrusted_xff_header_is_ignored() {
    let port = free_port();
    let mut e = engine_with_firewall(FirewallConfig::default()).await;

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

    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    let req = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Forwarded-For: 6.6.6.6\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();

    let addr = wait_remote_addr(&mut e).await;
    assert_eq!(
        addr, "127.0.0.1",
        "spoofed XFF from an untrusted peer must not change remoteAddress"
    );
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

/// Verify that a RUDY attack — a POST whose body arrives at ~1 byte/second —
/// is dropped by the minimum body-rate check and the server recovers to serve
/// subsequent requests normally.
///
/// The body deadline is raised to 120 s so the total-deadline path CANNOT fire
/// within this test: only `min_body_rate_bps` can reject the connection, which
/// is the point being proven.
#[tokio::test]
async fn firewall_rudy_slow_body_rejected_and_recovers() {
    let port = free_port();
    let mut e = engine_with_firewall(FirewallConfig {
        min_body_rate_bps: 50, // slower than 50 B/s is dropped
        body_timeout_ms: 120_000,
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

    // RUDY simulation: declare Content-Length: 200 but drip the body at ~1 B/s
    // (one byte every 500 ms). A complete body would take 100 s; the rate check
    // must drop the connection after ~2-3 s instead.
    let bytes_sent = drive_until(&mut e, async move {
        let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", port)).await else {
            return 0u32;
        };
        let req =
            "POST /slow HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 200\r\nConnection: close\r\n\r\n"
                .to_string();
        let _ = stream.write_all(req.as_bytes()).await;

        let mut sent = 0u32;
        let mut buf = [0u8; 16];
        for _ in 0..60 {
            let _ = stream.write_all(b"x").await;
            sent += 1;
            tokio::time::sleep(Duration::from_millis(500)).await;
            // If the server dropped us, the read returns EOF.
            match tokio::time::timeout(Duration::from_millis(50), stream.read(&mut buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return sent,
                _ => {}
            }
        }
        sent
    })
    .await;

    // A full body is 200 bytes; a rate-check drop happens after ~5 bytes. If we
    // got here with bytes_sent >= 30 the server waited 15+ s (still far below
    // the 120 s deadline), so neither mechanism dropped it — fail.
    assert!(
        bytes_sent < 30,
        "server must drop the slow body quickly (sent {bytes_sent}/200 bytes, \
         deadline is 120 s so only the rate check can have rejected it)"
    );

    // The incomplete RUDY body must never have reached JS.
    let count = e
        .eval_to_string("String(globalThis.__okCount)")
        .await
        .unwrap();
    assert_eq!(count, "0", "the RUDY request must not reach the handler");

    // A normal request after the attack must succeed.
    let resp = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(
        response_status(&resp),
        200,
        "server must recover after RUDY drop"
    );

    let count = e
        .eval_to_string("String(globalThis.__okCount)")
        .await
        .unwrap();
    assert_eq!(count, "1");
}

/// Verify adaptive rate limiting end-to-end: a repeat offender's block
/// duration escalates. First auto-block lasts `block_duration_secs`; after the
/// block expires the IP re-offends and the second block lasts
/// `block_duration_secs × blockEscalationFactor` (longer), then the IP is
/// served again only once that escalated block has fully elapsed.
#[tokio::test]
async fn firewall_adaptive_escalation_repeat_offender() {
    let port = free_port();
    let mut e = engine_with_firewall(FirewallConfig {
        rate_limit_rps: 100,
        rate_limit_burst: 1,
        auto_block_threshold: 1,
        block_duration_secs: 1,
        block_escalation_factor: 2,
        max_block_duration_secs: 4,
        strike_decay_secs: 3600, // keep the strike history across this test
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

    // Round 1: first request allowed, second auto-blocks at the base 1 s.
    let r1 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&r1), 200, "req 1 must be allowed");
    let r2 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&r2), 403, "req 2 must auto-block the IP");
    let r3 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&r3), 403, "req 3 must stay blocked");

    // Wait past the 1 s block.
    tokio::time::sleep(Duration::from_millis(1300)).await;

    // Round 2: allowed again, then re-offends → strike 2 → 2 s block.
    let r4 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(
        response_status(&r4),
        200,
        "req 4 must be allowed after the block"
    );
    let r5 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(response_status(&r5), 403, "req 5 must re-block the IP");

    // 1.5 s into the 2 s block it must STILL be active — the escalated block is
    // longer than the first one, which had already expired at 1.3 s.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let r6 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(
        response_status(&r6),
        403,
        "req 6 at 1.5 s into a 2 s block must still be blocked (escalated duration)"
    );

    // After the full 2 s elapses the IP is served again.
    tokio::time::sleep(Duration::from_millis(900)).await;
    let r7 = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    assert_eq!(
        response_status(&r7),
        200,
        "req 7 must be allowed after the escalated block"
    );
}
