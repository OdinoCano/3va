// Tests for the Node.js compatibility fixes introduced in this batch:
//   - Buffer.isBuffer() accepting raw Uint8Array
//   - crypto.webcrypto wrapper
//   - crypto.scryptSync using real scrypt
//   - crypto.generateKeyPair / generateKeyPairSync (RSA, EC)
//   - util.inspect (circular refs, Symbol.for custom)
//   - util.parseArgs
//   - reflect-metadata polyfill
//   - child_process.execSync / spawnSync
//   - EventEmitter.once / EventEmitter.on static helpers  (v2.0.3)
//   - http.globalAgent                                   (v2.0.3)
//   - process.resourceUsage()                            (v2.0.3)
//
// Run: cargo test -p vvva_js --test compat_fixes

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

async fn engine_with_spawn() -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::SpawnProcess);
    JsEngine::new(Arc::new(state)).await.unwrap()
}

async fn eval_async_result(e: &JsEngine, setup: &str, var: &str) -> String {
    e.eval(setup).await.unwrap();
    for _ in 0..100 {
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

// ── Buffer.isBuffer ───────────────────────────────────────────────────────────

#[tokio::test]
async fn buffer_is_buffer_accepts_uint8array() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.isBuffer(new Uint8Array(4)))")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_is_buffer_accepts_buffer_instance() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.isBuffer(Buffer.from('hello')))")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_is_buffer_rejects_string() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.isBuffer('hello'))")
        .await
        .unwrap();
    assert_eq!(r, "false");
}

#[tokio::test]
async fn buffer_is_buffer_rejects_plain_object() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.isBuffer({}))")
        .await
        .unwrap();
    assert_eq!(r, "false");
}

// ── crypto.webcrypto ──────────────────────────────────────────────────────────

#[tokio::test]
async fn crypto_webcrypto_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            String(typeof c.webcrypto === 'object' && typeof c.webcrypto.subtle === 'object')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn crypto_webcrypto_subtle_is_same_as_subtle() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            String(c.webcrypto.subtle === c.subtle)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── crypto.scryptSync ─────────────────────────────────────────────────────────

#[tokio::test]
async fn scrypt_sync_produces_correct_length() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var key = c.scryptSync('password', 'salt', 32);
            String(key instanceof Uint8Array && key.length === 32)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn scrypt_sync_is_deterministic() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var k1 = Array.from(c.scryptSync('pass', 'salt', 16));
            var k2 = Array.from(c.scryptSync('pass', 'salt', 16));
            String(JSON.stringify(k1) === JSON.stringify(k2))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn scrypt_sync_differs_from_pbkdf2() {
    // Real scrypt output must differ from PBKDF2 — this catches the old fallback.
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var scr  = Array.from(c.scryptSync('password', 'NaCl', 32));
            var pbk  = Array.from(c.pbkdf2Sync('password', 'NaCl', 16384, 32, 'sha256'));
            String(JSON.stringify(scr) !== JSON.stringify(pbk))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── crypto.generateKeyPair ────────────────────────────────────────────────────

#[tokio::test]
async fn generate_keypair_rsa_sync() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('rsa', { modulusLength: 1024 });
            var priv = pair.privateKey.export();
            var pub  = pair.publicKey.export();
            String(
                typeof priv === 'string' &&
                priv.indexOf('-----BEGIN PRIVATE KEY-----') === 0 &&
                typeof pub === 'string' &&
                pub.indexOf('-----BEGIN PUBLIC KEY-----') === 0
            )
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn generate_keypair_rsa_async() {
    let e = engine().await;
    let result = eval_async_result(
        &e,
        r#"
        var __result = null;
        var c = require('crypto');
        c.generateKeyPair('rsa', { modulusLength: 1024 }, function(err, pub, priv) {
            if (err) { __result = 'error:' + err.message; return; }
            __result = String(
                typeof pub.export() === 'string' &&
                pub.export().indexOf('-----BEGIN PUBLIC KEY-----') === 0
            );
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(result, "true");
}

#[tokio::test]
async fn generate_keypair_ec_p256_sync() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('ec', { namedCurve: 'P-256' });
            var priv = pair.privateKey.export();
            var pub  = pair.publicKey.export();
            String(
                priv.indexOf('-----BEGIN PRIVATE KEY-----') === 0 &&
                pub.indexOf('-----BEGIN PUBLIC KEY-----') === 0
            )
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn generate_keypair_ec_p384_sync() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var pair = c.generateKeyPairSync('ec', { namedCurve: 'P-384' });
            String(pair.privateKey.export().indexOf('-----BEGIN PRIVATE KEY-----') === 0)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn generate_keypair_rsa_unique_each_call() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            var a = c.generateKeyPairSync('rsa', { modulusLength: 1024 }).privateKey.export();
            var b = c.generateKeyPairSync('rsa', { modulusLength: 1024 }).privateKey.export();
            String(a !== b)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── util.inspect ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn util_inspect_primitives() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            String(
                u.inspect(42) === '42' &&
                u.inspect(true) === 'true' &&
                u.inspect(null) === 'null' &&
                u.inspect(undefined) === 'undefined'
            )
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn util_inspect_circular_ref() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            var obj = {};
            obj.self = obj;
            var result = u.inspect(obj);
            String(result.indexOf('[Circular *]') !== -1)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn util_inspect_custom_symbol() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            var obj = {};
            obj[Symbol.for('nodejs.util.inspect.custom')] = function() { return 'MyClass { x: 1 }'; };
            String(u.inspect(obj) === 'MyClass { x: 1 }')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn util_inspect_function() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            function myFn() {}
            String(u.inspect(myFn).indexOf('[Function: myFn]') !== -1)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── util.parseArgs ────────────────────────────────────────────────────────────

#[tokio::test]
async fn util_parse_args_basic_string_option() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            var result = u.parseArgs({
                args: ['--host', 'localhost', '--port', '3000'],
                options: { host: { type: 'string' }, port: { type: 'string' } }
            });
            String(result.values.host === 'localhost' && result.values.port === '3000')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn util_parse_args_boolean_flag() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            var result = u.parseArgs({
                args: ['--verbose'],
                options: { verbose: { type: 'boolean' } }
            });
            String(result.values.verbose === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn util_parse_args_positionals() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            var result = u.parseArgs({
                args: ['file1.txt', 'file2.txt'],
                allowPositionals: true
            });
            String(result.positionals.length === 2 &&
                   result.positionals[0] === 'file1.txt')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn util_parse_args_inline_value() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            var result = u.parseArgs({
                args: ['--name=world'],
                options: { name: { type: 'string' } }
            });
            String(result.values.name === 'world')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn util_parse_args_defaults() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = require('util');
            var result = u.parseArgs({
                args: [],
                options: { debug: { type: 'boolean', default: false } }
            });
            String(result.values.debug === false)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── reflect-metadata ──────────────────────────────────────────────────────────

#[tokio::test]
async fn reflect_metadata_define_and_get() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            require('reflect-metadata');
            function MyClass() {}
            Reflect.defineMetadata('design:type', String, MyClass, 'name');
            var got = Reflect.getMetadata('design:type', MyClass, 'name');
            String(got === String)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn reflect_metadata_decorator_pattern() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            require('reflect-metadata');
            var Injectable = Reflect.metadata('injectable', true);
            function MyService() {}
            Injectable(MyService);
            String(Reflect.getMetadata('injectable', MyService) === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn reflect_has_own_metadata() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            require('reflect-metadata');
            function Cls() {}
            Reflect.defineMetadata('k', 'v', Cls);
            String(Reflect.hasOwnMetadata('k', Cls) === true &&
                   Reflect.hasOwnMetadata('missing', Cls) === false)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn reflect_metadata_keys() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            require('reflect-metadata');
            function Svc() {}
            Reflect.defineMetadata('a', 1, Svc);
            Reflect.defineMetadata('b', 2, Svc);
            var keys = Reflect.getOwnMetadataKeys(Svc);
            String(keys.indexOf('a') !== -1 && keys.indexOf('b') !== -1)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── child_process.execSync / spawnSync ────────────────────────────────────────

#[tokio::test]
async fn exec_sync_requires_permission() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            try { cp.execSync('echo hi'); 'no-throw' } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

#[tokio::test]
async fn exec_sync_returns_output() {
    let e = engine_with_spawn().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            var out = cp.execSync('echo hello', { encoding: 'utf8' });
            String(out.trim() === 'hello')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn exec_sync_throws_on_nonzero_exit() {
    let e = engine_with_spawn().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            try { cp.execSync('exit 1', { encoding: 'utf8' }); 'no-throw' } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

#[tokio::test]
async fn spawn_sync_returns_result_object() {
    let e = engine_with_spawn().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            var result = cp.spawnSync('echo', ['world'], { encoding: 'utf8' });
            String(
                typeof result.status === 'number' &&
                result.status === 0 &&
                typeof result.stdout === 'string' &&
                result.stdout.trim() === 'world'
            )
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn spawn_sync_requires_permission() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            try { cp.spawnSync('echo', ['hi']); 'no-throw' } catch(e) { 'threw' }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

// ── EventEmitter.once static helper ───────────────────────────────────────────

#[tokio::test]
async fn event_emitter_once_static_promise() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis.__result = undefined;
        var EventEmitter = require('events');
        var ee = new EventEmitter();
        EventEmitter.once(ee, 'done').then(function(args) {
            globalThis.__result = (args[0] === 42 && args[1] === 'ok') ? 'true' : 'false';
        });
        ee.emit('done', 42, 'ok');
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "true");
}

#[tokio::test]
async fn event_emitter_once_error_rejection() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis.__result = undefined;
        var EventEmitter = require('events');
        var ee = new EventEmitter();
        EventEmitter.once(ee, 'fail').then(function() {
            globalThis.__result = 'resolved';
        }).catch(function() {
            globalThis.__result = 'rejected';
        });
        ee.emit('fail');
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "resolved");
}

#[tokio::test]
async fn event_emitter_on_async_iterator() {
    let e = engine().await;
    // Emit into buffer first, then read via iter.next()
    let r = eval_async_result(
        &e,
        r#"
        globalThis.__result = undefined;
        var EventEmitter = require('events');
        var ee = new EventEmitter();
        var iter = EventEmitter.on(ee, 'tick');
        ee.emit('tick', 'a');
        ee.emit('tick', 'b');
        iter.next().then(function(v) { globalThis.__result = v.value[0]; });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "a");
}

// ── http.globalAgent ──────────────────────────────────────────────────────────

#[tokio::test]
async fn http_global_agent_exposed() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var https = require('https');
            [typeof http.globalAgent, typeof https.globalAgent, http.globalAgent.maxSockets].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "object,object,Infinity");
}

// ── process.resourceUsage() ───────────────────────────────────────────────────

#[tokio::test]
async fn process_resource_usage_shape() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var ru = process.resourceUsage();
            [typeof ru.userCPUTime, typeof ru.systemCPUTime, typeof ru.maxRSS].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "number,number,number");
}

#[tokio::test]
async fn process_resource_usage_values_positive() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var ru = process.resourceUsage();
            [ru.userCPUTime >= 0, ru.systemCPUTime >= 0, ru.maxRSS > 0].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true,true,true");
}
