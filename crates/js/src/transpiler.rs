use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{
    DecoratorOptions, EnvOptions, JsxOptions, JsxRuntime, Module, TransformOptions, Transformer,
};

/// TypeScript/TSX/JSX → JavaScript transpiler backed by the Oxc toolchain.
///
/// Strips TypeScript types, transforms JSX to React.createElement calls, and
/// down-levels modern syntax. On any parse failure the original source is
/// returned unchanged so callers never observe an error.
pub fn transpile(source: &str) -> String {
    transpile_inner(source, false, false)
}

/// Like `transpile` but also enables JSX transform.
pub fn transpile_jsx(source: &str) -> String {
    transpile_inner(source, true, false)
}

/// Try to transpile a `.js` or plain file that may contain JSX/Flow syntax.
///
/// First attempts plain JS parse; if that yields errors, retries with JSX
/// enabled. If both fail, strips Flow annotations and tries again.
pub fn transpile_js(source: &str) -> String {
    // Try plain JS first (handles modern syntax OXC understands).
    let plain = transpile_inner(source, false, false);
    if !plain.is_empty() && plain != source {
        return plain;
    }

    // Retry with JSX enabled — handles `.js` files that contain JSX.
    let with_jsx = transpile_inner(source, true, false);
    if !with_jsx.is_empty() && with_jsx != source {
        return with_jsx;
    }

    // If both failed, strip Flow annotations and try again with JSX.
    // OXC does not support Flow, so we remove common Flow constructs first.
    let flow_stripped = strip_flow(source);
    if flow_stripped != source {
        let flow_jsx = transpile_inner(&flow_stripped, true, false);
        if !flow_jsx.is_empty() && flow_jsx != flow_stripped {
            return flow_jsx;
        }
        // Try inline Flow stripping on the flow-stripped source
        let inlined = strip_inline_flow_types(&flow_stripped);
        if inlined != flow_stripped {
            return inlined;
        }
    }

    // Last resort: aggressively strip inline type annotations (Flow-specific syntax
    // that OXC cannot parse, like `const x: {[string]: boolean} = {}`).
    strip_inline_flow_types(source)
}

/// Strip common Flow type annotations that OXC cannot parse.
/// Best-effort, no-regex implementation using simple string matching.
fn strip_flow(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut i = 0;
    let bytes = source.as_bytes();
    let len = bytes.len();

    while i < len {
        // Skip '@flow' and '@format' in comments (approximate: skip lines containing @flow)
        if bytes[i] == b'@' && i + 4 < len && &bytes[i..i + 5] == b"@flow" {
            // Skip to end of line
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'@' && i + 7 < len && &bytes[i..i + 8] == b"@format" {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Remove `import typeof * as X from '...'` or `import typeof * as X from "..."`
        if i + 14 < len && bytes[i..i + 14].eq_ignore_ascii_case(b"import typeof ") {
            // Skip to end of semicolon or newline at statement level
            let mut j = i;
            while j < len && bytes[j] != b';' && bytes[j] != b'\n' {
                j += 1;
            }
            if j < len && bytes[j] == b';' {
                j += 1; // include semicolon
            }
            i = j;
            continue;
        }

        // Remove `import type {` ... `} from '...'`
        if i + 11 < len && bytes[i..i + 12].eq_ignore_ascii_case(b"import type {") {
            let mut j = i;
            while j < len && bytes[j] != b';' && bytes[j] != b'\n' {
                j += 1;
            }
            if j < len && bytes[j] == b';' {
                j += 1;
            }
            i = j;
            continue;
        }

        // Remove `import type X from '...'`
        if i + 11 < len && bytes[i..i + 12].eq_ignore_ascii_case(b"import type ") {
            let mut j = i;
            while j < len && bytes[j] != b';' && bytes[j] != b'\n' {
                j += 1;
            }
            if j < len && bytes[j] == b';' {
                j += 1;
            }
            i = j;
            continue;
        }

        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

/// Aggressively strip inline Flow type annotations that OXC can't parse.
/// Works line-by-line, matching simple patterns with character-level scanning.
/// Handles:
///   `const x: Type = val` → `const x = val`
///   `function f(p: T): R {` → `function f(p) {`
fn strip_inline_flow_types(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();

        // Process variable declarations: const/let/var name: Type = value
        if trimmed.starts_with("const ")
            || trimmed.starts_with("let ")
            || trimmed.starts_with("var ")
        {
            let kw_len = if trimmed.starts_with("const ") { 6 } else { 4 };
            let after_kw = &trimmed[kw_len..];
            // Find first colon (type annotation marker)
            if let Some(cp) = after_kw.find(':') {
                let before_colon = &after_kw[..cp];
                // Ensure colon is not after `=` (already past value) or `?` (ternary)
                if !before_colon.contains('=') && !before_colon.contains(')') && cp > 0 {
                    // Find `=` after the colon, respecting brace depth
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

        // Process function declarations: remove param types and return types
        if trimmed.starts_with("function ") {
            let paren = trimmed.find('(');
            if let Some(po) = paren {
                let mut clean = String::with_capacity(trimmed.len());
                clean.push_str(&trimmed[..=po]); // up to and including '('
                let params = &trimmed[po + 1..];
                let mut depth = 0u32;
                let mut i = 0;
                let bytes = params.as_bytes();
                // Build clean params skipping type annotations
                while i < bytes.len() {
                    let ch = bytes[i] as char;
                    match ch {
                        '(' => {
                            depth += 1;
                            clean.push(ch);
                            i += 1;
                        }
                        ')' if depth == 0 => {
                            // End of params — check for return type `: ... {`
                            clean.push(')');
                            i += 1;
                            // Skip return type annotation: `: whatever {`
                            if i < bytes.len() && bytes[i] == b':' {
                                // Find the opening brace of function body
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
                            // Type annotation colon — skip to next `,` or `)`
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

/// Replace every `import.meta.*` usage with a named stub that 3va injects
/// at runtime.
///
/// QuickJS evaluates transpiled entry files in *script* mode, not module mode,
/// so `import.meta` is a syntax error there. We replace all occurrences with
/// the stubs below before the code reaches the parser:
///
/// | Pattern | Replacement |
/// |---------|-------------|
/// | `import.meta.url` | `__vvva_meta_url__` |
/// | `import.meta.env` | `__vvva_meta_env__` |
/// | `import.meta.hot` | `undefined` |
/// | `import.meta.vitest` | `undefined` |
/// | `import.meta.resolve(` | `__vvva_meta_resolve__(` |
/// | `import.meta.glob(` | `__vvva_meta_glob__(` |
///
/// The stubs are declared by `eval_file` (entry point) and by the CJS
/// `require()` module wrapper (nested imports).
///
/// Occurrences inside string literals or comments are intentionally left
/// unchanged so source-code strings are not corrupted.
pub fn replace_import_meta(source: &str) -> String {
    // Ordered longest-first so longer patterns match before their prefixes.
    const PATTERNS: &[(&str, &str)] = &[
        ("import.meta.resolve(", "__vvva_meta_resolve__("),
        ("import.meta.glob(", "__vvva_meta_glob__("),
        ("import.meta.hot", "undefined"),
        ("import.meta.vitest", "undefined"),
        ("import.meta.env", "__vvva_meta_env__"),
        ("import.meta.url", "__vvva_meta_url__"),
    ];

    let mut result = source.to_string();
    for (from, to) in PATTERNS {
        result = replace_outside_strings_and_comments(&result, from, to);
    }
    result
}

/// Byte-level scanner that replaces every occurrence of `from` in `source`
/// that does not lie inside a string literal or a comment.
///
/// Handles:
/// - `"..."` and `'...'` — stops matching inside single/double-quoted strings
/// - `` `...` `` — backtick templates (no recursive `${ }` parsing, but that
///   is fine: the patterns we search for never appear inside `${ }`)
/// - `// ...` — line comments
/// - `/* ... */` — block comments
fn replace_outside_strings_and_comments(source: &str, from: &str, to: &str) -> String {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let pat = from.as_bytes();
    let pat_len = pat.len();
    let mut out = Vec::with_capacity(len.saturating_add(to.len() * 4));
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // ── Line comment ──────────────────────────────────────────────────────
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                out.push(bytes[i]);
                i += 1;
            }
            continue;
        }

        // ── Block comment ─────────────────────────────────────────────────────
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            out.push(bytes[i]);
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

        // ── String literal (single/double quote) ──────────────────────────────
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

        // ── Template literal — recurse into `${ }` interpolations ────────────
        // Content between backticks is NOT code, but `${ … }` sections ARE.
        // We must replace patterns inside interpolations so that patterns like
        // `import.meta.url` inside `\`prefix ${import.meta.url} suffix\`` get
        // rewritten correctly.
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
                } // end of template
                // Start of interpolation — treat as normal code until matching `}`
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
                        // Inside the interpolation, apply the same pattern-replace logic
                        // (strings, comments, and pattern matches).
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

        // ── Pattern match ─────────────────────────────────────────────────────
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

/// Transpile ESM TypeScript/TSX to CommonJS JavaScript.
/// Converts `import`/`export` to `require()`/`module.exports` so the result
/// can run inside the 3va CJS `require()` shim without a separate ESM loader.
///
/// Also rewrites `import.meta.*` to runtime stubs (see [`replace_import_meta`]).
pub fn transpile_to_cjs(source: &str, jsx: bool) -> String {
    let replaced = replace_import_meta(source);
    let out = try_transpile_inner(&replaced, jsx, true).unwrap_or(replaced);
    // OXC 0.132 leaves a bare `export {};` in CJS output to flag the file as an
    // ES module in bundlers that inspect the AST. QuickJS script mode rejects it.
    // Strip it — the CJS require() shim does not need this marker.
    let out = strip_bare_export_marker(&out);
    // OXC 0.132 does not convert static `import`/`export` declarations to
    // `require()`/`module.exports` (the CJS plugin is marked TODO in their source).
    // Apply our own Rust-level converter as a second pass.
    let out = static_esm_to_cjs(&out);
    // Dynamic import() calls are not converted by static_esm_to_cjs (they are
    // expressions, not declarations). Rewrite them to __importAsync() so the
    // CJS require() shim handles resolution instead of the QuickJS ESM loader.
    if out.contains("import(") {
        let mut result = String::with_capacity(out.len());
        let bytes = out.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            if bytes[i] == b'i' && i + 7 <= len && &bytes[i..i + 6] == b"import" {
                let before_ok = i == 0
                    || !(bytes[i - 1].is_ascii_alphanumeric()
                        || bytes[i - 1] == b'_'
                        || bytes[i - 1] == b'$');
                let after_idx = i + 6;
                let mut j = after_idx;
                while j < len && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if before_ok && j < len && bytes[j] == b'(' {
                    result.push_str("__importAsync");
                    i = j;
                    continue;
                }
            }
            result.push(bytes[i] as char);
            i += 1;
        }
        result
    } else {
        out
    }
}

/// Convert static ESM `import`/`export` declarations to CommonJS equivalents.
///
/// OXC 0.132's `Module::CommonJS` setting only adds `"use strict"` and removes
/// unused imports; it does NOT convert used `import` declarations to `require()`.
/// This function fills that gap as a post-OXC pass.
///
/// Handles the patterns that appear in framework SSR bundles (Astro, SvelteKit,
/// Remix, Next.js standalone):
///
/// | ESM | CJS |
/// |-----|-----|
/// | `import 'mod'` | `require('mod');` |
/// | `import X from 'mod'` | `var X = require('mod');` |
/// | `import { a, b as B } from 'mod'` | `var { a, b: B } = require('mod');` |
/// | `import * as X from 'mod'` | `var X = require('mod');` |
/// | `import X, { a } from 'mod'` | `var _m = require('mod'); var X = _m; var { a } = _m;` |
/// | `export default expr` | `module.exports["default"] = expr;` |
/// | `export { a, b as B }` | `module.exports.a = a; module.exports.B = B;` |
/// | `export { a } from 'mod'` | `var _r = require('mod'); module.exports.a = _r.a;` |
/// | `export * from 'mod'` | `Object.assign(module.exports, require('mod'));` |
/// | `export * as X from 'mod'` | `module.exports.X = require('mod');` |
/// | `export function/class/const/let/var name` | declaration + `module.exports.name = name;` |
pub fn static_esm_to_cjs(source: &str) -> String {
    if !source.contains("import ") && !source.contains("export ") {
        return source.to_string();
    }
    let src = source.as_bytes();
    let len = src.len();
    let mut out: Vec<u8> = Vec::with_capacity(len + 512);
    let mut i = 0;
    let mut at_stmt = true; // currently at a statement boundary

    while i < len {
        let b = src[i];

        // ── string literal ────────────────────────────────────────────────────
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

        // ── line comment ──────────────────────────────────────────────────────
        if b == b'/' && i + 1 < len && src[i + 1] == b'/' {
            while i < len && src[i] != b'\n' {
                out.push(src[i]);
                i += 1;
            }
            at_stmt = true;
            continue;
        }

        // ── block comment ─────────────────────────────────────────────────────
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

        // ── statement-boundary characters ─────────────────────────────────────
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

        // ── ESM keyword detection ─────────────────────────────────────────────
        if at_stmt {
            // import …
            if kw_at(src, i, b"import") {
                let after = i + 6;
                if after < len
                    && matches!(src[after], b' ' | b'\t' | b'\n' | b'"' | b'\'')
                    && let Some((conv, ni)) = convert_import(src, i, len)
                {
                    out.extend_from_slice(conv.as_bytes());
                    i = ni;
                    at_stmt = true;
                    continue;
                }
            }
            // export …
            if kw_at(src, i, b"export") {
                let after = i + 6;
                if after < len
                    && matches!(src[after], b' ' | b'\t' | b'\n')
                    && let Some((conv, ni)) = convert_export(src, i, len)
                {
                    out.extend_from_slice(conv.as_bytes());
                    i = ni;
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

// ── helpers ───────────────────────────────────────────────────────────────────

#[inline]
fn kw_at(src: &[u8], i: usize, kw: &[u8]) -> bool {
    let end = i + kw.len();
    end <= src.len() && &src[i..end] == kw
}

/// Collect an ESM statement starting at `start`, reading until `;` at depth 0.
/// Multi-line input is collapsed to a single space-separated string.
/// Returns `(normalized_stmt_without_semicolon, position_after_semicolon)`.
fn collect_stmt(src: &[u8], start: usize, len: usize) -> (String, usize) {
    let mut stmt: Vec<u8> = Vec::new();
    let mut i = start;
    let mut depth: i32 = 0;

    while i < len {
        let b = src[i];
        // Quoted strings inside the stmt
        if b == b'"' || b == b'\'' {
            let q = b;
            stmt.push(b);
            i += 1;
            while i < len {
                let c = src[i];
                if c == b'\\' {
                    stmt.push(c);
                    i += 1;
                    if i < len {
                        stmt.push(src[i]);
                        i += 1;
                    }
                    continue;
                }
                stmt.push(c);
                i += 1;
                if c == q {
                    break;
                }
            }
            continue;
        }
        if b == b'{' || b == b'(' || b == b'[' {
            depth += 1;
        }
        if b == b'}' || b == b')' || b == b']' {
            depth -= 1;
        }
        if b == b';' && depth == 0 {
            i += 1;
            break;
        }
        // Normalise newlines → space
        stmt.push(if b == b'\n' || b == b'\r' { b' ' } else { b });
        i += 1;
    }
    let text = String::from_utf8_lossy(&stmt);
    let normalised = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (normalised, i)
}

fn extract_quoted(s: &str) -> Option<&str> {
    let s = s.trim().trim_end_matches(';').trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

fn find_from(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let len = b.len();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < len {
        match b[i] {
            b'"' | b'\'' => {
                let q = b[i];
                i += 1;
                while i < len {
                    if b[i] == b'\\' {
                        i += 2;
                        continue;
                    }
                    if b[i] == q {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'{' | b'[' | b'(' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' | b')' => {
                depth -= 1;
                i += 1;
            }
            _ => {
                if depth == 0 && i + 4 <= len && &b[i..i + 4] == b"from" {
                    let pre_ok = i == 0 || matches!(b[i - 1], b' ' | b'\t');
                    let post_ok = i + 4 >= len || matches!(b[i + 4], b' ' | b'\t' | b'"' | b'\'');
                    if pre_ok && post_ok {
                        return Some(i);
                    }
                }
                i += 1;
            }
        }
    }
    None
}

fn first_comma_at_depth0(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth: i32 = 0;
    for (i, &c) in b.iter().enumerate() {
        if c == b'{' || c == b'(' || c == b'[' {
            depth += 1;
        }
        if c == b'}' || c == b')' || c == b']' {
            depth -= 1;
        }
        if c == b',' && depth == 0 {
            return Some(i);
        }
    }
    None
}

fn mangle_mod(module: &str) -> String {
    module
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

fn named_list_to_destructure(inner: &str) -> String {
    let parts: Vec<String> = inner
        .split(',')
        .filter_map(|p| {
            let p = p.trim();
            if p.is_empty() {
                return None;
            }
            if let Some(pos) = p.find(" as ") {
                Some(format!("{}: {}", p[..pos].trim(), p[pos + 4..].trim()))
            } else {
                Some(p.to_string())
            }
        })
        .collect();
    format!("{{ {} }}", parts.join(", "))
}

fn convert_import(src: &[u8], start: usize, len: usize) -> Option<(String, usize)> {
    let (stmt, end) = collect_stmt(src, start, len);
    let s = stmt.trim();
    let rest = s.strip_prefix("import")?.trim_start();

    // Side-effect: `import 'mod'` or `import "mod"`
    if rest.starts_with('"') || rest.starts_with('\'') {
        let m = extract_quoted(rest)?;
        return Some((format!("require(\"{m}\");"), end));
    }

    // Locate `from 'module'`
    let from_pos = find_from(rest)?;
    let clause = rest[..from_pos].trim();
    let mod_part = rest[from_pos + 4..].trim();
    let module = extract_quoted(mod_part)?;

    // `* as X from`
    if let Some(ns) = clause.strip_prefix("* as ") {
        return Some((format!("var {} = require(\"{module}\");", ns.trim()), end));
    }

    // `{ … } from`
    if clause.starts_with('{') {
        let inner = clause.trim_start_matches('{').trim_end_matches('}').trim();
        let destruct = named_list_to_destructure(inner);
        return Some((format!("var {destruct} = require(\"{module}\");"), end));
    }

    // `default, rest from` — combined import
    if let Some(comma) = first_comma_at_depth0(clause) {
        let def_name = clause[..comma].trim();
        let named = clause[comma + 1..].trim();
        let tmp = format!("_im_{}", mangle_mod(module));
        let mut out = format!("var {tmp} = require(\"{module}\");\n");
        out.push_str(&format!(
            "var {def_name} = ({tmp}.default !== undefined) ? {tmp}.default : {tmp};\n"
        ));
        if let Some(ns) = named.strip_prefix("* as ") {
            out.push_str(&format!("var {} = {tmp};", ns.trim()));
        } else if named.starts_with('{') {
            let inner = named.trim_start_matches('{').trim_end_matches('}').trim();
            for part in inner.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if let Some(p) = part.find(" as ") {
                    let (orig, alias) = (part[..p].trim(), part[p + 4..].trim());
                    out.push_str(&format!("var {alias} = {tmp}.{orig};\n"));
                } else {
                    out.push_str(&format!("var {part} = {tmp}.{part};\n"));
                }
            }
        }
        return Some((out.trim_end().to_string(), end));
    }

    // Default: `import X from 'mod'`
    if !clause.is_empty() {
        return Some((
            format!("var {} = require(\"{module}\");", clause.trim()),
            end,
        ));
    }

    None
}

fn convert_export(src: &[u8], start: usize, len: usize) -> Option<(String, usize)> {
    // Peek ahead to decide how to collect the statement
    let peek_start = start + 7; // skip "export "
    let rest_preview = std::str::from_utf8(&src[peek_start.min(len)..]).unwrap_or("");
    let rp = rest_preview.trim_start();

    // export function/async function/class — body collection needed
    let is_fn_or_class = rp.starts_with("function ")
        || rp.starts_with("async function ")
        || rp.starts_with("class ")
        || rp.starts_with("default function ")
        || rp.starts_with("default async function ")
        || rp.starts_with("default class ");

    let (stmt_text, end) = if is_fn_or_class {
        // Collect "export " + signature up to first { ... }
        let mut text = Vec::new();
        let mut i = start;
        let mut depth: i32 = 0;
        let mut entered = false;
        while i < len {
            let b = src[i];
            if b == b'"' || b == b'\'' || b == b'`' {
                let q = b;
                text.push(b);
                i += 1;
                while i < len {
                    let c = src[i];
                    if c == b'\\' {
                        text.push(c);
                        i += 1;
                        if i < len {
                            text.push(src[i]);
                            i += 1;
                        }
                        continue;
                    }
                    text.push(c);
                    i += 1;
                    if c == q {
                        break;
                    }
                }
                continue;
            }
            if b == b'{' {
                depth += 1;
                entered = true;
            }
            if b == b'}' {
                depth -= 1;
            }
            text.push(b);
            i += 1;
            if entered && depth == 0 {
                break;
            }
        }
        (String::from_utf8_lossy(&text).into_owned(), i)
    } else {
        let (s, e) = collect_stmt(src, start, len);
        (s, e)
    };

    let s = stmt_text.trim();
    let rest = s.strip_prefix("export")?.trim_start();

    // export default …
    if let Some(val) = rest.strip_prefix("default ") {
        let val = val.trim().trim_end_matches(';');
        // export default function/class with name
        if val.starts_with("function ")
            || val.starts_with("async function ")
            || val.starts_with("class ")
        {
            let name = val
                .split_whitespace()
                .find(|&w| w != "function" && w != "async" && w != "class")
                .map(|w| w.trim_end_matches('('))
                .filter(|n| !n.is_empty());
            if let Some(n) = name {
                return Some((format!("{val}\nmodule.exports[\"default\"] = {n};\n"), end));
            }
        }
        return Some((format!("module.exports[\"default\"] = {val};\n"), end));
    }

    // export * from 'mod'
    if let Some(after_star) = rest.strip_prefix("* ") {
        let after_star = after_star.trim_start();
        if let Some(mod_part) = after_star.strip_prefix("from ") {
            let m = extract_quoted(mod_part.trim())?;
            return Some((
                format!("Object.assign(module.exports, require(\"{m}\"));\n"),
                end,
            ));
        }
        // export * as X from 'mod'
        if let Some(rest2) = after_star.strip_prefix("as ")
            && let Some(fp) = find_from(rest2)
        {
            let name = rest2[..fp].trim();
            let mod_part = rest2[fp + 4..].trim();
            let m = extract_quoted(mod_part)?;
            return Some((format!("module.exports.{name} = require(\"{m}\");\n"), end));
        }
    }

    // export { … } or export { … } from 'mod'
    if rest.starts_with('{') {
        let close = rest.find('}')?;
        let inner = rest[1..close].trim();
        let after = rest[close + 1..].trim().trim_start_matches(';').trim();

        if let Some(mod_part) = after.strip_prefix("from ") {
            let m = extract_quoted(mod_part.trim())?;
            let tmp = format!("_re_{}", mangle_mod(m));
            let mut o = format!("var {tmp} = require(\"{m}\");\n");
            for part in inner.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if let Some(p) = part.find(" as ") {
                    let (orig, alias) = (part[..p].trim(), part[p + 4..].trim());
                    o.push_str(&format!("module.exports.{alias} = {tmp}.{orig};\n"));
                } else {
                    o.push_str(&format!("module.exports.{part} = {tmp}.{part};\n"));
                }
            }
            return Some((o, end));
        }

        // Local re-export
        let mut o = String::new();
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(p) = part.find(" as ") {
                let (orig, alias) = (part[..p].trim(), part[p + 4..].trim());
                if alias == "default" {
                    o.push_str(&format!("module.exports[\"default\"] = {orig};\n"));
                } else {
                    o.push_str(&format!("module.exports.{alias} = {orig};\n"));
                }
            } else {
                o.push_str(&format!("module.exports.{part} = {part};\n"));
            }
        }
        return Some((o, end));
    }

    // export function name() { … }
    if rest.starts_with("function ") || rest.starts_with("async function ") {
        let after_kw = rest.strip_prefix("async ").unwrap_or(rest);
        let name = after_kw
            .strip_prefix("function ")?
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()?;
        if name.is_empty() {
            return None;
        }
        let decl = s[7..].trim(); // strip "export "
        return Some((format!("{decl}\nmodule.exports.{name} = {name};\n"), end));
    }

    // export class Name { … }
    if let Some(after_class) = rest.strip_prefix("class ") {
        let name = after_class
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()?;
        if name.is_empty() {
            return None;
        }
        let decl = s[7..].trim();
        return Some((format!("{decl}\nmodule.exports.{name} = {name};\n"), end));
    }

    // export const/let/var name = …
    for kw in &["const ", "let ", "var "] {
        if let Some(after_kw) = rest.strip_prefix(kw) {
            let name = after_kw
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
                .filter(|n| !n.is_empty())?;
            let decl = s[7..].trim().trim_end_matches(';');
            return Some((format!("{decl};\nmodule.exports.{name} = {name};\n"), end));
        }
    }

    None
}

/// Remove `export {};` (with optional whitespace/semicolons) that OXC injects
/// into CJS output as an ES-module marker. Matches the line exactly to avoid
/// accidentally removing meaningful export statements.
fn strip_bare_export_marker(code: &str) -> String {
    // Fast-path: no "export" at all.
    if !code.contains("export") {
        return code.to_string();
    }
    code.lines()
        .filter(|line| {
            let t = line.trim();
            // Remove `export {};` / `export {}` but keep all other export forms.
            !(t == "export {};" || t == "export {}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn transpile_inner(source: &str, jsx: bool, _flow: bool) -> String {
    try_transpile_inner(source, jsx, false).unwrap_or_else(|_| source.to_string())
}

fn try_transpile_inner(source: &str, jsx: bool, to_cjs: bool) -> Result<String, ()> {
    let allocator = Allocator::default();

    let source_type = if jsx {
        SourceType::tsx()
    } else {
        SourceType::mjs().with_typescript(true)
    };

    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.errors.is_empty() && parsed.program.body.is_empty() {
        return Err(());
    }

    let mut program = parsed.program;

    let scoping = SemanticBuilder::new()
        .with_enum_eval(true)
        .build(&program)
        .semantic
        .into_scoping();

    // Enable TypeScript legacy decorators (experimentalDecorators + emitDecoratorMetadata)
    // Required by TypeORM, MikroORM, tsyringe, routing-controllers, etc.
    let jsx_opts = if jsx {
        // Use Classic runtime: transforms <Foo /> → React.createElement(Foo, null)
        // This is what React Native and legacy React use.
        JsxOptions {
            jsx_plugin: true,
            runtime: JsxRuntime::Classic,
            pragma: Some("React.createElement".to_string()),
            pragma_frag: Some("React.Fragment".to_string()),
            ..JsxOptions::default()
        }
    } else {
        JsxOptions::default()
    };
    let options = TransformOptions {
        decorator: DecoratorOptions {
            legacy: true,
            emit_decorator_metadata: true,
        },
        jsx: jsx_opts,
        env: if to_cjs {
            EnvOptions {
                module: Module::CommonJS,
                ..EnvOptions::default()
            }
        } else {
            EnvOptions::default()
        },
        ..TransformOptions::default()
    };

    let ret = Transformer::new(&allocator, std::path::Path::new("input.tsx"), &options)
        .build_with_scoping(scoping, &mut program);

    if !ret.errors.is_empty() && program.body.is_empty() {
        return Err(());
    }

    Ok(Codegen::new().build(&program).code)
}

/// Heuristic: does this source look like it contains JSX elements?
/// Looks for `<Identifier` or `<identifier` not preceded by `<` or `=` (to avoid
/// template literals, generics, and comparison operators).
pub fn looks_like_jsx(source: &str) -> bool {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'<' && i + 1 < len {
            let next = bytes[i + 1];
            // Skip `<<` (bitshift), `<=` (comparison), `<!` (HTML comment, not JSX)
            if next == b'<' || next == b'=' {
                i += 2;
                continue;
            }
            // `</` starts a closing tag — that's JSX
            if next == b'/' {
                return true;
            }
            // `<Letter` — opening JSX tag
            if next.is_ascii_alphabetic() {
                return true;
            }
        }
        // Skip string literals to avoid false positives
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

/// Heuristic: detect top-level `await` in JavaScript source.
///
/// Scans for the `await` keyword outside of block-bodied function scopes by
/// tracking brace depth.  Returns `true` when `await` appears at brace depth 0
/// (top level) or depth 1 (inside a class body but outside any method — rare).
///
/// Known false positives:
/// - `async () => await expr` (async arrow with expression body, no braces) —
///   the `await` sits at depth 0 but is inside an async arrow function.
///
/// False positives are harmless: the caller enables `JS_EVAL_FLAG_ASYNC` which
/// is a no-op when no actual top-level await exists.
pub fn has_top_level_await(code: &str) -> bool {
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

#[inline]
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── static_esm_to_cjs ─────────────────────────────────────────────────────

    #[test]
    fn esm_side_effect_import() {
        let src = "import 'some-polyfill';\nconsole.log(1);";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("require(\"some-polyfill\")"), "got: {out}");
        assert!(!out.contains("import "), "got: {out}");
    }

    #[test]
    fn esm_default_import() {
        let src = "import path from 'path';\nconsole.log(path.join('a','b'));";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("var path = require(\"path\")"), "got: {out}");
        assert!(!out.contains("import "), "got: {out}");
    }

    #[test]
    fn esm_named_imports() {
        let src = "import { join, dirname } from 'path';\nconsole.log(join('a','b'));";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("require(\"path\")"), "got: {out}");
        assert!(out.contains("join"), "got: {out}");
        assert!(!out.contains("import "), "got: {out}");
    }

    #[test]
    fn esm_named_imports_with_alias() {
        let src = "import { readFile as rf } from 'fs/promises';";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("require(\"fs/promises\")"), "got: {out}");
        assert!(out.contains("readFile: rf"), "got: {out}");
    }

    #[test]
    fn esm_namespace_import() {
        let src = "import * as fs from 'fs';";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("var fs = require(\"fs\")"), "got: {out}");
    }

    #[test]
    fn esm_combined_default_named() {
        let src = "import React, { useState } from 'react';";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("require(\"react\")"), "got: {out}");
        assert!(out.contains("React"), "got: {out}");
        assert!(out.contains("useState"), "got: {out}");
    }

    #[test]
    fn esm_multiline_named_import() {
        let src = "import {\n  join,\n  dirname,\n  resolve\n} from 'path';\n";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("require(\"path\")"), "got: {out}");
        assert!(out.contains("join"), "got: {out}");
        assert!(out.contains("dirname"), "got: {out}");
        assert!(!out.contains("import "), "got: {out}");
    }

    #[test]
    fn esm_export_star_from() {
        let src = "export * from './utils';";
        let out = static_esm_to_cjs(src);
        assert!(
            out.contains("Object.assign(module.exports, require(\"./utils\"))"),
            "got: {out}"
        );
    }

    #[test]
    fn esm_export_named_local() {
        let src = "export { handler, config };";
        let out = static_esm_to_cjs(src);
        assert!(
            out.contains("module.exports.handler = handler"),
            "got: {out}"
        );
        assert!(out.contains("module.exports.config = config"), "got: {out}");
    }

    #[test]
    fn esm_export_named_as_default() {
        let src = "export { handler as default };";
        let out = static_esm_to_cjs(src);
        assert!(
            out.contains("module.exports[\"default\"] = handler"),
            "got: {out}"
        );
    }

    #[test]
    fn esm_export_named_from() {
        let src = "export { foo, bar as baz } from './mod';";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("require(\"./mod\")"), "got: {out}");
        assert!(out.contains("module.exports.foo"), "got: {out}");
        assert!(out.contains("module.exports.baz"), "got: {out}");
    }

    #[test]
    fn esm_export_default_value() {
        let src = "export default 42;";
        let out = static_esm_to_cjs(src);
        assert!(
            out.contains("module.exports[\"default\"] = 42"),
            "got: {out}"
        );
    }

    #[test]
    fn esm_export_const() {
        let src = "export const VERSION = '1.0.0';";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("const VERSION = '1.0.0'"), "got: {out}");
        assert!(
            out.contains("module.exports.VERSION = VERSION"),
            "got: {out}"
        );
    }

    #[test]
    fn esm_no_replace_inside_string() {
        let src = "const s = \"import path from 'path';\";";
        let out = static_esm_to_cjs(src);
        assert_eq!(out, src, "should not replace inside string");
    }

    #[test]
    fn esm_no_replace_inside_line_comment() {
        let src = "// import path from 'path';\nconst x = 1;";
        let out = static_esm_to_cjs(src);
        assert!(out.contains("// import path"), "got: {out}");
        assert!(!out.contains("require"), "got: {out}");
    }

    // ── import.meta replacement ───────────────────────────────────────────────

    #[test]
    fn replace_meta_url() {
        let src = "console.log(import.meta.url);";
        let out = replace_import_meta(src);
        assert!(out.contains("__vvva_meta_url__"), "got: {out}");
        assert!(!out.contains("import.meta.url"), "got: {out}");
    }

    #[test]
    fn replace_meta_env() {
        let src = "const mode = import.meta.env.MODE;";
        let out = replace_import_meta(src);
        assert!(out.contains("__vvva_meta_env__"), "got: {out}");
        assert!(!out.contains("import.meta.env"), "got: {out}");
    }

    #[test]
    fn replace_meta_hot_with_undefined() {
        let src = "if (import.meta.hot) { import.meta.hot.accept(); }";
        let out = replace_import_meta(src);
        assert!(!out.contains("import.meta.hot"), "got: {out}");
        assert_eq!(out.matches("undefined").count(), 2, "got: {out}");
    }

    #[test]
    fn replace_meta_resolve() {
        let src = "const p = import.meta.resolve('./foo');";
        let out = replace_import_meta(src);
        assert!(out.contains("__vvva_meta_resolve__("), "got: {out}");
        assert!(!out.contains("import.meta.resolve("), "got: {out}");
    }

    #[test]
    fn replace_meta_glob() {
        let src = "const mods = import.meta.glob('./routes/**');";
        let out = replace_import_meta(src);
        assert!(out.contains("__vvva_meta_glob__("), "got: {out}");
    }

    #[test]
    fn no_replace_inside_double_quoted_string() {
        let src = r#"var s = "import.meta.url";"#;
        let out = replace_import_meta(src);
        assert!(
            out.contains("\"import.meta.url\""),
            "should not replace in string, got: {out}"
        );
    }

    #[test]
    fn no_replace_inside_single_quoted_string() {
        let src = "var s = 'import.meta.env';";
        let out = replace_import_meta(src);
        assert!(
            out.contains("'import.meta.env'"),
            "should not replace in string, got: {out}"
        );
    }

    #[test]
    fn no_replace_inside_line_comment() {
        let src = "// import.meta.url\nconst x = 1;";
        let out = replace_import_meta(src);
        assert!(
            out.contains("// import.meta.url"),
            "should not replace in comment, got: {out}"
        );
        assert!(out.contains("const x = 1"), "should keep code, got: {out}");
    }

    #[test]
    fn no_replace_inside_block_comment() {
        let src = "/* import.meta.url is cool */ const x = 1;";
        let out = replace_import_meta(src);
        assert!(
            out.contains("/* import.meta.url"),
            "should not replace in block comment, got: {out}"
        );
    }

    #[test]
    fn no_replace_inside_template_literal() {
        let src = "const s = `see import.meta.url for details`;";
        let out = replace_import_meta(src);
        assert!(
            out.contains("import.meta.url"),
            "should not replace in template literal, got: {out}"
        );
    }

    #[test]
    fn multiple_replacements_in_one_file() {
        let src = "const u = import.meta.url; const e = import.meta.env;";
        let out = replace_import_meta(src);
        assert!(out.contains("__vvva_meta_url__"), "got: {out}");
        assert!(out.contains("__vvva_meta_env__"), "got: {out}");
    }

    #[test]
    fn transpile_to_cjs_strips_bare_export_marker() {
        // OXC 0.132 eliminates unused imports as dead code and emits a bare
        // `export {};` as an ES-module marker — verify it gets stripped so
        // QuickJS script mode can evaluate the result without a syntax error.
        let src = "import path from 'path';\nglobalThis.x = 1;";
        let out = transpile_to_cjs(src, false);
        assert!(
            !out.contains("export {}"),
            "bare export marker should be stripped, got:\n{out}"
        );
        // Unused import is removed by OXC dead-code elimination.
        assert!(
            !out.contains("import path"),
            "unused import should be gone, got:\n{out}"
        );
    }

    #[test]
    fn transpile_to_cjs_handles_import_meta() {
        let src = r#"
import { readFile } from 'fs/promises';
const dir = new URL('.', import.meta.url).pathname;
export function getDir() { return dir; }
"#;
        let out = transpile_to_cjs(src, false);
        assert!(
            !out.contains("import.meta"),
            "OXC CJS output must not contain import.meta, got:\n{out}"
        );
        assert!(
            out.contains("__vvva_meta_url__"),
            "should have url stub, got:\n{out}"
        );
    }

    // ── TypeScript type annotation tests ──────────────────────────────────────

    #[test]
    fn test_variable_type_annotation() {
        let input = "const x: string = 'hello';";
        let output = transpile(input);
        assert!(
            !output.contains(": string"),
            "should strip type annotation, got: {output}"
        );
        assert!(
            output.contains("const x"),
            "should keep variable declaration"
        );
        assert!(output.contains("hello"), "should keep value");
    }

    #[test]
    fn test_function_param_types() {
        let input = "function greet(name: string, age: number): void { console.log(name); }";
        let output = transpile(input);
        assert!(!output.contains(": string"), "got: {output}");
        assert!(!output.contains(": number"), "got: {output}");
        assert!(!output.contains(": void"), "got: {output}");
        assert!(output.contains("function greet("), "got: {output}");
    }

    #[test]
    fn test_interface_removal() {
        let input = "interface User { name: string; age: number; }\nconst x = 1;";
        let output = transpile(input);
        assert!(!output.contains("interface"), "got: {output}");
        assert!(output.contains("const x = 1"), "got: {output}");
    }

    #[test]
    fn test_jsx_classic_transform() {
        let input = "const el = <div className=\"foo\">hello</div>;";
        let output = transpile_jsx(input);
        assert!(
            output.contains("React.createElement"),
            "should use React.createElement, got: {output}"
        );
        assert!(
            !output.contains("<div"),
            "should not contain JSX, got: {output}"
        );
        assert!(
            output.contains("\"foo\""),
            "should keep props, got: {output}"
        );
    }

    #[test]
    fn test_jsx_component_transform() {
        let input = "const el = <MyComponent name=\"test\" />;";
        let output = transpile_jsx(input);
        assert!(output.contains("React.createElement"), "got: {output}");
        assert!(output.contains("MyComponent"), "got: {output}");
    }

    #[test]
    fn test_tsx_strips_types_and_jsx() {
        let input = "const el: JSX.Element = <div id={foo as string}>hi</div>;";
        let output = transpile_jsx(input);
        assert!(output.contains("React.createElement"), "got: {output}");
        assert!(!output.contains(": JSX.Element"), "got: {output}");
    }

    #[test]
    fn test_looks_like_jsx_positive() {
        assert!(looks_like_jsx("<View style={styles.container}>"));
        assert!(looks_like_jsx("return <div />;"));
        assert!(looks_like_jsx("return </div>;"));
        assert!(looks_like_jsx("const x = <MyComp foo={1} />;"));
    }

    #[test]
    fn test_looks_like_jsx_negative() {
        assert!(!looks_like_jsx("const x = a < b ? 1 : 0;"));
        assert!(!looks_like_jsx("const x = a <= b;"));
        assert!(!looks_like_jsx("const s = \"<not jsx>\";"));
    }

    #[test]
    fn test_type_alias_removal() {
        let input = "type StringOrNumber = string | number;\nconst y = 2;";
        let output = transpile(input);
        assert!(!output.contains("StringOrNumber"), "got: {output}");
        assert!(output.contains("const y = 2"), "got: {output}");
    }

    #[test]
    fn test_as_cast_stripping() {
        let input = "const x = value as string;";
        let output = transpile(input);
        assert!(!output.contains("as string"), "got: {output}");
        assert!(output.contains("const x = value"), "got: {output}");
    }

    #[test]
    fn test_generic_function() {
        let input = "function identity<T>(x: T): T { return x; }";
        let output = transpile(input);
        assert!(
            !output.contains("<T>"),
            "should strip generic, got: {output}"
        );
        assert!(output.contains("function identity("), "got: {output}");
    }

    #[test]
    fn test_preserve_object_literal() {
        let input = "const obj = { key: 'value', count: 42 };";
        let output = transpile(input);
        assert!(output.contains("key"), "got: {output}");
        assert!(output.contains("value"), "got: {output}");
    }

    #[test]
    fn test_fallback_on_invalid_source() {
        let input = "this is not valid ts or js @@##";
        let output = transpile(input);
        assert!(!output.is_empty());
    }

    #[test]
    fn test_import_type_removal() {
        let input = "import type { User } from './types';\nconst x = 1;";
        let output = transpile(input);
        assert!(!output.contains("User"), "got: {output}");
        assert!(output.contains("const x = 1"), "got: {output}");
    }

    #[test]
    fn test_access_modifiers() {
        let input = "class Foo { public name: string; private age: number; }";
        let output = transpile(input);
        assert!(!output.contains("public "), "got: {output}");
        assert!(!output.contains("private "), "got: {output}");
        assert!(output.contains("name"), "got: {output}");
    }

    #[test]
    fn test_non_null_assertion() {
        let input = "const x = foo!.bar;";
        let output = transpile(input);
        assert!(!output.contains("!."), "got: {output}");
        assert!(output.contains("foo.bar"), "got: {output}");
    }

    #[test]
    fn test_declare_removal() {
        let input = "declare const x: string;\nconst y = 1;";
        let output = transpile(input);
        assert!(!output.contains("declare"), "got: {output}");
        assert!(output.contains("const y = 1"), "got: {output}");
    }

    #[test]
    fn test_regular_import_preserved() {
        let input = "import { foo } from './bar';\nconst x = foo();";
        let output = transpile(input);
        assert!(output.contains("import"), "got: {output}");
        assert!(output.contains("foo"), "got: {output}");
    }
}
