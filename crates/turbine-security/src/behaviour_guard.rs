//! Behaviour Guard — per-IP rate limiting, scanning detection, and SQLi accumulation.
//!
//! Maintains lock-free per-IP profiles using DashMap.  All mutable state
//! inside each profile is stored as atomics so the hot path (`check_request`)
//! only holds a DashMap shard **read** lock — multiple threads serving the
//! same IP never serialise on each other.  Writes only happen when a new IP
//! is first seen or when we need to rebuild the map after an IP-rotation
//! flood (bounded by `max_profiles`).
//!
//! Detection strategies:
//! - Rate limiting: > N requests/second from a single IP
//! - Scanning detection: error rate > 50% over 20+ requests
//! - SQLi accumulation: block IP after M SQLi attempts

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
            // `0` = rate limiting disabled.  The previous default of 100
            // is impractical for any site served through a CDN / proxy /
            // NAT (all traffic appears to come from one IP) or for APIs
            // driven by a JavaScript SPA that legitimately fires tens of
            // requests per page load.  Operators who want rate limiting
            // should set an explicit number appropriate for their stack.
            max_rps: 0,
            sqli_block_threshold: 3,
            scanning_error_rate: 0.5,
            scanning_min_requests: 20,
            window_seconds: 60,
        }
    }
}

/// Per-IP profile tracking request patterns.
///
/// Every field is atomic so the DashMap shard lock only needs to be held
/// in read mode while we touch the profile — hot-path mutations happen
/// without serialising threads that share an IP.  `block_until_ns == 0`
/// is the canonical "no block deadline" sentinel (zero is unreachable
/// once `start` is set; the first increment is well after start-of-day).
struct IpProfile {
    request_count: AtomicU32,
    error_count: AtomicU32,
    sqli_attempts: AtomicU32,
    /// Window start timestamp, nanoseconds since `BehaviourGuard.start`.
    window_start_ns: AtomicU64,
    blocked: AtomicBool,
    /// Block deadline in ns since `start`; `0` means "no deadline set".
    block_until_ns: AtomicU64,
}

impl IpProfile {
    fn new(now_ns: u64) -> Self {
        IpProfile {
            request_count: AtomicU32::new(0),
            error_count: AtomicU32::new(0),
            sqli_attempts: AtomicU32::new(0),
            window_start_ns: AtomicU64::new(now_ns),
            blocked: AtomicBool::new(false),
            block_until_ns: AtomicU64::new(0),
        }
    }
}

pub struct BehaviourGuard {
    config: BehaviourConfig,
    profiles: DashMap<IpAddr, IpProfile>,
    total_blocked: AtomicU64,
    /// Soft cap on tracked IPs to bound memory under IP-rotation DoS.
    max_profiles: usize,
    /// Reference instant against which every `*_ns` field is measured.
    /// Using a monotonic offset lets us store deadlines in `AtomicU64`
    /// without needing a lock or `parking_lot::Mutex<Instant>`.
    start: Instant,
}

impl BehaviourGuard {
    pub fn new() -> Self {
        Self::with_config(BehaviourConfig::default())
    }

    pub fn with_config(config: BehaviourConfig) -> Self {
        // Use 8× CPU count as shard count (min 32) to reduce DashMap shard
        // contention when many distinct IPs hash to the same shard.
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let shards = (cpus * 8).next_power_of_two().max(32);
        BehaviourGuard {
            config,
            profiles: DashMap::with_capacity_and_shard_amount(256, shards),
            total_blocked: AtomicU64::new(0),
            max_profiles: 100_000,
            start: Instant::now(),
        }
    }

    /// Monotonic "now", expressed as nanoseconds elapsed since `self.start`.
    /// Saturating so a (practically impossible) backwards-going clock can't
    /// produce a nonsense deadline — it just clamps to 0.
    #[inline]
    fn now_ns(&self) -> u64 {
        Instant::now()
            .saturating_duration_since(self.start)
            .as_nanos() as u64
    }

    /// Evaluate rate limit + scanning + block state for a profile.
    ///
    /// Callers pass a borrowed `&IpProfile` obtained either from a DashMap
    /// read lock (`.get`) on the fast path or from a freshly-inserted
    /// `.entry()` write lock on the slow path.  All mutations happen via
    /// atomic ops so the shard lock can stay in read mode throughout.
    fn evaluate(&self, profile: &IpProfile, ip: IpAddr, now_ns: u64) -> Verdict {
        let window_ns = self.config.window_seconds.saturating_mul(1_000_000_000);

        // Step 1 — window rollover.  A single CAS installs the new window
        // start; losers skip the reset (another thread already did it).
        let win_start = profile.window_start_ns.load(Ordering::Acquire);
        if window_ns > 0
            && now_ns.saturating_sub(win_start) > window_ns
            && profile
                .window_start_ns
                .compare_exchange(win_start, now_ns, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        {
            profile.request_count.store(0, Ordering::Release);
            profile.error_count.store(0, Ordering::Release);
            let bu = profile.block_until_ns.load(Ordering::Acquire);
            if bu != 0 && bu <= now_ns {
                profile.blocked.store(false, Ordering::Release);
                profile.block_until_ns.store(0, Ordering::Release);
                profile.sqli_attempts.store(0, Ordering::Release);
            }
        }

        // Step 2 — still-blocked check (cheap fast fail).
        if profile.blocked.load(Ordering::Acquire) {
            let bu = profile.block_until_ns.load(Ordering::Acquire);
            if bu != 0 && now_ns < bu {
                self.total_blocked.fetch_add(1, Ordering::Relaxed);
                return Verdict::Block(format!("IP {ip} is temporarily blocked"));
            }
            // Expired block — clear and fall through.
            profile.blocked.store(false, Ordering::Release);
            profile.block_until_ns.store(0, Ordering::Release);
        }

        // Step 3 — increment request count and rate-limit check.  We use
        // `fetch_add` on the atomic so concurrent requests from the same
        // IP never serialise on a RefMut lock.  `max_rps == 0` disables
        // rate limiting entirely.
        let count = profile.request_count.fetch_add(1, Ordering::Relaxed) + 1;
        if self.config.max_rps > 0 && count > 10 {
            let win_start = profile.window_start_ns.load(Ordering::Acquire);
            // Floor at 1 ms to avoid a division-by-near-zero that would
            // explode the computed RPS when the first burst arrives inside
            // the same millisecond as the window start.
            let elapsed_ns = now_ns.saturating_sub(win_start).max(1_000_000);
            let elapsed_secs = elapsed_ns as f64 / 1_000_000_000.0;
            let rps = count as f64 / elapsed_secs;
            if rps > self.config.max_rps as f64 {
                self.total_blocked.fetch_add(1, Ordering::Relaxed);
                return Verdict::Block(format!(
                    "Rate limit exceeded: {:.0} req/s (max {})",
                    rps, self.config.max_rps
                ));
            }
        }

        // Step 4 — scanning detection (error rate > threshold).
        if count >= self.config.scanning_min_requests {
            let errors = profile.error_count.load(Ordering::Acquire);
            let error_rate = errors as f64 / count as f64;
            if error_rate > self.config.scanning_error_rate {
                warn!(
                    ip = %ip,
                    error_rate = error_rate,
                    requests = count,
                    "Scanning behaviour detected"
                );
                profile.blocked.store(true, Ordering::Release);
                profile.block_until_ns.store(
                    now_ns.saturating_add(300 * 1_000_000_000),
                    Ordering::Release,
                );
                self.total_blocked.fetch_add(1, Ordering::Relaxed);
                return Verdict::Block(format!(
                    "Scanning detected: {:.0}% error rate over {} requests",
                    error_rate * 100.0,
                    count
                ));
            }
        }

        Verdict::Allow
    }

    /// Check if a request from this IP should be allowed.
    pub fn check_request(&self, ip: IpAddr) -> Verdict {
        // DoS guard: if the tracked-IP map has grown past the soft cap and
        // this IP is unknown, don't insert a new profile (allow through).
        // Legitimate traffic is almost entirely repeat-IPs; rotating-IP
        // attacks would otherwise exhaust memory.
        if self.profiles.len() >= self.max_profiles && !self.profiles.contains_key(&ip) {
            return Verdict::Allow;
        }

        let now_ns = self.now_ns();

        // Fast path: IP already known — DashMap shard READ lock only.
        // `get` returns a Ref<'_> guard; multiple concurrent requests for
        // the same IP can take the shard in read mode simultaneously,
        // which is the whole point of this redesign.
        if let Some(profile) = self.profiles.get(&ip) {
            return self.evaluate(&profile, ip, now_ns);
        }

        // Slow path: first request from this IP — entry() takes a write
        // lock on the shard to insert.  `or_insert_with` only runs the
        // closure when we actually need to create a new profile, so races
        // where another thread inserted the same IP first degrade to a
        // plain read (no extra profile allocation).
        let entry = self
            .profiles
            .entry(ip)
            .or_insert_with(|| IpProfile::new(now_ns));
        self.evaluate(&entry, ip, now_ns)
    }

    /// Record a completed request result for an IP.
    pub fn record_request(&self, ip: IpAddr, was_error: bool) {
        if !was_error {
            return;
        }
        if let Some(profile) = self.profiles.get(&ip) {
            profile.error_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a SQLi attempt from an IP. Blocks after threshold.
    pub fn record_sqli_attempt(&self, ip: IpAddr) {
        if self.profiles.len() >= self.max_profiles && !self.profiles.contains_key(&ip) {
            return;
        }
        let now_ns = self.now_ns();
        let profile = self
            .profiles
            .entry(ip)
            .or_insert_with(|| IpProfile::new(now_ns));
        let attempts = profile.sqli_attempts.fetch_add(1, Ordering::Relaxed) + 1;

        if attempts >= self.config.sqli_block_threshold {
            warn!(
                ip = %ip,
                attempts = attempts,
                "Blocking IP after repeated SQLi attempts"
            );
            profile.blocked.store(true, Ordering::Release);
            // Block for 10 minutes.
            profile.block_until_ns.store(
                now_ns.saturating_add(600 * 1_000_000_000),
                Ordering::Release,
            );
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
        if let Some(profile) = self.profiles.get(&ip) {
            profile.blocked.store(false, Ordering::Release);
            profile.block_until_ns.store(0, Ordering::Release);
            profile.sqli_attempts.store(0, Ordering::Release);
            true
        } else {
            false
        }
    }

    /// Returns the list of currently blocked IPs with their block expiry time
    /// as seconds from now (None = no deadline / already expired).
    pub fn blocked_ips(&self) -> Vec<(IpAddr, Option<u64>)> {
        let now_ns = self.now_ns();
        self.profiles
            .iter()
            .filter_map(|entry| {
                let profile = entry.value();
                if !profile.blocked.load(Ordering::Acquire) {
                    return None;
                }
                let bu = profile.block_until_ns.load(Ordering::Acquire);
                let secs_remaining = if bu > now_ns {
                    Some(Duration::from_nanos(bu - now_ns).as_secs())
                } else {
                    None
                };
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
    fn default_config_disables_rate_limit() {
        let cfg = BehaviourConfig::default();
        // max_rps = 0 means rate limiting is off by default.  Operators
        // opt in with an explicit value — see the module docs.
        assert_eq!(cfg.max_rps, 0);
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
