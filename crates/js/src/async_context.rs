/// Async context propagation — runtime-level implementation.
///
/// ## Architecture
///
/// We patched three places in QuickJS's C source:
///
/// 1. `JSPromiseReactionData` gets an `async_ctx_id` field.
///    `perform_promise_then` captures `rt->current_async_ctx_id` into this
///    field at the moment `.then()` is called — i.e., when the async function
///    registers its continuation.
///
/// 2. `fulfill_or_reject_promise` restores `rt->current_async_ctx_id` from
///    the reaction's stored `async_ctx_id` just before calling `JS_EnqueueJob`.
///    This ensures the job entry carries the context from *registration time*,
///    not from *resolution time*.
///
/// 3. `JS_ExecutePendingJob` restores `rt->current_async_ctx_id` from the
///    job entry's `async_ctx_id` before executing the job function.
///
/// Because `rt->current_async_ctx_id` is per-runtime (not global), multiple
/// concurrent JsEngine instances (e.g. in parallel tests) are fully isolated.
///
/// No Rust-side global state is needed: `__asyncCtxGet` / `__asyncCtxSet`
/// read and write `rt->current_async_ctx_id` directly via FFI.
use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::Mutex;

// ── FFI to our patched QuickJS ────────────────────────────────────────────────

unsafe extern "C" {
    fn JS_SetCurrentAsyncCtxId(rt: *mut std::ffi::c_void, id: u64);
    fn JS_GetCurrentAsyncCtxId(rt: *mut std::ffi::c_void) -> u64;
}

// ── Context data store ────────────────────────────────────────────────────────

struct CtxEntry {
    data: HashMap<String, String>,
    ref_count: u32,
}

static STORE: Mutex<Option<HashMap<u64, CtxEntry>>> = Mutex::new(None);

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

// ── Public API ────────────────────────────────────────────────────────────────

/// Expose native async context functions to the JS runtime.
///
/// `rt_ptr` is the raw `*mut JSRuntime` from `ctx.get_runtime_ptr()`.
/// It is captured by value (as `usize`) in closures that must be `'static`.
/// # Safety
/// `rt_ptr` must be a valid `*mut JSRuntime` for the lifetime of the context.
pub unsafe fn install(ctx: &Ctx, rt_ptr: *mut std::ffi::c_void) -> Result<()> {
    // Initialise the runtime's context to 0 (root / no context).
    unsafe { JS_SetCurrentAsyncCtxId(rt_ptr, 0) };

    let rt = rt_ptr as usize; // usize: Send + 'static

    // __asyncCtxSet(id) — called by AsyncLocalStorage.run() to activate a context.
    ctx.globals().set(
        "__asyncCtxSet",
        Function::new(ctx.clone(), move |id: u64| unsafe {
            JS_SetCurrentAsyncCtxId(rt as *mut std::ffi::c_void, id);
        })?,
    )?;

    // __asyncCtxGet() → id — called by AsyncLocalStorage.getStore().
    // Reads rt->current_async_ctx_id which is kept up-to-date by the C patches.
    ctx.globals().set(
        "__asyncCtxGet",
        Function::new(ctx.clone(), move || -> u64 {
            unsafe { JS_GetCurrentAsyncCtxId(rt as *mut std::ffi::c_void) }
        })?,
    )?;

    // __asyncCtxAlloc(parent_id, als_key, json_value) → new_id
    ctx.globals().set(
        "__asyncCtxAlloc",
        Function::new(
            ctx.clone(),
            |parent_id: u64, als_key: String, json_value: String| -> u64 {
                with_store(|store| {
                    let new_id = next_id();
                    let parent_data = if parent_id != 0 {
                        store
                            .get(&parent_id)
                            .map(|e| e.data.clone())
                            .unwrap_or_default()
                    } else {
                        HashMap::new()
                    };
                    let mut data = parent_data;
                    data.insert(als_key, json_value);
                    store.insert(new_id, CtxEntry { data, ref_count: 1 });
                    new_id
                })
            },
        )?,
    )?;

    // __asyncCtxRead(ctx_id, als_key) → string | undefined
    ctx.globals().set(
        "__asyncCtxRead",
        Function::new(
            ctx.clone(),
            |ctx_id: u64, als_key: String| -> Option<String> {
                with_store(|store| {
                    store
                        .get(&ctx_id)
                        .and_then(|e| e.data.get(&als_key))
                        .cloned()
                })
            },
        )?,
    )?;

    // __asyncCtxRetain(ctx_id) — AsyncResource holds an extra ref
    ctx.globals().set(
        "__asyncCtxRetain",
        Function::new(ctx.clone(), |ctx_id: u64| {
            with_store(|store| {
                if let Some(e) = store.get_mut(&ctx_id) {
                    e.ref_count += 1;
                }
            });
        })?,
    )?;

    // __asyncCtxFree(ctx_id) — decrement ref; remove when it hits 0
    ctx.globals().set(
        "__asyncCtxFree",
        Function::new(ctx.clone(), |ctx_id: u64| {
            with_store(|store| {
                let remove = if let Some(e) = store.get_mut(&ctx_id) {
                    e.ref_count = e.ref_count.saturating_sub(1);
                    e.ref_count == 0
                } else {
                    false
                };
                if remove {
                    store.remove(&ctx_id);
                }
            });
        })?,
    )?;

    Ok(())
}
