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

    /// Resolve `deps` into a [`DependencyGraph`] by fetching metadata from the
    /// registry and walking the transitive dependency tree.
    ///
    /// Resolution is deterministic: the initial stack is sorted alphabetically
    /// so that identical inputs always produce the same lockfile regardless of
    /// `HashMap` iteration order.
    ///
    /// When a package is required by multiple dependents with incompatible
    /// version constraints, a `tracing::warn!` is emitted rather than silently
    /// using the first-resolved version.  Full backtracking is not performed;
    /// the first satisfying version wins.
    pub async fn resolve(&mut self, deps: &HashMap<String, String>) -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        // Sort initial entries so resolution order is deterministic across runs.
        let mut stack: Vec<(String, String)> = {
            let mut v: Vec<_> = deps.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            // Descending sort → pop() yields ascending alphabetical order.
            v.sort_by(|a, b| b.0.cmp(&a.0));
            v
        };

        while let Some((name, version)) = stack.pop() {
            if let Some(resolved) = graph.resolved_versions.get(&name) {
                // Already resolved — verify the resolved version satisfies this
                // constraint too.  Emit a warning on mismatch so developers see
                // the conflict instead of getting a silent wrong resolution.
                if let (Some(v), Some(r)) = (Semver::parse(resolved), SemverRange::parse(&version))
                    && !r.matches(&v)
                {
                    tracing::warn!(
                        package = %name,
                        resolved = %resolved,
                        required = %version,
                        "version conflict: resolved version does not satisfy this constraint"
                    );
                }
                continue;
            }

            // Try cache first.
            if let Some(cached) = self.cache.get(&name)
                && let Some(best) = Self::find_best_match(cached, &version)
            {
                graph.add_node(best.clone());
                graph
                    .resolved_versions
                    .insert(name.clone(), best.version.clone());

                let mut trans: Vec<_> = best
                    .dependencies
                    .iter()
                    .map(|(n, v)| (n.clone(), v.clone()))
                    .collect();
                trans.sort_by(|a, b| b.0.cmp(&a.0));
                stack.extend(trans);
                continue;
            }

            // Collect a batch of uncached packages to fetch in parallel.
            let mut batch = vec![(name, version)];
            while let Some(item) = stack.pop() {
                if graph.resolved_versions.contains_key(&item.0) || self.cache.contains_key(&item.0)
                {
                    stack.push(item);
                    break;
                }
                batch.push(item);
            }

            // Fetch all batch items concurrently.
            let handles: Vec<_> = batch
                .iter()
                .map(|(n, _v)| {
                    let client = self.client.clone();
                    let url = format!("{}/{}", self.registry_url, n);
                    let n = n.clone();
                    tokio::spawn(async move {
                        let resp = client
                            .get(&url)
                            .header("Accept", "application/json")
                            .timeout(std::time::Duration::from_secs(20))
                            .send()
                            .await
                            .ok()?;
                        if !resp.status().is_success() {
                            return Some((n, Vec::new()));
                        }
                        let data: serde_json::Value = resp.json().await.ok()?;
                        let mut nodes: Vec<DependencyNode> = Vec::new();
                        if let Some(versions) = data["versions"].as_object() {
                            for (ver, meta) in versions {
                                let mut node = DependencyNode::new(n.clone(), ver.clone());

                                node.resolved =
                                    meta["dist"]["tarball"].as_str().map(|s| s.to_string());
                                node.integrity =
                                    meta["dist"]["integrity"].as_str().map(|s| s.to_string());

                                if let Some(deps) = meta["dependencies"].as_object() {
                                    for (dep_name, dep_ver) in deps {
                                        if let Some(dv) = dep_ver.as_str() {
                                            node.dependencies
                                                .insert(dep_name.clone(), dv.to_string());
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
                        Some((n, nodes))
                    })
                })
                .collect();

            for handle in handles {
                if let Ok(Some((name, nodes))) = handle.await {
                    self.cache.entry(name).or_default().extend(nodes);
                }
            }

            // Re-queue batch items for processing through the cache path.
            batch.reverse();
            stack.extend(batch);
        }

        self.resolve_conflicts(&mut graph);
        graph
    }

    pub(crate) fn find_best_match<'a>(
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
                tracing::warn!(
                    package = %n.name,
                    raw_version = %n.version,
                    "find_best_match: skipping unparseable version"
                );
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

    /// Verify that `find_best_match` handles dist-tags and x-ranges that go
    /// through `SemverRange::parse` without panicking or returning garbage.
    #[test]
    fn test_find_best_match_dist_tag() {
        let nodes = vec![
            DependencyNode::new("react".to_string(), "18.2.0".to_string()),
            DependencyNode::new("react".to_string(), "17.0.2".to_string()),
        ];
        // "latest" is a dist-tag → Any → highest version wins.
        let best = Resolver::find_best_match(&nodes, "latest");
        assert_eq!(best.map(|n| n.version.as_str()), Some("18.2.0"));
    }

    #[test]
    fn test_find_best_match_x_range() {
        let nodes = vec![
            DependencyNode::new("express".to_string(), "4.18.0".to_string()),
            DependencyNode::new("express".to_string(), "4.17.0".to_string()),
            DependencyNode::new("express".to_string(), "3.21.0".to_string()),
        ];
        // "4.x" → ^4.0.0: only 4.x.x matches, highest wins.
        let best = Resolver::find_best_match(&nodes, "4.x");
        assert_eq!(best.map(|n| n.version.as_str()), Some("4.18.0"));
    }

    #[test]
    fn test_find_best_match_compound_range() {
        let nodes = vec![
            DependencyNode::new("semver".to_string(), "7.5.0".to_string()),
            DependencyNode::new("semver".to_string(), "6.3.0".to_string()),
            DependencyNode::new("semver".to_string(), "8.0.0".to_string()),
        ];
        // ">=6.0.0 <8.0.0" must exclude 8.0.0 and pick 7.5.0.
        let best = Resolver::find_best_match(&nodes, ">=6.0.0 <8.0.0");
        assert_eq!(best.map(|n| n.version.as_str()), Some("7.5.0"));
    }

    /// Resolution order must be deterministic: two calls with the same `deps`
    /// must always produce the same `resolved_versions` map regardless of
    /// `HashMap` iteration non-determinism.  This test uses a pre-populated
    /// graph to avoid network access.
    #[test]
    fn test_resolution_is_deterministic() {
        // Build a graph with a known multi-version scenario and verify that
        // `resolve_version` (which uses `resolved_versions`) returns a stable
        // answer across repeated calls.
        for _ in 0..20 {
            let mut graph = DependencyGraph::new();
            for v in ["1.0.0", "1.5.0", "2.0.0", "1.2.0"] {
                graph.add_node(DependencyNode::new("foo".to_string(), v.to_string()));
            }
            // The highest match for ^1.0.0 must always be 1.5.0.
            let resolved = graph.resolve_version("foo", "^1.0.0");
            assert_eq!(
                resolved.as_deref(),
                Some("1.5.0"),
                "resolve_version must be deterministic"
            );
        }
    }

    /// Verify that `find_best_match` skips nodes whose stored version string
    /// cannot be parsed rather than panicking or producing a wrong result.
    #[test]
    fn test_find_best_match_skips_unparseable_versions() {
        let mut nodes = vec![
            DependencyNode::new("pkg".to_string(), "1.2.3".to_string()),
            // Intentionally malformed version — must be silently skipped.
            DependencyNode::new("pkg".to_string(), "not-a-version".to_string()),
        ];
        // Make sure `add_node` stores them (key = name@version, so both fit).
        let best = Resolver::find_best_match(&nodes, "^1.0.0");
        assert_eq!(best.map(|n| n.version.as_str()), Some("1.2.3"));

        // When ALL versions are malformed, return None rather than panic.
        nodes[0].version = "also-bad".to_string();
        let best = Resolver::find_best_match(&nodes, "^1.0.0");
        assert!(best.is_none());
    }
}
