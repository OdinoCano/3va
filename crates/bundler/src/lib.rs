pub mod generator;
pub mod resolver;
pub mod tree_shaker;

pub use generator::{BundlerOptions, CodeGenerator, Chunk, OutputFormat};
pub use resolver::{ModuleKey, ModuleResolver, ModuleType};
pub use tree_shaker::{DeadCodeEliminator, TreeShaker};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Bundler {
    resolver: ModuleResolver,
    tree_shaker: TreeShaker,
    code_gen: CodeGenerator,
    modules: HashMap<String, PathBuf>,
}

impl Bundler {
    pub fn new(root: PathBuf) -> Self {
        Self {
            resolver: ModuleResolver::new(root),
            tree_shaker: TreeShaker::new(vec![]),
            code_gen: CodeGenerator::new(BundlerOptions::default()),
            modules: HashMap::new(),
        }
    }

    pub fn with_options(mut self, options: BundlerOptions) -> Self {
        self.code_gen = CodeGenerator::new(options);
        self
    }

    pub fn add_entry(&mut self, entry: &str) -> anyhow::Result<()> {
        let path = PathBuf::from(entry);
        if path.is_file() {
            self.modules.insert(entry.to_string(), path);
        } else {
            anyhow::bail!("Entry file not found: {}", entry);
        }
        Ok(())
    }

    pub fn bundle(&mut self) -> anyhow::Result<String> {
        for (name, path) in &self.modules {
            let code = self.process_module(path)?;
            self.code_gen.add_module(name.clone(), code);
        }

        Ok(self.code_gen.generate())
    }

    fn process_module(&self, path: &Path) -> anyhow::Result<String> {
        let content = std::fs::read_to_string(path)?;
        
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        
        match ext {
            "ts" | "tsx" => Ok(self.strip_types(&content)),
            "js" | "jsx" => Ok(content),
            "json" => Ok(format!("module.exports = {};", content)),
            _ => Ok(content),
        }
    }

    fn strip_types(&self, code: &str) -> String {
        let mut result = String::new();
        
        for line in code.lines() {
            let trimmed = line.trim();
            
            if trimmed.starts_with("interface ")
                || trimmed.starts_with("type ")
                || trimmed.contains(": string")
                || trimmed.contains(": number")
                || trimmed.contains(": boolean")
                || trimmed.contains(": void")
                || trimmed.starts_with("// @ts-")
            {
                continue;
            }
            
            result.push_str(line);
            result.push('\n');
        }
        
        result
    }
}

pub fn bundle_file(input: &str, output: &str, options: Option<BundlerOptions>) -> anyhow::Result<()> {
    let root = PathBuf::from(".");
    let mut bundler = Bundler::new(root);
    
    if let Some(opts) = options {
        bundler = bundler.with_options(opts);
    }
    
    bundler.add_entry(input)?;
    let result = bundler.bundle()?;
    
    std::fs::write(output, result)?;
    
    tracing::info!("Bundled {} -> {}", input, output);
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundler_creation() {
        let bundler = Bundler::new(PathBuf::from("."));
        assert!(bundler.modules.is_empty());
    }

    #[test]
    fn test_bundler_options_default() {
        let options = BundlerOptions::default();
        assert!(!options.minify);
        assert_eq!(options.format, OutputFormat::Iife);
    }
}