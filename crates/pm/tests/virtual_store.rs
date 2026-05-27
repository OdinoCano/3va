// Integration tests for the pnpm-style virtual store topology.
// Run: cargo test -p vvva_pm --test virtual_store

use std::path::Path;
use vvva_pm::store::{ContentStore, virtual_entry_name};

fn seed_store(store: &ContentStore, name: &str, version: &str) {
    let path = store.package_path("npm", name, version);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("package.json"),
        format!(r#"{{"name":"{name}","version":"{version}"}}"#),
    )
    .unwrap();
    std::fs::write(path.join("index.js"), "module.exports = {};").unwrap();
}

// ── virtual_entry_name ────────────────────────────────────────────────────────

#[test]
fn entry_name_plain() {
    assert_eq!(virtual_entry_name("react"), "react");
}

#[test]
fn entry_name_scoped() {
    assert_eq!(virtual_entry_name("@babel/core"), "@babel+core");
}

// ── link_to_virtual_store ─────────────────────────────────────────────────────

#[test]
fn virtual_store_layout_plain_package() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ContentStore::with_root(tmp.path().join("store"));
    let nm = tmp.path().join("node_modules");
    std::fs::create_dir_all(&nm).unwrap();

    seed_store(&store, "react", "18.2.0");
    let vpath = store
        .link_to_virtual_store("npm", "react", "18.2.0", &nm)
        .unwrap();

    // File layout: node_modules/.3va/react@18.2.0/node_modules/react/
    assert_eq!(
        vpath,
        nm.join(".3va")
            .join("react@18.2.0")
            .join("node_modules")
            .join("react")
    );
    assert!(vpath.join("package.json").exists());
    assert!(vpath.join("index.js").exists());

    // Original store entry still intact (hard-link, not move)
    assert!(
        store
            .package_path("npm", "react", "18.2.0")
            .join("index.js")
            .exists()
    );
}

#[test]
fn virtual_store_layout_scoped_package() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ContentStore::with_root(tmp.path().join("store"));
    let nm = tmp.path().join("node_modules");
    std::fs::create_dir_all(&nm).unwrap();

    seed_store(&store, "@babel/core", "7.24.0");
    let vpath = store
        .link_to_virtual_store("npm", "@babel/core", "7.24.0", &nm)
        .unwrap();

    // .3va/@babel+core@7.24.0/node_modules/@babel/core/
    assert_eq!(
        vpath,
        nm.join(".3va")
            .join("@babel+core@7.24.0")
            .join("node_modules")
            .join("@babel")
            .join("core")
    );
    assert!(vpath.join("package.json").exists());
}

#[test]
fn virtual_store_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ContentStore::with_root(tmp.path().join("store"));
    let nm = tmp.path().join("node_modules");
    std::fs::create_dir_all(&nm).unwrap();

    seed_store(&store, "ms", "2.1.3");
    store
        .link_to_virtual_store("npm", "ms", "2.1.3", &nm)
        .unwrap();
    // Second call must not fail or corrupt.
    store
        .link_to_virtual_store("npm", "ms", "2.1.3", &nm)
        .unwrap();
    assert!(
        nm.join(".3va")
            .join("ms@2.1.3")
            .join("node_modules")
            .join("ms")
            .join("package.json")
            .exists()
    );
}

#[test]
fn multiple_packages_each_get_own_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ContentStore::with_root(tmp.path().join("store"));
    let nm = tmp.path().join("node_modules");
    std::fs::create_dir_all(&nm).unwrap();

    seed_store(&store, "lodash", "4.17.21");
    seed_store(&store, "express", "4.18.2");

    store
        .link_to_virtual_store("npm", "lodash", "4.17.21", &nm)
        .unwrap();
    store
        .link_to_virtual_store("npm", "express", "4.18.2", &nm)
        .unwrap();

    let three_va = nm.join(".3va");
    assert!(three_va.join("lodash@4.17.21").exists());
    assert!(three_va.join("express@4.18.2").exists());
}

// ── symlink topology (Unix only) ──────────────────────────────────────────────

#[cfg(unix)]
mod unix_symlinks {
    use super::*;

    fn setup(name: &str, version: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let store = ContentStore::with_root(tmp.path().join("store"));
        let nm = tmp.path().join("node_modules");
        std::fs::create_dir_all(&nm).unwrap();

        seed_store(&store, name, version);
        store
            .link_to_virtual_store("npm", name, version, &nm)
            .unwrap();
        (tmp, nm)
    }

    fn make_symlink(nm: &Path, name: &str, version: &str) {
        let entry = virtual_entry_name(name);
        let link_path = if name.contains('/') {
            let scope = nm.join(name.split('/').next().unwrap());
            std::fs::create_dir_all(&scope).unwrap();
            nm.join(name)
        } else {
            nm.join(name)
        };
        if link_path.is_symlink() {
            std::fs::remove_file(&link_path).unwrap();
        }
        let rel = if name.contains('/') {
            format!("../.3va/{}@{}/node_modules/{}", entry, version, name)
        } else {
            format!(".3va/{}@{}/node_modules/{}", entry, version, name)
        };
        std::os::unix::fs::symlink(&rel, &link_path).unwrap();
    }

    #[test]
    fn top_level_symlink_resolves_for_plain_package() {
        let (_tmp, nm) = setup("react", "18.2.0");
        make_symlink(&nm, "react", "18.2.0");

        let link = nm.join("react");
        assert!(link.is_symlink(), "node_modules/react must be a symlink");
        // Following the symlink reaches the real package.json
        assert!(
            link.join("package.json").exists(),
            "symlink must resolve to package.json"
        );
    }

    #[test]
    fn top_level_symlink_resolves_for_scoped_package() {
        let (_tmp, nm) = setup("@babel/core", "7.24.0");
        make_symlink(&nm, "@babel/core", "7.24.0");

        let link = nm.join("@babel").join("core");
        assert!(
            link.is_symlink(),
            "node_modules/@babel/core must be a symlink"
        );
        assert!(link.join("package.json").exists());
    }

    #[test]
    fn virtual_store_is_visible_to_developer() {
        let (_tmp, nm) = setup("lodash", "4.17.21");
        make_symlink(&nm, "lodash", "4.17.21");

        // Developer can inspect .3va/ to understand what is installed.
        let three_va = nm.join(".3va");
        assert!(three_va.is_dir(), "node_modules/.3va/ must exist");
        let entries: Vec<_> = std::fs::read_dir(&three_va).unwrap().flatten().collect();
        assert!(!entries.is_empty(), ".3va/ must contain at least one entry");
        assert_eq!(entries[0].file_name().to_string_lossy(), "lodash@4.17.21");
    }
}
