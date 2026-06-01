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
            _ => Ordering::Equal,
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

        if range == "*" || range.is_empty() {
            return Some(SemverRange::Any);
        }

        // Dist-tags (e.g. "latest", "next", "beta") — purely alphabetic words.
        if is_dist_tag(range) {
            return Some(SemverRange::Any);
        }

        // Compound ranges: ">=1.0.0 <2.0.0" — split on whitespace and AND them.
        if range.contains(' ') {
            return parse_compound(range);
        }

        if let Some(rest) = range.strip_prefix('^') {
            let version = parse_partial(rest)?;
            return Some(SemverRange::Caret(version));
        }

        if let Some(rest) = range.strip_prefix('~') {
            let version = parse_partial(rest)?;
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

    pub fn matches(&self, version: &Semver) -> bool {
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
            SemverRange::And(a, b) => a.matches(version) && b.matches(version),
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
