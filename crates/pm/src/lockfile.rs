use crate::resolver::DependencyGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockfileDep {
    pub version: String,
    pub resolved: Option<String>,
    pub integrity: Option<String>,
    pub dependencies: Option<HashMap<String, String>>,
    pub dev: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
}

impl Lockfile {
    pub fn generate(graph: &DependencyGraph, name: &str, version: &str) -> Self {
        let mut packages = HashMap::new();
        let mut dependencies = HashMap::new();

        packages.insert(
            "".to_string(),
            LockfilePackage {
                version: version.to_string(),
                resolved: None,
                integrity: None,
                dev: None,
                registry: None,
            },
        );

        for node in graph.nodes().values() {
            let key = format!("node_modules/{}", node.name);
            packages.insert(
                key.clone(),
                LockfilePackage {
                    version: node.version.clone(),
                    resolved: node.resolved.clone(),
                    integrity: node.integrity.clone(),
                    dev: None,
                    registry: None,
                },
            );

            let pkg_key = format!("node_modules/{}/package.json", node.name);
            packages.insert(
                pkg_key,
                LockfilePackage {
                    version: node.version.clone(),
                    resolved: None,
                    integrity: None,
                    dev: None,
                    registry: None,
                },
            );

            let deps: HashMap<String, String> = node
                .dependencies
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            dependencies.insert(
                node.name.clone(),
                LockfileDep {
                    version: node.version.clone(),
                    resolved: node.resolved.clone(),
                    integrity: node.integrity.clone(),
                    dependencies: if deps.is_empty() { None } else { Some(deps) },
                    dev: None,
                    registry: None,
                },
            );
        }

        Self {
            lockfile_version: 3,
            name: name.to_string(),
            version: version.to_string(),
            packages,
            dependencies,
        }
    }

    /// Returns the stored registry host for a top-level dependency, if recorded.
    pub fn registry_for(&self, pkg_name: &str) -> Option<&str> {
        self.dependencies.get(pkg_name)?.registry.as_deref()
    }

    /// Returns a map of registry_host → [package names] for all packages that have a registry recorded.
    pub fn registries_needed(
        &self,
        packages: &[String],
    ) -> std::collections::HashMap<String, Vec<String>> {
        let mut map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for pkg in packages {
            if let Some(reg) = self.registry_for(pkg) {
                map.entry(reg.to_string()).or_default().push(pkg.clone());
            }
        }
        map
    }

    /// Records the registry for a specific top-level dependency in-place.
    pub fn set_registry(&mut self, pkg_name: &str, registry_host: &str) {
        if let Some(dep) = self.dependencies.get_mut(pkg_name) {
            dep.registry = Some(registry_host.to_string());
        }
        let node_key = format!("node_modules/{}", pkg_name);
        if let Some(pkg) = self.packages.get_mut(&node_key) {
            pkg.registry = Some(registry_host.to_string());
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

    // ── save / load round-trip ────────────────────────────────────────────────

    #[test]
    fn lockfile_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("3va-lock.json");

        let mut graph = DependencyGraph::new();
        let node = crate::resolver::DependencyNode::new("axios".to_string(), "1.7.9".to_string());
        graph.add_node(node);

        let original = Lockfile::generate(&graph, "my-project", "1.0.0");
        original.save(&path).unwrap();

        let loaded = Lockfile::load(&path).unwrap();

        assert_eq!(loaded.lockfile_version, 3);
        assert_eq!(loaded.name, "my-project");
        assert_eq!(loaded.version, "1.0.0");
        assert!(loaded.packages.contains_key("node_modules/axios"));
        assert!(loaded.dependencies.contains_key("axios"));
        assert_eq!(
            loaded.dependencies["axios"].version,
            original.dependencies["axios"].version
        );
    }

    #[test]
    fn lockfile_is_valid_json_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("3va-lock.json");

        let graph = DependencyGraph::new();
        let lockfile = Lockfile::generate(&graph, "test", "0.0.1");
        lockfile.save(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("lockfile en disco debe ser JSON válido");
        assert_eq!(parsed["lockfileVersion"], 3);
    }

    // ── registry tracking: set_registry / registry_for / registries_needed ───
    // Usado por `3va update` para saber qué --allow-net necesita cada paquete

    #[test]
    fn set_registry_and_registry_for() {
        let mut graph = DependencyGraph::new();
        graph.add_node(crate::resolver::DependencyNode::new(
            "lodash".to_string(),
            "4.17.21".to_string(),
        ));
        let mut lockfile = Lockfile::generate(&graph, "test", "1.0.0");

        lockfile.set_registry("lodash", "registry.npmjs.org");

        assert_eq!(lockfile.registry_for("lodash"), Some("registry.npmjs.org"));
        assert_eq!(lockfile.registry_for("nonexistent"), None);
    }

    #[test]
    fn registries_needed_groups_packages_by_registry() {
        let mut graph = DependencyGraph::new();
        graph.add_node(crate::resolver::DependencyNode::new(
            "lodash".to_string(),
            "4.17.21".to_string(),
        ));
        graph.add_node(crate::resolver::DependencyNode::new(
            "axios".to_string(),
            "1.7.9".to_string(),
        ));
        graph.add_node(crate::resolver::DependencyNode::new(
            "path".to_string(),
            "0.1.0".to_string(),
        ));
        let mut lockfile = Lockfile::generate(&graph, "test", "1.0.0");

        lockfile.set_registry("lodash", "registry.npmjs.org");
        lockfile.set_registry("axios", "registry.yarnpkg.com");
        lockfile.set_registry("path", "jsr.io");

        let needed = lockfile.registries_needed(&[
            "lodash".to_string(),
            "axios".to_string(),
            "path".to_string(),
        ]);

        assert!(needed["registry.npmjs.org"].contains(&"lodash".to_string()));
        assert!(needed["registry.yarnpkg.com"].contains(&"axios".to_string()));
        assert!(needed["jsr.io"].contains(&"path".to_string()));
    }

    #[test]
    fn registries_needed_skips_packages_without_registry() {
        let mut graph = DependencyGraph::new();
        graph.add_node(crate::resolver::DependencyNode::new(
            "lodash".to_string(),
            "4.17.21".to_string(),
        ));
        let lockfile = Lockfile::generate(&graph, "test", "1.0.0");
        // No llamamos set_registry → lodash no tiene registry registrado

        let needed = lockfile.registries_needed(&["lodash".to_string()]);
        assert!(needed.is_empty());
    }

    #[test]
    fn test_lockfile_generation() {
        let mut graph = DependencyGraph::new();
        let node =
            crate::resolver::DependencyNode::new("lodash".to_string(), "4.17.21".to_string());
        graph.add_node(node);

        let lockfile = Lockfile::generate(&graph, "test-project", "1.0.0");

        assert_eq!(lockfile.lockfile_version, 3);
        assert!(lockfile.packages.contains_key("node_modules/lodash"));
        assert!(lockfile.dependencies.contains_key("lodash"));
    }
}
