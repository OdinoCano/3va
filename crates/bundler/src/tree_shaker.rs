use std::collections::{HashMap, HashSet};

pub struct TreeShaker {
    used_exports: HashMap<String, HashSet<String>>,
    entry_points: Vec<String>,
}

impl TreeShaker {
    pub fn new(entry_points: Vec<String>) -> Self {
        Self {
            used_exports: HashMap::new(),
            entry_points,
        }
    }

    pub fn analyze_imports(&mut self, code: &str) -> HashSet<String> {
        let mut imports = HashSet::new();
        let mut in_string = false;
        let mut current = String::new();
        let mut chars = code.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\'' || c == '"' {
                in_string = !in_string;
            }

            if in_string && c != '\'' && c != '"' {
                current.push(c);
            }

            if !in_string && !current.is_empty() {
                if current.starts_with("./") || current.starts_with("../") || !current.contains('/')
                {
                    imports.insert(current.clone());
                }
                current.clear();
            }
        }

        imports
    }

    pub fn analyze_exports(&self, code: &str) -> Vec<String> {
        let mut exports = Vec::new();

        for line in code.lines() {
            let line = line.trim();
            if line.starts_with("export ") {
                exports.push(line.to_string());
            } else if line.starts_with("module.exports") || line.starts_with("exports.") {
                exports.push(line.to_string());
            }
        }

        exports
    }

    pub fn mark_used(&mut self, module: &str, export: &str) {
        self.used_exports
            .entry(module.to_string())
            .or_default()
            .insert(export.to_string());
    }

    pub fn is_used(&self, module: &str, export: &str) -> bool {
        self.used_exports
            .get(module)
            .map(|s| s.contains(export))
            .unwrap_or(true)
    }

    pub fn remove_dead_code(&self, code: &str) -> String {
        let mut result = String::new();

        for line in code.lines() {
            let trimmed = line.trim();

            if trimmed.contains("if (false)") || trimmed.starts_with("if (false)") {
                continue;
            }

            result.push_str(line);
            result.push('\n');
        }

        result
    }
}

pub struct DeadCodeEliminator {
    conditionals: Vec<String>,
}

impl DeadCodeEliminator {
    pub fn new() -> Self {
        Self {
            conditionals: Vec::new(),
        }
    }

    pub fn eliminate(&self, code: &str) -> String {
        let mut result = Vec::new();
        let mut skip_block = false;
        let mut brace_count = 0;

        for line in code.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("if (false)") || trimmed == "if (false) {" {
                skip_block = true;
                brace_count = 0;
            }

            if skip_block {
                brace_count += trimmed.matches('{').count() as i32;
                brace_count -= trimmed.matches('}').count() as i32;

                if brace_count <= 0 {
                    skip_block = false;
                }
                continue;
            }

            result.push(line);
        }

        result.join("\n")
    }
}

impl Default for DeadCodeEliminator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_shaker_analyze_imports() {
        let mut shaper = TreeShaker::new(vec!["main".to_string()]);

        let code = r#"
import foo from './foo';
import { bar } from './bar';
const x = require('./baz');
"#;

        let imports = shaper.analyze_imports(code);
        assert!(imports.contains(&"./foo".to_string()));
        assert!(imports.contains(&"./bar".to_string()));
    }

    #[test]
    fn test_tree_shaker_analyze_exports() {
        let shaper = TreeShaker::new(vec![]);

        let code = r#"
export function test() {}
export const x = 1;
module.exports = {};
"#;

        let exports = shaper.analyze_exports(code);
        assert!(!exports.is_empty());
    }

    #[test]
    fn test_dead_code_eliminator() {
        let elim = DeadCodeEliminator::new();

        let code = r#"
const a = 1;
if (false) {
    const dead = 2;
}
const b = 3;
"#;

        let result = elim.eliminate(code);
        assert!(!result.contains("if (false)"));
    }
}
