//! # vvva_firewall
//!
//! HTTP firewall for the 3va runtime's built-in HTTP server.
//!
//! ## What it protects against
//!
//! | Attack | Mechanism |
//! |--------|-----------|
//! | **Slowloris** | Per-line `header_timeout_ms` deadline — a connection that never finishes sending headers is dropped |
//! | **RUDY** | `body_timeout_ms` deadline on `read_exact` plus a `min_body_rate_bps` average-rate check — slow POST bodies are aborted |
//! | **Header flood** | `max_header_count` + `max_header_bytes` limits |
//! | **Rate-based DDoS** | Token-bucket per IP; IPs that exceed `auto_block_threshold` violations are blocked |
//! | **Adaptive (repeat offenders)** | Each auto-block adds a strike; the block duration escalates as `block_duration_secs × factor^(strikes-1)` up to `max_block_duration_secs`, and the history clears after `strike_decay_secs` of calm |
//! | **Connection exhaustion** | `max_connections_per_ip` and `max_connections_total` caps |
//!
//! ## Quick start
//!
//! ```rust
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
//!     blockEscalationFactor: 2,
//!     maxBlockDurationSecs: 3600,
//!     strikeDecaySecs: 3600,
//!     headerTimeoutMs: 10_000,
//!     bodyTimeoutMs: 30_000,
//!     minBodyRateBps: 100,
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

    /// Base duration to block an offending IP, in seconds.
    pub block_duration_secs: u64,

    /// Multiplier applied to the block duration for each repeat offense
    /// (adaptive escalation). 1 = fixed duration, no escalation.
    pub block_escalation_factor: u32,

    /// Upper bound on the escalated block duration, in seconds.
    pub max_block_duration_secs: u64,

    /// Seconds of calm (no auto-block) after which an IP's strike history
    /// clears and its block duration resets to `block_duration_secs`.
    pub strike_decay_secs: u64,

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

    /// Minimum body receive rate in bytes per second. Connections dripping
    /// body data slower than this are dropped (RUDY mitigation). 0 = disabled.
    pub min_body_rate_bps: u32,

    /// Adaptive rate limiting: when enabled, an IP whose observed legitimate
    /// traffic baseline (EWMA over 1-second windows) exceeds the static
    /// `rate_limit_rps` gets a proportionally raised threshold instead of
    /// being rate-limited, so gradual traffic growth isn't punished.
    pub adaptive_rate_limit: bool,

    /// EWMA smoothing factor as a percentage (0–100). Higher values adapt
    /// faster to traffic changes; lower values smooth more. Only used when
    /// `adaptive_rate_limit` is on.
    pub ewma_alpha_pct: u32,

    /// Reverse proxies trusted to set `X-Forwarded-For`. Each entry is a bare
    /// IP or CIDR (e.g. `"10.0.0.0/8"`, `"::1"`). When the direct peer is a
    /// trusted proxy, `req.socket.remoteAddress` reports the right-most
    /// non-trusted address from `X-Forwarded-For` and the firewall accounts
    /// rate limits against it; otherwise the header is ignored entirely.
    pub trusted_proxies: Vec<String>,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rate_limit_rps: 100,
            rate_limit_burst: 200,
            auto_block_threshold: 10,
            block_duration_secs: 300,
            block_escalation_factor: 2,
            max_block_duration_secs: 3600,
            strike_decay_secs: 3600,
            max_connections_per_ip: 50,
            max_connections_total: 10_000,
            header_timeout_ms: 10_000,
            body_timeout_ms: 30_000,
            max_header_count: 100,
            max_header_bytes: 16_384,
            max_body_bytes: 0,
            min_body_rate_bps: 100,
            adaptive_rate_limit: false,
            ewma_alpha_pct: 20,
            trusted_proxies: Vec::new(),
        }
    }
}

// ── Trusted proxies / X-Forwarded-For ────────────────────────────────────────

/// One trusted-proxy entry: a bare IP or a CIDR range.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyRule {
    addr: IpAddr,
    prefix: u8,
}

impl ProxyRule {
    /// Parse `"1.2.3.4"`, `"10.0.0.0/8"`, `"::1"`, `"fd00::/8"` …
    pub fn parse(s: &str) -> Option<Self> {
        let (ip_part, prefix) = match s.split_once('/') {
            Some((ip, p)) => (ip, p.parse::<u8>().ok()?),
            None => (
                s,
                match s.parse::<IpAddr>() {
                    Ok(IpAddr::V4(_)) => 32,
                    Ok(IpAddr::V6(_)) => 128,
                    Err(_) => return None,
                },
            ),
        };
        let addr: IpAddr = ip_part.trim().parse().ok()?;
        let max = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > max {
            return None;
        }
        Some(ProxyRule { addr, prefix })
    }

    /// True when `ip` falls inside this rule's range. Families must match
    /// (an IPv4 rule never matches an IPv6 address and vice versa).
    pub fn matches(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(base), IpAddr::V4(other)) => {
                if self.prefix == 0 {
                    return true;
                }
                let base = u32::from(base);
                let other = u32::from(other);
                let mask = u32::MAX << (32 - self.prefix);
                base & mask == other & mask
            }
            (IpAddr::V6(base), IpAddr::V6(other)) => {
                if self.prefix == 0 {
                    return true;
                }
                let base = u128::from(base);
                let other = u128::from(other);
                let mask = u128::MAX << (128 - self.prefix);
                base & mask == other & mask
            }
            _ => false,
        }
    }
}

/// Resolve the real client IP from an `X-Forwarded-For` header.
///
/// Safe only when the direct connection peer is itself a trusted proxy:
/// - peer NOT trusted → `None` (the header is attacker-controlled noise),
/// - walk entries right-to-left, skipping trusted proxies; the first
///   non-trusted entry is the client,
/// - a malformed entry stops the walk,
/// - if every entry is a trusted proxy, the left-most one wins (deepest
///   proxy chain we trust).
pub fn resolve_forwarded_for(peer: IpAddr, xff: &str, trusted: &[ProxyRule]) -> Option<IpAddr> {
    if !trusted.iter().any(|r| r.matches(peer)) {
        return None;
    }
    let entries: Vec<&str> = xff.split(',').map(str::trim).collect();
    // Right-to-left: first non-trusted, fully-parsed address.
    for part in entries.iter().rev() {
        match part.parse::<IpAddr>() {
            Ok(a) if !trusted.iter().any(|r| r.matches(a)) => return Some(a),
            Ok(_) => continue, // trusted intermediate hop
            Err(_) => break,   // malformed — stop walking
        }
    }
    // All entries trusted → leftmost parsed entry is the client as seen by
    // the innermost proxy.
    entries.first().and_then(|e| e.parse::<IpAddr>().ok())
}

// ── Adaptive rate limiting ───────────────────────────────────────────────────

/// Window (in seconds) over which per-IP request counts are folded into the
/// EWMA baseline.
pub const ADAPTIVE_WINDOW_SECS: u64 = 1;

/// Headroom multiplier applied to the EWMA baseline: an IP is allowed up to
/// `baseline × 1.5` req/s before the static limit bites, so normal jitter
/// around its own baseline doesn't produce violations.
pub const ADAPTIVE_HEADROOM: f64 = 1.5;

/// Hard cap on the adaptive threshold, as a multiple of `rate_limit_rps`.
/// Abuse stays bounded no matter how high a compromised IP pumps its own
/// baseline.
pub const ADAPTIVE_MAX_RATE_MULTIPLIER: f64 = 4.0;

/// Exponentially weighted moving average of window samples:
/// `ewma = α × sample + (1 − α) × prev`, with α = `alpha_pct`/100.
pub fn ewma_update(prev: f64, sample: f64, alpha_pct: u32) -> f64 {
    let alpha = f64::from(alpha_pct.min(100)) / 100.0;
    alpha * sample + (1.0 - alpha) * prev
}

/// Effective per-IP rate limit (req/s): static by default; with
/// `adaptive` on, `max(static, ceil(ewma × ADAPTIVE_HEADROOM))` capped at
/// `static × ADAPTIVE_MAX_RATE_MULTIPLIER`.
pub(crate) fn compute_effective_rps(
    static_rps: u32,
    ewma_per_window: Option<f64>,
    adaptive: bool,
) -> f64 {
    let base = f64::from(static_rps);
    if !adaptive {
        return base;
    }
    let baseline = ewma_per_window.unwrap_or(0.0);
    let adapted = (baseline * ADAPTIVE_HEADROOM).ceil();
    adapted.max(base).min(base * ADAPTIVE_MAX_RATE_MULTIPLIER)
}

// ── Token Bucket ──────────────────────────────────────────────────────────────

struct TokenBucket {
    capacity: f64,
    tokens: f64,
    rate: f64,
    last_refill: Instant,
    /// Consecutive violations (rate-limit exceeded).
    violations: u32,
    /// EWMA of observed requests per `ADAPTIVE_WINDOW_SECS` window
    /// (adaptive rate limiting baseline). None until the first window closes.
    ewma_per_window: Option<f64>,
    /// Requests observed in the currently open window.
    window_count: u32,
    /// When the current observation window opened.
    window_start: Option<Instant>,
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
            ewma_per_window: None,
            window_count: 0,
            window_start: None,
        }
    }

    /// Record one observed request at time `now`, rolling the EWMA whenever
    /// the current window closes. Splitting the instant out as a parameter
    /// keeps this deterministically testable.
    fn note_request_at(&mut self, now: Instant, alpha_pct: u32) {
        match self.window_start {
            None => self.window_start = Some(now),
            Some(start)
                if now.duration_since(start) >= Duration::from_secs(ADAPTIVE_WINDOW_SECS) =>
            {
                let sample = f64::from(self.window_count);
                self.ewma_per_window = Some(match self.ewma_per_window {
                    None => sample,
                    Some(prev) => ewma_update(prev, sample, alpha_pct),
                });
                self.window_count = 0;
                self.window_start = Some(now);
            }
            Some(_) => {}
        }
        self.window_count += 1;
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

/// Per-IP strike history for adaptive blocking. A strike is added every time
/// the IP is auto-blocked; the block duration for a strike count escalates as
/// `block_duration_secs × factor^(count-1)`, capped at `max_block_duration_secs`.
/// The record is dropped by `cleanup()` once `strike_decay_secs` have passed
/// without another auto-block, so a calm IP resets to the base duration.
struct StrikeRecord {
    count: u32,
    last: Instant,
}

/// Upper bound on an IP's strike count (bounds the escalation loop and the
/// value stored per IP; the duration cap governs the actual block length).
const MAX_STRIKES: u32 = 16;

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
    /// Per-IP adaptive strike history (see `StrikeRecord`).
    strikes: Mutex<HashMap<IpAddr, StrikeRecord>>,
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
            strikes: Mutex::new(HashMap::new()),
            conn_per_ip: Mutex::new(HashMap::new()),
            conn_total: Mutex::new(0),
        })
    }

    /// Effective client IP for a request that arrived from `peer`.
    ///
    /// When the peer is a trusted proxy (per `trusted_proxies`) and the
    /// request carried `X-Forwarded-For`, this is the resolved client
    /// address; otherwise it is the peer itself. Used both for rate-limit
    /// accounting and for `req.socket.remoteAddress`.
    pub fn client_ip_from_xff(&self, peer: IpAddr, xff: Option<&str>) -> IpAddr {
        let Some(xff) = xff else {
            return peer;
        };
        let rules: Vec<ProxyRule> = self
            .config
            .trusted_proxies
            .iter()
            .filter_map(|s| match ProxyRule::parse(s) {
                Some(r) => Some(r),
                None => {
                    warn!("Firewall: ignoring invalid trusted proxy entry '{s}'");
                    None
                }
            })
            .collect();
        if rules.is_empty() {
            return peer;
        }
        resolve_forwarded_for(peer, xff, &rules).unwrap_or(peer)
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

    /// Effective per-IP rate limit for a bucket, applying the adaptive
    /// baseline when enabled.
    fn effective_bucket_rate(&self, bucket: &TokenBucket) -> f64 {
        compute_effective_rps(
            self.config.rate_limit_rps,
            bucket.ewma_per_window,
            self.config.adaptive_rate_limit,
        )
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

        // Adaptive bookkeeping: fold this observed request into the EWMA
        // baseline and raise the bucket's refill rate if the IP's legitimate
        // traffic has outgrown the static limit.
        let alpha = self.config.ewma_alpha_pct;
        bucket.note_request_at(Instant::now(), alpha);
        bucket.rate = self.effective_bucket_rate(bucket);

        if bucket.consume() {
            return FirewallDecision::Allow;
        }

        let violations = bucket.violations;
        drop(buckets);

        // Auto-block after threshold violations. The block duration escalates
        // for repeat offenders (see `record_auto_block`).
        if violations >= self.config.auto_block_threshold {
            let duration_secs = self.record_auto_block(ip);
            let remaining_ms = duration_secs * 1000;
            warn!(ip = %ip, violations, duration_secs, "Firewall: auto-blocked IP after rate limit violations");
            return FirewallDecision::Blocked {
                reason: BlockReason::RateLimitViolation,
                remaining_ms,
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

        {
            // Drop strike history for IPs that have been calm long enough to
            // reset — repeat offenders who keep re-offending within the decay
            // window keep (and grow) their escalation.
            let decay = Duration::from_secs(self.config.strike_decay_secs);
            let mut strikes = self.strikes.lock().unwrap();
            strikes.retain(|_, rec| rec.last.elapsed() <= decay);
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

    /// Register an auto-block for `ip`: bump its strike count and block it for
    /// the (escalating) duration for that count. Returns the duration in seconds.
    fn record_auto_block(&self, ip: IpAddr) -> u64 {
        let mut strikes = self.strikes.lock().unwrap();
        let rec = strikes.entry(ip).or_insert(StrikeRecord {
            count: 0,
            last: Instant::now(),
        });
        rec.count = rec.count.saturating_add(1).min(MAX_STRIKES);
        rec.last = Instant::now();
        let count = rec.count;
        drop(strikes);

        let duration_secs = self.block_duration_secs_for(count);
        self.block_ip(
            ip,
            Duration::from_secs(duration_secs),
            BlockReason::RateLimitViolation,
        );
        duration_secs
    }

    /// Escalating block duration for a strike count: `block_duration_secs ×
    /// factor^(strikes-1)`, capped at `max_block_duration_secs` (never below the
    /// base duration). `strikes == 1` (a first offense) yields the base duration.
    fn block_duration_secs_for(&self, strikes: u32) -> u64 {
        let base = self.config.block_duration_secs;
        let factor = self.config.block_escalation_factor.max(1) as u64;
        let cap = self.config.max_block_duration_secs.max(base);
        let mut dur = base;
        for _ in 1..strikes.min(MAX_STRIKES) {
            dur = dur.saturating_mul(factor).min(cap);
            if dur >= cap {
                break;
            }
        }
        dur
    }

    /// Current strike count for an IP (used by tests to observe the adaptive
    /// escalation bookkeeping).
    #[cfg(test)]
    fn strikes_for(&self, ip: IpAddr) -> u32 {
        self.strikes.lock().unwrap().get(&ip).map_or(0, |r| r.count)
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

    #[test]
    fn min_body_rate_bps_default_is_nonzero() {
        let cfg = FirewallConfig::default();
        assert!(
            cfg.min_body_rate_bps > 0,
            "default must enforce a minimum rate"
        );
    }

    #[test]
    fn min_body_rate_bps_zero_disables_check() {
        let cfg = FirewallConfig {
            min_body_rate_bps: 0,
            ..FirewallConfig::default()
        };
        assert_eq!(cfg.min_body_rate_bps, 0);
    }

    #[test]
    fn min_body_rate_bps_custom_value_preserved() {
        let cfg = FirewallConfig {
            min_body_rate_bps: 512,
            ..FirewallConfig::default()
        };
        assert_eq!(cfg.min_body_rate_bps, 512);
        // Verify it clones correctly (used when passing config to Firewall::new).
        let cloned = cfg.clone();
        assert_eq!(cloned.min_body_rate_bps, 512);
    }

    // ── Adaptive escalation ──────────────────────────────────────────────────

    #[test]
    fn adaptive_block_duration_escalates_and_caps() {
        let fw = Firewall::new(FirewallConfig {
            block_duration_secs: 10,
            block_escalation_factor: 2,
            max_block_duration_secs: 80,
            ..FirewallConfig::default()
        });
        // First offense → base duration; each repeat offense doubles until capped.
        assert_eq!(fw.block_duration_secs_for(1), 10);
        assert_eq!(fw.block_duration_secs_for(2), 20);
        assert_eq!(fw.block_duration_secs_for(3), 40);
        assert_eq!(fw.block_duration_secs_for(4), 80);
        assert_eq!(
            fw.block_duration_secs_for(16),
            80,
            "must not exceed the cap"
        );
    }

    #[test]
    fn adaptive_escalation_factor_one_disables_escalation() {
        let fw = Firewall::new(FirewallConfig {
            block_duration_secs: 30,
            block_escalation_factor: 1,
            max_block_duration_secs: 3600,
            ..FirewallConfig::default()
        });
        for strikes in 1..=16 {
            assert_eq!(fw.block_duration_secs_for(strikes), 30);
        }
    }

    #[test]
    fn adaptive_max_below_base_never_shortens_first_block() {
        let fw = Firewall::new(FirewallConfig {
            block_duration_secs: 60,
            block_escalation_factor: 2,
            max_block_duration_secs: 30, // misconfigured below the base
            ..FirewallConfig::default()
        });
        assert_eq!(fw.block_duration_secs_for(1), 60);
        assert_eq!(fw.block_duration_secs_for(10), 60, "cap is clamped to base");
    }

    #[test]
    fn adaptive_auto_block_escalates_across_offenses() {
        let fw = Firewall::new(FirewallConfig {
            rate_limit_rps: 1,
            rate_limit_burst: 1,
            auto_block_threshold: 1,
            block_duration_secs: 10,
            block_escalation_factor: 3,
            max_block_duration_secs: 90,
            ..FirewallConfig::default()
        });
        let a = ip(70);
        fw.check_request(a); // allowed (burst token)

        // First offense → strike 1 → 10 s block.
        match fw.check_request(a) {
            FirewallDecision::Blocked { remaining_ms, .. } => {
                assert_eq!(
                    remaining_ms, 10_000,
                    "first block must use the base duration"
                );
            }
            other => panic!("expected Blocked, got {:?}", other),
        }
        assert_eq!(fw.strikes_for(a), 1);

        // Let the block expire (simulated via unblock) and re-offend: the
        // strike history persists, so the next block escalates to 30 s.
        fw.unblock_ip(a);
        match fw.check_request(a) {
            FirewallDecision::Blocked { remaining_ms, .. } => {
                assert_eq!(remaining_ms, 30_000, "repeat offense must escalate");
            }
            other => panic!("expected Blocked, got {:?}", other),
        }
        assert_eq!(fw.strikes_for(a), 2);
    }

    #[test]
    fn adaptive_strikes_persist_within_decay_window() {
        let fw = Firewall::new(FirewallConfig {
            rate_limit_rps: 1,
            rate_limit_burst: 1,
            auto_block_threshold: 1,
            block_duration_secs: 10,
            strike_decay_secs: 3600,
            ..FirewallConfig::default()
        });
        let a = ip(72);
        fw.check_request(a);
        fw.check_request(a); // auto-block → strike 1
        assert_eq!(fw.strikes_for(a), 1);

        // cleanup must NOT clear the strike while the decay window is open.
        fw.cleanup();
        assert_eq!(fw.strikes_for(a), 1);
    }

    #[test]
    fn adaptive_strikes_clear_after_decay_window() {
        let fw = Firewall::new(FirewallConfig {
            rate_limit_rps: 1,
            rate_limit_burst: 1,
            auto_block_threshold: 1,
            block_duration_secs: 10,
            strike_decay_secs: 0, // expire immediately on the next cleanup
            ..FirewallConfig::default()
        });
        let a = ip(73);
        fw.check_request(a);
        fw.check_request(a); // auto-block → strike 1
        assert_eq!(fw.strikes_for(a), 1);

        fw.cleanup();
        assert_eq!(
            fw.strikes_for(a),
            0,
            "calm IP must reset to the base duration"
        );
    }

    #[test]
    fn ewma_update_tracks_samples_with_configurable_smoothing() {
        // α=100% → pure passthrough of the newest sample.
        assert_eq!(ewma_update(1.0, 5.0, 100), 5.0);
        // α=0% → frozen at the previous value.
        assert_eq!(ewma_update(1.0, 5.0, 0), 1.0);
        // α=50% → halfway.
        assert!((ewma_update(2.0, 4.0, 50) - 3.0).abs() < 1e-9);
        // Repeated samples converge toward the sample value.
        let mut e = 1.0;
        for _ in 0..20 {
            e = ewma_update(e, 8.0, 30);
        }
        assert!((e - 8.0).abs() < 0.01, "must converge to sample, got {e}");
    }

    #[test]
    fn effective_rps_rises_with_baseline_and_stays_capped() {
        // Adaptive off → always static, regardless of baseline.
        assert_eq!(compute_effective_rps(10, Some(50.0), false), 10.0);
        // Adaptive on, no data yet → static.
        assert_eq!(compute_effective_rps(10, None, true), 10.0);
        // Baseline below static → static (never lowers).
        assert_eq!(compute_effective_rps(10, Some(3.0), true), 10.0);
        // Baseline above static → raised with headroom: ceil(12 × 1.5) = 18.
        assert_eq!(compute_effective_rps(10, Some(12.0), true), 18.0);
        // Hard cap: never more than static × ADAPTIVE_MAX_RATE_MULTIPLIER.
        assert_eq!(compute_effective_rps(10, Some(1000.0), true), 40.0);
    }

    #[test]
    fn growing_legitimate_traffic_raises_limit_without_violations() {
        use std::net::Ipv4Addr;
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let cfg = FirewallConfig {
            rate_limit_rps: 4,
            rate_limit_burst: 500,
            adaptive_rate_limit: true,
            ewma_alpha_pct: 60,
            auto_block_threshold: u32::MAX,
            ..Default::default()
        };
        let fw = Firewall::new(cfg);

        let mut buckets = fw.buckets.lock().unwrap();
        let bucket = buckets
            .entry(ip)
            .or_insert_with(|| TokenBucket::new(4, 500));
        // Simulate five closed observation windows of steadily growing
        // legitimate traffic: 7, 9, 12, 16, 21 requests per window. Each
        // note_request_at lands in the next second so windows roll.
        let counts = [7u32, 9, 12, 16, 21];
        let t0 = Instant::now();
        bucket.note_request_at(t0, 60);
        for (w, n) in counts.iter().enumerate() {
            for i in 0..*n {
                bucket.note_request_at(
                    t0 + Duration::from_millis((w as u64) * 1000 + 1 + i as u64),
                    60,
                );
            }
        }
        assert_eq!(
            bucket.violations, 0,
            "no violations may accrue while adapting"
        );
        let ewma = bucket.ewma_per_window.expect("windows must have rolled");
        assert!(
            ewma > 4.0,
            "baseline must exceed the static rps, got {ewma}"
        );
        let effective = fw.effective_bucket_rate(bucket);
        assert!(
            effective > 4.0 && effective <= 4.0 * ADAPTIVE_MAX_RATE_MULTIPLIER,
            "adaptive limit must be raised but capped, got {effective}"
        );
    }

    #[test]
    fn proxy_rule_parses_ips_and_cidrs() {
        let r = ProxyRule::parse("10.0.0.0/8").unwrap();
        assert!(r.matches("10.1.2.3".parse().unwrap()));
        assert!(r.matches("10.255.255.255".parse().unwrap()));
        assert!(!r.matches("11.0.0.1".parse().unwrap()));
        // Bare IP = /32 exact match.
        let r = ProxyRule::parse("127.0.0.1").unwrap();
        assert!(r.matches("127.0.0.1".parse().unwrap()));
        assert!(!r.matches("127.0.0.2".parse().unwrap()));
        // IPv6.
        let r = ProxyRule::parse("fd00::/8").unwrap();
        assert!(r.matches("fd12::1".parse().unwrap()));
        assert!(!r.matches("fe80::1".parse().unwrap()));
        // Family mismatch never matches.
        let r = ProxyRule::parse("10.0.0.0/8").unwrap();
        assert!(!r.matches("fd00::1".parse().unwrap()));
        // Junk is rejected, oversized prefix rejected.
        assert!(ProxyRule::parse("not-an-ip").is_none());
        assert!(ProxyRule::parse("10.0.0.0/33").is_none());
    }

    #[test]
    fn xff_ignored_when_peer_not_trusted() {
        let trusted: Vec<ProxyRule> = ["10.0.0.0/8"]
            .iter()
            .filter_map(|s| ProxyRule::parse(s))
            .collect();
        // Peer is the client itself (not in 10/8) → header must be ignored.
        assert_eq!(
            resolve_forwarded_for("203.0.113.7".parse().unwrap(), "9.9.9.9", &trusted),
            None
        );
    }

    #[test]
    fn xff_resolves_client_behind_trusted_proxy() {
        let trusted: Vec<ProxyRule> = ["127.0.0.1", "10.0.0.0/8"]
            .iter()
            .filter_map(|s| ProxyRule::parse(s))
            .collect();
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        // Simple chain: proxy saw one client.
        assert_eq!(
            resolve_forwarded_for(peer, "198.51.100.23", &trusted),
            Some("198.51.100.23".parse().unwrap())
        );
        // Chained proxies (both trusted): rightmost non-trusted wins.
        assert_eq!(
            resolve_forwarded_for(peer, "198.51.100.23, 10.1.2.3", &trusted),
            Some("198.51.100.23".parse().unwrap())
        );
        // All entries trusted → leftmost (deepest hop) wins.
        assert_eq!(
            resolve_forwarded_for(peer, "198.51.100.23, 10.1.2.3", &trusted),
            Some("198.51.100.23".parse().unwrap())
        );
        assert_eq!(
            resolve_forwarded_for(peer, "10.9.9.9, 10.1.2.3", &trusted),
            Some("10.9.9.9".parse().unwrap())
        );
    }

    #[test]
    fn xff_malformed_entry_falls_back_to_leftmost() {
        let trusted: Vec<ProxyRule> = ["127.0.0.1"]
            .iter()
            .filter_map(|s| ProxyRule::parse(s))
            .collect();
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        // Rightmost entry garbage → walk stops there, leftmost valid entry
        // (the deepest trusted proxy's view) is used.
        assert_eq!(
            resolve_forwarded_for(peer, "198.51.100.5, junk", &trusted),
            Some("198.51.100.5".parse().unwrap())
        );
        // Nothing parseable at all → None.
        assert_eq!(resolve_forwarded_for(peer, "junk", &trusted), None);
    }

    #[test]
    fn firewall_client_ip_uses_xff_only_through_trusted_proxies() {
        use std::net::Ipv4Addr;
        let fw = Firewall::new(FirewallConfig {
            trusted_proxies: vec!["127.0.0.1".to_string()],
            ..Default::default()
        });
        let peer = IpAddr::V4(Ipv4Addr::LOCALHOST);
        // Trusted peer with header → forwarded address.
        assert_eq!(
            fw.client_ip_from_xff(peer, Some("203.0.113.9")),
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))
        );
        // Trusted peer without header → peer itself.
        assert_eq!(fw.client_ip_from_xff(peer, None), peer);
        // Untrusted direct client sending a spoofed header → ignored.
        let outsider = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9));
        assert_eq!(
            fw.client_ip_from_xff(outsider, Some("9.9.9.9")),
            outsider,
            "spoofed XFF from an untrusted client must be ignored"
        );
    }
}
