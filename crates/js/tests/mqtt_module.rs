// Tests for the MQTT builtin.
// Run: cargo test -p vvva_js --test mqtt_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

#[allow(dead_code)]
async fn engine_with_net(host: &str) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::Network(host.to_string()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

async fn engine_no_net() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

/// Reads one MQTT control packet (fixed header + variable-length remaining
/// length + payload) off a blocking stream. Returns (packet_type, payload).
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

/// Starts a minimal local MQTT broker: accepts the CONNECT, replies CONNACK,
/// accepts a SUBSCRIBE, replies SUBACK, then pushes a real PUBLISH with a
/// known topic/payload — this is exactly the packet whose parsing the
/// `_handlePublish` off-by-one bug used to corrupt.
fn start_fake_broker() -> u16 {
    use std::io::Write;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let (packet_type, _) = read_mqtt_packet(&mut stream); // CONNECT
        assert_eq!(packet_type & 0xf0, 0x10);
        stream.write_all(&[0x20, 0x02, 0x00, 0x00]).unwrap(); // CONNACK, rc=0

        let (packet_type, _) = read_mqtt_packet(&mut stream); // SUBSCRIBE
        assert_eq!(packet_type & 0xf0, 0x80);
        stream.write_all(&[0x90, 0x03, 0x00, 0x01, 0x00]).unwrap(); // SUBACK

        let topic = b"test/topic";
        let payload = b"hello mqtt";
        let mut body = Vec::new();
        body.push((topic.len() >> 8) as u8);
        body.push((topic.len() & 0xff) as u8);
        body.extend_from_slice(topic);
        body.extend_from_slice(payload);
        let mut packet = vec![0x30]; // PUBLISH, QoS 0
        packet.extend(encode_remaining_length(body.len()));
        packet.extend(body);
        stream.write_all(&packet).unwrap();
    });

    port
}

#[tokio::test]
async fn mqtt_real_publish_parses_topic_and_payload_correctly() {
    let port = start_fake_broker();
    let e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var gotTopic = null, gotPayload = null;
        var mqtt = require('mqtt');
        var client = mqtt.connect({{ host: '127.0.0.1', port: {port} }});
        client.on('message', function(topic, payload) {{ gotTopic = topic; gotPayload = payload; }});
        client.on('connected', function() {{ client.subscribe('test/topic'); }});
        "#
        )
        .as_str(),
    )
    .await
    .unwrap();

    let mut got = false;
    for _ in 0..300 {
        tokio::select! {
            _ = e.idle() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(gotTopic !== null)").await.unwrap() == "true" {
            got = true;
            break;
        }
    }
    assert!(got, "client never parsed the real PUBLISH packet");

    let topic = e.eval_to_string("String(gotTopic)").await.unwrap();
    let payload = e.eval_to_string("String(gotPayload)").await.unwrap();
    // Confirms the _handlePublish off-by-one is gone: before the fix, the
    // spurious leading offset++ corrupted both fields into garbage.
    assert_eq!(topic, "test/topic");
    assert_eq!(payload, "hello mqtt");
}

// ── API shape ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mqtt_global_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof mqtt === 'object')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn mqtt_client_constructor_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof mqtt.Client === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn mqtt_client_has_expected_methods() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new mqtt.Client({ host: 'example.com' });
                var methods = ['connect', 'disconnect', 'subscribe', 'unsubscribe',
                              'publish', 'end', 'ack'];
                var missing = [];
                for (var i = 0; i < methods.length; i++) {
                    if (typeof client[methods[i]] !== 'function') {
                        missing.push(methods[i]);
                    }
                }
                return missing.length === 0 ? 'true' : 'missing: ' + missing.join(',');
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn mqtt_client_has_event_emitter() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new mqtt.Client({ host: 'example.com' });
                return String(
                    typeof client.on === 'function' &&
                    typeof client.off === 'function' &&
                    typeof client.emit === 'function'
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── Permission enforcement ───────────────────────────────────────────────────

#[tokio::test]
async fn mqtt_connect_blocked_without_net_grant() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                try {
                    var client = new mqtt.Client({ host: 'example.com', port: 1883 });
                    return 'created';
                } catch(e) {
                    return 'threw:' + e.code;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "created");
}

#[tokio::test]
async fn mqtt_constructor_options() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client1 = new mqtt.Client({ host: 'mqtt.example.com' });
                var client2 = new mqtt.Client({ host: 'mqtt.example.com', port: 8883 });
                var client3 = new mqtt.Client({ host: 'mqtt.example.com', tls: true });
                var client4 = new mqtt.Client({ host: 'mqtt.example.com', clientId: 'myclient' });
                var client5 = new mqtt.Client({ host: 'mqtt.example.com', clean: true, keepalive: 60 });
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn mqtt_subscribe_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new mqtt.Client({ host: 'example.com' });
                try {
                    client.subscribe('topic');
                    client.subscribe('topic', { qos: 0 });
                    client.subscribe('topic', { qos: 1 });
                    client.subscribe(['topic1', 'topic2']);
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn mqtt_unsubscribe_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new mqtt.Client({ host: 'example.com' });
                try {
                    client.unsubscribe('topic');
                    client.unsubscribe(['topic1', 'topic2']);
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn mqtt_publish_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new mqtt.Client({ host: 'example.com' });
                try {
                    client.publish('topic', 'message');
                    client.publish('topic', 'message', { qos: 0 });
                    client.publish('topic', 'message', { qos: 1, retain: false });
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn mqtt_end_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new mqtt.Client({ host: 'example.com' });
                try {
                    client.end();
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn mqtt_events_registerable() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new mqtt.Client({ host: 'example.com' });
                var events = ['connect', 'disconnect', 'error', 'message', 'offline',
                             'reconnect', 'subscribe', 'unsubscribe'];
                var registered = 0;
                events.forEach(function(ev) {
                    try {
                        client.on(ev, function() {});
                        registered++;
                    } catch(e) {}
                });
                return String(registered === events.length);
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn mqtt_qos_options() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new mqtt.Client({ host: 'example.com' });
                try {
                    client.subscribe('topic', { qos: 0 });
                    client.subscribe('topic', { qos: 1 });
                    client.publish('topic', 'msg', { qos: 0 });
                    client.publish('topic', 'msg', { qos: 1 });
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}
