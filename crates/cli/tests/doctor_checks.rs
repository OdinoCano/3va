// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// VULN-08 regression: `3va doctor` must only claim protections it actually
// verifies. Previously it printed "VirtualFS: Path traversal protection" and
// "VirtualNetwork: Host allowlist enforcement" — both were dead-code modules —
// and "Sandbox enforcement available" while the real enforcement gate had a
// path-traversal bypass. This test runs the real binary and checks the report.

use std::process::Command;

fn doctor_output() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_3va"))
        .arg("doctor")
        .output()
        .expect("failed to run `3va doctor`");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn doctor_no_longer_claims_dead_code_protections() {
    let output = doctor_output();
    for forbidden in [
        "VirtualFS: Path traversal protection",
        "VirtualNetwork: Host allowlist enforcement",
        "Sandbox enforcement available",
    ] {
        assert!(
            !output.contains(forbidden),
            "doctor must not print {forbidden:?}\n---\n{output}"
        );
    }
}

#[test]
fn doctor_reports_real_self_checks() {
    let output = doctor_output();
    for required in [
        "✓ Deny-by-default",
        "✓ Path containment",
        "✓ Deny wins over grant",
        "✓ V8 evaluates code",
    ] {
        assert!(
            output.contains(required),
            "doctor must print a real check for {required:?}\n---\n{output}"
        );
    }
}
