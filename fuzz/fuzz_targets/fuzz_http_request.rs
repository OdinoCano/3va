// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

#![no_main]

use libfuzzer_sys::fuzz_target;
use vvva_js::builtins::http_server::parse_request;

// Drives the real HTTP/1 request parser in crates/js/src/builtins/http_server.rs
// (the same framing code the server uses) over an in-memory slice. The whole
// point is the framing state machine: header floods, Content-Length /
// Transfer-Encoding conflicts, malformed chunked bodies and `../`-style
// request lines must all return a value, never panic or loop.
fuzz_target!(|data: &[u8]| {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("current-thread runtime");
    let _ = rt.block_on(async {
        let mut reader = tokio::io::BufReader::new(data);
        parse_request(
            &mut reader,
            std::time::Duration::from_millis(50),
            std::time::Duration::from_millis(50),
            64,       // max header count
            8 * 1024, // max header bytes
            1 << 20,  // max body bytes
            0,        // rate check off (no real time passes on a slice)
        )
        .await
    });
});
