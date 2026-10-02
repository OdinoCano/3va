// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// `3va run` starts from a V8 startup snapshot that the first run builds and
// later runs load. The in-process engine tests can't pin that path down: whether
// an engine starts from the snapshot depends on whether an earlier one already
// built it, so a bug that only exists when starting from the snapshot passes or
// fails with test ordering. These tests run the real binary twice against a
// private HOME (the snapshot cache lives under it): the first run builds the
// snapshot, the second starts from it.
//
// Regression: `WebAssembly.instantiate()` never settled when starting from the
// snapshot, because the event-loop keepalive wrapper was recorded into the
// snapshot and V8 reinstalls the `WebAssembly` functions when it creates a
// context from one.

use std::path::Path;
use std::process::Command;

fn run(home: &Path, script: &Path, extra: &[&str]) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_3va"))
        .arg("run")
        .arg(script)
        .arg(format!(
            "--allow-read={}",
            script.parent().unwrap().display()
        ))
        .args(extra)
        .env("HOME", home)
        .env_remove("VVVA_NO_SNAPSHOT")
        .env_remove("VVVA_SNAPSHOT")
        .output()
        .expect("failed to run `3va run`");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn snapshot_files(home: &Path) -> usize {
    std::fs::read_dir(home.join(".cache/3va/snapshot"))
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "bin"))
                .count()
        })
        .unwrap_or(0)
}

/// Runs `script` twice with an empty HOME and returns (first, second) stdout,
/// after checking that the first run built a snapshot for the second to load.
fn first_and_second_run(script_src: &str, extra: &[&str]) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let script = dir.path().join("script.js");
    std::fs::write(&script, script_src).unwrap();

    let (first, first_err) = run(&home, &script, extra);
    assert_eq!(
        snapshot_files(&home),
        1,
        "the first run should have built the snapshot\nstderr:\n{first_err}"
    );
    let (second, second_err) = run(&home, &script, extra);
    assert_eq!(
        snapshot_files(&home),
        1,
        "the second run must reuse it, not build another\nstderr:\n{second_err}"
    );
    (first, second)
}

#[test]
fn async_webassembly_settles_when_started_from_the_snapshot() {
    // (module (func (export "f") (result i32) i32.const 42)); nothing else keeps
    // the event loop alive, so only the wrapper holds it until the compile ends.
    let (first, second) = first_and_second_run(
        r#"
        const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,7,5,1,1,102,0,0,10,6,1,4,0,65,42,11]);
        WebAssembly.instantiate(bytes).then(
          r => console.log('settled', r.instance.exports.f()),
          e => console.log('rejected', String(e)));
        "#,
        &[],
    );
    assert_eq!(first.trim(), "settled 42", "first run (no snapshot yet)");
    assert_eq!(
        second.trim(),
        "settled 42",
        "second run (from the snapshot)"
    );
}

#[test]
fn per_process_state_is_not_baked_into_the_snapshot() {
    // Whatever the bootstrap computed while the snapshot was built must be
    // recomputed for each run: the script path, the environment and the
    // randomness all belong to the run that reads them.
    let src = r#"
        console.log(JSON.stringify({
          script: require('path').basename(process.argv[1]),
          marker: process.env.SNAPSHOT_TEST_MARKER || null,
          uptimeSmall: process.uptime() < 5,
          rand: Math.random(),
        }));
    "#;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let script = dir.path().join("probe.js");
    std::fs::write(&script, src).unwrap();

    let parse = |out: String| -> serde_json::Value {
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("not JSON ({e}): {out:?}"))
    };
    let (a, _) = run(&home, &script, &["--allow-env=SNAPSHOT_TEST_MARKER"]);
    let a = parse(a); // built the snapshot, started without it
    let out = Command::new(env!("CARGO_BIN_EXE_3va"))
        .args(["run"])
        .arg(&script)
        .arg(format!("--allow-read={}", dir.path().display()))
        .arg("--allow-env=SNAPSHOT_TEST_MARKER")
        .env("HOME", &home)
        .env("SNAPSHOT_TEST_MARKER", "run-two")
        .output()
        .unwrap();
    let b = parse(String::from_utf8_lossy(&out.stdout).to_string()); // from the snapshot

    assert_eq!(snapshot_files(&home), 1);
    assert_eq!(a["script"], "probe.js");
    assert_eq!(b["script"], "probe.js");
    assert_eq!(a["marker"], serde_json::Value::Null);
    assert_eq!(
        b["marker"], "run-two",
        "env must be this run's, not the snapshot's"
    );
    assert_eq!(b["uptimeSmall"], true);
    assert_ne!(
        a["rand"], b["rand"],
        "Math.random must not repeat across runs"
    );
}
