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

/// `resume()` with no `'data'` listener must drain the body and emit `'end'`
/// (Node's `flow()`: a resumed stream flows to nobody). The chunks used to be
/// buffered, and `_maybeEnd` (which requires an empty buffer) never emitted
/// `'end'`, so `res.resume()` then `'end'` hung — the idiom the http client
/// tests use to discard a body.
#[tokio::test]
async fn readable_resume_without_data_listener_emits_end() {
    let mut e = engine().await;
    e.eval(
        r#"
        var Readable = require('stream').Readable;
        globalThis.__resumed = null;
        var r = new Readable();
        r.resume();
        r.on('end', function() { globalThis.__resumed = 'ended'; });
        r.push('a');
        r.push('b');
        r.push(null);
        "#,
    )
    .await
    .unwrap();
    let r = pump_until(
        &mut e,
        "globalThis.__resumed !== null ? globalThis.__resumed : 'false'",
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("resume() then 'end' never fired");
    assert_eq!(r, "ended");
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
    e.eval_to_string(
        r#"
        globalThis.__drained = false;
        globalThis.__ret = null;
        var w = new (require('stream').Writable)({
            highWaterMark: 4,
            write: function(c, e, cb) {
                cb();
            }
        });
        w.on('drain', function() { globalThis.__drained = true; });
        globalThis.__ret = w.write('hello');
        "#,
    )
    .await
    .unwrap();
    // Node emits 'drain' on the next tick, after the caller could attach its
    // listener. A synchronous emit (the old behaviour, asserted here before)
    // lost the event for a caller that attached right after write() returned —
    // which is exactly how a socket/pipeline consumer waits.
    let pumped = pump_until(
        &mut e,
        "globalThis.__drained === true",
        std::time::Duration::from_secs(2),
    )
    .await;
    assert!(pumped.is_some(), "'drain' never fired");
    let ret = e.eval_to_string("String(globalThis.__ret)").await.unwrap();
    assert_eq!(ret, "false");
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

/// The fetch ↔ Node streams bridge: `Readable.fromWeb(response.body)` and binary bodies.
/// `Response.body` did not exist, so `fromWeb(res.body)` threw even though `fromWeb` did.
#[tokio::test]
async fn response_body_is_a_web_stream_and_binary_bodies_keep_their_bytes() {
    let mut e = engine().await;
    e.eval_to_string(
        r#"
        globalThis.__r = 'pending';
        (async function() {
            var Readable = require('stream').Readable, s = '';
            for await (var c of Readable.fromWeb(new Response('web body').body)) s += Buffer.from(c).toString();
            var bin = new Response(new Uint8Array([0, 255, 128]));
            var bytes = Array.from(new Uint8Array(await bin.arrayBuffer())).join(',');
            globalThis.__r = [s, bytes, (await new Response('xy').blob()).size, String(new Response(null).body)].join('|');
        })().catch(function(err) { globalThis.__r = 'ERR ' + err.message; });
        'ok'
        "#,
    )
    .await
    .unwrap();
    for _ in 0..50 {
        e.idle().await;
        let r = e.eval_to_string("globalThis.__r").await.unwrap();
        if r != "pending" {
            assert_eq!(r, "web body|0,255,128|2|null");
            return;
        }
    }
    panic!("response body test never finished");
}

/// `Readable.fromWeb` / `Readable.from` must emit `Buffer` chunks, not bare
/// `Uint8Array`: `Buffer.isBuffer` used to accept any Uint8Array, which made
/// the conversion guards in both bridges dead code and left `s += chunk`
/// producing "104,105" instead of "hi".
#[tokio::test]
async fn readable_from_web_and_from_emit_buffers() {
    let mut e = engine().await;
    let r = eval_async_result(
        &mut e,
        r#"
        var Readable = require('stream').Readable;
        globalThis.__bufs = null;
        (async function() {
            var out = [];
            var web = new ReadableStream({ start: function(c) { c.enqueue(new Uint8Array([104, 105])); c.close(); } });
            for await (var c of Readable.fromWeb(web)) out.push(Buffer.isBuffer(c) + ':' + c);
            for await (var d of Readable.from([new Uint8Array([104, 105])])) out.push(Buffer.isBuffer(d) + ':' + d);
            // pipeline through a generator transform must keep them Buffers too.
            var stream = require('stream');
            var pipeline = require('stream/promises').pipeline;
            var got = [];
            var dst = new stream.Writable({ write: function(c, enc, cb) { got.push(Buffer.isBuffer(c)); cb(); } });
            await pipeline(
                Readable.from([new Uint8Array([104, 105])]),
                async function*(s) { for await (var x of s) yield x; },
                dst
            );
            globalThis.__bufs = out.join(',') + '|' + got.join(',');
        })().catch(function(err) { globalThis.__bufs = 'ERR ' + err.message; });
        "#,
        "globalThis.__bufs",
    )
    .await;
    assert_eq!(r, "true:hi,true:hi|true");
}

/// `pause()` must actually stop `'data'`: pushing while paused has to buffer,
/// not emit. Libraries that pause the request while they flush to disk
/// (`formidable`) relied on this and silently lost part of the upload.
#[tokio::test]
async fn paused_stream_buffers_instead_of_emitting() {
    let mut e = engine().await;
    e.eval(
        r#"
        var Readable = require('stream').Readable;
        globalThis.__pause = null;
        var src = new Readable({ read: function() {} });
        var bad = 0, recv = 0, n = 0;
        src.on('data', function(c) {
            if (!src._readableState.flowing) bad++;
            recv += c.length;
            n++;
            if (n < 6) {
                src.pause();
                src.push(Buffer.alloc(65536, 7));   // pushed while paused
                setTimeout(function() { src.resume(); }, 1);
            } else {
                globalThis.__pause = 'recv:' + recv + '|bad:' + bad;
            }
        });
        src.push(Buffer.alloc(65536, 1));
        "#,
    )
    .await
    .unwrap();
    let r = pump_until(
        &mut e,
        "globalThis.__pause !== null ? globalThis.__pause : 'false'",
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("paused stream never delivered its chunks");
    assert_eq!(r, "recv:393216|bad:0");
}

/// `pipe()` must resume the source when a slow destination drains. Now that
/// `pause()` really stops `'data'`, a destination whose `write()` returns false
/// would otherwise stall the pipe after the first backpressured chunk.
#[tokio::test]
async fn pipe_resumes_after_destination_drain() {
    let mut e = engine().await;
    e.eval(
        r#"
        var stream = require('stream');
        globalThis.__pipebp = null;
        var got = 0;
        var w = new stream.Writable({ highWaterMark: 1024, write: function(c, e, cb) { got += c.length; setTimeout(cb, 1); } });
        var src = new stream.Readable({ read: function() {} });
        src.pipe(w);
        for (var i = 0; i < 20; i++) src.push(Buffer.alloc(1024, 3));
        src.push(null);
        w.on('finish', function() { globalThis.__pipebp = 'got:' + got; });
        "#,
    )
    .await
    .unwrap();
    let r = pump_until(
        &mut e,
        "globalThis.__pipebp !== null ? globalThis.__pipebp : 'false'",
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("pipe never finished");
    assert_eq!(r, "got:20480");
}

/// Node documents `stream.finished()`'s callback as asynchronous, and the code
/// that uses it emits `'finish'` and then `'close'` in a row: the callback must
/// see both (it used to run inside the `'finish'` emit, so a server response
/// looked unclosed from it).
#[tokio::test]
async fn finished_callback_runs_after_finish_and_close() {
    let mut e = engine().await;
    e.eval(
        r#"
        var stream = require('stream');
        var EE = require('events');
        var target = new EE();
        var seen = [];
        globalThis.__eos = null;
        stream.finished(target, function() { seen.push('cb'); });
        target.on('finish', function() { seen.push('finish'); });
        target.on('close', function() { seen.push('close'); });
        target.emit('finish');
        seen.push('after-finish');
        target.emit('close');
        setTimeout(function() { globalThis.__eos = seen.join(','); }, 5);
        "#,
    )
    .await
    .unwrap();
    pump_until(
        &mut e,
        "globalThis.__eos !== null ? 'done' : 'false'",
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("finished() never called back");
    let r = e.eval_to_string("globalThis.__eos").await.unwrap();
    assert_eq!(r, "finish,after-finish,close,cb");
}

/// `finished()` on a stream that is already over (destroyed before the call)
/// must still call back, asynchronously, instead of waiting for events that will
/// never come.
#[tokio::test]
async fn finished_on_an_already_finished_stream_calls_back_async() {
    let mut e = engine().await;
    e.eval(
        r#"
        var stream = require('stream');
        var EE = require('events');
        var target = new EE();
        target.destroyed = true;
        globalThis.__eos2 = 'sync';
        stream.finished(target, function() { globalThis.__eos2 = 'cb'; });
        "#,
    )
    .await
    .unwrap();
    pump_until(
        &mut e,
        "globalThis.__eos2 !== 'sync' ? 'done' : 'false'",
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("finished() never called back on an already-destroyed stream");
    let r = e.eval_to_string("globalThis.__eos2").await.unwrap();
    assert_eq!(r, "cb");
}

// Node's stream destroy (lib/internal/streams/destroy.js): `destroyed` and
// `errored` are set synchronously, `closed` flips to true just before 'close',
// and 'error'/'close' go out on later ticks (an 'error' nobody listens for is
// the process's uncaught exception, not a silent drop).
#[tokio::test]
async fn destroy_sets_state_and_emits_error_then_close() {
    let mut e = engine().await;
    e.eval(
        r#"
        var Readable = require('stream').Readable;
        var s = new Readable({ read: function() {} });
        globalThis.__order = [];
        s.on('error', function(err) { globalThis.__order.push('error:' + err.message); });
        s.on('close', function() { globalThis.__order.push('close'); });
        s.destroy(new Error('boom'));
        // destroy() is synchronous for the state, async for the events.
        globalThis.__state = [s.destroyed, s.errored && s.errored.message, s.closed].join(',');
        "#,
    )
    .await
    .unwrap();
    // `closed` must be true and 'close' emitted only after a tick.
    pump_until(
        &mut e,
        "globalThis.__order.join('|').indexOf('close') !== -1 ? 'done' : 'false'",
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("destroy() never emitted 'close'");
    let state = e.eval_to_string("globalThis.__state").await.unwrap();
    assert_eq!(
        state, "true,boom,false",
        "destroyed/errored must be set synchronously, closed still false"
    );
    let events = e
        .eval_to_string("globalThis.__order.join('|')")
        .await
        .unwrap();
    assert_eq!(events, "error:boom|close", "'error' must precede 'close'");
}
