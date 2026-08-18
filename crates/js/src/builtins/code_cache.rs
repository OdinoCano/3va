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
use v8::{ContextScope, HandleScope};

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

/// Compiles and runs `source` in `scope`, transparently reading/writing a
/// per-source-hash code cache under `~/.cache/3va/codecache/`. Falls back to
/// a plain compile on any cache miss/read/write failure — this is a pure
/// speed optimization, never a correctness dependency.
pub fn compile_and_run_cached(
    scope: &mut ContextScope<HandleScope>,
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
