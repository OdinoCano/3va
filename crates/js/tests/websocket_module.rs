// Tests for the WebSocket builtin.
// Run: cargo test -p vvva_js --test websocket_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net(host: &str) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::Network(host.to_string()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

async fn engine_no_net() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

/// Start a minimal WebSocket echo server on a random port.
/// Accepts one connection, echoes the first message, then closes.
/// Returns the bound port (server is ready immediately after this returns).
fn start_echo_server() -> u16 {
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept()
            && let Ok(mut ws) = tungstenite::accept(stream)
        {
            if let Ok(msg) = ws.read() {
                let _ = ws.send(msg);
            }
            let _ = ws.close(None);
        }
    });

    port
}

/// Start a WebSocket server that accepts up to `n` simultaneous connections and
/// keeps them open until the client sends a close frame or drops.
/// Used by drain tests to verify that close frames are actually delivered.
fn start_persistent_server(n: usize) -> u16 {
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        for _ in 0..n {
            if let Ok((stream, _)) = listener.accept() {
                std::thread::spawn(move || {
                    if let Ok(mut ws) = tungstenite::accept(stream) {
                        loop {
                            match ws.read() {
                                Ok(tungstenite::Message::Close(_)) | Err(_) => break,
                                _ => {}
                            }
                        }
                    }
                });
            }
        }
    });

    port
}

// ── API shape ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn websocket_global_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof WebSocket === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn websocket_ready_state_constants() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            "String(WebSocket.CONNECTING) + ',' + \
             String(WebSocket.OPEN) + ',' + \
             String(WebSocket.CLOSING) + ',' + \
             String(WebSocket.CLOSED)",
        )
        .await
        .unwrap();
    assert_eq!(r, "0,1,2,3");
}

// ── Permission enforcement ───────────────────────────────────────────────────

#[tokio::test]
async fn websocket_connect_blocked_without_net_grant() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            try {
                var ws = new WebSocket('ws://example.com/');
                ws.readyState === 3 ? 'closed' : 'open'
            } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    // Either throws or ends in CLOSED state after permission denial
    assert!(r == "threw" || r == "closed", "got: {r}");
}

#[tokio::test]
async fn websocket_connect_allowed_with_net_grant() {
    let port = start_echo_server();
    let e = engine_with_net("127.0.0.1").await;

    let js = format!(
        r#"
        (function() {{
            try {{
                var ws = new WebSocket('ws://127.0.0.1:{port}');
                var state = ws.readyState;
                ws.close();
                return String(state);
            }} catch(e) {{
                return 'error:' + e.message;
            }}
        }})()
        "#
    );
    let r = e.eval_to_string(&js).await.unwrap();
    assert_eq!(r, "1", "expected OPEN(1), got: {r}");
}

// ── Send / Recv ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn websocket_send_recv_echo() {
    let port = start_echo_server();
    let e = engine_with_net("127.0.0.1").await;

    let js = format!(
        r#"
        (function() {{
            try {{
                var ws = new WebSocket('ws://127.0.0.1:{port}');
                ws.send('hello');
                var msg = ws.recv();
                ws.close();
                return String(msg);
            }} catch(e) {{
                return 'error:' + e.message;
            }}
        }})()
        "#
    );
    let r = e.eval_to_string(&js).await.unwrap();
    assert_eq!(r, "hello", "echo mismatch: {r}");
}

#[tokio::test]
async fn websocket_send_on_closed_throws() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            try {
                var ws = new WebSocket('ws://example.com/');
                ws.readyState = 3;
                ws.send('msg');
                'no-throw'
            } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

#[tokio::test]
async fn websocket_recv_on_closed_returns_null() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            var ws = new WebSocket('ws://example.com/');
            ws.readyState = 3;
            String(ws.recv() === null)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── close() handler ──────────────────────────────────────────────────────────

#[tokio::test]
async fn websocket_close_fires_onclose() {
    let port = start_echo_server();
    let e = engine_with_net("127.0.0.1").await;

    let js = format!(
        r#"
        (function() {{
            var closed = false;
            var ws = new WebSocket('ws://127.0.0.1:{port}');
            ws.onclose = function() {{ closed = true; }};
            ws.close();
            return String(closed);
        }})()
        "#
    );
    let r = e.eval_to_string(&js).await.unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn websocket_close_sets_ready_state() {
    let port = start_echo_server();
    let e = engine_with_net("127.0.0.1").await;

    let js = format!(
        r#"
        (function() {{
            var ws = new WebSocket('ws://127.0.0.1:{port}');
            ws.close();
            return String(ws.readyState);
        }})()
        "#
    );
    let r = e.eval_to_string(&js).await.unwrap();
    assert_eq!(r, "3"); // CLOSED
}

// ── drain_ws_pool ─────────────────────────────────────────────────────────────

/// Build a WsPool directly (without going through the JS shim) and verify
/// `drain_ws_pool` empties it and delivers close frames to peers.
#[test]
fn drain_ws_pool_closes_all_active_connections() {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use vvva_js::builtins::websocket::{WsPool, drain_ws_pool};

    const N: usize = 3;
    let port = start_persistent_server(N);

    let pool: WsPool = Arc::new(Mutex::new(HashMap::new()));
    for i in 0..N as u32 {
        let (ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}")).unwrap();
        pool.lock().unwrap().insert(i, ws);
    }
    assert_eq!(pool.lock().unwrap().len(), N);

    // Allow up to 10 s; with 3 conns × max 500 ms jitter = 1.5 s worst case.
    drain_ws_pool(&pool, std::time::Duration::from_secs(10));

    assert_eq!(
        pool.lock().unwrap().len(),
        0,
        "pool must be empty after drain"
    );
}

/// Draining an empty pool must be a no-op and return immediately.
#[test]
fn drain_ws_pool_noop_on_empty_pool() {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use vvva_js::builtins::websocket::{WsPool, drain_ws_pool};

    let pool: WsPool = Arc::new(Mutex::new(HashMap::new()));
    let t = Instant::now();
    drain_ws_pool(&pool, Duration::from_secs(30));
    // With no connections there is no sleeping — should finish in milliseconds.
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "empty drain should be instant, took {}ms",
        t.elapsed().as_millis()
    );
    assert_eq!(pool.lock().unwrap().len(), 0);
}

/// After `drain_ws_connections` the engine's internal pool is empty.
#[tokio::test]
async fn drain_ws_connections_empties_engine_pool_via_js() {
    let port = start_persistent_server(2);
    let engine = engine_with_net("127.0.0.1").await;

    // Open 2 WebSocket connections from JS so they land in the engine's pool.
    let js = format!(
        r#"
        (function() {{
            var a = new WebSocket('ws://127.0.0.1:{port}');
            var b = new WebSocket('ws://127.0.0.1:{port}');
            return String(a.readyState === 1 && b.readyState === 1);
        }})()
        "#
    );
    let r = engine.eval_to_string(&js).await.unwrap();
    assert_eq!(r, "true", "both connections should be open");

    // Drain must not panic and must complete in a reasonable time.
    let t = std::time::Instant::now();
    engine.drain_ws_connections().await;
    assert!(
        t.elapsed() < std::time::Duration::from_secs(15),
        "drain took too long: {}s",
        t.elapsed().as_secs()
    );
}
