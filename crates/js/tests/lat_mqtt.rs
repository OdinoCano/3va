// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Latency baseline for the MQTT transport, measured end-to-end against a
// minimal local broker on loopback. Kept as its own test binary so each
// transport's engine runs in isolation.
//
// Run: cargo test -p vvva_js --test lat_mqtt
//      cargo test -p vvva_js --features fips --test lat_mqtt

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

fn read_mqtt_packet(r: &mut impl std::io::Read) -> (u8, Vec<u8>) {
    let mut type_byte = [0u8; 1];
    r.read_exact(&mut type_byte).unwrap();
    let mut multiplier: u32 = 1;
    let mut remaining_len: u32 = 0;
    loop {
        let mut b = [0u8; 1];
        r.read_exact(&mut b).unwrap();
        remaining_len += (b[0] & 0x7f) as u32 * multiplier;
        multiplier *= 128;
        if b[0] & 0x80 == 0 {
            break;
        }
    }
    let mut payload = vec![0u8; remaining_len as usize];
    r.read_exact(&mut payload).unwrap();
    (type_byte[0], payload)
}

fn encode_remaining_length(len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut x = len;
    loop {
        let mut byte = (x % 128) as u8;
        x /= 128;
        if x > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if x == 0 {
            break;
        }
    }
    out
}

/// Minimal broker: CONNECT→CONNACK, SUBSCRIBE→SUBACK, then a real PUBLISH.
/// Measures the full connect+subscribe+publish round-trip in JS.
#[tokio::test]
async fn mqtt_broker_round_trip_latency_budget() {
    use std::io::Write;

    const BUDGET_MS: u128 = 5_000;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let (packet_type, _) = read_mqtt_packet(&mut stream); // CONNECT
        assert_eq!(packet_type & 0xf0, 0x10);
        stream.write_all(&[0x20, 0x02, 0x00, 0x00]).unwrap();

        let (packet_type, _) = read_mqtt_packet(&mut stream); // SUBSCRIBE
        assert_eq!(packet_type & 0xf0, 0x80);
        stream.write_all(&[0x90, 0x03, 0x00, 0x01, 0x00]).unwrap();

        let topic = b"t/lat";
        let payload = b"hi";
        let mut body = Vec::new();
        body.push((topic.len() >> 8) as u8);
        body.push((topic.len() & 0xff) as u8);
        body.extend_from_slice(topic);
        body.extend_from_slice(payload);
        let mut packet = vec![0x30];
        packet.extend(encode_remaining_length(body.len()));
        packet.extend(body);
        stream.write_all(&packet).unwrap();
    });

    let mut e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var mqtt = require('mqtt');
        var t0 = Date.now();
        var client = mqtt.connect({{ host: '127.0.0.1', port: {port} }});
        client.on('message', function(topic, payload) {{
            globalThis.__elapsed = Date.now() - t0;
            globalThis.__done = true;
        }});
        client.on('connected', function() {{ client.subscribe('t/lat'); }});
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
    assert!(ok, "MQTT round-trip never completed within 10s");
    let elapsed_ms: u128 = e
        .eval_to_string("String(globalThis.__elapsed)")
        .await
        .unwrap()
        .parse()
        .expect("elapsed numeric");
    assert!(
        elapsed_ms < BUDGET_MS,
        "MQTT connect+subscribe+publish took {elapsed_ms} ms, over budget {BUDGET_MS} ms"
    );
    eprintln!("MQTT connect→publish RTT: {elapsed_ms} ms");
}
