// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Tests for Node.js-style streams and WHATWG Streams API.
// Run: cargo test -p vvva_js --test stream_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

async fn eval_async_result(e: &mut JsEngine, setup: &str, result_global: &str) -> String {
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(typeof ReadableStream === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn readable_stream_reader_reads_chunks() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
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
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
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
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(typeof WritableStream === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn writable_stream_writer_write_resolves() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
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
    let mut e = engine().await;
    let r = e
        .eval_to_string("String(typeof TransformStream === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn transform_stream_pipes_through_transform() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
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
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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
    let mut e = engine().await;
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

// ── Readable.from / fromWeb / toWeb and pipeline with async generators ────────

/// Drive the engine (timers + microtasks) until a global expression turns
/// truthy, or the deadline lapses.
async fn pump_until(
    e: &mut JsEngine,
    done_js: &str,
    timeout: std::time::Duration,
) -> Option<String> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let v = e.eval_to_string(done_js).await.unwrap_or_default();
        if v != "false" && !v.is_empty() {
            return Some(v);
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
    None
}

#[tokio::test]
async fn readable_from_sync_iterable() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var Readable = require('stream').Readable;
        globalThis.__rfrom = null;
        Readable.from(['a', 'b', 'c']).on('data', function(c) {
            globalThis.__rfrom = (globalThis.__rfrom === null ? '' : globalThis.__rfrom) + c;
        });
        "#,
        "globalThis.__rfrom",
    )
    .await;
    assert_eq!(r, "abc");
}

#[tokio::test]
async fn readable_from_async_generator() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var Readable = require('stream').Readable;
        globalThis.__rfrom = null;
        async function* gen() { yield 'x'; yield 'y'; }
        Readable.from(gen()).on('data', function(c) {
            globalThis.__rfrom = (globalThis.__rfrom === null ? '' : globalThis.__rfrom) + c;
        });
        "#,
        "globalThis.__rfrom",
    )
    .await;
    assert_eq!(r, "xy");
}

#[tokio::test]
async fn readable_from_web_and_to_web_roundtrip() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var Readable = require('stream').Readable;
        globalThis.__rweb = null;
        var web = new ReadableStream({
            start: function(c) { c.enqueue('p'); c.enqueue('q'); c.close(); }
        });
        var node = Readable.fromWeb(web);
        // And the inverse: a Node readable exposed as a web stream.
        var back = Readable.toWeb(Readable.from(['z']));
        var first = '';
        node.on('data', function(c) {
            globalThis.__rweb = (globalThis.__rweb === null ? '' : globalThis.__rweb) + c;
        });
        node.on('end', function() {
            back.getReader().read().then(function(res) {
                globalThis.__rweb = globalThis.__rweb + '/' + String(res.value);
            });
        });
        "#,
        "globalThis.__rweb",
    )
    .await;
    assert_eq!(r, "pq/z");
}

#[tokio::test]
async fn readable_from_and_to_web_are_functions() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var R = require('stream').Readable;
            [typeof R.from, typeof R.fromWeb, typeof R.toWeb].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "function,function,function");
}

#[tokio::test]
async fn pipeline_with_async_generator_transform() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var stream = require('stream');
        var pipeline = require('stream/promises').pipeline;
        globalThis.__pipe = null;
        var out = [];
        var dst = new stream.Writable({ write: function(c, enc, cb) { out.push(String(c)); cb(); } });
        pipeline(
            stream.Readable.from(['a', 'b', 'c']),
            async function*(s) { for await (var c of s) yield String(c).toUpperCase(); },
            dst
        ).then(function() { globalThis.__pipe = out.join(''); },
                function(err) { globalThis.__pipe = 'ERR:' + err.message; });
        "#,
        "globalThis.__pipe",
    )
    .await;
    assert_eq!(r, "ABC");
}

/// The exact shape from the bug report: a file piped through an async
/// generator transform into another file.
#[tokio::test]
async fn pipeline_file_through_async_generator() {
    let dir = tempfile::tempdir().unwrap();
    let in_path = dir.path().join("in.txt");
    let out_path = dir.path().join("out.txt");
    std::fs::write(&in_path, "hello world\n").unwrap();

    let state = PermissionState::new();
    state.grant(Capability::FileRead(dir.path().to_path_buf()));
    state.grant(Capability::FileWrite(dir.path().to_path_buf()));
    let mut e = JsEngine::new(Arc::new(state)).await.unwrap();

    e.eval(&format!(
        r#"
        var fs = require('fs');
        var pipeline = require('stream/promises').pipeline;
        globalThis.__pfile = null;
        pipeline(
            fs.createReadStream({inp:?}),
            async function*(s) {{ for await (var c of s) yield c; }},
            fs.createWriteStream({outp:?})
        ).then(function() {{ globalThis.__pfile = 'ok'; }},
                function(err) {{ globalThis.__pfile = 'ERR:' + err.message; }});
        "#,
        inp = in_path.to_string_lossy(),
        outp = out_path.to_string_lossy(),
    ))
    .await
    .unwrap();

    let done = pump_until(
        &mut e,
        "globalThis.__pfile !== null ? globalThis.__pfile : 'false'",
        std::time::Duration::from_secs(10),
    )
    .await;
    assert_eq!(done.as_deref(), Some("ok"), "pipeline failed");

    let written = std::fs::read_to_string(&out_path).unwrap();
    assert_eq!(written, "hello world\n");
}
