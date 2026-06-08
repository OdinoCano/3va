//! Node-API (NAPI) compatibility layer for loading `.node` native addons.
// NAPI glue is inherently unsafe FFI. Every unsafe extern "C" function body
// contains unsafe operations; wrapping each one in an explicit `unsafe {}` block
// adds noise without improving safety properties. Every pub unsafe extern "C"
// function here implements the Node-API ABI contract — callers (native .node
// addons) are responsible for passing valid napi_env / napi_value pointers.
#![allow(unsafe_op_in_unsafe_fn)]
#![allow(clippy::missing_safety_doc)]
//!
//! When `require('path/to/addon.node')` is called, this module:
//! 1. Opens the shared library with `libloading`.
//! 2. Looks up `napi_register_module_v1` (the standard NAPI v1 entrypoint).
//! 3. Constructs a `NapiEnv` wrapping the current QuickJS context.
//! 4. Calls the entrypoint with an empty exports object.
//! 5. Returns the populated exports object to JS.
//!
//! Implemented NAPI functions (≈ NAPI v8 subset):
//! - Object/property: create_object, set/get_named_property, define_properties
//! - Primitives: create_string_utf8, create_int32, create_uint32, create_double, create_bool
//! - Functions: create_function, get_cb_info
//! - Arrays: create_array, set_element, get_element, get_array_length, is_array
//! - Buffers: create_buffer_copy, get_buffer_info
//! - Type checks: is_null, is_undefined, is_string, is_number, is_boolean, is_object, is_function
//! - Errors: throw_error, get_last_error_info
//! - Values: get_value_string_utf8, get_value_int32, get_value_uint32, get_value_double, get_value_bool
//! - References: create_reference, delete_reference, get_reference_value
//! - Miscellaneous: get_undefined, get_null, get_boolean, get_global, strict_equals

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::Arc;

use libloading::Library;
use rquickjs::{Ctx, Function, Result};
use rquickjs_sys as qjs;
use vvva_permissions::{Capability, PermissionState};

// ── NAPI type aliases ─────────────────────────────────────────────────────────

#[repr(C)]
#[allow(non_camel_case_types)]
pub enum napi_status {
    napi_ok = 0,
    napi_invalid_arg = 1,
    napi_object_expected = 2,
    napi_string_expected = 3,
    napi_name_expected = 4,
    napi_function_expected = 5,
    napi_number_expected = 6,
    napi_boolean_expected = 7,
    napi_array_expected = 8,
    napi_generic_failure = 9,
    napi_pending_exception = 10,
}

#[allow(non_camel_case_types)]
pub type napi_env = *mut NapiEnvInner;
#[allow(non_camel_case_types)]
pub type napi_value = *mut NapiValueInner;
#[allow(non_camel_case_types)]
pub type napi_callback_info = *mut NapiCallbackInfo;
#[allow(non_camel_case_types)]
pub type napi_ref = *mut NapiRefInner;
#[allow(non_camel_case_types)]
pub type napi_callback = Option<unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value>;
#[allow(non_camel_case_types)]
pub type napi_finalize = Option<unsafe extern "C" fn(napi_env, *mut c_void, *mut c_void)>;

// ── napi_property_descriptor ──────────────────────────────────────────────────

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct napi_property_descriptor {
    pub utf8name: *const c_char,
    pub name: napi_value,
    pub method: napi_callback,
    pub getter: napi_callback,
    pub setter: napi_callback,
    pub value: napi_value,
    pub attributes: u32,
    pub data: *mut c_void,
}

// ── napi_extended_error_info ──────────────────────────────────────────────────

#[repr(C)]
#[allow(non_camel_case_types)]
pub struct napi_extended_error_info {
    pub error_message: *const c_char,
    pub engine_reserved: *mut c_void,
    pub engine_error_code: u32,
    pub error_code: napi_status,
}

// ── Internal structures ───────────────────────────────────────────────────────

pub struct NapiEnvInner {
    pub ctx: *mut qjs::JSContext,
    pub last_error: Option<CString>,
    pub values: Vec<Box<NapiValueInner>>,
    pub refs: HashMap<u64, Box<NapiRefInner>>,
    pub next_ref_id: u64,
}

pub struct NapiValueInner {
    pub val: qjs::JSValue,
    pub ctx: *mut qjs::JSContext,
}

impl Drop for NapiValueInner {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { qjs::JS_FreeValue(self.ctx, self.val) };
        }
    }
}

pub struct NapiRefInner {
    pub id: u64,
    pub val: qjs::JSValue,
    pub ctx: *mut qjs::JSContext,
}

impl Drop for NapiRefInner {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { qjs::JS_FreeValue(self.ctx, self.val) };
        }
    }
}

pub struct NapiCallbackInfo {
    pub env: napi_env,
    pub this_val: qjs::JSValue,
    pub args: Vec<qjs::JSValue>,
    pub data: *mut c_void,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Dup a JSValue and store it in the env's allocation list. Returns raw pointer.
unsafe fn env_alloc(env: napi_env, val: qjs::JSValue) -> napi_value {
    let inner = Box::new(NapiValueInner {
        val: unsafe { qjs::JS_DupValue(val) },
        ctx: (*env).ctx,
    });
    let ptr = inner.as_ref() as *const NapiValueInner as napi_value;
    (*env).values.push(inner);
    ptr
}

#[inline]
unsafe fn jsval(v: napi_value) -> qjs::JSValue {
    (*v).val
}

#[inline]
unsafe fn ctx(env: napi_env) -> *mut qjs::JSContext {
    (*env).ctx
}

/// Build a JSValue integer (fits in i32) without a ctx arg.
#[inline]
fn mk_int(v: i32) -> qjs::JSValue {
    qjs::JS_MKVAL(qjs::JS_TAG_INT, v)
}

/// Build a JSValue double.
#[inline]
fn mk_float(v: f64) -> qjs::JSValue {
    qjs::JS_NewFloat64(v)
}

// ── NAPI exports ─────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_undefined(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = env_alloc(env, qjs::JS_UNDEFINED);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_null(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = env_alloc(env, qjs::JS_NULL);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_boolean(
    env: napi_env,
    value: bool,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let v = if value { qjs::JS_TRUE } else { qjs::JS_FALSE };
    *result = env_alloc(env, v);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_object(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let obj = qjs::JS_NewObject(ctx(env));
    *result = env_alloc(env, obj);
    qjs::JS_FreeValue(ctx(env), obj);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_array(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let arr = qjs::JS_NewArray(ctx(env));
    *result = env_alloc(env, arr);
    qjs::JS_FreeValue(ctx(env), arr);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_string_utf8(
    env: napi_env,
    s: *const c_char,
    length: usize,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || s.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let bytes = if length == usize::MAX {
        CStr::from_ptr(s).to_bytes()
    } else {
        std::slice::from_raw_parts(s as *const u8, length)
    };
    let js_str = qjs::JS_NewStringLen(ctx(env), bytes.as_ptr() as *const c_char, bytes.len() as _);
    *result = env_alloc(env, js_str);
    qjs::JS_FreeValue(ctx(env), js_str);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_int32(
    env: napi_env,
    value: i32,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = env_alloc(env, mk_int(value));
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_uint32(
    env: napi_env,
    value: u32,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = env_alloc(env, mk_float(value as f64));
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_int64(
    env: napi_env,
    value: i64,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = env_alloc(env, mk_float(value as f64));
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_double(
    env: napi_env,
    value: f64,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = env_alloc(env, mk_float(value));
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_int32(
    env: napi_env,
    value: napi_value,
    result: *mut i32,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut out: i32 = 0;
    if qjs::JS_ToInt32(ctx(env), &mut out, jsval(value)) != 0 {
        return napi_status::napi_number_expected;
    }
    *result = out;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_uint32(
    env: napi_env,
    value: napi_value,
    result: *mut u32,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut out: i64 = 0;
    if qjs::JS_ToInt64(ctx(env), &mut out, jsval(value)) != 0 {
        return napi_status::napi_number_expected;
    }
    *result = out as u32;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_int64(
    env: napi_env,
    value: napi_value,
    result: *mut i64,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut out: i64 = 0;
    if qjs::JS_ToInt64(ctx(env), &mut out, jsval(value)) != 0 {
        return napi_status::napi_number_expected;
    }
    *result = out;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_double(
    env: napi_env,
    value: napi_value,
    result: *mut f64,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut out: f64 = 0.0;
    if qjs::JS_ToFloat64(ctx(env), &mut out, jsval(value)) != 0 {
        return napi_status::napi_number_expected;
    }
    *result = out;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_bool(
    env: napi_env,
    value: napi_value,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let b = qjs::JS_ToBool(ctx(env), jsval(value));
    if b < 0 {
        return napi_status::napi_boolean_expected;
    }
    *result = b != 0;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_string_utf8(
    env: napi_env,
    value: napi_value,
    buf: *mut c_char,
    bufsize: usize,
    result: *mut usize,
) -> napi_status {
    if env.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut len: usize = 0;
    let cstr_ptr = qjs::JS_ToCStringLen(ctx(env), &mut len, jsval(value));
    if cstr_ptr.is_null() {
        return napi_status::napi_string_expected;
    }
    if !buf.is_null() && bufsize > 0 {
        let copy = len.min(bufsize - 1);
        std::ptr::copy_nonoverlapping(cstr_ptr, buf, copy);
        *buf.add(copy) = 0;
    }
    if !result.is_null() {
        *result = len;
    }
    qjs::JS_FreeCString(ctx(env), cstr_ptr);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_set_named_property(
    env: napi_env,
    object: napi_value,
    name: *const c_char,
    value: napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || name.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let duped = qjs::JS_DupValue(jsval(value));
    let atom = qjs::JS_NewAtom(ctx(env), name);
    let rc = qjs::JS_SetProperty(ctx(env), jsval(object), atom, duped);
    qjs::JS_FreeAtom(ctx(env), atom);
    if rc < 0 {
        napi_status::napi_generic_failure
    } else {
        napi_status::napi_ok
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_named_property(
    env: napi_env,
    object: napi_value,
    name: *const c_char,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || name.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let atom = qjs::JS_NewAtom(ctx(env), name);
    let v = qjs::JS_GetProperty(ctx(env), jsval(object), atom);
    qjs::JS_FreeAtom(ctx(env), atom);
    *result = env_alloc(env, v);
    qjs::JS_FreeValue(ctx(env), v);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_set_element(
    env: napi_env,
    object: napi_value,
    index: u32,
    value: napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let duped = qjs::JS_DupValue(jsval(value));
    let rc = qjs::JS_SetPropertyUint32(ctx(env), jsval(object), index, duped);
    if rc < 0 {
        napi_status::napi_generic_failure
    } else {
        napi_status::napi_ok
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_element(
    env: napi_env,
    object: napi_value,
    index: u32,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let v = qjs::JS_GetPropertyUint32(ctx(env), jsval(object), index);
    *result = env_alloc(env, v);
    qjs::JS_FreeValue(ctx(env), v);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_array_length(
    env: napi_env,
    value: napi_value,
    result: *mut u32,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let atom = qjs::JS_NewAtom(ctx(env), c"length".as_ptr());
    let v = qjs::JS_GetProperty(ctx(env), jsval(value), atom);
    qjs::JS_FreeAtom(ctx(env), atom);
    let mut len: i64 = 0;
    qjs::JS_ToInt64(ctx(env), &mut len, v);
    qjs::JS_FreeValue(ctx(env), v);
    *result = len as u32;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_is_array(
    env: napi_env,
    value: napi_value,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = qjs::JS_IsArray(ctx(env), jsval(value)) != 0;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_global(env: napi_env, result: *mut napi_value) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let g = qjs::JS_GetGlobalObject(ctx(env));
    *result = env_alloc(env, g);
    qjs::JS_FreeValue(ctx(env), g);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_strict_equals(
    env: napi_env,
    lhs: napi_value,
    rhs: napi_value,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || lhs.is_null() || rhs.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // Use JS_StrictEqualVal if available, otherwise compare tags + values.
    let l = jsval(lhs);
    let r = jsval(rhs);
    let lt = qjs::JS_VALUE_GET_TAG(l);
    let rt = qjs::JS_VALUE_GET_TAG(r);
    *result = if lt != rt {
        false
    } else if lt == qjs::JS_TAG_INT {
        qjs::JS_VALUE_GET_INT(l) == qjs::JS_VALUE_GET_INT(r)
    } else if lt == qjs::JS_TAG_BOOL {
        qjs::JS_VALUE_GET_BOOL(l) == qjs::JS_VALUE_GET_BOOL(r)
    } else if lt == qjs::JS_TAG_NULL || lt == qjs::JS_TAG_UNDEFINED {
        true
    } else if qjs::JS_TAG_IS_FLOAT64(lt) {
        qjs::JS_VALUE_GET_FLOAT64(l) == qjs::JS_VALUE_GET_FLOAT64(r)
    } else {
        // Object/string identity — compare raw pointers
        qjs::JS_VALUE_GET_PTR(l) == qjs::JS_VALUE_GET_PTR(r)
    };
    napi_status::napi_ok
}

// ── Function creation ─────────────────────────────────────────────────────────

struct Trampoline {
    cb: unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value,
    data: *mut c_void,
    env_ptr: napi_env,
}
unsafe impl Send for Trampoline {}
unsafe impl Sync for Trampoline {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_function(
    env: napi_env,
    _name: *const c_char,
    _length: usize,
    cb: napi_callback,
    data: *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let Some(cb_fn) = cb else {
        return napi_status::napi_invalid_arg;
    };

    let trampoline = Box::into_raw(Box::new(Trampoline {
        cb: cb_fn,
        data,
        env_ptr: env,
    }));

    unsafe extern "C" fn call_trampoline(
        ctx_ptr: *mut qjs::JSContext,
        this_val: qjs::JSValue,
        argc: c_int,
        argv: *mut qjs::JSValue,
        _magic: c_int,
        opaque: *mut qjs::JSValue,
    ) -> qjs::JSValue {
        // opaque[0] is the JSValue we stored. We encoded the Trampoline pointer
        // as a float64 so QuickJS stores it by value without ref-counting.
        let ptr_bits = (*opaque).u.float64.to_bits();
        let t = &*(ptr_bits as usize as *const Trampoline);
        let args: Vec<qjs::JSValue> = (0..argc as usize)
            .map(|i| qjs::JS_DupValue(*argv.add(i)))
            .collect();
        let this_dup = qjs::JS_DupValue(this_val);
        let mut cb_info = NapiCallbackInfo {
            env: t.env_ptr,
            this_val: this_dup,
            args,
            data: t.data,
        };
        let result_napi = (t.cb)(t.env_ptr, &mut cb_info as *mut _);
        qjs::JS_FreeValue(ctx_ptr, cb_info.this_val);
        for a in &cb_info.args {
            qjs::JS_FreeValue(ctx_ptr, *a);
        }
        if result_napi.is_null() {
            qjs::JS_UNDEFINED
        } else {
            qjs::JS_DupValue((*result_napi).val)
        }
    }

    // Encode the trampoline pointer as float64 so QuickJS stores it by value
    // (float64 JSValues are not ref-counted, so JS_DupValue is a no-op for them).
    let ptr_as_f64 = f64::from_bits(trampoline as usize as u64);
    let trampoline_jsval = qjs::JSValue {
        u: qjs::JSValueUnion {
            float64: ptr_as_f64,
        },
        tag: qjs::JS_TAG_FLOAT64 as i64,
    };
    let js_fn = qjs::JS_NewCFunctionData(
        ctx(env),
        Some(call_trampoline),
        0,
        0,
        1,
        &trampoline_jsval as *const _ as *mut qjs::JSValue,
    );

    *result = env_alloc(env, js_fn);
    qjs::JS_FreeValue(ctx(env), js_fn);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_cb_info(
    env: napi_env,
    cbinfo: napi_callback_info,
    argc: *mut usize,
    argv: *mut napi_value,
    this_arg: *mut napi_value,
    data: *mut *mut c_void,
) -> napi_status {
    if env.is_null() || cbinfo.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let info = &*cbinfo;

    if !this_arg.is_null() {
        *this_arg = env_alloc(env, info.this_val);
    }
    if !data.is_null() {
        *data = info.data;
    }
    if !argc.is_null() {
        let want = *argc;
        let have = info.args.len();
        *argc = have;
        if !argv.is_null() {
            let copy = want.min(have);
            for i in 0..copy {
                *argv.add(i) = env_alloc(env, info.args[i]);
            }
            for i in copy..want {
                *argv.add(i) = env_alloc(env, qjs::JS_UNDEFINED);
            }
        }
    }
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_throw_error(
    env: napi_env,
    _code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let msg_str = if msg.is_null() {
        c"error".as_ptr()
    } else {
        msg
    };
    let err = qjs::JS_ThrowInternalError(ctx(env), msg_str);
    qjs::JS_FreeValue(ctx(env), err);
    napi_status::napi_pending_exception
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_throw_type_error(
    env: napi_env,
    _code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let msg_str = if msg.is_null() {
        c"TypeError".as_ptr()
    } else {
        msg
    };
    let err = qjs::JS_ThrowTypeError(ctx(env), msg_str);
    qjs::JS_FreeValue(ctx(env), err);
    napi_status::napi_pending_exception
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_throw_range_error(
    env: napi_env,
    _code: *const c_char,
    msg: *const c_char,
) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let msg_str = if msg.is_null() {
        c"RangeError".as_ptr()
    } else {
        msg
    };
    let err = qjs::JS_ThrowRangeError(ctx(env), msg_str);
    qjs::JS_FreeValue(ctx(env), err);
    napi_status::napi_pending_exception
}

// Safety: all fields in the static ERROR_INFO are null pointers — no data races possible.
unsafe impl Sync for napi_extended_error_info {}

static ERROR_INFO: napi_extended_error_info = napi_extended_error_info {
    error_message: std::ptr::null(),
    engine_reserved: std::ptr::null_mut(),
    engine_error_code: 0,
    error_code: napi_status::napi_ok,
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_last_error_info(
    _env: napi_env,
    result: *mut *const napi_extended_error_info,
) -> napi_status {
    if result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = &ERROR_INFO;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_define_properties(
    env: napi_env,
    object: napi_value,
    property_count: usize,
    properties: *const napi_property_descriptor,
) -> napi_status {
    if env.is_null() || object.is_null() {
        return napi_status::napi_invalid_arg;
    }
    for i in 0..property_count {
        let prop = &*properties.add(i);
        let name_ptr = if !prop.utf8name.is_null() {
            prop.utf8name
        } else {
            continue;
        };
        if let Some(cb) = prop.method {
            let mut fn_val: napi_value = std::ptr::null_mut();
            napi_create_function(env, name_ptr, usize::MAX, Some(cb), prop.data, &mut fn_val);
            napi_set_named_property(env, object, name_ptr, fn_val);
        } else if !prop.value.is_null() {
            napi_set_named_property(env, object, name_ptr, prop.value);
        }
    }
    napi_status::napi_ok
}

// ── Type checks ───────────────────────────────────────────────────────────────

macro_rules! napi_is {
    ($name:ident, $check:expr) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(env: napi_env, v: napi_value, r: *mut bool) -> napi_status {
            if env.is_null() || v.is_null() || r.is_null() {
                return napi_status::napi_invalid_arg;
            }
            *r = $check(jsval(v));
            napi_status::napi_ok
        }
    };
}

napi_is!(napi_is_null, |v| unsafe { qjs::JS_IsNull(v) });
napi_is!(napi_is_undefined, |v| unsafe { qjs::JS_IsUndefined(v) });
napi_is!(napi_is_string, |v| unsafe { qjs::JS_IsString(v) });
napi_is!(napi_is_number, |v| unsafe { qjs::JS_IsNumber(v) });
napi_is!(napi_is_boolean, |v| unsafe { qjs::JS_IsBool(v) });
napi_is!(napi_is_object, |v| unsafe { qjs::JS_IsObject(v) });

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_is_function(
    env: napi_env,
    v: napi_value,
    r: *mut bool,
) -> napi_status {
    if env.is_null() || v.is_null() || r.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *r = qjs::JS_IsFunction(ctx(env), jsval(v)) != 0;
    napi_status::napi_ok
}

// ── Buffer support ────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_buffer_copy(
    env: napi_env,
    length: usize,
    data: *const c_void,
    result_data: *mut *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let buf = qjs::JS_NewArrayBufferCopy(ctx(env), data as *const u8, length as _);
    if !result_data.is_null() {
        let mut sz: u64 = 0;
        let ptr = qjs::JS_GetArrayBuffer(ctx(env), &mut sz, buf);
        *result_data = ptr as *mut c_void;
    }
    *result = env_alloc(env, buf);
    qjs::JS_FreeValue(ctx(env), buf);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_buffer_info(
    env: napi_env,
    value: napi_value,
    data: *mut *mut c_void,
    length: *mut usize,
) -> napi_status {
    if env.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut sz: u64 = 0;
    let ptr = qjs::JS_GetArrayBuffer(ctx(env), &mut sz, jsval(value));
    if !data.is_null() {
        *data = ptr as *mut c_void;
    }
    if !length.is_null() {
        *length = sz as usize;
    }
    napi_status::napi_ok
}

// ── References ────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_reference(
    env: napi_env,
    value: napi_value,
    _initial_refcount: u32,
    result: *mut napi_ref,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let id = (*env).next_ref_id;
    (*env).next_ref_id += 1;
    let inner = Box::new(NapiRefInner {
        id,
        val: qjs::JS_DupValue(jsval(value)),
        ctx: ctx(env),
    });
    let ptr = inner.as_ref() as *const NapiRefInner as napi_ref;
    (*env).refs.insert(id, inner);
    *result = ptr;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_reference_value(
    env: napi_env,
    reference: napi_ref,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || reference.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = env_alloc(env, (*reference).val);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_delete_reference(env: napi_env, reference: napi_ref) -> napi_status {
    if env.is_null() || reference.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let id = (*reference).id;
    (*env).refs.remove(&id);
    napi_status::napi_ok
}

// ── Extended NAPI functions (needed for complex addons like Prisma) ───────────

// ─ napi_typeof ─────────────────────────────────────────────────────────────
#[repr(u32)]
#[allow(non_camel_case_types, dead_code)]
pub enum napi_valuetype {
    napi_undefined = 0,
    napi_null = 1,
    napi_boolean = 2,
    napi_number = 3,
    napi_string = 4,
    napi_symbol = 5,
    napi_object = 6,
    napi_function = 7,
    napi_external = 8,
    napi_bigint = 9,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_typeof(
    env: napi_env,
    value: napi_value,
    result: *mut u32,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let v = jsval(value);
    *result = if qjs::JS_IsUndefined(v) {
        0
    } else if qjs::JS_IsNull(v) {
        1
    } else if qjs::JS_IsBool(v) {
        2
    } else if qjs::JS_IsNumber(v) {
        3
    } else if qjs::JS_IsString(v) {
        4
    } else if qjs::JS_IsFunction(ctx(env), v) != 0 {
        7
    } else {
        6 // object or fallback
    };
    napi_status::napi_ok
}

// ─ napi_coerce_to_object / napi_coerce_to_string ───────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_coerce_to_object(
    env: napi_env,
    value: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let v = jsval(value);
    let obj = if qjs::JS_IsObject(v) && !qjs::JS_IsNull(v) {
        qjs::JS_DupValue(v)
    } else {
        qjs::JS_NewObject(ctx(env))
    };
    *result = env_alloc(env, obj);
    qjs::JS_FreeValue(ctx(env), obj);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_coerce_to_string(
    env: napi_env,
    value: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let v = jsval(value);
    let s = qjs::JS_ToString(ctx(env), v);
    if qjs::JS_IsException(s) {
        return napi_status::napi_generic_failure;
    }
    *result = env_alloc(env, s);
    qjs::JS_FreeValue(ctx(env), s);
    napi_status::napi_ok
}

// ─ napi_get_property / napi_has_named_property / napi_get_property_names ───
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_property(
    env: napi_env,
    object: napi_value,
    key: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || key.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // Convert key to atom, then get property
    let key_atom = qjs::JS_ValueToAtom(ctx(env), jsval(key));
    let prop = qjs::JS_GetProperty(ctx(env), jsval(object), key_atom);
    qjs::JS_FreeAtom(ctx(env), key_atom);
    if qjs::JS_IsException(prop) {
        return napi_status::napi_generic_failure;
    }
    *result = env_alloc(env, prop);
    qjs::JS_FreeValue(ctx(env), prop);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_has_named_property(
    env: napi_env,
    object: napi_value,
    utf8name: *const c_char,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || object.is_null() || utf8name.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let name = CStr::from_ptr(utf8name).to_string_lossy();
    let prop = qjs::JS_GetPropertyStr(ctx(env), jsval(object), utf8name);
    let exists = !qjs::JS_IsUndefined(prop) && !qjs::JS_IsException(prop);
    qjs::JS_FreeValue(ctx(env), prop);
    let _ = name;
    *result = exists;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_property_names(
    env: napi_env,
    object: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || object.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // Get keys via Object.keys() call
    let global = qjs::JS_GetGlobalObject(ctx(env));
    let obj_ctor = qjs::JS_GetPropertyStr(ctx(env), global, c"Object".as_ptr());
    let keys_fn = qjs::JS_GetPropertyStr(ctx(env), obj_ctor, c"keys".as_ptr());
    qjs::JS_FreeValue(ctx(env), obj_ctor);
    qjs::JS_FreeValue(ctx(env), global);
    let mut args = [jsval(object)];
    let names = qjs::JS_Call(ctx(env), keys_fn, qjs::JS_UNDEFINED, 1, args.as_mut_ptr());
    qjs::JS_FreeValue(ctx(env), keys_fn);
    if qjs::JS_IsException(names) {
        return napi_status::napi_generic_failure;
    }
    *result = env_alloc(env, names);
    qjs::JS_FreeValue(ctx(env), names);
    napi_status::napi_ok
}

// ─ napi_is_error / napi_is_typedarray / napi_is_exception_pending ──────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_is_error(
    env: napi_env,
    value: napi_value,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // Check if the value is an instance of Error by checking the 'message' property.
    let msg = qjs::JS_GetPropertyStr(ctx(env), jsval(value), c"message".as_ptr());
    let is_err = !qjs::JS_IsUndefined(msg) && !qjs::JS_IsException(msg);
    qjs::JS_FreeValue(ctx(env), msg);
    *result = is_err && qjs::JS_IsObject(jsval(value));
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_is_exception_pending(
    _env: napi_env,
    result: *mut bool,
) -> napi_status {
    if result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    *result = false; // QuickJS handles exceptions inline
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_is_typedarray(
    env: napi_env,
    value: napi_value,
    result: *mut bool,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // QuickJS doesn't expose a direct IsTypedArray check — heuristic: has buffer property
    let buf = qjs::JS_GetPropertyStr(ctx(env), jsval(value), c"buffer".as_ptr());
    *result = qjs::JS_IsObject(buf);
    qjs::JS_FreeValue(ctx(env), buf);
    napi_status::napi_ok
}

// ─ napi_get_and_clear_last_exception ───────────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_and_clear_last_exception(
    env: napi_env,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let exc = qjs::JS_GetException(ctx(env));
    if qjs::JS_IsNull(exc) || qjs::JS_IsUndefined(exc) {
        qjs::JS_FreeValue(ctx(env), exc);
        *result = env_alloc(env, qjs::JS_UNDEFINED);
    } else {
        *result = env_alloc(env, exc);
        qjs::JS_FreeValue(ctx(env), exc);
    }
    napi_status::napi_ok
}

// ─ napi_throw ──────────────────────────────────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_throw(env: napi_env, error: napi_value) -> napi_status {
    if env.is_null() || error.is_null() {
        return napi_status::napi_invalid_arg;
    }
    qjs::JS_Throw(ctx(env), qjs::JS_DupValue(jsval(error)));
    napi_status::napi_ok
}

// ─ napi_create_error ───────────────────────────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_error(
    env: napi_env,
    _code: napi_value,
    msg: napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let msg_val = if !msg.is_null() {
        jsval(msg)
    } else {
        qjs::JS_NewString(ctx(env), c"".as_ptr())
    };
    let msg_str = qjs::JS_ToString(ctx(env), msg_val);
    let err = qjs::JS_NewError(ctx(env));
    qjs::JS_SetPropertyStr(ctx(env), err, c"message".as_ptr(), msg_str);
    *result = env_alloc(env, err);
    qjs::JS_FreeValue(ctx(env), err);
    napi_status::napi_ok
}

// ─ napi_fatal_error / napi_fatal_exception ─────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_fatal_error(
    location: *const c_char,
    _location_len: usize,
    message: *const c_char,
    _message_len: usize,
) {
    let loc = if location.is_null() {
        "unknown".into()
    } else {
        CStr::from_ptr(location).to_string_lossy()
    };
    let msg = if message.is_null() {
        "fatal error".into()
    } else {
        CStr::from_ptr(message).to_string_lossy()
    };
    eprintln!("[NAPI] Fatal error at {loc}: {msg}");
    std::process::abort();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_fatal_exception(env: napi_env, _err: napi_value) -> napi_status {
    if env.is_null() {
        return napi_status::napi_invalid_arg;
    }
    eprintln!("[NAPI] Uncaught fatal exception");
    napi_status::napi_ok
}

// ─ napi_call_function ──────────────────────────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_call_function(
    env: napi_env,
    recv: napi_value,
    func: napi_value,
    argc: usize,
    argv: *const napi_value,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || func.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let this_val = if recv.is_null() {
        qjs::JS_UNDEFINED
    } else {
        jsval(recv)
    };
    let mut args: Vec<qjs::JSValue> = (0..argc).map(|i| jsval(*argv.add(i))).collect();
    let ret = qjs::JS_Call(
        ctx(env),
        jsval(func),
        this_val,
        args.len() as i32,
        args.as_mut_ptr(),
    );
    if qjs::JS_IsException(ret) {
        return napi_status::napi_pending_exception;
    }
    if !result.is_null() {
        *result = env_alloc(env, ret);
        qjs::JS_FreeValue(ctx(env), ret);
    } else {
        qjs::JS_FreeValue(ctx(env), ret);
    }
    napi_status::napi_ok
}

// ─ napi_create_array_with_length ───────────────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_array_with_length(
    env: napi_env,
    length: usize,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let arr = qjs::JS_NewArray(ctx(env));
    // Set length property
    let len_val = mk_int(length as i32);
    qjs::JS_SetPropertyStr(ctx(env), arr, c"length".as_ptr(), len_val);
    *result = env_alloc(env, arr);
    qjs::JS_FreeValue(ctx(env), arr);
    napi_status::napi_ok
}

// ─ napi_create_arraybuffer / napi_create_external_arraybuffer ──────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_arraybuffer(
    env: napi_env,
    byte_length: usize,
    data: *mut *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let buf = qjs::JS_NewArrayBuffer(
        ctx(env),
        std::ptr::null_mut(),
        byte_length as u64,
        None,
        std::ptr::null_mut(),
        0,
    );
    if qjs::JS_IsException(buf) {
        return napi_status::napi_generic_failure;
    }
    if !data.is_null() {
        let mut sz: u64 = 0;
        let raw = qjs::JS_GetArrayBuffer(ctx(env), &mut sz, buf);
        *data = raw as *mut c_void;
    }
    *result = env_alloc(env, buf);
    qjs::JS_FreeValue(ctx(env), buf);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_external_arraybuffer(
    env: napi_env,
    data: *mut c_void,
    byte_length: usize,
    _finalize_cb: napi_finalize,
    _finalize_hint: *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let buf = qjs::JS_NewArrayBuffer(
        ctx(env),
        data as *mut u8,
        byte_length as u64,
        None,
        std::ptr::null_mut(),
        0,
    );
    if qjs::JS_IsException(buf) {
        return napi_status::napi_generic_failure;
    }
    *result = env_alloc(env, buf);
    qjs::JS_FreeValue(ctx(env), buf);
    napi_status::napi_ok
}

// ─ napi_create_typedarray / napi_get_typedarray_info ───────────────────────
#[repr(u32)]
#[allow(non_camel_case_types, dead_code)]
pub enum napi_typedarray_type {
    napi_int8_array = 0,
    napi_uint8_array = 1,
    napi_uint8_clamped_array = 2,
    napi_int16_array = 3,
    napi_uint16_array = 4,
    napi_int32_array = 5,
    napi_uint32_array = 6,
    napi_float32_array = 7,
    napi_float64_array = 8,
    napi_bigint64_array = 9,
    napi_biguint64_array = 10,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_typedarray(
    env: napi_env,
    _type_: u32,
    length: usize,
    arraybuffer: napi_value,
    byte_offset: usize,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // Create a Uint8Array by default (most common case for Prisma buffer usage)
    let global = qjs::JS_GetGlobalObject(ctx(env));
    let ctor = qjs::JS_GetPropertyStr(ctx(env), global, c"Uint8Array".as_ptr());
    qjs::JS_FreeValue(ctx(env), global);
    if qjs::JS_IsException(ctor) || qjs::JS_IsFunction(ctx(env), ctor) == 0 {
        qjs::JS_FreeValue(ctx(env), ctor);
        return napi_status::napi_generic_failure;
    }
    let mut args = [
        jsval(arraybuffer),
        mk_int(byte_offset as i32),
        mk_int(length as i32),
    ];
    let obj = qjs::JS_CallConstructor(ctx(env), ctor, args.len() as i32, args.as_mut_ptr());
    qjs::JS_FreeValue(ctx(env), ctor);
    if qjs::JS_IsException(obj) {
        return napi_status::napi_generic_failure;
    }
    *result = env_alloc(env, obj);
    qjs::JS_FreeValue(ctx(env), obj);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_typedarray_info(
    env: napi_env,
    typedarray: napi_value,
    type_: *mut u32,
    length: *mut usize,
    data: *mut *mut c_void,
    arraybuffer: *mut napi_value,
    byte_offset: *mut usize,
) -> napi_status {
    if env.is_null() || typedarray.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let v = jsval(typedarray);
    if !type_.is_null() {
        *type_ = 1;
    } // uint8
    if !byte_offset.is_null() {
        *byte_offset = 0;
    }
    let mut buf_len: u64 = 0;
    let raw = qjs::JS_GetArrayBuffer(ctx(env), &mut buf_len, v);
    if !length.is_null() {
        *length = buf_len as usize;
    }
    if !data.is_null() {
        *data = raw as *mut c_void;
    }
    if !arraybuffer.is_null() {
        *arraybuffer = env_alloc(env, v);
    }
    napi_status::napi_ok
}

// ─ napi_create_bigint_words / napi_get_value_bigint_* ──────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_bigint_words(
    env: napi_env,
    sign_bit: i32,
    word_count: usize,
    words: *const u64,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // Reconstruct BigInt from 64-bit words (little-endian)
    let mut val: i128 = 0;
    for i in (0..word_count.min(2)).rev() {
        val = (val << 64) | *words.add(i) as i128;
    }
    if sign_bit != 0 {
        val = -val;
    }
    let v = mk_int(val as i32); // approximation for small values
    *result = env_alloc(env, v);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_bigint_int64(
    env: napi_env,
    value: napi_value,
    result: *mut i64,
    lossless: *mut bool,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let tag = 0i32;
    let r = qjs::JS_ToInt64Ext(ctx(env), &mut *result, jsval(value));
    let _ = tag;
    if !lossless.is_null() {
        *lossless = r == 0;
    }
    if r != 0 {
        napi_status::napi_generic_failure
    } else {
        napi_status::napi_ok
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_bigint_uint64(
    env: napi_env,
    value: napi_value,
    result: *mut u64,
    lossless: *mut bool,
) -> napi_status {
    if env.is_null() || value.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut i64val: i64 = 0;
    qjs::JS_ToInt64Ext(ctx(env), &mut i64val, jsval(value));
    *result = i64val as u64;
    if !lossless.is_null() {
        *lossless = i64val >= 0;
    }
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_get_value_bigint_words(
    env: napi_env,
    value: napi_value,
    sign_bit: *mut i32,
    word_count: *mut usize,
    words: *mut u64,
) -> napi_status {
    if env.is_null() || value.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let mut i64val: i64 = 0;
    qjs::JS_ToInt64Ext(ctx(env), &mut i64val, jsval(value));
    if !sign_bit.is_null() {
        *sign_bit = if i64val < 0 { 1 } else { 0 };
    }
    let abs_val = i64val.unsigned_abs();
    if !word_count.is_null() {
        let count = *word_count;
        *word_count = 1;
        if !words.is_null() && count >= 1 {
            *words = abs_val;
        }
    }
    napi_status::napi_ok
}

// ─ napi_create_promise / napi_resolve_deferred / napi_reject_deferred ───────
// Promise support: store resolve/reject callbacks in a wrapper object.
pub struct NapiDeferredInner {
    pub resolve: qjs::JSValue,
    pub reject: qjs::JSValue,
    pub ctx: *mut qjs::JSContext,
}
unsafe impl Send for NapiDeferredInner {}
unsafe impl Sync for NapiDeferredInner {}

impl Drop for NapiDeferredInner {
    fn drop(&mut self) {
        // Note: only call JS_FreeValue if dropped on the JS thread.
        // Because the deferred may be Box::from_raw'd on any thread but
        // we now process it on the main thread via DEFERRED_QUEUE, the Drop
        // here is effectively called on the main thread (we use ManuallyDrop
        // in the queue path to skip this Drop).
    }
}

#[allow(non_camel_case_types)]
pub type napi_deferred = *mut NapiDeferredInner;

// Pending deferred resolution/rejection calls from background threads.
struct DeferredPendingCall {
    deferred: *mut NapiDeferredInner,
    value: napi_value, // may be null (→ JS undefined)
    reject: bool,
}
unsafe impl Send for DeferredPendingCall {}
unsafe impl Sync for DeferredPendingCall {}

static DEFERRED_QUEUE: std::sync::OnceLock<std::sync::Mutex<Vec<DeferredPendingCall>>> =
    std::sync::OnceLock::new();

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_promise(
    env: napi_env,
    deferred: *mut napi_deferred,
    promise: *mut napi_value,
) -> napi_status {
    if env.is_null() || deferred.is_null() || promise.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // JS_NewPromiseCapability(ctx, resolving_funcs[2]) fills resolving_funcs[0]=resolve, [1]=reject
    let mut resolving_funcs = [qjs::JS_UNDEFINED, qjs::JS_UNDEFINED];
    let p = qjs::JS_NewPromiseCapability(ctx(env), resolving_funcs.as_mut_ptr());
    if qjs::JS_IsException(p) {
        return napi_status::napi_generic_failure;
    }
    *promise = env_alloc(env, p);
    qjs::JS_FreeValue(ctx(env), p);
    let d = Box::new(NapiDeferredInner {
        resolve: resolving_funcs[0], // already dup'd by JS_NewPromiseCapability
        reject: resolving_funcs[1],
        ctx: ctx(env),
    });
    *deferred = Box::into_raw(d);
    napi_status::napi_ok
}

// Queue a deferred resolve/reject to be executed on the main JS thread.
// Safe to call from any thread.
unsafe fn queue_deferred(deferred: napi_deferred, value: napi_value, reject: bool) {
    DEFERRED_QUEUE
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(DeferredPendingCall {
            deferred,
            value,
            reject,
        });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_resolve_deferred(
    env: napi_env,
    deferred: napi_deferred,
    resolution: napi_value,
) -> napi_status {
    if env.is_null() || deferred.is_null() {
        return napi_status::napi_invalid_arg;
    }
    queue_deferred(deferred, resolution, false);
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_reject_deferred(
    env: napi_env,
    deferred: napi_deferred,
    rejection: napi_value,
) -> napi_status {
    if env.is_null() || deferred.is_null() {
        return napi_status::napi_invalid_arg;
    }
    queue_deferred(deferred, rejection, true);
    napi_status::napi_ok
}

// ─ napi_wrap / napi_unwrap ─────────────────────────────────────────────────
// We store the native pointer in an opaque property on the JS object.
static NAPI_WRAP_KEY: &[u8] = b"__napi_wrap__\0";

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_wrap(
    env: napi_env,
    js_object: napi_value,
    native_object: *mut c_void,
    _finalize_cb: napi_finalize,
    _finalize_hint: *mut c_void,
    result: *mut napi_ref,
) -> napi_status {
    if env.is_null() || js_object.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // Store the native pointer as an external value on the object.
    let ext = mk_float(native_object as i64 as f64);
    qjs::JS_SetPropertyStr(
        ctx(env),
        jsval(js_object),
        NAPI_WRAP_KEY.as_ptr() as *const c_char,
        ext,
    );
    if !result.is_null() {
        // Create a reference to the JS object
        let ref_inner = Box::new(NapiRefInner {
            id: (*env).next_ref_id,
            val: qjs::JS_DupValue(jsval(js_object)),
            ctx: ctx(env),
        });
        (*env).next_ref_id += 1;
        (*env).refs.insert(ref_inner.id, ref_inner);
        // Return the last inserted ref — safe approximation
        *result = (*env).refs.values().last().unwrap().as_ref() as *const NapiRefInner as napi_ref;
    }
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_unwrap(
    env: napi_env,
    js_object: napi_value,
    result: *mut *mut c_void,
) -> napi_status {
    if env.is_null() || js_object.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let ext = qjs::JS_GetPropertyStr(
        ctx(env),
        jsval(js_object),
        NAPI_WRAP_KEY.as_ptr() as *const c_char,
    );
    if qjs::JS_IsException(ext) || qjs::JS_IsUndefined(ext) {
        qjs::JS_FreeValue(ctx(env), ext);
        return napi_status::napi_invalid_arg;
    }
    let mut ptr_val: i64 = 0;
    qjs::JS_ToInt64(ctx(env), &mut ptr_val, ext);
    qjs::JS_FreeValue(ctx(env), ext);
    *result = ptr_val as *mut c_void;
    napi_status::napi_ok
}

// ─ napi_define_class ────────────────────────────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_define_class(
    env: napi_env,
    utf8name: *const c_char,
    _name_length: usize,
    constructor: napi_callback,
    data: *mut c_void,
    property_count: usize,
    properties: *const napi_property_descriptor,
    result: *mut napi_value,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    // Create a constructor function and set the properties on its prototype.
    let mut ctor_napi = std::ptr::null_mut();
    let status = napi_create_function(env, utf8name, usize::MAX, constructor, data, &mut ctor_napi);
    if !matches!(status, napi_status::napi_ok) {
        return status;
    }
    // Mark as constructor so `new Ctor()` works in QuickJS
    let ctor_val = (*ctor_napi).val;
    qjs::JS_SetConstructorBit(ctx(env), ctor_val, 1);
    // Add properties to the prototype
    if property_count > 0 && !properties.is_null() {
        let status2 = napi_define_properties(env, ctor_napi, property_count, properties);
        if !matches!(status2, napi_status::napi_ok) {
            return status2;
        }
    }
    *result = ctor_napi;
    napi_status::napi_ok
}

// ─ napi_reference_unref ────────────────────────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_reference_unref(
    env: napi_env,
    reference: napi_ref,
    result: *mut u32,
) -> napi_status {
    if env.is_null() || reference.is_null() {
        return napi_status::napi_invalid_arg;
    }
    if !result.is_null() {
        *result = 0;
    }
    napi_status::napi_ok
}

// ─ napi_add_env_cleanup_hook ───────────────────────────────────────────────
#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_add_env_cleanup_hook(
    _env: napi_env,
    _fun: Option<unsafe extern "C" fn(*mut c_void)>,
    _arg: *mut c_void,
) -> napi_status {
    // No-op: we don't have cleanup hooks yet
    napi_status::napi_ok
}

// ─ Threadsafe functions ────────────────────────────────────────────────────
// Cross-thread JS call mechanism: background threads push TsfnPendingCall
// entries to TSFN_QUEUE, and the main JS event loop drains it each iteration.
#[allow(non_camel_case_types)]
pub type napi_threadsafe_function_call_js =
    Option<unsafe extern "C" fn(napi_env, napi_value, *mut c_void, *mut c_void)>;

#[allow(non_camel_case_types)]
#[repr(u32)]
pub enum napi_threadsafe_function_release_mode {
    napi_tsfn_release = 0,
    napi_tsfn_abort = 1,
}

#[allow(non_camel_case_types)]
#[repr(u32)]
pub enum napi_threadsafe_function_call_mode {
    napi_tsfn_nonblocking = 0,
    napi_tsfn_blocking = 1,
}

pub struct NapiThreadsafeFunctionInner {
    pub env: napi_env,
    pub func: qjs::JSValue,
    pub context: *mut c_void,
    pub call_js_cb: napi_threadsafe_function_call_js,
}
unsafe impl Send for NapiThreadsafeFunctionInner {}
unsafe impl Sync for NapiThreadsafeFunctionInner {}

#[allow(non_camel_case_types)]
pub type napi_threadsafe_function = *mut NapiThreadsafeFunctionInner;

struct TsfnPendingCall {
    tsfn: *mut NapiThreadsafeFunctionInner,
    data: *mut c_void,
}
unsafe impl Send for TsfnPendingCall {}
unsafe impl Sync for TsfnPendingCall {}

static TSFN_QUEUE: std::sync::OnceLock<std::sync::Mutex<Vec<TsfnPendingCall>>> =
    std::sync::OnceLock::new();

// Keep TSFNs alive until released
#[allow(clippy::vec_box)] // Box needed: pointer to inner struct is handed to callers
static NAPI_TSFN_STORE: std::sync::OnceLock<
    std::sync::Mutex<Vec<Box<NapiThreadsafeFunctionInner>>>,
> = std::sync::OnceLock::new();

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_create_threadsafe_function(
    env: napi_env,
    func: napi_value,
    _async_resource: napi_value,
    _async_resource_name: napi_value,
    _max_queue_size: usize,
    _initial_thread_count: usize,
    _thread_finalize_data: *mut c_void,
    _thread_finalize_cb: napi_finalize,
    context: *mut c_void,
    call_js_cb: napi_threadsafe_function_call_js,
    result: *mut napi_threadsafe_function,
) -> napi_status {
    if env.is_null() || result.is_null() {
        return napi_status::napi_invalid_arg;
    }
    let func_val = if func.is_null() {
        qjs::JS_UNDEFINED
    } else {
        qjs::JS_DupValue(jsval(func))
    };
    let tsfn = Box::new(NapiThreadsafeFunctionInner {
        env,
        func: func_val,
        context,
        call_js_cb,
    });
    let ptr = tsfn.as_ref() as *const NapiThreadsafeFunctionInner as napi_threadsafe_function;
    NAPI_TSFN_STORE
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(tsfn);
    *result = ptr;
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_call_threadsafe_function(
    func: napi_threadsafe_function,
    data: *mut c_void,
    _is_blocking: u32,
) -> napi_status {
    if func.is_null() {
        return napi_status::napi_invalid_arg;
    }
    TSFN_QUEUE
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(TsfnPendingCall { tsfn: func, data });
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_release_threadsafe_function(
    _func: napi_threadsafe_function,
    _mode: u32,
) -> napi_status {
    napi_status::napi_ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn napi_unref_threadsafe_function(
    _env: napi_env,
    _func: napi_threadsafe_function,
) -> napi_status {
    napi_status::napi_ok
}

/// Drain all pending cross-thread NAPI calls (TSFNs and deferred resolutions).
/// Call this from the JS event loop on every iteration.
///
/// # Safety
/// Must be called only from the main JS thread while holding no QuickJS locks.
pub unsafe fn drain_tsfn_queue() {
    // Drain TSFN calls
    let tsfn_calls = TSFN_QUEUE
        .get()
        .map(|q| std::mem::take(&mut *q.lock().unwrap()))
        .unwrap_or_default();

    for call in tsfn_calls {
        let tsfn = &*call.tsfn;
        let env = tsfn.env;
        if env.is_null() {
            continue;
        }
        let ctx_ptr = (*env).ctx;
        let func_napi: napi_value = if qjs::JS_IsUndefined(tsfn.func) {
            std::ptr::null_mut()
        } else {
            env_alloc(env, tsfn.func)
        };
        if let Some(cb) = tsfn.call_js_cb {
            cb(env, func_napi, tsfn.context, call.data);
        } else if !func_napi.is_null() {
            let ret = qjs::JS_Call(
                ctx_ptr,
                jsval(func_napi),
                qjs::JS_UNDEFINED,
                0,
                std::ptr::null_mut(),
            );
            qjs::JS_FreeValue(ctx_ptr, ret);
        }
        while qjs::JS_ExecutePendingJob(qjs::JS_GetRuntime(ctx_ptr), &mut ctx_ptr.cast()) > 0 {}
    }

    // Drain deferred resolve/reject calls (may come from background threads)
    let deferred_calls = DEFERRED_QUEUE
        .get()
        .map(|q| std::mem::take(&mut *q.lock().unwrap()))
        .unwrap_or_default();

    for call in deferred_calls {
        let d = Box::from_raw(call.deferred);
        let val = if call.value.is_null() {
            qjs::JS_UNDEFINED
        } else {
            jsval(call.value)
        };
        let mut args = [val];
        let func = if call.reject { d.reject } else { d.resolve };
        let ctx_ptr = d.ctx;
        let resolve_ref = d.resolve;
        let reject_ref = d.reject;
        std::mem::forget(d); // prevent Drop from trying to free JSValues on wrong thread
        let ret = qjs::JS_Call(ctx_ptr, func, qjs::JS_UNDEFINED, 1, args.as_mut_ptr());
        if qjs::JS_IsException(ret) {
            let exc = qjs::JS_GetException(ctx_ptr);
            qjs::JS_FreeValue(ctx_ptr, exc);
        }
        qjs::JS_FreeValue(ctx_ptr, ret);
        // Free both resolve+reject JSValues now that we've consumed the deferred
        qjs::JS_FreeValue(ctx_ptr, resolve_ref);
        qjs::JS_FreeValue(ctx_ptr, reject_ref);
        let mut ctx_tmp = ctx_ptr;
        while qjs::JS_ExecutePendingJob(qjs::JS_GetRuntime(ctx_ptr), &mut ctx_tmp) > 0 {}
    }
}

// ─ napi_create_string_utf8 duplicate check (it may exist already) ──────────
// (The original napi_create_string_utf8 is already defined; these aliases
//  ensure that napi_create_int32 / napi_create_int64 are visible even if the
//  existing ones use slightly different signatures.)

// ─ Load a .node addon ────────────────────────────────────────────────────────

struct NapiAddon {
    _lib: Library,
    env: *mut NapiEnvInner,
}
unsafe impl Send for NapiAddon {}
unsafe impl Sync for NapiAddon {}
impl Drop for NapiAddon {
    fn drop(&mut self) {
        if !self.env.is_null() {
            unsafe { drop(Box::from_raw(self.env)) };
        }
    }
}

static NAPI_LIBS: std::sync::OnceLock<std::sync::Mutex<Vec<NapiAddon>>> =
    std::sync::OnceLock::new();

/// Load a `.node` NAPI addon from `path` and return the exports object.
///
/// # Safety
/// Calls into foreign C code. The addon must be a valid NAPI v1 module.
pub fn load_napi_addon<'js>(ctx: &Ctx<'js>, path: &Path) -> anyhow::Result<rquickjs::Value<'js>> {
    // Safety: dlopen
    let lib = unsafe { Library::new(path) }
        .map_err(|e| anyhow::anyhow!("NAPI dlopen {:?}: {e}", path))?;

    type RegisterFn = unsafe extern "C" fn(napi_env, napi_value) -> napi_value;
    let register: libloading::Symbol<RegisterFn> = unsafe { lib.get(b"napi_register_module_v1\0") }
        .map_err(|e| anyhow::anyhow!("napi_register_module_v1 not found in {:?}: {e}", path))?;

    let raw_ctx = ctx.as_raw().as_ptr();
    // env_inner is heap-allocated and intentionally leaked into NAPI_LIBS so that
    // NAPI callback trampolines (which hold a raw pointer to it) remain valid for
    // the lifetime of the addon. It is freed when the NapiAddon is dropped.
    let env_ptr: napi_env = Box::into_raw(Box::new(NapiEnvInner {
        ctx: raw_ctx,
        last_error: None,
        values: Vec::new(),
        refs: HashMap::new(),
        next_ref_id: 1,
    }));

    let exports_js = unsafe { qjs::JS_NewObject(raw_ctx) };
    let exports_napi = unsafe { env_alloc(env_ptr, exports_js) };
    unsafe { qjs::JS_FreeValue(raw_ctx, exports_js) };

    let result_napi = unsafe { register(env_ptr, exports_napi) };

    let result_js_val = if result_napi.is_null() {
        unsafe { qjs::JS_DupValue((*exports_napi).val) }
    } else {
        unsafe { qjs::JS_DupValue((*result_napi).val) }
    };

    // Keep the library and env alive so symbols and callback trampolines remain valid.
    NAPI_LIBS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(NapiAddon {
            _lib: lib,
            env: env_ptr,
        });

    // Safety: result_js_val is owned, ctx is valid.
    let result = unsafe { rquickjs::Value::from_raw(ctx.clone(), result_js_val) };
    Ok(result)
}

// ── Inject __napiRequire into JS ──────────────────────────────────────────────
//
// We can't return Value<'js>/Object<'js> from a 'static closure because those
// types are invariant over 'js. Instead we store the exports in a temporary
// global (`__napi_tmp_exports`) and return a sentinel string. The JS wrapper
// around __napiRequire retrieves and deletes the global immediately after.

static NAPI_SLOT_KEY: &str = "__napi_tmp_exports__";

pub fn inject_napi(ctx: &Ctx<'_>, permissions: Arc<PermissionState>) -> Result<()> {
    let perms = permissions;

    // __napiRequire(path) → stores exports in __napi_tmp_exports__, returns sentinel.
    ctx.globals().set(
        "__napiRequireRaw",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, path: String| -> rquickjs::Result<String> {
                let p = std::path::PathBuf::from(&path);
                if !perms.check(&Capability::FFI(p.clone())) {
                    return Err(rquickjs::Error::new_from_js_message(
                        "napi",
                        "require",
                        &format!("Permission denied: --allow-ffi={path}"),
                    ));
                }
                let val = load_napi_addon(&ctx, &p).map_err(|e| {
                    rquickjs::Error::new_from_js_message("napi", "require", &e.to_string())
                })?;
                ctx.globals().set(NAPI_SLOT_KEY, val)?;
                Ok("ok".to_string())
            },
        )?,
    )?;

    // JS wrapper: call the raw loader, grab the exports, clean up the global.
    ctx.eval::<(), _>(
        r#"
        globalThis.__napiRequire = function(path) {
            var sentinel = globalThis.__napiRequireRaw(path);
            var exports = globalThis.__napi_tmp_exports__;
            delete globalThis.__napi_tmp_exports__;
            return exports;
        };
    "#,
    )?;

    Ok(())
}
