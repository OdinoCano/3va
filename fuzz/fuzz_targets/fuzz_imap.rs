// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

#![no_main]

use libfuzzer_sys::fuzz_target;
use vvva_js::builtins::imap::parse_mailbox_list;

// Calls the real IMAP mailbox-list parser (`parse_mailbox_list` in
// crates/js/src/builtins/imap.rs) on fuzz-derived lines. It is the one IMAP
// response parser that lives in Rust rather than embedded JS; the input is
// split the way the socket read loop splits lines.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let lines: Vec<String> = text.split('\n').map(|l| l.to_string()).collect();

    let names = parse_mailbox_list(&lines);

    // Determinism: the same response must yield the same mailboxes.
    assert_eq!(
        names,
        parse_mailbox_list(&lines),
        "parse_mailbox_list is non-deterministic"
    );

    // No empty mailbox names ever escape the parser.
    assert!(
        names.iter().all(|n| !n.is_empty()),
        "parse_mailbox_list returned an empty name: {names:?}"
    );
});
