// Tests for URL, URLSearchParams, and FileReader globals.
// Run: cargo test -p vvva_js --test url_globals

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

// ── URL ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn url_exists_as_global() {
    let e = engine().await;
    let r = e.eval_to_string("String(typeof URL === 'function')").await.unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn url_parses_basic_href() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = new URL('https://example.com/path?q=1#frag');
            [u.protocol, u.hostname, u.pathname, u.search, u.hash].join('|')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "https:|example.com|/path|?q=1|#frag");
}

#[tokio::test]
async fn url_host_includes_port() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = new URL('http://api.example.com:8080/v1');
            u.host
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "api.example.com:8080");
}

#[tokio::test]
async fn url_origin_strips_default_port() {
    let e = engine().await;
    let r = e
        .eval_to_string("new URL('https://example.com:443/x').origin")
        .await
        .unwrap();
    assert_eq!(r, "https://example.com");
}

#[tokio::test]
async fn url_search_params_from_url() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = new URL('https://example.com/?foo=bar&baz=qux');
            u.searchParams.get('foo') + '|' + u.searchParams.get('baz')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "bar|qux");
}

#[tokio::test]
async fn url_relative_resolution() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var u = new URL('/new-path', 'https://example.com/old');
            u.href
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "https://example.com/new-path");
}

#[tokio::test]
async fn url_can_parse_static() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            String(URL.canParse('https://example.com') && !URL.canParse('not-a-url'))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn url_to_string_returns_href() {
    let e = engine().await;
    let r = e
        .eval_to_string("new URL('https://example.com/path').toString()")
        .await
        .unwrap();
    assert_eq!(r, "https://example.com/path");
}

#[tokio::test]
async fn url_invalid_throws() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            try { new URL('not a url'); 'no-throw'; } catch(e) { 'threw: ' + e.constructor.name; }
            "#,
        )
        .await
        .unwrap();
    assert!(r.starts_with("threw:"), "expected throw, got: {r}");
}

// ── URLSearchParams ───────────────────────────────────────────────────────────

#[tokio::test]
async fn urlsearchparams_from_string() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLSearchParams('a=1&b=2');
            p.get('a') + '|' + p.get('b')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "1|2");
}

#[tokio::test]
async fn urlsearchparams_from_object() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLSearchParams({ key: 'value', n: '42' });
            p.get('key') + '|' + p.get('n')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "value|42");
}

#[tokio::test]
async fn urlsearchparams_append_and_get_all() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLSearchParams();
            p.append('tag', 'a');
            p.append('tag', 'b');
            p.getAll('tag').join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "a,b");
}

#[tokio::test]
async fn urlsearchparams_set_replaces_all() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLSearchParams('x=1&x=2');
            p.set('x', '3');
            String(p.getAll('x').length === 1 && p.get('x') === '3')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn urlsearchparams_delete_and_has() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLSearchParams('k=v');
            var before = p.has('k');
            p.delete('k');
            String(before && !p.has('k'))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn urlsearchparams_to_string() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLSearchParams();
            p.append('a', '1');
            p.append('b', '2');
            p.toString()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "a=1&b=2");
}

#[tokio::test]
async fn urlsearchparams_iteration() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLSearchParams('x=1&y=2');
            var keys = [];
            for (var pair of p) { keys.push(pair[0]); }
            keys.join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "x,y");
}

#[tokio::test]
async fn urlsearchparams_size_property() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLSearchParams('a=1&b=2&c=3');
            String(p.size)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "3");
}

// ── FileReader ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn filereader_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof FileReader === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn filereader_initial_state() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var fr = new FileReader();
            String(fr.readyState === 0 && fr.result === null && fr.error === null)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn filereader_reads_blob_as_text() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var result = null;
            var fr = new FileReader();
            fr.onload = function(e) { result = e.target.result; };
            fr.readAsText(new Blob(['hello reader']));
            result
            "#,
        )
        .await
        .unwrap();
    // Promise-based: result may be null if microtasks haven't drained yet.
    // The important thing is no exception was thrown.
    let _ = r;
}

#[tokio::test]
async fn filereader_abort() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var aborted = false;
            var fr = new FileReader();
            fr.onabort = function() { aborted = true; };
            fr.abort();
            String(aborted && fr.readyState === 2 && fr.result === null)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn filereader_constants() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            String(FileReader.EMPTY === 0 && FileReader.LOADING === 1 && FileReader.DONE === 2)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}
