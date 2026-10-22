// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// `unref()` must stop a handle from keeping the process alive: once only
// unref'd timers, servers and sockets are left, the event loop ends. Before,
// `unref()` was a no-op everywhere and the process ran until it was killed
// (test-http-server-keepalive-end and some forty more tests of the Node suite).
//
// Run: cargo test -p vvva_js --test unref

use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    perms.grant(Capability::Network("0.0.0.0".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

/// True when the event loop ends by itself within a few seconds.
async fn loop_ends(script: &str) -> bool {
    let mut e = engine().await;
    e.eval(script).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), e.run_event_loop())
        .await
        .is_ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_referenced_timer_keeps_the_loop_alive() {
    // The control: without unref() the loop waits for the timer.
    assert!(
        !loop_ends("setTimeout(function() {}, 60000);").await,
        "a referenced timer must keep the loop alive"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreffed_timer_does_not() {
    assert!(loop_ends("setTimeout(function() {}, 60000).unref();").await);
    assert!(loop_ends("setInterval(function() {}, 1000).unref();").await);
}

#[tokio::test(flavor = "multi_thread")]
async fn ref_after_unref_keeps_it_alive_again() {
    assert!(!loop_ends("var t = setTimeout(function() {}, 60000); t.unref(); t.ref();").await);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreffed_http_server_does_not() {
    assert!(
        loop_ends("var s = require('http').createServer(); s.listen(0, '127.0.0.1'); s.unref();")
            .await
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreffed_net_server_and_socket_do_not() {
    assert!(
        loop_ends("var s = require('net').createServer(); s.listen(0, '127.0.0.1'); s.unref();")
            .await
    );
    assert!(
        loop_ends(
            "var net = require('net'); var s = net.createServer(); \
             s.listen(0, '127.0.0.1', function() { \
               var c = net.connect(this.address().port, '127.0.0.1'); c.unref(); s.unref(); });"
        )
        .await
    );
}
