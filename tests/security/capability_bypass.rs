#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::RwLock;

    static ENABLED_CAPABILITIES: RwLock<HashSet<String>> = RwLock::new(HashSet::new());

    fn enable_capability(cap: &str) {
        let mut caps = ENABLED_CAPABILITIES.write().unwrap();
        caps.insert(cap.to_string());
    }

    fn has_capability(cap: &str) -> bool {
        let caps = ENABLED_CAPABILITIES.read().unwrap();
        caps.contains(cap)
    }

    fn check_read(path: &str) -> Result<String, String> {
        if has_capability("allow-read") {
            Ok(format!("Content of {}", path))
        } else {
            Err("Capability allow-read not granted".to_string())
        }
    }

    fn check_net(url: &str) -> Result<String, String> {
        if has_capability("allow-net") {
            Ok(format!("Response from {}", url))
        } else {
            Err("Capability allow-net not granted".to_string())
        }
    }

    fn check_env(key: &str) -> Result<String, String> {
        if has_capability("allow-env") {
            std::env::var(key).map_err(|e| e.to_string())
        } else {
            Err("Capability allow-env not granted".to_string())
        }
    }

    #[test]
    fn test_read_without_capability() {
        let result = check_read("/etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn test_read_with_capability() {
        enable_capability("allow-read");
        let result = check_read("/tmp/test.txt");
        assert!(result.is_ok());
    }

    #[test]
    fn test_net_without_capability() {
        let result = check_net("https://evil.com");
        assert!(result.is_err());
    }

    #[test]
    fn test_net_with_capability() {
        enable_capability("allow-net");
        let result = check_net("https://example.com");
        assert!(result.is_ok());
    }

    #[test]
    fn test_env_without_capability() {
        let result = check_env("SECRET_KEY");
        assert!(result.is_err());
    }

    #[test]
    fn test_env_with_capability() {
        enable_capability("allow-env");
        std::env::set_var("TEST_VAR", "test_value");
        let result = check_env("TEST_VAR");
        assert!(result.is_ok());
    }

    #[test]
    fn test_capability_scope_escape() {
        enable_capability("allow-read=/home/user");

        let result = check_read("/etc/passwd");
        assert!(result.is_err(), "Should not read outside allowed scope");
    }

    #[test]
    fn test_wildcard_capability() {
        enable_capability("allow-read=*");

        let result = check_read("/etc/passwd");
        assert!(result.is_ok(), "Wildcard should allow all reads");
    }

    #[test]
    fn test_deny_overrides_allow() {
        enable_capability("allow-read");
        enable_capability("deny-read=/etc/passwd");

        let result = check_read("/etc/passwd");
        assert!(result.is_err(), "Deny should override allow");
    }
}