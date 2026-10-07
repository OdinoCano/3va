// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

#![no_main]

use libfuzzer_sys::fuzz_target;
use vvva_pm::{DependencyGraph, DependencyNode, Semver, SemverRange};

// Exercises the local (offline) halves of the package.json resolver: JSON
// parsing of a manifest's dependency maps, the semver range/version parsers
// they feed into, and the `.npmrc` registry resolver. None of this touches the
// network; `Resolver::resolve` is deliberately not called.
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // A package.json is JSON; a manifest's dependency maps are name → range.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
        && let Some(obj) = value.as_object()
    {
        for key in [
            "dependencies",
            "devDependencies",
            "peerDependencies",
            "optionalDependencies",
        ] {
            let Some(deps) = obj.get(key).and_then(|v| v.as_object()) else {
                continue;
            };
            for (name, range) in deps {
                let Some(range_str) = range.as_str() else {
                    continue;
                };
                // The same pairing the resolver does: parse the range, match a
                // concrete version against it, and build graph nodes.
                if let (Some(r), Some(v)) = (SemverRange::parse(range_str), Semver::parse("1.0.0"))
                {
                    let _ = r.matches(&v);
                    let _ = v.satisfies(&r);
                }
                let mut graph = DependencyGraph::new();
                graph.add_node(DependencyNode::new(name.clone(), range_str.to_string()));
                let _ = graph.get_node(name, range_str);
                let _ = graph.resolve_version(name, range_str);
            }
        }
    }

    // Bare parsers must never panic on arbitrary text.
    let _ = Semver::parse(text);
    let _ = SemverRange::parse(text);

    // `.npmrc` parsing + scope-aware registry resolution.
    let cfg = vvva_pm::parse_npmrc(text);
    let _ = vvva_pm::resolve_registry(&cfg, text);
});
