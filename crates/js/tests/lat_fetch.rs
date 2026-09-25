// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Latency baseline for the fetch() (HTTP/1.1 client) transport, measured
// end-to-end against a local server on loopback. Kept as its own test binary
// so each transport's engine runs in isolation (a single consolidated binary
// with every transport was hostile to the async JS clients).
//
// Run: cargo test -p vvva_js --test lat_fetch
//      cargo test -p vvva_js --features fips --test lat_fetch

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net(host: &str) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::Network(host.to_string()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

/// Pump the engine's JS timers + async spawner until `done_js` (an expression
/// over globalThis) reads "true", or the deadline lapses.
///
/// The whole loop is wrapped in `tokio::time::timeout` because
/// `run_event_loop()` can block for a while while a control connection is open
/// and its EAGAIN-backoff poll keeps rescheduling itself.
async fn pump_until(e: &mut JsEngine, done_js: &str, deadline: std::time::Duration) -> bool {
    tokio::time::timeout(deadline, async {
        loop {
            tokio::select! {
                _ = e.idle() => {},
                _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
            }
            let _ = e.run_event_loop().await;
            tokio::task::yield_now().await;
            if e.eval_to_string(done_js).await.unwrap_or_default() == "true" {
                return true;
            }
        }
    })
    .await
    .is_ok()
        && e.eval_to_string(done_js).await.unwrap_or_default() == "true"
}

/// Serve up to `n` sequential HTTP/1.1 requests with an immediate tiny 200.
/// Each request gets its own connection; `Connection: close` per response.
fn n_shot_http_server(port: u16, n: usize) {
    use std::io::Write;
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        for _ in 0..n {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut sock, &mut buf);
            write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
            )
            .unwrap();
            let _ = sock.flush();
        }
        drop(listener);
    });
}

/// Per-request latency of fetch(): DNS comes from /etc/hosts (127.0.0.1),
/// TLS is off, and the server answers immediately — so what's measured is the
/// engine's HTTP client + body parsing path. Asserted as a CI-safe budget for
/// {N} sequential requests.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_latency_sequential_requests_budget() {
    const N: usize = 50;
    const BUDGET_MS: u128 = 5_000;

    let port = free_port();
    n_shot_http_server(port, N);
    let mut e = engine_with_net("127.0.0.1").await;

    let js = format!(
        r#"
        var i = 0, pending = 0, err = null, elapsed = 0;
        var t0 = Date.now();
        function next() {{
            fetch('http://127.0.0.1:{port}/', {{}})
                .then(function(r) {{ return r.text(); }})
                .then(function(t) {{
                    globalThis.__last = t;
                    i++;
                    if (i < {N}) next();
                    else {{
                        elapsed = Date.now() - t0;
                        globalThis.__done = true;
                    }}
                }})
                .catch(function(x) {{ err = String(x); globalThis.__done = true; }});
        }}
        next();
        "#
    );
    e.eval_to_string(&js).await.unwrap();

    let ok = pump_until(
        &mut e,
        "globalThis.__done === true && (i === {N} || err !== null)"
            .replace("{N}", &N.to_string())
            .as_str(),
        std::time::Duration::from_secs(15),
    )
    .await;
    assert!(ok, "fetch sequence never completed within 15s");

    let err = e.eval_to_string("String(err)").await.unwrap();
    assert_eq!(err, "null", "fetch error: {err}");
    let elapsed_ms: u128 = e
        .eval_to_string("String(elapsed)")
        .await
        .unwrap()
        .parse()
        .expect("elapsed numeric");
    assert!(
        elapsed_ms < BUDGET_MS,
        "{N} fetch round-trips took {elapsed_ms} ms, over budget {BUDGET_MS} ms"
    );
    eprintln!(
        "fetch latency: {N} requests in {elapsed_ms} ms (~{:.1} req/s)",
        N as f64 / (elapsed_ms as f64 / 1000.0)
    );
}
