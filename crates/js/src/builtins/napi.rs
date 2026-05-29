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
        let t = &*(opaque as *const Trampoline);
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

    let js_fn = qjs::JS_NewCFunctionData(
        ctx(env),
        Some(call_trampoline),
        0,
        0,
        1,
        trampoline as *mut qjs::JSValue,
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

// ── Load a .node addon ────────────────────────────────────────────────────────

static NAPI_LIBS: std::sync::OnceLock<std::sync::Mutex<Vec<Library>>> = std::sync::OnceLock::new();

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
    let mut env_inner = Box::new(NapiEnvInner {
        ctx: raw_ctx,
        last_error: None,
        values: Vec::new(),
        refs: HashMap::new(),
        next_ref_id: 1,
    });
    let env_ptr = env_inner.as_mut() as napi_env;

    let exports_js = unsafe { qjs::JS_NewObject(raw_ctx) };
    let exports_napi = unsafe { env_alloc(env_ptr, exports_js) };
    unsafe { qjs::JS_FreeValue(raw_ctx, exports_js) };

    let result_napi = unsafe { register(env_ptr, exports_napi) };

    let result_js_val = if result_napi.is_null() {
        unsafe { qjs::JS_DupValue((*exports_napi).val) }
    } else {
        unsafe { qjs::JS_DupValue((*result_napi).val) }
    };

    // Keep the library alive so symbols remain valid.
    NAPI_LIBS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(lib);

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
