// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// https.createServer({ key, cert }) must be a real TLS listener. It used to
// return a plain HTTP server and ignore the key and certificate, so an app
// that asked for HTTPS served cleartext on that port.
//
// The client side is a genuine third-party TLS implementation (`openssl
// s_client`, verifying the certificate), like tests/pq_tls.rs.
//
// Run: cargo test -p vvva_js --test https_server

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

struct TestCert {
    cert_path: std::path::PathBuf,
    cert_pem: String,
    key_pem: String,
    _dir: tempfile::TempDir,
}

/// A throwaway self-signed pair, generated per test run (never committed).
fn gen_test_cert() -> TestCert {
    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    let status = std::process::Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "ec",
            "-pkeyopt",
            "ec_paramgen_curve:prime256v1",
        ])
        .args(["-nodes", "-days", "1", "-subj", "/CN=127.0.0.1"])
        .args(["-addext", "subjectAltName=IP:127.0.0.1", "-keyout"])
        .arg(&key_path)
        .arg("-out")
        .arg(&cert_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl req");
    assert!(status.success(), "openssl req (self-signed cert) failed");
    TestCert {
        cert_pem: std::fs::read_to_string(&cert_path).unwrap(),
        key_pem: std::fs::read_to_string(&key_path).unwrap(),
        cert_path,
        _dir: dir,
    }
}

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

async fn wait_for_port(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_ok() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "port {port} never came up"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn drive_until<T>(e: &mut JsEngine, client: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(client);
    loop {
        tokio::select! {
            _ = async { e.idle().await; let _ = e.run_event_loop().await; tokio::task::yield_now().await; } => {}
            result = &mut client => return result,
        }
    }
}

async fn start_https_server(e: &mut JsEngine, cert: &TestCert, port: u16) {
    e.eval_to_string(&format!(
        r#"
        var https = require('https');
        var _server = https.createServer({{ key: {key:?}, cert: {cert:?} }}, function(req, res) {{
            res.writeHead(200, {{ 'Content-Type': 'text/plain' }});
            res.end('secure=' + String(req.socket.encrypted === true));
        }});
        _server.listen({port}, '127.0.0.1');
        'started'
        "#,
        key = cert.key_pem,
        cert = cert.cert_pem,
    ))
    .await
    .unwrap();
    wait_for_port(port).await;
}

/// One HTTP/1.1 request through `openssl s_client`, which verifies the
/// server certificate against `ca` and fails the handshake otherwise.
async fn openssl_get(port: u16, ca: &std::path::Path) -> String {
    let mut child = tokio::process::Command::new("openssl")
        .args(["s_client", "-quiet", "-verify_return_error", "-CAfile"])
        .arg(ca)
        .args(["-connect", &format!("127.0.0.1:{port}")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("openssl s_client");
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    let mut stdout = child.stdout.take().unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(10), stdout.read_to_string(&mut out)).await;
    drop(stdin);
    let _ = child.kill().await;
    out
}

#[tokio::test]
async fn https_server_serves_over_tls() {
    let cert = gen_test_cert();
    let port = free_port();
    let mut e = engine_with_net().await;
    start_https_server(&mut e, &cert, port).await;

    let resp = drive_until(&mut e, openssl_get(port, &cert.cert_path)).await;
    assert!(
        resp.starts_with("HTTP/1.1 200 OK"),
        "response over TLS:\n{resp}"
    );
    assert!(
        resp.ends_with("secure=true"),
        "req.socket.encrypted:\n{resp}"
    );
}

#[tokio::test]
async fn https_port_does_not_answer_plaintext_http() {
    let cert = gen_test_cert();
    let port = free_port();
    let mut e = engine_with_net().await;
    start_https_server(&mut e, &cert, port).await;

    let plain = async {
        let mut s = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        s.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut buf)).await;
        String::from_utf8_lossy(&buf).to_string()
    };
    let resp = drive_until(&mut e, plain).await;
    assert!(
        !resp.contains("HTTP/1.1") && !resp.contains("secure="),
        "a cleartext request must not get an HTTP response from an https server:\n{resp}"
    );
}

#[tokio::test]
async fn https_create_server_refuses_what_it_cannot_honour() {
    let mut e = engine_with_net().await;
    let r = e
        .eval_to_string(
            r#"
            var https = require('https');
            var codes = [];
            [{ key: 'not pem', cert: 'not pem' }, { pfx: 'x' }, { key: 'k', cert: 'c', requestCert: true }]
              .forEach(function (o) {
                try { https.createServer(o, function () {}); codes.push('created'); }
                catch (err) { codes.push(err.code); }
              });
            codes.join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r,
        "ERR_INVALID_ARG_VALUE,ERR_FEATURE_UNAVAILABLE_ON_PLATFORM,ERR_FEATURE_UNAVAILABLE_ON_PLATFORM"
    );
}

// Like Node, an https server can be created without a certificate; what it
// must never do is listen and answer in cleartext instead.
#[tokio::test]
async fn https_server_without_key_and_cert_fails_to_listen_not_to_serve_plaintext() {
    let port = free_port();
    let mut e = engine_with_net().await;
    e.eval_to_string(&format!(
        r#"
        var https = require('https');
        globalThis.__listenError = 'none';
        var srv = https.createServer(function (req, res) {{ res.end('plaintext!'); }});
        srv.on('error', function (err) {{ globalThis.__listenError = err.code; }});
        srv.listen({port}, '127.0.0.1');
        typeof srv.listen
        "#
    ))
    .await
    .unwrap();
    // Not `run_event_loop().await`: it only returns once no HTTP listener is
    // open anywhere in the process, and the other tests in this binary leave
    // theirs open. Drive the loop for a bounded time instead.
    drive_until(&mut e, tokio::time::sleep(Duration::from_millis(300))).await;
    assert_eq!(
        e.eval_to_string("globalThis.__listenError").await.unwrap(),
        "ERR_MISSING_ARGS"
    );
    assert!(
        std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_ok(),
        "nothing may be listening on the port"
    );
}

/// True if the system `openssl` can offer the hybrid post-quantum group
/// (OpenSSL >= 3.5), as in tests/pq_tls.rs.
fn openssl_supports_pq() -> bool {
    let Ok(out) = std::process::Command::new("openssl")
        .arg("version")
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(ver) = text.split_whitespace().nth(1) else {
        return false;
    };
    let mut parts = ver.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0)) >= (3, 5)
}

// The server side of TLS uses the same provider as the client side
// (rustls built with `prefer-post-quantum`): with a client that offers it,
// the key exchange is the X25519 + ML-KEM-768 hybrid.
#[tokio::test]
async fn https_server_negotiates_hybrid_post_quantum_key_exchange() {
    if !openssl_supports_pq() {
        eprintln!("skipping: system openssl is older than 3.5 (no X25519MLKEM768)");
        return;
    }
    let cert = gen_test_cert();
    let port = free_port();
    let mut e = engine_with_net().await;
    start_https_server(&mut e, &cert, port).await;

    let handshake = async {
        let out = tokio::process::Command::new("openssl")
            .args(["s_client", "-CAfile"])
            .arg(&cert.cert_path)
            .args(["-connect", &format!("127.0.0.1:{port}")])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .await
            .expect("openssl s_client");
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let report = drive_until(&mut e, handshake).await;
    assert!(
        report.contains("Negotiated TLS1.3 group: X25519MLKEM768"),
        "expected the hybrid PQ group:\n{report}"
    );
    assert!(report.contains("Verification: OK"), "{report}");
}
