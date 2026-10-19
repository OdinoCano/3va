// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! `node:test` — the subset of Node's test runner that the nodejs/node
//! `test/parallel` suite and a typical project suite need.
//!
//! Node exposes `node:test` as a *function* (`test` is callable and also
//! destructurable) with `describe`/`it`/`suite`, the `before`/`after`/
//! `beforeEach`/`afterEach` hooks, the `mock` helpers and a per-test context
//! `t` with `t.test()`, `t.mock` and `t.assert`.
//!
//! Everything that is not implemented (`mock.timers`, `snapshot`, the
//! `run()` entry point with its own concurrency) throws `ERR_NOT_SUPPORTED`
//! instead of quietly being `undefined`.

use v8::{ContextScope, HandleScope};

/// Install `node:test` into the module cache.
pub fn inject_node_test(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    crate::builtins::code_cache::bootstrap_js(scope, "node-test", NODE_TEST_JS)?;
    Ok(())
}

const NODE_TEST_JS: &str = r##"
(function () {
    'use strict';

    var assertMod = globalThis.__requireCache['node:assert'] || globalThis.__requireCache['assert'];
    // The shared argument validators (Node's messages), installed next to
    // this module by the builtins bootstrap.
    var validators = globalThis.__validators || {};

    // ── TAP output ───────────────────────────────────────────────────────────
    var outLines = [];
    var stats = { pass: 0, fail: 0, skip: 0, todo: 0 };
    var testNumber = 0;
    var currentIndent = '';

    function write(line) {
        outLines.push(currentIndent + line);
    }

    function flush() {
        if (!outLines.length) return;
        var text = outLines.join('\n') + '\n';
        outLines = [];
        if (typeof process !== 'undefined' && process.stdout &&
            typeof process.stdout.write === 'function') {
            process.stdout.write(text);
        } else if (typeof console !== 'undefined' && console.log) {
            console.log(text.replace(/\n$/, ''));
        }
    }

    function notSupported(what) {
        var err = new Error(what + ' is not supported by 3va');
        err.code = 'ERR_NOT_SUPPORTED';
        throw err;
    }

    function errText(e) {
        if (e === null || e === undefined) return 'unknown error';
        if (typeof e === 'string') return e;
        if (e.stack) return String(e.stack);
        return String(e.name || 'Error') + ': ' + String(e.message || e);
    }

    // ── mocks ────────────────────────────────────────────────────────────────
    // `mock.fn()` returns a function carrying a `mock` object with the call
    // records; `mock.method(obj, name)` replaces a method with such a mock and
    // restores it when the test that created it finishes.
    function makeMock(impl) {
        var calls = [];
        var mockFn = function () {
            var args = Array.prototype.slice.call(arguments);
            var record = {
                arguments: args,
                target: this,
                result: undefined,
                error: undefined,
                stack: undefined
            };
            calls.push(record);
            try {
                var res = mockFn._impl.apply(this, args);
                record.result = res;
                return res;
            } catch (err) {
                record.error = err;
                throw err;
            }
        };
        mockFn._impl = impl;
        var mock = {
            calls: calls,
            callCount: function () { return calls.length; },
            resetCalls: function () { calls.length = 0; },
            mockImplementation: function (fn) {
                if (typeof fn !== 'function') {
                    if (validators.validateFunction) validators.validateFunction(fn, 'fn');
                    var err = new TypeError('mockImplementation expects a function');
                    err.code = 'ERR_INVALID_ARG_TYPE';
                    throw err;
                }
                mockFn._impl = fn;
                return mockFn;
            },
            restore: function () { /* nothing to restore for a plain mock */ }
        };
        mockFn.mock = mock;
        return mockFn;
    }

    function createMockTracker(restores) {
        return {
            fn: function (impl) {
                return makeMock(typeof impl === 'function' ? impl : function () {});
            },
            method: function (object, methodName, implementation) {
                if (object === null || (typeof object !== 'object' && typeof object !== 'function')) {
                    if (validators.validateObject) {
                        validators.validateObject(object, 'object');
                    }
                    var err = new TypeError('The "object" argument must be of type object');
                    err.code = 'ERR_INVALID_ARG_TYPE';
                    throw err;
                }
                if (typeof methodName !== 'string') {
                    if (validators.validateString) {
                        validators.validateString(methodName, 'methodName');
                    }
                    var err2 = new TypeError('The "methodName" argument must be of type string');
                    err2.code = 'ERR_INVALID_ARG_TYPE';
                    throw err2;
                }
                var original = object[methodName];
                var descriptor = Object.getOwnPropertyDescriptor(object, methodName);
                var impl = typeof implementation === 'function' ? implementation : null;
                var mockFn = makeMock(function () {
                    if (impl) return impl.apply(this, arguments);
                    if (typeof original === 'function') return original.apply(this, arguments);
                    return undefined;
                });
                mockFn.mock.restore = function () {
                    if (descriptor) {
                        Object.defineProperty(object, methodName, descriptor);
                    } else {
                        try { delete object[methodName]; } catch (e) { /* non-configurable */ }
                    }
                };
                restores.push(mockFn.mock.restore);
                object[methodName] = mockFn;
                return mockFn;
            },
            restoreAll: function () {
                while (restores.length) {
                    var fn = restores.pop();
                    try { fn(); } catch (e) { /* already restored */ }
                }
            },
            timers: {
                enable: function () { notSupported('mock.timers.enable'); },
                disable: function () { notSupported('mock.timers.disable'); },
                reset: function () { notSupported('mock.timers.reset'); },
                tick: function () { notSupported('mock.timers.tick'); }
            }
        };
    }

    // Module-level `mock`: not tied to a test, so nothing restores it.
    var globalRestores = [];
    var mockGlobal = createMockTracker(globalRestores);

    // ── suites and tests ─────────────────────────────────────────────────────
    function Suite(name, parent, options) {
        this.name = name;
        this.parent = parent;
        this.options = options || {};
        this.children = [];
        this.hooks = { before: [], after: [], beforeEach: [], afterEach: [] };
    }

    var rootSuite = new Suite('', null, {});
    var currentSuite = rootSuite;
    var scheduled = false;
    var hasOnly = false;

    function schedule() {
        if (scheduled) return;
        scheduled = true;
        // The tests run once the module body has finished, like Node's runner:
        // a microtask is the earliest point that is still "after the script".
        queueMicrotask(function () { runAll(); });
    }

    function normalize(name, options, fn) {
        if (typeof name === 'function') {
            fn = name;
            name = fn.name || '';
            options = {};
        } else if (typeof options === 'function') {
            fn = options;
            options = {};
        }
        options = options || {};
        if (typeof name !== 'string') name = String(name);
        return { name: name, options: options, fn: fn };
    }

    function addTest(name, options, fn, extra) {
        var t = normalize(name, options, fn);
        t.options = Object.assign({}, t.options, extra || {});
        if (t.options.only) hasOnly = true;
        currentSuite.children.push({
            kind: 'test',
            name: t.name,
            options: t.options,
            fn: t.fn
        });
        schedule();
        return t;
    }

    function addSuite(name, options, fn, extra) {
        if (typeof options === 'function') {
            fn = options;
            options = {};
        }
        options = Object.assign({}, options || {}, extra || {});
        if (typeof name !== 'string') name = String(name);
        var suite = new Suite(name, currentSuite, options);
        if (options.only) hasOnly = true;
        currentSuite.children.push({ kind: 'suite', suite: suite });
        var previous = currentSuite;
        currentSuite = suite;
        try {
            if (typeof fn === 'function') fn();
        } finally {
            currentSuite = previous;
        }
        schedule();
        return suite;
    }

    function addHook(kind, fn) {
        if (typeof fn !== 'function') {
            if (validators.validateFunction) validators.validateFunction(fn, kind);
            var err = new TypeError('The hook must be a function');
            err.code = 'ERR_INVALID_ARG_TYPE';
            throw err;
        }
        currentSuite.hooks[kind].push(fn);
        schedule();
    }

    // ── running ──────────────────────────────────────────────────────────────
    function invoke(fn, ctx) {
        return new Promise(function (resolve, reject) {
            if (typeof fn !== 'function') { resolve(); return; }
            var settled = false;
            var done = function (err) {
                if (settled) return;
                settled = true;
                if (err) reject(err); else resolve();
            };
            var result;
            try {
                // A test that declares a second parameter wants the `done`
                // callback; otherwise it is a (possibly async) function of `t`.
                result = fn.length >= 2 ? fn.call(ctx, ctx, done) : fn.call(ctx, ctx);
            } catch (e) {
                done(e);
                return;
            }
            if (result && typeof result.then === 'function') {
                result.then(function () { done(); },
                           function (e) { done(e || new Error('test rejected')); });
            } else if (fn.length < 2) {
                done();
            }
        });
    }

    function reportResult(number, name, failed, error) {
        if (failed) {
            stats.fail += 1;
            write('not ok ' + number + ' - ' + name);
            write('  ---');
            write('  error:');
            write('    name: ' + ((error && error.name) || 'Error'));
            write('    message: ' + ((error && error.message) || String(error)));
            if (error && error.code) write('    code: ' + error.code);
            write('  ...');
        } else {
            stats.pass += 1;
            write('ok ' + number + ' - ' + name);
        }
    }

    // Runs one list of hooks (a suite's `before`/`after`, or a test's
    // `beforeEach`/`afterEach`).
    function runHookList(hooks, ctx) {
        var i = 0;
        function step() {
            if (i >= hooks.length) return Promise.resolve();
            var hook = hooks[i];
            i += 1;
            return invoke(hook, ctx).then(step);
        }
        return step();
    }

    // The `beforeEach`/`afterEach` hooks of every suite in the chain, from the
    // outermost suite inwards.
    function runEachHooks(kind, chain, ctx) {
        var s = 0;
        function stepSuite() {
            if (s >= chain.length) return Promise.resolve();
            var hooks = chain[s].hooks[kind];
            s += 1;
            return runHookList(hooks, ctx).then(stepSuite);
        }
        return stepSuite();
    }

    function makeContext(name, suite, ancestors) {
        var ctx = {
            name: name,
            fullName: name,
            diagnostic: function (message) { write('# ' + name + ' ' + message); },
            skip: function (message) {
                var err = new Error(message || 'test skipped');
                err.code = 'ERR_TEST_SKIP';
                throw err;
            },
            todo: function (message) {
                var err = new Error(message || 'test todo');
                err.code = 'ERR_TEST_TODO';
                throw err;
            },
            assert: assertMod,
            __failed: false
        };
        // Subtests: `t.test(name, fn)` runs the subtest and resolves with its
        // context. A failing subtest also fails its parent. `ancestors` is
        // the chain of suites the test belongs to, so the subtest inherits
        // the same `beforeEach`/`afterEach` hooks.
        ctx.test = function (name, fn, options) {
            if (typeof fn === 'object' && fn !== null) {
                options = fn;
                fn = options.fn;
            }
            options = options || {};
            if (typeof name !== 'string') name = String(name);
            var number = (testNumber += 1);
            if (options.skip) {
                stats.skip += 1;
                write('ok ' + number + ' - ' + name + ' # SKIP');
                return Promise.resolve(makeContext(name, suite, ancestors));
            }
            if (options.todo) {
                stats.todo += 1;
                write('ok ' + number + ' - ' + name + ' # TODO');
                return Promise.resolve(makeContext(name, suite, ancestors));
            }
            write('# Subtest: ' + name);
            var restores = [];
            var subCtx = makeContext(name, suite, ancestors);
            subCtx.mock = createMockTracker(restores);
            var failed = false;
            var error = null;
            return runEachHooks('beforeEach', ancestors, subCtx)
                .then(function () { return invoke(fn, subCtx); })
                .then(function () { return runEachHooks('afterEach', ancestors, subCtx); })
                .catch(function (e) { failed = true; error = e; })
                .then(function () {
                    while (restores.length) {
                        var restore = restores.pop();
                        try { restore(); } catch (e) { /* ignore */ }
                    }
                    if (failed && error && error.code === 'ERR_TEST_SKIP') {
                        stats.skip += 1;
                        write('ok ' + number + ' - ' + name + ' # SKIP');
                        return;
                    }
                    if (failed && error && error.code === 'ERR_TEST_TODO') {
                        stats.todo += 1;
                        write('ok ' + number + ' - ' + name + ' # TODO');
                        return;
                    }
                    if (failed) ctx.__failed = true;
                    reportResult(number, name, failed, error);
                })
                .then(function () { return subCtx; });
        };
        return ctx;
    }

    function runTestNode(node, suite, ancestors) {
        var options = node.options || {};
        var skip = !!options.skip;
        var todo = !!options.todo;
        for (var i = 0; i < ancestors.length; i += 1) {
            var parentOptions = ancestors[i].options || {};
            if (parentOptions.skip) skip = true;
            if (parentOptions.todo) todo = true;
        }
        if ((hasOnly && !options.only) || skip) {
            stats.skip += 1;
            write('ok ' + (testNumber += 1) + ' - ' + node.name + ' # SKIP');
            return Promise.resolve();
        }
        if (todo) {
            stats.todo += 1;
            write('ok ' + (testNumber += 1) + ' - ' + node.name + ' # TODO');
            return Promise.resolve();
        }

        var number = (testNumber += 1);
        write('# Subtest: ' + node.name);
        var restores = [];
        var chain = ancestors.concat([suite]);
        var ctx = makeContext(node.name, suite, chain);
        ctx.mock = createMockTracker(restores);
        var failed = false;
        var error = null;

        return runEachHooks('beforeEach', chain, ctx)
            .then(function () { return invoke(node.fn, ctx); })
            .then(function () { return runEachHooks('afterEach', chain, ctx); })
            .catch(function (e) { failed = true; error = e; })
            .then(function () {
                while (restores.length) {
                    var restore = restores.pop();
                    try { restore(); } catch (e) { /* ignore */ }
                }
                if (failed && error && error.code === 'ERR_TEST_SKIP') {
                    stats.skip += 1;
                    write('ok ' + number + ' - ' + node.name + ' # SKIP');
                    return;
                }
                if (failed && error && error.code === 'ERR_TEST_TODO') {
                    stats.todo += 1;
                    write('ok ' + number + ' - ' + node.name + ' # TODO');
                    return;
                }
                reportResult(number, node.name, failed || ctx.__failed, error);
            });
    }

    function runSuite(suite, ancestors) {
        var previousIndent = currentIndent;
        currentIndent = previousIndent + '    ';
        var chain = suite.children.slice();
        var index = 0;
        var suiteName = suite.name || 'suite';
        var own = ancestors.concat([suite]);

        function step() {
            if (index >= chain.length) {
                // The suite's own `after` hooks run once, when it ends.
                return runHookList(suite.hooks.after,
                                   makeContext(suiteName, suite, own))
                    .catch(function (e) {
                        stats.fail += 1;
                        write('not ok ' + (testNumber += 1) + ' - ' + suiteName + ' (after hook)');
                        write('  error: ' + ((e && e.message) || String(e)));
                    });
            }
            var child = chain[index];
            index += 1;
            if (child.kind === 'suite') {
                return runSuite(child.suite, ancestors.concat([suite])).then(step);
            }
            return runTestNode(child, suite, ancestors).then(step);
        }

        // The suite's own `before` hooks run once, when it starts.
        return runHookList(suite.hooks.before, makeContext(suiteName, suite, own))
            .catch(function (e) {
                stats.fail += 1;
                write('not ok ' + (testNumber += 1) + ' - ' + suiteName + ' (before hook)');
                write('  error: ' + ((e && e.message) || String(e)));
            })
            .then(step)
            .then(function () { currentIndent = previousIndent; },
                  function (e) { currentIndent = previousIndent; throw e; });
    }

    function runAll() {
        write('TAP version 13');
        return runSuite(rootSuite, [])
            .catch(function (e) {
                stats.fail += 1;
                write('not ok ' + (testNumber += 1) + ' - internal error');
                write('  ' + errText(e));
            })
            .then(function () {
                write('1..' + testNumber);
                write('# pass ' + stats.pass);
                write('# fail ' + stats.fail);
                write('# skipped ' + stats.skip);
                write('# todo ' + stats.todo);
                flush();
                if (stats.fail > 0 && typeof process !== 'undefined') {
                    process.exitCode = 1;
                }
            });
    }

    // ── public API ───────────────────────────────────────────────────────────
    function test(name, options, fn) {
        return addTest(name, options, fn);
    }
    test.skip = function (name, options, fn) { return addTest(name, options, fn, { skip: true }); };
    test.todo = function (name, options, fn) { return addTest(name, options, fn, { todo: true }); };
    test.only = function (name, options, fn) { return addTest(name, options, fn, { only: true }); };
    test.skipIf = function (condition) { return condition ? test.skip : test; };
    test.todoIf = function (condition) { return condition ? test.todo : test; };

    function describe(name, options, fn) {
        return addSuite(name, options, fn);
    }
    describe.skip = function (name, options, fn) { return addSuite(name, options, fn, { skip: true }); };
    describe.todo = function (name, options, fn) { return addSuite(name, options, fn, { todo: true }); };
    describe.only = function (name, options, fn) { return addSuite(name, options, fn, { only: true }); };
    describe.skipIf = function (condition) { return condition ? describe.skip : describe; };
    describe.todoIf = function (condition) { return condition ? describe.todo : describe; };

    var nodeTest = test;
    nodeTest.test = test;
    nodeTest.it = test;
    nodeTest.describe = describe;
    nodeTest.suite = describe;
    nodeTest.before = function (fn) { addHook('before', fn); };
    nodeTest.after = function (fn) { addHook('after', fn); };
    nodeTest.beforeEach = function (fn) { addHook('beforeEach', fn); };
    nodeTest.afterEach = function (fn) { addHook('afterEach', fn); };
    nodeTest.mock = mockGlobal;
    nodeTest.run = function () { notSupported('test.run'); };
    nodeTest.snapshot = function () { notSupported('test.snapshot'); };
    nodeTest.expectFailure = function () { notSupported('test.expectFailure'); };

    globalThis.__requireCache['node:test'] = nodeTest;
})();
"##;
