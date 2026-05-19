use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct MountPoint {
    pub source: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Default)]
pub struct VirtualFs {
    mounts: HashMap<PathBuf, MountPoint>,
}

impl VirtualFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mount<P: AsRef<Path>, S: AsRef<Path>>(
        &mut self,
        virtual_path: P,
        source: S,
        read_only: bool,
    ) {
        self.mounts.insert(
            virtual_path.as_ref().to_path_buf(),
            MountPoint {
                source: source.as_ref().to_path_buf(),
                read_only,
            },
        );
    }

    pub fn resolve(&self, path: &Path) -> Result<PathBuf, String> {
        let normalized = normalize_path(path);

        for (vp, mount) in &self.mounts {
            if let Ok(relative) = normalized.strip_prefix(vp) {
                let real = mount.source.join(relative);
                return Ok(real);
            }
        }
        Err("Path not mounted".to_string())
    }
}

/// Normalizes a path, resolving `.` and `..` without touching the filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            _ => {
                normalized.push(component);
            }
        }
    }
    normalized
}

#[derive(Debug, Default)]
pub struct VirtualNetwork {
    allowed_hosts: HashSet<String>,
}

impl VirtualNetwork {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow_host(&mut self, host: &str) {
        self.allowed_hosts.insert(host.to_string());
    }

    pub fn is_allowed(&self, host: &str) -> bool {
        self.allowed_hosts.iter().any(|allowed| {
            if allowed == "*" {
                return true;
            }
            if allowed == host {
                return true;
            }
            if let Some(suffix) = allowed.strip_prefix("*.") {
                return host.ends_with(suffix)
                    && host.len() > suffix.len()
                    && host.as_bytes()[host.len() - suffix.len() - 1] == b'.';
            }
            false
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_normalization_traversal() {
        let path = Path::new("/app/config/../../etc/passwd");
        let normalized = normalize_path(path);
        assert_eq!(normalized.to_str().unwrap(), "/etc/passwd");
    }

    #[test]
    fn test_virtual_fs_resolution() {
        let mut vfs = VirtualFs::new();
        vfs.mount("/app", "/var/lib/3va/sandbox1", true);

        // Valid resolution
        let resolved = vfs.resolve(Path::new("/app/config.json")).unwrap();
        assert_eq!(
            resolved.to_str().unwrap(),
            "/var/lib/3va/sandbox1/config.json"
        );

        // Path traversal attempt gets normalized to stay within bounds or errors if out
        // /app/../etc/passwd -> /etc/passwd -> not starts with /app -> error
        let error = vfs.resolve(Path::new("/app/../etc/passwd"));
        assert!(error.is_err());
        assert_eq!(error.unwrap_err(), "Path not mounted");
    }

    #[test]
    fn test_virtual_network_allow() {
        let mut vnet = VirtualNetwork::new();
        vnet.allow_host("api.github.com");
        vnet.allow_host("*.google.com");

        assert!(vnet.is_allowed("api.github.com"));
        assert!(!vnet.is_allowed("github.com"));

        assert!(vnet.is_allowed("maps.google.com"));
        assert!(vnet.is_allowed("api.maps.google.com"));
        assert!(!vnet.is_allowed("google.com"));
        assert!(!vnet.is_allowed("evildomain.com"));
    }
}
