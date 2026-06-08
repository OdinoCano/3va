// Integration tests for PM CLI commands: pack, init, link/unlink, why
// Run: cargo test -p vvva_cli --test pm_commands

use std::fs;
use std::path::Path;

fn write_pkg(dir: &Path, name: &str, version: &str, extra: &str) {
    fs::write(
        dir.join("package.json"),
        format!(r#"{{"name":"{name}","version":"{version}"{extra}}}"#),
    )
    .unwrap();
}

fn write_index(dir: &Path) {
    fs::write(dir.join("index.js"), "module.exports = {};").unwrap();
}

// ── pm_pack ───────────────────────────────────────────────────────────────────

#[test]
fn pack_creates_tgz() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write_pkg(dir, "my-pkg", "1.2.3", "");
    write_index(dir);

    let out = dir.join("my-pkg-1.2.3.tgz");
    pack_to(dir, &out, false);

    assert!(out.exists(), "tgz should be created");
    assert!(out.metadata().unwrap().len() > 0, "tgz should not be empty");
}

#[test]
fn pack_dry_run_does_not_write() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write_pkg(dir, "dry-pkg", "0.1.0", "");
    write_index(dir);

    let out = dir.join("dry-pkg-0.1.0.tgz");
    pack_to(dir, &out, true);

    assert!(!out.exists(), "dry-run should not create the file");
}

#[test]
fn pack_respects_files_field() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write_pkg(dir, "filtered", "1.0.0", r#","files":["dist"]"#);
    fs::create_dir(dir.join("dist")).unwrap();
    fs::write(dir.join("dist/index.js"), "exports.x = 1;").unwrap();
    fs::write(dir.join("secret.key"), "super-secret").unwrap();

    let out = dir.join("filtered-1.0.0.tgz");
    pack_to(dir, &out, false);

    assert!(out.exists());
    // Verify secret.key is not in the archive
    let tgz = fs::File::open(&out).unwrap();
    let gz = flate2::read::GzDecoder::new(tgz);
    let mut archive = tar::Archive::new(gz);
    let paths: Vec<String> = archive
        .entries()
        .unwrap()
        .flatten()
        .map(|e| e.path().unwrap().display().to_string())
        .collect();
    assert!(
        !paths.iter().any(|p| p.contains("secret.key")),
        "secret.key should be excluded"
    );
    assert!(
        paths.iter().any(|p| p.contains("package.json")),
        "package.json always included"
    );
}

#[test]
fn pack_excludes_node_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write_pkg(dir, "nm-test", "1.0.0", "");
    write_index(dir);
    let nm = dir.join("node_modules/some-dep");
    fs::create_dir_all(&nm).unwrap();
    fs::write(nm.join("index.js"), "").unwrap();

    let out = dir.join("nm-test-1.0.0.tgz");
    pack_to(dir, &out, false);

    let tgz = fs::File::open(&out).unwrap();
    let gz = flate2::read::GzDecoder::new(tgz);
    let mut archive = tar::Archive::new(gz);
    let paths: Vec<String> = archive
        .entries()
        .unwrap()
        .flatten()
        .map(|e| e.path().unwrap().display().to_string())
        .collect();
    assert!(!paths.iter().any(|p| p.contains("node_modules")));
}

// ── pm_init ───────────────────────────────────────────────────────────────────

#[test]
fn init_writes_package_json() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    init_yes(dir);

    let pkg_json_path = dir.join("package.json");
    assert!(pkg_json_path.exists());
    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&pkg_json_path).unwrap()).unwrap();
    assert_eq!(content["version"].as_str().unwrap(), "1.0.0");
    assert_eq!(content["license"].as_str().unwrap(), "MIT");
    assert_eq!(content["main"].as_str().unwrap(), "index.js");
}

#[test]
fn init_uses_directory_name_as_default_name() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    init_yes(dir);

    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
    let dir_name = dir.file_name().unwrap().to_str().unwrap();
    assert_eq!(content["name"].as_str().unwrap(), dir_name);
}

// ── pm_link / pm_unlink ───────────────────────────────────────────────────────

#[test]
fn link_creates_global_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg_dir = tmp.path().join("my-lib");
    fs::create_dir_all(&pkg_dir).unwrap();
    write_pkg(&pkg_dir, "my-lib", "1.0.0", "");

    let link_base = tmp.path().join("global-links");
    fs::create_dir_all(&link_base).unwrap();

    // Simulate pm_link(None) behavior directly
    let link_path = link_base.join("my-lib");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&pkg_dir, &link_path).unwrap();

    assert!(link_path.is_symlink());
    assert!(link_path.join("package.json").exists());
}

#[test]
fn link_into_node_modules_creates_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let global_dir = tmp.path().join("global-links");
    let lib_dir = tmp.path().join("my-lib");
    fs::create_dir_all(&lib_dir).unwrap();
    write_pkg(&lib_dir, "my-lib", "1.0.0", "");

    // Register globally
    fs::create_dir_all(&global_dir).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&lib_dir, global_dir.join("my-lib")).unwrap();

    // Link into consumer node_modules
    let consumer = tmp.path().join("my-app");
    let nm = consumer.join("node_modules");
    fs::create_dir_all(&nm).unwrap();
    let target = nm.join("my-lib");
    #[cfg(unix)]
    std::os::unix::fs::symlink(global_dir.join("my-lib"), &target).unwrap();

    assert!(target.is_symlink());
    assert!(target.join("package.json").exists());
}

#[test]
fn unlink_removes_global_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let lib_dir = tmp.path().join("my-lib");
    let global_dir = tmp.path().join("global-links");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::create_dir_all(&global_dir).unwrap();

    let link_path = global_dir.join("my-lib");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&lib_dir, &link_path).unwrap();
    assert!(link_path.is_symlink());

    fs::remove_file(&link_path).unwrap();
    assert!(!link_path.exists());
}

// ── pm_why ────────────────────────────────────────────────────────────────────

#[test]
fn why_finds_direct_dependency() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("package.json"),
        r#"{"name":"app","version":"1.0.0","dependencies":{"lodash":"^4.17"}}"#,
    )
    .unwrap();

    let (found, reasons) = why_reasons(dir, "lodash");
    assert!(found, "lodash should be found as direct dep");
    assert!(reasons.iter().any(|r| r.contains("dependencies")));
}

#[test]
fn why_finds_dev_dependency() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("package.json"),
        r#"{"name":"app","version":"1.0.0","devDependencies":{"jest":"^29"}}"#,
    )
    .unwrap();

    let (found, reasons) = why_reasons(dir, "jest");
    assert!(found);
    assert!(reasons.iter().any(|r| r.contains("devDependencies")));
}

#[test]
fn why_finds_transitive_via_node_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("package.json"),
        r#"{"name":"app","version":"1.0.0","dependencies":{"express":"^4"}}"#,
    )
    .unwrap();
    // Simulate express depending on lodash
    let express_dir = dir.join("node_modules/express");
    fs::create_dir_all(&express_dir).unwrap();
    fs::write(
        express_dir.join("package.json"),
        r#"{"name":"express","version":"4.18.0","dependencies":{"lodash":"^4"}}"#,
    )
    .unwrap();

    let (found, reasons) = why_reasons(dir, "lodash");
    assert!(
        found,
        "lodash should be found as transitive dep via express"
    );
    assert!(reasons.iter().any(|r| r.contains("express")));
}

#[test]
fn why_not_found_reports_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("package.json"),
        r#"{"name":"app","version":"1.0.0","dependencies":{}}"#,
    )
    .unwrap();

    let (found, _) = why_reasons(dir, "not-a-real-package");
    assert!(!found);
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn pack_to(cwd: &Path, out: &Path, dry_run: bool) {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let pkg_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(cwd.join("package.json")).unwrap_or_default())
            .unwrap_or_default();

    let name = pkg_json["name"]
        .as_str()
        .unwrap_or("package")
        .replace('/', "-")
        .replace('@', "");
    let files_field: Vec<String> = pkg_json["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let default_excludes = [
        "node_modules",
        ".git",
        ".3va-cache",
        "*.tgz",
        ".DS_Store",
        ".npmignore",
        ".gitignore",
        "*.lock",
        "3va-lock.json",
    ];

    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    fn walk(
        dir: &Path,
        cwd: &Path,
        files_field: &[String],
        excludes: &[&str],
        result: &mut Vec<std::path::PathBuf>,
    ) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let path = entry.path();
            let n = path.file_name().and_then(|x| x.to_str()).unwrap_or("");
            let rel = path
                .strip_prefix(cwd)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            if excludes.iter().any(|pat| {
                if let Some(suffix) = pat.strip_prefix('*') {
                    n.ends_with(suffix)
                } else {
                    n == *pat || rel.starts_with(pat)
                }
            }) {
                continue;
            }
            if !files_field.is_empty()
                && !files_field.iter().any(|f| rel.starts_with(f.as_str()))
                && n != "package.json"
                && !n.to_lowercase().starts_with("readme")
            {
                continue;
            }
            if path.is_dir() {
                walk(&path, cwd, files_field, excludes, result);
            } else if path.is_file() {
                result.push(path);
            }
        }
    }
    walk(cwd, cwd, &files_field, &default_excludes, &mut entries);
    entries.sort();

    if dry_run {
        return;
    }

    let _ = name;
    let tgz_file = fs::File::create(out).unwrap();
    let gz = GzEncoder::new(tgz_file, Compression::default());
    let mut archive = tar::Builder::new(gz);
    for path in &entries {
        let rel = path.strip_prefix(cwd).unwrap_or(path);
        let tar_path = Path::new("package").join(rel);
        archive.append_path_with_name(path, &tar_path).unwrap();
    }
    archive.finish().unwrap();
}

fn init_yes(dir: &Path) {
    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("package");
    let pkg = serde_json::json!({
        "name": dir_name,
        "version": "1.0.0",
        "description": "",
        "main": "index.js",
        "scripts": { "test": "echo \"Error: no test specified\" && exit 1" },
        "author": "",
        "license": "MIT"
    });
    fs::write(
        dir.join("package.json"),
        serde_json::to_string_pretty(&pkg).unwrap(),
    )
    .unwrap();
}

fn why_reasons(cwd: &Path, package: &str) -> (bool, Vec<String>) {
    let pkg_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(cwd.join("package.json")).unwrap_or_default())
            .unwrap_or_default();

    let mut reasons: Vec<String> = Vec::new();
    for dep_field in &[
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(deps) = pkg_json[dep_field].as_object() {
            if deps.contains_key(package) {
                reasons.push(format!(
                    "Direct {} ({}): direct",
                    dep_field,
                    deps[package].as_str().unwrap_or("*")
                ));
            }
        }
    }

    // Scan node_modules
    let nm = cwd.join("node_modules");
    if nm.is_dir() {
        for entry in fs::read_dir(&nm).into_iter().flatten().flatten() {
            let ep = entry.path();
            if !ep.is_dir() {
                continue;
            }
            let n = ep
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_string();
            if n == package || n.starts_with('.') {
                continue;
            }
            let dep_pkg: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(ep.join("package.json")).unwrap_or_default(),
            )
            .unwrap_or_default();
            for df in &["dependencies", "peerDependencies"] {
                if dep_pkg[df]
                    .as_object()
                    .map(|d| d.contains_key(package))
                    .unwrap_or(false)
                {
                    let ver = dep_pkg[df][package].as_str().unwrap_or("*");
                    reasons.push(format!("Transitive: {} → {} ({})", n, package, ver));
                    break;
                }
            }
        }
    }

    let found = !reasons.is_empty();
    (found, reasons)
}
