// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Which packages have code on the JS stack, read from V8 itself.
//!
//! Package `deny-*` rules used to rely only on the `require()` wrapper
//! setting a scope from JS, which code can sidestep: globals (`fetch`),
//! constructors (`new net.Socket()`), raw native bindings, or tampering with
//! the wrapper's globals (VULN-03). The stack at the moment a native binding
//! runs can't be sidestepped that way.

use std::cell::{Cell, RefCell};

thread_local! {
    // The live isolate of this thread's JsEngine; cleared when it drops.
    static ISOLATE: Cell<Option<v8::UnsafeRawIsolatePtr>> = const { Cell::new(None) };
    // The entry script's own package, treated as app code (see
    // `entry_package_scope`). Kept here, not in a JS global a package could set.
    static ENTRY_SCOPE: RefCell<Option<String>> = const { RefCell::new(None) };
    // StackTrace needs a context-bearing scope; outside JS none is entered.
    static CONTEXT: RefCell<Option<v8::Global<v8::Context>>> = const { RefCell::new(None) };
}

/// Makes `isolate` + `context` the pair the stack is read from. Both are set
/// together so a handle is never used in another isolate when a thread hosts
/// more than one engine (the most recently initialized one wins).
pub(crate) fn install(isolate: v8::UnsafeRawIsolatePtr, context: v8::Global<v8::Context>) {
    ISOLATE.with(|i| i.set(Some(isolate)));
    CONTEXT.with(|c| *c.borrow_mut() = Some(context));
    vvva_permissions::set_stack_scopes_resolver(Some(stack_scopes));
}

pub(crate) fn uninstall(isolate: &v8::Isolate) {
    let this = format!("{:?}", unsafe { isolate.as_raw_isolate_ptr() });
    ISOLATE.with(|i| {
        if i.get().is_some_and(|p| format!("{p:?}") == this) {
            i.set(None);
            CONTEXT.with(|c| c.borrow_mut().take());
            vvva_permissions::set_stack_scopes_resolver(None);
        }
    });
}

pub(crate) fn set_entry_scope(scope: Option<String>) {
    ENTRY_SCOPE.with(|e| *e.borrow_mut() = scope);
}

/// The innermost `node_modules/<pkg>` (or `@scope/pkg`) in `path`, mirroring
/// the require() wrapper's `__pkgScopeFor`.
pub(crate) fn package_scope(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    let rest = &path[path.rfind("/node_modules/")? + "/node_modules/".len()..];
    let mut parts = rest.split('/');
    let first = parts.next().filter(|s| !s.is_empty())?;
    if first.starts_with('@') {
        let second = parts.next().filter(|s| !s.is_empty())?;
        Some(format!("{first}/{second}"))
    } else {
        Some(first.to_string())
    }
}

fn stack_scopes() -> Vec<String> {
    let Some(ptr) = ISOLATE.with(|i| i.get()) else {
        return Vec::new();
    };
    // SAFETY: `ptr` is this thread's live isolate (cleared in JsEngine's
    // Drop before the isolate is disposed). Permission checks run on the
    // isolate's own thread, inside native callbacks; V8 allows opening a
    // nested HandleScope there.
    let mut isolate = unsafe { v8::Isolate::from_raw_isolate_ptr(ptr) };
    let Some(context) = CONTEXT.with(|c| c.borrow().clone()) else {
        return Vec::new();
    };
    v8::scope!(let scope, &mut isolate);
    let context = v8::Local::new(scope, &context);
    let scope = &mut v8::ContextScope::new(scope, context);
    // No practical frame cap: a cap would let code push its own frame out of
    // view with deep recursion through frames that carry a forged sourceURL.
    let Some(trace) = v8::StackTrace::current_stack_trace(scope, 1 << 20) else {
        return Vec::new();
    };
    let entry = ENTRY_SCOPE.with(|e| e.borrow().clone());
    let mut out: Vec<String> = Vec::new();
    for i in 0..trace.get_frame_count() {
        let Some(frame) = trace.get_frame(scope, i) else {
            continue;
        };
        let Some(name) = frame.get_script_name_or_source_url(scope) else {
            continue;
        };
        if let Some(pkg) = package_scope(&name.to_rust_string_lossy(scope))
            && entry.as_deref() != Some(pkg.as_str())
            && !out.contains(&pkg)
        {
            out.push(pkg);
        }
    }
    out
}

thread_local! {
    // Inherited scopes to restore after each promise reaction (they nest).
    static SAVED: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn install_promise_hook(isolate: &mut v8::Isolate) {
    isolate.set_promise_hook(promise_hook);
}

/// Carries deny scopes across promise reactions: a promise created while a
/// package is on the stack (or inside a reaction that inherited one) is
/// tagged, and its reaction runs with those scopes. Otherwise
/// `Promise.resolve(url).then(fetch)` runs `fetch` with only builtin frames
/// on the stack. The tag is a `v8::Private`, which JS can't read or clear.
// ponytail: tags every promise (with a stack walk) once any scoped rule
// exists; only projects that declare package rules pay for it.
unsafe extern "C" fn promise_hook(
    ty: v8::PromiseHookType,
    promise: v8::Local<v8::Promise>,
    parent: v8::Local<v8::Value>,
) {
    if !vvva_permissions::scoped_rules_active() {
        return;
    }
    match ty {
        v8::PromiseHookType::After => {
            if let Some(prev) = SAVED.with(|s| s.borrow_mut().pop()) {
                vvva_permissions::set_inherited_scopes(prev);
            }
            return;
        }
        v8::PromiseHookType::Resolve => return,
        _ => {}
    }
    v8::callback_scope!(unsafe scope, promise);
    let name = v8::String::new(scope, "3va.denyScopes").unwrap();
    let key = v8::Private::for_api(scope, Some(name));
    let read = |scope: &v8::PinScope, obj: v8::Local<v8::Object>| -> Vec<String> {
        obj.get_private(scope, key)
            .filter(|v| v.is_string())
            .map(|v| {
                v.to_rust_string_lossy(scope)
                    .split('\n')
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    match ty {
        v8::PromiseHookType::Init => {
            let mut scopes = vvva_permissions::deny_scopes();
            if let Ok(parent) = v8::Local::<v8::Object>::try_from(parent) {
                scopes.extend(read(scope, parent));
            }
            scopes.sort();
            scopes.dedup();
            if !scopes.is_empty() {
                let tag = v8::String::new(scope, &scopes.join("\n")).unwrap();
                promise.set_private(scope, key, tag.into());
            }
        }
        v8::PromiseHookType::Before => {
            let prev = vvva_permissions::set_inherited_scopes(read(scope, promise.into()));
            SAVED.with(|s| s.borrow_mut().push(prev));
        }
        _ => {}
    }
}

/// Pre-bound builtins (`fetch.bind(null, url)`) are the one way package code
/// can hand a gated call, with its own arguments, to someone else to run
/// with no package frame on the stack: as an event listener, a library
/// callback, anything. When package code calls `bind`, the result re-applies
/// the binder's deny scopes on every call. The original `bind` and the two
/// natives stay in a closure JS can't reach (vm contexts only exchange JSON,
/// so another realm's `bind` never reaches this one).
pub(crate) fn install_bind_guard(scope: &mut v8::PinScope) {
    let tag_fn = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope,
         _args: v8::FunctionCallbackArguments,
         mut rv: v8::ReturnValue| {
            // No rules: leave the return value undefined (falsy, same as "")
            // so every `bind` doesn't allocate a string for nothing.
            if !vvva_permissions::scoped_rules_active() {
                return;
            }
            let tag = vvva_permissions::deny_scopes().join("\n");
            rv.set(v8::String::new(scope, &tag).unwrap().into());
        },
    )
    .unwrap();
    let run_fn = v8::Function::new(
        scope,
        |scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue| {
            let tag = args.get(0).to_rust_string_lossy(scope);
            let Ok(target) = v8::Local::<v8::Function>::try_from(args.get(1)) else {
                return;
            };
            let this = args.get(2);
            let mut argv = Vec::new();
            if let Ok(list) = v8::Local::<v8::Object>::try_from(args.get(3)) {
                let len_key = v8::String::new(scope, "length").unwrap();
                let len = list
                    .get(scope, len_key.into())
                    .and_then(|v| v.uint32_value(scope))
                    .unwrap_or(0);
                for i in 0..len {
                    argv.push(
                        list.get_index(scope, i)
                            .unwrap_or_else(|| v8::undefined(scope).into()),
                    );
                }
            }
            let prev = vvva_permissions::set_inherited_scopes(Vec::new());
            let mut merged = prev.clone();
            merged.extend(
                tag.split('\n')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            );
            vvva_permissions::set_inherited_scopes(merged);
            let result = target.call(scope, this, &argv);
            vvva_permissions::set_inherited_scopes(prev);
            if let Some(v) = result {
                rv.set(v);
            }
        },
    )
    .unwrap();
    let context = scope.get_current_context();
    let global = context.global(scope);
    for (name, f) in [("__callerScopesTag", tag_fn), ("__runWithScopes", run_fn)] {
        let key = v8::String::new(scope, name).unwrap();
        global.set(scope, key.into(), f.into());
    }
    let src = r#"(function () {
        var origBind = Function.prototype.bind;
        var tagNow = globalThis.__callerScopesTag;
        var runTagged = globalThis.__runWithScopes;
        delete globalThis.__callerScopesTag;
        delete globalThis.__runWithScopes;
        Object.defineProperty(Function.prototype, 'bind', {
            writable: true, configurable: true, enumerable: false,
            value: function bind(thisArg) {
                var bound = origBind.apply(this, arguments);
                var tag = tagNow();
                if (!tag) return bound;
                var guarded = function () {
                    return new.target
                        ? Reflect.construct(bound, arguments, new.target)
                        : runTagged(tag, bound, undefined, arguments);
                };
                Object.defineProperty(guarded, 'name', { value: bound.name });
                Object.defineProperty(guarded, 'length', { value: bound.length });
                return guarded;
            },
        });
    })();"#;
    let _ = crate::builtins::code_cache::bootstrap_js_per_run(scope, "bind-guard", src);
}

#[cfg(test)]
mod tests {
    use super::package_scope;

    #[test]
    fn package_scope_takes_innermost_package() {
        assert_eq!(package_scope("/app/index.js"), None);
        assert_eq!(
            package_scope("/app/node_modules/evil/index.js").as_deref(),
            Some("evil")
        );
        assert_eq!(
            package_scope("/app/node_modules/a/node_modules/@s/b/x.js").as_deref(),
            Some("@s/b")
        );
        assert_eq!(
            package_scope("C:\\app\\node_modules\\w\\i.js").as_deref(),
            Some("w")
        );
    }
}
