// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! On-disk V8 code cache for the (large, unchanging) JS bootstrap strings
//! injected on every engine start. Compiling e.g. `modules::inject_require`'s
//! multi-thousand-line `require()`/`vm`/`cluster` polyfill from source costs
//! several milliseconds — on every single `3va run`, whether the script is
//! `console.log("hi")` or a full server. V8 can serialize the parsed
//! bytecode and skip straight to it on the next run, the same trick Node
//! uses for its own builtins.
//!
//! Keyed by a hash of the source text itself plus V8's own cached-data
//! version tag, so a source change or a V8/3va upgrade invalidates the
//! cache automatically (V8 also independently rejects a stale cache via
//! `CachedData::rejected()`, checked below as a second line of defense).

use std::hash::{Hash, Hasher};
use v8::script_compiler::{self, CompileOptions, NoCacheReason};

fn cache_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join(".cache")
            .join("3va")
            .join("codecache"),
    )
}

fn cache_path(name: &str, source: &str) -> Option<std::path::PathBuf> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    script_compiler::cached_data_version_tag().hash(&mut hasher);
    let digest = hasher.finish();
    Some(cache_dir()?.join(format!("{name}-{digest:016x}.v8cache")))
}

/// How engine bootstrap JS runs (see [`bootstrap_js`]). Per thread, like
/// the isolate that runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootMode {
    /// Run every bootstrap script (the normal engine start).
    Normal,
    /// Run them and record `(name, source)` in order, to replay later.
    Record,
    /// Skip them: the context already contains their effects.
    NativesOnly,
}

thread_local! {
    static BOOT_MODE: std::cell::Cell<BootMode> = const { std::cell::Cell::new(BootMode::Normal) };
    static RECORDED: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Sets this thread's bootstrap mode and returns the previous one.
pub fn set_boot_mode(mode: BootMode) -> BootMode {
    BOOT_MODE.with(|m| m.replace(mode))
}

/// The scripts recorded under [`BootMode::Record`], in execution order.
pub fn take_recorded() -> Vec<(String, String)> {
    RECORDED.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Runs one piece of engine bootstrap JS. Every script the builtins run
/// while the engine is created goes through here, so the set can be
/// recorded and replayed (or skipped) as a unit. Large sources use the
/// on-disk code cache; small ones just compile.
pub fn bootstrap_js(scope: &mut v8::PinScope, name: &str, source: &str) -> anyhow::Result<()> {
    match BOOT_MODE.with(|m| m.get()) {
        BootMode::NativesOnly => return Ok(()),
        BootMode::Record => {
            RECORDED.with(|r| r.borrow_mut().push((name.to_string(), source.to_string())));
        }
        BootMode::Normal => {}
    }
    if source.len() >= 4096 {
        return compile_and_run_cached(scope, name, source);
    }
    let src = v8::String::new(scope, source)
        .ok_or_else(|| anyhow::anyhow!("bootstrap source too large: {name}"))?;
    let script = v8::Script::compile(scope, src, None)
        .ok_or_else(|| anyhow::anyhow!("compile error in {name}"))?;
    script
        .run(scope)
        .ok_or_else(|| anyhow::anyhow!("execution error in {name}"))?;
    Ok(())
}

/// Like [`bootstrap_js`], for a script that must run on every engine start
/// and never be part of a startup snapshot: one that reads per-process state
/// (environment, argv) or holds a native function by value. It runs in every
/// mode, including [`BootMode::NativesOnly`], and is not recorded.
pub fn bootstrap_js_per_run(
    scope: &mut v8::PinScope,
    name: &str,
    source: &str,
) -> anyhow::Result<()> {
    let src = v8::String::new(scope, source)
        .ok_or_else(|| anyhow::anyhow!("bootstrap source too large: {name}"))?;
    let script = v8::Script::compile(scope, src, None)
        .ok_or_else(|| anyhow::anyhow!("compile error in {name}"))?;
    script
        .run(scope)
        .ok_or_else(|| anyhow::anyhow!("execution error in {name}"))?;
    Ok(())
}

/// The current thread's bootstrap mode.
pub fn boot_mode() -> BootMode {
    BOOT_MODE.with(|m| m.get())
}

/// Compiles and runs `source` in `scope`, transparently reading/writing a
/// per-source-hash code cache under `~/.cache/3va/codecache/`. Falls back to
/// a plain compile on any cache miss/read/write failure — this is a pure
/// speed optimization, never a correctness dependency.
pub fn compile_and_run_cached(
    scope: &mut v8::PinScope,
    name: &str,
    source: &str,
) -> anyhow::Result<()> {
    let path = cache_path(name, source);
    let src = v8::String::new(scope, source).unwrap();

    if let Some(path) = &path
        && let Ok(bytes) = std::fs::read(path)
    {
        let cached_data = v8::script_compiler::CachedData::new(&bytes);
        let mut src_obj = script_compiler::Source::new_with_cached_data(src, None, cached_data);
        if let Some(unbound) = script_compiler::compile_unbound_script(
            scope,
            &mut src_obj,
            CompileOptions::ConsumeCodeCache,
            NoCacheReason::NoReason,
        ) {
            let accepted = src_obj.get_cached_data().is_some_and(|cd| !cd.rejected());
            if accepted {
                let script = unbound.bind_to_current_context(scope);
                script
                    .run(scope)
                    .ok_or_else(|| anyhow::anyhow!("execution error in {name}"))?;
                return Ok(());
            }
        }
        // Cache was stale/corrupt/rejected — fall through and recompile
        // fresh below, re-creating `src` since it was consumed above.
    }

    let src = v8::String::new(scope, source).unwrap();
    let mut src_obj = script_compiler::Source::new(src, None);
    let unbound = script_compiler::compile_unbound_script(
        scope,
        &mut src_obj,
        CompileOptions::EagerCompile,
        NoCacheReason::NoReason,
    )
    .ok_or_else(|| anyhow::anyhow!("compile error in {name}"))?;

    if let Some(path) = &path
        && let Some(cache) = unbound.create_code_cache()
    {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, &**cache);
    }

    let script = unbound.bind_to_current_context(scope);
    script
        .run(scope)
        .ok_or_else(|| anyhow::anyhow!("execution error in {name}"))?;
    Ok(())
}
