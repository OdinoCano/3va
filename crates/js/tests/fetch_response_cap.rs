// Tests for fetch() response-body size cap.
//
// fetch() buffers the whole response body in memory before handing it to JS,
// so a malicious server must not be able to make it buffer an unbounded
// stream. These tests spin a raw TCP server that streams more bytes than the
// cap and assert fetch() rejects with a controlled error instead of
// accumulating.
//
// Run: cargo test -p vvva_js --test fetch_response_cap

use std::io::Write;
use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// Serve exactly one HTTP request on `port`, then respond per `respond` and
/// close. Runs on its own thread; ready as soon as the listener is bound.
fn one_shot_server(port: u16, respond: impl FnOnce(&mut std::net::TcpStream) + Send + 'static) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        if let Ok((mut sock, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            // Read until end of request headers; ignore parse errors — the
            // client (ureq) sends a complete request immediately.
            let _ = std::io::Read::read(&mut sock, &mut buf);
            respond(&mut sock);
            let _ = sock.flush();
        }
    });
}

/// Drive the JS event loop until `done_js` (an expression over globalThis)
/// turns truthy or the deadline lapses; returns the last value of `read_js`.
async fn wait_for(
    e: &mut JsEngine,
    done_js: &str,
    read_js: &str,
    deadline: std::time::Duration,
) -> Option<String> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > deadline {
            return None;
        }
        let done = e.eval_to_string(done_js).await.unwrap_or_default();
        if done == "true" {
            return Some(e.eval_to_string(read_js).await.unwrap_or_default());
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
}

fn fetch_and_capture(port: u16, options: &str) -> String {
    format!(
        r#"
        globalThis.__fetchResult = null;
        globalThis.__fetchError = null;
        fetch('http://127.0.0.1:{port}/', {options})
            .then(function(r) {{ return r.text(); }})
            .then(function(t) {{ globalThis.__fetchResult = t; }})
            .catch(function(err) {{
                globalThis.__fetchError = String((err && err.message) ? err.message : err);
            }});
        "#
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_body_under_cap_succeeds() {
    let port = free_port();
    let payload = "x".repeat(256 * 1024);
    let payload_clone = payload.clone();
    one_shot_server(port, move |sock| {
        write!(
            sock,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload_clone.len()
        )
        .unwrap();
        sock.write_all(payload_clone.as_bytes()).unwrap();
    });

    let mut e = engine_with_net().await;
    e.eval_to_string(&fetch_and_capture(port, "{}"))
        .await
        .unwrap();
    let result = wait_for(
        &mut e,
        "globalThis.__fetchResult !== null || globalThis.__fetchError !== null",
        "globalThis.__fetchError === null ? 'ok:' + globalThis.__fetchResult.length : 'err'",
        std::time::Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(result, format!("ok:{}", payload.len()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_rejects_oversized_stream_via_max_response_size_option() {
    let port = free_port();
    // Stream 512 KiB with no Content-Length: body framing is only terminated
    // by connection close, so the reader must count real bytes.
    let total = 512 * 1024;
    one_shot_server(port, move |sock| {
        write!(sock, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n").unwrap();
        let chunk = [b'a'; 8192];
        let mut sent = 0;
        while sent < total {
            sock.write_all(&chunk).unwrap();
            sent += chunk.len();
        }
    });

    let mut e = engine_with_net().await;
    e.eval_to_string(&fetch_and_capture(port, "{ maxResponseSize: 1024 }"))
        .await
        .unwrap();
    let err = wait_for(
        &mut e,
        "globalThis.__fetchError !== null",
        "globalThis.__fetchError",
        std::time::Duration::from_secs(10),
    )
    .await
    .expect("fetch neither resolved nor rejected within deadline");
    assert!(
        err.contains("maximum response size"),
        "unexpected error: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_rejects_early_on_oversized_content_length() {
    let port = free_port();
    // Declare a huge Content-Length but send no body at all: the early check
    // must reject before any body byte is read.
    one_shot_server(port, |sock| {
        write!(
            sock,
            "HTTP/1.1 200 OK\r\nContent-Length: 100000000\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });

    let mut e = engine_with_net().await;
    e.eval_to_string(&fetch_and_capture(port, "{ maxResponseSize: 1024 }"))
        .await
        .unwrap();
    let err = wait_for(
        &mut e,
        "globalThis.__fetchError !== null",
        "globalThis.__fetchError",
        std::time::Duration::from_secs(10),
    )
    .await
    .expect("fetch neither resolved nor rejected within deadline");
    assert!(
        err.contains("Content-Length") && err.contains("maximum response size"),
        "unexpected error: {err}"
    );
}
