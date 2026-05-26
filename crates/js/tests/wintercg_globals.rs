// Tests for WinterCG web platform globals: Headers, Request, Response, structuredClone,
// navigator, self.
//
// Run: cargo test -p vvva_js --test wintercg_globals

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

// ── self / navigator ─────────────────────────────────────────────────────────

#[tokio::test]
async fn self_equals_globalthis() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(self === globalThis)")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn navigator_user_agent() {
    let e = engine().await;
    let r = e.eval_to_string("navigator.userAgent").await.unwrap();
    assert!(
        r.contains("3va"),
        "userAgent should contain '3va', got: {r}"
    );
}

#[tokio::test]
async fn navigator_online_true() {
    let e = engine().await;
    let r = e.eval_to_string("String(navigator.onLine)").await.unwrap();
    assert_eq!(r, "true");
}

// ── structuredClone ──────────────────────────────────────────────────────────

#[tokio::test]
async fn structured_clone_primitives() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var c = structuredClone(42);
            String(c === 42)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn structured_clone_plain_object() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var orig = { a: 1, b: [2, 3] };
            var c = structuredClone(orig);
            c.b.push(99);
            String(orig.b.length === 2 && c.b.length === 3)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true", "clone should be independent of original");
}

#[tokio::test]
async fn structured_clone_date() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var d = new Date(1700000000000);
            var c = structuredClone(d);
            String(c instanceof Date && c.getTime() === d.getTime() && c !== d)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn structured_clone_regexp() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var re = /hello/gi;
            var c = structuredClone(re);
            String(c instanceof RegExp && c.source === 'hello' && c.flags === 'gi' && c !== re)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn structured_clone_map() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var m = new Map([['a', 1], ['b', 2]]);
            var c = structuredClone(m);
            c.set('c', 3);
            String(c instanceof Map && c.size === 3 && m.size === 2)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn structured_clone_set() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var s = new Set([1, 2, 3]);
            var c = structuredClone(s);
            c.add(4);
            String(c instanceof Set && c.size === 4 && s.size === 3)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn structured_clone_array_buffer() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var ab = new ArrayBuffer(4);
            new Uint8Array(ab)[0] = 0xFF;
            var c = structuredClone(ab);
            new Uint8Array(c)[0] = 0x00;
            String(new Uint8Array(ab)[0] === 0xFF && c.byteLength === 4)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn structured_clone_uint8array() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var arr = new Uint8Array([10, 20, 30]);
            var c = structuredClone(arr);
            c[0] = 99;
            String(arr[0] === 10 && c[0] === 99 && c instanceof Uint8Array)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn structured_clone_throws_on_function() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var threw = false;
            try { structuredClone(function() {}); }
            catch(e) { threw = e.name === 'DataCloneError' || e instanceof TypeError; }
            String(threw)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn structured_clone_throws_on_circular() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var obj = {};
            obj.self = obj;
            var threw = false;
            try { structuredClone(obj); } catch(e) { threw = true; }
            String(threw)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── Headers ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn headers_class_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof Headers === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn headers_get_is_case_insensitive() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var h = new Headers({ 'Content-Type': 'application/json' });
            h.get('content-type')
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "application/json");
}

#[tokio::test]
async fn headers_has_set_delete() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var h = new Headers();
            h.set('x-foo', 'bar');
            var had = h.has('X-FOO');
            h.delete('x-foo');
            String(had && !h.has('x-foo'))
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn headers_append_joins_values() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var h = new Headers();
            h.append('accept', 'text/html');
            h.append('accept', 'application/json');
            h.get('accept')
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "text/html, application/json");
}

#[tokio::test]
async fn headers_get_set_cookie_preserves_separate_values() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var h = new Headers();
            h.append('set-cookie', 'a=1');
            h.append('set-cookie', 'b=2');
            JSON.stringify(h.getSetCookie())
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, r#"["a=1","b=2"]"#);
}

#[tokio::test]
async fn headers_iterable_via_for_of() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var h = new Headers({ 'a': '1', 'b': '2' });
            var pairs = [];
            for (var entry of h) { pairs.push(entry[0] + '=' + entry[1]); }
            pairs.sort().join(',')
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "a=1,b=2");
}

#[tokio::test]
async fn headers_constructed_from_another_headers() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var h1 = new Headers({ 'x-token': 'abc' });
            var h2 = new Headers(h1);
            h2.set('x-token', 'xyz');
            String(h1.get('x-token') === 'abc' && h2.get('x-token') === 'xyz')
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── Request ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn request_class_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof Request === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn request_url_and_method() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var req = new Request('https://example.com/api', { method: 'post' });
            req.url + '|' + req.method
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "https://example.com/api|POST");
}

#[tokio::test]
async fn request_headers_is_headers_instance() {
    let e = engine().await;
    let r = e
        .eval_to_string(r#"
            var req = new Request('https://x.com', { headers: { 'content-type': 'text/plain' } });
            String(req.headers instanceof Headers && req.headers.get('content-type') === 'text/plain')
        "#)
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn request_clone_is_independent() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var req = new Request('https://x.com', { method: 'POST', body: 'hello' });
            var cloned = req.clone();
            String(cloned.url === req.url && cloned !== req)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── Response ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn response_class_exists() {
    let e = engine().await;
    let r = e
        .eval_to_string("String(typeof Response === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn response_ok_and_status() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var r200 = new Response('', { status: 200 });
            var r404 = new Response('', { status: 404 });
            String(r200.ok && !r404.ok)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn response_headers_is_headers_instance() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var res = new Response('body', { headers: { 'x-val': 'hello' } });
            String(res.headers instanceof Headers && res.headers.get('x-val') === 'hello')
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn response_json_static_sets_content_type() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var res = Response.json({ ok: true });
            res.headers.get('content-type')
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "application/json");
}

#[tokio::test]
async fn response_error_static() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var res = Response.error();
            String(res.status === 0 && res.type === 'error')
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn response_redirect_static() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var res = Response.redirect('https://example.com/new', 301);
            String(res.status === 301 && res.headers.get('location') === 'https://example.com/new')
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn response_clone_is_independent() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var res = new Response('hello', { status: 200 });
            var c = res.clone();
            String(c !== res && c.status === 200)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn response_redirect_throws_on_invalid_status() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var threw = false;
            try { Response.redirect('https://x.com', 200); } catch(e) { threw = true; }
            String(threw)
        "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}
