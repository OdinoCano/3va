// Latency baseline for the POP3 transport, measured end-to-end against a
// minimal local daemon on loopback. Kept as its own test binary so each
// transport's engine runs in isolation.
//
// Run: cargo test -p vvva_js --test lat_pop3
//      cargo test -p vvva_js --features fips --test lat_pop3

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

/// Minimal POP3 daemon: greets, USER/PASS, then STAT. Measures the
/// connect+login+command round-trip in JS.
#[tokio::test]
async fn pop3_login_and_stat_latency_budget() {
    use std::io::{BufRead, BufReader, Write};

    const BUDGET_MS: u128 = 5_000;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        writer.write_all(b"+OK POP3 ready\r\n").unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap(); // USER
        writer.write_all(b"+OK\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap(); // PASS
        writer.write_all(b"+OK logged in\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap(); // STAT
        writer.write_all(b"+OK 2 320\r\n").unwrap();
    });

    let mut e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var pop3 = require('pop3');
        var t0 = Date.now();
        var client = new pop3.Client({{ host: '127.0.0.1', port: {port} }});
        client.connect(function() {{
            client.login('u', 'p', function() {{
                client.stat(function(err, info) {{
                    globalThis.__elapsed = Date.now() - t0;
                    globalThis.__done = true;
                }});
            }});
        }});
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
    assert!(ok, "POP3 chain never completed within 10s");
    let elapsed_ms: u128 = e
        .eval_to_string("String(globalThis.__elapsed)")
        .await
        .unwrap()
        .parse()
        .expect("elapsed numeric");
    assert!(
        elapsed_ms < BUDGET_MS,
        "POP3 connect+login+stat took {elapsed_ms} ms, over budget {BUDGET_MS} ms"
    );
    eprintln!("POP3 connect+login+stat: {elapsed_ms} ms");
}
