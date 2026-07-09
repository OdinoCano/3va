/// Async context propagation — runtime-level implementation for V8.
///
/// In V8, async context is handled via `AsyncLocalStorage` which is part of the
/// V8 API itself. This module provides the async context functions that are
/// exposed to the JS runtime.
use std::collections::HashMap;
use std::sync::Mutex;
use v8::{Function, FunctionCallbackArguments, PinScope, ReturnValue};

struct CtxEntry {
    data: HashMap<String, String>,
    ref_count: u32,
}

static STORE: Mutex<Option<HashMap<u64, CtxEntry>>> = Mutex::new(None);
static CURRENT_ASYNC_CTX: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn with_store<F, R>(f: F) -> R
where
    F: FnOnce(&mut HashMap<u64, CtxEntry>) -> R,
{
    let mut guard = STORE.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    f(map)
}

fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(1);
    CTR.fetch_add(1, Ordering::Relaxed)
}

pub fn install(
    scope: &mut PinScope,
    _permissions: &std::sync::Arc<vvva_permissions::PermissionState>,
) -> anyhow::Result<()> {
    let async_ctx_set = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              _rv: ReturnValue<'_>| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0);
            CURRENT_ASYNC_CTX.store(id, std::sync::atomic::Ordering::Relaxed);
        },
    )
    .unwrap();

    let async_ctx_get = Function::new(
        scope,
        move |_scope: &mut PinScope<'_, '_>,
              _args: FunctionCallbackArguments<'_>,
              mut rv: ReturnValue<'_>| {
            let id = CURRENT_ASYNC_CTX.load(std::sync::atomic::Ordering::Relaxed);
            rv.set(v8::Integer::new_from_unsigned(_scope, id).into());
        },
    )
    .unwrap();

    let async_ctx_alloc = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              mut rv: ReturnValue<'_>| {
            let parent_id = args.get(0).uint32_value(scope).unwrap_or(0);
            let als_key = args
                .get(1)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let json_value = args
                .get(2)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();

            let new_id = with_store(|store| {
                let new_id = next_id();
                let parent_data = if parent_id != 0 {
                    store
                        .get(&(parent_id as u64))
                        .map(|e| e.data.clone())
                        .unwrap_or_default()
                } else {
                    HashMap::new()
                };
                let mut data = parent_data;
                data.insert(als_key, json_value);
                store.insert(new_id, CtxEntry { data, ref_count: 1 });
                new_id
            });
            rv.set(v8::Integer::new_from_unsigned(scope, new_id as u32).into());
        },
    )
    .unwrap();

    let async_ctx_read = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              mut rv: ReturnValue<'_>| {
            let ctx_id = args.get(0).uint32_value(scope).unwrap_or(0);
            let als_key = args
                .get(1)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();

            let result = with_store(|store| {
                store
                    .get(&(ctx_id as u64))
                    .and_then(|e| e.data.get(&als_key).cloned())
            });

            if let Some(s) = result {
                rv.set(v8::String::new(scope, &s).unwrap().into());
            }
        },
    )
    .unwrap();

    let async_ctx_retain = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              _rv: ReturnValue<'_>| {
            let ctx_id = args.get(0).uint32_value(scope).unwrap_or(0);
            with_store(|store| {
                if let Some(e) = store.get_mut(&(ctx_id as u64)) {
                    e.ref_count += 1;
                }
            });
        },
    )
    .unwrap();

    let async_ctx_free = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              _rv: ReturnValue<'_>| {
            let ctx_id = args.get(0).uint32_value(scope).unwrap_or(0);
            with_store(|store| {
                let remove = if let Some(e) = store.get_mut(&(ctx_id as u64)) {
                    e.ref_count = e.ref_count.saturating_sub(1);
                    e.ref_count == 0
                } else {
                    false
                };
                if remove {
                    store.remove(&(ctx_id as u64));
                }
            });
        },
    )
    .unwrap();

    let context = scope.get_current_context();
    let global = context.global(scope);
    global.set(
        scope,
        v8::String::new(scope, "__asyncCtxSet").unwrap().into(),
        async_ctx_set.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__asyncCtxGet").unwrap().into(),
        async_ctx_get.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__asyncCtxAlloc").unwrap().into(),
        async_ctx_alloc.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__asyncCtxRead").unwrap().into(),
        async_ctx_read.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__asyncCtxRetain").unwrap().into(),
        async_ctx_retain.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__asyncCtxFree").unwrap().into(),
        async_ctx_free.into(),
    );

    Ok(())
}
