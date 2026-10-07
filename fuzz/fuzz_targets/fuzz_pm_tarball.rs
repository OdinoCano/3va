// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

#![no_main]

use libfuzzer_sys::fuzz_target;
use vvva_pm::store::ContentStore;

// Drives the real tarball extractor used at install time
// (`ContentStore::store_tarball` → `extract_tarball`) over arbitrary bytes: it
// must reject path traversal / absolute entries / symlinks / hardlinks, and
// stop decompression bombs (per-file, total and entry-count caps) by returning
// an error — never panic and never write outside the store root.
fuzz_target!(|data: &[u8]| {
    let Ok(tmp) = tempfile::tempdir() else {
        return;
    };
    let store = ContentStore::with_root(tmp.path().join("store"));
    let _ = store.store_tarball(data, "npm", "fuzzpkg", "0.0.0");
});
