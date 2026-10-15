// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// `3va --allow-insecure start` must forward the global flag to the supervised
// `3va run` process. The supervisor and the app are separate processes, so the
// flag is otherwise consumed by the launcher and lost — `fetch('http://…')`
// inside the app then failed with "Insecure protocol HTTP … disabled".
//
// The test inspects the child's command line via /proc (Linux), which is what
// actually reaches the app, rather than trusting the argument builder.
//
// Run: cargo test -p vvva_cli --test flag_propagation

#![cfg(target_os = "linux")]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_3va"))
}

/// All child PIDs of `ppid`, from /proc.
fn children_of(ppid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            // "pid (comm) state ppid ..." — comm may contain spaces/parens.
            if let Some(rest) = stat.rsplit_once(") ").map(|(_, r)| r) {
                let mut it = rest.split_whitespace();
                let _state = it.next();
                if let Some(parsed) = it.next().and_then(|s| s.parse::<u32>().ok()) {
                    if parsed == ppid {
                        out.push(pid);
                    }
                }
            }
        }
    }
    out
}

fn cmdline(pid: u32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| {
            b.iter()
                .map(|&c| if c == 0 { ' ' } else { c as char })
                .collect()
        })
        .unwrap_or_default()
}

struct Cleanup(Child);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let pid = self.0.id();
        // Kill the supervisor and every descendant.
        for c in children_of(pid) {
            for g in children_of(c) {
                unsafe { libc::kill(g as i32, libc::SIGKILL) };
            }
            unsafe { libc::kill(c as i32, libc::SIGKILL) };
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn start_attach_forwards_allow_insecure_to_the_app() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"name":"flagtest","3va":{"permissions":{".":{"allow-net":["127.0.0.1"]}}}}"#,
    )
    .unwrap();
    let mut idle = std::fs::File::create(dir.path().join("idle.js")).unwrap();
    writeln!(idle, "setInterval(function () {{}}, 1000);").unwrap();

    let child = Command::new(bin())
        .args(["--allow-insecure", "start", "--attach", "idle.js"])
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn 3va start --attach");
    let sup = Cleanup(child);

    let mut found: Option<String> = None;
    for _ in 0..100 {
        for c in children_of(sup.0.id()) {
            let cl = cmdline(c);
            if cl.contains("run") && cl.contains("idle.js") {
                found = Some(cl);
                break;
            }
        }
        if found.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let cl = found.expect("the supervisor never spawned a `3va run` child");
    assert!(
        cl.contains("--allow-insecure"),
        "the supervised app must inherit --allow-insecure; child argv was: {cl}"
    );
}
