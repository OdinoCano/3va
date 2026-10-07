// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

#![no_main]

use libfuzzer_sys::fuzz_target;

// POP3 response classifier. The real one is `Client.prototype._handleLine` in
// the embedded JS of crates/js/src/builtins/pop3.rs; this mirrors its
// status/dot-unstuffing rules over a fuzz-derived line. Any line must classify
// into exactly one bucket without panicking.
#[derive(Debug, PartialEq, Eq)]
enum Pop3Line {
    /// `+OK …` or `+ …` — a positive status line.
    Ok,
    /// `-ERR …` — a negative status line.
    Err,
    /// A body line in a multiline response, already dot-unstuffed.
    Data(String),
    /// The lone `.` that ends a multiline response.
    End,
}

fn classify(line: &str) -> Pop3Line {
    if line.starts_with("+OK") || line.starts_with("+ ") {
        Pop3Line::Ok
    } else if line.starts_with("-ERR") {
        Pop3Line::Err
    } else if line == "." {
        Pop3Line::End
    } else if line.starts_with("..") {
        // Byte-stuffing: a leading ".." is one literal '.'.
        Pop3Line::Data(line[1..].to_string())
    } else {
        Pop3Line::Data(line.to_string())
    }
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        let a = classify(line);
        assert_eq!(a, classify(line), "POP3 classifier is non-deterministic");

        // Un-stuffing removes exactly one leading dot.
        if let Pop3Line::Data(body) = &a
            && line.starts_with("..")
        {
            assert_eq!(
                body.as_str(),
                &line[1..],
                "POP3 dot-unstuffing must drop exactly one leading dot"
            );
        }
    }
});
