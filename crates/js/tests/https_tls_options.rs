// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// The TLS options of a request (`ca`, `cert`, `key`, `rejectUnauthorized`)
// have to behave like Node's, and skipping the certificate verification needs
// the user's permission.
//
//   - `rejectUnauthorized: false` is the TLS version of a plaintext
//     connection: it needs `--allow-insecure` (or a loopback host), like the
//     SSH host-key check. Without that, any dependency could turn it into
//     "accept any certificate".
//   - `ca` *replaces* the default roots: a request that names a private CA
//     trusts only that CA.
//
// Requires `openssl` on PATH (it generates the CAs and serves TLS). Skips
// with a message rather than failing when it isn't available.
//
// Run: cargo test -p vvva_js --test https_tls_options

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

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

/// A CA (certificate + key) generated with openssl.
struct Ca {
    dir: tempfile::TempDir,
    cert: std::path::PathBuf,
    key: std::path::PathBuf,
}

impl Ca {
    fn new(name: &str) -> Ca {
        let dir = tempfile::tempdir().unwrap();
        let cert = dir.path().join(format!("{name}-cert.pem"));
        let key = dir.path().join(format!("{name}-key.pem"));
        let status = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                key.to_str().unwrap(),
                "-out",
                cert.to_str().unwrap(),
                "-days",
                "2",
                "-subj",
                "/CN=3va-test-ca",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "openssl could not create the CA {name}");
        Ca { dir, cert, key }
    }

    /// A server certificate signed by this CA.
    fn sign_server(&self, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let cert = self.dir.path().join(format!("{name}-cert.pem"));
        let key = self.dir.path().join(format!("{name}-key.pem"));
        let csr = self.dir.path().join(format!("{name}.csr"));
        let ext = self.dir.path().join(format!("{name}.ext"));
        std::fs::write(
            &ext,
            "subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\n",
        )
        .unwrap();
        let status = Command::new("openssl")
            .args([
                "req",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                key.to_str().unwrap(),
                "-out",
                csr.to_str().unwrap(),
                "-subj",
                "/CN=localhost",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(
            status.success(),
            "openssl could not create the key of {name}"
        );
        let status = Command::new("openssl")
            .args([
                "x509",
                "-req",
                "-in",
                csr.to_str().unwrap(),
                "-CA",
                self.cert.to_str().unwrap(),
                "-CAkey",
                self.key.to_str().unwrap(),
                "-CAcreateserial",
                "-out",
                cert.to_str().unwrap(),
                "-days",
                "2",
                "-extfile",
                ext.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "openssl could not sign {name}");
        (cert, key)
    }
}

/// A TLS server that answers everything with `ok`.
struct TlsServer {
    child: Child,
    port: u16,
}

impl Drop for TlsServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve_tls(port: u16, cert: &std::path::Path, key: &std::path::Path) -> TlsServer {
    let script = format!(
        "const https=require('https'),fs=require('fs');\
         const s=https.createServer({{key:fs.readFileSync({key:?}),cert:fs.readFileSync({cert:?})}},\
         (req,res)=>{{res.writeHead(200,{{'content-type':'text/plain'}});res.end('ok');}});\
         s.listen({port},'127.0.0.1',()=>process.stdout.write('READY\\n'));",
        key = key.to_str().unwrap(),
        cert = cert.to_str().unwrap(),
    );
    let child = Command::new("node")
        .arg("-e")
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("node must be on PATH");
    // Wait for the server to listen.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    TlsServer { child, port }
}

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    perms.grant(Capability::Network("localhost".to_string()));
    perms.grant(Capability::Network("example.com".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

/// Make a request and return the error message (or "" when it worked).
async fn request(e: &mut JsEngine, url: &str, options: &str) -> String {
    let code = format!(
        "globalThis.__err='';globalThis.__ok=false;\
         const https=require('https');\
         const r=https.get({url}{options}, (res)=>{{globalThis.__ok=true;res.resume();}});\
         r.on('error',(err)=>{{globalThis.__err=String(err&&err.message||err);}});\
         r.on('close',()=>{{}});"
    );
    e.eval(&code).await.unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let done = e
            .eval_to_string("String(globalThis.__err!==''||globalThis.__ok===true)")
            .await
            .unwrap_or_default();
        if done == "true" {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return "timeout".to_string();
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
    e.eval_to_string("String(globalThis.__err)")
        .await
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn skipping_the_verification_needs_the_permission() {
    if !openssl_available() {
        eprintln!("skipping: openssl is not on PATH");
        return;
    }
    // A host that is not loopback: the permission is required.
    let mut e = engine_with_net().await;
    let err = request(
        &mut e,
        "'https://example.com/'",
        ", {rejectUnauthorized:false}",
    )
    .await;
    assert!(
        err.contains("--allow-insecure"),
        "the error must name the opt-in, got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skipping_the_verification_is_allowed_with_the_permission() {
    if !openssl_available() {
        eprintln!("skipping: openssl is not on PATH");
        return;
    }
    // With the permission the request goes on (and fails later, at the
    // network, not at the permission).
    vvva_permissions::set_allow_insecure(true);
    let mut e = engine_with_net().await;
    let err = request(
        &mut e,
        "'https://example.invalid/'",
        ", {rejectUnauthorized:false}",
    )
    .await;
    vvva_permissions::set_allow_insecure(false);
    assert!(
        !err.contains("--allow-insecure"),
        "the permission was given, so the request must not be refused: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skipping_the_verification_is_allowed_for_loopback() {
    if !openssl_available() {
        eprintln!("skipping: openssl is not on PATH");
        return;
    }
    // Loopback: the peer is on this machine, so nobody can intercept it.
    let ca = Ca::new("loopback-ca");
    let (cert, key) = ca.sign_server("loopback-server");
    let server = serve_tls(free_port(), &cert, &key);
    let mut e = engine_with_net().await;
    let err = request(
        &mut e,
        &format!("'https://localhost:{}/'", server.port),
        ", {rejectUnauthorized:false}",
    )
    .await;
    assert_eq!(err, "", "the request to a loopback host must work");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ca_option_replaces_the_default_roots() {
    if !openssl_available() {
        eprintln!("skipping: openssl is not on PATH");
        return;
    }
    // The server's certificate is signed by `ca_a`. The request names `ca_b`,
    // so the server's CA is not trusted — even though `NODE_EXTRA_CA_CERTS`
    // (the other CA) is in the trust store, and even though the system roots
    // are there: `ca` replaces them, as in Node.
    let ca_a = Ca::new("ca-a");
    let ca_b = Ca::new("ca-b");
    let (cert, key) = ca_a.sign_server("server");
    let server = serve_tls(free_port(), &cert, &key);

    // The other CA is in the trust store through NODE_EXTRA_CA_CERTS.
    unsafe {
        std::env::set_var("NODE_EXTRA_CA_CERTS", ca_a.cert.to_str().unwrap());
    }

    // The right CA: the request works.
    let mut e = engine_with_net().await;
    let err = request(
        &mut e,
        &format!("'https://localhost:{}/'", server.port),
        &format!(", {{ca: {}}}", json_path(&ca_a.cert)),
    )
    .await;
    assert_eq!(err, "", "the request with the right CA must work");

    // The wrong CA: the roots were replaced, so the server is not trusted.
    let mut e2 = engine_with_net().await;
    let err = request(
        &mut e2,
        &format!("'https://localhost:{}/'", server.port),
        &format!(", {{ca: {}}}", json_path(&ca_b.cert)),
    )
    .await;
    assert!(
        !err.is_empty(),
        "with `ca` the default roots are replaced: the server's CA is not trusted any more"
    );
    unsafe {
        std::env::remove_var("NODE_EXTRA_CA_CERTS");
    }
}

/// A JSON string with the contents of a file.
fn json_path(path: &std::path::Path) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
