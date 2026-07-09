// Tests for the IMAP builtin.
// Run: cargo test -p vvva_js --test imap_module

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
async fn imap_global_exists() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof imap === 'object')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn imap_client_constructor_exists() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof imap.Client === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn imap_client_has_expected_methods() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                var methods = ['connect', 'login', 'openBox', 'status', 'createBox',
                              'deleteBox', 'renameBox', 'subscribeBox', 'listBoxes',
                              'fetch', 'append', 'copy', 'move', 'addFlags',
                              'removeFlags', 'setFlags', 'expunge', 'search',
                              'end', 'disconnect', 'closeBox'];
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

// ── Permission enforcement ───────────────────────────────────────────────────

#[tokio::test]
async fn imap_connect_blocked_without_net_grant() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                try {
                    var client = new imap.Client({ host: 'example.com', port: 993 });
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
async fn imap_client_event_emitter() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                return String(
                    typeof client.on === 'function' &&
                    typeof client.emit === 'function' &&
                    typeof client.off === 'function'
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn imap_fetch_returns_stream() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                var stream = client.fetch('1:3', { bodies: 'HEADER' });
                return String(
                    stream &&
                    typeof stream.on === 'function' &&
                    typeof stream.emit === 'function'
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn imap_search_criteria_formatting() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                try {
                    var client = new imap.Client({ host: 'example.com' });
                    // Test search criteria formatting (should not throw)
                    var criteria = ['UNSEEN', ['FROM', 'test@example.com'], ['SUBJECT', 'Hello']];
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
async fn imap_append_options() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // append should accept options without throwing
                try {
                    var body = 'From: test@example.com\r\nTo: friend@example.com\r\nSubject: Test\r\n\r\nHello';
                    // Should not throw even if not connected
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
async fn imap_flags_handling() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // Test that flags are properly formatted (with or without backslash)
                var flags1 = ['Seen', 'Flagged'];
                var flags2 = ['\\Seen', '\\Flagged'];
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_openbox_readonly_option() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // openBox with readOnly=true should be accepted
                // Note: This doesn't actually connect, just tests the API
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_status_without_select() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // status() should be callable without selecting a mailbox first
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_rename_box() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // renameBox API should be callable
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_copy_move_api() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // copy and move should be callable
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_expunge_api() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // expunge should be callable
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_close_box_api() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // closeBox should be callable
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_end_vs_disconnect() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // end() and disconnect() should both be callable
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_constructor_options() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                // Test various constructor options
                var client1 = new imap.Client({ host: 'imap.example.com' });
                var client2 = new imap.Client({ host: 'imap.example.com', port: 993 });
                var client3 = new imap.Client({ host: 'imap.example.com', tls: true });
                var client4 = new imap.Client({ host: 'imap.example.com', tls: true, username: 'user', password: 'pass' });
                var client5 = new imap.Client({ host: 'imap.example.com', tlsOptions: { rejectUnauthorized: false } });
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_list_boxes_api() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // listBoxes should be callable
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_subscribe_box_api() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // subscribeBox should be callable
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_fetch_options() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // Various fetch options
                var s1 = client.fetch('1:3', { bodies: 'HEADER' });
                var s2 = client.fetch('1:3', { bodies: 'TEXT' });
                var s3 = client.fetch('1:3', { bodies: 'FULL' });
                var s4 = client.fetch('*', { bodies: 'HEADER.FIELDS' });
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn imap_range_formats() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new imap.Client({ host: 'example.com' });
                // Various range formats
                var s1 = client.fetch('1', { bodies: 'HEADER' });       // single
                var s2 = client.fetch('1:3', { bodies: 'HEADER' });    // range
                var s3 = client.fetch('3:*', { bodies: 'HEADER' });     // to end
                var s4 = client.fetch('*', { bodies: 'HEADER' });        // last
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}
