// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! V8 startup snapshot of the engine's bootstrap JS.
//!
//! Building the builtins (≈500 KB of bootstrap scripts) costs ~4 ms of every
//! `3va run`. A snapshot of a context that already ran them deserializes in
//! ~0.5 ms instead. Native functions can't be serialized (their callback data
//! points into this process), so the snapshot holds only what the scripts
//! built; each run then installs the natives and per-process values on top
//! (`BootMode::NativesOnly`, see `builtins::code_cache`). Scripts that read
//! per-process state while loading run on every start instead
//! (`bootstrap_js_per_run`).
//!
//! The first engine created with snapshots enabled records the bootstrap
//! scripts as it runs them and builds the snapshot on a background thread;
//! it is cached under `~/.cache/3va/snapshot/`, keyed by the running
//! executable (path, size, mtime), 3va's version and V8's cached-data tag,
//! so a rebuilt or upgraded binary never loads a stale one. Same trust
//! model as the bytecode cache next to it. Any failure falls back to the
//! normal start.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// Index of the builtins context in the snapshot (`Context::from_snapshot`).
pub const BUILTINS_CONTEXT: usize = 0;

static ENABLED: AtomicBool = AtomicBool::new(false);
static BUILDING: AtomicBool = AtomicBool::new(false);
static BUILD_THREAD: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

/// Lets engines created after this call start from (and build) the snapshot.
/// `VVVA_NO_SNAPSHOT=1` keeps them on the normal path.
pub fn enable() {
    if std::env::var_os("VVVA_NO_SNAPSHOT").is_none() {
        ENABLED.store(true, Ordering::Relaxed);
    }
}

/// Also on when `VVVA_SNAPSHOT=1` (how the test suite exercises it).
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
        || (std::env::var_os("VVVA_SNAPSHOT").is_some_and(|v| v == "1")
            && std::env::var_os("VVVA_NO_SNAPSHOT").is_none())
}

fn cache_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let exe = std::env::current_exe().ok()?;
    let meta = std::fs::metadata(&exe).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_PKG_VERSION").hash(&mut h);
    exe.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok().hash(&mut h);
    v8::script_compiler::cached_data_version_tag().hash(&mut h);
    crate::builtins::tls::FIPS.hash(&mut h);
    Some(
        PathBuf::from(home)
            .join(".cache")
            .join("3va")
            .join("snapshot")
            .join(format!("{:016x}.bin", h.finish())),
    )
}

/// The cached snapshot for this executable, if one has been built.
pub fn load() -> Option<Vec<u8>> {
    if !enabled() {
        return None;
    }
    std::fs::read(cache_file()?).ok()
}

/// Claims the one build this process may start. False when snapshots are
/// off, a build is already running, or there is nowhere to store it.
pub fn should_record() -> bool {
    enabled() && cache_file().is_some() && !BUILDING.swap(true, Ordering::AcqRel)
}

/// Builds and stores the snapshot from the recorded bootstrap scripts on a
/// background thread (its own isolate). See [`wait_for_build`].
pub fn build_in_background(scripts: Vec<(String, String)>) {
    let Some(path) = cache_file() else {
        return;
    };
    let handle = std::thread::spawn(move || {
        if let Ok(blob) = build(&scripts) {
            store(&path, &blob);
        }
    });
    *BUILD_THREAD.lock().unwrap() = Some(handle);
}

/// Waits for a background build this process started, if any. `3va run`
/// calls it before exiting, so the first run leaves a snapshot behind even
/// when the script itself finishes in a millisecond.
pub fn wait_for_build() {
    let handle = BUILD_THREAD.lock().unwrap().take();
    if let Some(h) = handle {
        let _ = h.join();
    }
}

fn store(path: &std::path::Path, blob: &[u8]) {
    let Some(dir) = path.parent() else {
        return;
    };
    let _ = std::fs::create_dir_all(dir);
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    if std::fs::write(&tmp, blob).is_ok() {
        if std::fs::rename(&tmp, path).is_ok() {
            prune(dir, KEEP_SNAPSHOTS);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// How many snapshots stay in the cache directory. Each one belongs to a
/// single executable (path, size, mtime), so every rebuild or upgrade leaves
/// the previous one behind; a few are kept so that two installed versions used
/// in turn don't rebuild each other's on every run.
const KEEP_SNAPSHOTS: usize = 4;

/// Keeps the `keep` most recently written snapshots in `dir` and removes the
/// rest, plus temporary files of builds that were interrupted over a day ago.
/// Best effort: a file that can't be removed is left for the next time, and a
/// process that loses a snapshot to this simply builds it again.
fn prune(dir: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    let now = std::time::SystemTime::now();
    let mut snapshots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        match path.extension().and_then(|e| e.to_str()) {
            Some("bin") => snapshots.push((modified, path)),
            Some(ext)
                if ext.starts_with("tmp-")
                    && now.duration_since(modified).is_ok_and(|age| age > day) =>
            {
                let _ = std::fs::remove_file(&path);
            }
            _ => {}
        }
    }
    snapshots.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in snapshots.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
}

/// What the bootstrap scripts expect to exist while loading that the
/// natives-installing Rust code would otherwise provide: build-time
/// constants, and the `process` object they decorate (filled in per run).
fn prelude() -> String {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    format!(
        "globalThis.__osPlatform = function () {{ return {platform:?}; }};\
         globalThis.__cryptoFips = {fips};\
         globalThis.process = {{ argv: [], env: {{}}, stdout: {{ fd: 1 }}, stderr: {{ fd: 2 }} }};",
        fips = crate::builtins::tls::FIPS,
    )
}

/// Runs `scripts` (with the prelude) in a fresh snapshot-creator isolate and
/// returns the serialized startup snapshot.
pub fn build(scripts: &[(String, String)]) -> anyhow::Result<Vec<u8>> {
    crate::ensure_v8_initialized();
    let mut creator = v8::Isolate::snapshot_creator(None, None);
    {
        v8::scope!(let scope, &mut creator);
        // The default context stays clean: `v8::Context::new` (vm contexts,
        // test262 realms) must not inherit 3va's globals. The builtins
        // context is added separately and loaded by index (BUILTINS_CONTEXT).
        let clean = v8::Context::new(scope, Default::default());
        scope.set_default_context(clean);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let prelude = prelude();
        for (name, src) in std::iter::once(("snapshot-prelude", prelude.as_str()))
            .chain(scripts.iter().map(|(n, s)| (n.as_str(), s.as_str())))
        {
            v8::tc_scope!(let tc, scope);
            let source = v8::String::new(tc, src)
                .ok_or_else(|| anyhow::anyhow!("snapshot: source too large: {name}"))?;
            if v8::Script::compile(tc, source, None)
                .and_then(|s| s.run(tc))
                .is_none()
            {
                let msg = tc
                    .exception()
                    .map(|e| e.to_rust_string_lossy(tc))
                    .unwrap_or_default();
                anyhow::bail!("snapshot: {name} failed while loading: {msg}");
            }
        }
        let index = scope.add_context(context);
        anyhow::ensure!(
            index == BUILTINS_CONTEXT,
            "snapshot: unexpected context index {index}"
        );
    }
    let blob = creator
        .create_blob(v8::FunctionCodeHandling::Clear)
        .ok_or_else(|| anyhow::anyhow!("snapshot: V8 could not serialize the context"))?;
    Ok(blob.to_vec())
}

#[cfg(test)]
mod tests {
    use super::prune;
    use std::time::{Duration, SystemTime};

    fn touch(dir: &std::path::Path, name: &str, age: Duration) {
        let f = std::fs::File::create(dir.join(name)).unwrap();
        f.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn prune_keeps_the_newest_snapshots_and_drops_stale_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for (i, name) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
            // "f" is the oldest, "a" the newest.
            touch(
                d,
                &format!("{name}.bin"),
                Duration::from_secs(60 * (i as u64 + 1)),
            );
        }
        touch(d, "old.tmp-123", Duration::from_secs(3 * 24 * 60 * 60));
        touch(d, "fresh.tmp-456", Duration::from_secs(5));
        touch(d, "notes.txt", Duration::from_secs(10 * 24 * 60 * 60));

        prune(d, 4);

        let mut left: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "a.bin",
                "b.bin",
                "c.bin",
                "d.bin",
                "fresh.tmp-456", // a build may be running right now
                "notes.txt",     // not ours: never touched
            ]
        );
    }

    #[test]
    fn prune_on_a_missing_directory_is_a_no_op() {
        prune(std::path::Path::new("/nonexistent/3va-snapshot-test"), 4);
    }
}
