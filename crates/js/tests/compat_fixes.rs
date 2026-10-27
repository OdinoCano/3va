// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Tests for the Node.js compatibility fixes introduced in this batch:
//   - Buffer.isBuffer() matching Node (true only for actual Buffers)
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

async fn eval_async_result(e: &mut JsEngine, setup: &str, var: &str) -> String {
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
async fn buffer_is_buffer_rejects_plain_uint8array() {
    // Node's Buffer.isBuffer is true only for actual Buffers, not for every
    // Uint8Array. The old `instanceof Buffer || instanceof Uint8Array` made the
    // conversion guards in Readable.from/fromWeb unreachable.
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.isBuffer(new Uint8Array(4)))")
        .await
        .unwrap();
    assert_eq!(r, "false");
}

#[tokio::test]
async fn buffer_is_buffer_accepts_buffer_instance() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.isBuffer(Buffer.from('hello')))")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn buffer_is_buffer_rejects_string() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.isBuffer('hello'))")
        .await
        .unwrap();
    assert_eq!(r, "false");
}

#[tokio::test]
async fn buffer_is_buffer_rejects_plain_object() {
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(Buffer.isBuffer({}))")
        .await
        .unwrap();
    assert_eq!(r, "false");
}

// ── crypto.webcrypto ──────────────────────────────────────────────────────────

#[tokio::test]
async fn crypto_webcrypto_exists() {
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
    let result = eval_async_result(
        &mut e,
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
async fn generate_keypair_rsa_rejects_oversized_modulus_length() {
    // Regression test: modulusLength used to be passed straight to RSA
    // keygen unbounded — a script requesting an absurd size could tie up a
    // blocking-pool thread for an unbounded amount of time/memory.
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('crypto');
            try {
                c.generateKeyPairSync('rsa', { modulusLength: 999999999 });
                'no error thrown';
            } catch (e) {
                'threw:' + (e.message.indexOf('modulusLength') !== -1);
            }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw:true");
}

#[tokio::test]
async fn generate_keypair_ec_p256_sync() {
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine_with_spawn().await;
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
    let mut e = engine_with_spawn().await;
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
    let mut e = engine_with_spawn().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
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
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
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
    let mut e = engine().await;
    // Emit into buffer first, then read via iter.next()
    let r = eval_async_result(
        &mut e,
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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

// WebAssembly.instantiate() compiles on a V8 background thread and settles
// through a foreground task; the event loop used to exit before it did, so
// the promise never resolved.
#[tokio::test]
async fn async_webassembly_instantiate_settles() {
    let mut e = JsEngine::new(Arc::new(vvva_permissions::PermissionState::new()))
        .await
        .unwrap();
    // (module (func (export "f") (result i32) i32.const 42))
    e.eval(
        "globalThis.__r = 'pending'; \
         WebAssembly.instantiate(new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,7,5,1,1,102,0,0,10,6,1,4,0,65,42,11])) \
           .then(r => { globalThis.__r = String(r.instance.exports.f()); }, e => { globalThis.__r = 'error: ' + e; });",
    )
    .await
    .unwrap();
    e.run_event_loop().await.unwrap();
    assert_eq!(e.eval_to_string("globalThis.__r").await.unwrap(), "42");
}

/// util.parseArgs must honour `short` options (-f, -abc, -n 3, -n3), not just tokenise them.
#[tokio::test]
async fn util_parse_args_short_options() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"(function(){
              var o = { f:{type:'boolean',short:'f'}, g:{type:'boolean',short:'g'}, n:{type:'string',short:'n'} };
              var a = require('util').parseArgs({ args:['-fg','-n','3'], options:o });
              var b = require('util').parseArgs({ args:['-n3','-f'], options:o });
              return JSON.stringify([a.values, b.values]);
            })()"#,
        )
        .await
        .unwrap();
    assert_eq!(r, r#"[{"f":true,"g":true,"n":"3"},{"n":"3","f":true}]"#);
}

/// pg / pg-pool call `socket.ref()` / `socket.unref()` on idle connections; a missing method
/// made every `pg.Pool` query throw `this.stream.ref is not a function`.
#[tokio::test]
async fn net_socket_has_ref_and_unref() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            "var n = require('net'); var s = new n.Socket(); \
             [typeof s.ref, typeof s.unref, s.ref() === s, s.unref() === s].join(',')",
        )
        .await
        .unwrap();
    assert_eq!(r, "function,function,true,true");
}

/// pbkdf2/scrypt must return a Buffer (hex/base64 `toString`), not a bare Uint8Array:
/// `toString("hex")` on a Uint8Array gives "1,2,3" and broke postgres.js SCRAM auth.
#[tokio::test]
async fn pbkdf2_and_scrypt_return_buffers() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            "var c = require('crypto'); \
             [Buffer.isBuffer(c.pbkdf2Sync('a','b',1,8,'sha256')), \
              c.pbkdf2Sync('a','b',1,8,'sha256').toString('hex').length, \
              Buffer.isBuffer(c.scryptSync('a','b',32,{N:1024})), \
              Buffer.isBuffer(c.createHash('sha256').update('x').digest()), \
              c.createHmac('sha256','k').update('x').digest().toString('hex').length, \
              c.scryptSync('a','b',8,{N:1024}).length].join(',')",
        )
        .await
        .unwrap();
    assert_eq!(r, "true,16,true,true,64,8");
}

/// Node's internal per-encoding Buffer slice helpers, which `busboy`/`dicer`
/// (and so every `multipart/form-data` upload through `multer`) call while
/// scanning boundaries. Without `latin1Slice` uploads failed with
/// "chunk.latin1Slice is not a function".
#[tokio::test]
async fn buffer_has_internal_slice_helpers() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = Buffer.from([0x41, 0xff, 0x42, 0x43]);
            [
              b.latin1Slice(1, 3),
              b.utf8Slice(0, 1),
              b.hexSlice(0, 2),
              b.base64Slice(0, 1),
              typeof b.asciiSlice,
              typeof b.base64urlSlice,
              typeof b.ucs2Slice,
              typeof b.utf16leSlice
            ].join('|')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "\u{ff}B|A|41ff|QQ==|function|function|function|function");
}

// events.listenerCount(emitter, type) accepts an EventTarget too, not just an
// EventEmitter: `listenerCount(signal, 'abort')` is how libraries size the
// bookkeeping they keep for an AbortSignal (test-http-agent-abort-controller).
#[tokio::test]
async fn listener_count_on_abort_signal() {
    let mut e = engine().await;
    e.eval(
        r#"
        var EE = require('events');
        var ac = new AbortController();
        globalThis.__before = EE.listenerCount(ac.signal, 'abort');
        ac.signal.addEventListener('abort', function() {});
        globalThis.__after = EE.listenerCount(ac.signal, 'abort');
        ac.signal.removeEventListener('abort', function() {});
        globalThis.__removed = EE.listenerCount(ac.signal, 'abort');
        "#,
    )
    .await
    .unwrap();
    let r = e
        .eval_to_string("globalThis.__before + ',' + globalThis.__after")
        .await
        .unwrap();
    assert_eq!(
        r, "0,1",
        "listenerCount must count an AbortSignal's abort listeners"
    );
}

// assert.throws / assert.rejects check the error the way Node's expectedException
// does: a class (instanceof; a different Error class fails), a validation
// function (called, must return true), a RegExp and an object whose own keys
// must match. Before, an object matched ANY error and a validation function made
// `e instanceof fn` throw, so tests that expect a specific `code` passed by accident.
#[tokio::test]
async fn assert_throws_checks_the_expected_error_like_node() {
    let mut e = engine().await;
    let got = e
        .eval_to_string(
            r#"
            var assert = require('assert');
            var out = [];
            function t(name, fn) {
                try { fn(); out.push(name + ':ok'); }
                catch (err) { out.push(name + ':' + err.name); }
            }
            t('class', function() { assert.throws(function() { throw new TypeError('x'); }, TypeError); });
            t('validator-true', function() { assert.throws(function() { throw new Error('boom'); }, function(e) { return e.message === 'boom'; }); });
            t('regex', function() { assert.throws(function() { throw new Error('boom 12'); }, /boom \d+/); });
            t('object-ok', function() {
                assert.throws(function() { var e = new TypeError('m'); e.code = 'X'; throw e; }, { code: 'X', name: 'TypeError' });
            });
            t('object-message-regex', function() { assert.throws(function() { throw new Error('abc 1'); }, { message: /abc \d/ }); });
            t('class-mismatch', function() { assert.throws(function() { throw new RangeError('x'); }, TypeError); });
            t('validator-false', function() { assert.throws(function() { throw new Error('boom'); }, function() { return false; }); });
            t('regex-mismatch', function() { assert.throws(function() { throw new Error('zzz'); }, /boom/); });
            t('object-code-mismatch', function() {
                assert.throws(function() { var e = new Error('m'); e.code = 'Y'; throw e; }, { code: 'X' });
            });
            t('no-throw', function() { assert.throws(function() {}, TypeError); });
            out.join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        got,
        "class:ok,validator-true:ok,regex:ok,object-ok:ok,object-message-regex:ok,\
         class-mismatch:AssertionError,validator-false:AssertionError,regex-mismatch:AssertionError,\
         object-code-mismatch:AssertionError,no-throw:AssertionError"
    );
}
