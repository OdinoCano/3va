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
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
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
    tokio::time::sleep(Duration::from_millis(100)).await;

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

    tokio::time::sleep(Duration::from_millis(100)).await;

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

    tokio::time::sleep(Duration::from_millis(100)).await;

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

    tokio::time::sleep(Duration::from_millis(100)).await;

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

    tokio::time::sleep(Duration::from_millis(100)).await;

    for _ in 0..3u32 {
        drive_until(&e, raw_http(port, "GET", "/", "")).await;
    }

    let count = e
        .eval_to_string("String(globalThis.__reqCount)")
        .await
        .unwrap();
    assert_eq!(count, "3");
}
