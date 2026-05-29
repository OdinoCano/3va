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
    Any,
}

impl SemverRange {
    pub fn parse(range: &str) -> Option<Self> {
        let range = range.trim();

        if range == "*" || range.is_empty() {
            return Some(SemverRange::Any);
        }

        if let Some(rest) = range.strip_prefix('^') {
            let version = Semver::parse(rest)?;
            return Some(SemverRange::Caret(version));
        }

        if let Some(rest) = range.strip_prefix('~') {
            let version = Semver::parse(rest)?;
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
        }
    }
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
}
