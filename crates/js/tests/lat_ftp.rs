// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Latency baseline for the FTP transport, measured end-to-end against a
// minimal local daemon on loopback. Kept as its own test binary so each
// transport's engine runs in isolation.
//
// Run: cargo test -p vvva_js --test lat_ftp
//      cargo test -p vvva_js --features fips --test lat_ftp

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

/// Minimal FTP daemon: 220 greet, USER/PASS, PASV+RETR round. Measures the
/// connect+login+download chain in JS.
#[tokio::test]
async fn ftp_login_and_get_latency_budget() {
    use std::io::{BufRead, BufReader, Write};

    const BUDGET_MS: u128 = 8_000;

    let control_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = control_listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let (control, _) = control_listener.accept().unwrap();
        let mut cw = control.try_clone().unwrap();
        let mut cr = BufReader::new(control);
        let mut line = String::new();

        cw.write_all(b"220 fake ftpd ready\r\n").unwrap();
        cr.read_line(&mut line).unwrap(); // USER
        tx.send(format!("saw:USER:{line}")).ok();
        cw.write_all(b"331 need password\r\n").unwrap();
        line.clear();
        cr.read_line(&mut line).unwrap(); // PASS
        tx.send(format!("saw:PASS:{line}")).ok();
        cw.write_all(b"230 logged in\r\n").unwrap();

        line.clear();
        cr.read_line(&mut line).unwrap(); // PASV
        tx.send(format!("saw:PASV:{line}")).ok();
        let data_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let data_port = data_listener.local_addr().unwrap().port();
        cw.write_all(
            format!(
                "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                data_port >> 8,
                data_port & 0xff
            )
            .as_bytes(),
        )
        .unwrap();
        line.clear();
        cr.read_line(&mut line).unwrap(); // RETR
        tx.send(format!("saw:RETR:{line}")).ok();
        cw.write_all(b"150 ok\r\n").unwrap();
        let (mut data, _) = data_listener.accept().unwrap();
        tx.send("data:accepted".to_string()).ok();
        data.write_all(b"hello from ftp").unwrap();
        drop(data);
        cw.write_all(b"226 transfer complete\r\n").unwrap();
        tx.send("sent:226".to_string()).ok();
    });

    let mut e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var ftp = require('ftp');
        var t0 = Date.now();
        var client = new ftp.Client();
        client.on('error', function(err) {{
            globalThis.__err = 'event:' + err.message;
        }});
        client.connect({{ host: '127.0.0.1', port: {port}, username: 'u', password: 'p' }});
        client.on('ready', function() {{
            globalThis.__ready = true;
            client.get('/file.txt', function(err, data) {{
                globalThis.__err = err ? ('get:' + err.message) : null;
                globalThis.__elapsed = Date.now() - t0;
                globalThis.__done = true;
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
        "(globalThis.__done === true)",
        std::time::Duration::from_secs(15),
    )
    .await;
    if !ok {
        let dbg = e
            .eval_to_string(
                "JSON.stringify({done: globalThis.__done, err: globalThis.__err ?? 'null', ready: globalThis.__ready ?? false})",
            )
            .await
            .unwrap_or_else(|err| format!("eval error: {err}"));
        let mut daemon_log = Vec::new();
        while let Ok(v) = rx.try_recv() {
            daemon_log.push(v);
        }
        panic!("FTP chain never completed within 15s — state: {dbg}; daemon saw {daemon_log:?}");
    }
    assert!(ok, "FTP chain never completed within 15s");
    let elapsed_ms: u128 = e
        .eval_to_string("String(globalThis.__elapsed)")
        .await
        .unwrap()
        .parse()
        .expect("elapsed numeric");
    assert!(
        elapsed_ms < BUDGET_MS,
        "FTP connect+login+get took {elapsed_ms} ms, over budget {BUDGET_MS} ms"
    );
    eprintln!("FTP connect+login+get: {elapsed_ms} ms");
}
