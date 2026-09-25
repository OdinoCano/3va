// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! `node_modules/.bin` generation.
//!
//! Without `.bin`, nothing in a freshly `3va install`ed tree can call a
//! package's CLI — `vite`, `tsc`, `jest` — which is why `3va build` used to
//! have to shell out to a real `npm`. The layout produced here is the one npm
//! produces: on Unix a symlink named after the bin pointing at the package's
//! entry file, on Windows a `.cmd`/`.ps1` shim pair.
//!
//! Binaries are only *linked*, never executed: linking a shim grants no
//! capability. What a bin is allowed to do once it runs is decided by the
//! permission system, exactly like any other JS.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One `<name> → <relative path inside the package>` bin declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinLink {
    pub name: String,
    pub target: PathBuf,
    pub owner: String,
}

#[derive(Debug, Default, Clone)]
pub struct BinSummary {
    pub created: Vec<BinLink>,
    pub skipped: Vec<(String, String)>,
}

impl BinSummary {
    pub fn len(&self) -> usize {
        self.created.len()
    }
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
    }
}

/// Read a manifest's `bin` field into `(bin name, relative entry path)` pairs.
///
/// npm's rules: a string `bin` uses the package's own name (unscoped) as the
/// command; an object maps command names to paths. A `bin` entry pointing at a
/// directory is resolved through that directory's `package.json`.
pub fn bin_entries(manifest: &serde_json::Value, pkg_name: &str) -> Vec<(String, PathBuf)> {
    let default_name = pkg_name.rsplit('/').next().unwrap_or(pkg_name).to_string();
    let mut out = Vec::new();
    match manifest.get("bin") {
        Some(serde_json::Value::String(p)) => {
            if !p.is_empty() {
                out.push((default_name, PathBuf::from(p)));
            }
        }
        Some(serde_json::Value::Object(map)) => {
            for (name, path) in map {
                if let Some(p) = path.as_str()
                    && !p.is_empty()
                {
                    out.push((name.clone(), PathBuf::from(p)));
                }
            }
        }
        _ => {}
    }
    out
}

/// Create `node_modules/.bin` for every installed package under `node_modules`.
///
/// Walks the top level only (that is where npm puts the shims a script's PATH
/// lookup finds) and descends into `<pkg>/node_modules` for nested trees, so a
/// package that shells out to its own dependency's bin still resolves.
///
/// Existing shims belonging to a package that is no longer present are
/// removed, matching `npm prune` closely enough that a removed dependency does
/// not leave a dangling command behind. Returns the links created and the
/// (name, owner) pairs whose entry file could not be resolved.
pub fn link_bins(node_modules: &Path) -> anyhow::Result<BinSummary> {
    let mut summary = BinSummary::default();
    if !node_modules.is_dir() {
        return Ok(summary);
    }
    let bin_dir = node_modules.join(".bin");
    std::fs::create_dir_all(&bin_dir)?;

    let mut wanted: BTreeMap<String, BinLink> = BTreeMap::new();
    collect_from_level(node_modules, node_modules, &mut wanted, &mut summary)?;

    // Drop stale shims so `npm ls`-style PATH lookups can't hit a command for
    // a package that was uninstalled.
    if let Ok(existing) = std::fs::read_dir(&bin_dir) {
        for entry in existing.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || wanted.contains_key(&name) {
                continue;
            }
            let path = entry.path();
            let stale = std::fs::symlink_metadata(&path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if stale {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_file(bin_dir.join(format!("{name}.cmd")));
                let _ = std::fs::remove_file(bin_dir.join(format!("{name}.ps1")));
            }
        }
    }

    for (name, link) in &wanted {
        match write_shim(&bin_dir, name, &link.target) {
            Ok(()) => summary.created.push(link.clone()),
            Err(e) => summary
                .skipped
                .push((name.clone(), format!("{}: {e}", link.owner))),
        }
    }
    Ok(summary)
}

fn collect_from_level(
    node_modules: &Path,
    dir: &Path,
    wanted: &mut BTreeMap<String, BinLink>,
    summary: &mut BinSummary,
) -> anyhow::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    // Sort so the result is deterministic regardless of readdir order: two
    // packages declaring the same bin name is a broken tree either way, but the
    // winner should at least be the same on every machine.
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        let Some(file_name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        if file_name.starts_with('.') {
            continue;
        }
        if file_name.starts_with('@') && path.is_dir() {
            collect_from_level(node_modules, &path, wanted, summary)?;
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        // A symlinked package (3va's isolated layout) still reports as a dir.
        let Some(pkg_name) = package_name(&path) else {
            continue;
        };
        let manifest = read_manifest(&path);
        if let Some(manifest) = &manifest {
            for (name, rel) in bin_entries(manifest, &pkg_name) {
                let resolved = resolve_bin_entry(&path, &rel);
                if !resolved.is_file() {
                    summary.skipped.push((
                        name.clone(),
                        format!("{}: missing {}", pkg_name, rel.display()),
                    ));
                    continue;
                }
                // Relative from .bin/ back to the package, so the tree stays
                // movable and works inside a container bind-mount.
                let rel_from_bin = rel_from(&bin_parent(node_modules), &resolved);
                wanted.entry(name.clone()).or_insert(BinLink {
                    name,
                    target: rel_from_bin,
                    owner: pkg_name.clone(),
                });
            }
        }
        // Nested dependencies get their own .bin.
        let nested = path.join("node_modules");
        if nested.is_dir() {
            let nested_bin = nested.join(".bin");
            let _ = std::fs::create_dir_all(&nested_bin);
            let mut nested_wanted = BTreeMap::new();
            collect_from_level(node_modules, &nested, &mut nested_wanted, summary)?;
            for (name, link) in nested_wanted {
                let target = resolve_bin_entry(&nested, &link.target);
                let _ = std::fs::create_dir_all(&nested_bin);
                let rel = rel_from(&bin_parent(&nested), &target);
                let _ = write_shim(&nested_bin, &name, &rel);
            }
        }
    }
    Ok(())
}

fn bin_parent(node_modules: &Path) -> PathBuf {
    node_modules.join(".bin")
}

fn read_manifest(pkg_dir: &Path) -> Option<serde_json::Value> {
    let content = std::fs::read_to_string(pkg_dir.join("package.json")).ok()?;
    serde_json::from_str(&content).ok()
}

fn package_name(pkg_dir: &Path) -> Option<String> {
    let file_name = pkg_dir.file_name()?.to_string_lossy().to_string();
    if let Some(parent) = pkg_dir.parent()
        && let Some(scope) = parent.file_name()
    {
        let scope = scope.to_string_lossy().to_string();
        if scope.starts_with('@') {
            return Some(format!("{scope}/{file_name}"));
        }
    }
    Some(file_name)
}

/// Resolve a `bin` path inside a package, following a directory to the
/// `main`/`index.js` it points at, the way Node's own bin-link resolution does.
fn resolve_bin_entry(pkg_dir: &Path, rel: &Path) -> PathBuf {
    let direct = pkg_dir.join(rel);
    if direct.is_file() {
        return direct;
    }
    if direct.is_dir() {
        for candidate in ["index.js", "index.mjs", "index.cjs", "cli.js"] {
            let f = direct.join(candidate);
            if f.is_file() {
                return f;
            }
        }
        if let Some(manifest) = read_manifest(&direct) {
            for key in ["main", "module"] {
                if let Some(main) = manifest.get(key).and_then(|v| v.as_str()) {
                    let f = direct.join(main);
                    if f.is_file() {
                        return f;
                    }
                }
            }
        }
    }
    direct
}

fn rel_from(base_dir: &Path, target: &Path) -> PathBuf {
    // Both paths live under the same node_modules tree; a lexical relative path
    // is enough and avoids canonicalize() failing on not-yet-created symlinks.
    let base = base_dir.components().collect::<Vec<_>>();
    let tgt = target.components().collect::<Vec<_>>();
    let common = base
        .iter()
        .zip(tgt.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    for c in &tgt[common..] {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

fn write_shim(bin_dir: &Path, name: &str, target_rel: &Path) -> anyhow::Result<()> {
    let link_path = bin_dir.join(name);
    #[cfg(unix)]
    {
        // Replace whatever npm/pnpm/yarn left behind (a symlink, a shell stub,
        // a directory).
        let meta = std::fs::symlink_metadata(&link_path);
        if let Ok(m) = &meta {
            if m.file_type().is_dir() {
                let _ = std::fs::remove_dir_all(&link_path);
            } else {
                let _ = std::fs::remove_file(&link_path);
            }
        }
        std::os::unix::fs::symlink(target_rel, &link_path)?;
        // npm leaves the referenced file executable; so do we, otherwise a
        // `sh -c 'vite build'` delegated to a real package manager fails.
        if let Ok(real) = link_path.metadata() {
            let mut perms = real.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            {
                use std::os::unix::fs::PermissionsExt;
                perms.set_mode(perms.mode() | 0o111);
            }
            let _ = std::fs::set_permissions(&link_path, perms);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::remove_file(&link_path);
        let rel_display = target_rel.to_string_lossy().replace('\\', "/");
        // Windows shims delegate to the interpreter recorded in the shebang,
        // exactly like npm's shimmin, so a package that ships `#!/usr/bin/env
        // node` still runs.
        let script = format!(
            "@ECHO off\r\nSETLOCAL\r\nnode \"%~dp0{}\" %*\r\n",
            rel_display
        );
        std::fs::write(bin_dir.join(format!("{name}.cmd")), script)?;
        let ps1 = format!(
            "#!/usr/bin/env pwsh\n$basedir = Split-Path -Parent $MyInvocation.MyCommand.Definition\n\
             node \"$basedir/{}\" $args\nexit $LASTEXITCODE\n",
            rel_display
        );
        std::fs::write(bin_dir.join(format!("{name}.ps1")), ps1)?;
        Ok(())
    }
}

/// Resolve a command name the way a package script's PATH lookup would:
/// `<node_modules>/.bin/<name>` first, walking up from `start` like Node does.
pub fn find_bin(start: &Path, name: &str) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        for candidate in [
            d.join("node_modules").join(".bin").join(name),
            d.join("node_modules")
                .join(".bin")
                .join(format!("{name}.cmd")),
        ] {
            if candidate.is_file() || candidate.symlink_metadata().is_ok() {
                return Some(candidate);
            }
        }
        dir = d.parent();
    }
    None
}

/// Follow a `.bin` shim to the JavaScript file it ultimately runs.
///
/// A `.bin` entry is either a symlink to the entry file (this crate's own
/// output, and npm's on Unix) or — for pnpm, and for a package manager that
/// ran before us — a shell stub or a file carrying a `cmd-shim-target`
// comment. Resolving to the real `.js`/`.mjs`/`.cjs` is what lets a
/// `package.json` script like `"build": "vite build"` run *inside* the 3va
/// sandbox instead of delegating to an unsandboxed `npm run`.
pub fn resolve_bin_entry_file(bin_path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(bin_path).ok()?;
    let dir = bin_path.parent()?;

    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(bin_path).ok()?;
        let abs = lexical_normalize(&if target.is_absolute() {
            target
        } else {
            dir.join(target)
        });
        // A chained shim (bin → shim → real file) is one hop too far.
        if abs
            .file_name()
            .map(|n| n.to_string_lossy().starts_with('.'))
            .unwrap_or(false)
        {
            return None;
        }
        return abs.is_file().then_some(abs);
    }

    // Not a symlink: a shell stub (npm on Windows, pnpm) or a real script.
    let Ok(content) = std::fs::read_to_string(bin_path) else {
        return None;
    };
    if let Some(target) = parse_shim_target(&content, dir) {
        return Some(target);
    }
    if is_javascript_entry(bin_path) {
        return Some(bin_path.to_path_buf());
    }
    None
}

/// Resolve `.` / `..` without touching the filesystem: a shim may point at a
/// target that does not exist yet, and we still want a clean path to report.
pub fn lexical_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.last().is_some_and(|c| c != "..") {
                    out.pop();
                } else {
                    out.push("..".into());
                }
            }
            other => out.push(other.as_os_str().to_os_string()),
        }
    }
    let mut result = PathBuf::new();
    for c in out {
        result.push(c);
    }
    result
}

fn parse_shim_target(content: &str, dir: &Path) -> Option<PathBuf> {
    // pnpm records the real file in a `# cmd-shim-target=` comment.
    for line in content.lines().take(10) {
        if let Some(rest) = line.trim().strip_prefix("# cmd-shim-target=") {
            let rel = rest.trim().trim_matches('"');
            let p = Path::new(rel);
            let abs = lexical_normalize(&if p.is_absolute() {
                p.to_path_buf()
            } else {
                dir.join(p)
            });
            if abs.is_file() {
                return Some(abs);
            }
        }
    }
    // A POSIX stub we (or npm) generated: `exec node "$basedir/../pkg/bin.js" "$@"`.
    for line in content.lines().take(20) {
        let Some(idx) = line.find("\"$basedir/").or_else(|| line.find("\"$dir/")) else {
            continue;
        };
        let rest = &line[idx + 1..];
        let Some(end) = rest.find('"') else { continue };
        let rel = rest[..end]
            .trim_start_matches("$basedir/")
            .trim_start_matches("$dir/");
        let abs = lexical_normalize(&dir.join(rel));
        if abs.is_file() {
            return Some(abs);
        }
    }
    None
}

/// Is this a file 3va can execute inside its own runtime?
///
/// A `.js`/`.mjs`/`.cjs` file always. An extensionless file only when it opens
/// with a JavaScript shebang: `bin/esbuild` and `bin/tsc` are the most common
/// package bins in the ecosystem and are pure JS with no extension, while an
/// extensionless ELF binary must never be handed to the JS engine.
pub fn is_javascript_entry(path: &Path) -> bool {
    if matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("js") | Some("mjs") | Some("cjs")
    ) {
        return true;
    }
    if path.extension().is_some() {
        return false;
    }
    match std::fs::File::open(path) {
        Ok(mut f) => {
            use std::io::Read;
            let mut buf = [0u8; 64];
            match f.read(&mut buf) {
                Ok(n) if n >= 19 => {
                    buf[..n].starts_with(b"#!") && String::from_utf8_lossy(&buf).contains("node")
                }
                _ => false,
            }
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn pkg(dir: &Path, name: &str, manifest: &str) {
        write(&dir.join(name).join("package.json"), manifest);
    }

    #[test]
    fn bin_string_uses_the_unscoped_package_name() {
        let m = serde_json::json!({"bin": "./cli.js"});
        assert_eq!(
            bin_entries(&m, "@scope/tool"),
            vec![("tool".to_string(), PathBuf::from("./cli.js"))]
        );
    }

    #[test]
    fn bin_object_keeps_its_own_names() {
        let m = serde_json::json!({"bin": {"tsc": "./bin/tsc", "tsserver": "./bin/tsserver"}});
        let e = bin_entries(&m, "typescript");
        assert_eq!(e.len(), 2);
        assert!(e.contains(&("tsc".to_string(), PathBuf::from("./bin/tsc"))));
    }

    #[test]
    fn missing_or_empty_bin_yields_nothing() {
        assert!(bin_entries(&serde_json::json!({}), "x").is_empty());
        assert!(bin_entries(&serde_json::json!({"bin": ""}), "x").is_empty());
        assert!(bin_entries(&serde_json::json!({"bin": {}}), "x").is_empty());
    }

    #[test]
    fn link_bins_creates_a_shim_for_a_real_entry() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        pkg(
            &nm,
            "tool",
            r#"{"name":"tool","version":"1.0.0","bin":"./cli.js"}"#,
        );
        write(&nm.join("tool/cli.js"), "#!/usr/bin/env node\n");

        let summary = link_bins(&nm).unwrap();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary.created[0].name, "tool");

        let shim = nm.join(".bin/tool");
        assert!(shim.exists());
        // The shim must point at the real file, relatively, so the tree moves.
        assert!(shim.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(read_link(&shim), PathBuf::from("../tool/cli.js"));
    }

    #[test]
    fn link_bins_is_idempotent_and_replaces_foreign_shims() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        pkg(
            &nm,
            "tool",
            r#"{"name":"tool","version":"1.0.0","bin":{"tool":"./cli.js"}}"#,
        );
        write(&nm.join("tool/cli.js"), "x");
        link_bins(&nm).unwrap();

        // Simulate npm having left a shell stub in place of our symlink.
        std::fs::remove_file(nm.join(".bin/tool")).unwrap();
        write(
            &nm.join(".bin/tool"),
            "#!/bin/sh\nexec node \"$basedir/../tool/cli.js\" \"$@\"\n",
        );

        let summary = link_bins(&nm).unwrap();
        assert_eq!(summary.len(), 1);
        assert!(
            nm.join(".bin/tool")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn link_bins_skips_an_entry_that_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        pkg(
            &nm,
            "tool",
            r#"{"name":"tool","version":"1.0.0","bin":"./nope.js"}"#,
        );
        let summary = link_bins(&nm).unwrap();
        assert!(summary.is_empty());
        assert_eq!(summary.skipped.len(), 1);
        assert!(summary.skipped[0].1.contains("nope.js"));
    }

    #[test]
    fn link_bins_removes_shims_of_packages_that_are_gone() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        pkg(
            &nm,
            "tool",
            r#"{"name":"tool","version":"1.0.0","bin":"./cli.js"}"#,
        );
        write(&nm.join("tool/cli.js"), "x");
        link_bins(&nm).unwrap();
        assert!(nm.join(".bin/tool").exists());

        std::fs::remove_dir_all(nm.join("tool")).unwrap();
        link_bins(&nm).unwrap();
        assert!(
            nm.join(".bin/tool").symlink_metadata().is_err(),
            "the shim of a removed package must be removed too, not left dangling"
        );
    }

    #[test]
    fn scoped_packages_get_their_bin() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        write(
            &nm.join("@esbuild/linux-x64/package.json"),
            r#"{"name":"@esbuild/linux-x64","version":"0.24.0","bin":{"esbuild":"./bin/esbuild"}}"#,
        );
        write(&nm.join("@esbuild/linux-x64/bin/esbuild"), "#!/bin/sh\n");

        let summary = link_bins(&nm).unwrap();
        assert_eq!(summary.created[0].name, "esbuild");
        assert!(nm.join(".bin/esbuild").exists());
    }

    #[test]
    fn find_bin_walks_up_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        write(&nm.join(".bin/vite"), "x");
        let deep = dir.path().join("packages/app/src");
        std::fs::create_dir_all(&deep).unwrap();
        assert!(find_bin(&deep, "vite").is_some());
        assert!(find_bin(&deep, "tsc").is_none());
    }

    #[test]
    fn lexical_normalize_resolves_dot_segments() {
        assert_eq!(
            lexical_normalize(Path::new("/a/.bin/../b/c")),
            Path::new("/a/b/c")
        );
    }

    #[test]
    fn resolve_bin_entry_file_follows_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        pkg(
            &nm,
            "vite",
            r#"{"name":"vite","version":"1.0.0","bin":{"vite":"./bin/vite.js"}}"#,
        );
        write(&nm.join("vite/bin/vite.js"), "x");
        link_bins(&nm).unwrap();
        let resolved = resolve_bin_entry_file(&nm.join(".bin/vite"));
        assert_eq!(resolved, Some(nm.join("vite/bin/vite.js")));
    }

    #[test]
    fn resolve_bin_entry_file_reads_a_shell_stub() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        write(&nm.join("tsc/bin/tsc"), "x");
        write(
            &nm.join(".bin/tsc"),
            "#!/bin/sh\nexec node \"$basedir/../tsc/bin/tsc\" \"$@\"\n",
        );
        assert_eq!(
            resolve_bin_entry_file(&nm.join(".bin/tsc")),
            Some(nm.join("tsc/bin/tsc"))
        );
    }

    #[test]
    fn resolve_bin_entry_file_reads_a_pnpm_cmd_shim() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        write(&nm.join("jest/bin/jest.js"), "x");
        write(
            &nm.join(".bin/jest"),
            "#!/bin/sh\n# cmd-shim-target=../jest/bin/jest.js\nbasedir=$(dirname \"$0\")\n",
        );
        assert_eq!(
            resolve_bin_entry_file(&nm.join(".bin/jest")),
            Some(nm.join("jest/bin/jest.js"))
        );
    }

    #[cfg(unix)]
    fn read_link(path: &Path) -> PathBuf {
        std::fs::read_link(path).unwrap()
    }
}
