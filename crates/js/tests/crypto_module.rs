// Tests for the native crypto builtin.
// Run: cargo test -p vvva_js --test crypto_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

/// Drive the engine's event loop until a global variable is set or we time out.
async fn eval_async_result(e: &JsEngine, setup: &str, result_global: &str) -> String {
    e.eval(setup).await.unwrap();
    for _ in 0..50 {
        e.idle().await;
        tokio::task::yield_now().await;
        let r = e
            .eval_to_string(&format!(
                "String({result_global} !== null ? {result_global} : '')"
            ))
            .await
            .unwrap();
        if !r.is_empty() {
            return r;
        }
    }
    String::new()
}

// ── createHash ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn crypto_module_loads() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof require('crypto').createHash === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn hash_sha256_hex() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"require('crypto').createHash('sha256').update('abc').digest('hex')"#)
        .await
        .unwrap();
    // Verified: echo -n "abc" | sha256sum
    assert_eq!(
        r,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[tokio::test]
async fn hash_sha256_empty_string() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"require('crypto').createHash('sha256').update('').digest('hex')"#)
        .await
        .unwrap();
    assert_eq!(
        r,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[tokio::test]
async fn hash_sha512_hex() {
    let e = engine().await;
    // Verified: echo -n "abc" | sha512sum
    let r = e
        .eval_to_string(r#"require('crypto').createHash('sha512').update('abc').digest('hex')"#)
        .await
        .unwrap();
    assert_eq!(
        r,
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
         2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
    );
}

#[tokio::test]
async fn hash_chained_updates() {
    let e = engine().await;
    // hash("hello" + " " + "world") == hash("hello world")
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var h1 = c.createHash('sha256').update('hello').update(' ').update('world').digest('hex');
            var h2 = c.createHash('sha256').update('hello world').digest('hex');
            String(h1 === h2)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn hash_sha256_base64() {
    let e = engine().await;
    // Verified: echo -n "abc" | openssl dgst -sha256 -binary | base64
    let r = e
        .eval_to_string(r#"require('crypto').createHash('sha256').update('abc').digest('base64')"#)
        .await
        .unwrap();
    assert_eq!(r, "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");
}

#[tokio::test]
async fn hash_sha256_base64url() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"require('crypto').createHash('sha256').update('abc').digest('base64url')"#,
        )
        .await
        .unwrap();
    // base64url: replace +→-, /→_, strip =
    assert_eq!(r, "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0");
}

#[tokio::test]
async fn hash_shorthand() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"require('crypto').hash('sha256', 'abc')"#)
        .await
        .unwrap();
    assert_eq!(
        r,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[tokio::test]
async fn hash_unsupported_algorithm_throws() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            try {
                require('crypto').createHash('md5').update('x').digest('hex');
                'no-throw'
            } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

// ── createHmac ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn hmac_sha256_hex() {
    let e = engine().await;
    // Verified: echo -n "The quick brown fox..." | openssl dgst -sha256 -hmac "key"
    let r = e
        .eval_to_string(
            r#"
            require('crypto')
                .createHmac('sha256', 'key')
                .update('The quick brown fox jumps over the lazy dog')
                .digest('hex')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r,
        "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
    );
}

#[tokio::test]
async fn hmac_sha512_hex() {
    let e = engine().await;
    // Verified: echo -n "message" | openssl dgst -sha512 -hmac "key"
    let r = e
        .eval_to_string(
            r#"
            require('crypto')
                .createHmac('sha512', 'key')
                .update('message')
                .digest('hex')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r,
        "e477384d7ca229dd1426e64b63ebf2d36ebd6d7e669a6735424e72ea6c01d3f8\
         b56eb39c36d8232f5427999b8d1a3f9cd1128fc69f4d75b434216810fa367e98"
    );
}

// ── randomBytes / randomUUID ─────────────────────────────────────────────────

#[tokio::test]
async fn random_bytes_correct_length() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"String(require('crypto').randomBytes(32).length)"#)
        .await
        .unwrap();
    assert_eq!(r, "32");
}

#[tokio::test]
async fn random_bytes_returns_uint8array() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"String(require('crypto').randomBytes(8) instanceof Uint8Array)"#)
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn random_bytes_are_random() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var a = c.randomBytes(16);
            var b = c.randomBytes(16);
            String(JSON.stringify(Array.from(a)) !== JSON.stringify(Array.from(b)))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn random_uuid_format() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('crypto').randomUUID();
            var ok = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(u);
            String(ok)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── timingSafeEqual ──────────────────────────────────────────────────────────

#[tokio::test]
async fn timing_safe_equal_matching_buffers() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            String(c.timingSafeEqual(new Uint8Array([1,2,3,4]), new Uint8Array([1,2,3,4])))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn timing_safe_equal_different_buffers() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            String(c.timingSafeEqual(new Uint8Array([1,2,3,4]), new Uint8Array([1,2,3,5])))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "false");
}

#[tokio::test]
async fn timing_safe_equal_different_lengths_throws() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            try {
                require('crypto').timingSafeEqual(new Uint8Array([1,2]), new Uint8Array([1,2,3]));
                'no-throw'
            } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

// ── pbkdf2 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pbkdf2_sha256_known_vector() {
    let e = engine().await;
    // Verified: python3 -c "import hashlib; k=hashlib.pbkdf2_hmac('sha256',b'password',b'salt',1,32); print(k.hex())"
    let r = eval_async_result(
        &e,
        r#"
        globalThis._pbkdf2_result = null;
        require('crypto').pbkdf2('password', 'salt', 1, 32, 'sha256', function(err, key) {
            if (!err) {
                globalThis._pbkdf2_result = Array.from(key)
                    .map(function(b){ return ('0'+b.toString(16)).slice(-2); })
                    .join('');
            }
        });
        "#,
        "globalThis._pbkdf2_result",
    )
    .await;
    assert_eq!(
        r,
        "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
    );
}

// ── scrypt ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn scrypt_produces_correct_length() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._scrypt_len = null;
        require('crypto').scrypt('password', 'salt', 32, { N: 1024, r: 8, p: 1 }, function(err, key) {
            if (!err) globalThis._scrypt_len = String(key.length);
        });
        "#,
        "globalThis._scrypt_len",
    )
    .await;
    assert_eq!(r, "32");
}

#[tokio::test]
async fn scrypt_known_vector() {
    let e = engine().await;
    // Verified: python3 -c "import hashlib; k=hashlib.scrypt(b'',salt=b'',n=16,r=1,p=1,dklen=64); print(k.hex())"
    let r = eval_async_result(
        &e,
        r#"
        globalThis._scrypt_vec = null;
        require('crypto').scrypt('', '', 64, { N: 16, r: 1, p: 1 }, function(err, key) {
            if (!err) {
                globalThis._scrypt_vec = Array.from(key)
                    .map(function(b){ return ('0'+b.toString(16)).slice(-2); })
                    .join('');
            }
        });
        "#,
        "globalThis._scrypt_vec",
    )
    .await;
    assert_eq!(
        r,
        "77d6576238657b203b19ca42c18a0497f16b4844e3074ae8dfdffa3fede21442\
         fcd0069ded0948f8326a753a0fc81f17e8d3e0fb2e0d3628cf35e20c38d18906"
    );
}
