#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs as unix_fs;
    use tempfile::TempDir;

    #[test]
    fn test_symlink_escape() {
        let temp_dir = TempDir::new().unwrap();
        let sandbox = temp_dir.path().join("sandbox");
        fs::create_dir(&sandbox).unwrap();

        let target = temp_dir.path().join("target_file");
        fs::write(&target, "sensitive").unwrap();

        let link = sandbox.join("escape_link");
        unix_fs::symlink(&target, &link).unwrap();

        let resolved = fs::read_link(&link).unwrap();
        assert!(!resolved.starts_with(sandbox));
    }

    #[test]
    fn test_hardlink_escape() {
        let temp_dir = TempDir::new().unwrap();
        let sandbox = temp_dir.path().join("sandbox");
        fs::create_dir(&sandbox).unwrap();

        let target = temp_dir.path().join("target");
        fs::write(&target, "sensitive").unwrap();

        let link = sandbox.join("escape_hardlink");
        unix_fs::hardlink(&target, &link).unwrap();

        let metadata = fs::metadata(&link).unwrap();
        let target_metadata = fs::metadata(&target).unwrap();
        assert_eq!(metadata.ino(), target_metadata.ino());
    }

    #[test]
    fn test_procfs_leak() {
        let suspicious_paths = vec![
            "/proc/self/environ",
            "/proc/self/cmdline",
            "/proc/self/mem",
            "/proc/1/environ",
        ];

        for path in suspicious_paths {
            if std::path::Path::new(path).exists() {
                panic!("Sensitive path {} accessible", path);
            }
        }
    }

    #[test]
    fn test_dev_shm_escape() {
        let dev_shm = std::path::Path::new("/dev/shm");
        if dev_shm.exists() {
            let entries = fs::read_dir(dev_shm).unwrap();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    panic!("File in /dev/shm accessible: {:?}", path);
                }
            }
        }
    }

    #[test]
    fn test_cwd_escape() {
        let original_cwd = std::env::current_dir().unwrap();

        std::env::set_current_dir("/").ok();

        let escaped_cwd = std::env::current_dir().unwrap();
        assert!(escaped_cwd.to_string_lossy() == "/");

        std::env::set_current_dir(original_cwd).ok();
    }
}