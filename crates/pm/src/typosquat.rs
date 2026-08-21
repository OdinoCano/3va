//! Typosquatting detection for npm package names.
//!
//! Maintains a static list of the most-installed / most-impersonated npm
//! packages and flags requested names whose edit distance (Levenshtein) to a
//! list entry is small but non-zero — the classic typosquat signal
//! (`reqeust` → `request`, `exprss` → `express`).

use std::path::Path;

/// Maximum edit distance between a requested name and a known-popular
/// package for the request to be flagged as a possible typosquat.
/// Exact matches never warn.
pub const TYPOSQUAT_MAX_DISTANCE: usize = 2;

const POPULAR_PACKAGES_RAW: &str = include_str!("popular_packages.txt");

/// One flagged request: the requested name, the legitimate package it most
/// likely targets, and their Levenshtein distance.
#[derive(Debug, Clone, PartialEq)]
pub struct TyposquatFinding {
    pub requested: String,
    pub likely_target: String,
    pub distance: usize,
}

/// The embedded list of popular package names (one per line; blank lines and
/// `#` comments ignored).
pub fn popular_packages() -> Vec<&'static str> {
    POPULAR_PACKAGES_RAW
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

/// Classic two-row dynamic-programming Levenshtein distance over chars.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Check a package name against the popular list.
///
/// Returns `None` when the name exactly matches a popular package or has no
/// close relative in the list; otherwise the closest match within
/// [`TYPOSQUAT_MAX_DISTANCE`].
pub fn check_package_name(name: &str) -> Option<TyposquatFinding> {
    let mut best: Option<(usize, &str)> = None;
    for target in popular_packages() {
        if target == name {
            return None; // exact hit — nothing suspicious
        }
        let d = levenshtein(name, target);
        if d <= TYPOSQUAT_MAX_DISTANCE && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, target));
        }
    }
    best.map(|(distance, likely_target)| TyposquatFinding {
        requested: name.to_string(),
        likely_target: likely_target.to_string(),
        distance,
    })
}

/// Print an audit-style warning to stderr when `name` looks like a typo of a
/// popular package. Returns true when a warning was emitted.
pub fn warn_if_typosquat(name: &str) -> bool {
    if let Some(f) = check_package_name(name) {
        eprintln!(
            "! Possible typosquatting: '{}' looks like '{}' (edit distance {}). \
             Verify the spelling before trusting this package.",
            f.requested, f.likely_target, f.distance
        );
        return true;
    }
    false
}

/// Check every dependency name found in a lockfile-style map (name → meta)
/// and emit warnings; returns how many warnings were printed.
pub fn warn_for_all<'a, I>(names: I) -> usize
where
    I: IntoIterator<Item = &'a str>,
{
    names.into_iter().filter(|n| warn_if_typosquat(n)).count()
}

/// Convenience wrapper used by audit flows: scan the dependency names listed
/// under `dependencies` in an installed package.json manifest.
pub fn warn_for_manifest_deps(manifest_path: &Path) -> usize {
    let Ok(content) = std::fs::read_to_string(manifest_path) else {
        return 0;
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
        return 0;
    };
    val["dependencies"]
        .as_object()
        .map(|deps| warn_for_all(deps.keys().map(String::as_str)))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("request", "request"), 0);
        assert_eq!(levenshtein("reqeust", "request"), 2); // transposition = 2 ops
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn popular_list_is_loaded_and_clean() {
        let pkgs = popular_packages();
        assert!(
            pkgs.len() >= 40,
            "expected a meaningful list, got {}",
            pkgs.len()
        );
        assert!(pkgs.iter().all(|p| p.chars().all(|c| c.is_ascii_lowercase()
            || c.is_ascii_digit()
            || c == '-'
            || c == '_'
            || c == '.')));
        // A handful of the most-impersonated packages must be present.
        for must in ["request", "express", "react", "lodash", "axios"] {
            assert!(pkgs.contains(&must), "{must} missing from popular list");
        }
    }

    #[test]
    fn exact_popular_names_never_warn() {
        for name in ["request", "express", "react", "lodash", "axios"] {
            assert_eq!(check_package_name(name), None, "{name} should not warn");
            assert!(!warn_if_typosquat(name));
        }
    }

    #[test]
    fn classic_typos_are_flagged() {
        let f = check_package_name("reqeust").expect("reqeust must be flagged");
        assert_eq!(f.likely_target, "request");
        assert!(f.distance >= 1 && f.distance <= TYPOSQUAT_MAX_DISTANCE);

        assert!(check_package_name("expres").is_some());
        assert!(check_package_name("lodahs").is_some());
    }

    #[test]
    fn unrelated_names_do_not_warn() {
        for name in ["totally-unrelated-pkg-name", "my-corp-internal-tool"] {
            assert_eq!(check_package_name(name), None, "{name} should not warn");
        }
    }

    #[test]
    fn warn_for_manifest_deps_reports_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("package.json");
        std::fs::write(
            &path,
            r#"{ "dependencies": { "reqeust": "^1.0.0", "left-pad": "^1.3.0" } }"#,
        )
        .unwrap();
        assert_eq!(warn_for_manifest_deps(&path), 1);
    }
}
