// Sprint 1 compatibility tests:
//   - Buffer as real Uint8Array subclass
//   - crypto.createHash MD5
//   - crypto.createPrivateKey / createPublicKey / createSecretKey
//   - crypto.createSign / createVerify (RSA PKCS1v15, ECDSA P-256/P-384)
//   - crypto.sign / crypto.verify (one-shot)
//   - assert.deepStrictEqual full implementation
//
// Run: cargo test -p vvva_js --test compat_sprint1

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

async fn eval_async_result(e: &mut JsEngine, setup: &str, var: &str) -> String {
    e.eval(setup).await.unwrap();
    for _ in 0..200 {
        e.idle().await;
        tokio::task::yield_now().await;
        let r = e
            .eval_to_string(&format!(
                "String({var} !== undefined && {var} !== null ? {var} : '')"
            ))
            .await
            .unwrap();
        if !r.is_empty() {
            return r;
        }
    }
    String::new()
}

// ── Buffer as real Uint8Array subclass ────────────────────────────────────────

#[tokio::test]
async fn buffer_is_instanceof_uint8array() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.from('hello') instanceof Uint8Array)")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_index_access_works() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.from([0x41, 0x42, 0x43]);
            String(b[0] === 0x41 && b[1] === 0x42 && b[2] === 0x43)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_length_correct() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.alloc(10);
            String(b.length === 10 && b.byteLength === 10)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_write_index_and_read_back() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.alloc(4);
            b[0] = 0xFF;
            b[3] = 0x01;
            String(b[0] === 255 && b[3] === 1 && b[1] === 0)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_set_works_as_uint8array() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.alloc(4);
            b.set([1, 2, 3, 4]);
            String(b[0] === 1 && b[2] === 3)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_slice_is_buffer() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.from([1,2,3,4,5]);
            var s = b.slice(1, 3);
            String(s instanceof Buffer && s instanceof Uint8Array && s.length === 2 && s[0] === 2)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_spread_into_uint8array() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.from([10, 20, 30]);
            var u = new Uint8Array([...b]);
            String(u[0] === 10 && u[1] === 20 && u[2] === 30)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_read_write_uint32() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.alloc(4);
            b.writeUInt32BE(0xDEADBEEF, 0);
            String(b.readUInt32BE(0) === 0xDEADBEEF)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_to_json_round_trip() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.from([1,2,3]);
            var j = b.toJSON();
            String(j.type === 'Buffer' && JSON.stringify(j.data) === '[1,2,3]')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── crypto.createHash MD5 ────────────────────────────────────────────────────

#[tokio::test]
async fn crypto_hash_md5() {
    let mut e = engine().await;
    // md5('') = d41d8cd98f00b204e9800998ecf8427e
    let r = e
        .eval_to_string(r#"require('crypto').createHash('md5').update('').digest('hex')"#)
        .await
        .unwrap();
    assert_eq!(r, "d41d8cd98f00b204e9800998ecf8427e");
}

#[tokio::test]
async fn crypto_hash_md5_nonempty() {
    let mut e = engine().await;
    // md5('hello') = 5d41402abc4b2a76b9719d911017c592
    let r = e
        .eval_to_string(r#"require('crypto').createHash('md5').update('hello').digest('hex')"#)
        .await
        .unwrap();
    assert_eq!(r, "5d41402abc4b2a76b9719d911017c592");
}

#[tokio::test]
async fn crypto_get_hashes_includes_md5() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(r#"String(require('crypto').getHashes().indexOf('md5') !== -1)"#)
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── crypto.createPrivateKey / createPublicKey / createSecretKey ───────────────

#[tokio::test]
async fn create_private_key_from_pem() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('rsa', { modulusLength: 1024 });
            var privPem = pair.privateKey.export();
            var key = c.createPrivateKey(privPem);
            String(key.type === 'private' && typeof key.export() === 'string')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn create_public_key_from_pem() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('rsa', { modulusLength: 1024 });
            var pubPem = pair.publicKey.export();
            var key = c.createPublicKey(pubPem);
            String(key.type === 'public' && typeof key.export() === 'string')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn create_secret_key() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var key = c.createSecretKey(c.randomBytes(32));
            String(key.type === 'secret' && key.symmetricKeySize === 32)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── crypto.createSign / createVerify — RSA PKCS1v15 ──────────────────────────

#[tokio::test]
async fn rsa_sign_and_verify_sha256() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('rsa', { modulusLength: 1024 });
            var privKey = c.createPrivateKey(pair.privateKey.export());
            var pubKey  = c.createPublicKey(pair.publicKey.export());
            var data = 'hello world';
            var sig = c.createSign('RSA-SHA256').update(data).sign(privKey);
            var ok  = c.createVerify('RSA-SHA256').update(data).verify(pubKey, sig);
            String(ok === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rsa_sign_and_verify_sha512() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('rsa', { modulusLength: 1024 });
            var sig = c.createSign('RSA-SHA512').update('test').sign(pair.privateKey);
            var ok  = c.createVerify('RSA-SHA512').update('test').verify(pair.publicKey, sig);
            String(ok === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rsa_verify_fails_on_tampered_data() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('rsa', { modulusLength: 1024 });
            var sig = c.createSign('RSA-SHA256').update('original').sign(pair.privateKey);
            var ok  = c.createVerify('RSA-SHA256').update('tampered').verify(pair.publicKey, sig);
            String(ok === false)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn rsa_sign_with_pem_string_directly() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('rsa', { modulusLength: 1024 });
            var privPem = pair.privateKey.export();
            var pubPem  = pair.publicKey.export();
            var sig = c.createSign('SHA256').update('data').sign(privPem);
            var ok  = c.createVerify('SHA256').update('data').verify(pubPem, sig);
            String(ok === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── crypto.createSign / createVerify — ECDSA ─────────────────────────────────

#[tokio::test]
async fn ecdsa_p256_sign_and_verify() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('ec', { namedCurve: 'P-256' });
            var data = 'message to sign';
            var sig = c.createSign('SHA256').update(data).sign(pair.privateKey);
            var ok  = c.createVerify('SHA256').update(data).verify(pair.publicKey, sig);
            String(ok === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ecdsa_p384_sign_and_verify() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('ec', { namedCurve: 'P-384' });
            var sig = c.createSign('SHA384').update('hello').sign(pair.privateKey);
            var ok  = c.createVerify('SHA384').update('hello').verify(pair.publicKey, sig);
            String(ok === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ecdsa_verify_fails_wrong_data() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('ec', { namedCurve: 'P-256' });
            var sig = c.createSign('SHA256').update('correct').sign(pair.privateKey);
            var ok  = c.createVerify('SHA256').update('wrong').verify(pair.publicKey, sig);
            String(ok === false)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── crypto.sign / crypto.verify (one-shot) ───────────────────────────────────

#[tokio::test]
async fn crypto_sign_one_shot() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('rsa', { modulusLength: 1024 });
            var sig = c.sign('SHA256', Buffer.from('payload'), pair.privateKey);
            var ok  = c.verify('SHA256', Buffer.from('payload'), pair.publicKey, sig);
            String(ok === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── assert.deepStrictEqual ────────────────────────────────────────────────────

#[tokio::test]
async fn assert_deep_strict_equal_objects() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            try { assert.deepStrictEqual({a:1,b:{c:2}},{a:1,b:{c:2}}); 'ok' }
            catch(e) { 'failed: ' + e.message }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn assert_deep_strict_equal_arrays() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            try { assert.deepStrictEqual([1,[2,3]],[1,[2,3]]); 'ok' } catch(e) { 'fail' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn assert_deep_strict_equal_typed_array() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            try { assert.deepStrictEqual(new Uint8Array([1,2,3]),new Uint8Array([1,2,3])); 'ok' }
            catch(e) { 'fail: ' + e.message }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn assert_deep_strict_equal_fails_type_mismatch() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            try { assert.deepStrictEqual(1, '1'); 'no-throw' } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

#[tokio::test]
async fn assert_deep_strict_equal_fails_deep_mismatch() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            try { assert.deepStrictEqual({a:1},{a:2}); 'no-throw' } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

#[tokio::test]
async fn assert_deep_strict_equal_handles_undefined() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            // JSON.stringify drops undefined — the old impl would fail here
            var a = {x: undefined, y: 1};
            var b = {x: undefined, y: 1};
            try { assert.deepStrictEqual(a, b); 'ok' } catch(e) { 'fail' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn assert_deep_strict_equal_date() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            var d1 = new Date('2024-01-01'), d2 = new Date('2024-01-01');
            try { assert.deepStrictEqual(d1, d2); 'ok' } catch(e) { 'fail' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn assert_not_deep_strict_equal() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            try { assert.notDeepStrictEqual({a:1},{a:2}); 'ok' } catch(e) { 'fail' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn assert_if_error_passes_on_null() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            try { assert.ifError(null); 'ok' } catch(e) { 'fail' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn assert_if_error_throws_on_error() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            try { assert.ifError(new Error('boom')); 'no-throw' } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

// ── http.IncomingMessage / http.ServerResponse as classes ─────────────────────

#[tokio::test]
async fn http_incoming_message_is_exported() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("typeof require('http').IncomingMessage")
        .await
        .unwrap();
    assert_eq!(r, "function");
}

#[tokio::test]
async fn http_server_response_is_exported() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("typeof require('http').ServerResponse")
        .await
        .unwrap();
    assert_eq!(r, "function");
}

#[tokio::test]
async fn http_incoming_message_instanceof_readable() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var stream = require('stream');
            var req = new http.IncomingMessage({});
            String(req instanceof stream.Readable)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn http_server_response_instanceof_writable() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var stream = require('stream');
            var req = new http.IncomingMessage({});
            var res = new http.ServerResponse(req);
            String(res instanceof stream.Writable)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn http_prototype_extension_works() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            http.ServerResponse.prototype.send = function(m) { this.end(m); };
            var req = new http.IncomingMessage({});
            var res = new http.ServerResponse(req);
            typeof res.send
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "function");
}

#[tokio::test]
async fn http_server_response_writehead_works() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var req = new http.IncomingMessage({});
            var res = new http.ServerResponse(req);
            res.setHeader('X-Test', 'value');
            res.getHeader('X-Test')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "value");
}

#[tokio::test]
async fn http_incoming_message_properties() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var req = new http.IncomingMessage({});
            req.method = 'POST';
            req.url = '/test';
            req.method + ' ' + req.url
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "POST /test");
}

#[tokio::test]
async fn http_server_response_status_code_default() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var req = new http.IncomingMessage({});
            var res = new http.ServerResponse(req);
            String(res.statusCode)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "200");
}

// ── crypto.subtle asymmetric sign/verify (RSA + ECDSA) ────────────────────────

#[tokio::test]
async fn subtle_rsa_pkcs1_sign_verify() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var crypto = require('crypto');
        var encoder = new TextEncoder();
        (async function() {
            try {
                var keyPair = await crypto.subtle.generateKey(
                    { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, hash: 'SHA-256' },
                    true, ['sign', 'verify']
                );
                var data = encoder.encode('test data');
                var sig = await crypto.subtle.sign({ name: 'RSASSA-PKCS1-v1_5' }, keyPair.privateKey, data);
                var ok = await crypto.subtle.verify({ name: 'RSASSA-PKCS1-v1_5' }, keyPair.publicKey, sig, data);
                globalThis.__rsaPkcs1 = ok ? 'ok' : 'fail';
            } catch(e) { globalThis.__rsaPkcs1 = 'error:' + (e.message || e); }
        })();
        "#,
        "__rsaPkcs1",
    )
    .await;
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn subtle_rsa_pss_sign_verify() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var crypto = require('crypto');
        var encoder = new TextEncoder();
        (async function() {
            try {
                var keyPair = await crypto.subtle.generateKey(
                    { name: 'RSA-PSS', modulusLength: 2048, hash: 'SHA-256' },
                    true, ['sign', 'verify']
                );
                var data = encoder.encode('test pss');
                var sig = await crypto.subtle.sign({ name: 'RSA-PSS', saltLength: 32 }, keyPair.privateKey, data);
                var ok = await crypto.subtle.verify({ name: 'RSA-PSS', saltLength: 32 }, keyPair.publicKey, sig, data);
                globalThis.__rsaPss = ok ? 'ok' : 'fail';
            } catch(e) { globalThis.__rsaPss = 'error:' + (e.message || e); }
        })();
        "#,
        "__rsaPss",
    )
    .await;
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn subtle_ecdsa_p256_sign_verify() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var crypto = require('crypto');
        var encoder = new TextEncoder();
        (async function() {
            try {
                var keyPair = await crypto.subtle.generateKey(
                    { name: 'ECDSA', namedCurve: 'P-256', hash: 'SHA-256' },
                    true, ['sign', 'verify']
                );
                var data = encoder.encode('test ecdsa');
                var sig = await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, keyPair.privateKey, data);
                if (sig.byteLength !== 64) { globalThis.__ecdsa256 = 'bad-sig-length:' + sig.byteLength; return; }
                var ok = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, keyPair.publicKey, sig, data);
                globalThis.__ecdsa256 = ok ? 'ok' : 'fail';
            } catch(e) { globalThis.__ecdsa256 = 'error:' + (e.message || e); }
        })();
        "#,
        "__ecdsa256",
    )
    .await;
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn subtle_ecdsa_p384_sign_verify() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var crypto = require('crypto');
        var encoder = new TextEncoder();
        (async function() {
            try {
                var keyPair = await crypto.subtle.generateKey(
                    { name: 'ECDSA', namedCurve: 'P-384', hash: 'SHA-384' },
                    true, ['sign', 'verify']
                );
                var data = encoder.encode('test ecdsa p384');
                var sig = await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-384' }, keyPair.privateKey, data);
                if (sig.byteLength !== 96) { globalThis.__ecdsa384 = 'bad-sig-length:' + sig.byteLength; return; }
                var ok = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-384' }, keyPair.publicKey, sig, data);
                globalThis.__ecdsa384 = ok ? 'ok' : 'fail';
            } catch(e) { globalThis.__ecdsa384 = 'error:' + (e.message || e); }
        })();
        "#,
        "__ecdsa384",
    )
    .await;
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn subtle_rsa_wrong_data_fails_verify() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var crypto = require('crypto');
        var encoder = new TextEncoder();
        (async function() {
            try {
                var keyPair = await crypto.subtle.generateKey(
                    { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, hash: 'SHA-256' },
                    true, ['sign', 'verify']
                );
                var good = encoder.encode('good');
                var wrong = encoder.encode('bad');
                var sig = await crypto.subtle.sign({ name: 'RSASSA-PKCS1-v1_5' }, keyPair.privateKey, good);
                var ok = await crypto.subtle.verify({ name: 'RSASSA-PKCS1-v1_5' }, keyPair.publicKey, sig, wrong);
                globalThis.__rsaWrong = ok ? 'should-fail' : 'ok';
            } catch(e) { globalThis.__rsaWrong = 'error:' + (e.message || e); }
        })();
        "#,
        "__rsaWrong",
    )
    .await;
    assert_eq!(r, "ok");
}

// ── process.stdout/stderr as Writable streams ─────────────────────────────────

#[tokio::test]
async fn process_stdout_is_writable() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var stream = require('stream');
            String(process.stdout instanceof stream.Writable)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn process_stderr_is_writable() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var stream = require('stream');
            String(process.stderr instanceof stream.Writable)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn process_stdout_fd_is_1() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(process.stdout.fd === 1)")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn process_stderr_fd_is_2() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(process.stderr.fd === 2)")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── process.env as dynamic Proxy ──────────────────────────────────────────────

#[tokio::test]
async fn process_env_is_proxy() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(process.env.__isProxy === true)")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn process_env_set_get_delete() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            process.env.TEST_VAR = 'hello';
            var ok = process.env.TEST_VAR === 'hello';
            ok = ok && ('TEST_VAR' in process.env);
            process.env.TEST_VAR = 'world';
            ok = ok && (process.env.TEST_VAR === 'world');
            delete process.env.TEST_VAR;
            ok = ok && !('TEST_VAR' in process.env);
            String(ok)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn process_env_own_keys_works() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            process.env.A = '1';
            process.env.B = '2';
            var keys = Object.keys(process.env);
            var ok = Array.isArray(keys);
            ok = ok && keys.indexOf('A') !== -1;
            ok = ok && keys.indexOf('B') !== -1;
            ok = ok && keys.indexOf('PATH') === -1;
            String(ok)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── net.Socket as stream.Duplex ──────────────────────────────────────────────

#[tokio::test]
async fn socket_instanceof_duplex() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var net = require('net');
            var stream = require('stream');
            var s = new net.Socket({});
            String(s instanceof stream.Duplex)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn socket_instanceof_readable_writable() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var net = require('net');
            var stream = require('stream');
            var s = new net.Socket({});
            var ok = s instanceof stream.Readable;
            ok = ok && s instanceof stream.Writable;
            String(ok)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn socket_has_readable_writable_state() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var net = require('net');
            var s = new net.Socket({});
            var ok = typeof s._readableState === 'object';
            ok = ok && typeof s._writableState === 'object';
            ok = ok && s.readable === true;
            ok = ok && s.writable === true;
            String(ok)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn socket_set_timeout_no_delay_keep_alive() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var net = require('net');
            var s = new net.Socket({});
            s.setTimeout(0);
            s.setNoDelay();
            s.setKeepAlive();
            'ok'
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn tls_socket_instanceof_duplex() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var tls = require('tls');
            var stream = require('stream');
            var s = new tls.TLSSocket({}, {});
            String(s instanceof stream.Duplex)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}
