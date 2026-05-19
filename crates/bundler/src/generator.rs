use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct BundlerOptions {
    pub format: OutputFormat,
    pub minify: bool,
    pub sourcemap: bool,
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

    pub fn generate(&self) -> String {
        match self.options.format {
            OutputFormat::Iife => self.generate_iife(),
            OutputFormat::Umd => self.generate_umd(),
            OutputFormat::Cjs => self.generate_cjs(),
            OutputFormat::Esm => self.generate_esm(),
        }
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

    fn minify(&self, code: &str) -> String {
        let mut result = String::new();
        let mut in_string = false;
        let mut string_char = ' ';
        let mut prev_was_space = false;

        for c in code.chars() {
            if !in_string && c.is_whitespace() {
                if !prev_was_space {
                    result.push(' ');
                    prev_was_space = true;
                }
                continue;
            }

            if c == '"' || c == '\'' || c == '`' {
                if !in_string {
                    in_string = true;
                    string_char = c;
                } else if c == string_char {
                    in_string = false;
                }
            }

            result.push(c);
            prev_was_space = false;
        }

        result.trim().to_string()
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
