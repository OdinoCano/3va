#![no_main]

use libfuzzer_sys::fuzz_target;
use vvva_pm::{DependencyGraph, DependencyNode, Semver, SemverRange};

// NOTE: Resolver::resolve() makes real HTTP calls to the registry and is NOT
// suitable for fuzz testing. This target exercises only the local, offline
// components of the package manager:
//   - Semver::parse / SemverRange::parse
//   - Semver comparison and SemverRange::matches
//   - DependencyGraph construction and lookup
//
// Network-touching paths (Resolver::resolve) are tested by integration tests,
// not by the fuzzer.

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else { return };

    // ── Semver::parse: never panics, returns None for garbage ──────────────
    let parsed_version = Semver::parse(s);
    let parsed_range   = SemverRange::parse(s);

    // ── Semver/SemverRange comparison: no panics on any valid pair ──────────
    if let (Some(v), Some(r)) = (&parsed_version, &parsed_range) {
        let _ = r.matches(v);
        let _ = v.satisfies(r);
    }

    // ── Ordering: same input must produce equal Semvers ─────────────────────
    if let (Some(a), Some(b)) = (Semver::parse(s), Semver::parse(s)) {
        assert_eq!(
            a.cmp(&b),
            std::cmp::Ordering::Equal,
            "same input must produce equal Semver: {s:?}"
        );
    }

    // ── DependencyGraph: fuzz-derived node operations never panic ───────────
    {
        let mut graph = DependencyGraph::new();

        let node = DependencyNode::new(s.to_string(), "1.0.0".to_string());
        graph.add_node(node);

        let _ = graph.get_node(s, s);
        let _ = graph.get_node(s, "1.0.0");

        let _ = graph.resolve_version(s, s);
        let _ = graph.resolve_version(s, "^1.0.0");
        let _ = graph.resolve_version(s, "*");
    }

    // ── Split input for two-token scenarios ─────────────────────────────────
    let (left, right) = if let Some(pos) = data.iter().position(|&b| b == 0) {
        let l = std::str::from_utf8(&data[..pos]).unwrap_or("");
        let r = std::str::from_utf8(&data[pos + 1..]).unwrap_or("");
        (l, r)
    } else {
        let mid = s.len() / 2;
        (&s[..mid], &s[mid..])
    };

    // ── Two-token graph with mixed name/version inputs ───────────────────────
    {
        let mut graph = DependencyGraph::new();
        graph.add_node(DependencyNode::new(left.to_string(), "1.0.0".to_string()));
        graph.add_node(DependencyNode::new(right.to_string(), "2.0.0".to_string()));

        let _ = graph.get_node(left, "1.0.0");
        let _ = graph.get_node(right, "2.0.0");
        let _ = graph.nodes().len();
    }

    // ── Determinism: same input → same DependencyGraph outcome ──────────────
    {
        let mut g1 = DependencyGraph::new();
        let mut g2 = DependencyGraph::new();

        for v in ["1.0.0", "1.5.0", "2.0.0"] {
            g1.add_node(DependencyNode::new(s.to_string(), v.to_string()));
            g2.add_node(DependencyNode::new(s.to_string(), v.to_string()));
        }

        assert_eq!(
            g1.nodes().len(),
            g2.nodes().len(),
            "DependencyGraph must be deterministic for input {s:?}"
        );
    }
});
