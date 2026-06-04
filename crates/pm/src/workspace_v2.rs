//! Workspace v2.0.0 — topological script execution, affected-only mode,
//! graph visualization, and per-package permission scopes.
//!
//! # Topological execution
//!
//! Scripts run in dependency order: if package A depends on package B (via
//! `workspace:*`), B runs first. Packages with no dependency relationship
//! run concurrently up to `parallelism` slots.
//!
//! # Affected-only mode
//!
//! `--affected [--base=main]` detects which packages have changed since the
//! merge base with `--base` via `git diff --name-only`.  A package is
//! "affected" if any of its files changed OR any of its transitive deps
//! are affected.
//!
//! # ASCII graph
//!
//! `3va workspace graph` emits a topological DAG with arrows showing
//! dependency edges.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use crate::workspace::WorkspacePackage;

// ── Dependency graph ──────────────────────────────────────────────────────────

/// Directed-acyclic graph of workspace packages.
/// Edges: `edges[pkg_name] = set of packages that pkg_name depends on`.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceGraph {
    /// `name → package`
    pub packages: HashMap<String, WorkspacePackage>,
    /// `name → set of names it depends on (workspace-local deps only)`
    pub edges: HashMap<String, HashSet<String>>,
}

impl WorkspaceGraph {
    /// Build a `WorkspaceGraph` from the resolved package list.
    /// Cross-package deps are detected via `workspace:*` / `workspace:^` in
    /// the raw `package.json`.
    pub fn build(root: &Path, packages: &[WorkspacePackage]) -> anyhow::Result<Self> {
        let name_set: HashSet<String> = packages.iter().map(|p| p.name.clone()).collect();
        let mut edges: HashMap<String, HashSet<String>> = HashMap::new();
        let mut pkg_map: HashMap<String, WorkspacePackage> = HashMap::new();

        for pkg in packages {
            let pkg_json_path = pkg.path.join("package.json");
            let content = std::fs::read_to_string(&pkg_json_path).unwrap_or_else(|_| "{}".into());
            let val: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();

            let mut deps = HashSet::new();
            for section in ["dependencies", "devDependencies", "peerDependencies"] {
                if let Some(obj) = val[section].as_object() {
                    for (dep_name, ver) in obj {
                        let ver_s = ver.as_str().unwrap_or("");
                        if (ver_s.starts_with("workspace:") || ver_s == "*")
                            && name_set.contains(dep_name.as_str())
                        {
                            deps.insert(dep_name.clone());
                        }
                    }
                }
            }
            edges.insert(pkg.name.clone(), deps);
            pkg_map.insert(pkg.name.clone(), pkg.clone());
        }

        let _ = root; // keep for future use (e.g. lockfile root)
        Ok(Self {
            packages: pkg_map,
            edges,
        })
    }

    /// Kahn's algorithm: return packages in topological order (deps first).
    /// Returns `Err` if a cycle is detected.
    pub fn topological_order(&self) -> anyhow::Result<Vec<String>> {
        // in-degree: how many packages depend ON this package
        let mut in_degree: HashMap<&str, usize> =
            self.packages.keys().map(|n| (n.as_str(), 0)).collect();
        // reverse: who depends on me
        let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();

        for (pkg, deps) in &self.edges {
            for dep in deps {
                *in_degree.entry(pkg.as_str()).or_insert(0) += 1;
                rev.entry(dep.as_str()).or_default().push(pkg.as_str());
            }
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(&n, _)| n)
            .collect();
        queue.make_contiguous().sort(); // deterministic order

        let mut order = Vec::new();
        while let Some(pkg) = queue.pop_front() {
            order.push(pkg.to_string());
            if let Some(dependents) = rev.get(pkg) {
                let mut dependents = dependents.clone();
                dependents.sort();
                for dep in dependents {
                    let d = in_degree.get_mut(dep).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(dep);
                    }
                }
            }
        }

        if order.len() != self.packages.len() {
            anyhow::bail!("Workspace has cyclic dependencies");
        }
        Ok(order)
    }

    /// Compute which packages are "affected" given a set of changed paths.
    /// A package is affected if any file inside its directory was changed
    /// OR if any of its transitive workspace deps are affected.
    pub fn affected_packages(&self, changed_paths: &[String]) -> HashSet<String> {
        // Step 1: directly changed packages
        let mut affected: HashSet<String> = self
            .packages
            .iter()
            .filter(|(_, pkg)| {
                changed_paths
                    .iter()
                    .any(|p| p.starts_with(pkg.path.to_string_lossy().as_ref()))
            })
            .map(|(n, _)| n.clone())
            .collect();

        // Step 2: propagate transitively (BFS over reverse edges)
        let mut queue: VecDeque<String> = affected.iter().cloned().collect();
        // Build reverse dependency map: dep → set of packages that depend on it
        let mut rev: HashMap<String, Vec<String>> = HashMap::new();
        for (pkg, deps) in &self.edges {
            for dep in deps {
                rev.entry(dep.clone()).or_default().push(pkg.clone());
            }
        }
        while let Some(pkg) = queue.pop_front() {
            if let Some(dependents) = rev.get(&pkg) {
                for dep in dependents {
                    if affected.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }
        affected
    }

    /// Render an ASCII dependency graph.
    pub fn ascii_graph(&self) -> String {
        let order = self.topological_order().unwrap_or_else(|_| {
            let mut v: Vec<String> = self.packages.keys().cloned().collect();
            v.sort();
            v
        });
        let mut out = String::new();
        for pkg in &order {
            let deps = self.edges.get(pkg).cloned().unwrap_or_default();
            if deps.is_empty() {
                out.push_str(&format!("  {pkg}\n"));
            } else {
                let mut dep_list: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
                dep_list.sort();
                out.push_str(&format!("  {pkg}  ←  {}\n", dep_list.join(", ")));
            }
        }
        out
    }
}

// ── Affected-file detection via git ──────────────────────────────────────────

/// Run `git diff --name-only <base>...HEAD` and return relative file paths.
pub fn git_changed_files(repo_root: &Path, base: &str) -> anyhow::Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .current_dir(repo_root)
        .args(["diff", "--name-only", &format!("{base}...HEAD")])
        .output()
        .map_err(|e| anyhow::anyhow!("git not found: {e}"))?;

    if !output.status.success() {
        anyhow::bail!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let files: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|l| repo_root.join(l).to_string_lossy().to_string())
        .collect();
    Ok(files)
}

// ── Parallel topological execution ───────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStatus {
    Success,
    Failed(String),
    Skipped,
}

#[derive(Debug, Clone)]
pub struct PackageRunResult {
    pub name: String,
    pub status: RunStatus,
    pub duration_ms: u64,
}

/// Execute `script` in each package in topological order.
///
/// `parallelism` — max concurrently running packages.  Packages with no
/// unfulfilled dependency run in parallel up to this limit.
pub async fn run_workspace_script(
    _root: &Path,
    graph: &WorkspaceGraph,
    script: &str,
    affected_only: Option<&HashSet<String>>,
    parallelism: usize,
) -> anyhow::Result<Vec<PackageRunResult>> {
    let order = graph.topological_order()?;
    let parallelism = parallelism.max(1);
    let mut results: Vec<PackageRunResult> = Vec::new();

    // Chunks: respect topo order and limit parallelism.
    // Simple approach: process in topo order with up-to-N concurrent tasks,
    // releasing slots when each package completes.
    let mut remaining = order.clone();
    let mut completed: HashSet<String> = HashSet::new();

    while !remaining.is_empty() {
        // Find packages whose deps are all complete.
        let ready: Vec<String> = remaining
            .iter()
            .filter(|pkg| {
                let deps = graph.edges.get(*pkg).cloned().unwrap_or_default();
                deps.iter().all(|d| completed.contains(d))
            })
            .cloned()
            .collect();

        let batch: Vec<String> = ready.iter().take(parallelism).cloned().collect();
        if batch.is_empty() {
            // Cycle or everything done.
            break;
        }

        let mut tasks = Vec::new();
        for pkg_name in &batch {
            // Skip if not in the affected set.
            if let Some(aff) = affected_only
                && !aff.contains(pkg_name)
            {
                completed.insert(pkg_name.clone());
                results.push(PackageRunResult {
                    name: pkg_name.clone(),
                    status: RunStatus::Skipped,
                    duration_ms: 0,
                });
                remaining.retain(|n| n != pkg_name);
                continue;
            }

            // Check if this package has the script defined.
            let pkg = match graph.packages.get(pkg_name) {
                Some(p) => p.clone(),
                None => continue,
            };

            let pkg_json_path = pkg.path.join("package.json");
            let has_script = std::fs::read_to_string(&pkg_json_path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| v["scripts"][script].as_str().map(String::from))
                .is_some();

            if !has_script {
                completed.insert(pkg_name.clone());
                results.push(PackageRunResult {
                    name: pkg_name.clone(),
                    status: RunStatus::Skipped,
                    duration_ms: 0,
                });
                remaining.retain(|n| n != pkg_name);
                continue;
            }

            let pkg_dir = pkg.path.clone();
            let script_name = script.to_string();
            let pkg_name_clone = pkg_name.clone();

            tasks.push(tokio::spawn(async move {
                let start = std::time::Instant::now();
                let result = std::process::Command::new("npm")
                    .current_dir(&pkg_dir)
                    .args(["run", &script_name])
                    .status();

                let duration_ms = start.elapsed().as_millis() as u64;
                match result {
                    Ok(status) if status.success() => PackageRunResult {
                        name: pkg_name_clone,
                        status: RunStatus::Success,
                        duration_ms,
                    },
                    Ok(status) => PackageRunResult {
                        name: pkg_name_clone,
                        status: RunStatus::Failed(format!("exit code {:?}", status.code())),
                        duration_ms,
                    },
                    Err(e) => PackageRunResult {
                        name: pkg_name_clone,
                        status: RunStatus::Failed(e.to_string()),
                        duration_ms,
                    },
                }
            }));
        }

        for task in tasks {
            if let Ok(r) = task.await {
                let failed = matches!(r.status, RunStatus::Failed(_));
                completed.insert(r.name.clone());
                remaining.retain(|n| *n != r.name);
                results.push(r);

                if failed {
                    // Mark all remaining packages that depend (transitively) on
                    // this failure as skipped.
                    let failed_name = results.last().unwrap().name.clone();
                    let to_skip: Vec<String> = remaining
                        .iter()
                        .filter(|n| {
                            graph
                                .edges
                                .get(*n)
                                .map(|d| d.contains(&failed_name))
                                .unwrap_or(false)
                        })
                        .cloned()
                        .collect();
                    for skipped in to_skip {
                        completed.insert(skipped.clone());
                        remaining.retain(|n| *n != skipped);
                        results.push(PackageRunResult {
                            name: skipped,
                            status: RunStatus::Skipped,
                            duration_ms: 0,
                        });
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Print a summary table similar to the spec.
pub fn print_run_results(script: &str, results: &[PackageRunResult]) {
    let count = results.len();
    println!("\n[workspace] {script} — {count} packages");
    for r in results {
        let symbol = match &r.status {
            RunStatus::Success => "\x1b[32m✓\x1b[0m".to_string(),
            RunStatus::Failed(msg) => format!("\x1b[31m✗\x1b[0m ({msg})"),
            RunStatus::Skipped => "\x1b[33m↷\x1b[0m skipped".to_string(),
        };
        println!("  {symbol} {}", r.name);
    }
    println!();
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_pkg(name: &str) -> WorkspacePackage {
        WorkspacePackage {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            path: PathBuf::from(format!("/fake/{name}")),
            all_deps: HashMap::new(),
        }
    }

    fn simple_graph() -> WorkspaceGraph {
        // utils ← core ← api
        //               ↑
        //          frontend
        let packages: HashMap<String, WorkspacePackage> = [
            ("utils", make_pkg("utils")),
            ("core", make_pkg("core")),
            ("api", make_pkg("api")),
            ("frontend", make_pkg("frontend")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        let edges: HashMap<String, HashSet<String>> = [
            ("utils", vec![]),
            ("core", vec!["utils"]),
            ("api", vec!["core"]),
            ("frontend", vec!["core"]),
        ]
        .into_iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                v.into_iter().map(|s| s.to_string()).collect(),
            )
        })
        .collect();

        WorkspaceGraph { packages, edges }
    }

    #[test]
    fn topological_order_deps_before_dependents() {
        let g = simple_graph();
        let order = g.topological_order().unwrap();
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        // utils must come before core
        assert!(pos("utils") < pos("core"), "utils before core");
        // core must come before api and frontend
        assert!(pos("core") < pos("api"), "core before api");
        assert!(pos("core") < pos("frontend"), "core before frontend");
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn affected_packages_propagates_transitively() {
        let g = simple_graph();
        // "utils" changed → core, api, frontend are also affected
        let changed = vec!["/fake/utils/src/index.ts".to_string()];
        let affected = g.affected_packages(&changed);
        assert!(affected.contains("utils"));
        assert!(affected.contains("core"));
        assert!(affected.contains("api"));
        assert!(affected.contains("frontend"));
    }

    #[test]
    fn affected_packages_isolated_change() {
        let g = simple_graph();
        // only "api" changed, nothing depends on it
        let changed = vec!["/fake/api/src/main.ts".to_string()];
        let affected = g.affected_packages(&changed);
        assert!(affected.contains("api"));
        assert!(!affected.contains("utils"));
        assert!(!affected.contains("frontend"));
    }

    #[test]
    fn ascii_graph_contains_all_packages() {
        let g = simple_graph();
        let graph_str = g.ascii_graph();
        assert!(graph_str.contains("utils"));
        assert!(graph_str.contains("core"));
        assert!(graph_str.contains("api"));
        assert!(graph_str.contains("frontend"));
        // dep arrows
        assert!(graph_str.contains("←"));
    }

    #[test]
    fn cycle_detection() {
        // a → b → a
        let packages: HashMap<String, WorkspacePackage> =
            [("a", make_pkg("a")), ("b", make_pkg("b"))]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect();
        let edges: HashMap<String, HashSet<String>> = [("a", vec!["b"]), ("b", vec!["a"])]
            .into_iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    v.into_iter().map(|s| s.to_string()).collect(),
                )
            })
            .collect();
        let g = WorkspaceGraph { packages, edges };
        assert!(g.topological_order().is_err());
    }
}
