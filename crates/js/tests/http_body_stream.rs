// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Regression tests for streaming request bodies:
//
//   * a binary POST body reaches JS byte-for-byte and is emitted as a `Buffer`
//     (2.12.4 passed it to JS via `String::from_utf8_lossy`, corrupting every
//     non-UTF-8 byte into U+FFFD);
//   * a body is streamed in bounded chunks instead of being buffered whole in
//     Rust (a 300 MB upload used to panic at `V8String::new(...).unwrap()`);
//   * the deferred-delivery contract (2.12.3) still holds with streaming.
//
// Run: cargo test -p vvva_js --test http_body_stream

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

async fn wait_for_port(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("port {port} never became ready");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn drive_forever(e: &mut JsEngine) -> ! {
    loop {
        e.idle().await;
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
}

async fn drive_until<T>(e: &mut JsEngine, client: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(client);
    tokio::select! {
        _ = drive_forever(e) => unreachable!("engine event loop terminated unexpectedly"),
        result = &mut client => result,
    }
}

/// POST `body` (raw bytes) with an explicit Content-Length and return the
/// response body as bytes.
async fn post_bytes(port: u16, body: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("connect");
    let head = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut resp = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), stream.read_to_end(&mut resp))
        .await
        .expect("response timeout")
        .unwrap();
    resp
}

fn body_of(resp: &[u8]) -> &[u8] {
    let pos = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no header/body separator");
    &resp[pos + 4..]
}

#[tokio::test]
async fn binary_post_body_is_byte_identical_and_a_buffer() {
    let port = free_port();
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var crypto = require('crypto');
        globalThis.__res = null;
        var _server = http.createServer(function(req, res) {{
            var chunks = [];
            req.on('data', function(c) {{ chunks.push(c); }});
            req.on('end', function() {{
                var b = Buffer.concat(chunks);
                var allBuf = chunks.every(function(c) {{ return Buffer.isBuffer(c); }});
                globalThis.__res = allBuf + '|' + b.length + '|' + b.toString('hex');
                res.end('ok');
            }});
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;
    let payload: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x80, 0xc3];
    let resp = drive_until(&mut e, post_bytes(port, &payload)).await;
    assert_eq!(body_of(&resp), b"ok");

    let res = e.eval_to_string("globalThis.__res").await.unwrap();
    assert_eq!(res, "true|5|fffe0080c3");
}

#[tokio::test]
async fn binary_post_body_via_pipeline_to_file_is_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("body.bin");

    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    perms.grant(Capability::FileWrite(dir.path().to_path_buf()));
    let mut e = JsEngine::new(Arc::new(perms)).await.unwrap();

    let port = free_port();
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var fs = require('fs');
        var pipeline = require('stream/promises').pipeline;
        var _server = http.createServer(async function(req, res) {{
            await pipeline(req, fs.createWriteStream({out:?}));
            res.end('ok');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        out = out.to_string_lossy(),
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;
    // Mixed valid/invalid UTF-8 so a lossy String round-trip corrupts it.
    let payload: Vec<u8> = (0u16..=511).map(|i| (i % 256) as u8).collect();
    let resp = drive_until(&mut e, post_bytes(port, &payload)).await;
    assert_eq!(body_of(&resp), b"ok");
    // The pipeline future completes after the response; let it settle.
    for _ in 0..50 {
        e.idle().await;
        if out.exists() && std::fs::read(&out).map(|b| b.len()).unwrap_or(0) == payload.len() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let written = std::fs::read(&out).unwrap();
    assert_eq!(written, payload, "binary body corrupted by pipeline");
}

#[tokio::test]
async fn chunked_binary_body_streams_byte_identical() {
    let port = free_port();
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__res = null;
        var _server = http.createServer(function(req, res) {{
            var chunks = [];
            req.on('data', function(c) {{ chunks.push(c); }});
            req.on('end', function() {{
                var b = Buffer.concat(chunks);
                globalThis.__res = b.length + '|' + b.toString('hex');
                res.end('ok');
            }});
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;
    let a: Vec<u8> = vec![0x00, 0xff, 0x80, 0xc3, 0xfe];
    let b: Vec<u8> = vec![0xe2, 0x82, 0xac, 0x00, 0x01];
    let mut req = Vec::new();
    req.extend_from_slice(b"POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
    req.extend_from_slice(format!("{:x}\r\n", a.len()).as_bytes());
    req.extend_from_slice(&a);
    req.extend_from_slice(b"\r\n");
    req.extend_from_slice(format!("{:x}\r\n", b.len()).as_bytes());
    req.extend_from_slice(&b);
    req.extend_from_slice(b"\r\n0\r\n\r\n");

    let resp = drive_until(&mut e, async move {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        stream.write_all(&req).await.unwrap();
        let mut resp = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        resp
    })
    .await;
    assert_eq!(body_of(&resp), b"ok");
    let mut expected = a.clone();
    expected.extend_from_slice(&b);
    let res = e.eval_to_string("globalThis.__res").await.unwrap();
    assert_eq!(res, format!("{}|{}", expected.len(), hex(&expected)));
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 2.12.3 contract: the body is delivered only when consumed, and a consumed
/// request reports `readable: false` / `readableEnded: true` before `'end'`.
#[tokio::test]
async fn deferred_body_delivery_contract_survives_streaming() {
    let port = free_port();
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__state = null;
        var _server = http.createServer(function(req, res) {{
            // Attach listeners asynchronously, after awaiting, the way auth
            // middleware does before body-parser runs.
            setTimeout(function() {{
                var body = '';
                req.on('data', function(c) {{ body += c; }});
                req.on('end', function() {{
                    globalThis.__state = req.readable + '|' + req.readableEnded + '|' + req.complete + '|' + body;
                    res.end('ok');
                }});
            }}, 20);
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;
    let resp = drive_until(&mut e, post_bytes(port, b"late listener")).await;
    assert_eq!(body_of(&resp), b"ok");
    let state = e.eval_to_string("globalThis.__state").await.unwrap();
    assert_eq!(state, "false|true|true|late listener");
}
