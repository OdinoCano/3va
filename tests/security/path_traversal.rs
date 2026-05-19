#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn is_path_safe(base: &PathBuf, input: &PathBuf) -> bool {
        let Ok(canonical_base) = base.canonicalize() else { return false; };
        let Ok(canonical_input) = input.canonicalize() else { return false; };

        canonical_input.starts_with(&canonical_base)
    }

    #[test]
    fn test_simple_traversal() {
        let base = PathBuf::from("/home/user/sandbox");
        let input = PathBuf::from("/home/user/sandbox/../../../etc/passwd");

        assert!(!is_path_safe(&base, &input));
    }

    #[test]
    fn test_absolute_traversal() {
        let base = PathBuf::from("/home/user/sandbox");
        let input = PathBuf::from("/etc/passwd");

        assert!(!is_path_safe(&base, &input));
    }

    #[test]
    fn test_relative_traversal() {
        let base = PathBuf::from("/home/user/sandbox");
        let input = PathBuf::from("subdir/../../etc/passwd");

        assert!(!is_path_safe(&base, &input));
    }

    #[test]
    fn test_valid_path() {
        let base = PathBuf::from("/home/user/sandbox");
        let input = PathBuf::from("/home/user/sandbox/subdir/file.txt");

        assert!(is_path_safe(&base, &input));
    }

    #[test]
    fn test_null_byte_injection() {
        let input = "file.txt\x00malicious";
        assert!(input.contains('\0'));
    }

    #[test]
    fn test_unicode_normalization() {
        let malicious = "ＮＯＤＥ＿ＭＯＤＵＬＥＳ";
        let normalized = malicious
            .chars()
            .map(|c| {
                if c.is_whitespace() || !c.is_ascii_alphanumeric() && c != '_' && c != '-' {
                    '_'
                } else {
                    c
                }
            })
            .collect::<String>();

        assert_ne!(malicious, normalized);
    }

    #[test]
    fn test_dot_segment_normalization() {
        fn normalize_path(path: &str) -> String {
            let mut result = Vec::new();
            for segment in path.split('/') {
                match segment {
                    "" | "." => continue,
                    ".." => { result.pop(); }
                    _ => result.push(segment),
                }
            }
            let normalized = format!("/{}", result.join("/"));
            if normalized == path { path.to_string() } else { normalized }
        }

        assert_eq!(normalize_path("/a/b/c/../d"), "/a/b/d");
        assert_eq!(normalize_path("/a/./b/./c"), "/a/b/c");
    }
}