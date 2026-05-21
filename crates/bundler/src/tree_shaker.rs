use std::collections::{HashMap, HashSet};

pub struct TreeShaker {
    used_exports: HashMap<String, HashSet<String>>,
    #[allow(dead_code)]
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
        let chars = code.chars().peekable();

        for c in chars {
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
            if line.starts_with("export ")
                || line.starts_with("module.exports")
                || line.starts_with("exports.")
            {
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

    pub fn shake(&mut self, module_code: &str, used_exports: &HashSet<String>) -> String {
        let mut result = String::new();
        let mut in_export_block = false;
        let mut brace_count = 0;
        let mut current_export = String::new();

        for line in module_code.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("export ") && !trimmed.contains("from") {
                let export_name = self.extract_export_name(trimmed);
                if let Some(name) = export_name {
                    if used_exports.is_empty() || used_exports.contains(&name) {
                        result.push_str(line);
                        result.push('\n');
                    }
                    continue;
                }
            }

            if trimmed.starts_with("export {") || trimmed.starts_with("export { ") {
                in_export_block = true;
                brace_count = 0;
                current_export.clear();
            }

            if in_export_block {
                brace_count += trimmed.matches('{').count() as i32;
                brace_count -= trimmed.matches('}').count() as i32;

                for name in self.extract_named_exports(trimmed) {
                    if used_exports.is_empty() || used_exports.contains(&name) {
                        current_export.push_str(&format!(" {},", name));
                    }
                }

                if brace_count <= 0 {
                    in_export_block = false;
                    if !current_export.is_empty() {
                        result.push_str(&format!("export {{{}}};\n", current_export.trim()));
                    }
                }
                continue;
            }

            if trimmed.starts_with("if (false)") || trimmed.contains("if (false)") {
                continue;
            }

            result.push_str(line);
            result.push('\n');
        }

        result
    }

    fn extract_export_name(&self, export_line: &str) -> Option<String> {
        if export_line.starts_with("export function ") {
            let name = export_line.strip_prefix("export function ").unwrap_or("");
            return name.split_whitespace().next().map(String::from);
        }
        if export_line.starts_with("export const ") || export_line.starts_with("export let ") {
            let name = export_line
                .strip_prefix("export const ")
                .or(export_line.strip_prefix("export let "))
                .unwrap_or("");
            return name.split_whitespace().next().map(String::from);
        }
        if export_line.starts_with("export class ") {
            let name = export_line.strip_prefix("export class ").unwrap_or("");
            return name.split_whitespace().next().map(String::from);
        }
        None
    }

    fn extract_named_exports(&self, line: &str) -> Vec<String> {
        let mut exports = Vec::new();
        let trimmed = line.trim();
        let content = trimmed.trim_start_matches('{').trim_end_matches('}');

        for part in content.split(',') {
            let part = part.trim();
            if !part.is_empty()
                && !part.starts_with("//")
                && let Some(name) = part.split_whitespace().next()
            {
                let name = name.trim_end_matches(',').trim_end_matches(';');
                if !name.is_empty() && name != "from" {
                    exports.push(name.to_string());
                }
            }
        }

        exports
    }
}

pub struct DeadCodeEliminator {
    #[allow(dead_code)]
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
        assert!(imports.iter().any(|s| s == "./foo"));
        assert!(imports.iter().any(|s| s == "./bar"));
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
