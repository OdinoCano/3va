use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_void};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use libffi::middle::{Cif, CodePtr, Type, arg};
use libloading::Library;
use rquickjs::{Ctx, Function, Result, function::Rest};
use vvva_permissions::{Capability, PermissionState};

fn js_err(ctx: &Ctx<'_>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value, _>(format!("new Error(\"{}\")", escaped).as_str()) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

struct FfiLib {
    _lib: Library,
}

// Safety: the raw pointers obtained from dlsym are valid as long as _lib is alive.
// We never move _lib out of FfiLib, and FfiLib lives in the thread-local map.
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

/// Storage for a single native argument.
/// Box ensures the value doesn't move between construction and the libffi call.
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
    /// Pointer value (stored as usize to keep it FFI-safe).
    Ptr(Box<usize>),
    /// `_cs` keeps the CString buffer alive while `ptr` is passed to libffi.
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

/// Invoke a native function through libffi.
///
/// # Safety
/// `fn_ptr` must be a valid function pointer whose ABI matches the declared signature.
/// The caller is responsible for ensuring this invariant through the permission system
/// and the user-supplied type declarations.
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

pub fn inject_ffi(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // ── __ffiDlopen(path: string) -> string (handle_id) ─────────────────────
    let perms = permissions.clone();
    globals.set(
        "__ffiDlopen",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "dlopen() requires a library path".into()))?;

                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FFI(path.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!(
                            "Permission denied: --allow-ffi={} is required",
                            path.display()
                        ),
                    ));
                }

                let lib = unsafe {
                    Library::new(&path_str)
                        .map_err(|e| js_err(&ctx, format!("dlopen failed: {e}")))?
                };

                let handle_id = NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed);
                FFI_LIBS.with(|libs| {
                    libs.borrow_mut().insert(handle_id, FfiLib { _lib: lib });
                });

                Ok(handle_id.to_string())
            },
        )?,
    )?;

    // ── __ffiCall(handle_id, symbol, ret_type, arg_types_json, args_json) -> string ──
    globals.set(
        "__ffiCall",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let mut it = args.0.into_iter();
                let handle_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__ffiCall: missing handle_id".into()))?;
                let symbol_name = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__ffiCall: missing symbol name".into()))?;
                let ret_type = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__ffiCall: missing return type".into()))?;
                let arg_types_json = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__ffiCall: missing arg types".into()))?;
                let args_json = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__ffiCall: missing args".into()))?;

                let handle_id: u64 = handle_str
                    .parse()
                    .map_err(|_| js_err(&ctx, format!("Invalid FFI handle: {handle_str}")))?;

                let arg_types: Vec<String> = serde_json::from_str(&arg_types_json)
                    .map_err(|e| js_err(&ctx, format!("Invalid arg types JSON: {e}")))?;

                let js_args: Vec<serde_json::Value> = serde_json::from_str(&args_json)
                    .map_err(|e| js_err(&ctx, format!("Invalid args JSON: {e}")))?;

                // Look up the symbol and call — done inside the thread_local borrow
                // so the Library stays alive for the duration of the call.
                let result = FFI_LIBS.with(|libs| {
                    let libs = libs.borrow();
                    let handle = libs.get(&handle_id).ok_or_else(|| {
                        anyhow::anyhow!("Invalid or closed FFI handle: {handle_id}")
                    })?;

                    let fn_ptr: *mut c_void = unsafe {
                        let sym: libloading::Symbol<*mut c_void> =
                            handle._lib.get(symbol_name.as_bytes()).map_err(|e| {
                                anyhow::anyhow!("Symbol '{}' not found: {}", symbol_name, e)
                            })?;
                        *sym
                    };

                    unsafe { call_native(fn_ptr, &ret_type, &arg_types, &js_args) }
                });

                match result {
                    Ok(val) => Ok(val.to_string()),
                    Err(e) => Err(js_err(&ctx, e.to_string())),
                }
            },
        )?,
    )?;

    // ── __ffiClose(handle_id: string) ────────────────────────────────────────
    globals.set(
        "__ffiClose",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let handle_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__ffiClose: missing handle_id".into()))?;
                let handle_id: u64 = handle_str
                    .parse()
                    .map_err(|_| js_err(&ctx, format!("Invalid FFI handle: {handle_str}")))?;
                FFI_LIBS.with(|libs| {
                    libs.borrow_mut().remove(&handle_id);
                });
                Ok(())
            },
        )?,
    )?;

    // ── Inject the JS-facing `ffi` module ────────────────────────────────────
    ctx.eval::<(), _>(
        r#"(function() {
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
        var parsed = JSON.parse(resultJson);
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
})();"#,
    )?;

    Ok(())
}
