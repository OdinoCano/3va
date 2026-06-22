// Tests for the WebRTC builtin.
// Run: cargo test -p vvva_js --test webrtc_module

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
async fn webrtc_globals_exist() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof RTCPeerConnection === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");

    let r = e
        .eval_to_string("String(typeof RTCSessionDescription === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");

    let r = e
        .eval_to_string("String(typeof RTCIceCandidate === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");

    let r = e
        .eval_to_string("String(typeof RTCDataChannel === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn webrtc_native_functions_exist() {
    let e = engine_no_net().await;
    let funcs = [
        "__rtcCreatePeerConnection",
        "__rtcCreateOffer",
        "__rtcCreateAnswer",
        "__rtcSetLocalDescription",
        "__rtcSetRemoteDescription",
        "__rtcAddIceCandidate",
        "__rtcCreateDataChannel",
        "__rtcDataChannelSend",
        "__rtcDataChannelClose",
        "__rtcClosePeerConnection",
        "__rtcGetConnectionState",
    ];
    for func in funcs {
        let r = e
            .eval_to_string(&format!("String(typeof {} === 'function')", func))
            .await
            .unwrap();
        assert_eq!(r, "true", "{} should be a function", func);
    }
}

#[tokio::test]
async fn rtc_peer_connection_constructor() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                return String(pc !== null && typeof pc.createOffer === 'function');
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_peer_connection_has_expected_methods() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                var methods = ['createOffer', 'createAnswer', 'setLocalDescription',
                              'setRemoteDescription', 'addIceCandidate', 'createDataChannel',
                              'close', 'getConnectionState'];
                var missing = [];
                for (var i = 0; i < methods.length; i++) {
                    if (typeof pc[methods[i]] !== 'function') {
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
async fn rtc_peer_connection_has_event_handlers() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                return String(
                    typeof pc.onicecandidate === 'function' ||
                    pc.onicecandidate === null
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_peer_connection_has_event_emitter() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                return String(
                    typeof pc.addEventListener === 'function' &&
                    typeof pc.removeEventListener === 'function'
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_session_description_constructor() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var sd = new RTCSessionDescription({ type: 'offer', sdp: 'v=0...' });
                return String(sd.type === 'offer' && sd.sdp === 'v=0...');
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_ice_candidate_constructor() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var candidate = new RTCIceCandidate({
                    candidate: 'candidate:1 1 UDP 123 192.168.1.1 12345 typ host',
                    sdpMid: '0',
                    sdpMLineIndex: 0
                });
                return String(
                    candidate.candidate.indexOf('192.168.1.1') >= 0 &&
                    candidate.sdpMid === '0'
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_data_channel_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                try {
                    var dc = pc.createDataChannel('test');
                    return String(
                        dc.label === 'test' &&
                        typeof dc.send === 'function' &&
                        typeof dc.close === 'function'
                    );
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_data_channel_has_event_emitter() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                var dc = pc.createDataChannel('test');
                return String(
                    typeof dc.addEventListener === 'function' &&
                    typeof dc.removeEventListener === 'function'
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_peer_connection_config() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var config = {
                    iceServers: [
                        { urls: 'stun:stun.l.google.com:19302' },
                        { urls: 'stun:stun1.example.com' }
                    ]
                };
                var pc = new RTCPeerConnection(config);
                return String(pc !== null);
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_create_offer_returns_promise() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                var offer = pc.createOffer();
                return String(offer && typeof offer.then === 'function');
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_create_answer_returns_promise() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                var answer = pc.createAnswer();
                return String(answer && typeof answer.then === 'function');
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_data_channel_options() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                try {
                    var dc1 = pc.createDataChannel('ordered');
                    var dc2 = pc.createDataChannel('unreliable', {
                        ordered: false,
                        maxRetransmits: 0
                    });
                    var dc3 = pc.createDataChannel('maxlife', {
                        ordered: false,
                        maxPacketLifeTime: 1000
                    });
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
async fn rtc_get_connection_state() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                var state = pc.getConnectionState();
                var validStates = ['new', 'connecting', 'connected', 'disconnected',
                                   'failed', 'closed'];
                return String(validStates.indexOf(state) >= 0);
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_signaling_state() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                var state = pc.signalingState;
                var validStates = ['stable', 'have-local-offer', 'have-remote-offer',
                                   'have-local-pranswer', 'have-remote-pranswer',
                                   'closed'];
                return String(validStates.indexOf(state) >= 0);
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_ice_connection_state() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                var state = pc.iceConnectionState;
                var validStates = ['new', 'checking', 'connected', 'completed',
                                   'failed', 'disconnected', 'closed'];
                return String(validStates.indexOf(state) >= 0);
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rtc_close_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                try {
                    pc.close();
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
async fn rtc_data_channel_close_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var pc = new RTCPeerConnection();
                try {
                    var dc = pc.createDataChannel('test');
                    dc.close();
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
async fn webrtc_require_cache_integration() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var cached = globalThis.__requireCache && globalThis.__requireCache['webrtc'];
                return String(
                    cached &&
                    cached.RTCPeerConnection === RTCPeerConnection &&
                    cached.RTCSessionDescription === RTCSessionDescription &&
                    cached.RTCIceCandidate === RTCIceCandidate
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}
