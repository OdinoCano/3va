use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleKey {
    pub path: PathBuf,
    pub module_type: ModuleType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleType {
    JavaScript,
    TypeScript,
    Json,
    Mjs,
    Cjs,
}

pub struct ModuleResolver {
    root: PathBuf,
    extensions: Vec<String>,
    alias: HashMap<String, PathBuf>,
}

impl ModuleResolver {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            extensions: vec![
                ".ts".to_string(),
                ".tsx".to_string(),
                ".js".to_string(),
                ".jsx".to_string(),
                ".mjs".to_string(),
                ".json".to_string(),
            ],
            alias: HashMap::new(),
        }
    }

    pub fn with_alias(mut self, alias: HashMap<String, PathBuf>) -> Self {
        self.alias = alias;
        self
    }

    pub fn resolve(&self, from: &Path, specifier: &str) -> anyhow::Result<ModuleKey> {
        if let Some(alias_resolved) = self.resolve_alias(specifier) {
            return Ok(alias_resolved);
        }

        if specifier.starts_with('.') {
            self.resolve_relative(from, specifier)
        } else if specifier.starts_with('/') {
            self.resolve_absolute(specifier)
        } else {
            self.resolve_node_module(from, specifier)
        }
    }

    fn resolve_alias(&self, specifier: &str) -> Option<ModuleKey> {
        for (alias, target) in &self.alias {
            if specifier.starts_with(alias) || specifier == alias {
                let module_type = self.guess_type(target);
                return Some(ModuleKey {
                    path: target.clone(),
                    module_type,
                });
            }
        }
        None
    }

    fn resolve_relative(&self, from: &Path, specifier: &str) -> anyhow::Result<ModuleKey> {
        let base = from.parent().unwrap_or(Path::new("."));
        let resolved = base.join(specifier);

        if resolved.is_file() {
            let module_type = self.guess_type(&resolved);
            return Ok(ModuleKey {
                path: resolved,
                module_type,
            });
        }

        for ext in &self.extensions {
            let with_ext = resolved.with_extension(ext.trim_start_matches('.'));
            if with_ext.is_file() {
                let module_type = self.guess_type(&with_ext);
                return Ok(ModuleKey {
                    path: with_ext,
                    module_type,
                });
            }
        }

        let index = resolved.join("index");
        for ext in &self.extensions {
            let index_file = index.with_extension(ext.trim_start_matches('.'));
            if index_file.is_file() {
                let module_type = self.guess_type(&index_file);
                return Ok(ModuleKey {
                    path: index_file,
                    module_type,
                });
            }
        }

        anyhow::bail!("Cannot resolve module: {}", specifier)
    }

    fn resolve_absolute(&self, specifier: &str) -> anyhow::Result<ModuleKey> {
        let path = PathBuf::from(specifier);
        if path.is_file() {
            let module_type = self.guess_type(&path);
            return Ok(ModuleKey { path, module_type });
        }
        anyhow::bail!("Cannot resolve absolute path: {}", specifier)
    }

    fn resolve_node_module(&self, _from: &Path, specifier: &str) -> anyhow::Result<ModuleKey> {
        let node_modules = self.root.join("node_modules").join(specifier);

        if node_modules.is_dir() {
            let package_json = node_modules.join("package.json");
            if package_json.is_file() {
                if let Ok(pkg) = serde_json::from_str::<serde_json::Value>(
                    &std::fs::read_to_string(&package_json)?,
                ) && let Some(main) = pkg.get("main").and_then(|m| m.as_str())
                {
                    let main_path = node_modules.join(main);
                    if main_path.is_file() {
                        let module_type = self.guess_type(&main_path);
                        return Ok(ModuleKey {
                            path: main_path,
                            module_type,
                        });
                    }
                }

                let index_path = node_modules.join("index.js");
                if index_path.is_file() {
                    return Ok(ModuleKey {
                        path: index_path,
                        module_type: ModuleType::JavaScript,
                    });
                }
            }
        }

        anyhow::bail!("Cannot resolve node module: {}", specifier)
    }

    fn guess_type(&self, path: &Path) -> ModuleType {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "ts" => ModuleType::TypeScript,
            "tsx" => ModuleType::TypeScript,
            "js" => ModuleType::JavaScript,
            "jsx" => ModuleType::JavaScript,
            "mjs" => ModuleType::Mjs,
            "cjs" => ModuleType::Cjs,
            "json" => ModuleType::Json,
            _ => ModuleType::JavaScript,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolver_relative() {
        let root = std::env::temp_dir();
        let resolver = ModuleResolver::new(root.clone());

        let from = root.join("index.ts");
        let result = resolver.resolve(&from, "./nonexistent");

        assert!(result.is_err());
    }
}
