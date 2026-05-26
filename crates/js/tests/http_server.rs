// Tests for http.createServer() — real HTTP/1.1 listener.
//
// The port is bound synchronously (__httpListen) so it is ready immediately after
// eval_to_string returns.  The accept loop uses an async Promise (__httpAcceptAsync),
// so the engine event loop must run concurrently while the HTTP client runs.
//
// Pattern: start client via spawn_blocking (separate OS thread), drive the JS engine
// event loop via `loop { e.idle().await }` in a tokio::select! branch.
//
// Run: cargo test -p vvva_js --test http_server

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
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

fn raw_http(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut stream = match TcpStream::connect(format!("127.0.0.1:{}", port)) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

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
    stream.write_all(req.as_bytes()).ok();

    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok();
    resp
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

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn server_responds_200() {
    let port = free_port();
    let e = engine_with_net().await;

    // __httpListen is synchronous, so the port is bound before this returns.
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

    // Port is already bound — connect immediately.
    let http_task = tokio::task::spawn_blocking(move || raw_http(port, "GET", "/", ""));

    let resp = tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        r = http_task => r.unwrap(),
    };

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

    let http_task = tokio::task::spawn_blocking(move || raw_http(port, "POST", "/test-path", ""));

    tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        _ = http_task => {},
    }

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

    let http_task = tokio::task::spawn_blocking(move || raw_http(port, "POST", "/", "hello body"));

    tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        _ = http_task => {},
    }

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

    let http_task = tokio::task::spawn_blocking(move || raw_http(port, "GET", "/missing", ""));

    let resp = tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        r = http_task => r.unwrap(),
    };

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

    // Send three requests sequentially.  Each request is wrapped in a select! so
    // the engine drives while the blocking client runs.
    for _ in 0..3u32 {
        let http_task = tokio::task::spawn_blocking(move || raw_http(port, "GET", "/", ""));
        tokio::select! {
            _ = drive_forever(&e) => unreachable!(),
            _ = http_task => {},
        }
    }

    let count = e
        .eval_to_string("String(globalThis.__reqCount)")
        .await
        .unwrap();
    assert_eq!(count, "3");
}
