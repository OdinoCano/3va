// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

#![no_main]

use libfuzzer_sys::fuzz_target;

// FTP control-reply parser. The real one is `Client.prototype._handleLine` in
// the embedded JS of crates/js/src/builtins/ftp.rs, matching
// `/^(\d{3})([ -])(.*)$/`. This mirrors that shape (3 digits + separator +
// text) and the 4xx/5xx-is-error rule; it must classify any line without
// panicking, and a non-match must never be mistaken for a reply.
#[derive(Debug, PartialEq, Eq)]
struct Reply {
    code: String,
    /// `-` starts a multi-line reply; ` ` is a single-line reply.
    multiline: bool,
    text: String,
    is_error: bool,
}

fn parse_reply(line: &str) -> Option<Reply> {
    let bytes = line.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    if !bytes[..3].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let sep = bytes[3];
    if sep != b' ' && sep != b'-' {
        return None;
    }
    let code = line[..3].to_string();
    let is_error = code.starts_with('4') || code.starts_with('5');
    Some(Reply {
        code,
        multiline: sep == b'-',
        text: line[4..].to_string(),
        is_error,
    })
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        let a = parse_reply(line);
        assert_eq!(
            a,
            parse_reply(line),
            "FTP reply parser is non-deterministic"
        );
        if let Some(r) = &a {
            assert_eq!(r.code.len(), 3, "FTP reply code must be 3 digits");
            assert!(
                r.code.chars().all(|c| c.is_ascii_digit()),
                "FTP reply code must be numeric"
            );
            assert_eq!(
                r.is_error,
                r.code.starts_with('4') || r.code.starts_with('5'),
                "FTP 4xx/5xx must be an error"
            );
        }
    }
});
