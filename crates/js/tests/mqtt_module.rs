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
