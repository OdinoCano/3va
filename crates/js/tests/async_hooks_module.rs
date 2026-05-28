// Tests for AsyncLocalStorage backed by the patched QuickJS job hook.
// Run: cargo test -p vvva_js --test async_hooks_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

/// Drive promises to completion polling up to 100 iterations.
async fn eval_async(e: &JsEngine, setup: &str, result_global: &str) -> String {
    e.eval(setup).await.unwrap();
    for _ in 0..100 {
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

// ── basic run / getStore ──────────────────────────────────────────────────────

#[tokio::test]
async fn als_get_store_inside_run() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { AsyncLocalStorage } = require('async_hooks');
            var als = new AsyncLocalStorage();
            var result = 'none';
            als.run({ user: 'alice' }, function() {
                var s = als.getStore();
                result = s ? s.user : 'missing';
            });
            result
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "alice");
}

#[tokio::test]
async fn als_get_store_outside_run_is_undefined() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { AsyncLocalStorage } = require('async_hooks');
            var als = new AsyncLocalStorage();
            String(als.getStore())
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "undefined");
}

// ── async propagation through await ──────────────────────────────────────────

#[tokio::test]
async fn als_propagates_through_await() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var { AsyncLocalStorage } = require('async_hooks');
        var als = new AsyncLocalStorage();
        globalThis.__result = undefined;

        als.run({ reqId: 42 }, async function() {
            await Promise.resolve();          // yield to event loop
            await Promise.resolve();          // yield again
            var s = als.getStore();
            globalThis.__result = s ? String(s.reqId) : 'lost';
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "42", "context must survive two awaits");
}

// ── two concurrent chains don't bleed context ─────────────────────────────────

#[tokio::test]
async fn als_concurrent_chains_isolated() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var { AsyncLocalStorage } = require('async_hooks');
        var als = new AsyncLocalStorage();
        var results = [];

        function delay() { return new Promise(function(r) { setTimeout(r, 0); }); }

        // Chain A: reqId = 'A'
        var chainA = als.run({ reqId: 'A' }, async function() {
            await delay();
            results.push('A:' + (als.getStore() || {}).reqId);
            await delay();
            results.push('A:' + (als.getStore() || {}).reqId);
        });

        // Chain B: reqId = 'B' — starts while chain A is suspended
        var chainB = als.run({ reqId: 'B' }, async function() {
            await delay();
            results.push('B:' + (als.getStore() || {}).reqId);
            await delay();
            results.push('B:' + (als.getStore() || {}).reqId);
        });

        Promise.all([chainA, chainB]).then(function() {
            globalThis.__result = results.sort().join(',');
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(
        r, "A:A,A:A,B:B,B:B",
        "concurrent chains must not bleed: {r}"
    );
}

// ── nested run() ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn als_nested_run_inner_overrides() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { AsyncLocalStorage } = require('async_hooks');
            var als = new AsyncLocalStorage();
            var inner_val = 'x';
            var outer_val = 'x';
            als.run({ level: 'outer' }, function() {
                outer_val = (als.getStore() || {}).level;
                als.run({ level: 'inner' }, function() {
                    inner_val = (als.getStore() || {}).level;
                });
            });
            outer_val + ',' + inner_val
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "outer,inner");
}

// ── exit() removes context ────────────────────────────────────────────────────

#[tokio::test]
async fn als_exit_hides_store() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { AsyncLocalStorage } = require('async_hooks');
            var als = new AsyncLocalStorage();
            var inside = 'x';
            var exited = 'x';
            als.run({ v: 1 }, function() {
                inside = String((als.getStore() || {}).v);
                als.exit(function() {
                    exited = String(als.getStore());
                });
            });
            inside + ',' + exited
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "1,undefined");
}

// ── two independent ALS instances ────────────────────────────────────────────

#[tokio::test]
async fn two_als_instances_independent() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var { AsyncLocalStorage } = require('async_hooks');
        var alsA = new AsyncLocalStorage();
        var alsB = new AsyncLocalStorage();
        globalThis.__result = undefined;

        alsA.run({ who: 'alice' }, function() {
            alsB.run({ who: 'bob' }, async function() {
                await Promise.resolve();
                var a = (alsA.getStore() || {}).who;
                var b = (alsB.getStore() || {}).who;
                globalThis.__result = a + ':' + b;
            });
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "alice:bob");
}

// ── AsyncResource ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn async_resource_restores_context() {
    let e = engine().await;
    let r = eval_async(
        &e,
        r#"
        var ah = require('async_hooks');
        var als = new ah.AsyncLocalStorage();
        globalThis.__result = undefined;

        als.run({ tag: 'resource-test' }, function() {
            var res = new ah.AsyncResource('test');
            setTimeout(function() {
                res.runInAsyncScope(function() {
                    globalThis.__result = (als.getStore() || {}).tag || 'lost';
                });
            }, 0);
        });
        "#,
        "__result",
    )
    .await;
    assert_eq!(r, "resource-test");
}
