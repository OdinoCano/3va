// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Tests for the http server's `upgrade` event (WebSocket). The JS server
// implements a minimal WebSocket handshake + frame echo over the raw duplex
// socket 3va hands to `server.on('upgrade')`; a real tungstenite client (the
// same one 3va's own `websocket` builtin uses) drives the other end. This
// proves the handoff is a usable duplex socket, not just an event.
// Run: cargo test -p vvva_js --test http_upgrade_ws

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net(host: &str) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::Network(host.to_string()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

// Minimal WS echo server: performs the 101 handshake, then echoes every
// text/binary frame as "echo:<payload>". Written against the raw socket the
// http layer hands to `upgrade` — the same surface the `ws` npm library uses.
const ECHO_SERVER_JS: &str = r#"
    const http = require('http');
    const crypto = require('crypto');
    const server = http.createServer(function(req, res) { res.end('http-ok'); });
    server.on('upgrade', function(req, socket, head) {
        var key = req.headers['sec-websocket-key'];
        var accept = crypto.createHash('sha1')
            .update(String(key) + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11')
            .digest('base64');
        socket.write('HTTP/1.1 101 Switching Protocols\r\n' +
            'Upgrade: websocket\r\nConnection: Upgrade\r\n' +
            'Sec-WebSocket-Accept: ' + accept + '\r\n\r\n');
        var buf = Buffer.from(head && head.length ? head : []);
        // Frames can already be in `head` (a client may send its first frame
        // together with the handshake): parse what is buffered now, then on
        // every chunk — exactly what the ws library does with `head`.
        function pump() {
            while (buf.length >= 2) {
                var b0 = buf[0], b1 = buf[1];
                var op = b0 & 0x0f;
                var len = b1 & 0x7f, off = 2;
                if (len === 126) { if (buf.length < 4) break; len = buf.readUInt16BE(2); off = 4; }
                else if (len === 127) { if (buf.length < 10) break; len = Number(buf.readBigUInt64BE(2)); off = 10; }
                var masked = (b1 & 0x80) !== 0;
                var mask = [0, 0, 0, 0];
                if (masked) { if (buf.length < off + 4) break; mask = [buf[off], buf[off+1], buf[off+2], buf[off+3]]; off += 4; }
                if (buf.length < off + len) break;
                var payload = Buffer.from(buf.slice(off, off + len));
                if (masked) for (var i = 0; i < payload.length; i++) payload[i] ^= mask[i % 4];
                buf = buf.slice(off + len);
                if (op === 0x8) { socket.end(); return; }
                if (op === 0x1 || op === 0x2) {
                    var echo = Buffer.from('echo:' + payload.toString());
                    var n = echo.length, hdr;
                    if (n < 126) hdr = Buffer.from([0x81, n]);
                    else if (n < 65536) { hdr = Buffer.alloc(4); hdr[0] = 0x81; hdr[1] = 126; hdr.writeUInt16BE(n, 2); }
                    else { hdr = Buffer.alloc(10); hdr[0] = 0x81; hdr[1] = 127; hdr.writeBigUInt64BE(BigInt(n), 2); }
                    socket.write(Buffer.concat([hdr, echo]));
                }
            }
        }
        socket.on('data', function(d) { buf = Buffer.concat([buf, d]); pump(); });
        pump();
    });
    server.listen(0, '127.0.0.1');
    globalThis.__server = server;
"#;

/// Starts the echo server in the engine and returns its bound port. The http
/// listen path binds synchronously (__httpListen → std bind), so the port is
/// readable right after eval — no event-loop pumping needed (and with a
/// listener open, run_event_loop() would not return on its own anyway).
async fn start_echo_server(e: &mut JsEngine) -> u16 {
    e.eval(ECHO_SERVER_JS).await.unwrap();
    let r = e
        .eval_to_string("String(__server.address().port)")
        .await
        .unwrap();
    let port: u16 = r.parse().expect("server never bound a port");
    assert!(port > 0);
    port
}

/// Runs `client` (a future, e.g. a tokio::task::spawn_blocking around the
/// blocking tungstenite client) while pumping the engine's event loop;
/// cancels the loop the moment the client finishes. (Not `run_event_loop()`
/// alone: with an http listener open it never returns on its own.)
async fn run_with_client<F, T>(e: &mut JsEngine, client: F) -> T
where
    F: std::future::Future<Output = T>,
{
    async fn drive_forever(e: &mut JsEngine) -> ! {
        loop {
            e.idle().await;
            let _ = e.run_event_loop().await;
            tokio::task::yield_now().await;
        }
    }
    tokio::pin!(client);
    tokio::select! {
        _ = drive_forever(e) => unreachable!("engine event loop terminated unexpectedly"),
        result = &mut client => result,
    }
}

/// The blocking tungstenite client, run on a blocking thread so it never
/// starves the event-loop pump on the test thread.
fn ws_client(
    port: u16,
    payload: String,
) -> tokio::task::JoinHandle<std::result::Result<String, String>> {
    tokio::task::spawn_blocking(move || {
        let (mut ws, resp) = tungstenite::connect(format!("ws://127.0.0.1:{port}/chat"))
            .map_err(|e| format!("handshake: {e}"))?;
        if resp.status() != 101 {
            return Err(format!("expected 101, got {}", resp.status()));
        }
        ws.send(tungstenite::Message::Text(payload.clone()))
            .map_err(|e| format!("send: {e}"))?;
        match ws.read().map_err(|e| format!("read: {e}"))? {
            tungstenite::Message::Text(t) => Ok(t.to_string()),
            other => Err(format!("expected text echo, got {other:?}")),
        }
    })
}

#[tokio::test]
async fn upgrade_handshake_and_echo_roundtrip() {
    let mut e = engine_with_net("127.0.0.1").await;
    let port = start_echo_server(&mut e).await;

    let result = run_with_client(&mut e, async move {
        ws_client(port, "hello-3va".into())
            .await
            .expect("client thread panicked")
            .expect("ws round-trip")
    })
    .await;

    assert_eq!(result, "echo:hello-3va");
}

/// Plain HTTP GET over a raw socket; returns the response body.
async fn http_get(port: u16, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut all = Vec::new();
    s.read_to_end(&mut all).await.unwrap();
    String::from_utf8_lossy(&all).to_string()
}

#[tokio::test]
async fn upgrade_still_serves_plain_http_on_other_connections() {
    let mut e = engine_with_net("127.0.0.1").await;
    let port = start_echo_server(&mut e).await;

    let body = run_with_client(&mut e, http_get(port, "/")).await;
    assert!(body.ends_with("http-ok"), "got: {body}");
}

#[tokio::test]
async fn upgrade_large_frame_echoed() {
    let mut e = engine_with_net("127.0.0.1").await;
    let port = start_echo_server(&mut e).await;

    let payload = "x".repeat(200_000);
    let expected = format!("echo:{payload}");
    let result = run_with_client(&mut e, async move {
        ws_client(port, payload)
            .await
            .expect("client thread panicked")
            .expect("ws round-trip")
    })
    .await;
    assert_eq!(result, expected);
}

#[tokio::test]
async fn upgrade_handshake_offers_hostile_request_without_crashing() {
    // A raw TCP client sends the upgrade request plus garbage after it; the
    // server must hand off the socket and keep serving, not crash.
    let mut e = engine_with_net("127.0.0.1").await;
    let port = start_echo_server(&mut e).await;

    let (upgrade_ok, http_ok) = run_with_client(&mut e, async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let req = "GET /chat HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        s.write_all(req.as_bytes()).await.unwrap();
        s.write_all(b"\x00\x00GARBAGE").await.unwrap();
        let mut resp = Vec::new();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), s.read_to_end(&mut resp)).await;
        let upgrade_ok = resp.windows(12).any(|w| w == b"101 Switchin");
        drop(s);
        // The same server still answers normal HTTP afterwards.
        let plain = http_get(port, "/").await;
        (upgrade_ok, plain)
    })
    .await;

    assert!(upgrade_ok, "server must still answer the upgrade request");
    assert!(
        http_ok.ends_with("http-ok"),
        "server must survive a hostile upgrade, got: {http_ok}"
    );
}

// ── regressions found in review ─────────────────────────────────────────────

/// A masked text frame ("hi") that a client may send in the same TCP segment
/// as the upgrade request.
fn masked_text_frame(text: &str) -> Vec<u8> {
    let mask = [1u8, 2, 3, 4];
    let mut frame = vec![
        0x81,
        0x80 | text.len() as u8,
        mask[0],
        mask[1],
        mask[2],
        mask[3],
    ];
    for (i, b) in text.bytes().enumerate() {
        frame.push(b ^ mask[i % 4]);
    }
    frame
}

const UPGRADE_REQUEST: &str = "GET /chat HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";

#[tokio::test]
async fn upgrade_bytes_sent_with_the_handshake_are_delivered_once() {
    // The bytes the http layer had already read behind the headers go to the
    // `upgrade` listener as `head` (as in Node); they must not ALSO come out of
    // the socket, or the first frame is processed twice. The echo server
    // consumes `head` and then the socket, like the `ws` library does.
    let mut e = engine_with_net("127.0.0.1").await;
    let port = start_echo_server(&mut e).await;

    let echoes = run_with_client(&mut e, async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut wire = UPGRADE_REQUEST.as_bytes().to_vec();
        wire.extend_from_slice(&masked_text_frame("hi"));
        s.write_all(&wire).await.unwrap(); // one write: handshake + first frame
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(800);
        while let Ok(Ok(n)) = tokio::time::timeout_at(deadline, s.read(&mut buf)).await {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        got.windows(7).filter(|w| *w == b"echo:hi").count()
    })
    .await;

    assert_eq!(echoes, 1, "the first frame must be echoed exactly once");
}

#[tokio::test]
async fn upgrade_head_unshifted_by_the_handler_is_delivered_once_and_in_order() {
    // The way the `ws` library takes `head`: socket.unshift(head), then attach
    // the 'data' listener. Frames sent with the handshake must come out of the
    // socket exactly once, ahead of anything that arrives later.
    let mut e = engine_with_net("127.0.0.1").await;
    e.eval(&ECHO_SERVER_JS.replace(
        "var buf = Buffer.from(head && head.length ? head : []);",
        "var buf = Buffer.alloc(0); if (head && head.length) socket.unshift(head);",
    ))
    .await
    .unwrap();
    let port: u16 = e
        .eval_to_string("String(__server.address().port)")
        .await
        .unwrap()
        .parse()
        .unwrap();

    let echoes = run_with_client(&mut e, async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut wire = UPGRADE_REQUEST.as_bytes().to_vec();
        wire.extend_from_slice(&masked_text_frame("one"));
        s.write_all(&wire).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        s.write_all(&masked_text_frame("two")).await.unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(800);
        while let Ok(Ok(n)) = tokio::time::timeout_at(deadline, s.read(&mut buf)).await {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&got).to_string()
    })
    .await;

    let one = echoes.matches("echo:one").count();
    let two = echoes.matches("echo:two").count();
    assert_eq!(
        (one, two),
        (1, 1),
        "each frame echoed once, got: {echoes:?}"
    );
    assert!(
        echoes.find("echo:one") < echoes.find("echo:two"),
        "the head frame must come out first"
    );
}

#[tokio::test]
async fn upgrade_sockets_destroyed_in_a_data_handler_leave_no_timers_behind() {
    // The server destroys the socket from inside its own 'data' handler (what
    // the `ws` library does on a bad frame). The read poll must not re-arm
    // itself after that, nor leave a callback behind per connection.
    const SESSIONS: usize = 30;
    let mut e = engine_with_net("127.0.0.1").await;
    e.eval(&ECHO_SERVER_JS.replace(
        "if (op === 0x8)",
        "if (payload.toString() === 'kill') { socket.destroy(); return; } if (op === 0x8)",
    ))
    .await
    .unwrap();
    let port: u16 = e
        .eval_to_string("String(__server.address().port)")
        .await
        .unwrap()
        .parse()
        .unwrap();

    run_with_client(&mut e, async move {
        for _ in 0..SESSIONS {
            tokio::task::spawn_blocking(move || {
                let (mut ws, _) =
                    tungstenite::connect(format!("ws://127.0.0.1:{port}/chat")).unwrap();
                ws.send(tungstenite::Message::Text("kill".into())).unwrap();
                let _ = ws.read(); // the server drops the connection
            })
            .await
            .unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    })
    .await;

    let left: usize = e
        .eval_to_string("String(Object.keys(globalThis.__timerCallbacks).length)")
        .await
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        left < 5,
        "{SESSIONS} destroyed upgrade sockets left {left} timer callbacks registered"
    );
}
