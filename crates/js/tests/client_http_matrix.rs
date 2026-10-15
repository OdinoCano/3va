// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Regression tests for the *client* side of `http`/`https`, `fetch`, `net` and
// `stream`, exercised with the JS client in the engine and a real server on a
// separate OS thread — the same shape as production (server and client in
// different processes), which is where the 2.12.5 gaps were invisible.
//
// Each test fails without the matching fix:
//   - http.request sent no body framing (no Content-Length / chunked), so a
//     Node server never read the body;
//   - fetch serialised the body with String(), corrupting binary bodies;
//   - net.Socket.end() destroyed the connection instead of half-closing it, so
//     a response written after the client's FIN was lost;
//   - Writable/Transform.end(chunk) marked the stream ended before writing the
//     chunk, so PassThrough.end(buf) and socket.end(data) dropped it;
//   - 'drain' was emitted synchronously inside write();
//   - pipeline() did not propagate a mid-stream error;
//   - finished() invoked its callback twice on a Duplex;
//   - process lacked Symbol.toStringTag, so axios used its fetch adapter.
//
// Run: cargo test -p vvva_js --test client_http_matrix

use std::io::{Read, Write};
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

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decode an HTTP/1.1 chunked body.
fn dechunk(mut buf: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(nl) = find(buf, b"\r\n") {
        let line = String::from_utf8_lossy(&buf[..nl]);
        let size_str = line.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_str, 16) else {
            break;
        };
        buf = &buf[nl + 2..];
        if size == 0 {
            break;
        }
        if buf.len() < size {
            break;
        }
        out.extend_from_slice(&buf[..size]);
        buf = &buf[size + 2..];
    }
    out
}

/// Read one request from `sock`, honouring Content-Length or chunked framing,
/// and return the body bytes.
fn read_request_body(sock: &mut std::net::TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        let n = sock.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            return Vec::new();
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
    let mut body = buf[header_end + 4..].to_vec();
    if let Some(cl) = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        while body.len() < cl {
            let n = sock.read(&mut tmp).unwrap_or(0);
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(cl);
    } else if head.contains("transfer-encoding: chunked") {
        loop {
            let decoded_done = find(&body, b"0\r\n\r\n").is_some()
                || body.ends_with(b"0\r\n\r\n")
                || (find(&body, b"0\r\n").is_some() && body.len() >= 3);
            if decoded_done {
                break;
            }
            let n = sock.read(&mut tmp).unwrap_or(0);
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body = dechunk(&body);
    }
    body
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Serve `count` HTTP requests, echoing the request body as hex. When
/// `chunked_response` is set the reply has no Content-Length and is sent in two
/// writes (chunked by Node's server rules).
fn http_echo_server(port: u16, count: usize, chunked_response: bool) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        for _ in 0..count {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let body = read_request_body(&mut sock);
            let hex = to_hex(&body);
            let resp = if chunked_response {
                format!(
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                    hex.len(),
                    hex
                )
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    hex.len(),
                    hex
                )
            };
            let _ = sock.write_all(resp.as_bytes());
            let _ = sock.flush();
        }
    });
}

/// A net server that reads until EOF and then answers `reply` on the same
/// (now read-closed) socket — the classic half-close echo.
fn net_eof_reply_server(port: u16, reply: &'static [u8]) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        if let Ok((mut sock, _)) = listener.accept() {
            let mut tmp = [0u8; 4096];
            while sock.read(&mut tmp).map(|n| n > 0).unwrap_or(false) {}
            let _ = sock.write_all(reply);
            let _ = sock.flush();
        }
    });
}

/// Drive the JS event loop until `globalThis.__done === true` or the deadline.
async fn wait_done(e: &mut JsEngine, secs: u64) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if e.eval_to_string("String(globalThis.__done === true)")
            .await
            .unwrap_or_default()
            == "true"
        {
            return e
                .eval_to_string("String(globalThis.__result)")
                .await
                .unwrap_or_default();
        }
        if tokio::time::Instant::now() >= deadline {
            let err = e
                .eval_to_string("String(globalThis.__err)")
                .await
                .unwrap_or_default();
            panic!("JS never finished; __err={err:?}");
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_request_post_string_is_framed_and_sent() {
    let port = free_port();
    http_echo_server(port, 1, false);
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var req = http.request({{ host: '127.0.0.1', port: {port}, method: 'POST', path: '/' }}, function(res) {{
            var d = ''; res.on('data', function(c) {{ d += c; }});
            res.on('end', function() {{ globalThis.__result = d; globalThis.__done = true; }});
        }});
        req.on('error', function(e) {{ globalThis.__err = e.message; globalThis.__done = true; }});
        req.end('hi');
        "#
    ))
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 10).await, "6869");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_request_post_buffer_binary_is_framed_and_sent() {
    let port = free_port();
    http_echo_server(port, 1, false);
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var req = http.request({{ host: '127.0.0.1', port: {port}, method: 'POST', path: '/' }}, function(res) {{
            var d = ''; res.on('data', function(c) {{ d += c; }});
            res.on('end', function() {{ globalThis.__result = d; globalThis.__done = true; }});
        }});
        req.on('error', function(e) {{ globalThis.__err = e.message; globalThis.__done = true; }});
        req.end(Buffer.from([0, 255, 104, 105, 200]));
        "#
    ))
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 10).await, "00ff6869c8");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_request_streaming_writes_use_chunked() {
    let port = free_port();
    http_echo_server(port, 1, false);
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var req = http.request({{ host: '127.0.0.1', port: {port}, method: 'POST', path: '/' }}, function(res) {{
            var d = ''; res.on('data', function(c) {{ d += c; }});
            res.on('end', function() {{ globalThis.__result = d; globalThis.__done = true; }});
        }});
        req.on('error', function(e) {{ globalThis.__err = e.message; globalThis.__done = true; }});
        req.write('ab');
        setTimeout(function() {{ req.write(Buffer.from([0, 255])); req.end('cd'); }}, 20);
        "#
    ))
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 10).await, "616200ff6364");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_client_dechunks_response() {
    let port = free_port();
    http_echo_server(port, 1, true);
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var req = http.request({{ host: '127.0.0.1', port: {port}, method: 'POST', path: '/' }}, function(res) {{
            var d = ''; res.on('data', function(c) {{ d += c; }});
            res.on('end', function() {{ globalThis.__result = d; globalThis.__done = true; }});
        }});
        req.on('error', function(e) {{ globalThis.__err = e.message; globalThis.__done = true; }});
        req.end('hi');
        "#
    ))
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 10).await, "6869");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_buffer_body_keeps_bytes() {
    let port = free_port();
    http_echo_server(port, 1, false);
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        fetch('http://127.0.0.1:{port}/', {{ method: 'POST', body: Buffer.from([0, 255, 104, 105, 200]) }})
            .then(function(r) {{ return r.text(); }})
            .then(function(t) {{ globalThis.__result = t; globalThis.__done = true; }})
            .catch(function(e) {{ globalThis.__err = String(e && e.message || e); globalThis.__done = true; }});
        "#
    ))
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 10).await, "00ff6869c8");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_uint8array_body_keeps_bytes() {
    let port = free_port();
    http_echo_server(port, 1, false);
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        fetch('http://127.0.0.1:{port}/', {{ method: 'POST', body: new Uint8Array([0, 255, 104, 105, 200]) }})
            .then(function(r) {{ return r.text(); }})
            .then(function(t) {{ globalThis.__result = t; globalThis.__done = true; }})
            .catch(function(e) {{ globalThis.__err = String(e && e.message || e); globalThis.__done = true; }});
        "#
    ))
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 10).await, "00ff6869c8");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn net_end_data_half_closes_and_reads_reply() {
    let port = free_port();
    net_eof_reply_server(port, &[0, 255, 104, 105, 200]);
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var net = require('net');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var c = net.connect({port}, '127.0.0.1', function() {{
            c.end(Buffer.from([0, 255, 104, 105, 200]));
        }});
        var d = '';
        c.on('data', function(x) {{ d += Buffer.from(x).toString('hex'); }});
        c.on('end', function() {{ globalThis.__result = d; globalThis.__done = true; }});
        c.on('error', function(e) {{ globalThis.__err = e.message; globalThis.__done = true; }});
        "#
    ))
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 10).await, "00ff6869c8");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn net_delayed_end_still_sends_body_and_reads_reply() {
    let port = free_port();
    net_eof_reply_server(port, b"36383639");
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var net = require('net');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var c = net.connect({port}, '127.0.0.1', function() {{
            c.write('hi');
            setTimeout(function() {{ c.end(); }}, 60);
        }});
        var d = '';
        c.on('data', function(x) {{ d += x; }});
        c.on('end', function() {{ globalThis.__result = d; globalThis.__done = true; }});
        c.on('error', function(e) {{ globalThis.__err = e.message; globalThis.__done = true; }});
        "#
    ))
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 10).await, "36383639");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn passthrough_end_chunk_flows() {
    let mut e = engine_with_net().await;
    e.eval_to_string(
        r#"
        var stream = require('stream');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var p = new stream.PassThrough();
        var got = '';
        p.on('data', function(d) {{ got += Buffer.from(d).toString('hex'); }});
        p.on('end', function() {{ globalThis.__result = got; globalThis.__done = true; }});
        p.end(Buffer.from([0, 255, 104, 105, 200]));
        "#,
    )
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 5).await, "00ff6869c8");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writable_emits_drain_after_sync_write() {
    let mut e = engine_with_net().await;
    e.eval_to_string(
        r#"
        var stream = require('stream');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var w = new stream.Writable({ highWaterMark: 4, write: function(c, enc, cb) {{ cb(); }} });
        var ret = w.write('0123456789');
        var drained = false;
        w.once('drain', function() {{ drained = true; }});
        setTimeout(function() {{ globalThis.__result = 'ret=' + ret + ' drain=' + drained; globalThis.__done = true; }}, 50);
        "#,
    )
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 5).await, "ret=false drain=true");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_propagates_midstream_error() {
    let mut e = engine_with_net().await;
    e.eval_to_string(
        r#"
        var stream = require('stream');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var bad = new stream.Transform({ transform: function(c, enc, cb) {{ cb(new Error('boom')); }} });
        stream.pipeline(stream.Readable.from(['a']), bad, new stream.Transform({ transform: function(c, e, cb) {{ cb(null, c); }} }), function(err) {
            globalThis.__result = err ? err.message : 'none';
            globalThis.__done = true;
        });
        "#,
    )
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 5).await, "boom");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finished_called_once_on_duplex() {
    let mut e = engine_with_net().await;
    e.eval_to_string(
        r#"
        var stream = require('stream');
        globalThis.__done = false; globalThis.__result = null; globalThis.__err = null;
        var p = new stream.PassThrough();
        var calls = 0;
        stream.finished(p, function() { calls++; });
        p.on('data', function() {});
        p.end('x');
        setTimeout(function() {{ globalThis.__result = 'calls=' + calls; globalThis.__done = true; }}, 50);
        "#,
    )
    .await
    .unwrap();
    assert_eq!(wait_done(&mut e, 5).await, "calls=1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_has_to_string_tag() {
    let mut e = engine_with_net().await;
    let v = e
        .eval_to_string("Object.prototype.toString.call(process)")
        .await
        .unwrap();
    assert_eq!(v, "[object process]");
}
