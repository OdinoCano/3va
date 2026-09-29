// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Integration tests for the `dgram` (UDP) builtin: real datagrams over the
// loopback interface, both directions, plus the permission gate.
// Run: cargo test -p vvva_js --test dgram_module

use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

async fn engine_no_net() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

/// Drive the JS event loop until `js_expr` evaluates to the string `"true"`.
///
/// Uses `pump_timers()`, not `run_event_loop()`: a dgram socket keeps a
/// 10 ms `setInterval` poll running until `close()`, which would make
/// `run_event_loop()` spin to its 100_000-iteration cap (~10 ms per
/// iteration) — i.e. ~18 minutes — before returning. `pump_timers()` fires
/// each expired timer once and returns, so the recurring poll makes
/// progress every iteration without the runaway loop.
async fn wait_true(engine: &mut JsEngine, js_expr: &str) -> bool {
    for _ in 0..200 {
        engine.idle().await;
        let _ = engine.pump_timers().await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        if engine.eval_to_string(js_expr).await.unwrap() == "true" {
            return true;
        }
    }
    false
}

// ── JS socket ← Rust datagram ────────────────────────────────────────────────

#[tokio::test]
async fn socket_receives_a_real_datagram_from_loopback() {
    let mut e = engine_with_net().await;
    e.eval(
        r#"
        globalThis.__udp = { received: null, port: 0 };
        var dgram = require('dgram');
        var sock = dgram.createSocket('udp4');
        sock.on('message', function(msg, rinfo) {
            globalThis.__udp.received = msg.toString();
        });
        sock.bind(0, '127.0.0.1');
        globalThis.__udp.port = sock.address().port;
        "#,
    )
    .await
    .unwrap();

    let port: u16 = e
        .eval_to_string("String(globalThis.__udp.port)")
        .await
        .unwrap()
        .parse()
        .expect("the JS socket must expose a bound port");
    assert_ne!(port, 0, "socket did not bind to an ephemeral port");

    // Rust side sends a real UDP datagram to the JS socket.
    let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sender
        .send_to(b"ping-udp", ("127.0.0.1", port))
        .await
        .unwrap();

    assert!(
        wait_true(&mut e, "String(globalThis.__udp.received !== null)").await,
        "the 'message' event never fired for the Rust datagram"
    );
    let got = e
        .eval_to_string("String(globalThis.__udp.received)")
        .await
        .unwrap();
    assert_eq!(got, "ping-udp");
}

// ── JS socket → Rust datagram ────────────────────────────────────────────────

#[tokio::test]
async fn socket_sends_a_real_datagram_to_loopback() {
    let recv = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = recv.local_addr().unwrap().port();

    let mut e = engine_with_net().await;
    e.eval(&format!(
        r#"
            globalThis.__udp = {{ err: null }};
            var dgram = require('dgram');
            var sock = dgram.createSocket('udp4');
            sock.send('hello-udp', {port}, '127.0.0.1', function(err) {{
                globalThis.__udp.err = err ? String(err) : null;
            }});
            "#
    ))
    .await
    .unwrap();

    // `__udpSend` is synchronous, so the datagram is already on the wire after
    // the eval; still drive the loop a little so the send callback runs.
    let _ = wait_true(&mut e, "globalThis.__udp.err === null").await;

    let mut buf = [0u8; 64];
    let (n, _src) =
        tokio::time::timeout(std::time::Duration::from_secs(5), recv.recv_from(&mut buf))
            .await
            .expect("timed out waiting for the JS socket's datagram")
            .unwrap();
    assert_eq!(&buf[..n], b"hello-udp");
}

// ── Permission gate ──────────────────────────────────────────────────────────

#[tokio::test]
async fn send_is_denied_without_a_net_grant() {
    let mut e = engine_no_net().await;
    e.eval(
        r#"
        globalThis.__udp = { err: null };
        var dgram = require('dgram');
        var sock = dgram.createSocket('udp4');
        sock.send('x', 9999, '127.0.0.1', function(err) {
            globalThis.__udp.err = err ? String(err) : null;
        });
        "#,
    )
    .await
    .unwrap();

    let _ = wait_true(&mut e, "globalThis.__udp.err !== null").await;
    let err = e
        .eval_to_string("String(globalThis.__udp.err)")
        .await
        .unwrap();
    assert!(
        err.contains("EACCES") || err.contains("denied"),
        "expected a permission denial, got: {err}"
    );
}
