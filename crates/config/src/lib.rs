//! `3va.config.ts` / `3va.config.js` / `3va.config.json` loader.
//!
//! Searches the current directory and parents for a config file, deserialises
//! it, and exposes a merged view that CLI flags and `3VA_*` env-var overrides
//! are applied on top of.
//!
//! # Examples
//!
//! ```
//! use vvva_config::ProjectConfig;
//!
//! // Returns Ok(None) when no config file is present.
//! let cfg = ProjectConfig::discover(std::env::current_dir().unwrap_or_default()).unwrap();
//! let dev_port = cfg.map(|c| c.dev.port).unwrap_or(3000);
//! assert!(dev_port > 0);
//! ```

pub mod env_override;
pub mod loader;
pub mod schema;

pub use schema::ProjectConfig;

use std::path::{Path, PathBuf};

/// Candidate filenames searched in order.
const CONFIG_NAMES: &[&str] = &["3va.config.ts", "3va.config.js", "3va.config.json"];

/// Walk from `start` toward the filesystem root, returning the first config
/// file path found (or `None`).
pub fn find_config_file(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        for name in CONFIG_NAMES {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

impl ProjectConfig {
    /// Locate and load the project config, returning `None` if no config file
    /// exists. CLI-flag overrides are not applied here; callers merge them
    /// after loading.
    pub fn discover(cwd: PathBuf) -> anyhow::Result<Option<Self>> {
        match find_config_file(&cwd) {
            None => Ok(None),
            Some(path) => {
                let cfg = loader::load(&path)?;
                Ok(Some(env_override::apply(cfg)))
            }
        }
    }
}
