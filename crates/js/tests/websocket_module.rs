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
