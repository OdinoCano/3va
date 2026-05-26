// Tests for net.createServer() — raw TCP server backed by __netListen/__netAcceptAsync.
//
// Run: cargo test -p vvva_js --test net_server

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

/// Drive the JS engine indefinitely — fires setTimeout callbacks AND async Promises.
///
/// Two systems need driving:
/// - `run_event_loop()` fires `timer_manager` (setTimeout) and `runtime_core` timers.
/// - `e.idle()` drives the rquickjs spawner (Async Promise futures like __netAcceptAsync).
///
/// The problem: `idle()` suspends on pending accepts (Poll::Pending) while we need timers
/// to fire concurrently.  Using tokio::select! with a short timeout breaks the deadlock.
async fn drive_forever(e: &JsEngine) -> ! {
    loop {
        // Drive spawner Promises (with 2ms cap so pending accepts don't block timers)
        tokio::select! {
            _ = e.idle() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
        }
        // Fire any scheduled setTimeout callbacks
        let _ = e.run_event_loop().await;
        // Give Tokio I/O reactor time to process events
        tokio::task::yield_now().await;
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn net_server_accepts_connection() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var net = require('net');
        globalThis.__connected = false;
        var _server = net.createServer(function(socket) {{
            globalThis.__connected = true;
            socket.write('hello\n');
            socket.end();
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    let task = tokio::task::spawn_blocking(move || {
        let mut s = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).ok();
        let mut buf = String::new();
        s.read_to_string(&mut buf).ok();
        buf
    });

    let response = tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        r = task => r.unwrap(),
    };

    assert_eq!(response.trim(), "hello");
}

#[tokio::test]
async fn net_server_receives_data() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var net = require('net');
        globalThis.__received = '';
        var _server = net.createServer(function(socket) {{
            socket.on('data', function(chunk) {{
                globalThis.__received += (typeof chunk === 'string' ? chunk : new TextDecoder().decode(chunk));
                socket.end();
            }});
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    let task = tokio::task::spawn_blocking(move || {
        let mut s = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).ok();
        s.write_all(b"ping").unwrap();
        s.shutdown(std::net::Shutdown::Write).ok();
        let mut resp = String::new();
        s.read_to_string(&mut resp).ok();
        resp
    });

    // drive_forever fires setTimeout (for _startPoll) and Promises.
    // The task completes only after the server reads data, fires 'data' callback,
    // and calls socket.end() — which means __received is already set.
    tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        _ = task => {},
    }

    let result = e.eval_to_string("globalThis.__received").await.unwrap();
    assert_eq!(result, "ping");
}

#[tokio::test]
async fn net_server_handles_multiple_connections() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var net = require('net');
        globalThis.__connCount = 0;
        var _server = net.createServer(function(socket) {{
            globalThis.__connCount++;
            socket.write(String(globalThis.__connCount));
            socket.end();
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    let mut responses = Vec::new();
    for _ in 0..3u32 {
        let task = tokio::task::spawn_blocking(move || {
            let mut s = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).ok();
            let mut buf = String::new();
            s.read_to_string(&mut buf).ok();
            buf
        });
        let resp = tokio::select! {
            _ = drive_forever(&e) => unreachable!(),
            r = task => r.unwrap(),
        };
        responses.push(resp);
    }

    assert_eq!(responses, vec!["1", "2", "3"]);
}

#[tokio::test]
async fn net_server_listening_flag() {
    let port = free_port();
    let e = engine_with_net().await;

    let result = e
        .eval_to_string(&format!(
            r#"
        var net = require('net');
        var _server = net.createServer(function(socket) {{ socket.end(); }});
        var before = _server.listening;
        _server.listen({port}, '127.0.0.1');
        var after = _server.listening;
        String(before) + ',' + String(after)
        "#,
            port = port,
        ))
        .await
        .unwrap();

    assert_eq!(result, "false,true");
}
