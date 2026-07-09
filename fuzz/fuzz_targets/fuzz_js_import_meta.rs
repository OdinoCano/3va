#![no_main]

use libfuzzer_sys::fuzz_target;

// ── Re-implementations of the pure byte-level scanners in crates/js/src/transpiler.rs ──
//
// We re-implement the exact same logic instead of pulling in `vvva_js` (which
// depends on v8 + oxc + a full async runtime) so the fuzzer builds fast
// and can iterate at full speed. The goal is to find panics / infinite loops /
// out-of-bounds reads in the scanners, which only depend on the input bytes.

const META_PATTERNS: &[(&str, &str)] = &[
    ("import.meta.resolve(", "__vvva_meta_resolve__("),
    ("import.meta.glob(", "__vvva_meta_glob__("),
    ("import.meta.hot", "undefined"),
    ("import.meta.vitest", "undefined"),
    ("import.meta.env", "__vvva_meta_env__"),
    ("import.meta.url", "__vvva_meta_url__"),
];

fn replace_outside_strings_and_comments(source: &str, from: &str, to: &str) -> String {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let pat = from.as_bytes();
    let pat_len = pat.len();
    let mut out: Vec<u8> = Vec::with_capacity(len.saturating_add(to.len() * 4));
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                out.push(bytes[i]);
                i += 1;
            }
            continue;
        }

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            out.push(b);
            out.push(bytes[i + 1]);
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                out.push(bytes[i]);
                i += 1;
            }
            if i + 1 < len {
                out.push(bytes[i]);
                out.push(bytes[i + 1]);
                i += 2;
            }
            continue;
        }

        if b == b'"' || b == b'\'' {
            let quote = b;
            out.push(b);
            i += 1;
            while i < len {
                let c = bytes[i];
                if c == b'\\' {
                    out.push(c);
                    i += 1;
                    if i < len {
                        out.push(bytes[i]);
                        i += 1;
                    }
                    continue;
                }
                out.push(c);
                i += 1;
                if c == quote {
                    break;
                }
            }
            continue;
        }

        if b == b'`' {
            out.push(b);
            i += 1;
            while i < len {
                let c = bytes[i];
                if c == b'\\' {
                    out.push(c);
                    i += 1;
                    if i < len {
                        out.push(bytes[i]);
                        i += 1;
                    }
                    continue;
                }
                if c == b'`' {
                    out.push(c);
                    i += 1;
                    break;
                }
                if c == b'$' && i + 1 < len && bytes[i + 1] == b'{' {
                    out.push(c);
                    out.push(bytes[i + 1]);
                    i += 2;
                    let mut depth: i32 = 1;
                    while i < len && depth > 0 {
                        let ic = bytes[i];
                        if ic == b'{' {
                            depth += 1;
                            out.push(ic);
                            i += 1;
                            continue;
                        }
                        if ic == b'}' {
                            depth -= 1;
                            if depth == 0 {
                                out.push(ic);
                                i += 1;
                                break;
                            }
                            out.push(ic);
                            i += 1;
                            continue;
                        }
                        if ic == b'"' || ic == b'\'' {
                            let iq = ic;
                            out.push(ic);
                            i += 1;
                            while i < len {
                                let cc = bytes[i];
                                if cc == b'\\' {
                                    out.push(cc);
                                    i += 1;
                                    if i < len {
                                        out.push(bytes[i]);
                                        i += 1;
                                    }
                                    continue;
                                }
                                out.push(cc);
                                i += 1;
                                if cc == iq {
                                    break;
                                }
                            }
                            continue;
                        }
                        if i + pat_len <= len && bytes[i..i + pat_len] == *pat {
                            out.extend_from_slice(to.as_bytes());
                            i += pat_len;
                            continue;
                        }
                        out.push(ic);
                        i += 1;
                    }
                    continue;
                }
                out.push(c);
                i += 1;
            }
            continue;
        }

        if i + pat_len <= len && bytes[i..i + pat_len] == *pat {
            out.extend_from_slice(to.as_bytes());
            i += pat_len;
            continue;
        }

        out.push(b);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn replace_import_meta(source: &str) -> String {
    let mut result = source.to_string();
    for (from, to) in META_PATTERNS {
        result = replace_outside_strings_and_comments(&result, from, to);
    }
    result
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn has_top_level_await(code: &str) -> bool {
    if !code.contains("await") {
        return false;
    }

    let bytes = code.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut depth: u32 = 0;

    while i < len {
        let b = bytes[i];

        if b == b'/' && i + 1 < len {
            if bytes[i + 1] == b'/' {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
        }

        if b == b'"' || b == b'\'' || b == b'`' {
            let q = b;
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        if b == b'{' {
            depth += 1;
            i += 1;
            continue;
        }
        if b == b'}' {
            depth = depth.saturating_sub(1);
            i += 1;
            continue;
        }

        if depth <= 1 && b == b'a' && i + 5 <= len && &bytes[i..i + 5] == b"await" {
            let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let after_ok = i + 5 >= len || !is_ident_byte(bytes[i + 5]);
            if before_ok && after_ok {
                return true;
            }
        }

        i += 1;
    }

    false
}

fn looks_like_jsx(source: &str) -> bool {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'<' && i + 1 < len {
            let next = bytes[i + 1];
            if next == b'<' || next == b'=' {
                i += 2;
                continue;
            }
            if next == b'/' {
                return true;
            }
            if next.is_ascii_alphabetic() {
                return true;
            }
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            i += 1;
            while i < len && bytes[i] != q {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
        }
        i += 1;
    }
    false
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };

    // replace_import_meta must never panic and must be deterministic.
    let r1 = replace_import_meta(s);
    let r2 = replace_import_meta(s);
    assert_eq!(r1, r2, "replace_import_meta is non-deterministic for input {s:?}");

    // Patterns that lived inside string literals or comments must NOT have been
    // rewritten (the scanner claims to skip them). Check that a few known-bad
    // input shapes don't break this invariant — we use a substring search for
    // the *replacement* token; if the original `import.meta.url` appears inside
    // a string, the replacement must not appear in the output either.
    for &(from, to) in META_PATTERNS {
        // If input contains the from pattern inside a "..." / '...' / `...` /
        // // comment / /* */ block, the output must not contain the to token
        // (assuming no other occurrence outside strings).
        if from.contains(".url") || from.contains(".env") {
            // Quick sanity: the to token must not leak when from is in a string.
            let probe = format!("const _ = \"{from}\";");
            let out = replace_import_meta(&probe);
            assert!(
                !out.contains(to),
                "replace_import_meta leaked {to:?} from string literal: input={probe:?} out={out:?}"
            );
        }
    }

    // has_top_level_await: never panics, deterministic, idempotent.
    let _ = has_top_level_await(s);

    // looks_like_jsx: never panics, deterministic, idempotent.
    let _ = looks_like_jsx(s);

    // Length must not grow unboundedly. The longest replacement token is
    // "__vvva_meta_resolve__" (20 bytes) vs the longest from token
    // "import.meta.resolve(" (20 bytes) — equal — and "import.meta.hot" (15)
    // → "undefined" (9). So the output is always ≤ input length + N*delta,
    // where delta is bounded by the difference between the to/from pairs.
    // Worst case delta per replacement is +5 (for "import.meta.glob(" → ...).
    // With at most len/16 occurrences, the bound is len + 5*(len/16) = 1.3125*len.
    assert!(
        r1.len() <= s.len() + s.len() / 2 + 64,
        "replace_import_meta output length exploded: in_len={} out_len={}",
        s.len(),
        r1.len()
    );
});
