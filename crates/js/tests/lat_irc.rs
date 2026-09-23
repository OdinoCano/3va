// Latency baseline for the IRC transport, measured end-to-end against a
// minimal local daemon on loopback. Kept as its own test binary so each
// transport's engine runs in isolation.
//
// Run: cargo test -p vvva_js --test lat_irc
//      cargo test -p vvva_js --features fips --test lat_irc

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net(host: &str) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::Network(host.to_string()));
    JsEngine::new(Arc::new(state)).await.unwrap()
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

/// Minimal IRC daemon: waits for NICK/USER registration, sends 001, PINGs the
/// client and reads back the PONG. Measures registration → PONG round-trip.
#[tokio::test]
async fn irc_registration_and_pong_latency_budget() {
    use std::io::{BufRead, BufReader, Write};

    const BUDGET_MS: u128 = 5_000;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut saw_user = false;
        while !saw_user {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                return;
            }
            if line.starts_with("USER ") {
                saw_user = true;
            }
        }
        writer.write_all(b":t 001 nick :Welcome\r\n").unwrap();
        writer.write_all(b"PING :tok\r\n").unwrap();
        // Read the client's PONG so the daemon's join() correlates with it.
        let mut pong = String::new();
        let _ = reader.read_line(&mut pong);
    });

    let mut e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var irc = require('irc');
        var t0 = Date.now();
        var client = new irc.Client({{ host: '127.0.0.1', port: {port}, nick: 'nick' }});
        client.on('ping', function() {{
            globalThis.__elapsed = Date.now() - t0;
            globalThis.__done = true;
        }});
        client.connect();
        "#
        )
        .as_str(),
    )
    .await
    .unwrap();

    let ok = pump_until(
        &mut e,
        "globalThis.__done === true",
        std::time::Duration::from_secs(10),
    )
    .await;
    assert!(ok, "IRC registration+PONG never completed within 10s");
    let _ = tokio::task::spawn_blocking(move || server.join()).await;
    let elapsed_ms: u128 = e
        .eval_to_string("String(globalThis.__elapsed)")
        .await
        .unwrap()
        .parse()
        .expect("elapsed numeric");
    assert!(
        elapsed_ms < BUDGET_MS,
        "IRC registration+PONG took {elapsed_ms} ms, over budget {BUDGET_MS} ms"
    );
    eprintln!("IRC register→PONG: {elapsed_ms} ms");
}
