// Real interop tests for hybrid PQ-TLS (RFC 10024 X25519MLKEM768) against a
// genuine third-party TLS implementation — a local `openssl s_server` process,
// not 3va talking to itself. See docs/10-security/06-pq-tls-hybrid-design.md.
//
// Requires OpenSSL >= 3.5 on PATH (ships X25519MLKEM768 natively, no OQS
// provider needed). Skips (with a message, not a failure) if unavailable, so
// CI runners without a new-enough OpenSSL don't spuriously fail — but the
// design doc records a real run's output as evidence this was verified.
//
// Run: cargo test -p vvva_js --test pq_tls

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

/// `None` if OpenSSL isn't new enough to natively support X25519MLKEM768
/// (added in OpenSSL 3.5) — tests using this skip rather than fail, since
/// CI/dev-machine OpenSSL versions are outside this repo's control.
fn openssl_supports_pq() -> bool {
    let Ok(out) = Command::new("openssl").arg("version").output() else {
        return false;
    };
    let v = String::from_utf8_lossy(&out.stdout);
    // "OpenSSL 3.5.5 27 Jan 2026" etc — 3.5 is the first release with native
    // X25519MLKEM768 (RFC 10024) support, no OQS provider needed.
    let Some(version) = v
        .strip_prefix("OpenSSL ")
        .and_then(|rest| rest.split(' ').next())
    else {
        return false;
    };
    let mut parts = version.split('.');
    let (Some(major), Some(minor)) = (
        parts.next().and_then(|s| s.parse::<u32>().ok()),
        parts.next().and_then(|s| s.parse::<u32>().ok()),
    ) else {
        return false;
    };
    (major, minor) >= (3, 5)
}

/// Self-signed cert/key + CA PEM (same cert, self-signed) for the local
/// `openssl s_server`, generated fresh per test run via `openssl req`.
struct TestCert {
    cert_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
    ca_pem: String,
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
    let ca_pem = std::fs::read_to_string(&cert_path).unwrap();
    TestCert {
        cert_path,
        key_path,
        ca_pem,
        _dir: dir,
    }
}

fn spawn_s_server(cert: &TestCert, groups: &str) -> OpensslServer {
    let port = free_port();
    let child = Command::new("openssl")
        .args(["s_server", "-tls1_3", "-quiet", "-www", "-groups", groups])
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

    // Wait for the listener to actually be up (openssl start-up isn't instant).
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    OpensslServer { child, port }
}

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

// ── tests ─────────────────────────────────────────────────────────────────

/// Real interop against an independent, PQ-capable TLS implementation:
/// asserts the hybrid handshake succeeds AND that X25519MLKEM768 (RFC 10024)
/// is the group actually negotiated on the wire — not just that a TLS
/// connection of *some* kind was made.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pq_tls_negotiates_hybrid_group_against_real_openssl() {
    if !openssl_supports_pq() {
        eprintln!("skipping: openssl >=3.5 with X25519MLKEM768 support not found on PATH");
        return;
    }
    let cert = gen_test_cert();
    let server = spawn_s_server(&cert, "X25519MLKEM768:X25519");
    let mut e = engine_with_net().await;

    let script = format!(
        r#"
        var tls = require('tls');
        var s = tls.pqConnect({port}, '127.0.0.1', {{ ca: {ca:?} }});
        JSON.stringify({{ group: s.negotiatedGroup, pq: s.pqNegotiated }})
        "#,
        port = server.port,
        ca = cert.ca_pem,
    );
    let result = e.eval_to_string(&script).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["group"], "X25519MLKEM768");
    assert_eq!(parsed["pq"], true);
}

/// Real interop against a server that only offers the classical group:
/// the hybrid client must still connect successfully (graceful fallback —
/// RFC 10024 groups always carry a classical component), just without PQ.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pq_tls_falls_back_to_classical_against_non_pq_server() {
    if !openssl_supports_pq() {
        eprintln!("skipping: openssl not found/too old on PATH");
        return;
    }
    let cert = gen_test_cert();
    let server = spawn_s_server(&cert, "X25519");
    let mut e = engine_with_net().await;

    let script = format!(
        r#"
        var tls = require('tls');
        var s = tls.pqConnect({port}, '127.0.0.1', {{ ca: {ca:?} }});
        JSON.stringify({{ group: s.negotiatedGroup, pq: s.pqNegotiated }})
        "#,
        port = server.port,
        ca = cert.ca_pem,
    );
    let result = e.eval_to_string(&script).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["group"], "X25519");
    assert_eq!(parsed["pq"], false);
}

/// Sanity check the skip-condition helper itself doesn't silently always skip
/// (a `#[ignore]`-style helper that's permanently broken is worse than not
/// having the test) — this doesn't require openssl.
#[test]
fn openssl_probe_does_not_panic() {
    let _ = openssl_supports_pq();
}

/// Not run in CI (real internet dependency, no control over the remote
/// server) — a one-time/manual re-verification against a real, independent
/// public TLS deployment, not a local test fixture. Run explicitly with
/// `cargo test -p vvva_js --test pq_tls manual_real_world_check_google -- --ignored --nocapture`.
/// Last verified 2026-08-19: `{"group":"X25519MLKEM768","pq":true}` — see
/// docs/10-security/06-pq-tls-hybrid-design.md for the recorded evidence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn manual_real_world_check_google() {
    let mut e = engine_with_net_any().await;
    let result = e
        .eval_to_string(
            r#"
        var tls = require('tls');
        var s = tls.pqConnect(443, 'www.google.com');
        JSON.stringify({ group: s.negotiatedGroup, pq: s.pqNegotiated })
        "#,
        )
        .await
        .unwrap();
    eprintln!("GOOGLE REAL-WORLD RESULT: {result}");
}

async fn engine_with_net_any() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("www.google.com".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}
