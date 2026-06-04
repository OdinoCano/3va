#![no_main]

use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;
use vvva_pm::{DependencyGraph, DependencyNode, Resolver, Semver, SemverRange};

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

    // ── Ordering: if two Semvers parse from the same string they must be equal
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

        // Insert a node with fuzz-controlled name (version pinned to valid)
        let node = DependencyNode::new(s.to_string(), "1.0.0".to_string());
        graph.add_node(node);

        // Exact lookup with the fuzz input as both name and version
        let _ = graph.get_node(s, s);
        let _ = graph.get_node(s, "1.0.0");

        // resolve_version: fuzz-controlled name and range string
        let _ = graph.resolve_version(s, s);
        let _ = graph.resolve_version(s, "^1.0.0");
        let _ = graph.resolve_version(s, "*");
    }

    // ── Resolver::resolve: arbitrary package name/version never panics ──────
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut resolver = Resolver::new("https://registry.npmjs.org");

        // Single fuzz-controlled dependency
        let mut deps = HashMap::new();
        deps.insert(s.to_string(), s.to_string());
        let graph = rt.block_on(resolver.resolve(&deps));

        // Whatever came back, the graph must be inspectable without panic
        let _ = graph.nodes();
        let _ = graph.get_node(s, s);
    }

    // ── Split input into two tokens for two-package scenarios ───────────────
    // Use the first NUL byte (if any) as a separator; otherwise split at midpoint.
    let (left, right) = if let Some(pos) = data.iter().position(|&b| b == 0) {
        let l = std::str::from_utf8(&data[..pos]).unwrap_or("");
        let r = std::str::from_utf8(&data[pos + 1..]).unwrap_or("");
        (l, r)
    } else {
        let mid = s.len() / 2;
        (&s[..mid], &s[mid..])
    };

    // Two-package resolve with independent name/version strings
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut resolver = Resolver::new("https://registry.npmjs.org");
        let mut deps = HashMap::new();
        deps.insert(left.to_string(), right.to_string());
        deps.insert(right.to_string(), left.to_string());
        let graph = rt.block_on(resolver.resolve(&deps));
        let _ = graph.nodes();
    }

    // ── Determinism: same input → same resolve result ───────────────────────
    {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut r1 = Resolver::new("https://registry.npmjs.org");
        let mut r2 = Resolver::new("https://registry.npmjs.org");
        let mut deps = HashMap::new();
        deps.insert(s.to_string(), "1.0.0".to_string());
        let g1 = rt.block_on(r1.resolve(&deps));
        let g2 = rt.block_on(r2.resolve(&deps));
        assert_eq!(
            g1.nodes().len(),
            g2.nodes().len(),
            "resolver must be deterministic for input {s:?}"
        );
    }
});
