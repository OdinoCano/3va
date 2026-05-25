// Tests for the Blob, File, and related Web APIs.
// Run: cargo test -p vvva_js --test blob_file_module

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

// ── Blob constructor ──────────────────────────────────────────────────────────

#[tokio::test]
async fn blob_global_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof Blob === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn blob_size_from_string() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(new Blob(['hello']).size)")
        .await
        .unwrap();
    assert_eq!(r, "5");
}

#[tokio::test]
async fn blob_size_multiple_parts() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(new Blob(['hello', ' world']).size)")
        .await
        .unwrap();
    assert_eq!(r, "11");
}

#[tokio::test]
async fn blob_empty_has_size_zero() {
    let e = engine().await;
    let r = e.eval_to_string("String(new Blob([]).size)").await.unwrap();
    assert_eq!(r, "0");
}

#[tokio::test]
async fn blob_type_defaults_to_empty_string() {
    let e = engine().await;
    let r = e
        .eval_to_string("JSON.stringify(new Blob(['x']).type)")
        .await
        .unwrap();
    assert_eq!(r, r#""""#);
}

#[tokio::test]
async fn blob_type_option_is_set() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"new Blob(['x'], { type: 'text/plain' }).type"#)
        .await
        .unwrap();
    assert_eq!(r, "text/plain");
}

// ── Blob.text() ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn blob_text_resolves_with_content() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._blob_text = null;
        new Blob(['hello blob']).text().then(function(t) {
            globalThis._blob_text = t;
        });
        "#,
        "globalThis._blob_text",
    )
    .await;
    assert_eq!(r, "hello blob");
}

// ── Blob.arrayBuffer() ────────────────────────────────────────────────────────

#[tokio::test]
async fn blob_array_buffer_correct_length() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._ab_len = null;
        new Blob(['abc']).arrayBuffer().then(function(buf) {
            globalThis._ab_len = String(buf.byteLength);
        });
        "#,
        "globalThis._ab_len",
    )
    .await;
    assert_eq!(r, "3");
}

#[tokio::test]
async fn blob_array_buffer_correct_bytes() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._ab_bytes = null;
        new Blob(['ABC']).arrayBuffer().then(function(buf) {
            var view = new Uint8Array(buf);
            globalThis._ab_bytes = Array.from(view).join(',');
        });
        "#,
        "globalThis._ab_bytes",
    )
    .await;
    // ASCII: A=65, B=66, C=67
    assert_eq!(r, "65,66,67");
}

// ── Blob.bytes() ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn blob_bytes_returns_uint8array() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._bytes_type = null;
        new Blob(['hi']).bytes().then(function(b) {
            globalThis._bytes_type = String(b instanceof Uint8Array);
        });
        "#,
        "globalThis._bytes_type",
    )
    .await;
    assert_eq!(r, "true");
}

#[tokio::test]
async fn blob_bytes_correct_values() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._bytes_vals = null;
        new Blob(['hi']).bytes().then(function(b) {
            globalThis._bytes_vals = Array.from(b).join(',');
        });
        "#,
        "globalThis._bytes_vals",
    )
    .await;
    // ASCII: h=104, i=105
    assert_eq!(r, "104,105");
}

// ── Blob.slice() ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn blob_slice_returns_new_blob() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = new Blob(['hello world']);
            var s = b.slice(0, 5);
            String(s instanceof Blob) + ',' + String(s.size)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true,5");
}

#[tokio::test]
async fn blob_slice_content_is_correct() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._slice_text = null;
        var b = new Blob(['hello world']);
        b.slice(6, 11).text().then(function(t) {
            globalThis._slice_text = t;
        });
        "#,
        "globalThis._slice_text",
    )
    .await;
    assert_eq!(r, "world");
}

#[tokio::test]
async fn blob_slice_preserves_type() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = new Blob(['abc'], { type: 'text/plain' });
            b.slice(0, 1).type
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "text/plain");
}

// ── Blob.stream() ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn blob_stream_returns_readable_stream() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var b = new Blob(['data']);
            String(b.stream() instanceof ReadableStream)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn blob_stream_readable_content() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._stream_content = null;
        var rs = new Blob(['streamed']).stream();
        var reader = rs.getReader();
        reader.read().then(function(res) {
            globalThis._stream_content = res.value;
        });
        "#,
        "globalThis._stream_content",
    )
    .await;
    assert_eq!(r, "streamed");
}

// ── File constructor ──────────────────────────────────────────────────────────

#[tokio::test]
async fn file_global_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof File === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn file_has_name() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"new File(['content'], 'test.txt').name"#)
        .await
        .unwrap();
    assert_eq!(r, "test.txt");
}

#[tokio::test]
async fn file_has_size() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(new File(['hello'], 'f.txt').size)")
        .await
        .unwrap();
    assert_eq!(r, "5");
}

#[tokio::test]
async fn file_has_type() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"new File(['x'], 'f.txt', { type: 'text/html' }).type"#)
        .await
        .unwrap();
    assert_eq!(r, "text/html");
}

#[tokio::test]
async fn file_has_last_modified() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof new File(['x'], 'f.txt').lastModified === 'number')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn file_last_modified_option() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            "String(new File(['x'], 'f.txt', { lastModified: 1234567890 }).lastModified)",
        )
        .await
        .unwrap();
    assert_eq!(r, "1234567890");
}

#[tokio::test]
async fn file_is_blob_instance() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(new File(['x'], 'f.txt') instanceof Blob)")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn file_text_inherits_from_blob() {
    let e = engine().await;
    let r = eval_async_result(
        &e,
        r#"
        globalThis._file_text = null;
        new File(['file content'], 'f.txt').text().then(function(t) {
            globalThis._file_text = t;
        });
        "#,
        "globalThis._file_text",
    )
    .await;
    assert_eq!(r, "file content");
}
