use crate::semver::{Semver, SemverRange};
use std::collections::HashMap;

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

        let candidates: Vec<_> = self
            .nodes
            .values()
            .filter(|n| n.name == name)
            .filter(|n| {
                if let Some(v) = Semver::parse(&n.version)
                    && let Some(r) = SemverRange::parse(range)
                {
                    return r.matches(&v);
                }
                false
            })
            .collect();

        if let Some(best) = candidates
            .iter()
            .max_by(|a, b| Semver::parse(&a.version).cmp(&Semver::parse(&b.version)))
        {
            let version = best.version.clone();
            self.resolved_versions
                .insert(name.to_string(), version.clone());
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
    client: reqwest::Client,
}

impl Resolver {
    pub fn new(registry_url: &str) -> Self {
        Self {
            registry_url: registry_url.to_string(),
            cache: HashMap::new(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn resolve(&mut self, deps: &HashMap<String, String>) -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        let mut stack: Vec<(String, String)> =
            deps.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        while let Some((name, version)) = stack.pop() {
            if graph.resolved_versions.contains_key(&name) {
                continue;
            }

            if let Some(node) = self.fetch_metadata(&name, &version).await {
                graph.add_node(node.clone());
                graph
                    .resolved_versions
                    .insert(name.clone(), node.version.clone());

                for (dep_name, dep_version) in &node.dependencies {
                    stack.push((dep_name.clone(), dep_version.clone()));
                }
            }
        }

        self.resolve_conflicts(&mut graph);
        graph
    }

    async fn fetch_metadata(&mut self, name: &str, version: &str) -> Option<DependencyNode> {
        let key = name.to_string();

        if let Some(cached) = self.cache.get(&key)
            && let Some(best) = Self::find_best_match(cached, version)
        {
            return Some(best.clone());
        }

        let url = format!("{}/{}", self.registry_url, name);
        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let data: serde_json::Value = resp.json().await.ok()?;

        let mut nodes: Vec<DependencyNode> = Vec::new();
        if let Some(versions) = data["versions"].as_object() {
            for (ver, meta) in versions {
                let mut node = DependencyNode::new(name.to_string(), ver.clone());

                node.resolved = meta["dist"]["tarball"].as_str().map(|s| s.to_string());
                node.integrity = meta["dist"]["integrity"].as_str().map(|s| s.to_string());

                if let Some(deps) = meta["dependencies"].as_object() {
                    for (dep_name, dep_ver) in deps {
                        if let Some(dv) = dep_ver.as_str() {
                            node.dependencies.insert(dep_name.clone(), dv.to_string());
                        }
                    }
                }

                if let Some(dev_deps) = meta["devDependencies"].as_object() {
                    for (dep_name, dep_ver) in dev_deps {
                        if let Some(dv) = dep_ver.as_str() {
                            node.dev_dependencies
                                .insert(dep_name.clone(), dv.to_string());
                        }
                    }
                }

                if let Some(peer_deps) = meta["peerDependencies"].as_object() {
                    for (dep_name, dep_ver) in peer_deps {
                        if let Some(dv) = dep_ver.as_str() {
                            node.peer_dependencies
                                .insert(dep_name.clone(), dv.to_string());
                        }
                    }
                }

                nodes.push(node);
            }
        }

        if nodes.is_empty() {
            return None;
        }

        self.cache.entry(key).or_default().extend(nodes);

        let cached = self.cache.get(name)?;
        Self::find_best_match(cached, version).cloned()
    }

    fn find_best_match<'a>(
        nodes: &'a [DependencyNode],
        version: &str,
    ) -> Option<&'a DependencyNode> {
        let range = SemverRange::parse(version)?;

        let candidates: Vec<&DependencyNode> = nodes
            .iter()
            .filter(|n| {
                if let Some(v) = Semver::parse(&n.version) {
                    return range.matches(&v);
                }
                false
            })
            .collect();

        candidates
            .into_iter()
            .max_by(|a, b| Semver::parse(&a.version).cmp(&Semver::parse(&b.version)))
    }

    fn resolve_conflicts(&self, _graph: &mut DependencyGraph) {}
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
    fn test_resolve_version() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DependencyNode::new(
            "lodash".to_string(),
            "4.17.21".to_string(),
        ));
        graph.add_node(DependencyNode::new(
            "lodash".to_string(),
            "4.17.20".to_string(),
        ));

        let v = graph.resolve_version("lodash", "^4.17.0");
        assert_eq!(v, Some("4.17.21".to_string()));
    }

    #[test]
    fn test_resolve_version_no_match() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DependencyNode::new(
            "lodash".to_string(),
            "3.10.1".to_string(),
        ));

        let v = graph.resolve_version("lodash", "^4.0.0");
        assert_eq!(v, None);
    }
}
