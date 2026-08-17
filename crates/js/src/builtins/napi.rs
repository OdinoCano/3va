//! Node-API (NAPI) compatibility layer for V8.
//!
//! Native `.node` addons expose a C ABI whose symbols we implement here as
//! `#[unsafe(no_mangle)] unsafe extern "C" fn`. When `require()` loads a .node
//! file, the loader (`napi_load_module` below) looks up
//! `napi_register_module_v1`, then calls into the addon with a `NapiEnv` it
//! can use to talk back to V8 through these functions.
//!
//! NAPI glue is inherently unsafe FFI. The `unsafe` blocks just add noise
//! here — callers (native addons) are responsible for passing valid
//! napi_env / napi_value pointers per the NAPI ABI contract.
//!
//! Debug tracing: `NAPI_TRACE` records the most recent ~500 NAPI entrypoints
//! so that when a callback receives a null pointer it can dump the path that
//! produced the null. Toggle with `3VA_NAPI_TRACE=1`.
//!
//! Scope handling: when a NAPI function is called from inside a V8 callback
//! (the case for `napi_bridge_callback`), we reuse the existing
//! `PinScope`/`ContextScope` rather than nesting a fresh `HandleScope`, which
//! would violate V8's nesting invariant. `NAPI_CB_SCOPE` exposes the current
//! callback's scope to the macro so it can pick the right path.
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::missing_safety_doc)]

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use libloading::Library;
use v8::{FunctionCallbackArguments, PinScope, ReturnValue};
use vvva_permissions::{Capability, PermissionState};

type NapiEnvHandle = *mut NapiEnv;
type NapiValue = *mut NapiValueInner;
type NapiRef = *mut NapiRefInner;
type NapiDeferred = *mut NapiDeferredInner;
type NapiCallbackInfo = *mut NapiCallbackInfoInner;
type NapiAsyncWork = *mut c_void;
type NapiThreadsafeFunction = *mut c_void;
type NapiStatus = i32;
type NapiValuetype = i32;

const NAPI_OK: NapiStatus = 0;
const NAPI_INVALID_ARG: NapiStatus = 1;
const NAPI_GENERIC_FAILURE: NapiStatus = 8;
const NAPI_OBJECT_EXPECTED: NapiStatus = 15;
const NAPI_STRING_EXPECTED: NapiStatus = 3;

const NAPI_UNDEFINED: NapiValuetype = 0;
const NAPI_NULL: NapiValuetype = 1;
const NAPI_BOOLEAN: NapiValuetype = 2;
const NAPI_NUMBER: NapiValuetype = 3;
const NAPI_STRING: NapiValuetype = 4;
const NAPI_OBJECT: NapiValuetype = 6;
const NAPI_FUNCTION: NapiValuetype = 7;
const NAPI_BIGINT: NapiValuetype = 9;

type NapiCallback = unsafe extern "C" fn(NapiEnvHandle, NapiCallbackInfo) -> NapiValue;
type NapiFinalizer = unsafe extern "C" fn(NapiEnvHandle, *mut c_void, *mut c_void);

struct NapiEnv {
    isolate: *mut v8::RealIsolate,
    context: v8::Global<v8::Context>,
    values: Vec<NapiValue>,
    pending_exception: Option<v8::Global<v8::Value>>,
    cleanup_hooks: Vec<(unsafe extern "C" fn(*mut c_void), *mut c_void)>,
    _library: Option<Arc<Library>>,
}

struct NapiValueInner {
    global: v8::Global<v8::Value>,
}

struct NapiRefInner {
    global: Option<v8::Global<v8::Value>>,
    refcount: u32,
}

struct NapiDeferredInner {
    resolver: v8::Global<v8::PromiseResolver>,
}

struct NapiCallbackInfoInner {
    argc: usize,
    argv: Vec<NapiValue>,
    this_arg: NapiValue,
    new_target: NapiValue,
    data: *mut c_void,
}

struct NapiBridge {
    cb: NapiCallback,
    data: *mut c_void,
    env: NapiEnvHandle,
}

struct AsyncWorkInner {
    env: NapiEnvHandle,
    exec: Option<unsafe extern "C" fn(NapiEnvHandle, *mut c_void)>,
    complete: Option<unsafe extern "C" fn(NapiEnvHandle, NapiStatus, *mut c_void)>,
    data: *mut c_void,
    cancelled: Arc<AtomicBool>,
}

struct ThreadsafeFunctionInner {
    env: NapiEnvHandle,
    js_func: NapiValue,
    context: *mut c_void,
    call_js: Option<unsafe extern "C" fn(NapiEnvHandle, NapiValue, *mut c_void, *mut c_void)>,
}

/// Completions marshaled from native worker threads to the main V8 thread.
/// `run_event_loop` drains this via `drain_async_completions()`.
static NAPI_ASYNC_COMPLETIONS: Mutex<VecDeque<Box<dyn FnOnce() + Send>>> =
    Mutex::new(VecDeque::new());

/// Raw pointers captured by a completion closure. Raw pointers aren't `Send`
/// by default; the worker thread owns the pointer until the closure runs on
/// the main thread, which is safe here.
struct CompleteCtx {
    env: NapiEnvHandle,
    data: *mut c_void,
}
unsafe impl Send for CompleteCtx {}

struct TSCallCtx {
    env: NapiEnvHandle,
    js_func: NapiValue,
    ctx: *mut c_void,
    d: *mut c_void,
}
unsafe impl Send for TSCallCtx {}

#[repr(C)]
struct NapiPropertyDescriptor {
    utf8name: *const c_char,
    value: NapiValue,
    getter: Option<unsafe extern "C" fn(NapiEnvHandle, NapiCallbackInfo) -> NapiValue>,
    setter: Option<unsafe extern "C" fn(NapiEnvHandle, NapiCallbackInfo) -> NapiValue>,
    attributes: i32,
    data: *mut c_void,
}

thread_local! {
    static NAPI_PERMISSIONS: Cell<Option<Arc<PermissionState>>> = const { Cell::new(None) };
    // Current V8 callback scope, set by napi_bridge_callback before invoking
    // the user's NAPI callback. Read by napi_scope! to know it should reuse
    // the existing scope instead of nesting a fresh HandleScope.
    static NAPI_CB_SCOPE: Cell<*const ()> = const { Cell::new(std::ptr::null()) };
    // Rolling trace buffer for debugging null-pointer regressions.
    static NAPI_TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    // Global atomic counter to ensure trace indices are unique even when the
    // trace buffer is per-thread (some calls happen on worker threads).
    static NAPI_TRACE_SEQ: Cell<u64> = const { Cell::new(0) };
}

fn trace_enabled() -> bool {
    std::env::var("3VA_NAPI_TRACE").is_ok()
}

macro_rules! napi_trace {
    ($name:literal) => {{
        if trace_enabled() {
            let seq = NAPI_TRACE_SEQ.with(|s| {
                let v = s.get();
                s.set(v + 1);
                v
            });
            NAPI_TRACE.with(|t| {
                let mut v = t.borrow_mut();
                if v.len() < 500 {
                    v.push(format!(
                        "[{}] {} @ {:?}",
                        seq,
                        $name,
                        std::ptr::null::<u8>()
                    ));
                }
            });
        }
    }};
}

#[inline]
fn trace_call(name: &'static str) {
    if trace_enabled() {
        let seq = NAPI_TRACE_SEQ.with(|s| {
            let v = s.get();
            s.set(v + 1);
            v
        });
        NAPI_TRACE.with(|t| {
            let mut v = t.borrow_mut();
            if v.len() < 500 {
                v.push(format!("[{}] {} @ {:?}", seq, name, std::ptr::null::<u8>()));
            }
        });
    }
}

#[repr(C)]
struct NapiModule {
    nm_version: i32,
    nm_flags: u32,
    nm_filename: *const c_char,
    nm_register_func: Option<unsafe extern "C" fn(NapiEnvHandle, NapiValue) -> NapiValue>,
    nm_modname: *const c_char,
    nm_priv_data: *mut c_void,
}

static mut NAPI_REGISTERED_MODULE: Option<NapiModule> = None;

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_module_register(module: *const NapiModule) {
    napi_trace!("napi_module_register");
    if !module.is_null() {
        *std::ptr::addr_of_mut!(NAPI_REGISTERED_MODULE) = Some(std::ptr::read(module));
    }
}

unsafe fn store_value(env: &mut NapiEnv, global: v8::Global<v8::Value>) -> NapiValue {
    let inner = Box::new(NapiValueInner { global });
    let ptr = Box::into_raw(inner);
    env.values.push(ptr);
    ptr
}

/// Pull a Local<Value> out of a stored NapiValue using the current scope's
/// isolate. Caller must already be in a HandleScope for this isolate.
unsafe fn get_local<'a>(
    scope: &'a PinScope<'_, '_>,
    val: NapiValue,
) -> Option<v8::Local<'a, v8::Value>> {
    if val.is_null() {
        return None;
    }
    Some(v8::Local::new(scope, &(*val).global))
}

/// Two-branch scope macro:
///   - If we're inside a V8 callback (NAPI_CB_SCOPE set), reuse it.
///   - Otherwise, create a fresh HandleScope+ContextScope on env's isolate.
///
/// The first path avoids the "sibling HandleScope violates V8 nesting" panic.
// ponytail: napi_scope! pastes $body into two branches whose `$cs` has a
// different concrete type (a `&mut PinScope` reborrow vs. an owned
// `ContextScope`), so clippy's per-call-site suggestions (drop this `&mut`,
// this `mut` is unused, ...) are only valid for one branch and silently
// wrong for the other — that's what turned a blanket `clippy --fix` into
// dozens of E0308s. Suppressed here, scoped to just this macro's expansion,
// instead of chasing ~300 call sites by hand for a lint that's a false
// positive by construction.
macro_rules! napi_scope {
    ($env:expr, $cs:ident, $body:block) => {{
        let in_cb = !NAPI_CB_SCOPE.with(|s| s.get().is_null());
        #[allow(
            unused_mut,
            clippy::needless_borrow,
            clippy::needless_return,
            clippy::useless_conversion,
            clippy::collapsible_if,
            clippy::needless_bool
        )]
        if in_cb {
            // Reuse the active callback scope; skip fresh HandleScope creation.
            let cb_ptr = NAPI_CB_SCOPE.with(|s| s.get());
            let cb_ref: &mut PinScope<'_, '_> = &mut *(cb_ptr as *mut PinScope<'_, '_>);
            let mut $cs = cb_ref;
            $body
        } else {
            let env_ref: &mut NapiEnv = unsafe { &mut *$env };
            // Reconstruct the real isolate: env stores the `*mut RealIsolate`
            // (stable for the isolate's lifetime), wrapped here per call. A
            // reborrow of the creating scope's internal isolate field would be
            // a dangling stack pointer once that scope has dropped.
            let mut isolate = unsafe { v8::Isolate::from_raw_ptr(env_ref.isolate) };
            let mut scope_storage = Box::pin(v8::HandleScope::new(&mut isolate));
            let mut hs = scope_storage.as_mut().init();
            // ponytail: Context::new per-call instead of reusing a stored Global.
            // A stored Global<Context> can outlive the isolate that created it
            // (e.g. when NAPI state is held across isolate lifetimes), so we
            // build a fresh one each time rather than risk an isolate-host
            // mismatch when napi_create_reference is called with a stale pointer.
            let ctx = v8::Context::new(&mut hs, Default::default());
            let mut $cs = v8::ContextScope::new(&mut hs, ctx);
            $body
        }
    }};
}

macro_rules! store {
    ($cs:expr, $env:expr, $expr:expr) => {{
        let local: v8::Local<v8::Value> = $expr.into();
        let g = v8::Global::new($cs, local);
        store_value($env, g)
    }};
}

unsafe fn backing_data(bs: &v8::BackingStore) -> *mut c_void {
    bs.data().map_or(std::ptr::null_mut(), |nn| nn.as_ptr())
}

// ── Value creation ──────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_undefined(env: NapiEnvHandle, result: *mut NapiValue) -> NapiStatus {
    napi_trace!("napi_get_undefined");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::undefined(&cs));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_null(env: NapiEnvHandle, result: *mut NapiValue) -> NapiStatus {
    napi_trace!("napi_get_null");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::null(&cs));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_boolean(
    env: NapiEnvHandle,
    value: bool,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_get_boolean");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::Boolean::new(&cs, value));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_global(env: NapiEnvHandle, result: *mut NapiValue) -> NapiStatus {
    napi_trace!("napi_get_global");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let g = cs.get_current_context().global(&cs);
        *result = store!(&cs, &mut *env, g);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_string_utf8(
    env: NapiEnvHandle,
    str_ptr: *const c_char,
    len: usize,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_string_utf8");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let slice = if len == usize::MAX {
        CStr::from_ptr(str_ptr).to_bytes()
    } else {
        std::slice::from_raw_parts(str_ptr as *const u8, len)
    };
    let s = std::str::from_utf8_unchecked(slice).to_owned();
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::String::new(&cs, &s).unwrap());
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_double(
    env: NapiEnvHandle,
    value: f64,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_double");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::Number::new(&cs, value));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_int32(
    env: NapiEnvHandle,
    value: i32,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_int32");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::Integer::new(&cs, value));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_uint32(
    env: NapiEnvHandle,
    value: u32,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_uint32");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::Integer::new_from_unsigned(&cs, value));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_int64(
    env: NapiEnvHandle,
    value: i64,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_int64");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::Number::new(&cs, value as f64));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_bigint_int64(
    env: NapiEnvHandle,
    value: i64,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_bigint_int64");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::BigInt::new_from_i64(&cs, value));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_object(env: NapiEnvHandle, result: *mut NapiValue) -> NapiStatus {
    napi_trace!("napi_create_object");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::Object::new(&cs));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_array(env: NapiEnvHandle, result: *mut NapiValue) -> NapiStatus {
    napi_trace!("napi_create_array");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::Array::new(&cs, 0));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_array_with_length(
    env: NapiEnvHandle,
    length: usize,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_array_with_length");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::Array::new(&cs, length as i32));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_function(
    env: NapiEnvHandle,
    name: *const c_char,
    name_len: usize,
    cb: Option<NapiCallback>,
    data: *mut c_void,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_function");
    if env.is_null() || result.is_null() || cb.is_none() {
        return NAPI_INVALID_ARG;
    }
    let cb = cb.unwrap();
    let fn_name = if !name.is_null() {
        let slice = if name_len == usize::MAX {
            CStr::from_ptr(name).to_bytes()
        } else {
            std::slice::from_raw_parts(name as *const u8, name_len)
        };
        std::str::from_utf8_unchecked(slice).to_owned()
    } else {
        String::new()
    };
    napi_scope!(env, cs, {
        let bridge = Box::new(NapiBridge { cb, data, env });
        let bridge_ptr = Box::into_raw(bridge) as *mut c_void;
        let external = v8::External::new(&cs, bridge_ptr);
        match v8::Function::builder(napi_bridge_callback)
            .data(external.into())
            .build(&cs)
        {
            Some(f) => {
                if !fn_name.is_empty() {
                    f.set_name(v8::String::new(&cs, &fn_name).unwrap());
                }
                *result = store!(&cs, &mut *env, f);
                return NAPI_OK;
            }
            None => return NAPI_GENERIC_FAILURE,
        }
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_buffer(
    env: NapiEnvHandle,
    size: usize,
    data: *mut *mut c_void,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_buffer");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let backing = v8::ArrayBuffer::new_backing_store(&mut cs, size);
        if !data.is_null() {
            *data = backing_data(&backing);
        }
        let ab = v8::ArrayBuffer::with_backing_store(&cs, &backing.make_shared());
        let buf = v8::Uint8Array::new(&cs, ab, 0, size).unwrap();
        *result = store!(&cs, &mut *env, buf);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_buffer_copy(
    env: NapiEnvHandle,
    size: usize,
    src: *const c_void,
    result_data: *mut *mut c_void,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_buffer_copy");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let backing = v8::ArrayBuffer::new_backing_store(&mut cs, size);
        if !src.is_null() && size > 0 {
            std::ptr::copy_nonoverlapping(
                src as *const u8,
                backing_data(&backing) as *mut u8,
                size,
            );
        }
        if !result_data.is_null() {
            *result_data = backing_data(&backing);
        }
        let ab = v8::ArrayBuffer::with_backing_store(&cs, &backing.make_shared());
        let buf = v8::Uint8Array::new(&cs, ab, 0, size).unwrap();
        *result = store!(&cs, &mut *env, buf);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_external_buffer(
    env: NapiEnvHandle,
    size: usize,
    data: *mut c_void,
    _fcb: Option<NapiFinalizer>,
    _fhint: *mut c_void,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_external_buffer");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let backing = v8::ArrayBuffer::new_backing_store(&mut cs, size);
        if !data.is_null() && size > 0 {
            std::ptr::copy_nonoverlapping(
                data as *const u8,
                backing_data(&backing) as *mut u8,
                size,
            );
        }
        let ab = v8::ArrayBuffer::with_backing_store(&cs, &backing.make_shared());
        let buf = v8::Uint8Array::new(&cs, ab, 0, size).unwrap();
        *result = store!(&cs, &mut *env, buf);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_external(
    env: NapiEnvHandle,
    data: *mut c_void,
    _fcb: Option<NapiFinalizer>,
    _fhint: *mut c_void,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_external");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::External::new(&cs, data));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_promise(
    env: NapiEnvHandle,
    deferred: *mut NapiDeferred,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_promise");
    if env.is_null() || deferred.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let resolver = v8::PromiseResolver::new(&cs).unwrap();
        let promise = resolver.get_promise(&cs);
        *deferred = Box::into_raw(Box::new(NapiDeferredInner {
            resolver: v8::Global::new(&cs, resolver),
        }));
        *result = store!(&cs, &mut *env, promise);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_error(
    env: NapiEnvHandle,
    _code: NapiValue,
    msg: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_error");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let message =
            get_local(&cs, msg).unwrap_or_else(|| v8::String::new(&cs, "Error").unwrap().into());
        let msg_str = v8::Local::<v8::String>::try_from(message)
            .unwrap_or_else(|_| v8::String::new(&cs, "Error").unwrap());
        *result = store!(&cs, &mut *env, v8::Exception::error(&cs, msg_str));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_type_error(
    env: NapiEnvHandle,
    _code: NapiValue,
    msg: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_type_error");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let message =
            get_local(&cs, msg).unwrap_or_else(|| v8::String::new(&cs, "Error").unwrap().into());
        let msg_str = v8::Local::<v8::String>::try_from(message)
            .unwrap_or_else(|_| v8::String::new(&cs, "Error").unwrap());
        *result = store!(&cs, &mut *env, v8::Exception::type_error(&cs, msg_str));
        return NAPI_OK;
    })
}

// ── Value reading ───────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_string_utf8(
    env: NapiEnvHandle,
    value: NapiValue,
    buf: *mut c_char,
    bufsize: usize,
    result: *mut usize,
) -> NapiStatus {
    napi_trace!("napi_get_value_string_utf8");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    // ponytail: Node-API's napi_get_value_string_utf8 returns napi_string_expected
    // for non-string values (including null/undefined). Several SWC options are
    // `Option<String>` and the from_napi_value path checks the status to decide
    // whether the field was absent; coercing non-strings to "undefined" /
    // "[object Object]" would let JSON deserializers see garbage. Match Node.
    if value.is_null() {
        if !result.is_null() {
            *result = 0;
        }
        if !buf.is_null() && bufsize > 0 {
            *buf = 0;
        }
        return NAPI_STRING_EXPECTED;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if !local.is_string() {
            if !result.is_null() {
                *result = 0;
            }
            if !buf.is_null() && bufsize > 0 {
                *buf = 0;
            }
            return NAPI_STRING_EXPECTED;
        }
        let s = local.to_rust_string_lossy(&cs);
        let bytes = s.as_bytes();
        if !result.is_null() {
            *result = bytes.len();
        }
        if !buf.is_null() && bufsize > 0 {
            let n = bytes.len().min(bufsize - 1);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
            *buf.add(n) = 0;
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_double(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut f64,
) -> NapiStatus {
    napi_trace!("napi_get_value_double");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = l.number_value(&cs).unwrap_or(0.0);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_int32(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut i32,
) -> NapiStatus {
    napi_trace!("napi_get_value_int32");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = l.int32_value(&cs).unwrap_or(0);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_uint32(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut u32,
) -> NapiStatus {
    napi_trace!("napi_get_value_uint32");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = l.uint32_value(&cs).unwrap_or(0);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_int64(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut i64,
) -> NapiStatus {
    napi_trace!("napi_get_value_int64");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = l.number_value(&cs).unwrap_or(0.0) as i64;
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_bool(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_get_value_bool");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = l.boolean_value(&cs);
        if trace_enabled() {
            eprintln!("[napi_get_value_bool] value={:p} result={}", value, *result);
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_buffer_info(
    env: NapiEnvHandle,
    value: NapiValue,
    data: *mut *mut c_void,
    byte_length: *mut usize,
) -> NapiStatus {
    napi_trace!("napi_get_buffer_info");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    // SWC's options struct contains a Buffer field that may be undefined
    // (e.g. when source maps aren't requested). Its
    // `Option<Buffer>::from_napi_value` ends up calling this with a NULL
    // napi_value, which Node-API tolerates by setting *data=NULL and
    // *len=0. Mirror that so SWC's "Failed to get Buffer pointer and
    // length" check succeeds instead of bubbling up.
    if value.is_null() {
        if !data.is_null() {
            *data = std::ptr::null_mut();
        }
        if !byte_length.is_null() {
            *byte_length = 0;
        }
        if trace_enabled() {
            eprintln!("[napi_get_buffer_info] value=null (returning empty)");
            NAPI_TRACE.with(|t| {
                let trace = t.borrow();
                for (i, s) in trace.iter().enumerate() {
                    eprintln!("  {:3}: {}", i, s);
                }
            });
        }
        return NAPI_OK;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(ta) = v8::Local::<v8::Uint8Array>::try_from(local) {
            let ab = ta.buffer(&cs).unwrap();
            if !data.is_null() {
                *data = backing_data(&ab.get_backing_store());
            }
            if !byte_length.is_null() {
                *byte_length = ta.byte_length();
            }
            if trace_enabled() {
                eprintln!(
                    "[napi_get_buffer_info] value={:p} data={:p} len={}",
                    value,
                    *data,
                    ta.byte_length()
                );
            }
            return NAPI_OK;
        }
        if trace_enabled() {
            eprintln!("[napi_get_buffer_info] value={:p} NOT a Uint8Array", value);
        }
        return NAPI_INVALID_ARG;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_arraybuffer_info(
    env: NapiEnvHandle,
    value: NapiValue,
    data: *mut *mut c_void,
    byte_length: *mut usize,
) -> NapiStatus {
    napi_trace!("napi_get_arraybuffer_info");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(local) {
            if !data.is_null() {
                *data = backing_data(&ab.get_backing_store());
            }
            if !byte_length.is_null() {
                *byte_length = ab.byte_length();
            }
            return NAPI_OK;
        }
        return NAPI_INVALID_ARG;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_typedarray_info(
    env: NapiEnvHandle,
    value: NapiValue,
    _ty: *mut i32,
    length: *mut usize,
    data: *mut *mut c_void,
    arraybuffer: *mut NapiValue,
    byte_offset: *mut usize,
) -> NapiStatus {
    napi_trace!("napi_get_typedarray_info");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(ta) = v8::Local::<v8::TypedArray>::try_from(local) {
            if !length.is_null() {
                *length = ta.length();
            }
            if !byte_offset.is_null() {
                *byte_offset = ta.byte_offset();
            }
            if !data.is_null() || !arraybuffer.is_null() {
                let ab = ta.buffer(&cs).unwrap();
                if !data.is_null() {
                    *data = (backing_data(&ab.get_backing_store()) as *mut u8).add(ta.byte_offset())
                        as *mut c_void;
                }
                if !arraybuffer.is_null() {
                    *arraybuffer = store!(&cs, &mut *env, ab);
                }
            }
            return NAPI_OK;
        }
        return NAPI_INVALID_ARG;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_array_length(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut u32,
) -> NapiStatus {
    napi_trace!("napi_get_array_length");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = v8::Local::<v8::Array>::try_from(l).map_or(0, |a| a.length());
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_element(
    env: NapiEnvHandle,
    object: NapiValue,
    index: u32,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_get_element");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            let elem = obj
                .get_index(&cs, index)
                .unwrap_or_else(|| v8::undefined(&cs).into());
            let g: v8::Local<v8::Value> = elem.into();
            *result = store!(&cs, &mut *env, g);
            return NAPI_OK;
        }
        return NAPI_INVALID_ARG;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_named_property(
    env: NapiEnvHandle,
    object: NapiValue,
    name: *const c_char,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_get_named_property");
    if env.is_null() || result.is_null() || name.is_null() {
        return NAPI_INVALID_ARG;
    }
    let name_str = CStr::from_ptr(name).to_str().unwrap_or("").to_owned();
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            let key = v8::String::new(&cs, &name_str).unwrap();
            let val = obj
                .get(&cs, key.into())
                .unwrap_or_else(|| v8::undefined(&cs).into());
            let g: v8::Local<v8::Value> = val.into();
            *result = store!(&cs, &mut *env, g);
            return NAPI_OK;
        }
        return NAPI_INVALID_ARG;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_cb_info(
    _env: NapiEnvHandle,
    info: NapiCallbackInfo,
    argc: *mut usize,
    argv: *mut NapiValue,
    this_arg: *mut NapiValue,
    data: *mut *mut c_void,
) -> NapiStatus {
    napi_trace!("napi_get_cb_info");
    if info.is_null() {
        return NAPI_INVALID_ARG;
    }
    let ci = &*info;
    if !argc.is_null() {
        let count = (*argc).min(ci.argc);
        if !argv.is_null() {
            for i in 0..count {
                *argv.add(i) = ci.argv[i];
            }
        }
        if trace_enabled() {
            eprintln!(
                "[napi_get_cb_info] ci.argc={}, *argc={}, count={}, argv[0]={:p}, argv[1]={:p}",
                ci.argc,
                *argc,
                count,
                if count >= 1 {
                    *argv
                } else {
                    std::ptr::null_mut()
                },
                if count >= 2 {
                    *argv.add(1)
                } else {
                    std::ptr::null_mut()
                }
            );
        }
        *argc = count;
    }
    if !this_arg.is_null() {
        *this_arg = ci.this_arg;
    }
    if !data.is_null() {
        *data = ci.data;
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_new_target(
    _env: NapiEnvHandle,
    info: NapiCallbackInfo,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_get_new_target");
    if info.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    *result = (*info).new_target;
    NAPI_OK
}

// ── Object operations ───────────────────────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_set_named_property(
    env: NapiEnvHandle,
    object: NapiValue,
    name: *const c_char,
    value: NapiValue,
) -> NapiStatus {
    napi_trace!("napi_set_named_property");
    if env.is_null() || name.is_null() {
        return NAPI_INVALID_ARG;
    }
    let name_str = CStr::from_ptr(name).to_str().unwrap_or("").to_owned();
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let val = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            obj.set(&cs, v8::String::new(&cs, &name_str).unwrap().into(), val);
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_has_named_property(
    env: NapiEnvHandle,
    object: NapiValue,
    name: *const c_char,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_has_named_property");
    if env.is_null() || result.is_null() || name.is_null() {
        return NAPI_INVALID_ARG;
    }
    let name_str = CStr::from_ptr(name).to_str().unwrap_or("").to_owned();
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            *result = obj
                .has(&cs, v8::String::new(&cs, &name_str).unwrap().into())
                .unwrap_or(false);
        } else {
            *result = false;
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_set_element(
    env: NapiEnvHandle,
    object: NapiValue,
    index: u32,
    value: NapiValue,
) -> NapiStatus {
    napi_trace!("napi_set_element");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let val = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            obj.set_index(&cs, index, val);
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_define_properties(
    env: NapiEnvHandle,
    object: NapiValue,
    count: usize,
    properties: *const NapiPropertyDescriptor,
) -> NapiStatus {
    napi_trace!("napi_define_properties");
    if env.is_null() || (count > 0 && properties.is_null()) {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let obj = match v8::Local::<v8::Object>::try_from(obj) {
            Ok(o) => o,
            Err(_) => return NAPI_INVALID_ARG,
        };
        for i in 0..count {
            let prop = &*properties.add(i);
            if !prop.utf8name.is_null() {
                let name = CStr::from_ptr(prop.utf8name).to_str().unwrap_or("");
                let key = v8::String::new(&cs, name).unwrap();
                if !prop.value.is_null() {
                    if let Some(val) = get_local(&cs, prop.value) {
                        obj.set(&cs, key.into(), val);
                    }
                }
            }
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_set_property(
    env: NapiEnvHandle,
    object: NapiValue,
    key: NapiValue,
    value: NapiValue,
) -> NapiStatus {
    napi_trace!("napi_set_property");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let k = match get_local(&cs, key) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let v = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            obj.set(&cs, k, v);
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_property(
    env: NapiEnvHandle,
    object: NapiValue,
    key: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_get_property");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let k = match get_local(&cs, key) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            let val = obj.get(&cs, k).unwrap_or_else(|| v8::undefined(&cs).into());
            let g: v8::Local<v8::Value> = val.into();
            *result = store!(&cs, &mut *env, g);
            return NAPI_OK;
        }
        return NAPI_INVALID_ARG;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_delete_property(
    env: NapiEnvHandle,
    object: NapiValue,
    key: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_delete_property");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let k = match get_local(&cs, key) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            if !result.is_null() {
                *result = obj.delete(&cs, k).unwrap_or(false);
            }
        }
        return NAPI_OK;
    })
}

// ── Type checks ─────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_typeof(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut NapiValuetype,
) -> NapiStatus {
    napi_trace!("napi_typeof");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    // NULL napi_value = missing/optional arg → treat as undefined (matches Node.js NAPI behavior)
    if value.is_null() {
        *result = NAPI_UNDEFINED;
        return NAPI_OK;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = if l.is_undefined() {
            NAPI_UNDEFINED
        } else if l.is_null() {
            NAPI_NULL
        } else if l.is_boolean() {
            NAPI_BOOLEAN
        } else if l.is_number() {
            NAPI_NUMBER
        } else if l.is_string() {
            NAPI_STRING
        } else if l.is_function() {
            NAPI_FUNCTION
        } else if l.is_big_int() {
            NAPI_BIGINT
        } else {
            NAPI_OBJECT
        };
        if trace_enabled() {
            eprintln!("[napi_typeof] value={:p} type={}", value, *result);
        }
        return NAPI_OK;
    })
}

macro_rules! napi_type_check {
    ($name:ident, $check:ident) => {
        #[unsafe(no_mangle)]
        unsafe extern "C" fn $name(
            env: NapiEnvHandle,
            value: NapiValue,
            result: *mut bool,
        ) -> NapiStatus {
            trace_call(stringify!($name));
            if env.is_null() || result.is_null() || value.is_null() {
                return NAPI_INVALID_ARG;
            }
            napi_scope!(env, cs, {
                let l = match get_local(&cs, value) {
                    Some(v) => v,
                    None => return NAPI_INVALID_ARG,
                };
                *result = l.$check();
                return NAPI_OK;
            })
        }
    };
}

napi_type_check!(napi_is_array, is_array);
napi_type_check!(napi_is_null, is_null);
napi_type_check!(napi_is_undefined, is_undefined);
napi_type_check!(napi_is_string, is_string);
napi_type_check!(napi_is_number, is_number);
napi_type_check!(napi_is_function, is_function);
napi_type_check!(napi_is_promise, is_promise);
napi_type_check!(napi_is_error, is_native_error);

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_is_buffer(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_is_buffer");
    if env.is_null() || result.is_null() || value.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        // Accept any Uint8Array as a Buffer — 3va's Buffer is a Uint8Array with modified prototype
        *result = v8::Local::<v8::Uint8Array>::try_from(l).is_ok();
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_is_arraybuffer(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_is_arraybuffer");
    if env.is_null() || result.is_null() || value.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = v8::Local::<v8::ArrayBuffer>::try_from(l).is_ok();
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_is_typedarray(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_is_typedarray");
    if env.is_null() || result.is_null() || value.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = v8::Local::<v8::TypedArray>::try_from(l).is_ok();
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_is_object(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_is_object");
    if env.is_null() || result.is_null() || value.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = l.is_object() && !l.is_function();
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_strict_equals(
    env: NapiEnvHandle,
    lhs: NapiValue,
    rhs: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_strict_equals");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let l = match get_local(&cs, lhs) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let r = match get_local(&cs, rhs) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        *result = l.strict_equals(r);
        return NAPI_OK;
    })
}

// ── Function calls ──────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_call_function(
    env: NapiEnvHandle,
    recv: NapiValue,
    func: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_call_function");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let f = match get_local(&cs, func) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let r = get_local(&cs, recv).unwrap_or_else(|| v8::undefined(&cs).into());
        let f = match v8::Local::<v8::Function>::try_from(f) {
            Ok(f) => f,
            Err(_) => return NAPI_INVALID_ARG,
        };
        let mut args = Vec::with_capacity(argc);
        for i in 0..argc {
            args.push(get_local(&cs, *argv.add(i)).unwrap_or_else(|| v8::undefined(&cs).into()));
        }
        let ret = f
            .call(&cs, r, &args)
            .unwrap_or_else(|| v8::undefined(&cs).into());
        if !result.is_null() {
            let g: v8::Local<v8::Value> = ret.into();
            *result = store!(&cs, &mut *env, g);
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_new_instance(
    env: NapiEnvHandle,
    constructor: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_new_instance");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let c = match get_local(&cs, constructor) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let c = match v8::Local::<v8::Function>::try_from(c) {
            Ok(f) => f,
            Err(_) => return NAPI_INVALID_ARG,
        };
        let mut args = Vec::with_capacity(argc);
        for i in 0..argc {
            args.push(get_local(&cs, *argv.add(i)).unwrap_or_else(|| v8::undefined(&cs).into()));
        }
        match c.new_instance(&cs, &args) {
            Some(inst) => {
                let g: v8::Local<v8::Value> = inst.into();
                *result = store!(&cs, &mut *env, g);
            }
            None => {
                *result = store!(&cs, &mut *env, v8::undefined(&cs));
            }
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_resolve_deferred(
    env: NapiEnvHandle,
    deferred: NapiDeferred,
    resolution: NapiValue,
) -> NapiStatus {
    napi_trace!("napi_resolve_deferred");
    if env.is_null() || deferred.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let resolver = v8::Local::new(&cs, &(*deferred).resolver);
        let val = get_local(&cs, resolution).unwrap_or_else(|| v8::undefined(&cs).into());
        resolver.resolve(&cs, val);
        let _ = Box::from_raw(deferred);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_reject_deferred(
    env: NapiEnvHandle,
    deferred: NapiDeferred,
    rejection: NapiValue,
) -> NapiStatus {
    napi_trace!("napi_reject_deferred");
    if env.is_null() || deferred.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let resolver = v8::Local::new(&cs, &(*deferred).resolver);
        let val = get_local(&cs, rejection).unwrap_or_else(|| v8::undefined(&cs).into());
        resolver.reject(&cs, val);
        let _ = Box::from_raw(deferred);
        return NAPI_OK;
    })
}

// ── References ──────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_reference(
    env: NapiEnvHandle,
    value: NapiValue,
    _refcount: u32,
    result: *mut NapiRef,
) -> NapiStatus {
    napi_trace!("napi_create_reference");
    if trace_enabled() {
        eprintln!("[napi_create_reference] value={:p}", value);
    }
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    // SWC's Buffer::from_napi_value (and several other napi-rs callers)
    // expect create_reference to return a valid ref for any non-null value,
    // but when called from internal helper paths with an uninitialised
    // local napi_value (== NULL), returning InvalidArg aborts the whole
    // transform with "Failed to create reference from Buffer". Returning
    // a null ref here is what Node-API does in practice: callers that
    // dereference it crash loudly instead of producing a misleading error.
    if value.is_null() {
        if !result.is_null() {
            *result = Box::into_raw(Box::new(NapiRefInner {
                global: None,
                refcount: 0,
            }));
        }
        if trace_enabled() {
            NAPI_TRACE.with(|t| {
                let trace = t.borrow();
                eprintln!("[NAPI trace] napi_create_reference with null value");
                for (i, s) in trace.iter().enumerate() {
                    eprintln!("  {:3}: {}", i, s);
                }
            });
        }
        return NAPI_OK;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let global = v8::Global::new(&cs, local);
        *result = Box::into_raw(Box::new(NapiRefInner {
            global: Some(global),
            refcount: 1,
        }));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_delete_reference(_env: NapiEnvHandle, ref_: NapiRef) -> NapiStatus {
    napi_trace!("napi_delete_reference");
    if ref_.is_null() {
        return NAPI_INVALID_ARG;
    }
    let _ = Box::from_raw(ref_);
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_reference_ref(
    _env: NapiEnvHandle,
    ref_: NapiRef,
    result: *mut u32,
) -> NapiStatus {
    napi_trace!("napi_reference_ref");
    if ref_.is_null() {
        return NAPI_INVALID_ARG;
    }
    (*ref_).refcount += 1;
    if !result.is_null() {
        *result = (*ref_).refcount;
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_reference_unref(
    _env: NapiEnvHandle,
    ref_: NapiRef,
    result: *mut u32,
) -> NapiStatus {
    napi_trace!("napi_reference_unref");
    if ref_.is_null() {
        return NAPI_INVALID_ARG;
    }
    (*ref_).refcount = (*ref_).refcount.saturating_sub(1);
    if !result.is_null() {
        *result = (*ref_).refcount;
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_reference_value(
    env: NapiEnvHandle,
    ref_: NapiRef,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_get_reference_value");
    if env.is_null() || result.is_null() || ref_.is_null() {
        return NAPI_INVALID_ARG;
    }
    match &(*ref_).global {
        Some(global) => {
            napi_scope!(env, cs, {
                let local = v8::Local::new(&cs, global);
                let g: v8::Local<v8::Value> = local.into();
                *result = store!(&cs, &mut *env, g);
                return NAPI_OK;
            })
        }
        None => {
            *result = std::ptr::null_mut();
            NAPI_OK
        }
    }
}

// ── Error handling ──────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_last_error_info(
    _env: NapiEnvHandle,
    result: *mut *const c_void,
) -> NapiStatus {
    napi_trace!("napi_get_last_error_info");
    if !result.is_null() {
        *result = std::ptr::null();
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_throw(env: NapiEnvHandle, error: NapiValue) -> NapiStatus {
    napi_trace!("napi_throw");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        if let Some(local) = get_local(&cs, error) {
            cs.throw_exception(local);
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_throw_error(
    env: NapiEnvHandle,
    _code: *const c_char,
    msg: *const c_char,
) -> NapiStatus {
    napi_trace!("napi_throw_error");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    let s = if msg.is_null() {
        "Error"
    } else {
        CStr::from_ptr(msg).to_str().unwrap_or("Error")
    }
    .to_owned();
    napi_scope!(env, cs, {
        cs.throw_exception(v8::Exception::error(&cs, v8::String::new(&cs, &s).unwrap()));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_throw_type_error(
    env: NapiEnvHandle,
    _code: *const c_char,
    msg: *const c_char,
) -> NapiStatus {
    napi_trace!("napi_throw_type_error");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    let s = if msg.is_null() {
        "TypeError"
    } else {
        CStr::from_ptr(msg).to_str().unwrap_or("TypeError")
    }
    .to_owned();
    napi_scope!(env, cs, {
        cs.throw_exception(v8::Exception::type_error(
            &cs,
            v8::String::new(&cs, &s).unwrap(),
        ));
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_is_exception_pending(
    env: NapiEnvHandle,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_is_exception_pending");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    *result = (*env).pending_exception.is_some();
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_and_clear_last_exception(
    env: NapiEnvHandle,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_get_and_clear_last_exception");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    match (*env).pending_exception.take() {
        Some(global) => {
            napi_scope!(env, cs, {
                let local = v8::Local::new(&cs, &global);
                let g: v8::Local<v8::Value> = local.into();
                *result = store!(&cs, &mut *env, g);
                return NAPI_OK;
            })
        }
        None => {
            *result = std::ptr::null_mut();
            NAPI_OK
        }
    }
}

// ── Wrap/unwrap + missing NAPI functions ────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_wrap(
    env: NapiEnvHandle,
    js_object: NapiValue,
    native_object: *mut c_void,
    _finalize_cb: Option<NapiFinalizer>,
    _finalize_hint: *mut c_void,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_wrap");
    if env.is_null() || js_object.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj_local = match get_local(&cs, js_object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let obj = match obj_local.to_object(&cs) {
            Some(o) => o,
            None => return NAPI_INVALID_ARG,
        };
        let ext = v8::External::new(&cs, native_object);
        let key = v8::String::new(&cs, "__napi_wrap__").unwrap();
        obj.set_private(&cs, v8::Private::for_api(&cs, Some(key)), ext.into());
        if !result.is_null() {
            *result = store!(&cs, &mut *env, ext);
        }
        NAPI_OK
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_unwrap(
    env: NapiEnvHandle,
    js_object: NapiValue,
    result: *mut *mut c_void,
) -> NapiStatus {
    napi_trace!("napi_unwrap");
    if env.is_null() || js_object.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj_local = match get_local(&cs, js_object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let obj = match obj_local.to_object(&cs) {
            Some(o) => o,
            None => return NAPI_OBJECT_EXPECTED,
        };
        let key = v8::String::new(&cs, "__napi_wrap__").unwrap();
        let val = match obj.get_private(&cs, v8::Private::for_api(&cs, Some(key))) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if val.is_external() {
            let ext: v8::Local<v8::External> = val.try_into().unwrap();
            *result = ext.value();
            return NAPI_OK;
        }
        NAPI_INVALID_ARG
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_define_class(
    env: NapiEnvHandle,
    name: *const c_char,
    name_len: usize,
    constructor: Option<NapiCallback>,
    data: *mut c_void,
    property_count: usize,
    properties: *const NapiPropertyDescriptor,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_define_class");
    if env.is_null() || result.is_null() || constructor.is_none() {
        return NAPI_INVALID_ARG;
    }
    let class_name = if !name.is_null() {
        let slice = if name_len == usize::MAX {
            CStr::from_ptr(name).to_bytes()
        } else {
            std::slice::from_raw_parts(name as *const u8, name_len)
        };
        std::str::from_utf8_unchecked(slice).to_owned()
    } else {
        String::new()
    };
    let constructor = constructor.unwrap();
    napi_scope!(env, cs, {
        let bridge = Box::new(NapiBridge {
            cb: constructor,
            data,
            env,
        });
        let bridge_ptr = Box::into_raw(bridge) as *mut c_void;
        let external = v8::External::new(&cs, bridge_ptr);
        let func = match v8::Function::builder(napi_bridge_callback)
            .data(external.into())
            .build(&cs)
        {
            Some(f) => f,
            None => return NAPI_GENERIC_FAILURE,
        };
        if !class_name.is_empty() {
            func.set_name(v8::String::new(&cs, &class_name).unwrap());
        }
        let proto = v8::Object::new(&cs);
        for i in 0..property_count {
            let prop = &*properties.add(i);
            if !prop.utf8name.is_null() {
                let name_str = CStr::from_ptr(prop.utf8name).to_str().unwrap_or("");
                let key = v8::String::new(&cs, name_str).unwrap();
                if !prop.value.is_null() {
                    if let Some(val) = get_local(&cs, prop.value) {
                        proto.set(&cs, key.into(), val);
                    }
                }
            }
        }
        func.set_prototype(&cs, proto.into());
        *result = store!(&cs, &mut *env, func);
        NAPI_OK
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_coerce_to_string(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_coerce_to_string");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let str_val = local
            .to_string(&cs)
            .unwrap_or_else(|| v8::String::new(&cs, "").unwrap());
        *result = store!(&cs, &mut *env, str_val);
        NAPI_OK
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_fatal_exception(_env: NapiEnvHandle, _msg: NapiValue) -> NapiStatus {
    napi_trace!("napi_fatal_exception");
    eprintln!("napi_fatal_exception called");
    std::process::exit(1);
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_property_names(
    env: NapiEnvHandle,
    object: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_get_property_names");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj_local = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let obj = match v8::Local::<v8::Object>::try_from(obj_local) {
            Ok(o) => o,
            Err(_) => return NAPI_INVALID_ARG,
        };
        let args = v8::GetPropertyNamesArgs::default();
        let names = obj
            .get_own_property_names(&cs, args)
            .unwrap_or_else(|| v8::Array::new(&cs, 0));
        *result = store!(&cs, &mut *env, names);
        NAPI_OK
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_external(
    env: NapiEnvHandle,
    value: NapiValue,
    result: *mut *mut c_void,
) -> NapiStatus {
    napi_trace!("napi_get_value_external");
    if env.is_null() || result.is_null() || value.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if local.is_external() {
            let ext: v8::Local<v8::External> = local.try_into().unwrap();
            *result = ext.value();
            return NAPI_OK;
        }
        *result = std::ptr::null_mut();
        NAPI_INVALID_ARG
    })
}

// ── Async/threadsafe ────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_async_work(
    e: NapiEnvHandle,
    _r: NapiValue,
    _n: NapiValue,
    exec: Option<unsafe extern "C" fn(NapiEnvHandle, *mut c_void)>,
    comp: Option<unsafe extern "C" fn(NapiEnvHandle, NapiStatus, *mut c_void)>,
    d: *mut c_void,
    result: *mut NapiAsyncWork,
) -> NapiStatus {
    napi_trace!("napi_create_async_work");
    if e.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let work = Box::new(AsyncWorkInner {
        env: e,
        exec,
        complete: comp,
        data: d,
        cancelled: Arc::new(AtomicBool::new(false)),
    });
    *result = Box::into_raw(work) as NapiAsyncWork;
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_queue_async_work(_e: NapiEnvHandle, w: NapiAsyncWork) -> NapiStatus {
    napi_trace!("napi_queue_async_work");
    if w.is_null() {
        return NAPI_INVALID_ARG;
    }
    eprintln!("[napi queue_async_work]");
    let work = &*(w as *const AsyncWorkInner);
    let env = work.env;
    let exec = work.exec;
    let complete = work.complete;
    let data = work.data;
    let cancelled = work.cancelled.clone();
    let ctx = Box::new(CompleteCtx { env, data });
    std::thread::spawn(move || {
        if let Some(exec) = exec {
            unsafe { exec(ctx.env, ctx.data) };
        }
        if cancelled.load(Ordering::SeqCst) {
            return;
        }
        if let Some(complete) = complete {
            NAPI_ASYNC_COMPLETIONS
                .lock()
                .unwrap()
                .push_back(Box::new(move || unsafe {
                    complete(ctx.env, NAPI_OK, ctx.data);
                }));
        }
    });
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_cancel_async_work(_e: NapiEnvHandle, w: NapiAsyncWork) -> NapiStatus {
    napi_trace!("napi_cancel_async_work");
    if w.is_null() {
        return NAPI_INVALID_ARG;
    }
    let work = &*(w as *const AsyncWorkInner);
    work.cancelled.store(true, Ordering::SeqCst);
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_delete_async_work(_e: NapiEnvHandle, w: NapiAsyncWork) -> NapiStatus {
    napi_trace!("napi_delete_async_work");
    if w.is_null() {
        return NAPI_INVALID_ARG;
    }
    drop(Box::from_raw(w as *mut AsyncWorkInner));
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_threadsafe_function(
    e: NapiEnvHandle,
    f: NapiValue,
    _r: NapiValue,
    _n: NapiValue,
    _mq: usize,
    _itc: usize,
    _tfd: *mut c_void,
    _tfc: Option<NapiFinalizer>,
    ctx: *mut c_void,
    cjc: Option<unsafe extern "C" fn(NapiEnvHandle, NapiValue, *mut c_void, *mut c_void)>,
    result: *mut NapiThreadsafeFunction,
) -> NapiStatus {
    napi_trace!("napi_create_threadsafe_function");
    if e.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(e, cs, {
        let local = match get_local(&cs, f) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let inner = Box::new(ThreadsafeFunctionInner {
            env: e,
            js_func: store!(&cs, &mut *e, local),
            context: ctx,
            call_js: cjc,
        });
        *result = Box::into_raw(inner) as NapiThreadsafeFunction;
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_call_threadsafe_function(
    t: NapiThreadsafeFunction,
    d: *mut c_void,
    _m: i32,
) -> NapiStatus {
    napi_trace!("napi_call_threadsafe_function");
    if t.is_null() {
        return NAPI_INVALID_ARG;
    }
    let ts = &*(t as *const ThreadsafeFunctionInner);
    let env = ts.env;
    let call_js = ts.call_js;
    let js_func = ts.js_func;
    let ctx = ts.context;
    let tc = Box::new(TSCallCtx {
        env,
        js_func,
        ctx,
        d,
    });
    let closure: Box<dyn FnOnce() + Send> = Box::new(move || {
        napi_scope!(tc.env, cs, {
            if let Some(call_js) = call_js {
                if let Some(local) = get_local(&cs, tc.js_func) {
                    let handle = store!(&cs, &mut *tc.env, local);
                    call_js(tc.env, handle, tc.ctx, tc.d);
                }
            }
        });
    });
    NAPI_ASYNC_COMPLETIONS.lock().unwrap().push_back(closure);
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_release_threadsafe_function(
    t: NapiThreadsafeFunction,
    _m: i32,
) -> NapiStatus {
    napi_trace!("napi_release_threadsafe_function");
    if t.is_null() {
        return NAPI_INVALID_ARG;
    }
    drop(Box::from_raw(t as *mut ThreadsafeFunctionInner));
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_ref_threadsafe_function(
    _e: NapiEnvHandle,
    _t: NapiThreadsafeFunction,
) -> NapiStatus {
    NAPI_OK
}
#[unsafe(no_mangle)]
unsafe extern "C" fn napi_unref_threadsafe_function(
    _e: NapiEnvHandle,
    _t: NapiThreadsafeFunction,
) -> NapiStatus {
    NAPI_OK
}
#[unsafe(no_mangle)]
unsafe extern "C" fn napi_acquire_threadsafe_function(_t: NapiThreadsafeFunction) -> NapiStatus {
    NAPI_OK
}
#[unsafe(no_mangle)]
unsafe extern "C" fn napi_add_env_cleanup_hook(
    env: NapiEnvHandle,
    fun: Option<unsafe extern "C" fn(*mut c_void)>,
    arg: *mut c_void,
) -> NapiStatus {
    napi_trace!("napi_add_env_cleanup_hook");
    if env.is_null() || fun.is_none() {
        return NAPI_INVALID_ARG;
    }
    (*env).cleanup_hooks.push((fun.unwrap(), arg));
    NAPI_OK
}
#[unsafe(no_mangle)]
unsafe extern "C" fn napi_remove_env_cleanup_hook(
    env: NapiEnvHandle,
    fun: Option<unsafe extern "C" fn(*mut c_void)>,
    arg: *mut c_void,
) -> NapiStatus {
    napi_trace!("napi_remove_env_cleanup_hook");
    if env.is_null() || fun.is_none() {
        return NAPI_INVALID_ARG;
    }
    let f = fun.unwrap();
    (*env)
        .cleanup_hooks
        .retain(|(fn_, a)| !(std::ptr::fn_addr_eq(*fn_, f) && *a == arg));
    NAPI_OK
}
#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_module_file_name(
    _e: NapiEnvHandle,
    r: *mut *const c_char,
) -> NapiStatus {
    if !r.is_null() {
        *r = c"".as_ptr();
    }
    NAPI_OK
}
#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_uv_event_loop(_e: NapiEnvHandle, r: *mut *mut c_void) -> NapiStatus {
    if !r.is_null() {
        *r = std::ptr::null_mut();
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_symbol(
    env: NapiEnvHandle,
    description: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_symbol");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let desc = if description.is_null() {
            None
        } else {
            get_local(&cs, description)
        };
        let sym = match desc {
            Some(d) => {
                if let Ok(s) = v8::Local::<v8::String>::try_from(d) {
                    v8::Symbol::new(&cs, Some(s))
                } else {
                    v8::Symbol::new(&cs, None)
                }
            }
            None => v8::Symbol::new(&cs, None),
        };
        *result = store!(&cs, &mut *env, sym);
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_string_latin1(
    env: NapiEnvHandle,
    str_ptr: *const c_char,
    len: usize,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_string_latin1");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let slice = if len == usize::MAX {
        CStr::from_ptr(str_ptr).to_bytes()
    } else {
        std::slice::from_raw_parts(str_ptr as *const u8, len)
    };
    let s = String::from_utf8_lossy(slice).into_owned();
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::String::new(&cs, &s).unwrap());
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_open_handle_scope(
    _env: NapiEnvHandle,
    _result: *mut *mut c_void,
) -> NapiStatus {
    if !_result.is_null() {
        *_result = std::ptr::dangling_mut::<c_void>();
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_close_handle_scope(
    _env: NapiEnvHandle,
    _scope: *mut c_void,
) -> NapiStatus {
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_open_escapable_handle_scope(
    _env: NapiEnvHandle,
    _result: *mut *mut c_void,
) -> NapiStatus {
    if !_result.is_null() {
        *_result = std::ptr::dangling_mut::<c_void>();
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_close_escapable_handle_scope(
    _env: NapiEnvHandle,
    _scope: *mut c_void,
) -> NapiStatus {
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_escape_handle(
    _env: NapiEnvHandle,
    _scope: *mut c_void,
    escapee: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if !result.is_null() {
        *result = escapee;
    }
    NAPI_OK
}

type NapiCallbackScope = *mut c_void;

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_open_callback_scope(
    _env: NapiEnvHandle,
    _resource: NapiValue,
    _context: NapiValue,
    result: *mut NapiCallbackScope,
) -> NapiStatus {
    if !result.is_null() {
        *result = std::ptr::dangling_mut::<c_void>();
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_close_callback_scope(
    _env: NapiEnvHandle,
    _scope: NapiCallbackScope,
) -> NapiStatus {
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_has_property(
    env: NapiEnvHandle,
    object: NapiValue,
    key: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    napi_trace!("napi_has_property");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let obj = match get_local(&cs, object) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let k = match get_local(&cs, key) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(obj) {
            *result = obj.has(&cs, k).unwrap_or(false);
        } else {
            *result = false;
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_fatal_error(
    _location: *const c_char,
    _location_len: usize,
    _message: *const c_char,
    _message_len: usize,
) {
    let msg = if _message.is_null() {
        "fatal error"
    } else {
        let slice = if _message_len == usize::MAX {
            CStr::from_ptr(_message).to_bytes()
        } else {
            std::slice::from_raw_parts(_message as *const u8, _message_len)
        };
        std::str::from_utf8_unchecked(slice)
    };
    eprintln!("napi_fatal_error: {}", msg);
    std::process::abort();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_async_destroy(
    _env: NapiEnvHandle,
    _async_resource: NapiValue,
) -> NapiStatus {
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_string_utf16(
    env: NapiEnvHandle,
    str_ptr: *const u16,
    len: usize,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_trace!("napi_create_string_utf16");
    if env.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let slice = if len == usize::MAX {
        let mut end = str_ptr;
        while *end != 0 {
            end = end.add(1);
        }
        std::slice::from_raw_parts(str_ptr, end.offset_from(str_ptr) as usize)
    } else {
        std::slice::from_raw_parts(str_ptr, len)
    };
    let s = String::from_utf16_lossy(slice);
    napi_scope!(env, cs, {
        *result = store!(&cs, &mut *env, v8::String::new(&cs, &s).unwrap());
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_string_latin1(
    env: NapiEnvHandle,
    value: NapiValue,
    buf: *mut c_char,
    bufsize: usize,
    result: *mut usize,
) -> NapiStatus {
    napi_trace!("napi_get_value_string_latin1");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let s = local.to_rust_string_lossy(&cs);
        let bytes = s.as_bytes();
        if !result.is_null() {
            *result = bytes.len();
        }
        if !buf.is_null() && bufsize > 0 {
            let n = bytes.len().min(bufsize - 1);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
            *buf.add(n) = 0;
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_value_string_utf16(
    env: NapiEnvHandle,
    value: NapiValue,
    buf: *mut u16,
    bufsize: usize,
    result: *mut usize,
) -> NapiStatus {
    napi_trace!("napi_get_value_string_utf16");
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    napi_scope!(env, cs, {
        let local = match get_local(&cs, value) {
            Some(v) => v,
            None => return NAPI_INVALID_ARG,
        };
        let s = local.to_rust_string_lossy(&cs);
        let utf16: Vec<u16> = s.encode_utf16().collect();
        if !result.is_null() {
            *result = utf16.len();
        }
        if !buf.is_null() && bufsize > 0 {
            let n = utf16.len().min(bufsize - 1);
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), buf, n);
            *buf.add(n) = 0;
        }
        return NAPI_OK;
    })
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_make_callback(
    env: NapiEnvHandle,
    _async_context: *mut c_void,
    recv: NapiValue,
    func: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_call_function(env, recv, func, argc, argv, result)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_create_async_work2(
    env: NapiEnvHandle,
    async_resource: NapiValue,
    async_resource_name: NapiValue,
    execute: Option<unsafe extern "C" fn(NapiEnvHandle, *mut c_void)>,
    complete: Option<unsafe extern "C" fn(NapiEnvHandle, NapiStatus, *mut c_void)>,
    data: *mut c_void,
    result: *mut NapiAsyncWork,
) -> NapiStatus {
    napi_create_async_work(
        env,
        async_resource,
        async_resource_name,
        execute,
        complete,
        data,
        result,
    )
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_node_version(
    _env: NapiEnvHandle,
    result: *mut *const c_void,
) -> NapiStatus {
    if !result.is_null() {
        *result = std::ptr::null();
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_adjust_external_memory(
    _env: NapiEnvHandle,
    _change: i64,
    result: *mut i64,
) -> NapiStatus {
    if !result.is_null() {
        *result = 0;
    }
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_set_instance_data(
    _env: NapiEnvHandle,
    _data: *mut c_void,
    _finalize_cb: Option<NapiFinalizer>,
    _finalize_hint: *mut c_void,
) -> NapiStatus {
    NAPI_OK
}

#[unsafe(no_mangle)]
unsafe extern "C" fn napi_get_instance_data(
    _env: NapiEnvHandle,
    result: *mut *mut c_void,
) -> NapiStatus {
    if !result.is_null() {
        *result = std::ptr::null_mut();
    }
    NAPI_OK
}

// ── V8 callback bridge ──────────────────────────────────────────────────────

fn napi_bridge_callback(
    scope: &mut PinScope,
    args: FunctionCallbackArguments,
    mut rv: ReturnValue,
) {
    napi_trace!("napi_bridge_callback");
    unsafe {
        let data = args.data().cast::<v8::External>().value();
        let bridge = &*(data as *const NapiBridge);
        let env_ref = &mut *bridge.env;
        let argc = args.length() as usize;

        // Mark the active callback scope so napi_scope! inside this invocation
        // reuses it instead of nesting a fresh HandleScope (which V8 forbids).
        let scope_ptr = scope as *mut _ as *const () as *const PinScope<'_, '_>;
        NAPI_CB_SCOPE.with(|s| s.set(scope_ptr as *const ()));

        let ctx = v8::Local::new(scope, &env_ref.context);
        let cs = v8::ContextScope::new(scope, ctx);

        let mut argv_handles: Vec<NapiValue> = Vec::with_capacity(argc);
        for i in 0..argc {
            let arg: v8::Local<v8::Value> = args.get(i as i32);
            let global = v8::Global::new(&cs, arg);
            argv_handles.push(store_value(env_ref, global));
        }

        let this: v8::Local<v8::Value> = args.this().into();
        let this_global = v8::Global::new(&cs, this);
        let this_handle = store_value(env_ref, this_global);

        let ci = Box::new(NapiCallbackInfoInner {
            argc,
            argv: argv_handles,
            this_arg: this_handle,
            new_target: std::ptr::null_mut(),
            data: bridge.data,
        });
        let ci_ptr = Box::into_raw(ci);
        let result = (bridge.cb)(bridge.env, ci_ptr);
        if !result.is_null()
            && let Some(local) = get_local(&cs, result)
        {
            rv.set(local);
        }
        let _ = Box::from_raw(ci_ptr);

        NAPI_CB_SCOPE.with(|s| s.set(std::ptr::null()));
    }
}

// ── Module loading ──────────────────────────────────────────────────────────

fn napi_load_module(scope: &mut PinScope, path: &str) -> Result<v8::Global<v8::Value>, String> {
    let perms = NAPI_PERMISSIONS
        .with(|p| p.take())
        .ok_or("permissions not set")?;
    if !perms.check(&Capability::FFI(std::path::PathBuf::from(path))) {
        NAPI_PERMISSIONS.with(|p| p.set(Some(perms)));
        return Err(format!("FFI access denied. Run with --allow-ffi={}", path));
    }

    let lib = unsafe { Library::new(path).map_err(|e| format!("Failed to load {}: {}", path, e))? };
    let lib = Arc::new(lib);

    let isolate: &mut v8::Isolate = scope;
    // `Isolate` is `#[repr(transparent)]` over `NonNull<RealIsolate>`; the
    // reborrowed address is the scope's internal field, so read the real
    // isolate pointer it wraps instead of storing that (dangling) field addr.
    let isolate_ptr: *mut v8::RealIsolate =
        unsafe { *(isolate as *const v8::Isolate as *const *mut v8::RealIsolate) };

    let ctx = scope.get_current_context();
    let context_global = v8::Global::new(scope, ctx);

    let env = Box::new(NapiEnv {
        isolate: isolate_ptr,
        context: context_global,
        values: Vec::new(),
        pending_exception: None,
        cleanup_hooks: Vec::new(),
        _library: Some(lib.clone()),
    });
    let env_ptr = Box::into_raw(env);

    unsafe {
        let cs = v8::ContextScope::new(scope, ctx);

        let exports = v8::Object::new(&cs);
        let exports_val: v8::Local<v8::Value> = exports.into();
        let exports_global = v8::Global::new(&cs, exports_val);
        let exports_handle = store_value(&mut *env_ptr, exports_global);

        let register_v1: Result<
            libloading::Symbol<unsafe extern "C" fn(NapiEnvHandle, NapiValue) -> NapiValue>,
            _,
        > = lib.get(b"napi_register_module_v1");

        let result = match register_v1 {
            Ok(func) => {
                let ret = func(env_ptr, exports_handle);
                if !ret.is_null() {
                    let g = &(*ret).global;
                    let local = v8::Local::new(&cs, g);
                    let val: v8::Local<v8::Value> = local;
                    v8::Global::new(&cs, val)
                } else {
                    let last = (*env_ptr).values.last().unwrap();
                    let local = v8::Local::new(&cs, &(**last).global);
                    let val: v8::Local<v8::Value> = local;
                    v8::Global::new(&cs, val)
                }
            }
            Err(_) => {
                let stored = (*std::ptr::addr_of_mut!(NAPI_REGISTERED_MODULE)).take();
                if let Some(module) = stored {
                    if let Some(reg_fn) = module.nm_register_func {
                        let ret = reg_fn(env_ptr, exports_handle);
                        if !ret.is_null() {
                            let g = &(*ret).global;
                            let local = v8::Local::new(&cs, g);
                            let val: v8::Local<v8::Value> = local;
                            v8::Global::new(&cs, val)
                        } else {
                            let last = (*env_ptr).values.last().unwrap();
                            let local = v8::Local::new(&cs, &(**last).global);
                            let val: v8::Local<v8::Value> = local;
                            v8::Global::new(&cs, val)
                        }
                    } else {
                        let last = *(*env_ptr).values.last().unwrap();
                        let local = v8::Local::new(&cs, &(*last).global);
                        let val: v8::Local<v8::Value> = local;
                        v8::Global::new(&cs, val)
                    }
                } else {
                    let last = *(*env_ptr).values.last().unwrap();
                    let local = v8::Local::new(&cs, &(*last).global);
                    let val: v8::Local<v8::Value> = local;
                    v8::Global::new(&cs, val)
                }
            }
        };

        NAPI_PERMISSIONS.with(|p| p.set(Some(perms)));
        Ok(result)
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Run native async-work/threadsafe completions on the main V8 thread.
/// Called from `run_event_loop`; each callback builds its own scope via
/// `napi_scope!`, so no caller scope is required.
pub fn drain_async_completions() {
    loop {
        let next = NAPI_ASYNC_COMPLETIONS.lock().unwrap().pop_front();
        match next {
            Some(closure) => closure(),
            None => break,
        }
    }
}

pub fn inject_napi(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    permissions: Arc<PermissionState>,
) -> anyhow::Result<()> {
    NAPI_PERMISSIONS.with(|p| p.set(Some(permissions)));

    let context = scope.get_current_context();
    let global = context.global(scope);

    let napi_load_fn = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path = args.get(0).to_rust_string_lossy(scope);
            match napi_load_module(scope, &path) {
                Ok(global_val) => {
                    let local = v8::Local::new(scope, &global_val);
                    rv.set(local);
                }
                Err(e) => {
                    let msg = v8::String::new(scope, &e).unwrap();
                    scope.throw_exception(v8::Exception::error(scope, msg));
                }
            }
        },
    )
    .build(scope)
    .unwrap();

    global.set(
        scope,
        v8::String::new(scope, "__napiLoad").unwrap().into(),
        napi_load_fn.into(),
    );

    Ok(())
}
