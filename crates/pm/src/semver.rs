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
    Range(String),
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

        if range.starts_with('^') {
            let version = Semver::parse(&range[1..])?;
            return Some(SemverRange::Caret(version));
        }

        if range.starts_with('~') {
            let version = Semver::parse(&range[1..])?;
            return Some(SemverRange::Tilde(version));
        }

        if range.starts_with(">=") {
            let version = Semver::parse(&range[2..])?;
            return Some(SemverRange::Gte(version));
        }

        if range.starts_with('>') {
            let version = Semver::parse(&range[1..])?;
            return Some(SemverRange::Gt(version));
        }

        if range.starts_with("<=") {
            let version = Semver::parse(&range[2..])?;
            return Some(SemverRange::Lte(version));
        }

        if range.starts_with('<') {
            let version = Semver::parse(&range[1..])?;
            return Some(SemverRange::Lt(version));
        }

        if range.starts_with('=') {
            let version = Semver::parse(&range[1..])?;
            return Some(SemverRange::Exact(version));
        }

        Semver::parse(range).map(SemverRange::Exact)
    }

    pub fn matches(&self, version: &Semver) -> bool {
        match self {
            SemverRange::Any => true,
            SemverRange::Exact(v) => version == v,
            SemverRange::Caret(base) => {
                version.major == base.major && version >= base && version.major < base.major + 1
            }
            SemverRange::Tilde(base) => {
                version.major == base.major
                    && version.minor == base.minor
                    && version >= base
                    && version.minor < base.minor + 1
            }
            SemverRange::Gt(v) => version > v,
            SemverRange::Gte(v) => version >= v,
            SemverRange::Lt(v) => version < v,
            SemverRange::Lte(v) => version <= v,
            SemverRange::Range(_) => true,
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
    fn test_tilde_range() {
        let range = SemverRange::parse("~1.2.0").unwrap();
        assert!(range.matches(&Semver::parse("1.2.3").unwrap()));
        assert!(range.matches(&Semver::parse("1.2.0").unwrap()));
        assert!(!range.matches(&Semver::parse("1.3.0").unwrap()));
    }
}
