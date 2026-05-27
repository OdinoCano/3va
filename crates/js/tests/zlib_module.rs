// Tests for the zlib module: gzip/gunzip, deflate/inflate, deflateRaw/inflateRaw.
// Run: cargo test -p vvva_js --test zlib_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

/// Drive async callbacks (Promises) to completion — polls up to 50 iterations.
async fn eval_async(e: &JsEngine, setup: &str, result_global: &str) -> String {
    e.eval(setup).await.unwrap();
    for _ in 0..50 {
        e.idle().await;
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        let r = e
            .eval_to_string(&format!(
                "typeof {result_global} !== 'undefined' ? String({result_global}) : ''"
            ))
            .await
            .unwrap();
        if !r.is_empty() {
            return r;
        }
    }
    String::new()
}

// ── gzip / gunzip ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn zlib_gzip_gunzip_roundtrip() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var z = require('zlib');
        globalThis.__result = undefined;
        var input = [72, 101, 108, 108, 111]; // "Hello"
        z.gzip(input, function(err, compressed) {
            if (err) { globalThis.__result = 'gzip_err:' + err; return; }
            z.gunzip(Array.from(compressed), function(err2, decompressed) {
                if (err2) { globalThis.__result = 'gunzip_err:' + err2; return; }
                var s = '';
                for (var i = 0; i < decompressed.length; i++) s += String.fromCharCode(decompressed[i]);
                globalThis.__result = s;
            });
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "Hello");
}

#[tokio::test]
async fn zlib_gzip_produces_uint8array() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var z = require('zlib');
        globalThis.__result = undefined;
        z.gzip([1, 2, 3], function(err, out) {
            globalThis.__result = (!err && out instanceof Uint8Array && out.length > 0) ? 'ok' : 'fail';
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "ok");
}

// ── deflate / inflate ─────────────────────────────────────────────────────────

#[tokio::test]
async fn zlib_deflate_inflate_roundtrip() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var z = require('zlib');
        globalThis.__result = undefined;
        var input = [119, 111, 114, 108, 100]; // "world"
        z.deflate(input, function(err, compressed) {
            if (err) { globalThis.__result = 'deflate_err'; return; }
            z.inflate(Array.from(compressed), function(err2, out) {
                if (err2) { globalThis.__result = 'inflate_err'; return; }
                var s = '';
                for (var i = 0; i < out.length; i++) s += String.fromCharCode(out[i]);
                globalThis.__result = s;
            });
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "world");
}

#[tokio::test]
async fn zlib_deflate_compressed_is_smaller_than_repetitive_input() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var z = require('zlib');
        globalThis.__result = undefined;
        // 200 repeated bytes — highly compressible
        var input = new Array(200).fill(65);
        z.deflate(input, function(err, compressed) {
            globalThis.__result = (!err && compressed.length < input.length) ? 'smaller' : 'not_smaller';
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "smaller");
}

// ── deflateRaw / inflateRaw ───────────────────────────────────────────────────

#[tokio::test]
async fn zlib_deflate_raw_inflate_raw_roundtrip() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var z = require('zlib');
        globalThis.__result = undefined;
        var input = [102, 111, 111, 98, 97, 114]; // "foobar"
        z.deflateRaw(input, function(err, compressed) {
            if (err) { globalThis.__result = 'raw_deflate_err'; return; }
            z.inflateRaw(Array.from(compressed), function(err2, out) {
                if (err2) { globalThis.__result = 'raw_inflate_err'; return; }
                var s = '';
                for (var i = 0; i < out.length; i++) s += String.fromCharCode(out[i]);
                globalThis.__result = s;
            });
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "foobar");
}

// ── opts argument is optional ─────────────────────────────────────────────────

#[tokio::test]
async fn zlib_callback_without_opts() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var z = require('zlib');
        globalThis.__result = undefined;
        z.gzip([65], function(err, out) {
            globalThis.__result = (!err && out instanceof Uint8Array) ? 'ok' : 'fail';
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "ok");
}

// ── sync stubs throw ──────────────────────────────────────────────────────────

#[tokio::test]
async fn zlib_sync_methods_roundtrip() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var z = require('zlib');
            var ok = 0;
            try {
                var gz = z.gzipSync(new Uint8Array([104,101,108,108,111]));
                var raw = z.gunzipSync(gz);
                if (raw.length === 5 && raw[0] === 104) ok++;
            } catch(e) {}
            try {
                var df = z.deflateSync(new Uint8Array([119,111,114,108,100]));
                var raw2 = z.inflateSync(df);
                if (raw2.length === 5 && raw2[0] === 119) ok++;
            } catch(e) {}
            String(ok)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "2");
}

// ── constants ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn zlib_constants_values() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = require('zlib').constants;
            [c.Z_OK, c.Z_NO_COMPRESSION, c.Z_BEST_SPEED, c.Z_BEST_COMPRESSION, c.Z_DEFLATED].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "0,0,1,9,8");
}

// ── node: prefix ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn zlib_node_prefix_same_object() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var z1 = require('zlib');
            var z2 = require('node:zlib');
            String(z1 === z2)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── Transform streams ─────────────────────────────────────────────────────────

#[tokio::test]
async fn create_gzip_stream_pipes_data() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        globalThis.__result = undefined;
        var zlib = require('zlib');
        var gz = zlib.createGzip();
        var chunks = [];
        gz.on('data', function(c) { chunks.push(c); });
        gz.on('end', function() {
            var total = 0;
            chunks.forEach(function(c) { total += c.length; });
            globalThis.__result = total > 0 ? 'ok' : 'empty';
        });
        gz.write(new Uint8Array([104,101,108,108,111]));
        gz.end();
    "#,
        "__result",
    )
    .await;
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn create_gunzip_decompresses_gzip_data() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        globalThis.__result = undefined;
        var zlib = require('zlib');
        var compressed = zlib.gzipSync(new Uint8Array([104,101,108,108,111]));
        var gz = zlib.createGunzip();
        var out = [];
        gz.on('data', function(c) { for (var i=0;i<c.length;i++) out.push(c[i]); });
        gz.on('end', function() {
            var s = String.fromCharCode.apply(null, out);
            globalThis.__result = s === 'hello' ? 'ok' : 'got:' + s;
        });
        gz.write(compressed);
        gz.end();
    "#,
        "__result",
    )
    .await;
    assert_eq!(r, "ok");
}
