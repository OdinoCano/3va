// Latency baseline for the TLS transport (tls.pqConnect, gated on OpenSSL >=
// 3.5), measured end-to-end against a real `openssl s_server` on loopback.
// Kept as its own test binary so each transport's engine runs in isolation.
//
// Run: cargo test -p vvva_js --test lat_tls
//      cargo test -p vvva_js --features fips --test lat_tls

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

/// Pump the engine's JS timers + async spawner until `done_js` (an expression
/// over globalThis) reads "true", or the deadline lapses.
///
/// The whole loop is wrapped in `tokio::time::timeout` because
/// `run_event_loop()` can block for a while while a control connection is open
/// and its EAGAIN-backoff poll keeps rescheduling itself.
async fn pump_until(e: &mut JsEngine, done_js: &str, deadline: std::time::Duration) -> bool {
    tokio::time::timeout(deadline, async {
        loop {
            tokio::select! {
                _ = e.idle() => {},
                _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
            }
            let _ = e.run_event_loop().await;
            tokio::task::yield_now().await;
            if e.eval_to_string(done_js).await.unwrap_or_default() == "true" {
                return true;
            }
        }
    })
    .await
    .is_ok()
        && e.eval_to_string(done_js).await.unwrap_or_default() == "true"
}

struct OpensslServer {
    child: std::process::Child,
    port: u16,
}

impl Drop for OpensslServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// OpenSSL >= 3.5 ships X25519MLKEM768 natively. Gated just like pq_tls.rs:
/// skip (not fail) when the runner's OpenSSL is too old.
fn openssl_supports_pq() -> bool {
    use std::process::Command;
    let Ok(out) = Command::new("openssl").arg("version").output() else {
        return false;
    };
    let v = String::from_utf8_lossy(&out.stdout);
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

/// Self-signed cert/key for a local `openssl s_server`.
struct TestCert {
    cert_path: std::path::PathBuf,
    key_path: std::path::PathBuf,
    ca_pem: String,
    _dir: tempfile::TempDir,
}

fn gen_test_cert() -> TestCert {
    use std::process::{Command, Stdio};
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
    assert!(status.success(), "openssl req failed");
    let ca_pem = std::fs::read_to_string(&cert_path).unwrap();
    TestCert {
        cert_path,
        key_path,
        ca_pem,
        _dir: dir,
    }
}

fn spawn_s_server(cert: &TestCert) -> OpensslServer {
    use std::process::{Command, Stdio};
    let port = free_port();
    let child = Command::new("openssl")
        .args([
            "s_server",
            "-tls1_3",
            "-quiet",
            "-www",
            "-groups",
            "X25519MLKEM768:X25519",
        ])
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
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    OpensslServer { child, port }
}

/// TLS handshake latency measured in JS around the blocking pqConnect: TCP
/// connect + full TLS 1.3 handshake (hybrid X25519MLKEM768 negotiation in the
/// non-FIPS build) against a real `openssl s_server`. Asserted as a CI-safe
/// budget dialed against loopback.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_handshake_latency_budget() {
    if !openssl_supports_pq() {
        eprintln!("skipping: openssl >=3.5 with X25519MLKEM768 not found on PATH");
        return;
    }
    const N: usize = 10;
    const MAX_MEAN_MS: f64 = 1000.0;
    const MAX_P99_MS: f64 = 3000.0;

    let cert = gen_test_cert();
    let server = spawn_s_server(&cert);
    let mut e = engine_with_net("127.0.0.1").await;

    let script = format!(
        r#"
        var tls = require('tls');
        globalThis.__samples = [];
        globalThis.__done = false;
        var step = {N};
        function run() {{
            var t0 = Date.now();
            var s = tls.pqConnect({port}, '127.0.0.1', {{ ca: {ca:?} }});
            var t1 = Date.now();
            if (!s.pqNegotiated && !(s.negotiatedGroup)) throw new Error('handshake failed');
            s.destroy();
            globalThis.__samples.push(t1 - t0);
            step--;
            if (step > 0) setTimeout(run, 1);
            else globalThis.__done = true;
        }}
        setTimeout(run, 0);
        "#,
        N = N,
        port = server.port,
        ca = cert.ca_pem,
    );
    e.eval_to_string(&script).await.unwrap();

    let ok = pump_until(
        &mut e,
        "globalThis.__done === true",
        std::time::Duration::from_secs(60),
    )
    .await;
    assert!(ok, "TLS handshakes never completed within 60s");
    let r = e
        .eval_to_string("JSON.stringify(globalThis.__samples)")
        .await
        .unwrap();
    let mut samples: Vec<f64> = serde_json::from_str(&r).expect("valid latency samples");
    assert_eq!(samples.len(), N, "all handshakes must complete");
    samples.sort_by(|a, b| a.total_cmp(b));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let p99 = samples[(samples.len() as f64 * 0.99).floor() as usize];
    let stats = serde_json::json!({
        "n": samples.len(),
        "min": samples[0],
        "mean": mean,
        "p95": samples[(samples.len() as f64 * 0.95).floor() as usize],
        "p99": p99,
        "max": samples[samples.len() - 1],
    });

    assert!(
        mean <= MAX_MEAN_MS,
        "mean TLS handshake {mean:.1} ms exceeds {MAX_MEAN_MS} ms (stats: {stats})"
    );
    assert!(
        p99 <= MAX_P99_MS,
        "p99 TLS handshake {p99:.1} ms exceeds {MAX_P99_MS} ms (stats: {stats})"
    );
    eprintln!("TLS handshake latency (n={N}): {stats}");
}
