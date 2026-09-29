// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Integration tests for the `http2` builtin over real HTTP/2 frames:
//   - the engine's JS h2 server answered by a raw `h2` client, and
//   - the engine's JS h2 client talking to a raw `h2` server.
// Run: cargo test -p vvva_js --test http2_module

use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
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

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

/// Poll until the JS server has bound to `port`, without opening a connection.
async fn wait_for_port(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err() {
            return; // port taken → server is up
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("port {port} never became ready within 5 s");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Drive the JS event loop forever (for use in tokio::select! left branch).
async fn drive_forever(e: &mut JsEngine) -> ! {
    loop {
        e.idle().await;
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
}

/// Drive the JS event loop until the client future completes.
async fn drive_until<T>(e: &mut JsEngine, client: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(client);
    tokio::select! {
        _ = drive_forever(e) => unreachable!("engine event loop terminated unexpectedly"),
        result = &mut client => result,
    }
}

/// A real `h2` server, run in its own thread so the synchronous JS
/// `http2.connect()` (which blocks the V8 thread while it handshakes) can
/// complete against it. Answers every request with `200 rust-h2:{path}`.
fn spawn_rust_h2_server() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let tl = tokio::net::TcpListener::from_std(listener).unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let (tcp, _) = tl.accept().await.unwrap();
            let mut conn = h2::server::handshake(tcp).await.unwrap();
            let (request, mut respond) = conn.accept().await.unwrap().unwrap();
            let path = request.uri().path().to_string();
            let resp = http::Response::builder().status(200).body(()).unwrap();
            let mut send = respond.send_response(resp, false).unwrap();
            send.send_data(bytes::Bytes::from(format!("rust-h2:{path}")), true)
                .unwrap();
            // Drive the connection so the response frames actually flush; the
            // client sends nothing else, so bound this on a short timeout.
            let _ = tokio::time::timeout(Duration::from_millis(300), async {
                let _ = conn.accept().await;
            })
            .await;
        });
    });
    port
}

// ── Engine's h2 server ← raw h2 client ──────────────────────────────────────

#[tokio::test]
async fn h2_server_answers_a_real_client_request() {
    let port = free_port();
    let mut e = engine_with_net().await;
    e.eval(&format!(
        r#"
        globalThis.__h2s = {{ paths: [] }};
        var http2 = require('http2');
        var server = http2.createServer(function(stream, headers) {{
            globalThis.__h2s.paths.push(new URL(stream.url).pathname);
            stream.respond({{ ':status': 200, 'content-type': 'text/plain' }});
            stream.end('h2 hello');
        }});
        server.listen({port}, '127.0.0.1');
        "#
    ))
    .await
    .unwrap();
    wait_for_port(port).await;

    let client = async {
        let tcp = TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap_or_else(|e| panic!("connect: {e}"));
        let (mut send_request, conn) = h2::client::handshake(tcp).await.unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let req = http::Request::builder()
            .method("GET")
            .uri(format!("http://127.0.0.1:{port}/ping"))
            .body(())
            .unwrap();
        let (resp, _send_stream) = send_request.send_request(req, true).unwrap();
        let resp = resp.await.unwrap();
        assert_eq!(resp.status(), 200, "expected a 200 over HTTP/2");
        let mut body = resp.into_body();
        let mut out = Vec::new();
        while let Some(chunk) = body.data().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        String::from_utf8(out).unwrap()
    };
    let body = drive_until(&mut e, client).await;
    assert_eq!(
        body, "h2 hello",
        "the JS h2 server must send real DATA frames"
    );

    // The server side of the engine must have seen the real request path.
    let paths = e
        .eval_to_string("JSON.stringify(globalThis.__h2s.paths)")
        .await
        .unwrap();
    assert_eq!(paths, "[\"/ping\"]");
}

// ── Engine's JS h2 client → raw h2 server ───────────────────────────────────

#[tokio::test]
async fn js_h2_client_receives_a_real_response() {
    let port = spawn_rust_h2_server();
    let mut e = engine_with_net().await;

    e.eval(&format!(
        r#"
        globalThis.__h2c = {{ status: null, body: '', done: false, err: null, cid: null }};
        var http2 = require('http2');
        var session = http2.connect('http://127.0.0.1:{port}/');
        globalThis.__h2c.cid = String(session && session._clientId);
        session.on('error', function(e) {{ globalThis.__h2c.err = e.message || String(e); }});
        session.on('response', function(info) {{ globalThis.__h2c.status = info.status; }});
        session.on('data', function(info) {{ globalThis.__h2c.body += info.data; }});
        session.on('end', function() {{ globalThis.__h2c.done = true; }});
        try {{
            var stream = session.request({{ ':method': 'GET', ':path': '/ping' }});
            stream.end();
            globalThis.__h2c.reqOk = true;
        }} catch(e) {{
            globalThis.__h2c.reqErr = e.message || String(e);
        }}
        "#
    ))
    .await
    .unwrap();

    for _ in 0..1000 {
        e.idle().await;
        let _ = e.pump_timers().await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        if e.eval_to_string("String(globalThis.__h2c.done)")
            .await
            .unwrap()
            == "true"
        {
            break;
        }
    }

    let err = e
        .eval_to_string("String(globalThis.__h2c.err)")
        .await
        .unwrap();
    assert_eq!(err, "null", "the JS h2 client reported an error: {err}");
    let status = e
        .eval_to_string("String(globalThis.__h2c.status)")
        .await
        .unwrap();
    assert_eq!(
        status, "200",
        "the JS h2 client must receive the real status"
    );
    let body = e
        .eval_to_string("String(globalThis.__h2c.body)")
        .await
        .unwrap();
    assert_eq!(body, "rust-h2:/ping");
}

// ── Permission gate ──────────────────────────────────────────────────────────

#[tokio::test]
async fn h2_listen_is_denied_without_a_net_grant() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var http2 = require('http2');
                var server = http2.createServer(function() {});
                server.listen(0, '127.0.0.1');
                var addr = server.address();
                return String(addr === null);
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "without a net grant the h2 server must never bind (address() === null)"
    );
}
