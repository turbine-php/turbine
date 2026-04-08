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
}
