// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! `node:test`: el subconjunto del runner de Node que usan la suite oficial
//! `test/parallel` y una suite de proyecto típica.
//!
//! Ejecutar: cargo test -p vvva_js --test node_test_module

use std::sync::Arc;
use tempfile::TempDir;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_read(dir: &TempDir) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::FileRead(dir.path().to_path_buf()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

async fn run_file(content: &str) -> i32 {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("test.js");
    std::fs::write(&path, content).unwrap();
    let mut engine = engine_with_read(&temp).await;
    engine.eval_file(&path).await.unwrap();
    engine.finish().await.unwrap()
}

#[tokio::test]
async fn a_passing_file_exits_with_zero() {
    let code = run_file(
        r#"
        const { test } = require('node:test');
        const assert = require('node:assert');
        test('sums', () => { assert.strictEqual(1 + 1, 2); });
        "#,
    )
    .await;
    assert_eq!(code, 0, "una suite que pasa debe salir con 0");
}

#[tokio::test]
async fn a_failing_test_exits_with_one() {
    let code = run_file(
        r#"
        const { test } = require('node:test');
        const assert = require('node:assert');
        test('fails', () => { assert.strictEqual(1, 2); });
        "#,
    )
    .await;
    assert_eq!(code, 1, "una suite que falla debe salir con 1");
}

#[tokio::test]
async fn describe_and_it_run_the_tests() {
    let code = run_file(
        r#"
        const { describe, it } = require('node:test');
        const assert = require('node:assert');
        describe('a group', () => {
            it('passes', () => { assert.ok(true); });
        });
        "#,
    )
    .await;
    assert_eq!(code, 0);
}

#[tokio::test]
async fn a_failing_subtest_fails_its_parent() {
    let code = run_file(
        r#"
        const { test } = require('node:test');
        const assert = require('node:assert');
        test('parent', async (t) => {
            await t.test('child', () => { assert.strictEqual(1, 2); });
        });
        "#,
    )
    .await;
    assert_eq!(code, 1, "un subtest que falla hace fallar al padre");
}

#[tokio::test]
async fn the_hooks_run_around_the_tests() {
    let code = run_file(
        r#"
        const { test, before, after, beforeEach, afterEach } = require('node:test');
        const assert = require('node:assert');
        let order = [];
        before(() => { order.push('before'); });
        beforeEach(() => { order.push('beforeEach'); });
        afterEach(() => { order.push('afterEach'); });
        after(() => {
            order.push('after');
            assert.deepStrictEqual(order, [
                'before', 'beforeEach', 'test', 'afterEach',
                'beforeEach', 'afterEach', 'after'
            ]);
        });
        test('one', () => { order.push('test'); });
        test('two', () => {});
        "#,
    )
    .await;
    assert_eq!(code, 0);
}

#[tokio::test]
async fn mock_method_records_the_calls_and_is_restored() {
    let code = run_file(
        r#"
        const { test } = require('node:test');
        const assert = require('node:assert');
        const fs = require('node:fs');
        const original = fs.fsyncSync;
        test('spies', (t) => {
            const spy = t.mock.method(fs, 'fsyncSync');
            fs.fsyncSync(1);
            assert.strictEqual(spy.mock.callCount(), 1);
            assert.strictEqual(spy.mock.calls[0].arguments.length, 1);
        });
        test('restored', () => {
            assert.strictEqual(fs.fsyncSync, original, 'el mock debe restaurarse');
        });
        "#,
    )
    .await;
    assert_eq!(code, 0);
}

#[tokio::test]
async fn mock_fn_counts_the_calls() {
    let code = run_file(
        r#"
        const { test } = require('node:test');
        const assert = require('node:assert');
        test('counts', (t) => {
            const fn = t.mock.fn();
            fn(1, 2);
            fn(3);
            assert.strictEqual(fn.mock.callCount(), 2);
            assert.strictEqual(fn.mock.calls[1].arguments[0], 3);
        });
        "#,
    )
    .await;
    assert_eq!(code, 0);
}

#[tokio::test]
async fn a_test_with_a_done_callback_completes() {
    let code = run_file(
        r#"
        const { test } = require('node:test');
        const assert = require('node:assert');
        test('callback style', (t, done) => {
            setTimeout(() => { assert.ok(true); done(); }, 1);
        });
        "#,
    )
    .await;
    assert_eq!(code, 0);
}

#[tokio::test]
async fn a_skipped_test_does_not_fail_the_run() {
    let code = run_file(
        r#"
        const { test } = require('node:test');
        test('skipped', { skip: true }, () => { throw new Error('never'); });
        "#,
    )
    .await;
    assert_eq!(code, 0);
}

#[tokio::test]
async fn mock_timers_is_not_supported_and_says_so() {
    let code = run_file(
        r#"
        const { test } = require('node:test');
        test('timers', (t) => {
            try {
                t.mock.timers.enable();
                throw new Error('should have thrown');
            } catch (e) {
                if (e.code !== 'ERR_NOT_SUPPORTED') throw e;
            }
        });
        "#,
    )
    .await;
    assert_eq!(code, 0);
}
