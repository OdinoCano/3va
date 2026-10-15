// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Top-level `fetch()` over TLS:
//
//   * a certificate that cannot be verified must make the promise *reject* —
//     the process must not silently exit 0 with the fetch neither resolved nor
//     rejected;
//   * while a `fetch()` is still in flight the event loop must keep running;
//   * an unhandled rejection (the default when the script never attaches a
//     `.catch`) makes the process exit non-zero, as Node does.
//
// The trust store is process-wide and cached, so this lives in its own test
// binary (no other TLS user); it never sets `NODE_EXTRA_CA_CERTS`.
//
// Requires `openssl` on PATH; skips (with a message) when it is missing.
//
// Run: cargo test -p vvva_js --test fetch_toplevel_tls

use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

struct OpensslServer {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
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

/// A self-signed leaf for 127.0.0.1 with no CA in the trust store: any TLS
/// client must reject it (unknown issuer).
fn spawn_untrusted_tls() -> Option<OpensslServer> {
    if !openssl_available() {
        eprintln!("skipping: openssl not found on PATH");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
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
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl req");
    assert!(status.success(), "openssl req failed");

    let port = free_port();
    let child = Command::new("openssl")
        .args(["s_server", "-tls1_3", "-quiet", "-www"])
        .arg("-cert")
        .arg(&cert)
        .arg("-key")
        .arg(&key)
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
    Some(OpensslServer {
        child,
        port,
        _dir: dir,
    })
}

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

async fn pump_until(e: &mut JsEngine, done_js: &str, timeout: Duration) -> Option<String> {
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

/// A top-level fetch to an untrusted certificate must reject, not hang and not
/// let the process finish cleanly with the promise unsettled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn top_level_fetch_to_untrusted_cert_rejects() {
    let Some(server) = spawn_untrusted_tls() else {
        return;
    };
    let mut e = engine_with_net().await;
    e.eval(&format!(
        r#"
        globalThis.__tl = null;
        setTimeout(function() {{}}, 2000);
        fetch('https://127.0.0.1:{port}/').then(function(r) {{
            globalThis.__tl = 'resolved:' + r.status;
        }}).catch(function(err) {{
            globalThis.__tl = 'rejected:' + (err && err.message);
        }});
        "#,
        port = server.port,
    ))
    .await
    .unwrap();

    let result = pump_until(
        &mut e,
        "globalThis.__tl !== null ? globalThis.__tl : 'false'",
        Duration::from_secs(15),
    )
    .await
    .expect("top-level fetch neither resolved nor rejected");

    assert!(
        result.starts_with("rejected:"),
        "an untrusted certificate must reject, got: {result}"
    );
    assert!(
        result.to_lowercase().contains("certificate") || result.to_lowercase().contains("tls"),
        "the rejection must carry the TLS error, got: {result}"
    );
}

/// A `fetch()` still in flight is pending work: the event loop must not stop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_fetch_keeps_the_event_loop_alive() {
    // A TCP server that accepts and then never speaks: the TLS handshake hangs.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _server = tokio::spawn(async move {
        while let Ok((s, _)) = listener.accept().await {
            // Hold the socket open, send nothing.
            tokio::spawn(async move {
                let _s = s;
                tokio::time::sleep(Duration::from_secs(300)).await;
            });
        }
    });

    let mut e = engine_with_net().await;
    e.eval(&format!(
        "fetch('https://127.0.0.1:{port}/').then(function(){{}}, function(){{}});",
        port = port,
    ))
    .await
    .unwrap();

    let stopped = tokio::time::timeout(Duration::from_millis(500), e.run_event_loop())
        .await
        .is_ok();
    assert!(
        !stopped,
        "the event loop stopped while a fetch was still in flight"
    );
}

/// An unhandled rejection makes the process exit non-zero, as in Node.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unhandled_rejection_exits_nonzero() {
    let Some(server) = spawn_untrusted_tls() else {
        return;
    };
    let mut e = engine_with_net().await;
    // No `.catch`: this is exactly the run whose TLS error used to be reported
    // and then swallowed, with the process still exiting 0.
    e.eval(&format!(
        "fetch('https://127.0.0.1:{port}/');",
        port = server.port
    ))
    .await
    .unwrap();

    // Drain the loop: it keeps running until the pending fetch settles, then
    // returns on its own.
    for _ in 0..600 {
        if tokio::time::timeout(Duration::from_millis(50), e.run_event_loop())
            .await
            .is_ok()
        {
            break;
        }
    }

    assert!(
        vvva_js::rejection_tracker::had_unhandled(),
        "the rejection must be reported as unhandled"
    );
    let code = e.finish().await.unwrap();
    assert_eq!(code, 1, "an unhandled rejection must exit non-zero");
}
