//! Behaviour Guard — per-IP rate limiting, scanning detection, and SQLi accumulation.
//!
//! Maintains lock-free per-IP profiles using DashMap. All operations are O(1).
//! Overhead: ~80ns per check.
//!
//! Detection strategies:
//! - Rate limiting: > N requests/second from a single IP
//! - Scanning detection: error rate > 50% over 20+ requests
//! - SQLi accumulation: block IP after M SQLi attempts

use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tracing::warn;

use crate::Verdict;

/// Configuration for the behaviour guard.
#[derive(Debug, Clone)]
pub struct BehaviourConfig {
    /// Maximum requests per second from a single IP.
    pub max_rps: u32,
    /// Block IP after this many SQLi attempts.
    pub sqli_block_threshold: u32,
    /// Error rate threshold (0.0-1.0) for scanning detection.
    pub scanning_error_rate: f64,
    /// Minimum requests before scanning detection activates.
    pub scanning_min_requests: u32,
    /// How long to remember IP state (seconds).
    pub window_seconds: u64,
}

impl Default for BehaviourConfig {
    fn default() -> Self {
        BehaviourConfig {
            max_rps: 100,
            sqli_block_threshold: 3,
            scanning_error_rate: 0.5,
            scanning_min_requests: 20,
            window_seconds: 60,
        }
    }
}

/// Per-IP profile tracking request patterns.
struct IpProfile {
    /// Requests in the current window.
    request_count: u32,
    /// Error responses in the current window.
    error_count: u32,
    /// SQLi attempts.
    sqli_attempts: u32,
    /// Window start time.
    window_start: Instant,
    /// Whether this IP is currently blocked.
    blocked: bool,
    /// When the block expires (if blocked).
    block_until: Option<Instant>,
}

impl IpProfile {
    fn new() -> Self {
        IpProfile {
            request_count: 0,
            error_count: 0,
            sqli_attempts: 0,
            window_start: Instant::now(),
            blocked: false,
            block_until: None,
        }
    }

    /// Reset the window if it has expired.
    fn maybe_reset_window(&mut self, window_duration: Duration) {
        if self.window_start.elapsed() > window_duration {
            self.request_count = 0;
            self.error_count = 0;
            self.window_start = Instant::now();

            // Unblock if the block has expired
            if let Some(until) = self.block_until {
                if Instant::now() > until {
                    self.blocked = false;
                    self.block_until = None;
                    self.sqli_attempts = 0;
                }
            }
        }
    }
}

pub struct BehaviourGuard {
    config: BehaviourConfig,
    profiles: DashMap<IpAddr, IpProfile>,
    total_blocked: AtomicU64,
}

impl BehaviourGuard {
    pub fn new() -> Self {
        Self::with_config(BehaviourConfig::default())
    }

    pub fn with_config(config: BehaviourConfig) -> Self {
        BehaviourGuard {
            config,
            profiles: DashMap::with_capacity(256),
            total_blocked: AtomicU64::new(0),
        }
    }

    /// Check if a request from this IP should be allowed.
    pub fn check_request(&self, ip: IpAddr) -> Verdict {
        let window = Duration::from_secs(self.config.window_seconds);

        let mut profile = self.profiles.entry(ip).or_insert_with(IpProfile::new);
        profile.maybe_reset_window(window);

        // Check if IP is blocked
        if profile.blocked {
            if let Some(until) = profile.block_until {
                if Instant::now() < until {
                    self.total_blocked.fetch_add(1, Ordering::Relaxed);
                    return Verdict::Block(format!("IP {ip} is temporarily blocked"));
                }
                // Block expired
                profile.blocked = false;
                profile.block_until = None;
            }
        }

        // Rate limiting — only check after a minimum burst
        profile.request_count += 1;
        if profile.request_count > 10 {
            let elapsed = profile.window_start.elapsed().as_secs_f64().max(0.001);
            let rps = profile.request_count as f64 / elapsed;

            if rps > self.config.max_rps as f64 {
                self.total_blocked.fetch_add(1, Ordering::Relaxed);
                return Verdict::Block(format!(
                    "Rate limit exceeded: {:.0} req/s (max {})",
                    rps, self.config.max_rps
                ));
            }
        }

        // Scanning detection
        if profile.request_count >= self.config.scanning_min_requests {
            let error_rate = profile.error_count as f64 / profile.request_count as f64;
            if error_rate > self.config.scanning_error_rate {
                warn!(
                    ip = %ip,
                    error_rate = error_rate,
                    requests = profile.request_count,
                    "Scanning behaviour detected"
                );
                profile.blocked = true;
                profile.block_until = Some(Instant::now() + Duration::from_secs(300));
                self.total_blocked.fetch_add(1, Ordering::Relaxed);
                return Verdict::Block(format!(
                    "Scanning detected: {:.0}% error rate over {} requests",
                    error_rate * 100.0,
                    profile.request_count
                ));
            }
        }

        Verdict::Allow
    }

    /// Record a completed request result for an IP.
    pub fn record_request(&self, ip: IpAddr, was_error: bool) {
        if let Some(mut profile) = self.profiles.get_mut(&ip) {
            if was_error {
                profile.error_count += 1;
            }
        }
    }

    /// Record a SQLi attempt from an IP. Blocks after threshold.
    pub fn record_sqli_attempt(&self, ip: IpAddr) {
        let mut profile = self.profiles.entry(ip).or_insert_with(IpProfile::new);
        profile.sqli_attempts += 1;

        if profile.sqli_attempts >= self.config.sqli_block_threshold {
            warn!(
                ip = %ip,
                attempts = profile.sqli_attempts,
                "Blocking IP after repeated SQLi attempts"
            );
            profile.blocked = true;
            // Block for 10 minutes
            profile.block_until = Some(Instant::now() + Duration::from_secs(600));
        }
    }

    /// Number of tracked IPs.
    pub fn tracked_ips(&self) -> usize {
        self.profiles.len()
    }

    /// Total blocked requests since startup.
    pub fn total_blocked(&self) -> u64 {
        self.total_blocked.load(Ordering::Relaxed)
    }

    /// Manually unblock an IP address, clearing its block state and SQLi counter.
    /// Returns `true` if the IP was found and unblocked, `false` if it was not tracked.
    pub fn unblock_ip(&self, ip: IpAddr) -> bool {
        if let Some(mut profile) = self.profiles.get_mut(&ip) {
            profile.blocked = false;
            profile.block_until = None;
            profile.sqli_attempts = 0;
            true
        } else {
            false
        }
    }

    /// Returns the list of currently blocked IPs with their block expiry time as
    /// seconds from now (None = block has no expiry / already expired).
    pub fn blocked_ips(&self) -> Vec<(IpAddr, Option<u64>)> {
        let now = Instant::now();
        self.profiles
            .iter()
            .filter_map(|entry| {
                let profile = entry.value();
                if !profile.blocked {
                    return None;
                }
                let secs_remaining = profile.block_until.and_then(|until| {
                    if until > now {
                        Some(until.duration_since(now).as_secs())
                    } else {
                        None
                    }
                });
                Some((*entry.key(), secs_remaining))
            })
            .collect()
    }
}

impl Default for BehaviourGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn localhost() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    fn other_ip() -> IpAddr {
        "10.0.0.1".parse().unwrap()
    }

    #[test]
    fn allows_normal_request() {
        let guard = BehaviourGuard::new();
        assert_eq!(guard.check_request(localhost()), Verdict::Allow);
    }

    #[test]
    fn rate_limits_excessive_requests() {
        let config = BehaviourConfig {
            max_rps: 5,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);

        // First 5 should be allowed (within rate)
        for _ in 0..5 {
            guard.check_request(localhost());
        }

        // At this point we're well over 5 rps (all in < 1ms)
        // Keep going until we hit the limit
        let mut blocked = false;
        for _ in 0..100 {
            if guard.check_request(localhost()).is_blocked() {
                blocked = true;
                break;
            }
        }
        assert!(blocked, "Should have been rate limited");
    }

    #[test]
    fn different_ips_tracked_separately() {
        let guard = BehaviourGuard::new();
        guard.check_request(localhost());
        guard.check_request(other_ip());
        assert_eq!(guard.tracked_ips(), 2);
    }

    #[test]
    fn sqli_accumulation_blocks_ip() {
        let config = BehaviourConfig {
            sqli_block_threshold: 2,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);

        // First request allowed
        assert_eq!(guard.check_request(localhost()), Verdict::Allow);

        // Record 2 SQLi attempts
        guard.record_sqli_attempt(localhost());
        guard.record_sqli_attempt(localhost());

        // Next request should be blocked
        assert!(guard.check_request(localhost()).is_blocked());
    }

    #[test]
    fn scanning_detection() {
        let config = BehaviourConfig {
            scanning_min_requests: 5,
            scanning_error_rate: 0.5,
            max_rps: 10000, // Don't hit rate limit
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);

        let ip = localhost();
        // Send 5 requests, 4 of which are errors (80% error rate)
        for _ in 0..5 {
            guard.check_request(ip);
        }
        for _ in 0..4 {
            guard.record_request(ip, true);
        }
        guard.record_request(ip, false);

        // Next request should detect scanning
        let v = guard.check_request(ip);
        assert!(v.is_blocked());
        assert!(v.reason().unwrap().contains("Scanning"));
    }

    #[test]
    fn total_blocked_counter() {
        let config = BehaviourConfig {
            sqli_block_threshold: 1,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);

        guard.check_request(localhost());
        guard.record_sqli_attempt(localhost());

        // These should all be blocked
        guard.check_request(localhost());
        guard.check_request(localhost());

        assert!(guard.total_blocked() >= 2);
    }

    // ─── Additional behaviour guard coverage ─────────────────────────────────

    #[test]
    fn fresh_guard_has_zero_total_blocked() {
        let guard = BehaviourGuard::new();
        assert_eq!(guard.total_blocked(), 0);
    }

    #[test]
    fn fresh_guard_tracks_zero_ips() {
        let guard = BehaviourGuard::new();
        assert_eq!(guard.tracked_ips(), 0);
    }

    #[test]
    fn first_request_is_always_allowed() {
        let guard = BehaviourGuard::new();
        let v = guard.check_request("192.168.1.1".parse().unwrap());
        assert_eq!(v, Verdict::Allow);
    }

    #[test]
    fn record_request_increments_error_count_toward_scanning() {
        let config = BehaviourConfig {
            scanning_min_requests: 4,
            scanning_error_rate: 0.5,
            max_rps: 100_000,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = "10.10.10.1".parse::<IpAddr>().unwrap();

        for _ in 0..4 {
            guard.check_request(ip);
        }
        // All 4 are errors → 100 % error rate which exceeds 0.5
        for _ in 0..4 {
            guard.record_request(ip, true);
        }
        // The next check_request should evaluate scanning
        let v = guard.check_request(ip);
        assert!(
            v.is_blocked(),
            "100% error rate should trigger scanning block"
        );
        assert!(v.reason().unwrap().contains("Scanning"));
    }

    #[test]
    fn scanning_block_raises_total_blocked_counter() {
        let config = BehaviourConfig {
            scanning_min_requests: 4,
            scanning_error_rate: 0.5,
            max_rps: 100_000,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = "10.20.30.40".parse::<IpAddr>().unwrap();

        for _ in 0..4 {
            guard.check_request(ip);
        }
        for _ in 0..4 {
            guard.record_request(ip, true);
        }
        guard.check_request(ip); // triggers scanning block
        assert!(guard.total_blocked() >= 1);
    }

    #[test]
    fn sqli_single_attempt_below_threshold_does_not_block() {
        let config = BehaviourConfig {
            sqli_block_threshold: 3,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = localhost();

        guard.check_request(ip);
        guard.record_sqli_attempt(ip); // only 1, threshold is 3
        assert_eq!(guard.check_request(ip), Verdict::Allow);
    }

    #[test]
    fn sqli_at_threshold_blocks_immediately() {
        let config = BehaviourConfig {
            sqli_block_threshold: 3,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = localhost();

        guard.check_request(ip);
        guard.record_sqli_attempt(ip);
        guard.record_sqli_attempt(ip);
        guard.record_sqli_attempt(ip); // reaches threshold

        assert!(guard.check_request(ip).is_blocked());
    }

    #[test]
    fn sqli_block_reason_mentions_ip() {
        let config = BehaviourConfig {
            sqli_block_threshold: 1,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip: IpAddr = "172.16.0.5".parse().unwrap();

        guard.check_request(ip);
        guard.record_sqli_attempt(ip);

        let v = guard.check_request(ip);
        assert!(v.is_blocked());
        let reason = v.reason().unwrap();
        assert!(
            reason.contains("172.16.0.5"),
            "Reason should include the IP, got: {reason}"
        );
    }

    #[test]
    fn multiple_ips_sqli_blocked_independently() {
        let config = BehaviourConfig {
            sqli_block_threshold: 1,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let bad: IpAddr = "1.2.3.4".parse().unwrap();
        let good: IpAddr = "5.6.7.8".parse().unwrap();

        guard.check_request(bad);
        guard.record_sqli_attempt(bad);
        guard.check_request(good);

        assert!(
            guard.check_request(bad).is_blocked(),
            "bad IP should be blocked"
        );
        assert_eq!(
            guard.check_request(good),
            Verdict::Allow,
            "good IP must stay allowed"
        );
        assert_eq!(guard.tracked_ips(), 2);
    }

    #[test]
    fn rate_limit_block_reason_contains_rps() {
        let config = BehaviourConfig {
            max_rps: 2,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = localhost();

        let mut blocked_reason = None;
        for _ in 0..200 {
            let v = guard.check_request(ip);
            if v.is_blocked() {
                blocked_reason = Some(v.reason().unwrap().to_owned());
                break;
            }
        }
        let reason = blocked_reason.expect("should have been rate-limited");
        assert!(
            reason.contains("Rate limit exceeded"),
            "Reason should say 'Rate limit exceeded', got: {reason}"
        );
    }

    #[test]
    fn default_config_uses_100_max_rps() {
        let cfg = BehaviourConfig::default();
        assert_eq!(cfg.max_rps, 100);
        assert_eq!(cfg.sqli_block_threshold, 3);
        assert_eq!(cfg.window_seconds, 60);
    }

    #[test]
    fn record_request_no_error_does_not_change_error_count() {
        let config = BehaviourConfig {
            scanning_min_requests: 2,
            scanning_error_rate: 0.4,
            max_rps: 100_000,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = "192.168.50.1".parse::<IpAddr>().unwrap();

        for _ in 0..2 {
            guard.check_request(ip);
        }
        // Record successful responses — should NOT trigger scanning
        for _ in 0..2 {
            guard.record_request(ip, false);
        }
        assert_eq!(guard.check_request(ip), Verdict::Allow);
    }

    #[test]
    fn unblock_ip_clears_block() {
        let config = BehaviourConfig {
            sqli_block_threshold: 1,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = localhost();

        guard.check_request(ip);
        guard.record_sqli_attempt(ip); // triggers block

        assert!(
            guard.check_request(ip).is_blocked(),
            "should be blocked before unblock"
        );

        let found = guard.unblock_ip(ip);
        assert!(found, "unblock_ip should return true for a known IP");
        assert_eq!(
            guard.check_request(ip),
            Verdict::Allow,
            "should be allowed after unblock"
        );
    }

    #[test]
    fn unblock_ip_returns_false_for_unknown_ip() {
        let guard = BehaviourGuard::new();
        let unknown: IpAddr = "9.9.9.9".parse().unwrap();
        assert!(!guard.unblock_ip(unknown));
    }

    #[test]
    fn blocked_ips_lists_blocked_ip() {
        let config = BehaviourConfig {
            sqli_block_threshold: 1,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = localhost();

        guard.check_request(ip);
        guard.record_sqli_attempt(ip);

        let blocked = guard.blocked_ips();
        let ips: Vec<IpAddr> = blocked.iter().map(|(ip, _)| *ip).collect();
        assert!(
            ips.contains(&ip),
            "blocked list should contain the blocked IP"
        );
    }

    #[test]
    fn blocked_ips_empty_after_unblock() {
        let config = BehaviourConfig {
            sqli_block_threshold: 1,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = localhost();

        guard.check_request(ip);
        guard.record_sqli_attempt(ip);
        guard.unblock_ip(ip);

        assert!(
            guard.blocked_ips().is_empty(),
            "blocked list should be empty after unblock"
        );
    }

    #[test]
    fn blocked_ips_expiry_seconds_present() {
        let config = BehaviourConfig {
            sqli_block_threshold: 1,
            ..Default::default()
        };
        let guard = BehaviourGuard::with_config(config);
        let ip = localhost();

        guard.check_request(ip);
        guard.record_sqli_attempt(ip);

        let blocked = guard.blocked_ips();
        let entry = blocked
            .iter()
            .find(|(i, _)| *i == ip)
            .expect("IP should be in list");
        assert!(
            entry.1.is_some(),
            "expiry seconds should be Some for a fresh block"
        );
        assert!(entry.1.unwrap() > 0, "expiry should be > 0 seconds");
    }
}
