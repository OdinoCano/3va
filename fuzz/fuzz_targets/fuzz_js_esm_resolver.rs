#![no_main]

use libfuzzer_sys::fuzz_target;

// Re-implementations of the pure byte-level helpers in crates/js/src/esm.rs.

fn source_is_esm(code: &str, path: &str) -> bool {
    if path.ends_with(".mjs") {
        return true;
    }
    if path.ends_with(".cjs") {
        return false;
    }
    let mut in_block = false;
    for line in code.lines() {
        let t = line.trim();
        if in_block {
            if t.contains("*/") {
                in_block = false;
            }
            continue;
        }
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        if t.starts_with("/*") {
            in_block = true;
            continue;
        }
        if t.starts_with("import ")
            || t.starts_with("import{")
            || t.starts_with("export ")
            || t.starts_with("export{")
            || t.starts_with("export default")
        {
            return true;
        }
    }
    false
}

fn extract_cjs_named_exports(source: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut names = BTreeSet::new();
    let has_export_helper = source.contains("__export(");
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("exports.")
            && let Some(eq_pos) = rest.find(" = ")
        {
            let name = rest[..eq_pos].trim();
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            {
                names.insert(name.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("module.exports.")
            && let Some(eq_pos) = rest.find(" = ")
        {
            let name = rest[..eq_pos].trim();
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            {
                names.insert(name.to_string());
            }
        }
        if has_export_helper
            && let Some(colon_pos) = trimmed.find(':')
            && trimmed[colon_pos + 1..].trim_start().starts_with("() =>")
        {
            let name = trimmed[..colon_pos].trim().trim_matches(['"', '\'']);
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            {
                names.insert(name.to_string());
            }
        }
    }
    names.retain(|n| n != "default" && n != "__esModule");
    names.into_iter().collect()
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };

    // Split the input into a code half and a path half. The path can be empty.
    let (code, path) = if let Some(pos) = s.find('\0') {
        (&s[..pos], &s[pos + 1..])
    } else {
        (s, "")
    };

    // ── source_is_esm ─────────────────────────────────────────────────────────
    // .mjs is always ESM regardless of code.
    if path.ends_with(".mjs") {
        assert!(
            source_is_esm(code, path),
            ".mjs must always be ESM: code={code:?}"
        );
    }
    // .cjs is never ESM.
    if path.ends_with(".cjs") {
        assert!(
            !source_is_esm(code, path),
            ".cjs must never be ESM: code={code:?}"
        );
    }
    // Determinism.
    assert_eq!(
        source_is_esm(code, path),
        source_is_esm(code, path),
        "source_is_esm is non-deterministic"
    );
    // Import keyword inside a line comment is not ESM.
    let commented = "// import foo from 'bar'\nmodule.exports = {};\n";
    assert!(
        !source_is_esm(commented, "x.js"),
        "import inside line comment must not be ESM"
    );
    // Import keyword inside a block comment is not ESM.
    let block_commented = "/* import foo from 'bar' */\nmodule.exports = {};\n";
    assert!(
        !source_is_esm(block_commented, "x.js"),
        "import inside block comment must not be ESM"
    );

    // ── extract_cjs_named_exports ─────────────────────────────────────────────
    // Determinism and no-panic on arbitrary input.
    let v1 = extract_cjs_named_exports(code);
    let v2 = extract_cjs_named_exports(code);
    assert_eq!(v1, v2, "extract_cjs_named_exports is non-deterministic");
    // Result must be sorted (BTreeSet) and unique.
    for w in v1.windows(2) {
        assert!(w[0] < w[1], "extract_cjs_named_exports output not sorted");
    }
    // "default" and "__esModule" must be filtered out.
    assert!(!v1.iter().any(|n| n == "default" || n == "__esModule"));
});
