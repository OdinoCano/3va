// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! `napi_get_prototype`, `napi_has_own_property` and `napi_remove_wrap`.
//!
//! napi-rs addons (e.g. `@node-rs/argon2`) import all three, so without them the
//! addon fails to load at all ("undefined symbol"). Error cases check the status
//! codes against Node's `napi_status` enum (object_expected = 2, name_expected = 4).

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;

    use vvva_js::JsEngine;
    use vvva_permissions::{Capability, PermissionState};

    fn build_addon() -> Option<PathBuf> {
        let dir = tempfile::TempDir::new().unwrap();
        let out = dir.path().join("napi_objects.node");
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("napi_objects.c");
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let ok = Command::new(cc)
            .args(["-shared", "-fPIC", "-O1", "-o"])
            .arg(&out)
            .arg(&src)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return None;
        }
        let out = out.canonicalize().ok()?;
        std::mem::forget(dir);
        Some(out)
    }

    async fn check(engine: &mut JsEngine, js: &str, expected: &str) {
        let got = engine.eval_to_string(js).await.unwrap();
        assert_eq!(got, expected, "{js}");
    }

    #[tokio::test]
    async fn prototype_own_property_and_remove_wrap() {
        let Some(addon) = build_addon() else {
            eprintln!("skipping prototype_own_property_and_remove_wrap: no C compiler");
            return;
        };
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::FFI(addon.clone()));
        let mut engine = JsEngine::new(perms).await.unwrap();
        engine
            .eval_to_string(&format!(
                "globalThis.a = require({:?}); 'ok'",
                addon.to_string_lossy()
            ))
            .await
            .unwrap();

        // napi_get_prototype
        check(
            &mut engine,
            "class P {} ; var x = new P(); String(a.proto(x) === P.prototype && a.proto(x) === Object.getPrototypeOf(x))",
            "true",
        )
        .await;
        check(&mut engine, "String(a.proto(Object.create(null)))", "null").await;
        check(&mut engine, "String(a.proto(undefined))", "-2").await; // object_expected

        // napi_has_own_property: own yes, inherited no, symbols ok, bad key/object
        check(&mut engine, "String(a.own({ p: 1 }, 'p'))", "true").await;
        check(&mut engine, "String(a.own({}, 'p'))", "false").await;
        check(
            &mut engine,
            "String(a.own(Object.create({ p: 1 }), 'p'))",
            "false",
        )
        .await;
        check(
            &mut engine,
            "var s = Symbol('k'); var o = {}; o[s] = 1; String(a.own(o, s))",
            "true",
        )
        .await;
        check(&mut engine, "String(a.own({}, 5))", "-4").await; // name_expected
        check(&mut engine, "String(a.own(undefined, 'p'))", "-2").await; // object_expected

        // napi_remove_wrap: first ok + same pointer, second invalid_arg (0*100 + 1*10 + 1)
        check(&mut engine, "String(a.wrapRemove({}))", "11").await;
    }
    // The descriptor layout is Node's (64 bytes, with `name` and `method`); it used
    // to omit those two fields, so only a one-element array worked and methods,
    // accessors and statics were ignored (a 2+ property class corrupted memory).
    #[tokio::test]
    async fn class_with_method_accessor_and_statics() {
        let Some(addon) = build_addon() else {
            eprintln!("skipping class_with_method_accessor_and_statics: no C compiler");
            return;
        };
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::FFI(addon.clone()));
        let mut engine = JsEngine::new(perms).await.unwrap();
        engine
            .eval_to_string(&format!(
                "globalThis.a = require({:?}); 'ok'",
                addon.to_string_lossy()
            ))
            .await
            .unwrap();

        check(
            &mut engine,
            r#"var C = a.Counter, c = new C(), d = new C();
               c.inc(); c.inc(); d.inc();
               var before = [c.value, d.value];
               c.value = 10;
               var desc = Object.getOwnPropertyDescriptor(C.prototype, 'inc');
               var acc = Object.getOwnPropertyDescriptor(C.prototype, 'value');
               JSON.stringify([
                 before, c.value, c instanceof C, C.kind, C.answer(),
                 Object.getOwnPropertyNames(C.prototype).sort().join(),
                 desc.writable, desc.enumerable, typeof acc.get, typeof acc.set, acc.enumerable
               ])"#,
            r#"[[2,1],10,true,"counter",42,"constructor,inc,value",true,false,"function","function",true]"#,
        )
        .await;
    }
    // `napi_get_typedarray_info` ignored its type out-parameter, so every addon
    // saw 0 (`napi_int8_array`). Each constructor must report Node-API's number,
    // together with the element length and byte offset.
    #[tokio::test]
    async fn typedarray_info_reports_the_real_type() {
        let Some(addon) = build_addon() else {
            eprintln!("skipping typedarray_info_reports_the_real_type: no C compiler");
            return;
        };
        let perms = Arc::new(PermissionState::new());
        perms.grant(Capability::FFI(addon.clone()));
        let mut engine = JsEngine::new(perms).await.unwrap();
        engine
            .eval_to_string(&format!(
                "globalThis.a = require({:?}); 'ok'",
                addon.to_string_lossy()
            ))
            .await
            .unwrap();

        // type * 10000 + length * 100 + byteOffset, one entry per napi_typedarray_type
        check(
            &mut engine,
            r#"var buf = new ArrayBuffer(64);
               JSON.stringify([
                 new Int8Array(buf, 0, 3), new Uint8Array(buf, 1, 4),
                 new Uint8ClampedArray(buf, 2, 5), new Int16Array(buf, 2, 3),
                 new Uint16Array(buf, 4, 2), new Int32Array(buf, 8, 2),
                 new Uint32Array(buf, 8, 3), new Float32Array(buf, 4, 6),
                 new Float64Array(buf, 8, 2), new BigInt64Array(buf, 8, 2),
                 new BigUint64Array(buf, 16, 1)
               ].map(function (t) { return a.taInfo(t); }))"#,
            "[300,10401,20502,30302,40204,50208,60308,70604,80208,90208,100116]",
        )
        .await;
        // A Node Buffer is a Uint8Array.
        check(
            &mut engine,
            "String(a.taInfo(Buffer.from('abcd')))",
            "10400",
        )
        .await;
        // Not a typed array: napi_invalid_arg.
        check(&mut engine, "String(a.taInfo({}))", "-1").await;
    }
}
