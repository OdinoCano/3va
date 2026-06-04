//! Loads a `3va.config.*` file from disk and parses it into a [`ProjectConfig`].
//!
//! `.json` files are parsed directly with `serde_json`.
//! `.js` / `.ts` files are expected to export a single JSON-serialisable
//! default object; we extract it with a lightweight regex that matches the
//! `export default { … }` pattern and then parse the object literal via
//! `serde_json` after stripping TS type annotations.  Full JS evaluation is
//! intentionally avoided to keep the loader zero-dependency on the JS engine
//! and to preserve the sandboxed execution guarantee described in the spec.

use crate::schema::ProjectConfig;
use anyhow::{bail, Context};
use std::path::Path;

/// Load a config file, dispatching on extension.
pub fn load(path: &Path) -> anyhow::Result<ProjectConfig> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "json" => load_json(path),
        "js" | "ts" => load_js_ts(path),
        other => bail!("Unsupported config file extension: .{other}"),
    }
}

fn load_json(path: &Path) -> anyhow::Result<ProjectConfig> {
    let src =
        std::fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;
    serde_json::from_str(&src).with_context(|| format!("Invalid JSON in {}", path.display()))
}

/// Parse a `.js` / `.ts` config by extracting the `export default { … }` /
/// `module.exports = { … }` object literal and deserialising it as JSON5-ish.
///
/// This intentionally does NOT execute arbitrary JavaScript; it only parses
/// a subset of static object literal syntax that `serde_json` can understand.
fn load_js_ts(path: &Path) -> anyhow::Result<ProjectConfig> {
    let src =
        std::fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;

    // Strip TypeScript type annotations on object literals: `satisfies Config`
    // and `as Config` suffixes.
    let src = src
        .replace(" satisfies Config", "")
        .replace(" satisfies ProjectConfig", "");

    // Remove single-line comments (// …) and import/type lines.
    let cleaned: String = src
        .lines()
        .filter(|l| {
            let t = l.trim();
            // drop import statements and type-only lines
            !t.starts_with("import ") && !t.starts_with("export type ")
        })
        .map(|l| {
            // strip inline // comments (naively — good enough for config files)
            if let Some(pos) = l.find("//") {
                // make sure we're not inside a string (simple heuristic)
                let before = &l[..pos];
                if before.chars().filter(|&c| c == '"' || c == '\'').count() % 2 == 0 {
                    return before.to_string();
                }
            }
            l.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Find the exported object literal.
    let json_obj = extract_exported_object(&cleaned).with_context(|| {
        format!(
            "Could not extract default export object from {}. \
                 Use a plain object literal: `export default {{ … }}`",
            path.display()
        )
    })?;

    // Quote unquoted JS object keys (e.g. `port:` → `"port":`)
    let json_obj = quote_unquoted_keys(&json_obj);

    // Replace single-quoted strings with double-quoted strings.
    let json_obj = single_to_double_quotes(&json_obj);

    // Trailing commas in object/arrays are not valid JSON — strip them.
    let json_obj = strip_trailing_commas(&json_obj);

    serde_json::from_str(&json_obj)
        .with_context(|| format!("Failed to parse config from {}", path.display()))
}

/// Extract the object literal after `export default` or `module.exports =`.
fn extract_exported_object(src: &str) -> Option<String> {
    // Try `export default { … }` and `export default ({ … })`
    for prefix in &[
        "export default {",
        "export default ({",
        "module.exports = {",
        "module.exports={",
    ] {
        if let Some(start) = src.find(prefix) {
            let brace_start = src[start..].find('{')? + start;
            return extract_balanced_braces(src, brace_start);
        }
    }
    None
}

/// Extract the balanced `{ … }` block starting at `start_idx` in `src`.
fn extract_balanced_braces(src: &str, start_idx: usize) -> Option<String> {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut in_string: Option<u8> = None;
    let mut escaped = false;

    for (i, &b) in bytes.iter().enumerate().skip(start_idx) {
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' && in_string.is_some() {
            escaped = true;
            continue;
        }
        if let Some(q) = in_string {
            if b == q {
                in_string = None;
            }
        } else if b == b'"' || b == b'\'' || b == b'`' {
            in_string = Some(b);
        } else if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(src[start_idx..=i].to_string());
            }
        }
    }
    None
}

/// Remove trailing commas before `}` or `]` so the result is valid JSON.
fn strip_trailing_commas(src: &str) -> String {
    // Repeated passes until stable is simple and correct for typical configs.
    let re1 = regex_lite::Regex::new(r",(\s*[}\]])").unwrap();
    let mut prev = src.to_string();
    loop {
        let next = re1.replace_all(&prev, "$1").to_string();
        if next == prev {
            break next;
        }
        prev = next;
    }
}

/// Quote unquoted JS object keys so the result is valid JSON.
/// e.g. `{ port: 3000 }` → `{ "port": 3000 }`.
fn quote_unquoted_keys(src: &str) -> String {
    let re = regex_lite::Regex::new(r#"([,\{\n\s])([a-zA-Z_$][a-zA-Z0-9_$]*)(\s*):"#).unwrap();
    re.replace_all(src, |caps: &regex_lite::Captures| {
        // caps[3] is the optional whitespace between key and `:`. Include `:` explicitly.
        format!("{}\"{}\"{}:", &caps[1], &caps[2], &caps[3])
    })
    .to_string()
}

/// Replace single-quoted string literals with double-quoted ones.
/// Only handles simple (non-escaped, non-multiline) strings.
fn single_to_double_quotes(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            out.push('"');
            while let Some(inner) = chars.next() {
                if inner == '\'' {
                    out.push('"');
                    break;
                } else if inner == '\\' {
                    if let Some(escaped) = chars.next() {
                        out.push('\\');
                        out.push(escaped);
                    }
                } else {
                    out.push(inner);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_tmp(content: &str, ext: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(&format!(".{ext}"))
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn load_json_config() {
        let f = write_tmp(r#"{"dev":{"port":9000}}"#, "json");
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.dev.port, 9000);
    }

    #[test]
    fn load_js_config_export_default() {
        let src = r#"
// 3va.config.js
export default {
  dev: {
    port: 4000,
    open: true,
  },
  test: {
    coverage: true,
  },
}
"#;
        let f = write_tmp(src, "js");
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.dev.port, 4000);
        assert!(cfg.dev.open);
        assert!(cfg.test.coverage);
    }

    #[test]
    fn load_ts_config_satisfies() {
        let src = r#"
import type { Config } from '3va/config';
export default {
  bundle: {
    outDir: "./build",
    minify: true,
  },
} satisfies Config;
"#;
        let f = write_tmp(src, "ts");
        let cfg = load(f.path()).unwrap();
        assert_eq!(cfg.bundle.out_dir, "./build");
        assert!(cfg.bundle.minify);
    }

    #[test]
    fn strip_trailing_commas_works() {
        let src = r#"{"a": 1, "b": [2, 3,], "c": {"d": 4,},}"#;
        let clean = strip_trailing_commas(src);
        assert!(serde_json::from_str::<serde_json::Value>(&clean).is_ok());
    }

    #[test]
    fn extract_exported_object_finds_braces() {
        let src = r#"export default { "dev": { "port": 5000 } }"#;
        let obj = extract_exported_object(src).unwrap();
        assert!(obj.starts_with('{'));
        assert!(obj.ends_with('}'));
    }
}
