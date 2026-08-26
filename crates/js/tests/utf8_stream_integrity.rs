// Regression tests for multi-byte UTF-8 integrity through streamed/chunked
// data paths. Pins the exact defect class behind the "Metro under 3va serves
// bundles with emoji/accented characters replaced by U+FFFD" bug:
//
//   1. The TextDecoder fallback (buffer.rs) never decoded 4-byte UTF-8
//      sequences — every astral-plane character (all emoji) became U+FFFD on
//      ANY Buffer→string conversion, regardless of chunking.
//   2. Readable.prototype.setEncoding() only stored the encoding; push()
//      emitted raw byte chunks, so consumers collecting 'data' chunks as
//      strings (Node semantics) re-decoded each chunk independently and
//      corrupted every multi-byte sequence straddling a chunk boundary.
//   3. StringDecoder dropped its pending incomplete sequence whenever two
//      consecutive writes both ended mid-sequence.
//
// The stream tests split payloads at exact 64 KiB boundaries (Node's
// fs.ReadStream default highWaterMark) with sequences deliberately positioned
// to straddle each boundary.
//
// Run: cargo test -p vvva_js --test utf8_stream_integrity

use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

// ── TextDecoder / Buffer (root cause 1) ───────────────────────────────────────

#[tokio::test]
async fn textdecoder_decodes_four_byte_utf8_sequences() {
    let mut e = engine().await;
    // U+1F600 GRINNING FACE = F0 9F 98 80 (a surrogate pair in JS strings).
    let r = e
        .eval_to_string(
            r#"new TextDecoder('utf-8').decode(new Uint8Array([0xF0, 0x9F, 0x98, 0x80])) === '\u{1F600}' ? 'OK' : 'CORRUPT'"#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

#[tokio::test]
async fn textdecoder_replaces_truncated_sequence_with_single_replacement_char() {
    let mut e = engine().await;
    // A one-shot decode of a truncated sequence must yield exactly one
    // U+FFFD (the old implementation read past the buffer end and produced
    // garbage code points instead).
    let r = e
        .eval_to_string(
            r#"var s = new TextDecoder('utf-8').decode(new Uint8Array([0x61, 0xF0, 0x9F])); s === 'a\uFFFD' ? 'OK' : 'BAD:' + JSON.stringify(s)"#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

#[tokio::test]
async fn textdecoder_stream_mode_holds_incomplete_sequence() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var dec = new TextDecoder('utf-8');
            var a = dec.decode(new Uint8Array([0x61, 0xF0, 0x9F]), { stream: true });
            var b = dec.decode(new Uint8Array([0x98, 0x80, 0x62]), { stream: true });
            (a === 'a' && b === '\u{1F600}b') ? 'OK' : 'BAD:' + JSON.stringify(a) + '/' + JSON.stringify(b)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

#[tokio::test]
async fn buffer_tostring_roundtrips_multibyte_characters() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"var s = 'accents \u00e9\u00e8\u00fc, euro \u20ac, emoji \u{1F600}\u{1F680}'; Buffer.from(s).toString('utf8') === s ? 'OK' : 'CORRUPT'"#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

// ── string_decoder (root cause 3) ─────────────────────────────────────────────

#[tokio::test]
async fn string_decoder_reassembles_sequences_split_across_chunks() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var SD = require('string_decoder').StringDecoder;
            var bytes = new TextEncoder().encode('a\u{1F600}b\u00e9\u20ac c\u{1F680}d');
            var sd = new SD('utf8');
            var out = '';
            // Split at every odd offset so sequences straddle chunk boundaries,
            // including two consecutive writes that both end mid-sequence.
            for (var off = 0; off < bytes.length; off += 3) {
                out += sd.write(bytes.subarray(off, off + 3));
            }
            out += sd.end();
            out === 'a\u{1F600}b\u00e9\u20ac c\u{1F680}d' ? 'OK' : 'BAD:' + JSON.stringify(out)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

#[tokio::test]
async fn string_decoder_consecutive_writes_ending_mid_sequence() {
    let mut e = engine().await;
    // w1 and w2 both end mid-emoji; the old implementation silently dropped
    // w1's pending bytes when w2 arrived.
    let r = e
        .eval_to_string(
            r#"
            var SD = require('string_decoder').StringDecoder;
            var bytes = new TextEncoder().encode('\u{1F600}b\u{1F600}c\u{1F600}d');
            var sd = new SD('utf8');
            var out = sd.write(bytes.subarray(0, 2))
                     + sd.write(bytes.subarray(2, 6))
                     + sd.write(bytes.subarray(6, 8))
                     + sd.write(bytes.subarray(8, 14))
                     + sd.end(bytes.subarray(14));
            out === '\u{1F600}b\u{1F600}c\u{1F600}d' ? 'OK' : 'BAD:' + JSON.stringify(out)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

#[tokio::test]
async fn string_decoder_end_flushes_dangling_bytes_as_replacement() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var SD = require('string_decoder').StringDecoder;
            var sd = new SD('utf8');
            var a = sd.write(new Uint8Array([0x61, 0xF0, 0x9F]));
            var b = sd.end();
            (a === 'a' && b === '\uFFFD') ? 'OK' : 'BAD:' + JSON.stringify(a) + '/' + JSON.stringify(b)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

// ── Readable.setEncoding (root cause 2) ───────────────────────────────────────

/// Build a payload whose é (2-byte), € (3-byte) and 😀 (4-byte) sequences
/// each straddle a consecutive 64 KiB chunk boundary, followed by plain
/// multi-byte content, and return the JS expression that feeds it through a
/// Readable with setEncoding('utf8') in exact 64 KiB pushes.
const ROUNDTRIP_SETUP: &str = r#"
    var orig = 'x'.repeat(65535) + '\u00e9'
             + 'x'.repeat(65534) + '\u20ac'
             + 'x'.repeat(65533) + '\u{1F600}'
             + 'plain \u{1F600} \u00e9 \u20ac tail';
    var bytes = new TextEncoder().encode(orig);
    var Readable = require('stream').Readable;
    var src = new Readable();
    src.setEncoding('utf8');
    var got = '';
    src.on('data', function (c) {
        if (typeof c !== 'string') { got = 'NON_STRING_CHUNK'; return; }
        got += c;
    });
    for (var off = 0; off < bytes.length; off += 65536) {
        src.push(Buffer.from(bytes.subarray(off, off + 65536)));
    }
    src.push(null);
    __result = (got === orig) ? 'OK' : 'CORRUPT:' + got.length + ':' + orig.length;
    __result;
"#;

#[tokio::test]
async fn readable_setencoding_reassembles_multibyte_across_64k_boundaries() {
    let mut e = engine().await;
    let r = e.eval_to_string(ROUNDTRIP_SETUP).await.unwrap();
    assert_eq!(r, "OK");
}

#[tokio::test]
async fn readable_constructor_encoding_option_is_honored() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var Readable = require('stream').Readable;
            var bytes = new TextEncoder().encode('a\u{1F600}\u00e9b\u20ac');
            var src = new Readable({ encoding: 'utf8' });
            var got = '';
            src.on('data', function (c) { got += c; });
            src.push(bytes.subarray(0, 3));
            src.push(bytes.subarray(3));
            src.push(null);
            got === 'a\u{1F600}\u00e9b\u20ac' && typeof got === 'string' ? 'OK' : 'BAD:' + JSON.stringify(got)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

#[tokio::test]
async fn readable_read_returns_decoded_string_when_encoding_set() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var Readable = require('stream').Readable;
            var bytes = new TextEncoder().encode('a\u{1F600}b\u{1F600}c');
            var src = new Readable();
            src.setEncoding('utf8');
            src.push(bytes.subarray(0, 2));
            src.push(bytes.subarray(2));
            src.push(null);
            var s = src.read();
            (typeof s === 'string' && s === 'a\u{1F600}b\u{1F600}c') ? 'OK' : 'BAD:' + JSON.stringify(String(s))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "OK");
}

// ── End-to-end: Metro-shaped HTTP response assembly ───────────────────────────
//
// The original bug report: a ~4 MB bundle assembled from streamed chunks and
// served through http.ServerResponse arrived on the client with individual
// multi-byte sequences replaced by U+FFFD. This test reproduces the shape of
// that path — byte chunks pushed through a Readable with setEncoding('utf8'),
// reassembled, and served — over a real socket, and asserts the response body
// is byte-identical to the original string's UTF-8 encoding.

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

async fn wait_for_port(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if std::net::TcpListener::bind(format!("127.0.0.1:{port}")).is_err() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("port {port} never became ready within 5 s");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn drive_until<T>(e: &mut JsEngine, client: impl std::future::Future<Output = T>) -> T {
    tokio::pin!(client);
    let result = tokio::select! {
        _ = async {
            loop {
                e.idle().await;
                let _ = e.run_event_loop().await;
                tokio::task::yield_now().await;
            }
        } => unreachable!("engine event loop terminated unexpectedly"),
        result = &mut client => result,
    };
    result
}

async fn http_get_body(port: u16) -> Vec<u8> {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap_or_else(|e| panic!("connect to port {port}: {e}"));
    let req = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut resp = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        let n = stream.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        resp.extend_from_slice(&buf[..n]);
    }
    // Strip headers: everything after the first CRLFCRLF.
    let split = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no header terminator in response");
    resp.split_off(split + 4)
}

#[tokio::test]
async fn http_bundle_assembly_serves_multibyte_intact() {
    let port = free_port();
    let mut e = engine_with_net().await;

    // Same payload construction on the JS side: module-ish lines carrying
    // emoji and accented characters, fed through the stream in small chunks
    // so multi-byte sequences straddle chunk boundaries.
    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var Readable = require('stream').Readable;
        var orig = '';
        for (var i = 0; i < 500; i++) orig += 'module_' + i + ' "\u00e9\u20ac\u{{1F600}}"\n';
        var bytes = new TextEncoder().encode(orig);
        var server = http.createServer(function(req, res) {{
            var src = new Readable();
            src.setEncoding('utf8');
            var body = '';
            src.on('data', function(c) {{ body += c; }});
            src.on('end', function() {{
                res.setHeader('Content-Type', 'application/javascript');
                res.end(body);
            }});
            for (var off = 0; off < bytes.length; off += 97) {{
                src.push(Buffer.from(bytes.subarray(off, off + 97)));
            }}
            src.push(null);
        }});
        server.listen({port}, '127.0.0.1');
        'started'
        "#,
        port = port,
    ))
    .await
    .unwrap();

    wait_for_port(port).await;

    let body = drive_until(&mut e, http_get_body(port)).await;

    let mut expected = String::new();
    for i in 0..500 {
        expected.push_str(&format!("module_{i} \"\u{e9}\u{20ac}\u{1F600}\"\n"));
    }
    assert_eq!(
        body,
        expected.as_bytes(),
        "served bundle body must be byte-identical to the original UTF-8 payload"
    );
}
