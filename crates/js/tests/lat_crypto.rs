// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Latency baseline for the crypto transport (FIPS-relevant), measured in JS on
// the main thread. Kept as its own test binary so each transport's engine runs
// in isolation.
//
// Run: cargo test -p vvva_js --test lat_crypto
//      cargo test -p vvva_js --features fips --test lat_crypto

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    let state = PermissionState::new();
    // crypto does not need network; keep the grant minimal for the baseline.
    JsEngine::new(Arc::new(state)).await.unwrap()
}

/// Synchronous crypto latency: repeated SHA-256 hashing and an RSA-2048
/// sign+verify session, measured in JS. Uses only FIPS-approved algorithms in
/// the fips build; md5 is deliberately avoided here.
#[tokio::test]
async fn crypto_sync_verify_latency_budget() {
    // Debug builds spend ~2.7 s here on a dev machine (mostly JS byte-array
    // glue, not the hash itself); shared CI runners can be 2-3x slower.
    const HASH_BUDGET_MS: u128 = 15_000;
    const SIGN_BUDGET_MS: u128 = 10_000;

    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var payload = 'x'.repeat(1024 * 16);

            var t0 = Date.now();
            var out = '';
            for (var i = 0; i < 2000; i++) out = c.createHash('sha256').update(payload).digest('hex');
            var hashMs = Date.now() - t0;

            var pair = c.generateKeyPairSync('rsa', { modulusLength: 2048 });
            var t1 = Date.now();
            for (var i = 0; i < 50; i++) {
                var sig = c.sign('sha256', Buffer.from('m'), pair.privateKey);
                var ok = c.verify('sha256', Buffer.from('m'), pair.publicKey, sig);
                if (!ok) throw new Error('verify failed');
            }
            var signMs = Date.now() - t1;

            JSON.stringify({ hashMs: hashMs, signMs: signMs });
            "#,
        )
        .await
        .unwrap();
    let stats: serde_json::Value = serde_json::from_str(&r).expect("valid crypto latency JSON");
    let hash_ms = stats["hashMs"].as_u64().unwrap();
    let sign_ms = stats["signMs"].as_u64().unwrap();

    assert!(
        (hash_ms as u128) < HASH_BUDGET_MS,
        "2000×SHA-256 took {hash_ms} ms, over budget {HASH_BUDGET_MS} ms"
    );
    assert!(
        (sign_ms as u128) < SIGN_BUDGET_MS,
        "50×RSA sign+verify took {sign_ms} ms, over budget {SIGN_BUDGET_MS} ms"
    );
    eprintln!("crypto latency: 2000×SHA-256 {hash_ms} ms; 50×RSA-2048 sign+verify {sign_ms} ms");
}
