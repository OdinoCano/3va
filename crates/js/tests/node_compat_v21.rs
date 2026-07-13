// Tests for v2.1.0 Node.js compat additions:
// timers/promises, dns (real lookup), readline, and --heap-snapshot.
// Run: cargo test -p vvva_js --test node_compat_v21

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("*".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

/// Forces this process's real fd 0 to `/dev/null`. Relying on whatever stdin
/// the test binary happened to inherit is flaky — e.g. a git pre-push hook's
/// stdin is a pipe carrying ref-update data, not `/dev/null`, so a real read
/// on fd 0 wouldn't hit EOF quickly and a test asserting on that would
/// hang/fail depending on how `cargo test` was invoked.
#[cfg(unix)]
fn redirect_stdin_to_devnull() {
    unsafe {
        unsafe extern "C" {
            fn open(path: *const std::os::raw::c_char, flags: i32) -> i32;
            fn dup2(oldfd: i32, newfd: i32) -> i32;
            fn close(fd: i32) -> i32;
        }
        let path = std::ffi::CString::new("/dev/null").unwrap();
        let fd = open(path.as_ptr(), 0 /* O_RDONLY */);
        if fd >= 0 {
            dup2(fd, 0);
            close(fd);
        }
    }
}

// ── timers/promises ───────────────────────────────────────────────────────────

#[tokio::test]
async fn timers_promises_module_loads() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            "var t = require('timers/promises'); String(typeof t.setTimeout === 'function')",
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn timers_promises_set_timeout_resolves() {
    let mut e = engine().await;
    e.eval("var done = null; require('timers/promises').setTimeout(1, 'ok').then(function(v){ done = v; });")
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    e.run_event_loop().await.unwrap();
    let r = e.eval_to_string("String(done)").await.unwrap();
    assert_eq!(r, "ok", "timers/promises.setTimeout did not resolve");
}

#[tokio::test]
async fn timers_promises_set_immediate_resolves() {
    let mut e = engine().await;
    e.eval(
        "var done = null; require('timers/promises').setImmediate('immediate').then(function(v){ done = v; });",
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    e.run_event_loop().await.unwrap();
    let r = e.eval_to_string("String(done)").await.unwrap();
    assert_eq!(
        r, "immediate",
        "timers/promises.setImmediate did not resolve"
    );
}

#[tokio::test]
async fn timers_promises_node_prefix_alias() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            "var t = require('node:timers/promises'); String(typeof t.setInterval === 'function')",
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn timers_promises_abort_signal_rejects() {
    let mut e = engine().await;
    e.eval(
        r#"
        var result = null;
        var ac = new AbortController();
        require('timers/promises').setTimeout(10000, 'never', { signal: ac.signal })
            .then(function() { result = 'resolved'; })
            .catch(function(e) { result = e.name || 'AbortError'; });
        ac.abort();
        "#,
    )
    .await
    .unwrap();
    e.run_event_loop().await.unwrap();
    let r = e.eval_to_string("String(result)").await.unwrap();
    assert!(
        r != "null" && (r.contains("Abort") || r.contains("abort")),
        "expected AbortError, got: {r}"
    );
}

// ── dns ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dns_module_loads() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            "var d = require('dns'); String(typeof d.lookup === 'function' && typeof d.resolve === 'function')",
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn dns_promises_namespace_exists() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            "var d = require('dns'); String(typeof d.promises === 'object' && typeof d.promises.lookup === 'function')",
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn dns_lookup_localhost_resolves() {
    let mut e = engine_with_net().await;
    e.eval(
        r#"
        var result = null;
        require('dns').lookup('localhost', function(err, addr) {
            result = err ? ('ERR:' + err.message) : addr;
        });
        "#,
    )
    .await
    .unwrap();
    for _ in 0..40 {
        e.idle().await;
        // Timers (setTimeout, used by dns's/readline's callback deferral)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        let r = e.eval_to_string("String(result)").await.unwrap();
        if r != "null" {
            assert!(
                r == "127.0.0.1" || r == "::1" || r.starts_with("127.") || r.contains(':'),
                "unexpected dns.lookup result: {r}"
            );
            return;
        }
    }
    panic!("dns.lookup('localhost') did not call back");
}

#[tokio::test]
async fn dns_promises_lookup_localhost() {
    let mut e = engine_with_net().await;
    e.eval(
        r#"
        var result = null;
        require('dns').promises.lookup('localhost').then(function(r) {
            result = r.address || r;
        }).catch(function(e) { result = 'ERR:' + e.message; });
        "#,
    )
    .await
    .unwrap();
    for _ in 0..40 {
        e.idle().await;
        // Timers (setTimeout, used by dns's/readline's callback deferral)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        let r = e.eval_to_string("String(result)").await.unwrap();
        if r != "null" {
            assert!(!r.starts_with("ERR:"), "dns.promises.lookup failed: {r}");
            return;
        }
    }
    panic!("dns.promises.lookup('localhost') did not resolve");
}

#[tokio::test]
async fn dns_resolve4_callback_form() {
    let mut e = engine_with_net().await;
    e.eval(
        r#"
        var result = null;
        require('dns').resolve4('localhost', function(err, addrs) {
            result = err ? ('ERR:' + err.code) : JSON.stringify(addrs);
        });
        "#,
    )
    .await
    .unwrap();
    for _ in 0..40 {
        e.idle().await;
        // Timers (setTimeout, used by dns's/readline's callback deferral)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        let r = e.eval_to_string("String(result)").await.unwrap();
        if r != "null" {
            // Some environments return ENOTFOUND for localhost on resolve4 — acceptable
            assert!(
                r.contains("127.") || r.contains("ENOTFOUND") || r.contains("ENOTSUP"),
                "unexpected dns.resolve4 result: {r}"
            );
            return;
        }
    }
    panic!("dns.resolve4('localhost') did not call back");
}

#[tokio::test]
async fn dns_resolve_mx_uses_real_query() {
    // Exercises the hickory-resolver-backed __dnsQuery path (previously a stub
    // that always returned []). Tolerant of no network/DNS in the sandbox —
    // the point is that a real query round-trip happens, not a hardcoded [].
    let mut e = engine_with_net().await;
    e.eval(
        r#"
        var result = null;
        require('dns').resolveMx('gmail.com', function(err, records) {
            result = err ? ('ERR:' + err.code) : JSON.stringify(records);
        });
        "#,
    )
    .await
    .unwrap();
    for _ in 0..40 {
        e.idle().await;
        // Timers (setTimeout, used by dns's/readline's callback deferral)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        let r = e.eval_to_string("String(result)").await.unwrap();
        if r != "null" {
            assert!(
                r.contains("exchange") || r.contains("ERR:ENODATA") || r.contains("ERR:ENOTFOUND"),
                "unexpected dns.resolveMx result: {r}"
            );
            return;
        }
    }
    panic!("dns.resolveMx('gmail.com') did not call back");
}

#[tokio::test]
async fn dns_resolve_soa_returns_object_not_stub_array() {
    let mut e = engine_with_net().await;
    e.eval(
        r#"
        var result = null;
        require('dns').resolveSoa('gmail.com', function(err, soa) {
            result = err ? ('ERR:' + err.code) : JSON.stringify(soa);
        });
        "#,
    )
    .await
    .unwrap();
    for _ in 0..40 {
        e.idle().await;
        // Timers (setTimeout, used by dns's/readline's callback deferral)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        let r = e.eval_to_string("String(result)").await.unwrap();
        if r != "null" {
            assert!(
                r.contains("nsname") || r.contains("ERR:ENODATA") || r.contains("ERR:ENOTFOUND"),
                "unexpected dns.resolveSoa result: {r}"
            );
            return;
        }
    }
    panic!("dns.resolveSoa('gmail.com') did not call back");
}

// ── readline ─────────────────────────────────────────────────────────────────
// process.stdin is now backed by real OS stdin (native __stdinRead, process.rs),
// and Interface consumes it via Node-style 'data' events (see _feedChars /
// Interface constructor in crates/js/src/builtins/modules.rs). These tests
// still only check API shape since the test process's stdin is /dev/null
// (immediate EOF) — genuine line-by-line behavior needs a piped mock stdin.

#[tokio::test]
async fn readline_module_loads() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            "var rl = require('readline'); String(typeof rl.createInterface === 'function')",
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn readline_interface_created() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var rl = require('readline');
            var iface = rl.createInterface({ input: process.stdin, output: process.stdout });
            String(typeof iface.close === 'function' && typeof iface.question === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn readline_async_iterator_exists() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var rl = require('readline');
            var iface = rl.createInterface({ input: process.stdin });
            String(typeof iface[Symbol.asyncIterator] === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn readline_promises_namespace_exists() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            "var rl = require('readline'); String(typeof rl.promises === 'object' && typeof rl.promises.createInterface === 'function')",
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn readline_set_prompt_survives() {
    let mut e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var rl = require('readline');
            var iface = rl.createInterface({ input: process.stdin });
            iface.setPrompt('> ');
            String(iface.getPrompt())
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "> ");
}

// ── heap snapshot ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn heap_snapshot_is_valid_json() {
    let mut e = engine().await;
    e.eval("var x = { a: 1, b: [1,2,3] }; var s = 'hello world';")
        .await
        .unwrap();
    let snap = e.take_heap_snapshot().await.unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&snap).expect("heap snapshot is not valid JSON");
    assert!(
        v.get("snapshot").is_some(),
        "missing 'snapshot' key in heapsnapshot"
    );
}

#[tokio::test]
async fn heap_snapshot_has_required_meta_fields() {
    let mut e = engine().await;
    let snap = e.take_heap_snapshot().await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&snap).unwrap();
    let meta = v["snapshot"]["meta"]
        .as_object()
        .expect("snapshot.meta missing");
    assert!(meta.contains_key("node_fields"), "missing node_fields");
    assert!(meta.contains_key("node_types"), "missing node_types");
    assert!(meta.contains_key("edge_fields"), "missing edge_fields");
    assert!(meta.contains_key("edge_types"), "missing edge_types");
}

#[tokio::test]
async fn heap_snapshot_has_nodes_and_strings() {
    let mut e = engine().await;
    e.eval("var obj = {}; for (var i = 0; i < 10; i++) obj['k'+i] = i;")
        .await
        .unwrap();
    let snap = e.take_heap_snapshot().await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&snap).unwrap();
    assert!(
        v["nodes"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "nodes array is empty"
    );
    assert!(
        v["strings"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "strings array is empty"
    );
}

// ── stdin ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn stdin_read_native_binding_resolves() {
    // Force real fd 0 to /dev/null (see helper doc) so this can assert EOF
    // deterministically — it can't assert real interactive input, but it does
    // confirm the native __stdinRead() binding (blocking OS read on a
    // background thread) actually runs to completion and reports EOF instead
    // of hanging the event loop.
    #[cfg(unix)]
    redirect_stdin_to_devnull();
    let mut e = engine().await;
    e.eval(
        r#"
        var done = false;
        var bytesRead = -1;
        __stdinRead().then(function(chunk) {
            bytesRead = chunk.length;
            done = true;
        });
        "#,
    )
    .await
    .unwrap();
    for _ in 0..100 {
        e.idle().await;
        // Timers (setTimeout, used by dns's/readline's callback deferral)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(done)").await.unwrap() == "true" {
            let bytes = e.eval_to_string("String(bytesRead)").await.unwrap();
            assert_eq!(bytes, "0", "expected EOF (0 bytes) from /dev/null stdin");
            return;
        }
    }
    panic!("__stdinRead() never resolved");
}

#[tokio::test]
async fn readline_over_process_stdin_reaches_close_on_eof() {
    // Force real fd 0 to /dev/null (see helper doc above engine_with_net).
    #[cfg(unix)]
    redirect_stdin_to_devnull();

    // With fd 0 now /dev/null, the readline Interface should hit EOF almost
    // immediately and emit 'close' — proving the Node-style 'data'/'end'
    // wiring in the Interface constructor actually drives the stream instead
    // of sitting inert (the old stub never emitted 'close' on its own).
    let mut e = engine().await;
    e.eval(
        r#"
        var closed = false;
        var rl = require('readline').createInterface({ input: process.stdin });
        rl.on('close', function() { closed = true; });
        "#,
    )
    .await
    .unwrap();
    for _ in 0..100 {
        e.idle().await;
        // Timers (setTimeout, used by dns's/readline's callback deferral)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(closed)").await.unwrap() == "true" {
            return;
        }
    }
    panic!("readline Interface over process.stdin never closed on EOF");
}
