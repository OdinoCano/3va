use std::path::{Path, PathBuf};

/// Global content-addressable store at `~/.3va/store/`.
///
/// Each unique `{registry}/{name}@{version}` is extracted exactly once.
/// Projects hard-link (or copy on cross-filesystem) files from the store into
/// their `node_modules/`, so disk space is shared across every project on the
/// machine.
///
/// ## Concurrency safety
/// `store_tarball` uses an atomic `rename(2)` so two concurrent processes
/// writing the same package cannot corrupt the store.  The last writer wins
/// (idempotent).
pub struct ContentStore {
    root: PathBuf,
}

impl Clone for ContentStore {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
        }
    }
}

impl ContentStore {
    pub fn global() -> Self {
        if let Ok(custom) = std::env::var("_3VA_STORE")
            && !custom.is_empty()
        {
            return Self {
                root: PathBuf::from(custom),
            };
        }
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        Self {
            root: PathBuf::from(home).join(".3va").join("store"),
        }
    }

    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the canonical store path for a package.
    /// The directory exists iff the package is already cached.
    pub fn package_path(&self, registry: &str, name: &str, version: &str) -> PathBuf {
        self.root
            .join(safe_registry(registry))
            .join(format!("{}@{}", safe_name(name), version))
    }

    /// True if the package is fully cached (package.json present in the store).
    pub fn is_cached(&self, registry: &str, name: &str, version: &str) -> bool {
        self.package_path(registry, name, version)
            .join("package.json")
            .exists()
    }

    /// Extract `tarball` bytes into the store **atomically** and return the
    /// store path.  Callers must verify integrity before calling.
    ///
    /// Uses a tmp dir + `rename(2)` so a crash or concurrent write can never
    /// leave a half-extracted package in the store.
    pub fn store_tarball(
        &self,
        tarball: &[u8],
        registry: &str,
        name: &str,
        version: &str,
    ) -> anyhow::Result<PathBuf> {
        let dest = self.package_path(registry, name, version);

        // Fast path: already stored (race-free check — package.json is written
        // last during extraction so its presence means the entry is complete).
        if dest.join("package.json").exists() {
            return Ok(dest);
        }

        // Create store root & registry subdirectory.
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Extract into a temporary sibling directory.
        let tmp = dest.with_file_name(format!(
            "{}@{}__tmp_{}",
            safe_name(name),
            version,
            std::process::id()
        ));
        if tmp.exists() {
            std::fs::remove_dir_all(&tmp)?;
        }

        if let Err(e) = crate::extract_tarball(tarball, &tmp) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }

        // Atomic rename — if two processes race the last writer wins; both are
        // identical so neither can corrupt the store.
        match std::fs::rename(&tmp, &dest) {
            Ok(()) => {}
            Err(_) => {
                // Another process won the race; clean up our copy.
                let _ = std::fs::remove_dir_all(&tmp);
                if !dest.join("package.json").exists() {
                    anyhow::bail!(
                        "Failed to store {}@{}: rename failed and dest is incomplete",
                        name,
                        version
                    );
                }
            }
        }

        Ok(dest)
    }

    /// Hard-link (or copy) the package from the store into `node_modules/{name}`.
    ///
    /// Hard links share the inode, so no extra disk space is used as long as
    /// the store and the project are on the same filesystem.  A transparent
    /// copy fallback handles cross-filesystem cases.
    pub fn link_to_node_modules(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        node_modules: &Path,
    ) -> anyhow::Result<()> {
        let src = self.package_path(registry, name, version);
        if !src.exists() {
            anyhow::bail!(
                "Package {}@{} is not in the store — call store_tarball first",
                name,
                version
            );
        }
        let dst = if name.contains('/') {
            let dst = node_modules.join(name);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            dst
        } else {
            node_modules.join(name)
        };

        if dst.exists() {
            std::fs::remove_dir_all(&dst)?;
        }
        link_or_copy_dir(&src, &dst)
    }

    /// Link from the global store into the per-project virtual store at
    /// `node_modules/.3va/{entry}@{version}/node_modules/{name}/`.
    ///
    /// This mirrors pnpm's `node_modules/.pnpm/` layout: actual files live here
    /// (hard-linked from `~/.3va/store`), and the caller creates a symlink from
    /// `node_modules/{name}` pointing back here.  The result is that the project's
    /// top-level `node_modules/` contains only symlinks — the real bytes are shared.
    ///
    /// Returns the path to the package directory inside `.3va/` so the caller can
    /// build the symlink target.
    pub fn link_to_virtual_store(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        node_modules: &Path,
    ) -> anyhow::Result<PathBuf> {
        let src = self.package_path(registry, name, version);
        if !src.exists() {
            anyhow::bail!(
                "Package {}@{} not in global store — call store_tarball first",
                name,
                version
            );
        }

        // node_modules/.3va/@scope+pkg@version/node_modules/@scope/pkg/
        let entry = format!("{}@{}", virtual_entry_name(name), version);
        let virtual_pkg_dir = node_modules
            .join(".3va")
            .join(&entry)
            .join("node_modules")
            .join(name); // preserves @scope/pkg directory structure

        if virtual_pkg_dir.join("package.json").exists() {
            return Ok(virtual_pkg_dir); // already linked — idempotent
        }

        if let Some(parent) = virtual_pkg_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if virtual_pkg_dir.exists() {
            std::fs::remove_dir_all(&virtual_pkg_dir)?;
        }
        link_or_copy_dir(&src, &virtual_pkg_dir)?;
        Ok(virtual_pkg_dir)
    }

    // ── Maintenance ───────────────────────────────────────────────────────────

    /// Verify every entry in the store has a `package.json` (i.e., was
    /// extracted completely).  Returns a list of corrupt entries.
    pub fn verify(&self) -> Vec<PathBuf> {
        let mut corrupt = Vec::new();
        if let Ok(reg_entries) = std::fs::read_dir(&self.root) {
            for reg in reg_entries.flatten() {
                if !reg.path().is_dir() {
                    continue;
                }
                if let Ok(pkg_entries) = std::fs::read_dir(reg.path()) {
                    for pkg in pkg_entries.flatten() {
                        let p = pkg.path();
                        if p.is_dir() && !p.join("package.json").exists() {
                            corrupt.push(p);
                        }
                    }
                }
            }
        }
        corrupt
    }

    /// Remove corrupt entries (incomplete extractions left by a prior crash).
    pub fn repair(&self) -> anyhow::Result<usize> {
        let corrupt = self.verify();
        let count = corrupt.len();
        for path in &corrupt {
            tracing::warn!("Removing corrupt store entry: {}", path.display());
            std::fs::remove_dir_all(path)?;
        }
        Ok(count)
    }

    /// Remove store entries that are NOT in any of the provided `keep` sets.
    ///
    /// `keep` is a set of `(registry, name, version)` strings in the form
    /// `"{safe_registry}/{safe_name}@{version}"` (the path segment under
    /// `~/.3va/store/`).  Build it by reading all project lockfiles you want
    /// to keep alive.
    pub fn prune(&self, keep: &std::collections::HashSet<String>) -> anyhow::Result<PruneResult> {
        let mut removed = 0usize;
        let mut freed_bytes = 0u64;

        if !self.root.exists() {
            return Ok(PruneResult {
                removed,
                freed_bytes,
            });
        }

        if let Ok(reg_entries) = std::fs::read_dir(&self.root) {
            for reg in reg_entries.flatten() {
                let reg_path = reg.path();
                if !reg_path.is_dir() {
                    continue;
                }
                let reg_name = reg.file_name().to_string_lossy().to_string();

                if let Ok(pkg_entries) = std::fs::read_dir(&reg_path) {
                    for pkg in pkg_entries.flatten() {
                        let pkg_path = pkg.path();
                        if !pkg_path.is_dir() {
                            continue;
                        }
                        let pkg_name = pkg.file_name().to_string_lossy().to_string();
                        let key = format!("{}/{}", reg_name, pkg_name);
                        if !keep.contains(&key) {
                            let size = dir_size(&pkg_path);
                            std::fs::remove_dir_all(&pkg_path)?;
                            removed += 1;
                            freed_bytes += size;
                        }
                    }
                }
            }
        }

        Ok(PruneResult {
            removed,
            freed_bytes,
        })
    }

    /// Collect `{safe_registry}/{safe_name}@{version}` keys for all deps in a lockfile.
    pub fn keys_from_lockfile(lockfile: &crate::Lockfile) -> std::collections::HashSet<String> {
        let mut keys = std::collections::HashSet::new();
        for (pkg_name, dep) in &lockfile.dependencies {
            if let Some(reg) = dep.registry.as_deref() {
                let key = format!(
                    "{}/{}@{}",
                    safe_registry(reg),
                    safe_name(pkg_name),
                    dep.version
                );
                keys.insert(key);
            }
        }
        keys
    }

    /// Human-readable statistics about the global store.
    pub fn stats(&self) -> StoreStats {
        let mut total_packages = 0usize;
        let mut total_bytes = 0u64;

        if let Ok(reg_entries) = std::fs::read_dir(&self.root) {
            for reg in reg_entries.flatten() {
                if !reg.path().is_dir() {
                    continue;
                }
                if let Ok(pkg_entries) = std::fs::read_dir(reg.path()) {
                    for pkg in pkg_entries.flatten() {
                        if pkg.path().is_dir() {
                            total_packages += 1;
                            total_bytes += dir_size(&pkg.path());
                        }
                    }
                }
            }
        }

        StoreStats {
            total_packages,
            total_bytes,
            store_path: self.root.clone(),
        }
    }
}

pub struct StoreStats {
    pub total_packages: usize,
    pub total_bytes: u64,
    pub store_path: PathBuf,
}

pub struct PruneResult {
    pub removed: usize,
    pub freed_bytes: u64,
}

impl StoreStats {
    pub fn human_size(&self) -> String {
        fmt_bytes(self.total_bytes)
    }
}

impl PruneResult {
    pub fn human_freed(&self) -> String {
        fmt_bytes(self.freed_bytes)
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

pub(crate) fn safe_name(name: &str) -> String {
    name.replace('/', "+")
}

pub(crate) fn safe_registry(registry: &str) -> String {
    registry
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(registry)
        .to_string()
}

fn fmt_bytes(b: u64) -> String {
    if b < 1024 {
        format!("{} B", b)
    } else if b < 1024 * 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else if b < 1024 * 1024 * 1024 {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", b as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Recursively hard-link every file from `src` into `dst`.
/// Falls back to `fs::copy` for files that fail hard-linking (different mount).
/// Encode a package name for use as a virtual-store directory entry.
///
/// `@scope/pkg` → `@scope+pkg`  (mirrors pnpm's convention)
pub fn virtual_entry_name(name: &str) -> String {
    name.replace('/', "+")
}

pub(crate) fn link_or_copy_dir(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)
        .map_err(|e| anyhow::anyhow!("Cannot read store dir {}: {}", src.display(), e))?
    {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            link_or_copy_dir(&src_path, &dst_path)?;
        } else if std::fs::hard_link(&src_path, &dst_path).is_err() {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                size += p.metadata().map(|m| m.len()).unwrap_or(0);
            } else if p.is_dir() {
                size += dir_size(&p);
            }
        }
    }
    size
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_scoped() {
        assert_eq!(safe_name("@scope/pkg"), "@scope+pkg");
        assert_eq!(safe_name("lodash"), "lodash");
    }

    #[test]
    fn safe_registry_strips_scheme_and_path() {
        assert_eq!(
            safe_registry("https://registry.npmjs.org"),
            "registry.npmjs.org"
        );
        assert_eq!(safe_registry("registry.npmjs.org"), "registry.npmjs.org");
        assert_eq!(safe_registry("https://jsr.io/api/something"), "jsr.io");
    }

    #[test]
    fn store_stats_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(dir.path().to_path_buf());
        let stats = store.stats();
        assert_eq!(stats.total_packages, 0);
        assert_eq!(stats.total_bytes, 0);
    }

    #[test]
    fn package_path_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(dir.path().to_path_buf());
        let p1 = store.package_path("registry.npmjs.org", "lodash", "4.17.21");
        let p2 = store.package_path("registry.npmjs.org", "lodash", "4.17.21");
        assert_eq!(p1, p2);
    }

    #[test]
    fn package_path_scoped_package() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(dir.path().to_path_buf());
        let p = store.package_path("registry.npmjs.org", "@scope/pkg", "1.0.0");
        assert!(p.to_string_lossy().contains("@scope+pkg@1.0.0"));
    }

    #[test]
    fn link_or_copy_dir_copies_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("index.js"), b"export default 42").unwrap();

        let dst = tmp.path().join("dst");
        link_or_copy_dir(&src, &dst).unwrap();

        let content = std::fs::read_to_string(dst.join("index.js")).unwrap();
        assert_eq!(content, "export default 42");
    }

    #[test]
    fn link_or_copy_dir_is_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("lib")).unwrap();
        std::fs::write(src.join("lib").join("util.js"), b"// util").unwrap();

        link_or_copy_dir(&src, &tmp.path().join("dst")).unwrap();
        assert!(tmp.path().join("dst/lib/util.js").exists());
    }

    #[test]
    fn verify_detects_incomplete_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(tmp.path().to_path_buf());
        // Simulate a crash: directory exists but package.json is missing
        let path = store.package_path("registry.npmjs.org", "bad-pkg", "1.0.0");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("index.js"), b"").unwrap();

        let corrupt = store.verify();
        assert_eq!(corrupt.len(), 1);
    }

    #[test]
    fn verify_passes_complete_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(tmp.path().to_path_buf());
        let path = store.package_path("registry.npmjs.org", "good-pkg", "1.0.0");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("package.json"), b"{}").unwrap();

        assert!(store.verify().is_empty());
    }

    #[test]
    fn repair_removes_corrupt_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(tmp.path().to_path_buf());
        let path = store.package_path("registry.npmjs.org", "bad-pkg", "1.0.0");
        std::fs::create_dir_all(&path).unwrap();

        let removed = store.repair().unwrap();
        assert_eq!(removed, 1);
        assert!(!path.exists());
    }

    #[test]
    fn prune_removes_unlisted_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(tmp.path().to_path_buf());

        // Store two packages
        for (name, ver) in [("keep-me", "1.0.0"), ("remove-me", "1.0.0")] {
            let path = store.package_path("registry.npmjs.org", name, ver);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("package.json"), b"{}").unwrap();
        }

        let mut keep = std::collections::HashSet::new();
        keep.insert("registry.npmjs.org/keep-me@1.0.0".to_string());

        let result = store.prune(&keep).unwrap();
        assert_eq!(result.removed, 1);
        assert!(store.is_cached("registry.npmjs.org", "keep-me", "1.0.0"));
        assert!(!store.is_cached("registry.npmjs.org", "remove-me", "1.0.0"));
    }

    #[test]
    fn human_size_formats_correctly() {
        let s = StoreStats {
            total_packages: 1,
            total_bytes: 500,
            store_path: PathBuf::from("/"),
        };
        assert!(s.human_size().ends_with("B"));

        let s = StoreStats {
            total_packages: 1,
            total_bytes: 2048,
            store_path: PathBuf::from("/"),
        };
        assert!(s.human_size().contains("KB"));

        let s = StoreStats {
            total_packages: 1,
            total_bytes: 3 * 1024 * 1024,
            store_path: PathBuf::from("/"),
        };
        assert!(s.human_size().contains("MB"));
    }

    #[test]
    fn is_cached_false_for_incomplete_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(tmp.path().to_path_buf());
        let path = store.package_path("registry.npmjs.org", "partial", "1.0.0");
        std::fs::create_dir_all(&path).unwrap();
        // No package.json → not cached
        assert!(!store.is_cached("registry.npmjs.org", "partial", "1.0.0"));
    }

    // ── virtual store ─────────────────────────────────────────────────────────

    #[test]
    fn virtual_entry_name_encodes_scope() {
        assert_eq!(virtual_entry_name("lodash"), "lodash");
        assert_eq!(virtual_entry_name("@scope/pkg"), "@scope+pkg");
        assert_eq!(virtual_entry_name("@a/b"), "@a+b");
    }

    #[test]
    fn link_to_virtual_store_creates_correct_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let store_root = tmp.path().join("store");
        let store = ContentStore::with_root(store_root);
        let node_modules = tmp.path().join("nm");
        std::fs::create_dir_all(&node_modules).unwrap();

        // Seed global store with a fake package.
        let pkg_path = store.package_path("npm", "lodash", "4.17.21");
        std::fs::create_dir_all(&pkg_path).unwrap();
        std::fs::write(
            pkg_path.join("package.json"),
            r#"{"name":"lodash","version":"4.17.21"}"#,
        )
        .unwrap();
        std::fs::write(pkg_path.join("index.js"), "module.exports = {};").unwrap();

        let vpath = store
            .link_to_virtual_store("npm", "lodash", "4.17.21", &node_modules)
            .unwrap();

        // Virtual store entry exists at the expected location.
        assert_eq!(
            vpath,
            node_modules
                .join(".3va")
                .join("lodash@4.17.21")
                .join("node_modules")
                .join("lodash")
        );
        assert!(
            vpath.join("package.json").exists(),
            "package.json must be in virtual store"
        );
        assert!(
            vpath.join("index.js").exists(),
            "index.js must be in virtual store"
        );
    }

    #[test]
    fn link_to_virtual_store_scoped_package_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(tmp.path().join("store"));
        let node_modules = tmp.path().join("nm");
        std::fs::create_dir_all(&node_modules).unwrap();

        let pkg_path = store.package_path("npm", "@babel/core", "7.24.0");
        std::fs::create_dir_all(&pkg_path).unwrap();
        std::fs::write(
            pkg_path.join("package.json"),
            r#"{"name":"@babel/core","version":"7.24.0"}"#,
        )
        .unwrap();

        let vpath = store
            .link_to_virtual_store("npm", "@babel/core", "7.24.0", &node_modules)
            .unwrap();

        // Entry dir encodes scope with '+': .3va/@babel+core@7.24.0/node_modules/@babel/core/
        let expected = node_modules
            .join(".3va")
            .join("@babel+core@7.24.0")
            .join("node_modules")
            .join("@babel")
            .join("core");
        assert_eq!(vpath, expected);
        assert!(vpath.join("package.json").exists());
    }

    #[test]
    fn link_to_virtual_store_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(tmp.path().join("store"));
        let node_modules = tmp.path().join("nm");
        std::fs::create_dir_all(&node_modules).unwrap();

        let pkg_path = store.package_path("npm", "ms", "2.1.3");
        std::fs::create_dir_all(&pkg_path).unwrap();
        std::fs::write(
            pkg_path.join("package.json"),
            r#"{"name":"ms","version":"2.1.3"}"#,
        )
        .unwrap();

        // Calling twice must not error.
        store
            .link_to_virtual_store("npm", "ms", "2.1.3", &node_modules)
            .unwrap();
        store
            .link_to_virtual_store("npm", "ms", "2.1.3", &node_modules)
            .unwrap();
    }
}
