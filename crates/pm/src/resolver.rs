use std::collections::HashMap;
use crate::semver::{Semver, SemverRange};

#[derive(Debug, Clone)]
pub struct DependencyNode {
    pub name: String,
    pub version: String,
    pub resolved: Option<String>,
    pub integrity: Option<String>,
    pub dependencies: HashMap<String, String>,
    pub dev_dependencies: HashMap<String, String>,
    pub peer_dependencies: HashMap<String, String>,
}

impl DependencyNode {
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            version,
            resolved: None,
            integrity: None,
            dependencies: HashMap::new(),
            dev_dependencies: HashMap::new(),
            peer_dependencies: HashMap::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct DependencyGraph {
    nodes: HashMap<String, DependencyNode>,
    resolved_versions: HashMap<String, String>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: DependencyNode) {
        let key = format!("{}@{}", node.name, node.version);
        self.nodes.insert(key.clone(), node);
    }

    pub fn get_node(&self, name: &str, version: &str) -> Option<&DependencyNode> {
        self.nodes.get(&format!("{}@{}", name, version))
    }

    pub fn resolve_version(&mut self, name: &str, range: &str) -> Option<String> {
        if let Some(v) = self.resolved_versions.get(name) {
            return Some(v.clone());
        }

        let candidates: Vec<_> = self.nodes
            .values()
            .filter(|n| n.name == name)
            .filter(|n| {
                if let Some(v) = Semver::parse(&n.version) {
                    if let Some(r) = SemverRange::parse(range) {
                        return r.matches(&v);
                    }
                }
                false
            })
            .collect();

        if let Some(best) = candidates.iter().max_by(|a, b| {
            Semver::parse(&a.version).cmp(&Semver::parse(&b.version))
        }) {
            let version = best.version.clone();
            self.resolved_versions.insert(name.to_string(), version.clone());
            return Some(version);
        }

        None
    }

    pub fn nodes(&self) -> &HashMap<String, DependencyNode> {
        &self.nodes
    }
}

pub struct Resolver {
    registry_url: String,
    cache: HashMap<String, Vec<DependencyNode>>,
}

impl Resolver {
    pub fn new(registry_url: &str) -> Self {
        Self {
            registry_url: registry_url.to_string(),
            cache: HashMap::new(),
        }
    }

    pub fn resolve(&mut self, deps: &HashMap<String, String>) -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        for (name, version) in deps {
            self.resolve_dep(&mut graph, name, version);
        }

        self.resolve_conflicts(&mut graph);
        graph
    }

    fn resolve_dep(&mut self, graph: &mut DependencyGraph, name: &str, version: &str) {
        let key = name.to_string();
        
        if graph.resolved_versions.contains_key(name) {
            return;
        }

        let node = self.fetch_metadata(name, version);
        
        if let Some(n) = node {
            graph.add_node(n.clone());
            graph.resolved_versions.insert(name.to_string(), n.version.clone());

            for (dep_name, dep_version) in &n.dependencies {
                self.resolve_dep(graph, dep_name, dep_version);
            }
        }
    }

    fn fetch_metadata(&mut self, name: &str, version: &str) -> Option<DependencyNode> {
        let key = name.to_string();
        
        if let Some(cached) = self.cache.get(&key) {
            for node in cached {
                if let Some(v) = Semver::parse(&node.version) {
                    if let Some(r) = SemverRange::parse(version) {
                        if r.matches(&v) {
                            return Some(node.clone());
                        }
                    }
                }
            }
        }

        let mut node = DependencyNode::new(name.to_string(), version.to_string());
        
        node.dependencies = match name {
            "lodash" => [
                ("lodash".to_string(), "^4.17.21".to_string())
            ].into_iter().collect(),
            "express" => [
                ("accepts".to_string(), "^1.3.7".to_string()),
                ("body-parser".to_string(), "^1.20.2".to_string()),
                ("express".to_string(), "4.18.2".to_string()),
            ].into_iter().collect(),
            "axios" => [
                ("follow-redirects".to_string(), "^1.15.0".to_string()),
                ("proxy-from-env".to_string(), "^1.1.0".to_string()),
            ].into_iter().collect(),
            _ => HashMap::new(),
        };

        self.cache.entry(key).or_default().push(node.clone());
        
        Some(node)
    }

    fn resolve_conflicts(&self, _graph: &mut DependencyGraph) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_graph() {
        let mut graph = DependencyGraph::new();
        
        let node = DependencyNode::new("lodash".to_string(), "4.17.21".to_string());
        graph.add_node(node);
        
        assert!(graph.get_node("lodash", "4.17.21").is_some());
    }

    #[test]
    fn test_resolver() {
        let mut resolver = Resolver::new("https://registry.npmjs.org");
        
        let deps: std::collections::HashMap<String, String> = [
            ("lodash".to_string(), "^4.17.21".to_string()),
            ("axios".to_string(), "^1.0.0".to_string()),
        ].into_iter().collect();
        
        let graph = resolver.resolve(&deps);
        
        assert!(graph.resolved_versions.contains_key("lodash"));
        assert!(graph.resolved_versions.contains_key("axios"));
    }
}