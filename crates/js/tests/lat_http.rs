// Latency baseline for the http.createServer transport, measured end-to-end
// against a real JS server on loopback. Kept as its own test binary so each
// transport's engine runs in isolation.
//
// Run: cargo test -p vvva_js --test lat_http
//      cargo test -p vvva_js --features fips --test lat_http

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

/// Drive the engine indefinitely, firing setTimeout callbacks AND async
/// Promises, for transports measured from the Rust side (HTTP server, TCP).
async fn drive_forever(e: &mut JsEngine) -> ! {
    loop {
        tokio::select! {
            _ = e.idle() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
        }
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

async fn raw_http(port: u16, method: &str, path: &str, body: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    let req = if body.is_empty() {
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
            len = body.len(),
        )
    };
    let _ = stream.write_all(req.as_bytes()).await;
    let mut resp = Vec::new();
    let _ = stream.read_to_end(&mut resp).await;
    String::from_utf8_lossy(&resp).into_owned()
}

#[tokio::test]
async fn http_server_response_latency_budget() {
    const N: usize = 100;
    const BUDGET_MS: u128 = 5_000;

    let port = free_port();
    let mut e = engine_with_net("127.0.0.1").await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var _server = http.createServer(function(req, res) {{
            res.end('pong');
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    // Wait for the accept loop to be up.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "port {port} never became ready"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let t0 = std::time::Instant::now();
    for _ in 0..N - 1 {
        drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    }
    let resp = drive_until(&mut e, raw_http(port, "GET", "/", "")).await;
    let elapsed_ms = t0.elapsed().as_millis();

    assert!(resp.contains("200"), "expected 200 response: {resp}");
    assert!(
        elapsed_ms < BUDGET_MS,
        "{N} HTTP requests took {elapsed_ms} ms, over budget {BUDGET_MS} ms"
    );
    eprintln!(
        "http.createServer latency: {N} requests in {elapsed_ms} ms (~{:.1} req/s)",
        N as f64 / (elapsed_ms as f64 / 1000.0)
    );
}
