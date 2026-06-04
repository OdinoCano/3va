//! Apply `3VA_<SECTION>_<KEY>` environment-variable overrides to a loaded config.
//!
//! Priority: CLI flags > env vars > config file > built-in defaults.
//!
//! Variables follow the pattern `3VA_<SECTION>_<KEY>` (uppercase, underscores
//! for camelCase boundaries).  Examples:
//!
//! - `3VA_DEV_PORT=8080`          → `config.dev.port`
//! - `3VA_TEST_COVERAGE=true`     → `config.test.coverage`
//! - `3VA_BUNDLE_MINIFY=true`     → `config.bundle.minify`
//! - `3VA_WORKSPACE_PARALLELISM=8` → `config.workspace.parallelism`

use crate::schema::ProjectConfig;
use std::env;

pub fn apply(mut cfg: ProjectConfig) -> ProjectConfig {
    // dev
    if let Ok(v) = env::var("3VA_DEV_PORT") {
        if let Ok(n) = v.parse::<u16>() {
            cfg.dev.port = n;
        }
    }
    if let Ok(v) = env::var("3VA_DEV_HOST") {
        cfg.dev.host = v;
    }
    if let Ok(v) = env::var("3VA_DEV_PUBLIC_DIR") {
        cfg.dev.public_dir = v;
    }
    if let Ok(v) = env::var("3VA_DEV_OPEN") {
        cfg.dev.open = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_DEV_CSP") {
        cfg.dev.csp.enabled = parse_bool(&v);
    }

    // test
    if let Ok(v) = env::var("3VA_TEST_COVERAGE") {
        cfg.test.coverage = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_TEST_WATCH") {
        cfg.test.watch = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_TEST_UPDATE_SNAPSHOTS") {
        cfg.test.update_snapshots = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_TEST_CONCURRENCY") {
        if let Ok(n) = v.parse::<usize>() {
            cfg.test.concurrency = n;
        }
    }

    // audit
    if let Ok(v) = env::var("3VA_AUDIT_DENY") {
        cfg.audit.deny = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_AUDIT_SECRETS") {
        cfg.audit.secrets = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_AUDIT_UPDATE_CACHE") {
        cfg.audit.update_cache = parse_bool(&v);
    }

    // bundle
    if let Ok(v) = env::var("3VA_BUNDLE_OUT_DIR") {
        cfg.bundle.out_dir = v;
    }
    if let Ok(v) = env::var("3VA_BUNDLE_MINIFY") {
        cfg.bundle.minify = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_BUNDLE_SOURCE_MAP") {
        cfg.bundle.source_map = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_BUNDLE_SPLIT") {
        cfg.bundle.split = parse_bool(&v);
    }

    // workspace
    if let Ok(v) = env::var("3VA_WORKSPACE_HOISTING") {
        cfg.workspace.hoisting = parse_bool(&v);
    }
    if let Ok(v) = env::var("3VA_WORKSPACE_PARALLELISM") {
        if let Ok(n) = v.parse::<usize>() {
            cfg.workspace.parallelism = n;
        }
    }

    cfg
}

fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ProjectConfig;

    #[test]
    fn env_override_dev_port() {
        // Safety: tests are single-threaded here; set/remove is fine.
        unsafe { env::set_var("3VA_DEV_PORT", "9999") };
        let cfg = apply(ProjectConfig::default());
        unsafe { env::remove_var("3VA_DEV_PORT") };
        assert_eq!(cfg.dev.port, 9999);
    }

    #[test]
    fn env_override_test_coverage() {
        unsafe { env::set_var("3VA_TEST_COVERAGE", "true") };
        let cfg = apply(ProjectConfig::default());
        unsafe { env::remove_var("3VA_TEST_COVERAGE") };
        assert!(cfg.test.coverage);
    }

    #[test]
    fn env_override_workspace_parallelism() {
        unsafe { env::set_var("3VA_WORKSPACE_PARALLELISM", "8") };
        let cfg = apply(ProjectConfig::default());
        unsafe { env::remove_var("3VA_WORKSPACE_PARALLELISM") };
        assert_eq!(cfg.workspace.parallelism, 8);
    }

    #[test]
    fn parse_bool_variants() {
        for t in &["1", "true", "yes", "on", "True", "YES"] {
            assert!(parse_bool(t), "{t} should be truthy");
        }
        for f in &["0", "false", "no", "off", ""] {
            assert!(!parse_bool(f), "{f} should be falsy");
        }
    }
}
