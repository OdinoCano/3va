// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// A large upload must not grow the process with the body size. This lives in
// its own test binary so the RSS measurement isn't polluted by other tests
// running in parallel in the same process.
//
// Without streaming, 2.12.4 buffered the whole body several times over
// (~1.4 GiB RSS for a 50 MiB upload) and panicked at ~300 MiB.
//
// Run: cargo test -p vvva_js --test http_body_memory

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
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

async fn wait_for_port(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("port {port} never became ready");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn drive_forever(e: &mut JsEngine) -> ! {
    loop {
        e.idle().await;
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
}

fn body_of(resp: &[u8]) -> &[u8] {
    let pos = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no header/body separator");
    &resp[pos + 4..]
}

fn rss_bytes() -> usize {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: usize = statm
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    pages * 4096
}

#[tokio::test]
async fn large_post_body_keeps_memory_bounded() {
    let port = free_port();
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var crypto = require('crypto');
        globalThis.__sum = null;
        var _server = http.createServer(function(req, res) {{
            var n = 0;
            var h = crypto.createHash('sha256');
            req.on('data', function(c) {{ n += c.length; h.update(c); }});
            req.on('end', function() {{
                globalThis.__sum = n + '|' + h.digest('hex');
                res.end('ok');
            }});
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    // 64 MiB of pseudo-random (mostly invalid-UTF-8) bytes. Without streaming
    // this buffered the whole body several times over and grew with the size.
    const N: usize = 64 * 1024 * 1024;
    let baseline = rss_bytes();
    let mut peak = baseline;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    let head = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {N}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes()).await.unwrap();

    let client = async {
        let mut chunk = vec![0u8; 256 * 1024];
        let mut state: u32 = 0x12345678;
        let mut sent = 0usize;
        while sent < N {
            for b in chunk.iter_mut() {
                // xorshift32 for cheap, deterministic, non-UTF-8-ish bytes.
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                *b = (state & 0xff) as u8;
            }
            let n = chunk.len().min(N - sent);
            stream.write_all(&chunk[..n]).await.unwrap();
            sent += n;
        }
        let mut resp = Vec::new();
        tokio::time::timeout(Duration::from_secs(120), stream.read_to_end(&mut resp))
            .await
            .unwrap()
            .unwrap();
        resp
    };

    // Sample RSS while the upload is driven by the engine loop.
    let resp = {
        let mut client = std::pin::pin!(client);
        let out = loop {
            tokio::select! {
                _ = drive_forever(&mut e) => unreachable!(),
                r = &mut client => break r,
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    peak = peak.max(rss_bytes());
                }
            }
        };
        peak = peak.max(rss_bytes());
        out
    };

    assert_eq!(body_of(&resp), b"ok");
    let sum = e.eval_to_string("globalThis.__sum").await.unwrap();
    assert!(sum.starts_with(&format!("{N}|")), "unexpected: {sum}");

    let growth = peak.saturating_sub(baseline);
    // A loose bound: this runs in the unoptimized test profile, where V8's
    // heap alone grows past 700 MiB for a 64 MiB upload. The unstreamed path
    // held the body several times over and grew past 1.4 GiB for a *50* MiB
    // upload in release, so anything near that (or a panic) fails here. The
    // tight, realistic figure is the release binary: 300 MiB upload → 345 MiB
    // peak RSS (see the PR).
    assert!(
        growth < 1200 * 1024 * 1024,
        "RSS grew by {} MiB for a {N}-byte upload — body is not streamed",
        growth / (1024 * 1024)
    );
}
