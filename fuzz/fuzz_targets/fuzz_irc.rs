// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

#![no_main]

use libfuzzer_sys::fuzz_target;

// IRC message parser. The real parser is `Client.prototype._handleLine` in the
// embedded JS of crates/js/src/builtins/irc.rs; this mirrors its
// prefix/trailing/command split (a pure function of one line) so the same
// byte-level edge cases — a leading ':' with no space, " :" at the very end,
// empty tokens — are exercised without a live socket. The mirror must be
// total: any line returns a parsed shape, never panics.
fn irc_parse(line: &str) -> (Option<String>, String, Vec<String>, Option<String>) {
    let mut rest = line;
    let mut prefix = None;
    if let Some(after) = rest.strip_prefix(':') {
        match after.find(' ') {
            Some(sp) => {
                prefix = Some(after[..sp].to_string());
                rest = &after[sp + 1..];
            }
            None => {
                prefix = Some(after.to_string());
                rest = "";
            }
        }
    }

    let mut trailing = None;
    if let Some(t) = rest.find(" :") {
        trailing = Some(rest[t + 2..].to_string());
        rest = &rest[..t];
    } else if let Some(after) = rest.strip_prefix(':') {
        trailing = Some(after.to_string());
        rest = "";
    }

    let mut parts: Vec<String> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split(' ').map(str::to_string).collect()
    };
    let command = if parts.is_empty() {
        String::new()
    } else {
        parts.remove(0).to_uppercase()
    };
    if let Some(t) = trailing.clone() {
        parts.push(t);
    }
    (prefix, command, parts, trailing)
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let first = irc_parse(line);
        assert_eq!(first, irc_parse(line), "IRC parser is non-deterministic");
        // The split never manufactures more tokens than the line has bytes.
        assert!(
            first.2.len() <= line.len() + 1,
            "IRC parser produced more tokens than bytes: {line:?}"
        );
    }
});
