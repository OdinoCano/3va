use std::collections::HashMap;
use std::path::Path;

// ── .npmrc parser ──────────────────────────────────────────────────────────────
//
// Format:
//
//   registry=https://registry.npmjs.org
//   //registry.npmjs.org/:_authToken=xxxx
//   @myscope:registry=https://my-private-registry.com
//   //my-private-registry.com/:always-auth=true
//   cache=/path/to/cache
//
//   Lines starting with ';' or '#' are comments.
//   Empty lines are ignored.

/// Parsed .npmrc configuration.
#[derive(Debug, Clone, Default)]
pub struct NpmrcConfig {
    /// Default registry URL (from `registry=...`)
    pub registry: Option<String>,
    /// Scoped registries: scope (e.g. "@myscope") → registry URL
    pub scoped_registries: HashMap<String, String>,
    /// Auth tokens per registry host: host → token
    pub auth_tokens: HashMap<String, String>,
    /// Auth credentials per registry host: host → "username:password" (base64)
    pub auth_credentials: HashMap<String, String>,
    /// Always-auth flag per registry host
    pub always_auth: HashMap<String, bool>,
    /// Raw key-value pairs not categorized above
    pub raw: HashMap<String, String>,
}

/// Parse an .npmrc string into a config.
pub fn parse_npmrc(content: &str) -> NpmrcConfig {
    let mut config = NpmrcConfig::default();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.is_empty()
            || trimmed.starts_with(';')
            || trimmed.starts_with('#')
        {
            continue;
        }

        // Skip lines without '='
        let eq_pos = match trimmed.find('=') {
            Some(p) => p,
            None => continue,
        };

        let key = trimmed[..eq_pos].trim();
        let value = trimmed[eq_pos + 1..].trim().trim_matches('"').to_string();

        if key == "registry" {
            config.registry = Some(normalize_registry_url(&value));
        } else if let Some(scope) = key.strip_suffix(":registry") {
            // @myscope:registry=...
            config
                .scoped_registries
                .insert(scope.to_string(), normalize_registry_url(&value));
        } else if let Some(host) = key.strip_suffix(":_authToken") {
            // //registry.npmjs.org/:_authToken=...
            config
                .auth_tokens
                .insert(normalize_registry_host(host), value);
        } else if let Some(host) = key.strip_suffix(":_auth") {
            // //registry.npmjs.org/:_auth=base64user:pass
            config
                .auth_credentials
                .insert(normalize_registry_host(host), value);
        } else if let Some(host) = key.strip_suffix(":always-auth") {
            // //registry.npmjs.org/:always-auth=true
            config
                .always_auth
                .insert(normalize_registry_host(host), value == "true");
        } else {
            config.raw.insert(key.to_string(), value);
        }
    }

    config
}

fn normalize_registry_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    // Ensure https:// prefix if no scheme
    if url.contains("://") {
        url.to_string()
    } else {
        format!("https://{}", url)
    }
}

fn normalize_registry_host(host: &str) -> String {
    host.trim()
        .trim_start_matches("//")
        .trim_end_matches('/')
        .trim_end_matches(':')
        .to_string()
}

/// Discover .npmrc files in order of precedence:
/// 1. Project-level (.npmrc in the project root)
/// 2. User-level (~/.npmrc)
/// 3. Global ($PREFIX/etc/npmrc)
///
/// Later files override earlier ones.
pub fn discover_npmrc(project_root: Option<&Path>) -> NpmrcConfig {
    let mut config = NpmrcConfig::default();

    // Global: $PREFIX/etc/npmrc (if PREFIX is set)
    if let Ok(prefix) = std::env::var("PREFIX") {
        let global_path = Path::new(&prefix).join("etc").join("npmrc");
        if let Ok(content) = std::fs::read_to_string(&global_path) {
            config = merge_configs(config, parse_npmrc(&content));
        }
    }

    // User-level: ~/.npmrc
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let user_path = Path::new(&home).join(".npmrc");
    if let Ok(content) = std::fs::read_to_string(&user_path) {
        config = merge_configs(config, parse_npmrc(&content));
    }

    // Project-level: project_root/.npmrc
    if let Some(root) = project_root {
        let project_path = root.join(".npmrc");
        if let Ok(content) = std::fs::read_to_string(&project_path) {
            config = merge_configs(config, parse_npmrc(&content));
        }
    }

    config
}

fn merge_configs(base: NpmrcConfig, overrides: NpmrcConfig) -> NpmrcConfig {
    let mut merged = base;

    if let Some(reg) = overrides.registry {
        merged.registry = Some(reg);
    }
    for (scope, url) in overrides.scoped_registries {
        merged.scoped_registries.insert(scope, url);
    }
    for (host, token) in overrides.auth_tokens {
        merged.auth_tokens.insert(host, token);
    }
    for (host, creds) in overrides.auth_credentials {
        merged.auth_credentials.insert(host, creds);
    }
    for (host, val) in overrides.always_auth {
        merged.always_auth.insert(host, val);
    }
    for (k, v) in overrides.raw {
        merged.raw.insert(k, v);
    }

    merged
}

/// Resolve the registry URL for a given package name, based on .npmrc config.
///
/// If the package has a scope (e.g. `@scope/pkg`), checks scoped registries first,
/// then falls back to the default registry.
pub fn resolve_registry(config: &NpmrcConfig, package_name: &str) -> String {
    if package_name.starts_with('@') {
        let scope = package_name.split('/').next().unwrap_or(package_name);
        if let Some(url) = config.scoped_registries.get(scope) {
            return url.clone();
        }
    }
    config
        .registry
        .clone()
        .unwrap_or_else(|| "https://registry.npmjs.org".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_registry() {
        let content = "registry=https://registry.npmjs.org";
        let config = parse_npmrc(content);
        assert_eq!(
            config.registry.as_deref(),
            Some("https://registry.npmjs.org")
        );
    }

    #[test]
    fn parse_registry_without_scheme() {
        let content = "registry=registry.npmjs.org";
        let config = parse_npmrc(content);
        assert_eq!(
            config.registry.as_deref(),
            Some("https://registry.npmjs.org")
        );
    }

    #[test]
    fn parse_registry_custom() {
        let content = "registry=https://npm.pkg.github.com/my-org";
        let config = parse_npmrc(content);
        assert_eq!(
            config.registry.as_deref(),
            Some("https://npm.pkg.github.com/my-org")
        );
    }

    #[test]
    fn parse_auth_token() {
        let content = "//registry.npmjs.org/:_authToken=my-secret-token";
        let config = parse_npmrc(content);
        assert_eq!(
            config.auth_tokens.get("registry.npmjs.org"),
            Some(&"my-secret-token".to_string())
        );
    }

    #[test]
    fn parse_scoped_registry() {
        let content = "@my-org:registry=https://npm.pkg.github.com";
        let config = parse_npmrc(content);
        assert_eq!(
            config.scoped_registries.get("@my-org"),
            Some(&"https://npm.pkg.github.com".to_string())
        );
    }

    #[test]
    fn parse_always_auth() {
        let content = "//registry.npmjs.org/:always-auth=true";
        let config = parse_npmrc(content);
        assert_eq!(config.always_auth.get("registry.npmjs.org"), Some(&true));
    }

    #[test]
    fn parse_comments_ignored() {
        let content = "; this is a comment\n# this is also a comment\nregistry=https://registry.npmjs.org";
        let config = parse_npmrc(content);
        assert!(config.registry.is_some());
    }

    #[test]
    fn parse_empty_lines_ignored() {
        let content = "\n\nregistry=https://registry.npmjs.org\n\n";
        let config = parse_npmrc(content);
        assert!(config.registry.is_some());
    }

    #[test]
    fn resolve_registry_default() {
        let config = NpmrcConfig::default();
        let url = resolve_registry(&config, "lodash");
        assert_eq!(url, "https://registry.npmjs.org");
    }

    #[test]
    fn resolve_registry_scoped() {
        let mut config = NpmrcConfig::default();
        config
            .scoped_registries
            .insert("@my-org".to_string(), "https://npm.pkg.github.com".to_string());
        let url = resolve_registry(&config, "@my-org/pkg");
        assert_eq!(url, "https://npm.pkg.github.com");
    }

    #[test]
    fn resolve_registry_custom_default() {
        let mut config = NpmrcConfig::default();
        config.registry = Some("https://custom.registry.com".to_string());
        let url = resolve_registry(&config, "lodash");
        assert_eq!(url, "https://custom.registry.com");
    }

    #[test]
    fn discover_npmrc_project_level() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(".npmrc"),
            "registry=https://my-private-registry.com\n@my-scope:registry=https://other-registry.com",
        )
        .unwrap();

        let config = discover_npmrc(Some(dir.path()));
        assert_eq!(
            config.registry.as_deref(),
            Some("https://my-private-registry.com")
        );
        assert_eq!(
            config.scoped_registries.get("@my-scope"),
            Some(&"https://other-registry.com".to_string())
        );
    }

    #[test]
    fn parse_auth_credentials() {
        let content = "//registry.npmjs.org/:_auth=dXNlcjpwYXNz";
        let config = parse_npmrc(content);
        assert_eq!(
            config.auth_credentials.get("registry.npmjs.org"),
            Some(&"dXNlcjpwYXNz".to_string())
        );
    }

    #[test]
    fn raw_key_values() {
        let content = "cache=/tmp/cache\nloglevel=warn";
        let config = parse_npmrc(content);
        assert_eq!(config.raw.get("cache"), Some(&"/tmp/cache".to_string()));
        assert_eq!(config.raw.get("loglevel"), Some(&"warn".to_string()));
    }
}
