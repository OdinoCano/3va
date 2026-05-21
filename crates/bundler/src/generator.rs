use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct BundlerOptions {
    pub format: OutputFormat,
    pub minify: bool,
    pub sourcemap: bool,
    pub splitting: bool,
    pub chunk_filename: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Iife,
    Umd,
    Cjs,
    Esm,
}

impl Default for BundlerOptions {
    fn default() -> Self {
        Self {
            format: OutputFormat::Iife,
            minify: false,
            sourcemap: false,
            splitting: false,
            chunk_filename: "[name].[hash].js".to_string(),
        }
    }
}

pub struct CodeGenerator {
    modules: HashMap<String, String>,
    options: BundlerOptions,
}

impl CodeGenerator {
    pub fn new(options: BundlerOptions) -> Self {
        Self {
            modules: HashMap::new(),
            options,
        }
    }

    pub fn add_module(&mut self, name: String, code: String) {
        self.modules.insert(name, code);
    }

    pub fn get_module(&self, name: &str) -> Option<&String> {
        self.modules.get(name)
    }

    pub fn get_options(&self) -> &BundlerOptions {
        &self.options
    }

    pub fn generate(&self) -> String {
        match self.options.format {
            OutputFormat::Iife => self.generate_iife(),
            OutputFormat::Umd => self.generate_umd(),
            OutputFormat::Cjs => self.generate_cjs(),
            OutputFormat::Esm => self.generate_esm(),
        }
    }

    /// Generate bundle + optional inline source map comment.
    /// Returns `(bundle_code, Option<source_map_json>)`.
    pub fn generate_with_sourcemap(&self) -> (String, Option<String>) {
        let code = self.generate();
        if !self.options.sourcemap {
            return (code, None);
        }

        let sources: Vec<&String> = self.modules.keys().collect();
        let sources_content: Vec<&String> = self.modules.values().collect();

        // Build line→source mappings: each source module contributes N lines.
        // Mapping format: generated_line → (source_index, original_line).
        let mut mappings_lines: Vec<String> = Vec::new();

        // Wrap/header offset depends on format
        let header_lines = match self.options.format {
            OutputFormat::Iife => 1, // "(function() {"
            OutputFormat::Umd => 4,
            OutputFormat::Cjs | OutputFormat::Esm => 0,
        };

        for _ in 0..header_lines {
            mappings_lines.push(String::new()); // unmapped header lines
        }

        for (src_idx, (_name, code)) in self.modules.iter().enumerate() {
            let line_count = code.lines().count().max(1);
            for orig_line in 0..line_count {
                // Each segment: generated_col=0, source=src_idx, orig_line, orig_col=0
                // VLQ encode: [0, src_idx_delta, orig_line_delta, 0]
                let src_delta = if orig_line == 0 { src_idx as i64 } else { 0 };
                let line_delta = if orig_line == 0 { 0i64 } else { 1 };
                let seg = encode_vlq_segment(0, src_delta, line_delta, 0);
                mappings_lines.push(seg);
            }
        }

        let mappings = mappings_lines.join(";");

        let map = serde_json::json!({
            "version": 3,
            "sources": sources.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "sourcesContent": sources_content.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "names": [],
            "mappings": mappings,
        });

        let map_json = serde_json::to_string(&map).unwrap_or_default();
        (code, Some(map_json))
    }

    fn generate_iife(&self) -> String {
        if self.modules.is_empty() {
            return String::new();
        }

        let main = self
            .modules
            .get("index")
            .or(self.modules.get("main"))
            .or(self.modules.values().next());

        match main {
            Some(code) => {
                if self.options.minify {
                    self.minify(code)
                } else {
                    format!("(function() {{\n{}\n}})();", code)
                }
            }
            None => String::new(),
        }
    }

    fn generate_umd(&self) -> String {
        let main = self
            .modules
            .get("index")
            .or(self.modules.values().next())
            .cloned()
            .unwrap_or_default();

        format!(
            r#"(function (root, factory) {{
    if (typeof module === 'object' && module.exports) {{
        module.exports = factory();
    }} else {{
        root['bundle'] = factory();
    }}
}}(this, function() {{
    return {};
}}));"#,
            if self.options.minify {
                self.minify(&main)
            } else {
                main
            }
        )
    }

    fn generate_cjs(&self) -> String {
        let mut output = String::new();

        for (name, code) in &self.modules {
            output.push_str(&format!("// Module: {}\n", name));
            output.push_str(code);
            output.push_str("\n\n");
        }

        if self.options.minify {
            self.minify(&output)
        } else {
            output
        }
    }

    fn generate_esm(&self) -> String {
        let mut output = String::new();

        for (name, code) in &self.modules {
            output.push_str(&format!("// Module: {}\n", name));
            output.push_str("export ");
            output.push_str(code);
            output.push_str("\n\n");
        }

        if self.options.minify {
            self.minify(&output)
        } else {
            output
        }
    }

    /// Minify JavaScript:
    /// - Strips `//` line comments and `/* */` block comments (outside strings)
    /// - Collapses runs of whitespace to a single space
    /// - Removes spaces around operator characters: `= + - * / % < > ! & | ^ ~ ? : ; , { } ( ) [ ]`
    /// - Removes trailing semicolons before `}` and leading semicolons after `{`
    fn minify(&self, code: &str) -> String {
        let chars: Vec<char> = code.chars().collect();
        let len = chars.len();
        let mut out = String::with_capacity(len / 2);
        let mut i = 0;
        let mut in_str = false;
        let mut str_char = ' ';
        let mut in_template = false;
        let mut template_depth = 0usize;

        while i < len {
            let c = chars[i];

            // ── Inside string literals — pass through verbatim ──────────────
            if in_str {
                out.push(c);
                if c == '\\' && i + 1 < len {
                    i += 1;
                    out.push(chars[i]);
                } else if c == str_char {
                    in_str = false;
                }
                i += 1;
                continue;
            }
            if in_template {
                out.push(c);
                if c == '\\' && i + 1 < len {
                    i += 1;
                    out.push(chars[i]);
                } else if c == '$' && i + 1 < len && chars[i + 1] == '{' {
                    template_depth += 1;
                } else if c == '}' && template_depth > 0 {
                    template_depth -= 1;
                } else if c == '`' && template_depth == 0 {
                    in_template = false;
                }
                i += 1;
                continue;
            }

            // ── Line comment `//` ────────────────────────────────────────────
            if c == '/' && i + 1 < len && chars[i + 1] == '/' {
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }

            // ── Block comment `/* ... */` ─────────────────────────────────────
            if c == '/' && i + 1 < len && chars[i + 1] == '*' {
                i += 2;
                while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2; // skip */
                continue;
            }

            // ── String / template literal start ───────────────────────────────
            if c == '"' || c == '\'' {
                in_str = true;
                str_char = c;
                out.push(c);
                i += 1;
                continue;
            }
            if c == '`' {
                in_template = true;
                out.push(c);
                i += 1;
                continue;
            }

            // ── Whitespace handling ──────────────────────────────────────────
            if c.is_whitespace() {
                // Replace any whitespace run with a single space, unless
                // we can drop it entirely around operators.
                let prev = out.chars().last();
                let next = chars[i + 1..]
                    .iter()
                    .find(|&&ch| !ch.is_whitespace())
                    .copied();
                let can_drop = matches!(prev, Some(p) if is_op_char(p))
                    || matches!(next, Some(n) if is_op_char(n));
                if !can_drop {
                    // Keep a single space if previous char was alphanumeric/identifier
                    let need_space = matches!(prev, Some(p) if p.is_alphanumeric() || p == '_' || p == '$')
                        && matches!(next, Some(n) if n.is_alphanumeric() || n == '_' || n == '$');
                    if need_space {
                        out.push(' ');
                    }
                }
                // Skip all whitespace
                while i < len && chars[i].is_whitespace() {
                    i += 1;
                }
                continue;
            }

            out.push(c);
            i += 1;
        }

        // Post-pass: remove redundant semicolons before `}`
        let out = out.replace(";}", "}").replace(",}", "}");
        out.trim().to_string()
    }
}

#[derive(Clone)]
pub struct Chunk {
    pub name: String,
    pub modules: Vec<String>,
    pub deps: Vec<String>,
}

impl Chunk {
    pub fn new(name: String) -> Self {
        Self {
            name,
            modules: Vec::new(),
            deps: Vec::new(),
        }
    }

    pub fn add_module(&mut self, module: String) {
        self.modules.push(module);
    }
}

pub struct CodeSplitter {
    chunks: Vec<Chunk>,
}

impl CodeSplitter {
    pub fn new() -> Self {
        Self { chunks: Vec::new() }
    }

    pub fn split(
        &mut self,
        entry_points: &[String],
        deps: &HashMap<String, Vec<String>>,
    ) -> Vec<Chunk> {
        let mut visited = std::collections::HashSet::new();

        for entry in entry_points {
            if !visited.contains(entry) {
                self.split_entry(entry, deps, &mut visited);
            }
        }

        self.chunks.clone()
    }

    fn split_entry(
        &mut self,
        entry: &str,
        deps: &HashMap<String, Vec<String>>,
        visited: &mut std::collections::HashSet<String>,
    ) {
        if visited.contains(entry) {
            return;
        }
        visited.insert(entry.to_string());

        let mut chunk = Chunk::new(entry.to_string());
        chunk.add_module(entry.to_string());

        if let Some(entry_deps) = deps.get(entry) {
            for dep in entry_deps {
                chunk.add_module(dep.clone());
                chunk.deps.push(dep.clone());

                if let Some(sub_deps) = deps.get(dep) {
                    for sub in sub_deps {
                        if !visited.contains(sub) {
                            self.split_entry(sub, deps, visited);
                        }
                    }
                }
            }
        }

        self.chunks.push(chunk);
    }
}

// Base64 VLQ encoding for source map v3 mappings.
// Each value is sign-magnitude encoded then split into 5-bit groups, MSB first,
// with continuation bit (bit 5) set on all but the last group.
const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn vlq_encode_value(mut value: i64) -> String {
    let sign_bit = if value < 0 { 1i64 } else { 0i64 };
    value = value.unsigned_abs() as i64;
    let mut vlq = (value << 1) | sign_bit;

    let mut result = String::new();
    loop {
        let mut digit = vlq & 0x1f;
        vlq >>= 5;
        if vlq > 0 {
            digit |= 0x20; // continuation bit
        }
        result.push(BASE64_CHARS[digit as usize] as char);
        if vlq == 0 {
            break;
        }
    }
    result
}

/// Encode a source map segment: [generated_col, source_idx, orig_line, orig_col] as VLQ.
fn encode_vlq_segment(gen_col: i64, src_idx: i64, orig_line: i64, orig_col: i64) -> String {
    format!(
        "{}{}{}{}",
        vlq_encode_value(gen_col),
        vlq_encode_value(src_idx),
        vlq_encode_value(orig_line),
        vlq_encode_value(orig_col),
    )
}

fn is_op_char(c: char) -> bool {
    matches!(
        c,
        '=' | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '<'
            | '>'
            | '!'
            | '&'
            | '|'
            | '^'
            | '~'
            | '?'
            | ':'
            | ';'
            | ','
            | '{'
            | '}'
            | '('
            | ')'
            | '['
            | ']'
    )
}

impl Default for CodeSplitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_generator_iife() {
        let mut generator = CodeGenerator::new(BundlerOptions::default());
        generator.add_module("main".to_string(), "console.log('hello')".to_string());

        let output = generator.generate();
        assert!(output.contains("console.log('hello')"));
    }

    #[test]
    fn test_code_generator_minify() {
        let mut generator = CodeGenerator::new(BundlerOptions {
            minify: true,
            ..Default::default()
        });
        generator.add_module("main".to_string(), "const   x   =   1;".to_string());

        let output = generator.generate();
        assert!(output.contains("const x"));
    }

    #[test]
    fn test_code_splitter() {
        let mut splitter = CodeSplitter::new();

        let mut deps = HashMap::new();
        deps.insert("main".to_string(), vec!["util".to_string()]);

        let chunks = splitter.split(&["main".to_string()], &deps);
        assert!(!chunks.is_empty());
    }
}
