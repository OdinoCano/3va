//! # vvva_firewall
//!
//! HTTP firewall for the 3va runtime's built-in HTTP server.
//!
//! ## What it protects against
//!
//! | Attack | Mechanism |
//! |--------|-----------|
//! | **Slowloris** | Per-line `header_timeout_ms` deadline — a connection that never finishes sending headers is dropped |
//! | **RUDY** | `body_timeout_ms` deadline on `read_exact` — slow POST bodies are aborted |
//! | **Header flood** | `max_header_count` + `max_header_bytes` limits |
//! | **Rate-based DDoS** | Token-bucket per IP; IPs that exceed `auto_block_threshold` violations are blocked |
//! | **Connection exhaustion** | `max_connections_per_ip` and `max_connections_total` caps |
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use vvva_firewall::{Firewall, FirewallConfig};
//!
//! let fw = Firewall::new(FirewallConfig::default());
//! // pass fw into JsEngine::new_with_firewall(permissions, fw)
//! ```
//!
//! ## Configuration via `3va.config.ts`
//!
//! ```ts
//! export default {
//!   firewall: {
//!     enabled: true,
//!     rateLimitRps: 100,
//!     rateLimitBurst: 200,
//!     autoBlockThreshold: 10,
//!     blockDurationSecs: 300,
//!     headerTimeoutMs: 10_000,
//!     bodyTimeoutMs: 30_000,
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct FirewallConfig {
    /// Enable the firewall (default: true).
    pub enabled: bool,

    /// Max requests per second per IP (token bucket refill rate).
    pub rate_limit_rps: u32,

    /// Burst capacity: how many requests an IP can fire before the rate limit kicks in.
    pub rate_limit_burst: u32,

    /// How many rate-limit violations before the IP is auto-blocked.
    pub auto_block_threshold: u32,

    /// Duration to block an offending IP, in seconds.
    pub block_duration_secs: u64,

    /// Max simultaneous open connections from a single IP.
    pub max_connections_per_ip: u32,

    /// Max total simultaneous open connections across all IPs.
    pub max_connections_total: u32,

    /// Timeout for receiving the full HTTP request line + headers, in milliseconds.
    /// Protects against Slowloris: attacker sends headers one byte per second.
    pub header_timeout_ms: u64,

    /// Timeout for reading the request body after headers are complete, in milliseconds.
    /// Protects against RUDY: attacker sends body one byte per second.
    pub body_timeout_ms: u64,

    /// Maximum number of HTTP headers accepted per request.
    pub max_header_count: u32,

    /// Maximum total bytes consumed by all HTTP headers combined.
    pub max_header_bytes: u32,

    /// Maximum body size in bytes (0 = use the http_server 100 MB cap).
    pub max_body_bytes: u32,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rate_limit_rps: 100,
            rate_limit_burst: 200,
            auto_block_threshold: 10,
            block_duration_secs: 300,
            max_connections_per_ip: 50,
            max_connections_total: 10_000,
            header_timeout_ms: 10_000,
            body_timeout_ms: 30_000,
            max_header_count: 100,
            max_header_bytes: 16_384,
            max_body_bytes: 0,
        }
    }
}

// ── Token Bucket ──────────────────────────────────────────────────────────────

struct TokenBucket {
    capacity: f64,
    tokens: f64,
    rate: f64,
    last_refill: Instant,
    /// Consecutive violations (rate-limit exceeded).
    violations: u32,
}

impl TokenBucket {
    fn new(rate_rps: u32, burst: u32) -> Self {
        let capacity = burst as f64;
        Self {
            capacity,
            tokens: capacity,
            rate: rate_rps as f64,
            last_refill: Instant::now(),
            violations: 0,
        }
    }

    /// Try to consume one token. Returns `true` if allowed.
    fn consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.last_refill = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            // Reset violations on a successful request.
            if self.violations > 0 {
                self.violations = self.violations.saturating_sub(1);
            }
            true
        } else {
            self.violations += 1;
            false
        }
    }

    /// True if the bucket hasn't been touched recently (can be garbage collected).
    fn is_idle(&self, idle_threshold: Duration) -> bool {
        self.last_refill.elapsed() > idle_threshold
    }
}

// ── Blocklist ────────────────────────────────────────────────────────────────

struct BlockEntry {
    expires: Instant,
    reason: BlockReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    RateLimitViolation,
    ManualBlock,
}

// ── Decision ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum FirewallDecision {
    Allow,
    RateLimited {
        retry_after_ms: u64,
    },
    Blocked {
        reason: BlockReason,
        remaining_ms: u64,
    },
    ConnectionLimitReached,
}

impl FirewallDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, FirewallDecision::Allow)
    }

    /// HTTP 429 or 503 status code appropriate for this decision.
    pub fn http_status(&self) -> u16 {
        match self {
            FirewallDecision::Allow => 200,
            FirewallDecision::RateLimited { .. } => 429,
            FirewallDecision::Blocked { .. } => 403,
            FirewallDecision::ConnectionLimitReached => 503,
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            FirewallDecision::Allow => "OK",
            FirewallDecision::RateLimited { .. } => "Too Many Requests",
            FirewallDecision::Blocked { .. } => "Forbidden",
            FirewallDecision::ConnectionLimitReached => "Service Unavailable",
        }
    }
}

// ── Firewall ──────────────────────────────────────────────────────────────────

pub struct Firewall {
    pub config: FirewallConfig,
    buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
    blocklist: Mutex<HashMap<IpAddr, BlockEntry>>,
    /// Per-IP open connection count.
    conn_per_ip: Mutex<HashMap<IpAddr, u32>>,
    /// Total open connection count.
    conn_total: Mutex<u32>,
}

impl Firewall {
    pub fn new(config: FirewallConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            buckets: Mutex::new(HashMap::new()),
            blocklist: Mutex::new(HashMap::new()),
            conn_per_ip: Mutex::new(HashMap::new()),
            conn_total: Mutex::new(0),
        })
    }

    /// Main entry point: call when a new TCP connection is accepted.
    /// Returns the decision before reading any request bytes.
    pub fn check_connection(&self, ip: IpAddr) -> FirewallDecision {
        if !self.config.enabled {
            return FirewallDecision::Allow;
        }

        // 1. Blocklist check.
        if let Some(decision) = self.check_blocklist(ip) {
            warn!(ip = %ip, "Firewall: blocked IP attempted connection");
            return decision;
        }

        // 2. Total connection cap.
        {
            let total = *self.conn_total.lock().unwrap();
            if total >= self.config.max_connections_total {
                warn!(ip = %ip, total, "Firewall: total connection limit reached");
                return FirewallDecision::ConnectionLimitReached;
            }
        }

        // 3. Per-IP connection cap.
        {
            let per_ip = self.conn_per_ip.lock().unwrap();
            let count = per_ip.get(&ip).copied().unwrap_or(0);
            if count >= self.config.max_connections_per_ip {
                warn!(ip = %ip, count, "Firewall: per-IP connection limit reached");
                return FirewallDecision::ConnectionLimitReached;
            }
        }

        FirewallDecision::Allow
    }

    /// Call after parsing each HTTP request (once per request on a connection).
    pub fn check_request(&self, ip: IpAddr) -> FirewallDecision {
        if !self.config.enabled {
            return FirewallDecision::Allow;
        }

        // Re-check blocklist (the IP might have been blocked mid-connection).
        if let Some(decision) = self.check_blocklist(ip) {
            return decision;
        }

        // Token bucket.
        let mut buckets = self.buckets.lock().unwrap();
        let bucket = buckets.entry(ip).or_insert_with(|| {
            TokenBucket::new(self.config.rate_limit_rps, self.config.rate_limit_burst)
        });

        if bucket.consume() {
            return FirewallDecision::Allow;
        }

        let violations = bucket.violations;
        drop(buckets);

        // Auto-block after threshold violations.
        if violations >= self.config.auto_block_threshold {
            let duration = Duration::from_secs(self.config.block_duration_secs);
            self.block_ip(ip, duration, BlockReason::RateLimitViolation);
            warn!(ip = %ip, violations, "Firewall: auto-blocked IP after rate limit violations");
            return FirewallDecision::Blocked {
                reason: BlockReason::RateLimitViolation,
                remaining_ms: self.config.block_duration_secs * 1000,
            };
        }

        let retry_after_ms = (1000.0 / self.config.rate_limit_rps as f64) as u64;
        warn!(ip = %ip, violations, "Firewall: rate limited");
        FirewallDecision::RateLimited { retry_after_ms }
    }

    /// Register that a connection has been opened.
    pub fn on_connect(&self, ip: IpAddr) {
        if !self.config.enabled {
            return;
        }
        *self.conn_per_ip.lock().unwrap().entry(ip).or_insert(0) += 1;
        *self.conn_total.lock().unwrap() += 1;
    }

    /// Register that a connection has been closed.
    pub fn on_disconnect(&self, ip: IpAddr) {
        if !self.config.enabled {
            return;
        }
        let mut per_ip = self.conn_per_ip.lock().unwrap();
        if let Some(c) = per_ip.get_mut(&ip) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                per_ip.remove(&ip);
            }
        }
        let mut total = self.conn_total.lock().unwrap();
        *total = total.saturating_sub(1);
    }

    /// Manually block an IP for the given duration.
    pub fn block_ip(&self, ip: IpAddr, duration: Duration, reason: BlockReason) {
        self.blocklist.lock().unwrap().insert(
            ip,
            BlockEntry {
                expires: Instant::now() + duration,
                reason,
            },
        );
    }

    /// Remove an IP from the blocklist.
    pub fn unblock_ip(&self, ip: IpAddr) {
        self.blocklist.lock().unwrap().remove(&ip);
    }

    /// Returns true if the IP is currently blocked.
    pub fn is_blocked(&self, ip: IpAddr) -> bool {
        let list = self.blocklist.lock().unwrap();
        if let Some(entry) = list.get(&ip) {
            return entry.expires > Instant::now();
        }
        false
    }

    /// Remove expired entries from blocklist and idle entries from rate-limit buckets.
    /// Call this periodically (e.g. every 60 s) to prevent unbounded memory growth.
    pub fn cleanup(&self) {
        let now = Instant::now();

        {
            let mut list = self.blocklist.lock().unwrap();
            list.retain(|_, entry| entry.expires > now);
        }

        {
            let idle = Duration::from_secs(300);
            let mut buckets = self.buckets.lock().unwrap();
            buckets.retain(|_, bucket| !bucket.is_idle(idle));
        }
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn check_blocklist(&self, ip: IpAddr) -> Option<FirewallDecision> {
        let list = self.blocklist.lock().unwrap();
        if let Some(entry) = list.get(&ip) {
            let now = Instant::now();
            if entry.expires > now {
                let remaining_ms = entry.expires.duration_since(now).as_millis() as u64;
                return Some(FirewallDecision::Blocked {
                    reason: entry.reason,
                    remaining_ms,
                });
            }
        }
        None
    }
}

// ── Background cleanup task ───────────────────────────────────────────────────

/// Spawn a Tokio task that calls `firewall.cleanup()` every `interval`.
/// Returns a handle — drop it to stop the task.
pub fn spawn_cleanup_task(
    firewall: Arc<Firewall>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            firewall.cleanup();
        }
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(1, 2, 3, a))
    }

    fn fw_tight() -> Arc<Firewall> {
        Firewall::new(FirewallConfig {
            rate_limit_rps: 2,
            rate_limit_burst: 2,
            auto_block_threshold: 3,
            block_duration_secs: 60,
            max_connections_per_ip: 2,
            max_connections_total: 5,
            ..FirewallConfig::default()
        })
    }

    #[test]
    fn allow_within_burst() {
        let fw = fw_tight();
        let a = ip(1);
        assert!(fw.check_request(a).is_allowed());
        assert!(fw.check_request(a).is_allowed());
    }

    #[test]
    fn rate_limited_after_burst() {
        let fw = fw_tight();
        let a = ip(2);
        fw.check_request(a);
        fw.check_request(a);
        // burst exhausted
        let d = fw.check_request(a);
        assert!(matches!(d, FirewallDecision::RateLimited { .. }));
    }

    #[test]
    fn auto_block_after_threshold() {
        let fw = fw_tight();
        let a = ip(3);
        // exhaust burst then hit threshold
        for _ in 0..10 {
            fw.check_request(a);
        }
        assert!(fw.is_blocked(a));
    }

    #[test]
    fn manual_block_and_unblock() {
        let fw = fw_tight();
        let a = ip(4);
        fw.block_ip(a, Duration::from_secs(60), BlockReason::ManualBlock);
        assert!(matches!(
            fw.check_connection(a),
            FirewallDecision::Blocked { .. }
        ));
        fw.unblock_ip(a);
        assert!(fw.check_connection(a).is_allowed());
    }

    #[test]
    fn connection_tracking() {
        let fw = fw_tight();
        let a = ip(5);
        fw.on_connect(a);
        fw.on_connect(a);
        // third connection should be refused
        assert!(matches!(
            fw.check_connection(a),
            FirewallDecision::ConnectionLimitReached
        ));
        fw.on_disconnect(a);
        assert!(fw.check_connection(a).is_allowed());
    }

    #[test]
    fn total_connection_cap() {
        let fw = fw_tight();
        for i in 1..=5 {
            let a = ip(i);
            fw.on_connect(a);
        }
        // 6th connection from a new IP should be refused
        assert!(matches!(
            fw.check_connection(ip(6)),
            FirewallDecision::ConnectionLimitReached
        ));
    }

    #[test]
    fn cleanup_removes_expired_blocks() {
        let fw = fw_tight();
        let a = ip(7);
        fw.block_ip(a, Duration::from_nanos(1), BlockReason::ManualBlock);
        std::thread::sleep(Duration::from_millis(2));
        fw.cleanup();
        assert!(!fw.is_blocked(a));
    }

    #[test]
    fn check_connection_allows_fresh_ip() {
        let fw = fw_tight();
        assert!(fw.check_connection(ip(10)).is_allowed());
    }

    #[test]
    fn disabled_firewall_allows_everything() {
        let fw = Firewall::new(FirewallConfig {
            enabled: false,
            ..FirewallConfig::default()
        });
        let a = ip(20);
        // Manually block the IP — should still be allowed because firewall is off.
        fw.block_ip(a, Duration::from_secs(60), BlockReason::ManualBlock);
        assert!(fw.check_connection(a).is_allowed());
        assert!(fw.check_request(a).is_allowed());
        // on_connect / on_disconnect must not panic when disabled.
        fw.on_connect(a);
        fw.on_disconnect(a);
    }

    #[test]
    fn decision_http_status_codes() {
        assert_eq!(FirewallDecision::Allow.http_status(), 200);
        assert_eq!(
            FirewallDecision::RateLimited { retry_after_ms: 10 }.http_status(),
            429
        );
        assert_eq!(
            FirewallDecision::Blocked {
                reason: BlockReason::ManualBlock,
                remaining_ms: 1000
            }
            .http_status(),
            403
        );
        assert_eq!(FirewallDecision::ConnectionLimitReached.http_status(), 503);
    }

    #[test]
    fn decision_messages() {
        assert_eq!(FirewallDecision::Allow.message(), "OK");
        assert_eq!(
            FirewallDecision::RateLimited { retry_after_ms: 10 }.message(),
            "Too Many Requests"
        );
        assert_eq!(
            FirewallDecision::Blocked {
                reason: BlockReason::ManualBlock,
                remaining_ms: 0
            }
            .message(),
            "Forbidden"
        );
        assert_eq!(
            FirewallDecision::ConnectionLimitReached.message(),
            "Service Unavailable"
        );
    }

    #[test]
    fn auto_block_reason_is_rate_limit_violation() {
        let fw = fw_tight(); // burst=2, threshold=3
        let a = ip(30);
        for _ in 0..10 {
            fw.check_request(a);
        }
        // After auto-block, check_connection returns Blocked with RateLimitViolation reason.
        match fw.check_connection(a) {
            FirewallDecision::Blocked { reason, .. } => {
                assert_eq!(reason, BlockReason::RateLimitViolation);
            }
            other => panic!("expected Blocked, got {:?}", other),
        }
    }

    #[test]
    fn block_remaining_ms_is_positive() {
        let fw = fw_tight();
        let a = ip(40);
        fw.block_ip(a, Duration::from_secs(60), BlockReason::ManualBlock);
        match fw.check_connection(a) {
            FirewallDecision::Blocked { remaining_ms, .. } => {
                assert!(remaining_ms > 0, "remaining_ms should be > 0");
                assert!(
                    remaining_ms <= 60_000,
                    "remaining_ms should not exceed block duration"
                );
            }
            other => panic!("expected Blocked, got {:?}", other),
        }
    }

    #[test]
    fn connection_count_stays_consistent_after_disconnect() {
        let fw = fw_tight(); // max_connections_per_ip=2
        let a = ip(50);
        fw.on_connect(a);
        fw.on_connect(a);
        fw.on_disconnect(a);
        fw.on_disconnect(a);
        // After two disconnects the per-IP counter is gone → new connections allowed.
        assert!(fw.check_connection(a).is_allowed());
    }

    #[test]
    fn disconnect_below_zero_does_not_panic() {
        let fw = fw_tight();
        let a = ip(60);
        // Calling on_disconnect without on_connect must not panic or underflow.
        fw.on_disconnect(a);
        fw.on_disconnect(a);
        assert!(fw.check_connection(a).is_allowed());
    }
}
