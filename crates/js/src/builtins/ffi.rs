use libffi::middle::{Cif, CodePtr, Type, arg};
use libloading::Library;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use v8::{
    Function, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue, Script,
    String as V8String,
};
use vvva_permissions::{Capability, PermissionState};

static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

struct FfiLib {
    _lib: Library,
}

unsafe impl Send for FfiLib {}

thread_local! {
    static FFI_LIBS: RefCell<HashMap<u64, FfiLib>> = RefCell::new(HashMap::new());
}

fn ffi_type_from_str(s: &str) -> anyhow::Result<Type> {
    Ok(match s {
        "void" => Type::void(),
        "i8" => Type::i8(),
        "i16" => Type::i16(),
        "i32" => Type::i32(),
        "i64" => Type::i64(),
        "u8" => Type::u8(),
        "u16" => Type::u16(),
        "u32" => Type::u32(),
        "u64" => Type::u64(),
        "f32" => Type::f32(),
        "f64" => Type::f64(),
        "pointer" | "buffer" => Type::pointer(),
        "cstring" => Type::pointer(),
        _ => anyhow::bail!("Unknown FFI type: {s}"),
    })
}

enum ArgStorage {
    I8(Box<i8>),
    I16(Box<i16>),
    I32(Box<i32>),
    I64(Box<i64>),
    U8(Box<u8>),
    U16(Box<u16>),
    U32(Box<u32>),
    U64(Box<u64>),
    F32(Box<f32>),
    F64(Box<f64>),
    Ptr(Box<usize>),
    CStr {
        _cs: CString,
        ptr: Box<*const c_char>,
    },
}

impl ArgStorage {
    fn as_ffi_arg(&self) -> libffi::middle::Arg {
        match self {
            ArgStorage::I8(v) => arg(v.as_ref()),
            ArgStorage::I16(v) => arg(v.as_ref()),
            ArgStorage::I32(v) => arg(v.as_ref()),
            ArgStorage::I64(v) => arg(v.as_ref()),
            ArgStorage::U8(v) => arg(v.as_ref()),
            ArgStorage::U16(v) => arg(v.as_ref()),
            ArgStorage::U32(v) => arg(v.as_ref()),
            ArgStorage::U64(v) => arg(v.as_ref()),
            ArgStorage::F32(v) => arg(v.as_ref()),
            ArgStorage::F64(v) => arg(v.as_ref()),
            ArgStorage::Ptr(v) => arg(v.as_ref()),
            ArgStorage::CStr { ptr, .. } => arg(ptr.as_ref()),
        }
    }
}

fn js_value_to_arg(typ: &str, val: &serde_json::Value) -> anyhow::Result<ArgStorage> {
    Ok(match typ {
        "i8" => ArgStorage::I8(Box::new(val.as_i64().unwrap_or(0) as i8)),
        "i16" => ArgStorage::I16(Box::new(val.as_i64().unwrap_or(0) as i16)),
        "i32" => ArgStorage::I32(Box::new(val.as_i64().unwrap_or(0) as i32)),
        "i64" => ArgStorage::I64(Box::new(val.as_i64().unwrap_or(0))),
        "u8" => ArgStorage::U8(Box::new(val.as_u64().unwrap_or(0) as u8)),
        "u16" => ArgStorage::U16(Box::new(val.as_u64().unwrap_or(0) as u16)),
        "u32" => ArgStorage::U32(Box::new(val.as_u64().unwrap_or(0) as u32)),
        "u64" => ArgStorage::U64(Box::new(val.as_u64().unwrap_or(0))),
        "f32" => ArgStorage::F32(Box::new(val.as_f64().unwrap_or(0.0) as f32)),
        "f64" => ArgStorage::F64(Box::new(val.as_f64().unwrap_or(0.0))),
        "pointer" | "buffer" => {
            let raw: usize = val.as_u64().unwrap_or(0) as usize;
            ArgStorage::Ptr(Box::new(raw))
        }
        "cstring" => {
            let s = val.as_str().unwrap_or("");
            let cs = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
            let raw_ptr = cs.as_ptr();
            ArgStorage::CStr {
                _cs: cs,
                ptr: Box::new(raw_ptr),
            }
        }
        _ => anyhow::bail!("Unknown FFI type for argument: {typ}"),
    })
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn call_native(
    fn_ptr: *mut c_void,
    ret_type: &str,
    arg_types: &[String],
    js_args: &[serde_json::Value],
) -> anyhow::Result<serde_json::Value> {
    let ret_ffi = ffi_type_from_str(ret_type)?;
    let arg_ffi: Vec<Type> = arg_types
        .iter()
        .map(|t| ffi_type_from_str(t))
        .collect::<anyhow::Result<_>>()?;

    let cif = Cif::new(arg_ffi, ret_ffi);
    let code = CodePtr(fn_ptr as *mut _);

    let storages: Vec<ArgStorage> = arg_types
        .iter()
        .zip(js_args.iter())
        .map(|(t, v)| js_value_to_arg(t, v))
        .collect::<anyhow::Result<_>>()?;

    let ffi_args: Vec<libffi::middle::Arg> = storages.iter().map(|s| s.as_ffi_arg()).collect();

    Ok(match ret_type {
        "void" => {
            cif.call::<()>(code, &ffi_args);
            serde_json::Value::Null
        }
        "i8" => {
            let r: i8 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "i16" => {
            let r: i16 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "i32" => {
            let r: i32 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "i64" => {
            let r: i64 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "u8" => {
            let r: u8 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "u16" => {
            let r: u16 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "u32" => {
            let r: u32 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "u64" => {
            let r: u64 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "f32" => {
            let r: f32 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "f64" => {
            let r: f64 = cif.call(code, &ffi_args);
            serde_json::json!(r)
        }
        "pointer" | "buffer" => {
            let r: *const c_void = cif.call(code, &ffi_args);
            serde_json::json!(r as usize)
        }
        "cstring" => {
            let ptr: *const c_char = cif.call(code, &ffi_args);
            if ptr.is_null() {
                serde_json::Value::Null
            } else {
                let cstr = unsafe { std::ffi::CStr::from_ptr(ptr) };
                serde_json::json!(cstr.to_string_lossy().into_owned())
            }
        }
        _ => anyhow::bail!("Unknown FFI return type: {ret_type}"),
    })
}

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static INJECT_FFI_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn permissions() -> Arc<PermissionState> {
    INJECT_FFI_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_ffi not called on this thread")
    })
}

pub fn inject_ffi(
    scope: &mut v8::ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    INJECT_FFI_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));
    let ffi_dlopen_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              mut rv: ReturnValue<'_>| {
            let path_arg = args.get(0);
            let path_str = path_arg.to_rust_string_lossy(scope);

            let path = PathBuf::from(&path_str);
            if !permissions().check(&Capability::FFI(path.clone())) {
                let err_str = V8String::new(
                    scope,
                    &format!(
                        "Permission denied: --allow-ffi={} is required",
                        path.display()
                    ),
                )
                .unwrap();
                rv.set(err_str.into());
                return;
            }

            let lib = unsafe {
                match Library::new(&path_str) {
                    Ok(l) => l,
                    Err(e) => {
                        let err_str =
                            V8String::new(scope, &format!("dlopen failed: {}", e)).unwrap();
                        rv.set(err_str.into());
                        return;
                    }
                }
            };

            let handle_id = NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed);
            FFI_LIBS.with(|libs| {
                libs.borrow_mut().insert(handle_id, FfiLib { _lib: lib });
            });

            rv.set(V8String::new(scope, &handle_id.to_string()).unwrap().into());
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__ffiDlopen").unwrap().into(),
        ffi_dlopen_fn.unwrap().into(),
    );

    let ffi_call_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              mut rv: ReturnValue<'_>| {
            let handle_str_arg = args.get(0);
            let handle_str = handle_str_arg.to_rust_string_lossy(scope);
            let symbol_name_arg = args.get(1);
            let symbol_name = symbol_name_arg.to_rust_string_lossy(scope);
            let ret_type_arg = args.get(2);
            let ret_type = ret_type_arg.to_rust_string_lossy(scope);
            let arg_types_json_arg = args.get(3);
            let arg_types_json = arg_types_json_arg.to_rust_string_lossy(scope);
            let args_json_arg = args.get(4);
            let args_json = args_json_arg.to_rust_string_lossy(scope);

            let handle_id: u64 = match handle_str.parse() {
                Ok(id) => id,
                Err(_) => {
                    let err_str = V8String::new(scope, "Invalid FFI handle").unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            let arg_types: Vec<String> = match serde_json::from_str(&arg_types_json) {
                Ok(t) => t,
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("Invalid arg types JSON: {}", e)).unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            let js_args: Vec<serde_json::Value> = match serde_json::from_str(&args_json) {
                Ok(a) => a,
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("Invalid args JSON: {}", e)).unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            let result = FFI_LIBS.with(|libs| {
                let libs = libs.borrow();
                let handle = match libs.get(&handle_id) {
                    Some(h) => h,
                    None => {
                        return Err(anyhow::anyhow!(
                            "Invalid or closed FFI handle: {}",
                            handle_id
                        ));
                    }
                };

                let fn_ptr: *mut c_void = unsafe {
                    let sym: libloading::Symbol<*mut c_void> =
                        match handle._lib.get(symbol_name.as_bytes()) {
                            Ok(s) => s,
                            Err(e) => {
                                return Err(anyhow::anyhow!(
                                    "Symbol '{}' not found: {}",
                                    symbol_name,
                                    e
                                ));
                            }
                        };
                    *sym
                };

                unsafe { call_native(fn_ptr, &ret_type, &arg_types, &js_args) }
            });

            match result {
                Ok(val) => {
                    let result_str = V8String::new(scope, &val.to_string()).unwrap();
                    rv.set(result_str.into());
                }
                Err(e) => {
                    let err_str = V8String::new(scope, &e.to_string()).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__ffiCall").unwrap().into(),
        ffi_call_fn.unwrap().into(),
    );

    let ffi_close_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments<'_>,
              mut rv: ReturnValue<'_>| {
            let handle_str_arg = args.get(0);
            let handle_str = handle_str_arg.to_rust_string_lossy(scope);

            let handle_id: u64 = match handle_str.parse() {
                Ok(id) => id,
                Err(_) => {
                    let err_str = V8String::new(scope, "Invalid FFI handle").unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            FFI_LIBS.with(|libs| {
                libs.borrow_mut().remove(&handle_id);
            });
            rv.set(v8::undefined(scope).into());
        },
    );
    global.set(
        scope,
        V8String::new(scope, "__ffiClose").unwrap().into(),
        ffi_close_fn.unwrap().into(),
    );

    let js_code = r#"(function() {
        var FFIType = {
            void:    'void',
            i8:      'i8',
            i16:     'i16',
            i32:     'i32',
            i64:     'i64',
            u8:      'u8',
            u16:     'u16',
            u32:     'u32',
            u64:     'u64',
            f32:     'f32',
            f64:     'f64',
            pointer: 'pointer',
            cstring: 'cstring',
            buffer:  'buffer'
        };

        function dlopen(libPath, symbolDefs) {
            var handleId = __ffiDlopen(libPath);
            // Success is a bare numeric handle id; anything else is a native
            // error message (permission denied, dlopen failed, ...) that was
            // returned rather than thrown across the Rust/V8 boundary.
            if (!/^\d+$/.test(handleId)) {
                var openErr = new Error(handleId);
                if (handleId.indexOf('Permission denied') !== -1) openErr.code = 'EACCES';
                throw openErr;
            }
            var symbols = {};

            Object.keys(symbolDefs).forEach(function(name) {
                var def = symbolDefs[name];
                var argTypes = def.args || [];
                var retType  = def.returns || 'void';

                symbols[name] = function() {
                    var jsArgs = Array.prototype.slice.call(arguments);
                    var resultJson = __ffiCall(
                        handleId,
                        name,
                        retType,
                        JSON.stringify(argTypes),
                        JSON.stringify(jsArgs)
                    );
                    var parsed;
                    try { parsed = JSON.parse(resultJson); }
                    catch (e) { throw new Error(resultJson); }
                    return (parsed === null && retType === 'void') ? undefined : parsed;
                };
            });

            return {
                symbols: symbols,
                close: function() { __ffiClose(handleId); }
            };
        }

        var mod = { dlopen: dlopen, FFIType: FFIType };
        globalThis.__requireCache['ffi']      = mod;
        globalThis.__requireCache['node:ffi'] = mod;
    })();"#;
    let source = V8String::new(scope, js_code).unwrap();
    let _ = Script::compile(scope, source, None).and_then(|s| s.run(scope));

    Ok(())
}
