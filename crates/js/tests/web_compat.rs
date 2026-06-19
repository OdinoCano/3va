// Tests for Web compat globals: sessionStorage, localStorage, URLPattern, EventSource.
// Run: cargo test -p vvva_js --test web_compat

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

// ── sessionStorage ────────────────────────────────────────────────────────────

#[tokio::test]
async fn session_storage_set_get() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            sessionStorage.setItem('key1', 'value1');
            sessionStorage.getItem('key1')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "value1");
}

#[tokio::test]
async fn session_storage_remove() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            sessionStorage.setItem('k', 'v');
            sessionStorage.removeItem('k');
            String(sessionStorage.getItem('k') === null)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn session_storage_clear_and_length() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            sessionStorage.setItem('a', '1');
            sessionStorage.setItem('b', '2');
            var len1 = sessionStorage.length;
            sessionStorage.clear();
            [String(len1), String(sessionStorage.length)].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "2,0");
}

#[tokio::test]
async fn session_storage_key() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            sessionStorage.clear();
            sessionStorage.setItem('x', 'y');
            sessionStorage.key(0)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "x");
}

// ── localStorage ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn local_storage_set_get() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            localStorage.setItem('lk', 'lv');
            localStorage.getItem('lk')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "lv");
}

#[tokio::test]
async fn local_storage_remove() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            localStorage.removeItem('lk');
            localStorage.setItem('lk2', 'v');
            localStorage.removeItem('lk2');
            String(localStorage.getItem('lk2') === null && localStorage.getItem('lk') === null)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── URLPattern ────────────────────────────────────────────────────────────────
// URLPattern relies on `new URL()` internally for string inputs. When the URL
// polyfill is available, full string-based matching works. These tests verify
// the constructor and basic API contract.

#[tokio::test]
async fn url_pattern_constructor_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof URLPattern === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn url_pattern_test_api_surface() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLPattern('/:a/:b');
            [typeof p.test, typeof p.exec].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "function,function");
}

#[tokio::test]
async fn url_pattern_pathname_match() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLPattern({ pathname: '/users/:id' });
            String(p.test('https://example.com/users/42'))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn url_pattern_pathname_no_match() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLPattern({ pathname: '/users/:id' });
            String(p.test('https://example.com/posts/42'))
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "false");
}

#[tokio::test]
async fn url_pattern_exec_extracts_groups() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLPattern({ pathname: '/users/:id/posts/:pid' });
            var m = p.exec('https://example.com/users/7/posts/99');
            [m && m.groups.id, m && m.groups.pid].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "7,99");
}

#[tokio::test]
async fn url_pattern_wildcard_matches_any() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLPattern({ pathname: '/api/*' });
            [p.test('https://x.com/api/'),
             p.test('https://x.com/api/v1/users'),
             p.test('https://x.com/other')].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true,true,false");
}

#[tokio::test]
async fn url_pattern_hostname_match() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLPattern({ hostname: 'example.com', pathname: '/*' });
            [p.test('https://example.com/anything'),
             p.test('https://other.com/anything')].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true,false");
}

#[tokio::test]
async fn url_pattern_relative_pathname_exec() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLPattern({ pathname: '/:lang/home' });
            var m = p.exec({ pathname: '/en/home' });
            m && m.groups.lang
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "en");
}

#[tokio::test]
async fn url_pattern_string_init_full_url() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var p = new URLPattern('https://cdn.example.com/assets/:file');
            [p.test('https://cdn.example.com/assets/app.js'),
             p.test('https://other.com/assets/app.js')].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true,false");
}

// ── EventSource ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn event_source_constructor_and_constants() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            [EventSource.CONNECTING, EventSource.OPEN, EventSource.CLOSED].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "0,1,2");
}

#[tokio::test]
async fn event_source_close_does_not_throw() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var es = new EventSource('http://localhost:0/nonexistent');
            es.close();
            String(es.readyState === EventSource.CLOSED)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn event_source_add_event_listener() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var es = new EventSource('http://localhost:0/x');
            String(typeof es.addEventListener === 'function' && typeof es.removeEventListener === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}
