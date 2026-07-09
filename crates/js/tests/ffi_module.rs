use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

// Used only for permission-denial tests (never actually loaded).
const TEST_LIB_PATH: &str = "/lib/test/dummy.so";

#[cfg(target_os = "linux")]
const LIBM: &str = "/lib/x86_64-linux-gnu/libm.so.6";
#[cfg(target_os = "linux")]
const LIBC: &str = "/lib/x86_64-linux-gnu/libc.so.6";

async fn engine_no_perms() -> JsEngine {
    let perms = Arc::new(PermissionState::new());
    JsEngine::new(perms).await.unwrap()
}

async fn engine_with_ffi(lib_path: &str) -> JsEngine {
    let perms = Arc::new(PermissionState::new());
    perms.grant(Capability::FFI(std::path::PathBuf::from(lib_path)));
    JsEngine::new(perms).await.unwrap()
}

async fn engine_ffi_all() -> JsEngine {
    let perms = Arc::new(PermissionState::new());
    perms.grant(Capability::FFI(std::path::PathBuf::from("/")));
    JsEngine::new(perms).await.unwrap()
}

// ── Module API surface ────────────────────────────────────────────────────────

#[tokio::test]
async fn ffi_require_exposes_dlopen_and_types() {
    let mut e = engine_no_perms().await;
    let r = e
        .eval_to_string(
            "const ffi = require('ffi');
             typeof ffi.dlopen + ',' + typeof ffi.FFIType",
        )
        .await
        .unwrap();
    assert_eq!(r, "function,object");
}

#[tokio::test]
async fn ffi_ffitypes_has_expected_keys() {
    let mut e = engine_no_perms().await;
    let r = e
        .eval_to_string(
            "const t = require('ffi').FFIType;
             [t.void, t.i32, t.i64, t.u32, t.u64, t.f32, t.f64, t.pointer, t.cstring].join(',')",
        )
        .await
        .unwrap();
    assert_eq!(r, "void,i32,i64,u32,u64,f32,f64,pointer,cstring");
}

#[tokio::test]
async fn ffi_node_prefix_alias_works() {
    let mut e = engine_no_perms().await;
    let r = e
        .eval_to_string(
            "const a = require('ffi'); const b = require('node:ffi'); a === b ? 'same' : 'diff'",
        )
        .await
        .unwrap();
    assert_eq!(r, "same");
}

// ── Permission enforcement ────────────────────────────────────────────────────

#[tokio::test]
async fn dlopen_denied_without_allow_ffi() {
    let mut e = engine_no_perms().await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 try {{
                   require('ffi').dlopen('{}', {{}});
                   return 'allowed';
                 }} catch(err) {{
                   return err.message.includes('Permission denied') ? 'denied' : 'wrong:' + err.message;
                 }}
               }})()"#,
            TEST_LIB_PATH
        ))
        .await
        .unwrap();
    assert_eq!(r, "denied", "dlopen without --allow-ffi should be denied");
}

#[tokio::test]
async fn dlopen_denied_when_path_not_in_grant() {
    let mut e = engine_with_ffi("/opt/custom").await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 try {{
                   require('ffi').dlopen('{}', {{}});
                   return 'allowed';
                 }} catch(err) {{
                   return err.message.includes('Permission denied') ? 'denied' : 'wrong:' + err.message;
                 }}
               }})()"#,
            TEST_LIB_PATH
        ))
        .await
        .unwrap();
    assert_eq!(r, "denied", "dlopen outside granted path should be denied");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dlopen_allowed_with_exact_path_grant() {
    let mut e = engine_with_ffi(LIBM).await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 try {{
                   var lib = require('ffi').dlopen('{}', {{
                     sqrt: {{ args: ['f64'], returns: 'f64' }}
                   }});
                   lib.close();
                   return 'ok';
                 }} catch(err) {{ return 'err:' + err.message; }}
               }})()"#,
            LIBM
        ))
        .await
        .unwrap();
    assert_eq!(r, "ok", "dlopen with exact path grant should succeed");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dlopen_allowed_with_prefix_grant() {
    // Granting /lib/x86_64-linux-gnu/ should cover LIBM
    let perms = Arc::new(PermissionState::new());
    perms.grant(Capability::FFI(std::path::PathBuf::from(
        "/lib/x86_64-linux-gnu",
    )));
    let mut e = JsEngine::new(perms).await.unwrap();
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 try {{
                   var lib = require('ffi').dlopen('{}', {{}});
                   lib.close();
                   return 'ok';
                 }} catch(err) {{ return 'err:' + err.message; }}
               }})()"#,
            LIBM
        ))
        .await
        .unwrap();
    assert_eq!(r, "ok", "dlopen under granted prefix should succeed");
}

// ── Calling native functions ──────────────────────────────────────────────────

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ffi_call_sqrt_f64() {
    let mut e = engine_with_ffi(LIBM).await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 var {{ dlopen, FFIType }} = require('ffi');
                 var lib = dlopen('{}', {{
                   sqrt: {{ args: [FFIType.f64], returns: FFIType.f64 }}
                 }});
                 var result = lib.symbols.sqrt(4.0);
                 lib.close();
                 return String(result);
               }})()"#,
            LIBM
        ))
        .await
        .unwrap();
    assert_eq!(r, "2", "sqrt(4.0) should return 2");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ffi_call_abs_i32() {
    let mut e = engine_with_ffi(LIBC).await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 var {{ dlopen, FFIType }} = require('ffi');
                 var lib = dlopen('{}', {{
                   abs: {{ args: [FFIType.i32], returns: FFIType.i32 }}
                 }});
                 var r1 = lib.symbols.abs(-42);
                 var r2 = lib.symbols.abs(7);
                 lib.close();
                 return r1 + ',' + r2;
               }})()"#,
            LIBC
        ))
        .await
        .unwrap();
    assert_eq!(r, "42,7", "abs(-42)=42, abs(7)=7");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ffi_call_strlen_cstring() {
    let mut e = engine_with_ffi(LIBC).await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 var {{ dlopen, FFIType }} = require('ffi');
                 var lib = dlopen('{}', {{
                   strlen: {{ args: [FFIType.cstring], returns: FFIType.u64 }}
                 }});
                 var r = lib.symbols.strlen('hello');
                 lib.close();
                 return String(r);
               }})()"#,
            LIBC
        ))
        .await
        .unwrap();
    assert_eq!(r, "5", "strlen('hello') should return 5");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ffi_call_pow_two_f64_args() {
    let mut e = engine_with_ffi(LIBM).await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 var {{ dlopen, FFIType }} = require('ffi');
                 var lib = dlopen('{}', {{
                   pow: {{ args: [FFIType.f64, FFIType.f64], returns: FFIType.f64 }}
                 }});
                 var r = lib.symbols.pow(2.0, 10.0);
                 lib.close();
                 return String(r);
               }})()"#,
            LIBM
        ))
        .await
        .unwrap();
    assert_eq!(r, "1024", "pow(2,10) should return 1024");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ffi_call_void_return() {
    // free(NULL) is a no-op — tests that void return works without crashing
    let mut e = engine_with_ffi(LIBC).await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 var {{ dlopen, FFIType }} = require('ffi');
                 var lib = dlopen('{}', {{
                   free: {{ args: [FFIType.pointer], returns: FFIType.void }}
                 }});
                 var result = lib.symbols.free(0);
                 lib.close();
                 return typeof result;
               }})()"#,
            LIBC
        ))
        .await
        .unwrap();
    assert_eq!(r, "undefined", "void return should be undefined in JS");
}

// ── Error handling ────────────────────────────────────────────────────────────

#[tokio::test]
async fn dlopen_nonexistent_library_throws() {
    let mut e = engine_ffi_all().await;
    let r = e
        .eval_to_string(
            r#"(function() {
                 try {
                   require('ffi').dlopen('/nonexistent/lib.so', {});
                   return 'no_throw';
                 } catch(err) {
                   return err.message.includes('dlopen failed') ? 'ok' : 'wrong:' + err.message;
                 }
               })()"#,
        )
        .await
        .unwrap();
    assert_eq!(
        r, "ok",
        "loading nonexistent library should throw dlopen error"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ffi_unknown_symbol_throws() {
    let mut e = engine_with_ffi(LIBM).await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 var {{ dlopen, FFIType }} = require('ffi');
                 var lib = dlopen('{}', {{
                   nonexistent_fn_xyz: {{ args: [], returns: FFIType.void }}
                 }});
                 try {{
                   lib.symbols.nonexistent_fn_xyz();
                   return 'no_throw';
                 }} catch(err) {{
                   lib.close();
                   return err.message.includes('nonexistent_fn_xyz') ? 'ok' : 'wrong:' + err.message;
                 }}
               }})()"#,
            LIBM
        ))
        .await
        .unwrap();
    assert_eq!(r, "ok", "calling unknown symbol should throw");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ffi_close_makes_handle_invalid() {
    let mut e = engine_with_ffi(LIBM).await;
    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 var {{ dlopen, FFIType }} = require('ffi');
                 var lib = dlopen('{}', {{
                   sqrt: {{ args: [FFIType.f64], returns: FFIType.f64 }}
                 }});
                 lib.close();
                 try {{
                   lib.symbols.sqrt(4.0);
                   return 'no_throw';
                 }} catch(err) {{
                   return err.message.includes('Invalid or closed') ? 'ok' : 'wrong:' + err.message;
                 }}
               }})()"#,
            LIBM
        ))
        .await
        .unwrap();
    assert_eq!(
        r, "ok",
        "calling after close() should throw invalid handle error"
    );
}

// ── Multiple libraries open simultaneously ────────────────────────────────────

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ffi_multiple_libs_open_simultaneously() {
    let perms = Arc::new(PermissionState::new());
    perms.grant(Capability::FFI(std::path::PathBuf::from("/")));
    let mut e = JsEngine::new(perms).await.unwrap();

    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                 var {{ dlopen, FFIType }} = require('ffi');
                 var libm = dlopen('{}', {{
                   sqrt: {{ args: [FFIType.f64], returns: FFIType.f64 }}
                 }});
                 var libc = dlopen('{}', {{
                   abs: {{ args: [FFIType.i32], returns: FFIType.i32 }}
                 }});
                 var r1 = libm.symbols.sqrt(9.0);
                 var r2 = libc.symbols.abs(-100);
                 libm.close();
                 libc.close();
                 return r1 + ',' + r2;
               }})()"#,
            LIBM, LIBC
        ))
        .await
        .unwrap();
    assert_eq!(r, "3,100", "both libraries should work independently");
}
