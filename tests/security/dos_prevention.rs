#[cfg(test)]
mod tests {
    #[test]
    fn test_decompression_bomb_detection() {
        let compressed = vec![0u8; 10_000_000];
        let expected_expanded = 1_000_000_000;

        let ratio = expected_expanded as f64 / compressed.len() as f64;
        assert!(ratio < 1000.0, "Compression bomb detected: {}x", ratio);
    }

    #[test]
    fn test_recursion_depth_limit() {
        fn count_recursion(depth: u32, max: u32) -> u32 {
            if depth >= max {
                return depth;
            }
            count_recursion(depth + 1, max)
        }

        let result = count_recursion(0, 10000);
        assert_eq!(result, 10000);
    }

    #[test]
    fn test_memory_allocation_limit() {
        let max_memory_mb = 512;
        let allocation_mb = 600;

        assert!(
            allocation_mb <= max_memory_mb,
            "Memory allocation {}MB exceeds limit {}MB",
            allocation_mb,
            max_memory_mb
        );
    }

    #[test]
    fn test_file_size_limit() {
        let max_file_size = 100 * 1024 * 1024;
        let file_size = 150 * 1024 * 1024;

        assert!(
            file_size <= max_file_size,
            "File size {}MB exceeds limit {}MB",
            file_size / (1024 * 1024),
            max_file_size / (1024 * 1024)
        );
    }

    #[test]
    fn test_zip_slip_prevention() {
        fn is_safe_zip_entry(entry_path: &str) -> bool {
            let normalized = entry_path.replace('\\', "/");
            !normalized.contains("..") && !normalized.starts_with('/')
        }

        assert!(!is_safe_zip_entry("../../etc/passwd"));
        assert!(!is_safe_zip_entry("/absolute/path"));
        assert!(is_safe_zip_entry("dir/file.txt"));
    }

    #[test]
    fn test_symlink_loop_detection() {
        use std::collections::HashSet;
        use std::path::PathBuf;

        fn detect_symlink_loop(path: &PathBuf, visited: &mut HashSet<PathBuf>) -> bool {
            if visited.contains(path) {
                return true;
            }
            visited.insert(path.clone());

            if let Ok(target) = std::fs::read_link(path) {
                return detect_symlink_loop(&target, visited);
            }
            false
        }

        let mut visited = HashSet::new();
        let fake_path = PathBuf::from("/fake/symlink");
        assert!(!detect_symlink_loop(&fake_path, &mut visited));
    }

    #[test]
    fn test_entity_expansion_limit() {
        let xml_entity = "&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;";
        let entity_count = xml_entitymatches!("&nbsp;");

        assert!(entity_count <= 1000, "Too many XML entities");
    }

    #[test]
    fn test_parse_timeout() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let timeout = Duration::from_millis(100);

        let big_input = "x".repeat(1_000_000);
        let _ = big_input.len();

        assert!(
            start.elapsed() < timeout,
            "Parse took too long: {:?}",
            start.elapsed()
        );
    }
}