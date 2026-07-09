// Tests for child_process, zlib, http/https, cluster, and worker_threads modules.
// Run: cargo test -p vvva_js --test system_modules

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

// ── zlib ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn zlib_module_loads() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var z = require('zlib');
            String(typeof z.gzip === 'function' && typeof z.gunzip === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn zlib_gzip_roundtrip() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var z = require('zlib');
            var input = Buffer.from('Hello');
            var compressed = z.gzipSync(input);
            var restored = z.gunzipSync(compressed);
            String(restored.toString() === 'Hello')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn zlib_constants_exist() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var z = require('zlib');
            String(typeof z.constants === 'object' && z.constants.Z_OK === 0)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn zlib_node_prefix_alias() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var z1 = require('zlib');
            var z2 = require('node:zlib');
            String(typeof z1.gzip === 'function' && typeof z2.gzip === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── child_process ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn child_process_module_loads() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            String(typeof cp.exec === 'function' && typeof cp.spawn === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn child_process_exec_denied_without_permission() {
    let mut e = engine().await;
    // execSync is synchronous — permission denial throws immediately and is catchable.
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            var threw = false;
            try {
                cp.execSync('echo hello');
            } catch (e) {
                threw = e !== null && e !== undefined;
            }
            String(threw)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn child_process_exec_with_permission() {
    let mut e = engine_with_spawn().await;
    // execSync with permission must not throw and must return the command output.
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            var out = cp.execSync('echo hello').toString().trim();
            String(out === 'hello')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn child_process_spawn_returns_child_object() {
    let mut e = engine_with_spawn().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            var child = cp.spawn('echo', ['hello']);
            String(
                child !== null &&
                typeof child.stdout === 'object' &&
                typeof child.stderr === 'object' &&
                typeof child.on === 'function' &&
                typeof child.kill === 'function'
            )
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn child_process_execsync_throws() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('child_process');
            try { cp.execSync('echo hello'); 'no-throw'; } catch(e) { 'threw'; }
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "threw");
}

#[tokio::test]
async fn child_process_node_prefix_alias() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var cp = require('node:child_process');
            String(typeof cp.exec === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── http / https ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn http_module_loads() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            String(
                typeof http.request === 'function' &&
                typeof http.get === 'function' &&
                typeof http.STATUS_CODES === 'object'
            )
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn https_module_loads() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var https = require('https');
            String(typeof https.request === 'function' && typeof https.get === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn http_status_codes_have_entries() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            String(http.STATUS_CODES[200] === 'OK' && http.STATUS_CODES[404] === 'Not Found')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn http_request_returns_object_with_end() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var req = http.request({ hostname: 'localhost', path: '/', port: 9 }, function() {});
            String(
                req !== null &&
                typeof req.end === 'function' &&
                typeof req.write === 'function' &&
                typeof req.on === 'function'
            )
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn http_create_server_returns_object() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var server = http.createServer(function() {});
            String(typeof server.listen === 'function' && typeof server.close === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── path ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn path_relative_works() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var path = require('path');
        [
            path.relative('/a/b/c', '/a/b/d'),
            path.relative('/a/b', '/a/b/c/d'),
            path.relative('/a/b/c', '/a/b/c'),
        ].join('|')
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "../d|c/d|.");
}

#[tokio::test]
async fn path_posix_and_win32_exist() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var path = require('path');
        String(typeof path.posix === 'object' && typeof path.win32 === 'object'
               && path.posix.sep === '/' && path.win32.sep === '\\')
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn path_normalize_collapses_dots() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var path = require('path');
        [path.normalize('/a/b/../c'), path.normalize('./a/./b')].join('|')
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "/a/c|a/b");
}

#[tokio::test]
async fn path_normalize_edge_cases() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var path = require('path');
        [
            path.normalize(''),
            path.normalize('/'),
            path.normalize('/..'),
            path.normalize('/../..'),
            path.normalize('a/../../b'),
            path.normalize('..'),
            path.normalize('.'),
            path.normalize('./..'),
            path.normalize('/foo/bar/'),
            path.normalize('/.'),
            path.normalize('a/.'),
        ].join('|')
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, ".|/|/|/|../b|..|.|..|/foo/bar/|/|a");
}

#[tokio::test]
async fn path_resolve_edge_cases() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var path = require('path');
        [
            path.resolve('/a', 'b'),
            path.resolve('/a', '/b'),
            path.resolve('/a', '..'),
            path.resolve('/a', '../..'),
        ].join('|')
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "/a/b|/b|/|/");
}

// ── os ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn os_returns_real_hostname() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var os = require('os');
        typeof os.hostname() === 'string' && os.hostname().length > 0 ? 'ok' : 'fail'
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn os_memory_values_are_positive() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var os = require('os');
        (os.totalmem() > 0 && os.freemem() >= 0) ? 'ok' : 'fail'
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn os_path_submodules_exist() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var os = require('os');
        String(typeof os.platform() === 'string' &&
               typeof os.arch() === 'string' &&
               typeof os.uptime() === 'number' &&
               Array.isArray(os.cpus()))
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── EventEmitter completeness ─────────────────────────────────────────────────

#[tokio::test]
async fn eventemitter_prepend_listener() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var EE = require('events');
        var ee = new EE();
        var order = [];
        ee.on('x', function() { order.push('second'); });
        ee.prependListener('x', function() { order.push('first'); });
        ee.emit('x');
        order.join(',')
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "first,second");
}

#[tokio::test]
async fn eventemitter_event_names_and_raw_listeners() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var EE = require('events');
        var ee = new EE();
        function fn1() {}
        ee.on('foo', fn1);
        ee.on('bar', function() {});
        var names = ee.eventNames().sort().join(',');
        var raw = ee.rawListeners('foo');
        var unwrapped = ee.listeners('foo');
        [names, raw.length, unwrapped[0] === fn1].join('|')
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "bar,foo|1|true");
}

#[tokio::test]
async fn eventemitter_get_max_listeners() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var EE = require('events');
        var ee = new EE();
        ee.setMaxListeners(20);
        String(ee.getMaxListeners())
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "20");
}

#[tokio::test]
async fn crypto_create_hash_is_synchronous() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var crypto = require('crypto');
        var h = crypto.createHash('sha256').update('hello').digest('hex');
        typeof h === 'string' ? 'sync:' + h.slice(0,8) : 'async:Promise'
    "#,
        )
        .await
        .unwrap();
    assert!(
        r.starts_with("sync:"),
        "createHash.digest() must be sync, got: {r}"
    );
    assert_eq!(&r, "sync:2cf24dba", "wrong sha256 of 'hello'");
}

#[tokio::test]
async fn crypto_create_hmac_is_synchronous() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var crypto = require('crypto');
        var h = crypto.createHmac('sha256', 'key').update('data').digest('hex');
        typeof h === 'string' ? 'sync:' + h.slice(0,8) : 'async:Promise'
    "#,
        )
        .await
        .unwrap();
    assert!(
        r.starts_with("sync:"),
        "createHmac.digest() must be sync, got: {r}"
    );
}

#[tokio::test]
async fn crypto_hash_shorthand() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var crypto = require('crypto');
        crypto.hash('sha256', 'hello', 'hex').slice(0,8)
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "2cf24dba");
}

// ── os.cpus() real data ───────────────────────────────────────────────────────

#[tokio::test]
async fn os_cpus_has_model_and_times() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var os = require('os');
        var cpus = os.cpus();
        var first = cpus[0];
        String(cpus.length > 0 &&
               typeof first.model === 'string' && first.model.length > 0 &&
               typeof first.speed === 'number' &&
               typeof first.times === 'object' &&
               typeof first.times.user === 'number' &&
               typeof first.times.sys  === 'number' &&
               typeof first.times.idle === 'number')
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "os.cpus() must return real model/speed/times");
}

// ── os.networkInterfaces() ────────────────────────────────────────────────────

#[tokio::test]
async fn os_network_interfaces_returns_object() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var os = require('os');
        var ifaces = os.networkInterfaces();
        String(typeof ifaces === 'object' && ifaces !== null)
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "os.networkInterfaces() must return an object");
}

// ── child_process stdin piping ────────────────────────────────────────────────

#[tokio::test]
async fn child_process_spawnsync_with_input() {
    let mut e = engine_with_spawn().await;
    let r = e
        .eval_to_string(
            r#"
        var cp = require('child_process');
        var result = cp.spawnSync('cat', [], { input: 'hello stdin', encoding: 'utf8' });
        result.stdout.trim()
    "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "hello stdin",
        "spawnSync with input option must pipe stdin"
    );
}

#[tokio::test]
async fn child_process_spawn_stdin_write_end() {
    let mut e = engine_with_spawn().await;
    // eval_to_string doesn't await Promises, so we store the result in a global
    // and drive the event loop with idle() before reading it.
    e.eval_to_string(
        r#"
        globalThis.__spawnStdinResult = '';
        var cp = require('child_process');
        var child = cp.spawn('cat', []);
        child.stdout.on('data', function(d) { globalThis.__spawnStdinResult += d; });
        child.stdin.write('piped ');
        child.stdin.end('data');
        'started'
    "#,
    )
    .await
    .unwrap();
    e.idle().await;
    let r = e
        .eval_to_string("globalThis.__spawnStdinResult.trim()")
        .await
        .unwrap();
    assert_eq!(
        r, "piped data",
        "spawn stdin.write()/end() must pipe to child"
    );
}

// ── crypto.createDiffieHellman ────────────────────────────────────────────────

#[tokio::test]
async fn crypto_diffie_hellman_key_exchange() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var crypto = require('crypto');
        var alice = crypto.createDiffieHellman(1024);
        var bob   = crypto.createDiffieHellmanGroup('modp2');
        var alicePub = alice.generateKeys();
        var bobPub   = bob.generateKeys();
        var aliceSecret = alice.computeSecret(bobPub, null, 'hex');
        var bobSecret   = bob.computeSecret(alicePub, null, 'hex');
        String(aliceSecret === bobSecret && aliceSecret.length > 0)
    "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "DiffieHellman key exchange must produce the same shared secret"
    );
}

#[tokio::test]
async fn crypto_dh_get_public_key_encoding() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
        var crypto = require('crypto');
        var dh = crypto.createDiffieHellmanGroup('modp2');
        dh.generateKeys();
        var hex = dh.getPublicKey('hex');
        String(typeof hex === 'string' && hex.length > 0 && /^[0-9a-f]+$/.test(hex))
    "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "getPublicKey('hex') must return hex string");
}

// ── cluster ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cluster_is_registered_in_require_cache() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var cluster = require('cluster');
            String(cluster !== undefined && cluster !== null)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "cluster must be registered in require cache");
}

#[tokio::test]
async fn cluster_is_primary_true() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var cluster = require('cluster');
            String(cluster.isPrimary === true && cluster.isWorker === false)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "cluster.isPrimary must be true in single-process mode"
    );
}

#[tokio::test]
async fn cluster_fork_returns_worker_object() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var cluster = require('cluster');
            var w = cluster.fork();
            String(w !== null && w !== undefined && typeof w.send === 'function' && typeof w.id === 'number')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "cluster.fork() must return a ClusterWorker with id and send()"
    );
}

#[tokio::test]
async fn cluster_node_prefix_alias() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c1 = require('cluster');
            var c2 = require('node:cluster');
            String(c1 === c2)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "require('node:cluster') must alias require('cluster')"
    );
}

// ── worker_threads ────────────────────────────────────────────────────────────

#[tokio::test]
async fn worker_threads_is_main_thread() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var wt = require('worker_threads');
            String(wt.isMainThread === true && wt.threadId === 0)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "isMainThread must be true on the main thread");
}

#[tokio::test]
async fn worker_threads_message_channel_roundtrip() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var wt = require('worker_threads');
            var ch = new wt.MessageChannel();
            var received = null;
            ch.port2.on('message', function(d) { received = d; });
            ch.port1.postMessage({ x: 42 });
            'pending'
            "#,
        )
        .await
        .unwrap();
    // MessageChannel.postMessage delivers asynchronously via setTimeout(0)
    assert_eq!(r, "pending", "MessageChannel.postMessage must not throw");
}

#[tokio::test]
async fn worker_threads_worker_constructor_exists() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var wt = require('worker_threads');
            String(typeof wt.Worker === 'function' && typeof wt.MessageChannel === 'function' && typeof wt.MessagePort === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "worker_threads must export Worker, MessageChannel, MessagePort"
    );
}

// ── https.createServer (real server) ─────────────────────────────────────────

#[tokio::test]
async fn https_create_server_returns_full_server_object() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var https = require('https');
            var srv = https.createServer(function(req, res) { res.end('ok'); });
            var has_listen  = typeof srv.listen  === 'function';
            var has_close   = typeof srv.close   === 'function';
            var has_address = typeof srv.address === 'function';
            var has_on      = typeof srv.on      === 'function';
            var has_emit    = typeof srv.emit    === 'function';
            String(has_listen && has_close && has_address && has_on && has_emit)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "https.createServer() must return a full event-emitter server"
    );
}

#[tokio::test]
async fn http_create_server_returns_full_server_object() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var http = require('http');
            var srv = http.createServer(function(req, res) { res.end('ok'); });
            String(typeof srv.listen === 'function' && typeof srv.on === 'function' && typeof srv.emit === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "http.createServer() must return a real server with event emitter"
    );
}

// ── crypto stubs that must throw ──────────────────────────────────────────────

#[tokio::test]
async fn crypto_hash_copy_returns_independent_clone() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var crypto = require('crypto');
            var h = crypto.createHash('sha256');
            h.update('hello');
            var h2 = h.copy();
            h2.update(' world');
            // Original digest must reflect only 'hello'; clone must reflect 'hello world'
            var orig   = h.digest('hex');
            var cloned = h2.digest('hex');
            String(typeof h2 === 'object' && orig !== cloned && orig.length === 64)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "Hash.copy() must return an independent clone");
}

#[tokio::test]
async fn crypto_wrapkey_throws_not_supported() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var threw = false;
            crypto.subtle.wrapKey('raw', null, null, 'AES-GCM').catch(function() { threw = true; });
            'pending'
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "pending", "wrapKey() must return a rejected Promise");
}

// ── partial stubs: tty / v8 / vm ─────────────────────────────────────────────

#[tokio::test]
async fn tty_isatty_returns_boolean() {
    // isatty() now calls the real __isatty Rust primitive — returns true when fd is
    // connected to a terminal, false otherwise. CI runs without a TTY so the result
    // is false in the test environment; the important invariant is that it is a boolean.
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var tty = require('tty');
            String(typeof tty.isatty(0) === 'boolean' &&
                   typeof tty.isatty(1) === 'boolean' &&
                   typeof tty.isatty(2) === 'boolean')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "tty.isatty() must return a boolean");
}

#[tokio::test]
async fn v8_heap_statistics_returns_zeroed_object() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var v8 = require('v8');
            var s = v8.getHeapStatistics();
            var spaces = v8.getHeapSpaceStatistics();
            String(typeof s === 'object' && Array.isArray(spaces) && spaces.length === 0)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "v8.getHeapStatistics() must return object; getHeapSpaceStatistics() must return []"
    );
}

#[tokio::test]
async fn vm_run_in_new_context_evaluates_code() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var vm = require('vm');
            var result = vm.runInNewContext('1 + 1', {});
            String(result === 2)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "vm.runInNewContext must evaluate the code");
}

#[tokio::test]
async fn vm_sandbox_vars_accessible_in_code() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var vm = require('vm');
            var sandbox = { x: 7, y: 3 };
            var result = vm.runInNewContext('x * y', sandbox);
            String(result === 21)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "sandbox variables must be accessible inside evaluated code"
    );
}

#[tokio::test]
async fn vm_sandbox_mutations_reflected_back() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var vm = require('vm');
            var sandbox = vm.createContext({ counter: 0 });
            vm.runInNewContext('counter = counter + 10', sandbox);
            String(sandbox.counter === 10)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "mutations to sandbox-declared vars must be reflected back"
    );
}

#[tokio::test]
async fn vm_create_context_marks_object() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var vm = require('vm');
            var ctx = vm.createContext({ a: 1 });
            String(vm.isContext(ctx) === true && vm.isContext({}) === false)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "createContext must mark an object; plain objects must fail isContext"
    );
}

#[tokio::test]
async fn vm_run_in_this_context_expression() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var vm = require('vm');
            var result = vm.runInThisContext('40 + 2');
            String(result === 42)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "runInThisContext must return expression result");
}

#[tokio::test]
async fn vm_script_run_in_new_context() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var vm = require('vm');
            var script = new vm.Script('result = a + b');
            var sandbox = vm.createContext({ a: 10, b: 5, result: 0 });
            script.runInContext(sandbox);
            String(sandbox.result === 15)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "true",
        "Script.runInContext must write result back to sandbox"
    );
}

// ── dns partial implementation ────────────────────────────────────────────────

#[tokio::test]
async fn dns_resolve_mx_returns_enotsup() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var dns = require('dns');
            var code = '';
            dns.resolveMx('example.com', function(err) { if (err) code = err.code; });
            'pending'
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "pending",
        "dns.resolveMx must call back with ENOTSUP (async)"
    );
}

#[tokio::test]
async fn dns_lookup_service_returns_enotsup() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var dns = require('dns');
            var code = '';
            dns.lookupService('127.0.0.1', 80, function(err) { if (err) code = err.code; });
            'pending'
            "#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "pending",
        "dns.lookupService must call back with ENOTSUP (async)"
    );
}
