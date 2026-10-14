// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// fetch() must honour NODE_EXTRA_CA_CERTS, the same variable Node uses to add
// a private CA to the trust store. `NODE_EXTRA_CA_CERTS` is read once per
// process (the rustls client config is cached), so this lives in its own test
// binary where it is the only TLS user — setting the variable mid-process
// after a previous connection would be a no-op.
//
// Requires `openssl` on PATH (generates the cert and serves TLS). Skips with a
// message rather than failing when it isn't available.
//
// Run: cargo test -p vvva_js --test fetch_extra_ca

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

struct OpensslServer {
    child: Child,
    port: u16,
}

impl Drop for OpensslServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

fn openssl_available() -> bool {
    Command::new("openssl")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct TestCert {
    cert_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn gen_test_cert() -> TestCert {
    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    let status = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=127.0.0.1",
            "-addext",
            "subjectAltName=IP:127.0.0.1",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-keyout",
        ])
        .arg(&key_path)
        .arg("-out")
        .arg(&cert_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl req");
    assert!(status.success(), "openssl req (self-signed cert) failed");
    TestCert {
        cert_path,
        key_path,
        _dir: dir,
    }
}

fn spawn_s_server(cert: &TestCert) -> OpensslServer {
    let port = free_port();
    let child = Command::new("openssl")
        .args(["s_server", "-tls1_3", "-quiet", "-www"])
        .arg("-cert")
        .arg(&cert.cert_path)
        .arg("-key")
        .arg(&cert.key_path)
        .arg("-accept")
        .arg(port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn openssl s_server");

    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    OpensslServer { child, port }
}

/// Pump the engine until the fetch settles.
async fn pump_until(
    e: &mut JsEngine,
    done_js: &str,
    timeout: std::time::Duration,
) -> Option<String> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let v = e.eval_to_string(done_js).await.unwrap_or_default();
        if v != "false" && !v.is_empty() {
            return Some(v);
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_trusts_private_ca_from_node_extra_ca_certs() {
    if !openssl_available() {
        eprintln!("skipping: openssl not found on PATH");
        return;
    }
    let cert = gen_test_cert();
    let server = spawn_s_server(&cert);

    // SAFETY: this test binary contains this single TLS-using test, so no
    // other thread observes the environment concurrently.
    unsafe {
        std::env::set_var("NODE_EXTRA_CA_CERTS", &cert.cert_path);
    }

    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    let mut e = JsEngine::new(Arc::new(perms)).await.unwrap();

    e.eval(&format!(
        r#"
        globalThis.__extra = null;
        fetch('https://127.0.0.1:{port}/')
            .then(function(r) {{ return r.text().then(function(t) {{ return r.status; }}); }})
            .then(function(status) {{ globalThis.__extra = 'status:' + status; }})
            .catch(function(err) {{ globalThis.__extra = 'ERR:' + (err && err.message); }});
        "#,
        port = server.port,
    ))
    .await
    .unwrap();

    let result = pump_until(
        &mut e,
        "globalThis.__extra !== null ? globalThis.__extra : 'false'",
        std::time::Duration::from_secs(15),
    )
    .await
    .expect("fetch never settled");

    assert_eq!(
        result, "status:200",
        "fetch must trust the CA named by NODE_EXTRA_CA_CERTS"
    );

    unsafe {
        std::env::remove_var("NODE_EXTRA_CA_CERTS");
    }
    drop(server);
}
