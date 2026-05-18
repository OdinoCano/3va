use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::resolver::{DependencyGraph, DependencyNode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    #[serde(rename = "lockfileVersion")]
    pub lockfile_version: u32,
    pub name: String,
    pub version: String,
    pub packages: HashMap<String, LockfilePackage>,
    pub dependencies: HashMap<String, LockfileDep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockfilePackage {
    pub version: String,
    pub resolved: Option<String>,
    pub integrity: Option<String>,
    pub dev: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockfileDep {
    pub version: String,
    pub resolved: Option<String>,
    pub integrity: Option<String>,
    pub dependencies: Option<HashMap<String, String>>,
    pub dev: Option<bool>,
}

impl Lockfile {
    pub fn generate(graph: &DependencyGraph, name: &str, version: &str) -> Self {
        let mut packages = HashMap::new();
        let mut dependencies = HashMap::new();

        packages.insert("".to_string(), LockfilePackage {
            version: version.to_string(),
            resolved: None,
            integrity: None,
            dev: None,
        });

        for (_, node) in graph.nodes() {
            let key = format!("node_modules/{}", node.name);
            packages.insert(key.clone(), LockfilePackage {
                version: node.version.clone(),
                resolved: node.resolved.clone(),
                integrity: node.integrity.clone(),
                dev: None,
            });

            let pkg_key = format!("node_modules/{}/package.json", node.name);
            packages.insert(pkg_key, LockfilePackage {
                version: node.version.clone(),
                resolved: None,
                integrity: None,
                dev: None,
            });

            let deps: HashMap<String, String> = node.dependencies
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            dependencies.insert(node.name.clone(), LockfileDep {
                version: node.version.clone(),
                resolved: node.resolved.clone(),
                integrity: node.integrity.clone(),
                dependencies: if deps.is_empty() { None } else { Some(deps) },
                dev: None,
            });
        }

        Self {
            lockfile_version: 3,
            name: name.to_string(),
            version: version.to_string(),
            packages,
            dependencies,
        }
    }

    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let lock: Lockfile = serde_json::from_str(&content)?;
        Ok(lock)
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::DependencyGraph;

    #[test]
    fn test_lockfile_generation() {
        let mut graph = DependencyGraph::new();
        let node = crate::resolver::DependencyNode::new("lodash".to_string(), "4.17.21".to_string());
        graph.add_node(node);
        
        let lockfile = Lockfile::generate(&graph, "test-project", "1.0.0");
        
        assert_eq!(lockfile.lockfile_version, 3);
        assert!(lockfile.packages.contains_key("node_modules/lodash"));
        assert!(lockfile.dependencies.contains_key("lodash"));
    }
}