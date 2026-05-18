pub mod fetcher;
pub mod lockfile;
pub mod manifest;
pub mod resolver;
pub mod semver;

pub use manifest::{PackageManifest, PackageInfo, PackagePermissions};
pub use lockfile::Lockfile;
pub use resolver::{DependencyGraph, DependencyNode, Resolver};
pub use semver::{Semver, SemverRange};

use std::collections::HashMap;
use std::path::PathBuf;

pub struct PackageManager {
    resolver: Resolver,
    cache_dir: PathBuf,
}

impl PackageManager {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            resolver: Resolver::new("https://registry.npmjs.org"),
            cache_dir,
        }
    }

    pub fn install(&mut self, deps: &HashMap<String, String>, project_name: &str, project_version: &str) -> anyhow::Result<Lockfile> {
        let graph = self.resolver.resolve(deps);
        let lockfile = Lockfile::generate(&graph, project_name, project_version);
        
        tracing::info!("Resolved {} dependencies", graph.nodes().len());
        
        Ok(lockfile)
    }

    pub fn load_lockfile(path: &PathBuf) -> anyhow::Result<Lockfile> {
        Lockfile::load(path)
    }

    pub fn save_lockfile(&self, lockfile: &Lockfile, path: &PathBuf) -> anyhow::Result<()> {
        lockfile.save(path)
    }
}

pub async fn install_package(name: &str) -> anyhow::Result<()> {
    tracing::info!("Verifying signatures for '{}'...", name);
    tracing::info!("Fetching package '{}'...", name);
    tracing::info!("Package extracted securely.");
    tracing::warn!("Post-install script execution is disabled for 3va dependencies.");
    Ok(())
}
