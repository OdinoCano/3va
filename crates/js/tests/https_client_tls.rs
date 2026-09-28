// Regression tests: https.request/get must speak TLS, and EventSource must
// honour --allow-net like fetch(). If the https fix regresses, the first test
// hangs (the plaintext client never finishes) rather than failing fast.
// Run: cargo test -p vvva_js --test https_client_tls

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

#[tokio::test(flavor = "multi_thread")]
async fn https_get_sends_a_tls_client_hello_not_plaintext() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // Record the first bytes, then answer with a minimal HTTP response and
    // close: a plaintext client finishes, a TLS client fails its handshake.
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut head = [0u8; 3];
        conn.read_exact(&mut head).unwrap();
        let _ =
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        head
    });

    let state = PermissionState::new();
    state.grant(Capability::Network("127.0.0.1".to_string()));
    let mut e = JsEngine::new(Arc::new(state)).await.unwrap();
    e.eval(&format!(
        "require('https').get({{ hostname: '127.0.0.1', port: {port}, path: '/?token=secret' }}).on('error', function() {{}});"
    ))
    .await
    .unwrap();

    let head = server.join().unwrap();
    assert_ne!(&head, b"GET", "https.get sent the request in plaintext");
    // TLS record header: handshake (0x16), protocol major version 3.
    assert_eq!(
        &head[..2],
        &[0x16, 0x03],
        "expected a TLS ClientHello, got {head:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn event_source_requires_allow_net() {
    let mut e = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    e.eval(
        "globalThis.__err = 'no error'; \
         try { new EventSource('https://example.com/stream'); } \
         catch (err) { globalThis.__err = String((err && err.message) || err); }",
    )
    .await
    .unwrap();
    let err = e.eval_to_string("globalThis.__err").await.unwrap();
    assert!(err.contains("--allow-net=example.com:443"), "got {err}");
}
