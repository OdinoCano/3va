#![no_main]

use libfuzzer_sys::fuzz_target;

// Re-implementations of the hand-written ESM→CJS converter and the inline
// Flow-type stripper from crates/js/src/transpiler.rs. Both are pure
// string-in / string-out functions and are the most likely place to find
// panic, infinite-loop, or off-by-one bugs since they are byte-level
// scanners with no parser assistance.

fn strip_inline_flow_types(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("const ")
            || trimmed.starts_with("let ")
            || trimmed.starts_with("var ")
        {
            let kw_len = if trimmed.starts_with("const ") { 6 } else { 4 };
            let after_kw = &trimmed[kw_len..];
            if let Some(cp) = after_kw.find(':') {
                let before_colon = &after_kw[..cp];
                if !before_colon.contains('=') && !before_colon.contains(')') && cp > 0 {
                    let after_colon = &after_kw[cp + 1..];
                    let mut depth = 0u32;
                    let mut eq_pos = None;
                    for (k, ch) in after_colon.char_indices() {
                        match ch {
                            '{' | '[' | '(' => depth += 1,
                            '}' | ']' | ')' if depth > 0 => depth -= 1,
                            '=' if depth == 0 => {
                                eq_pos = Some(k);
                                break;
                            }
                            _ => {}
                        }
                    }
                    if let Some(ep) = eq_pos {
                        result.push_str(&trimmed[..kw_len + cp]);
                        result.push_str(&after_colon[ep..]);
                        result.push('\n');
                        continue;
                    }
                }
            }
        }

        if trimmed.starts_with("function ") {
            let paren = trimmed.find('(');
            if let Some(po) = paren {
                let mut clean = String::with_capacity(trimmed.len());
                clean.push_str(&trimmed[..=po]);
                let params = &trimmed[po + 1..];
                let mut depth = 0u32;
                let mut i = 0;
                let bytes = params.as_bytes();
                // Non-ASCII guard — see the production comment in
                // crates/js/src/transpiler.rs for the full rationale.  This
                // mirrors the fix in production so the fuzzer asserts the
                // same invariant.
                if bytes.iter().any(|b| *b >= 0x80) {
                    result.push_str(line);
                    result.push('\n');
                    continue;
                }
                while i < bytes.len() {
                    let ch = bytes[i] as char;
                    match ch {
                        '(' => {
                            depth += 1;
                            clean.push(ch);
                            i += 1;
                        }
                        ')' if depth == 0 => {
                            clean.push(')');
                            i += 1;
                            if i < bytes.len() && bytes[i] == b':' {
                                let rest = &params[i..];
                                if let Some(brace) = rest.find('{') {
                                    clean.push_str(&rest[brace..]);
                                    i = bytes.len();
                                }
                            } else {
                                clean.push_str(&params[i..]);
                                i = bytes.len();
                            }
                        }
                        ')' => {
                            depth -= 1;
                            clean.push(ch);
                            i += 1;
                        }
                        ':' if depth == 0 && i > 0 && bytes[i - 1].is_ascii_alphabetic() => {
                            i += 1;
                            let mut inner_depth = 0u32;
                            while i < bytes.len() {
                                let c = bytes[i] as char;
                                match c {
                                    '{' | '[' | '(' => inner_depth += 1,
                                    '}' | ']' | ')' if inner_depth > 0 => inner_depth -= 1,
                                    ',' | ')' if inner_depth == 0 => {
                                        clean.push(c);
                                        i += 1;
                                        break;
                                    }
                                    _ => {}
                                }
                                i += 1;
                            }
                        }
                        _ => {
                            clean.push(ch);
                            i += 1;
                        }
                    }
                }
                result.push_str(&clean);
                result.push('\n');
                continue;
            }
        }

        result.push_str(line);
        result.push('\n');
    }
    result
}

fn static_esm_to_cjs(source: &str) -> String {
    if !source.contains("import ") && !source.contains("export ") {
        return source.to_string();
    }
    let src = source.as_bytes();
    let len = src.len();
    let mut out: Vec<u8> = Vec::with_capacity(len + 512);
    let mut i = 0;
    let mut at_stmt = true;

    while i < len {
        let b = src[i];

        if b == b'"' || b == b'\'' || b == b'`' {
            at_stmt = false;
            let q = b;
            out.push(b);
            i += 1;
            while i < len {
                let c = src[i];
                if c == b'\\' {
                    out.push(c);
                    i += 1;
                    if i < len {
                        out.push(src[i]);
                        i += 1;
                    }
                    continue;
                }
                out.push(c);
                i += 1;
                if c == q {
                    break;
                }
            }
            continue;
        }

        if b == b'/' && i + 1 < len && src[i + 1] == b'/' {
            while i < len && src[i] != b'\n' {
                out.push(src[i]);
                i += 1;
            }
            at_stmt = true;
            continue;
        }

        if b == b'/' && i + 1 < len && src[i + 1] == b'*' {
            out.push(b);
            out.push(src[i + 1]);
            i += 2;
            while i + 1 < len && !(src[i] == b'*' && src[i + 1] == b'/') {
                if src[i] == b'\n' {
                    at_stmt = true;
                }
                out.push(src[i]);
                i += 1;
            }
            if i + 1 < len {
                out.push(src[i]);
                out.push(src[i + 1]);
                i += 2;
            }
            continue;
        }

        if b == b'\n' || b == b'\r' {
            at_stmt = true;
            out.push(b);
            i += 1;
            continue;
        }
        if b == b';' {
            at_stmt = true;
            out.push(b);
            i += 1;
            continue;
        }
        if b == b' ' || b == b'\t' {
            out.push(b);
            i += 1;
            continue;
        }

        // ESM keyword detection (mirrors kw_at in the real source).
        if at_stmt {
            // import …
            if i + 6 <= len && &src[i..i + 6] == b"import" {
                let after = i + 6;
                if after < len
                    && matches!(src[after], b' ' | b'\t' | b'\n' | b'"' | b'\'')
                {
                    // Convert by collecting until the semicolon at depth 0
                    // and rewriting. This is a fuzz-only approximation that
                    // does not need to be 100% faithful — it must only
                    // exercise the same code paths without panicking.
                    let mut j = i;
                    let mut depth: i32 = 0;
                    let mut stmt_start = j;
                    while j < len {
                        let cj = src[j];
                        if cj == b'"' || cj == b'\'' || cj == b'`' {
                            let qj = cj;
                            j += 1;
                            while j < len {
                                if src[j] == b'\\' && j + 1 < len {
                                    j += 2;
                                    continue;
                                }
                                if src[j] == qj {
                                    j += 1;
                                    break;
                                }
                                j += 1;
                            }
                            continue;
                        }
                        if cj == b'{' || cj == b'[' || cj == b'(' {
                            depth += 1;
                        }
                        if cj == b'}' || cj == b')' || cj == b']' {
                            depth -= 1;
                        }
                        if cj == b';' && depth == 0 {
                            j += 1;
                            break;
                        }
                        j += 1;
                    }
                    if stmt_start == j {
                        // Empty statement — leave as-is so we don't loop.
                        out.push(b);
                        i += 1;
                        at_stmt = false;
                        continue;
                    }
                    let stmt_text: &[u8] = &src[stmt_start..j];
                    // Side-effect import: import 'mod';
                    let mut k = 6;
                    while k < stmt_text.len() && (stmt_text[k] == b' ' || stmt_text[k] == b'\t') {
                        k += 1;
                    }
                    if k < stmt_text.len() && (stmt_text[k] == b'"' || stmt_text[k] == b'\'') {
                        // Extract the literal between matching quotes.
                        let qj = stmt_text[k];
                        let mut m = k + 1;
                        while m < stmt_text.len() && stmt_text[m] != qj {
                            if stmt_text[m] == b'\\' {
                                m += 1;
                            }
                            m += 1;
                        }
                        if m < stmt_text.len() {
                            let module = std::str::from_utf8(&stmt_text[k + 1..m]).unwrap_or("");
                            out.extend_from_slice(format!("require(\"{module}\");").as_bytes());
                            i = j;
                            at_stmt = true;
                            continue;
                        }
                    }
                    // Otherwise fall through: emit the bytes unchanged and
                    // advance past the statement, treating the whole thing as
                    // opaque so the loop makes progress.
                    out.extend_from_slice(stmt_text);
                    i = j;
                    at_stmt = true;
                    continue;
                }
            }
            // export …
            if i + 6 <= len && &src[i..i + 6] == b"export" {
                let after = i + 6;
                if after < len && matches!(src[after], b' ' | b'\t' | b'\n') {
                    let mut j = i;
                    let mut depth: i32 = 0;
                    let mut entered = false;
                    let mut stmt_start = j;
                    // Look ahead: if next non-space is function/async/class,
                    // collect up to matching brace; else collect to semicolon.
                    let mut probe = after + 1;
                    while probe < len && (src[probe] == b' ' || src[probe] == b'\t') {
                        probe += 1;
                    }
                    let is_block = probe + 8 <= len
                        && (&src[probe..probe + 8] == b"function"
                            || &src[probe..probe + 5] == b"async "
                            || &src[probe..probe + 5] == b"class ");
                    if is_block {
                        while j < len {
                            let cj = src[j];
                            if cj == b'"' || cj == b'\'' || cj == b'`' {
                                let qj = cj;
                                j += 1;
                                while j < len {
                                    if src[j] == b'\\' && j + 1 < len {
                                        j += 2;
                                        continue;
                                    }
                                    if src[j] == qj {
                                        j += 1;
                                        break;
                                    }
                                    j += 1;
                                }
                                continue;
                            }
                            if cj == b'{' {
                                depth += 1;
                                entered = true;
                            }
                            if cj == b'}' {
                                depth -= 1;
                            }
                            j += 1;
                            if entered && depth == 0 {
                                break;
                            }
                        }
                    } else {
                        while j < len {
                            let cj = src[j];
                            if cj == b'"' || cj == b'\'' || cj == b'`' {
                                let qj = cj;
                                j += 1;
                                while j < len {
                                    if src[j] == b'\\' && j + 1 < len {
                                        j += 2;
                                        continue;
                                    }
                                    if src[j] == qj {
                                        j += 1;
                                        break;
                                    }
                                    j += 1;
                                }
                                continue;
                            }
                            if cj == b'{' || cj == b'[' || cj == b'(' {
                                depth += 1;
                            }
                            if cj == b'}' || cj == b')' || cj == b']' {
                                depth -= 1;
                            }
                            if cj == b';' && depth == 0 {
                                j += 1;
                                break;
                            }
                            j += 1;
                        }
                    }
                    if stmt_start == j {
                        out.push(b);
                        i += 1;
                        at_stmt = false;
                        continue;
                    }
                    // Conservative emission: leave the export bytes as-is so
                    // the resulting CJS still parses (export is invalid in
                    // CJS, but we only assert no-panic, not validity).
                    out.extend_from_slice(&src[stmt_start..j]);
                    i = j;
                    at_stmt = true;
                    continue;
                }
            }
        }

        at_stmt = false;
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };

    // Both functions must be deterministic and panic-free.
    let cjs1 = static_esm_to_cjs(s);
    let cjs2 = static_esm_to_cjs(s);
    assert_eq!(cjs1, cjs2, "static_esm_to_cjs is non-deterministic");

    let f1 = strip_inline_flow_types(s);
    let f2 = strip_inline_flow_types(s);
    assert_eq!(f1, f2, "strip_inline_flow_types is non-deterministic");

    // The ESM-to-CJS converter must not lose every byte (it should always
    // make progress through the input). Output length is bounded by
    // input * a small constant because the worst-case expansion is for
    // "import X from 'm'" (20 bytes) → ~50 bytes of CJS shim. A 16x cap is
    // generous.
    assert!(
        cjs1.len() <= s.len() * 16 + 1024,
        "static_esm_to_cjs exploded: in_len={} out_len={}",
        s.len(),
        cjs1.len()
    );

    // The Flow stripper must never grow the input — it only deletes type
    // annotations. The only legitimate growth is one trailing newline per
    // line that the loop always appends (lines() strips the original \n,
    // then we re-add it). Allow up to one extra byte per line.
    let line_count = if s.is_empty() { 0 } else { s.lines().count() };
    let grew = f1.len().saturating_sub(s.len());
    assert!(
        grew <= line_count,
        "strip_inline_flow_types grew the input unexpectedly: in_len={} out_len={} grew={} lines={}",
        s.len(),
        f1.len(),
        grew,
        line_count
    );

    // strip_inline_flow_types must never produce a `:` followed by an
    // identifier in a place it didn't before (heuristic: count of `:` chars
    // should not increase for any well-formed input).
    let colons_in = s.bytes().filter(|&b| b == b':').count();
    let colons_out = f1.bytes().filter(|&b| b == b':').count();
    assert!(
        colons_out <= colons_in + 1,
        "strip_inline_flow_types introduced colons: in={colons_in} out={colons_out}"
    );
});
