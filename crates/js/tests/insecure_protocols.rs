// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Plaintext protocols to non-loopback hosts are refused unless the user opts
// in with --allow-insecure, even when --allow-net grants the host.
// Own test binary: `set_allow_insecure` is process-wide.
// Run: cargo test -p vvva_js --test insecure_protocols

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net(host: &str) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::Network(host.to_string()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

async fn error_of(e: &mut JsEngine, expr: &str) -> String {
    e.eval(&format!(
        "globalThis.__err = 'no error'; try {{ {expr}; }} catch (err) {{ globalThis.__err = String((err && err.message) || err); }}"
    ))
    .await
    .unwrap();
    e.eval_to_string("globalThis.__err").await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn plaintext_to_remote_host_needs_opt_in() {
    let mut e = engine_with_net("192.0.2.1").await;
    for expr in [
        "__fetchAsync('http://192.0.2.1/', 'GET', '{}', null, undefined)",
        "__wsConnect('ws://192.0.2.1/')",
        "__ftpConnect(__ftpCreate(), '192.0.2.1', 21, false)",
    ] {
        let err = error_of(&mut e, expr).await;
        assert!(err.contains("--allow-insecure"), "{expr}: got {err}");
    }
}
