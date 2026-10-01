// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Semver {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub prerelease: String,
    pub build: String,
}

impl Semver {
    pub fn parse(version: &str) -> Option<Self> {
        let version = version.trim();

        let (main, prerelease_build) = if let Some((main, pb)) = version.split_once('-') {
            (main, Some(pb))
        } else {
            (version, None)
        };

        let parts: Vec<&str> = main.split('.').collect();
        if parts.len() < 3 {
            return None;
        }

        let major = parts[0].parse().ok()?;
        let minor = parts[1].parse().ok()?;
        let patch = parts[2].split('+').next()?.parse().ok()?;

        let (prerelease, build) = match prerelease_build {
            Some(pb) => {
                let parts: Vec<&str> = pb.split('+').collect();
                let pre = parts.first().map(|s| s.to_string()).unwrap_or_default();
                let build = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
                (pre, build)
            }
            None => (String::new(), String::new()),
        };

        Some(Semver {
            major,
            minor,
            patch,
            prerelease,
            build,
        })
    }

    pub fn satisfies(&self, range: &SemverRange) -> bool {
        range.matches(self)
    }
}

impl PartialOrd for Semver {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Semver {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            r => return r,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            r => return r,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            r => return r,
        }

        match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (true, true) => Ordering::Equal,
            (false, false) => cmp_prerelease(&self.prerelease, &other.prerelease),
        }
    }
}

/// SemVer 2.0 §11: dot-separated identifiers compared left to right,
/// numeric ones numerically and below alphanumeric ones; a shorter list of
/// otherwise equal identifiers is lower.
fn cmp_prerelease(a: &str, b: &str) -> Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(n), Ok(m)) => n.cmp(&m),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum SemverRange {
    Exact(Semver),
    Caret(Semver),
    Tilde(Semver),
    Gt(Semver),
    Gte(Semver),
    Lt(Semver),
    Lte(Semver),
    /// Conjunction of two ranges, both must match (used for compound ranges like
    /// `">=1.0.0 <2.0.0"`).
    And(Box<SemverRange>, Box<SemverRange>),
    /// Either range matches (`"^1.0.0 || ^2.0.0"`).
    Or(Box<SemverRange>, Box<SemverRange>),
    Any,
}

impl SemverRange {
    /// Parse an npm-compatible version range string.
    ///
    /// Handles all common npm range forms:
    /// - Exact: `"1.2.3"`, `"=1.2.3"`
    /// - Caret: `"^1.2.3"` (compatible minor/patch), `"^0.2.3"` (pins minor),
    ///   `"^0.0.3"` (exact)
    /// - Tilde: `"~1.2.3"` (compatible patch)
    /// - Comparators: `">1.0.0"`, `">=1.0.0"`, `"<2.0.0"`, `"<=2.0.0"`
    /// - Wildcards: `"*"`, `""`, `"1.x"`, `"1.2.x"`, `"1"`, `"1.2"`
    /// - Compound (AND): `">=1.0.0 <2.0.0"` (space-separated components)
    /// - Dist-tags: `"latest"`, `"next"`, `"beta"` → treated as `Any`
    pub fn parse(range: &str) -> Option<Self> {
        let range = range.trim();

        // `a || b`: any alternative may match; an empty alternative is `*`.
        if range.contains("||") {
            let mut out: Option<SemverRange> = None;
            for alt in range.split("||") {
                let r = SemverRange::parse(alt)?;
                out = Some(match out {
                    None => r,
                    Some(prev) => SemverRange::Or(Box::new(prev), Box::new(r)),
                });
            }
            return out;
        }

        // Hyphen range `1.2.3 - 2.3.4`: inclusive on both ends; a partial
        // upper bound (`- 2.3`) means below the next minor/major.
        if let Some((lo, hi)) = range.split_once(" - ") {
            let lo = Semver::parse(lo.trim()).or_else(|| parse_partial(lo.trim()))?;
            let hi = hi.trim();
            let upper = match Semver::parse(hi) {
                Some(v) => SemverRange::Lte(v),
                None => {
                    let dots = hi
                        .trim_end_matches(".x")
                        .trim_end_matches(".*")
                        .matches('.')
                        .count();
                    let p = parse_partial(hi)?;
                    let next = if dots == 0 {
                        Semver {
                            major: p.major + 1,
                            minor: 0,
                            patch: 0,
                            prerelease: String::new(),
                            build: String::new(),
                        }
                    } else {
                        Semver {
                            major: p.major,
                            minor: p.minor + 1,
                            patch: 0,
                            prerelease: String::new(),
                            build: String::new(),
                        }
                    };
                    SemverRange::Lt(next)
                }
            };
            return Some(SemverRange::And(
                Box::new(SemverRange::Gte(lo)),
                Box::new(upper),
            ));
        }

        if range == "*" || range.is_empty() {
            return Some(SemverRange::Any);
        }

        // Dist-tags (e.g. "latest", "next", "beta") — purely alphabetic words.
        if is_dist_tag(range) {
            return Some(SemverRange::Any);
        }

        // npm allows whitespace after an operator (">= 2.1.2 < 3.0.0", used by
        // safer-buffer); glue each bare operator token onto the next token.
        let is_op = |t: &str| matches!(t, ">=" | "<=" | ">" | "<" | "=" | "^" | "~");
        let glued;
        let range = if range.split_whitespace().any(is_op) {
            let mut parts: Vec<String> = Vec::new();
            let mut op = String::new();
            for t in range.split_whitespace() {
                if is_op(t) {
                    op.push_str(t);
                } else {
                    parts.push(format!("{op}{t}"));
                    op.clear();
                }
            }
            glued = parts.join(" ");
            glued.as_str()
        } else {
            range
        };

        // Compound ranges: ">=1.0.0 <2.0.0" — split on whitespace and AND them.
        if range.contains(' ') {
            return parse_compound(range);
        }

        if let Some(rest) = range.strip_prefix('^') {
            let version = Semver::parse(rest).or_else(|| parse_partial(rest))?;
            return Some(SemverRange::Caret(version));
        }

        if let Some(rest) = range.strip_prefix('~') {
            let version = Semver::parse(rest).or_else(|| parse_partial(rest))?;
            return Some(SemverRange::Tilde(version));
        }

        if let Some(rest) = range.strip_prefix(">=") {
            let version = Semver::parse(rest)?;
            return Some(SemverRange::Gte(version));
        }

        if let Some(rest) = range.strip_prefix('>') {
            let version = Semver::parse(rest)?;
            return Some(SemverRange::Gt(version));
        }

        if let Some(rest) = range.strip_prefix("<=") {
            let version = Semver::parse(rest)?;
            return Some(SemverRange::Lte(version));
        }

        if let Some(rest) = range.strip_prefix('<') {
            let version = Semver::parse(rest)?;
            return Some(SemverRange::Lt(version));
        }

        if let Some(rest) = range.strip_prefix('=') {
            let version = Semver::parse(rest)?;
            return Some(SemverRange::Exact(version));
        }

        // X-ranges: "1", "1.x", "1.2", "1.2.x" — treat as caret/tilde.
        if let Some(r) = parse_x_range(range) {
            return Some(r);
        }

        Semver::parse(range).map(SemverRange::Exact)
    }

    /// npm semantics: a prerelease version only matches when the range
    /// itself names a prerelease of the same `major.minor.patch`
    /// (`^1.2.3-beta.1` accepts `1.2.3-beta.2`, but `>=1.0.0` or `*` never
    /// pick `2.0.0-rc.1`). Otherwise `*`/`latest`/`>=x` resolved to
    /// whatever dev build had the highest number.
    pub fn matches(&self, version: &Semver) -> bool {
        if let SemverRange::Or(a, b) = self {
            return a.matches(version) || b.matches(version);
        }
        self.matches_ignoring_prerelease_rule(version)
            && (version.prerelease.is_empty() || self.names_prerelease_of(version))
    }

    fn names_prerelease_of(&self, v: &Semver) -> bool {
        match self {
            SemverRange::Exact(b)
            | SemverRange::Caret(b)
            | SemverRange::Tilde(b)
            | SemverRange::Gt(b)
            | SemverRange::Gte(b)
            | SemverRange::Lt(b)
            | SemverRange::Lte(b) => {
                !b.prerelease.is_empty()
                    && (b.major, b.minor, b.patch) == (v.major, v.minor, v.patch)
            }
            SemverRange::And(a, b) => a.names_prerelease_of(v) || b.names_prerelease_of(v),
            SemverRange::Or(_, _) | SemverRange::Any => false,
        }
    }

    fn matches_ignoring_prerelease_rule(&self, version: &Semver) -> bool {
        match self {
            SemverRange::Any => true,
            SemverRange::Exact(v) => version == v,
            SemverRange::Caret(base) => {
                if base.major != 0 {
                    version.major == base.major && version >= base
                } else if base.minor != 0 {
                    version.major == 0 && version.minor == base.minor && version >= base
                } else {
                    version.major == 0 && version.minor == 0 && version.patch == base.patch
                }
            }
            SemverRange::Tilde(base) => {
                version.major == base.major && version.minor == base.minor && version >= base
            }
            SemverRange::Gt(v) => version > v,
            SemverRange::Gte(v) => version >= v,
            SemverRange::Lt(v) => version < v,
            SemverRange::Lte(v) => version <= v,
            SemverRange::And(a, b) => {
                a.matches_ignoring_prerelease_rule(version)
                    && b.matches_ignoring_prerelease_rule(version)
            }
            SemverRange::Or(a, b) => a.matches(version) || b.matches(version),
        }
    }
}

/// Returns true for npm dist-tags: purely alphabetic words optionally joined
/// by hyphens (e.g. "latest", "next", "beta", "rc", "canary").
fn is_dist_tag(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
}

/// Parse a partial version like "1", "1.2", "1.x", "1.2.x" into a Semver
/// base with missing/wildcard components filled with 0.
fn parse_partial(s: &str) -> Option<Semver> {
    // Strip trailing wildcard components.
    let s = s
        .trim_end_matches(".x")
        .trim_end_matches(".X")
        .trim_end_matches(".*");
    let parts: Vec<&str> = s.split('.').collect();
    let major: u32 = parts.first()?.parse().ok()?;
    let minor: u32 = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch: u32 = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    Some(Semver {
        major,
        minor,
        patch,
        prerelease: String::new(),
        build: String::new(),
    })
}

/// Interpret x-range shorthand as a Caret or Tilde range.
///
/// | Input  | Meaning       | Result               |
/// |--------|---------------|----------------------|
/// | `"1"`  | `^1.0.0`      | `Caret(1.0.0)`       |
/// | `"1.x"`| `^1.0.0`      | `Caret(1.0.0)`       |
/// | `"1.2"`| `~1.2.0`      | `Tilde(1.2.0)`       |
/// |`"1.2.x"`| `~1.2.0`     | `Tilde(1.2.0)`       |
fn parse_x_range(s: &str) -> Option<SemverRange> {
    let parts: Vec<&str> = s.split('.').collect();
    let is_wild = |p: &str| matches!(p, "x" | "X" | "*");

    match parts.as_slice() {
        // "1" or "1.x" or "1.x.x" → ^1.0.0
        [maj] | [maj, _, ..] if parts.get(1).is_none_or(|p| is_wild(p)) => {
            let major: u32 = maj.parse().ok()?;
            Some(SemverRange::Caret(Semver {
                major,
                minor: 0,
                patch: 0,
                prerelease: String::new(),
                build: String::new(),
            }))
        }
        // "1.2" or "1.2.x" → ~1.2.0
        [maj, min] | [maj, min, _] if parts.get(2).is_none_or(|p| is_wild(p)) => {
            let major: u32 = maj.parse().ok()?;
            let minor: u32 = min.parse().ok()?;
            Some(SemverRange::Tilde(Semver {
                major,
                minor,
                patch: 0,
                prerelease: String::new(),
                build: String::new(),
            }))
        }
        _ => None,
    }
}

/// Parse a space-separated compound range like `">=1.0.0 <2.0.0"` into an
/// `And` chain.  Returns `None` only if no component parses successfully.
fn parse_compound(range: &str) -> Option<SemverRange> {
    let mut result: Option<SemverRange> = None;
    for part in range.split_whitespace() {
        if let Some(r) = SemverRange::parse(part) {
            result = Some(match result {
                None => r,
                Some(prev) => SemverRange::And(Box::new(prev), Box::new(r)),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    fn m(range: &str, v: &str) -> bool {
        SemverRange::parse(range)
            .unwrap()
            .matches(&Semver::parse(v).unwrap())
    }

    #[test]
    fn prereleases_need_an_explicit_prerelease_comparator() {
        // The T3 bench install resolved `typescript` to a nightly this way.
        assert!(!m(">=4.8.4", "7.1.0-dev.20260930.4"));
        assert!(!m("*", "7.1.0-dev.20260930.4"));
        assert!(!m("^5.8.2", "5.9.0-beta"));
        assert!(m("^1.2.3-beta.1", "1.2.3-beta.2"));
        assert!(!m("^1.2.3-beta.2", "1.2.3-beta.1"));
        assert!(m("^1.2.3-beta.1", "1.4.0"));
        assert!(!m("^1.2.3-beta.1", "1.4.0-beta.1"));
        assert!(m("1.2.3-rc.1", "1.2.3-rc.1"));
    }

    #[test]
    fn prerelease_ordering_follows_semver() {
        let p = |s| Semver::parse(s).unwrap();
        assert!(p("1.0.0-alpha") < p("1.0.0-alpha.1"));
        assert!(p("1.0.0-alpha.1") < p("1.0.0-alpha.beta"));
        assert!(p("1.0.0-beta.2") < p("1.0.0-beta.11"));
        assert!(p("1.0.0-rc.1") < p("1.0.0"));
    }

    #[test]
    fn or_and_hyphen_ranges() {
        assert!(m("^4.0.0 || ^5.0.0", "5.3.1"));
        assert!(m("^4 || ^5", "4.1.0"));
        assert!(!m("^4 || ^5", "6.0.0"));
        assert!(m("1.2.3 - 2.3.4", "2.3.4"));
        assert!(!m("1.2.3 - 2.3.4", "2.3.5"));
        assert!(m("1.2 - 2.3", "2.3.9"));
        assert!(!m("1.2 - 2.3", "2.4.0"));
        assert!(!m("1.2.3 - 2", "3.0.0"));
    }

    #[test]
    fn operator_followed_by_space() {
        let r = SemverRange::parse(">= 2.1.2 < 3.0.0").unwrap();
        assert!(r.matches(&Semver::parse("2.1.2").unwrap()));
        assert!(!r.matches(&Semver::parse("3.0.0").unwrap()));
        assert!(
            SemverRange::parse("^ 1.2.0")
                .unwrap()
                .matches(&Semver::parse("1.9.0").unwrap())
        );
    }

    use super::*;

    #[test]
    fn test_semver_parsing() {
        let v = Semver::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn test_semver_prerelease() {
        let v = Semver::parse("1.2.3-beta.1").unwrap();
        assert_eq!(v.prerelease, "beta.1");
    }

    #[test]
    fn test_caret_range() {
        let range = SemverRange::parse("^1.0.0").unwrap();
        assert!(range.matches(&Semver::parse("1.5.0").unwrap()));
        assert!(range.matches(&Semver::parse("1.0.0").unwrap()));
        assert!(!range.matches(&Semver::parse("2.0.0").unwrap()));
        assert!(!range.matches(&Semver::parse("0.9.9").unwrap()));
    }

    #[test]
    fn test_caret_range_zero_major() {
        // ^0.2.3 → >=0.2.3 <0.3.0 (pin minor when major is 0)
        let range = SemverRange::parse("^0.2.3").unwrap();
        assert!(range.matches(&Semver::parse("0.2.3").unwrap()));
        assert!(range.matches(&Semver::parse("0.2.9").unwrap()));
        assert!(!range.matches(&Semver::parse("0.3.0").unwrap()));
        assert!(!range.matches(&Semver::parse("1.0.0").unwrap()));
    }

    #[test]
    fn test_caret_range_zero_minor() {
        // ^0.0.3 → =0.0.3 (exact when both major and minor are 0)
        let range = SemverRange::parse("^0.0.3").unwrap();
        assert!(range.matches(&Semver::parse("0.0.3").unwrap()));
        assert!(!range.matches(&Semver::parse("0.0.4").unwrap()));
        assert!(!range.matches(&Semver::parse("0.1.0").unwrap()));
    }

    #[test]
    fn test_tilde_range() {
        let range = SemverRange::parse("~1.2.0").unwrap();
        assert!(range.matches(&Semver::parse("1.2.3").unwrap()));
        assert!(range.matches(&Semver::parse("1.2.0").unwrap()));
        assert!(!range.matches(&Semver::parse("1.3.0").unwrap()));
        assert!(!range.matches(&Semver::parse("2.2.0").unwrap()));
    }

    #[test]
    fn test_comparison_ranges() {
        assert!(
            SemverRange::parse(">1.0.0")
                .unwrap()
                .matches(&Semver::parse("1.0.1").unwrap())
        );
        assert!(
            !SemverRange::parse(">1.0.0")
                .unwrap()
                .matches(&Semver::parse("1.0.0").unwrap())
        );
        assert!(
            SemverRange::parse(">=1.0.0")
                .unwrap()
                .matches(&Semver::parse("1.0.0").unwrap())
        );
        assert!(
            SemverRange::parse("<2.0.0")
                .unwrap()
                .matches(&Semver::parse("1.9.9").unwrap())
        );
        assert!(
            !SemverRange::parse("<2.0.0")
                .unwrap()
                .matches(&Semver::parse("2.0.0").unwrap())
        );
        assert!(
            SemverRange::parse("<=2.0.0")
                .unwrap()
                .matches(&Semver::parse("2.0.0").unwrap())
        );
    }

    #[test]
    fn test_wildcard_any() {
        let range = SemverRange::parse("*").unwrap();
        assert!(range.matches(&Semver::parse("0.0.1").unwrap()));
        assert!(range.matches(&Semver::parse("99.99.99").unwrap()));
    }

    #[test]
    fn test_dist_tags_treated_as_any() {
        // npm dist-tags like "latest", "next", "beta" must not cause resolution
        // to fail — they are treated as a wildcard that matches any version.
        for tag in &["latest", "next", "beta", "rc", "canary"] {
            let range = SemverRange::parse(tag).expect(tag);
            assert!(
                range.matches(&Semver::parse("1.0.0").unwrap()),
                "dist-tag {tag} should match any version"
            );
        }
    }

    #[test]
    fn test_x_range_major_only() {
        // "1" → ^1.0.0: any 1.x.x release
        let range = SemverRange::parse("1").unwrap();
        assert!(range.matches(&Semver::parse("1.0.0").unwrap()));
        assert!(range.matches(&Semver::parse("1.99.99").unwrap()));
        assert!(!range.matches(&Semver::parse("2.0.0").unwrap()));
        assert!(!range.matches(&Semver::parse("0.9.9").unwrap()));
    }

    #[test]
    fn test_x_range_major_dot_x() {
        // "1.x" → ^1.0.0
        let range = SemverRange::parse("1.x").unwrap();
        assert!(range.matches(&Semver::parse("1.5.0").unwrap()));
        assert!(!range.matches(&Semver::parse("2.0.0").unwrap()));
    }

    #[test]
    fn test_x_range_major_minor() {
        // "1.2" → ~1.2.0: any 1.2.x release
        let range = SemverRange::parse("1.2").unwrap();
        assert!(range.matches(&Semver::parse("1.2.0").unwrap()));
        assert!(range.matches(&Semver::parse("1.2.9").unwrap()));
        assert!(!range.matches(&Semver::parse("1.3.0").unwrap()));
        assert!(!range.matches(&Semver::parse("2.2.0").unwrap()));
    }

    #[test]
    fn test_x_range_major_minor_dot_x() {
        // "1.2.x" → ~1.2.0
        let range = SemverRange::parse("1.2.x").unwrap();
        assert!(range.matches(&Semver::parse("1.2.5").unwrap()));
        assert!(!range.matches(&Semver::parse("1.3.0").unwrap()));
    }

    #[test]
    fn test_compound_range_and() {
        // ">=1.0.0 <2.0.0" — standard npm compatible range
        let range = SemverRange::parse(">=1.0.0 <2.0.0").unwrap();
        assert!(range.matches(&Semver::parse("1.0.0").unwrap()));
        assert!(range.matches(&Semver::parse("1.99.99").unwrap()));
        assert!(!range.matches(&Semver::parse("2.0.0").unwrap()));
        assert!(!range.matches(&Semver::parse("0.9.9").unwrap()));
    }

    #[test]
    fn test_compound_range_three_parts() {
        // ">=1.2.3 <2.0.0" with explicit lower bound
        let range = SemverRange::parse(">=1.2.3 <2.0.0").unwrap();
        assert!(range.matches(&Semver::parse("1.2.3").unwrap()));
        assert!(!range.matches(&Semver::parse("1.2.2").unwrap()));
        assert!(!range.matches(&Semver::parse("2.0.0").unwrap()));
    }

    #[test]
    fn test_partial_version_caret_prefix() {
        // "^1.x" — caret with wildcard minor
        let range = SemverRange::parse("^1.x").unwrap();
        assert!(range.matches(&Semver::parse("1.0.0").unwrap()));
        assert!(range.matches(&Semver::parse("1.9.9").unwrap()));
        assert!(!range.matches(&Semver::parse("2.0.0").unwrap()));
    }
}
