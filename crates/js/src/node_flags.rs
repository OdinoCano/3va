// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Node-compatible command-line flags that change the runtime's behaviour:
//! `--no-warnings`, `--pending-deprecation`, `--expose-gc`,
//! `--zero-fill-buffers`, `--insecure-http-parser` and `--tls-min-v1.0`.
//!
//! Like `vvva_permissions::set_allow_insecure`, these are process-wide
//! switches set by the CLI before any script runs, and published to the
//! JavaScript side as `globalThis.__node_flags__`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

// ponytail: process-wide switches, set once by the CLI before any script runs.
static NO_WARNINGS: AtomicBool = AtomicBool::new(false);
static PENDING_DEPRECATION: AtomicBool = AtomicBool::new(false);
static ZERO_FILL_BUFFERS: AtomicBool = AtomicBool::new(false);
static INSECURE_HTTP_PARSER: AtomicBool = AtomicBool::new(false);
static EXPOSE_GC: AtomicBool = AtomicBool::new(false);

/// The runtime flags as they were written on the command line, in order.
/// Node exposes them as `process.execArgv`, and code (Node's own test suite
/// among others) reads that array to know how the process was started.
static EXEC_ARGV: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// The runtime flags given on the command line, in the order they were given.
pub fn exec_argv() -> Vec<String> {
    EXEC_ARGV
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default()
}

/// Records the runtime flags given on the command line (`process.execArgv`).
pub fn set_exec_argv(flags: Vec<String>) {
    let slot = EXEC_ARGV.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut v) = slot.lock() {
        *v = flags;
    }
}

/// `--no-warnings`: `process.emitWarning` does not emit anything.
pub fn set_no_warnings(value: bool) {
    NO_WARNINGS.store(value, Ordering::Relaxed);
}

/// True when `--no-warnings` was passed.
pub fn no_warnings() -> bool {
    NO_WARNINGS.load(Ordering::Relaxed)
}

/// `--pending-deprecation`: pending deprecation warnings are emitted.
pub fn set_pending_deprecation(value: bool) {
    PENDING_DEPRECATION.store(value, Ordering::Relaxed);
}

/// True when `--pending-deprecation` was passed.
pub fn pending_deprecation() -> bool {
    PENDING_DEPRECATION.load(Ordering::Relaxed)
}

/// `--zero-fill-buffers`: `Buffer.allocUnsafe()` returns zero-filled memory.
pub fn set_zero_fill_buffers(value: bool) {
    ZERO_FILL_BUFFERS.store(value, Ordering::Relaxed);
}

/// True when `--zero-fill-buffers` was passed.
pub fn zero_fill_buffers() -> bool {
    ZERO_FILL_BUFFERS.load(Ordering::Relaxed)
}

/// `--insecure-http-parser`: lenient HTTP header validation.
pub fn set_insecure_http_parser(value: bool) {
    INSECURE_HTTP_PARSER.store(value, Ordering::Relaxed);
}

/// True when `--insecure-http-parser` was passed.
pub fn insecure_http_parser() -> bool {
    INSECURE_HTTP_PARSER.load(Ordering::Relaxed)
}

/// `--expose-gc`: `globalThis.gc()` is available.
pub fn set_expose_gc(value: bool) {
    EXPOSE_GC.store(value, Ordering::Relaxed);
}

/// True when `--expose-gc` was passed.
pub fn expose_gc() -> bool {
    EXPOSE_GC.load(Ordering::Relaxed)
}

/// The flags as a JavaScript object literal, published as
/// `globalThis.__node_flags__` on every engine start.
pub fn js_flags_literal() -> String {
    let argv = exec_argv()
        .iter()
        .map(|f| format!("'{}'", f.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{ noWarnings: {no_warnings}, pendingDeprecation: {pending_deprecation}, \
           zeroFillBuffers: {zero_fill_buffers}, insecureHttpParser: {insecure_http_parser}, \
           execArgv: [{argv}] }}",
        no_warnings = no_warnings(),
        pending_deprecation = pending_deprecation(),
        zero_fill_buffers = zero_fill_buffers(),
        insecure_http_parser = insecure_http_parser(),
        argv = argv,
    )
}
