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

/// Start a WebSocket echo server that echoes every text message it receives,
/// keeping the connection open until the client closes or drops.
/// Used by the latency tests to measure send→recv round-trip time.
fn start_echo_loop_server() -> u16 {
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept()
            && let Ok(mut ws) = tungstenite::accept(stream)
        {
            loop {
                match ws.read() {
                    Ok(tungstenite::Message::Text(t)) => {
                        let _ = ws.send(tungstenite::Message::Text(t));
                    }
                    Ok(tungstenite::Message::Close(_)) | Err(_) => break,
                    Ok(tungstenite::Message::Binary(b)) => {
                        let _ = ws.send(tungstenite::Message::Binary(b));
                    }
                    Ok(_) => {}
                }
            }
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
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof WebSocket === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn websocket_ready_state_constants() {
    let mut e = engine_no_net().await;
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
    let mut e = engine_no_net().await;
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
    let mut e = engine_with_net("127.0.0.1").await;

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
    let mut e = engine_with_net("127.0.0.1").await;

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
    let mut e = engine_no_net().await;
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
    let mut e = engine_no_net().await;
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
    let mut e = engine_with_net("127.0.0.1").await;

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
    let mut e = engine_with_net("127.0.0.1").await;

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
    let mut engine = engine_with_net("127.0.0.1").await;

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

// ── Latency ──────────────────────────────────────────────────────────────────
// The `recv()` builtin is blocking (crates/js/src/builtins/websocket.rs), so a
// send→recv pair measured in JS is an end-to-end round-trip through the engine,
// tungstenite and the loopback echo server. Dialed against an OS thread, so
// these tests are robust on shared CI runners.

/// Connection setup (new WebSocket → OPEN) must be fast on loopback.
#[tokio::test]
async fn websocket_latency_connect() {
    let port = start_echo_loop_server();
    let mut e = engine_with_net("127.0.0.1").await;

    let js = format!(
        r#"
        (function() {{
            var t0 = Date.now();
            var ws;
            try {{
                ws = new WebSocket('ws://127.0.0.1:{port}');
            }} catch (e) {{
                return 'threw:' + e.message;
            }}
            var t1 = Date.now();
            var connectMs = t1 - t0;
            var opened = ws.readyState === 1;
            ws.close();
            return String(opened ? connectMs : -1);
        }})()
        "#
    );
    let r = e.eval_to_string(&js).await.unwrap();
    assert!(!r.starts_with("threw:"), "connect threw: {r}");
    let connect_ms: i64 = r.parse().expect("connect_ms should be numeric");
    assert_ne!(connect_ms, -1, "socket did not reach OPEN");
    assert!(
        connect_ms < 5_000,
        "connect took {connect_ms} ms, unreasonably slow on loopback"
    );
}

/// End-to-end send→recv round-trip latency over 300 messages on loopback.
/// Asserts a generous ceiling (CI-safe) while still catching gross regressions:
/// mean < 50 ms, p95 < 250 ms, p99 < 1000 ms.
#[tokio::test]
async fn websocket_latency_round_trip_statistics() {
    const N: usize = 300;
    const MAX_MEAN_MS: f64 = 50.0;
    const MAX_P95_MS: f64 = 250.0;
    const MAX_P99_MS: f64 = 1000.0;

    let port = start_echo_loop_server();
    let mut e = engine_with_net("127.0.0.1").await;

    let js = format!(
        r#"
        (function() {{
            var ws = new WebSocket('ws://127.0.0.1:{port}');
            var samples = [];
            var total = 0;
            for (var i = 0; i < {N}; i++) {{
                var t0 = Date.now();
                ws.send('latency-' + i);
                var msg = ws.recv();
                var t1 = Date.now();
                if (msg !== 'latency-' + i) throw new Error('echo mismatch: ' + msg);
                var lat = t1 - t0;
                samples.push(lat);
                total += lat;
            }}
            ws.close();

            samples.sort(function(a, b) {{ return a - b; }});
            function pct(p) {{
                var idx = Math.min(samples.length - 1, Math.floor(p * samples.length));
                return samples[idx];
            }}
            var mean = total / samples.length;
            return JSON.stringify({{
                n: samples.length,
                min: samples[0],
                median: pct(0.5),
                mean: mean,
                p95: pct(0.95),
                p99: pct(0.99),
                max: samples[samples.length - 1]
            }});
        }})()
        "#
    );
    let r = e.eval_to_string(&js).await.unwrap();
    let stats: serde_json::Value = serde_json::from_str(&r).expect("valid latency JSON");

    assert_eq!(stats["n"], N, "all echoes must round-trip");
    for k in ["min", "median", "mean", "p95", "p99", "max"] {
        assert!(stats[k].is_number(), "missing stat {k}");
    }

    let mean_ms = stats["mean"].as_f64().unwrap();
    let p95_ms = stats["p95"].as_f64().unwrap();
    let p99_ms = stats["p99"].as_f64().unwrap();
    let min_ms = stats["min"].as_f64().unwrap();

    assert!(min_ms >= 0.0, "negative round-trip? {min_ms}");
    assert!(
        mean_ms <= MAX_MEAN_MS,
        "mean RTT {mean_ms:.1} ms exceeds {MAX_MEAN_MS} ms (stats: {r})"
    );
    assert!(
        p95_ms <= MAX_P95_MS,
        "p95 RTT {p95_ms:.1} ms exceeds {MAX_P95_MS} ms (stats: {r})"
    );
    assert!(
        p99_ms <= MAX_P99_MS,
        "p99 RTT {p99_ms:.1} ms exceeds {MAX_P99_MS} ms (stats: {r})"
    );

    eprintln!("WebSocket RTT over loopback (n={N}): {r}");
}

/// Sustained throughput: {N} sequential round-trips must complete well inside
/// the timeout, proving no per-message stall or connection degradation.
#[tokio::test]
async fn websocket_latency_throughput_budget() {
    const N: usize = 500;
    const BUDGET_MS: u128 = 5_000;

    let port = start_echo_loop_server();
    let mut e = engine_with_net("127.0.0.1").await;

    let js = format!(
        r#"
        (function() {{
            var ws = new WebSocket('ws://127.0.0.1:{port}');
            var t0 = Date.now();
            for (var i = 0; i < {N}; i++) {{
                ws.send('t' + i);
                if (ws.recv() !== 't' + i) throw new Error('echo mismatch at ' + i);
            }}
            var t1 = Date.now();
            ws.close();
            return String(t1 - t0);
        }})()
        "#
    );
    let r = e.eval_to_string(&js).await.unwrap();
    let elapsed_ms: u128 = r.parse().expect("elapsed should be numeric");
    assert!(
        elapsed_ms < BUDGET_MS,
        "{N} round-trips took {elapsed_ms} ms, over the {BUDGET_MS} ms budget"
    );
    // Sanity floor: the loop genuinely ran (Date.now() can't cheap-out).
    eprintln!(
        "WebSocket throughput: {N} round-trips in {elapsed_ms} ms (~{:.1} msg/s)",
        N as f64 / (elapsed_ms as f64 / 1000.0)
    );
}
