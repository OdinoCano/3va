// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Development probe for the startup snapshot: records every bootstrap
//! script a normal engine runs, replays them in a fresh context with no
//! native functions, and prints which ones fail and why.
//! Run: cargo test -p vvva_js --test snapshot_probe -- --ignored --nocapture

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_js::builtins::code_cache::{BootMode, set_boot_mode, take_recorded};
use vvva_permissions::PermissionState;

#[tokio::test]
#[ignore]
async fn replay_bootstrap_without_natives() {
    set_boot_mode(BootMode::Record);
    let engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    set_boot_mode(BootMode::Normal);
    drop(engine);
    let scripts = take_recorded();
    let total: usize = scripts.iter().map(|(_, s)| s.len()).sum();
    println!("recorded {} scripts, {} KB", scripts.len(), total / 1024);

    vvva_js::ensure_v8_initialized();
    let isolate = &mut v8::Isolate::new(Default::default());
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let mut failures = 0;
    for (name, src) in &scripts {
        v8::tc_scope!(let tc, scope);
        let source = v8::String::new(tc, src).unwrap();
        let ok = v8::Script::compile(tc, source, None)
            .and_then(|s| s.run(tc))
            .is_some();
        if !ok {
            failures += 1;
            let msg = tc
                .exception()
                .map(|e| e.to_rust_string_lossy(tc))
                .unwrap_or_default();
            println!("FAIL {name}: {msg}");
        } else {
            println!("ok   {name} ({} B)", src.len());
        }
    }
    println!("{failures} failing");
}

#[tokio::test]
#[ignore]
async fn list_native_functions() {
    let mut engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    let out = engine
        .eval_to_string(
            r#"(function () {
                var fresh = new Set(Object.getOwnPropertyNames(globalThis));
                var nat = function (f) { try { return typeof f === 'function' && /\{\s*\[native code\]\s*\}$/.test(Function.prototype.toString.call(f)); } catch (e) { return false; } };
                var out = [];
                var scan = function (obj, path, depth) {
                    Object.getOwnPropertyNames(obj).forEach(function (k) {
                        var d = Object.getOwnPropertyDescriptor(obj, k);
                        if (!d || !('value' in d)) return;
                        var v = d.value;
                        if (nat(v) && !(v.prototype && v.prototype.constructor === v && /^[A-Z]/.test(k) && path === '')) out.push(path + k);
                        else if (depth < 1 && v && typeof v === 'object' && k !== 'globalThis' && k !== 'global' && k !== 'GLOBAL' && k !== '__requireCache') scan(v, path + k + '.', depth + 1);
                    });
                };
                scan(globalThis, '', 0);
                return out.join('\n');
            })()"#,
        )
        .await
        .unwrap();
    println!("{out}");
}

#[tokio::test]
#[ignore]
async fn measure_snapshot_startup() {
    set_boot_mode(BootMode::Record);
    let engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    set_boot_mode(BootMode::Normal);
    drop(engine);
    let scripts = take_recorded();
    vvva_js::ensure_v8_initialized();

    // Build: replay what runs without natives.
    let t = std::time::Instant::now();
    let mut creator = v8::Isolate::snapshot_creator(None, None);
    let mut kept = 0;
    {
        v8::scope!(let scope, &mut creator);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        for (_, src) in &scripts {
            v8::tc_scope!(let tc, scope);
            let source = v8::String::new(tc, src).unwrap();
            if v8::Script::compile(tc, source, None)
                .and_then(|s| s.run(tc))
                .is_some()
            {
                kept += 1;
            }
        }
        scope.set_default_context(context);
    }
    let blob = creator.create_blob(v8::FunctionCodeHandling::Keep).unwrap();
    let bytes: Vec<u8> = blob.to_vec();
    println!(
        "built snapshot of {kept} scripts: {} KB in {:?}",
        bytes.len() / 1024,
        t.elapsed()
    );

    for _ in 0..5 {
        let data = bytes.clone();
        let t = std::time::Instant::now();
        let isolate = &mut v8::Isolate::new(v8::CreateParams::default().snapshot_blob(data.into()));
        let t_iso = t.elapsed();
        v8::scope!(let scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let t_ctx = t.elapsed();
        let src =
            v8::String::new(scope, "typeof require + typeof Buffer + typeof TextEncoder").unwrap();
        let r = v8::Script::compile(scope, src, None)
            .unwrap()
            .run(scope)
            .unwrap();
        println!(
            "isolate {:?} +context {:?} -> {}",
            t_iso,
            t_ctx,
            r.to_rust_string_lossy(scope)
        );
    }
    for _ in 0..3 {
        let t = std::time::Instant::now();
        let isolate = &mut v8::Isolate::new(Default::default());
        let t_iso = t.elapsed();
        v8::scope!(let scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let t_ctx = t.elapsed();
        for (_, src) in &scripts {
            v8::tc_scope!(let tc, scope);
            let source = v8::String::new(tc, src).unwrap();
            let _ = v8::Script::compile(tc, source, None).and_then(|s| s.run(tc));
        }
        println!(
            "baseline isolate {:?} context {:?} +scripts(no code cache) {:?}",
            t_iso,
            t_ctx,
            t.elapsed()
        );
    }
}

/// Replays the bootstrap with every native global and every Rust-provided
/// `process` property replaced by a logging accessor, and prints which of
/// them the scripts touch while loading (each one is a snapshot hazard).
#[tokio::test]
#[ignore]
async fn load_time_native_access() {
    set_boot_mode(BootMode::Record);
    let mut engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    set_boot_mode(BootMode::Normal);
    let natives = engine
        .eval_to_string(
            r#"Object.getOwnPropertyNames(globalThis).filter(function (k) {
                var d = Object.getOwnPropertyDescriptor(globalThis, k);
                return d && typeof d.value === 'function' && k.indexOf('__') === 0 &&
                    /\{\s*\[native code\]\s*\}$/.test(Function.prototype.toString.call(d.value));
            }).join(',')"#,
        )
        .await
        .unwrap();
    drop(engine);
    let scripts = take_recorded();
    let process_props = "pid,versions,config,features,argv,execArgv,env,stdout,stderr,umask,exit,cwd,chdir,memoryUsage,cpuUsage,platform,arch,version,execPath,title,release,hrtime,uptime";
    let prelude = format!(
        r#"(function () {{
            var log = globalThis.__snapLog = [];
            {natives:?}.split(',').forEach(function (name) {{
                Object.defineProperty(globalThis, name, {{ configurable: true,
                    get: function () {{ log.push(name); return function () {{ throw new Error('native ' + name + ' called at load'); }}; }},
                    set: function (v) {{ Object.defineProperty(globalThis, name, {{ value: v, writable: true, configurable: true }}); }} }});
            }});
            var p = {{}};
            {process_props:?}.split(',').forEach(function (name) {{
                Object.defineProperty(p, name, {{ configurable: true, enumerable: true,
                    get: function () {{ log.push('process.' + name); return name === 'env' || name === 'versions' || name === 'config' || name === 'features' ? {{}} : (name === 'argv' || name === 'execArgv' ? [] : undefined); }},
                    set: function (v) {{ Object.defineProperty(p, name, {{ value: v, writable: true, configurable: true, enumerable: true }}); }} }});
            }});
            globalThis.process = p;
            // Build-time constants the real snapshot prelude provides.
            Object.defineProperty(globalThis, '__osPlatform', {{ value: function () {{ return 'linux'; }}, writable: true, configurable: true }});
            Object.defineProperty(globalThis, '__cryptoFips', {{ value: false, writable: true, configurable: true }});
        }})();"#
    );

    vvva_js::ensure_v8_initialized();
    let isolate = &mut v8::Isolate::new(Default::default());
    v8::scope!(let scope, isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let run = |scope: &mut v8::ContextScope<v8::HandleScope>, src: &str| -> Result<(), String> {
        v8::tc_scope!(let tc, scope);
        let source = v8::String::new(tc, src).unwrap();
        match v8::Script::compile(tc, source, None).and_then(|s| s.run(tc)) {
            Some(_) => Ok(()),
            None => Err(tc
                .exception()
                .map(|e| e.to_rust_string_lossy(tc))
                .unwrap_or_default()),
        }
    };
    run(scope, &prelude).unwrap();
    let mut seen = 0usize;
    for (name, src) in &scripts {
        let r = run(scope, src);
        let log = {
            v8::tc_scope!(let tc, scope);
            let s = v8::String::new(tc, "__snapLog.join(',')").unwrap();
            v8::Script::compile(tc, s, None)
                .unwrap()
                .run(tc)
                .unwrap()
                .to_rust_string_lossy(tc)
        };
        let all: Vec<&str> = log.split(',').filter(|s| !s.is_empty()).collect();
        let new: Vec<&str> = all[seen.min(all.len())..].to_vec();
        seen = all.len();
        let mut uniq = new.clone();
        uniq.sort();
        uniq.dedup();
        println!(
            "{:<8} {name}: {}",
            if r.is_ok() { "ok" } else { "FAIL" },
            uniq.join(" ")
        );
        if let Err(e) = r {
            println!("         -> {e}");
        }
    }
}

#[tokio::test]
#[ignore]
async fn dump_bootstrap_scripts() {
    set_boot_mode(BootMode::Record);
    let engine = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();
    set_boot_mode(BootMode::Normal);
    drop(engine);
    let dir = std::env::var("SNAP_DUMP_DIR").unwrap_or_else(|_| "/tmp/3va-bootstrap".into());
    std::fs::create_dir_all(&dir).unwrap();
    for (i, (name, src)) in take_recorded().iter().enumerate() {
        std::fs::write(format!("{dir}/{i:02}-{name}.js"), src).unwrap();
    }
}
