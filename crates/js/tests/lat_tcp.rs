// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Latency baseline for the net/TCP echo transport, measured as a true
// end-to-end loopback ping→pong with RTT statistics. Kept as its own test
// binary so each transport's engine runs in isolation.
//
// Run: cargo test -p vvva_js --test lat_tcp
//      cargo test -p vvva_js --features fips --test lat_tcp

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

/// Round-trip ping→pong latency over a JS net server that echoes each chunk.
/// Measured from the Rust side (Instant around write+read) while the engine
/// runs the socket write path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_echo_round_trip_statistics() {
    const N: usize = 200;
    const MAX_MEAN_MS: f64 = 25.0;
    const MAX_P99_MS: f64 = 500.0;

    let port = free_port();
    let mut e = engine_with_net("127.0.0.1").await;

    e.eval_to_string(&format!(
        r#"
        var net = require('net');
        var _server = net.createServer(function(socket) {{
            socket.on('data', function(chunk) {{
                socket.write(chunk);
            }});
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "echo server never up");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let client = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        use std::net::TcpStream;

        let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .ok();
        let mut latencies = Vec::with_capacity(N);
        for i in 0..N {
            let msg = format!("ping{i:08}");
            let t0 = std::time::Instant::now();
            s.write_all(msg.as_bytes()).unwrap();
            let mut buf = [0u8; 12];
            let mut n = 0;
            while n < msg.len() {
                let r = s.read(&mut buf[n..]).unwrap();
                if r == 0 {
                    break;
                }
                n += r;
            }
            latencies.push(t0.elapsed().as_secs_f64() * 1000.0);
            if &buf[..msg.len()] != msg.as_bytes() {
                panic!("echo mismatch at {i}");
            }
        }
        latencies
    });
    let latencies = drive_until(&mut e, client).await.unwrap();

    assert_eq!(latencies.len(), N, "must complete {N} echoes");
    let mut sorted = latencies.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let mean = latencies.iter().sum::<f64>() / latencies.len() as f64;
    let p99 = sorted[(sorted.len() as f64 * 0.99).floor() as usize];

    assert!(
        mean <= MAX_MEAN_MS,
        "mean TCP RTT {mean:.2} ms exceeds {MAX_MEAN_MS} ms"
    );
    assert!(
        p99 <= MAX_P99_MS,
        "p99 TCP RTT {p99:.2} ms exceeds {MAX_P99_MS} ms"
    );
    eprintln!(
        "TCP echo RTT (n={N}): mean {mean:.2} ms, p99 {p99:.2} ms, min {:.2} ms, max {:.2} ms",
        sorted[0],
        sorted[sorted.len() - 1]
    );
}
