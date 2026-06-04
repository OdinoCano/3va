// Tests for Node.js-style streams and WHATWG Streams API.
// Run: cargo test -p vvva_js --test stream_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

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

// ── require('stream') ─────────────────────────────────────────────────────────

#[tokio::test]
async fn stream_module_loads() {
    let e = engine().await;
    let r = e.eval_to_string("typeof require('stream')").await.unwrap();
    // Node.js returns a Stream constructor (function), so typeof is "function".
    // Accept both "function" and "object" — just not "undefined".
    assert!(
        r == "function" || r == "object",
        "require('stream') should load: got typeof = {r}"
    );
}

#[tokio::test]
async fn stream_has_readable_writable_transform() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var s = require('stream');
            String(typeof s.Readable === 'function') + ',' +
            String(typeof s.Writable === 'function') + ',' +
            String(typeof s.Transform === 'function') + ',' +
            String(typeof s.PassThrough === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true,true,true,true");
}

// ── Readable stream ───────────────────────────────────────────────────────────

#[tokio::test]
async fn readable_push_emits_data_event() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var Readable = require('stream').Readable;
            var r = new Readable();
            var received = '';
            r.on('data', function(chunk) { received += chunk; });
            r.push('hello');
            r.push(' world');
            received
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "hello world");
}

#[tokio::test]
async fn readable_push_null_emits_end() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var Readable = require('stream').Readable;
            var r = new Readable();
            var ended = false;
            r.on('end', function() { ended = true; });
            r.push(null);
            String(ended)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── Writable stream ───────────────────────────────────────────────────────────

#[tokio::test]
async fn writable_write_returns_true() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var Writable = require('stream').Writable;
            var w = new Writable();
            String(w.write('chunk'))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn writable_end_emits_finish() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var Writable = require('stream').Writable;
            var w = new Writable();
            var finished = false;
            w.on('finish', function() { finished = true; });
            w.end();
            String(finished)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── pipe ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pipe_connects_readable_to_writable() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var s = require('stream');
            var r = new s.Readable();
            var w = new s.Writable();
            var written = '';
            w.write = function(chunk) { written += chunk; return true; };
            r.pipe(w);
            r.push('foo');
            r.push('bar');
            written
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "foobar");
}

// ── Transform ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn transform_passes_data_through() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var Transform = require('stream').Transform;
            var t = new Transform();
            var out = '';
            t.on('data', function(c) { out += c; });
            t.write('a');
            t.write('b');
            out
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ab");
}

// ── WHATWG ReadableStream ─────────────────────────────────────────────────────

#[tokio::test]
async fn readable_stream_global_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof ReadableStream === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn readable_stream_reader_reads_chunks() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._rs_result = null;
        var rs = new ReadableStream({
            start: function(c) {
                c.enqueue('chunk1');
                c.enqueue('chunk2');
                c.close();
            }
        });
        var reader = rs.getReader();
        var out = '';
        function pump() {
            reader.read().then(function(res) {
                if (res.done) { globalThis._rs_result = out; return; }
                out += res.value;
                pump();
            });
        }
        pump();
        "#,
        "globalThis._rs_result",
    )
    .await;
    assert_eq!(r, "chunk1chunk2");
}

#[tokio::test]
async fn readable_stream_reader_done_after_close() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._done_flag = null;
        var rs = new ReadableStream({ start: function(c) { c.close(); } });
        rs.getReader().read().then(function(res) {
            globalThis._done_flag = String(res.done);
        });
        "#,
        "globalThis._done_flag",
    )
    .await;
    assert_eq!(r, "true");
}

// ── WHATWG WritableStream ─────────────────────────────────────────────────────

#[tokio::test]
async fn writable_stream_global_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof WritableStream === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn writable_stream_writer_write_resolves() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._ws_result = null;
        var written = [];
        var ws = new WritableStream({
            write: function(chunk) { written.push(chunk); }
        });
        var writer = ws.getWriter();
        writer.write('a').then(function() {
            return writer.write('b');
        }).then(function() {
            globalThis._ws_result = written.join(',');
        });
        "#,
        "globalThis._ws_result",
    )
    .await;
    assert_eq!(r, "a,b");
}

// ── WHATWG TransformStream ────────────────────────────────────────────────────

#[tokio::test]
async fn transform_stream_global_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof TransformStream === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn transform_stream_pipes_through_transform() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._ts_result = null;
        var ts = new TransformStream({
            transform: function(chunk, controller) {
                controller.enqueue(chunk.toUpperCase());
            }
        });
        var reader = ts.readable.getReader();
        var writer = ts.writable.getWriter();
        var out = '';
        function pump() {
            reader.read().then(function(res) {
                if (res.done) { globalThis._ts_result = out; return; }
                out += res.value;
                pump();
            });
        }
        pump();
        writer.write('hello').then(function() {
            return writer.write('world');
        }).then(function() {
            return writer.close();
        });
        "#,
        "globalThis._ts_result",
    )
    .await;
    assert_eq!(r, "HELLOWORLD");
}

// ── pipeTo ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn readable_stream_pipe_to_writable_stream() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._pipe_result = null;
        var received = [];
        var rs = new ReadableStream({
            start: function(c) { c.enqueue('x'); c.enqueue('y'); c.close(); }
        });
        var ws = new WritableStream({
            write: function(chunk) { received.push(chunk); }
        });
        rs.pipeTo(ws).then(function() {
            globalThis._pipe_result = received.join('');
        });
        "#,
        "globalThis._pipe_result",
    )
    .await;
    assert_eq!(r, "xy");
}

// ── Writable backpressure ─────────────────────────────────────────────────────

#[tokio::test]
async fn writable_backpressure_returns_false() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var w = new (require('stream').Writable)({
                highWaterMark: 4,
                write: function(c, e, cb) { setTimeout(cb, 5); }
            });
            String(w.write('hello') === false)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn writable_write_after_end_returns_false() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var w = new (require('stream').Writable)({
                write: function(c, e, cb) { cb(); }
            });
            w.end();
            String(w.write('test') === false)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn writable_cork_delays_write() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var calls = [];
            var w = new (require('stream').Writable)({
                write: function(c, e, cb) { calls.push('write:' + c); cb(); }
            });
            w.cork();
            w.write('a');
            w.write('b');
            w.uncork();
            calls.join(',')
            "#,
        )
        .await
        .unwrap();
    // After uncork, both writes should flush
    assert_eq!(r, "write:a,write:b");
}

#[tokio::test]
async fn writable_drain_emitted_after_backpressure() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var drained = false;
            var w = new (require('stream').Writable)({
                highWaterMark: 4,
                write: function(c, e, cb) {
                    cb();
                }
            });
            w.on('drain', function() { drained = true; });
            var ret = w.write('hello');
            // drain fires synchronously because _write calls cb immediately
            // and ret was captured as false before _write
            String(ret === false && drained === true)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}
