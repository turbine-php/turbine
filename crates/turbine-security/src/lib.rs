//! OWASP Top 10 security guards — in-process, zero-network-overhead.
//!
//! All guards run inside the process with < 2μs total overhead per request.
//! No external WAF needed.

mod behaviour_guard;
mod code_guard;
mod error;
mod sql_guard;

pub use behaviour_guard::{BehaviourConfig, BehaviourGuard};
pub use code_guard::CodeGuard;
pub use error::SecurityError;
pub use sql_guard::SqlGuard;

use std::net::IpAddr;
use tracing::{debug, warn};

/// Result of a security check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Request is safe — proceed.
    Allow,
    /// Request is blocked — contains the reason.
    Block(String),
}

impl Verdict {
    pub fn is_blocked(&self) -> bool {
        matches!(self, Verdict::Block(_))
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Verdict::Block(r) => Some(r),
            Verdict::Allow => None,
        }
    }
}

/// Configuration for enabling/disabling individual guards.
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    pub enabled: bool,
    pub sql_guard: bool,
    pub code_injection_guard: bool,
    pub behaviour_guard: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sql_guard: true,
            code_injection_guard: true,
            behaviour_guard: true,
        }
    }
}

/// The unified security layer that orchestrates all guards.
///
/// Cheap checks run first (Aho-Corasick ~150ns), expensive ones only if needed.
pub struct SecurityLayer {
    pub sql_guard: SqlGuard,
    pub code_guard: CodeGuard,
    pub behaviour_guard: BehaviourGuard,
    config: SecurityConfig,
}

impl SecurityLayer {
    /// Create a new security layer with all guards enabled.
    pub fn new() -> Self {
        Self::with_config(SecurityConfig::default())
    }

    /// Create a security layer with the given configuration.
    pub fn with_config(config: SecurityConfig) -> Self {
        SecurityLayer {
            sql_guard: SqlGuard::new(),
            code_guard: CodeGuard::new(),
            behaviour_guard: BehaviourGuard::new(),
            config,
        }
    }

    /// Create a security layer with custom behaviour guard configuration.
    pub fn with_behaviour_config(config: SecurityConfig, behaviour: BehaviourConfig) -> Self {
        SecurityLayer {
            sql_guard: SqlGuard::new(),
            code_guard: CodeGuard::new(),
            behaviour_guard: BehaviourGuard::with_config(behaviour),
            config,
        }
    }

    /// Check an incoming request (input parameters, query strings, etc.).
    ///
    /// Returns `Verdict::Block` on the first guard that triggers.
    pub fn check_input(&self, ip: IpAddr, params: &[(&str, &str)]) -> Verdict {
        if !self.config.enabled {
            return Verdict::Allow;
        } // 1. Behaviour guard — rate limit + scanning detection (cheapest)
        if self.config.behaviour_guard {
            let bv = self.behaviour_guard.check_request(ip);
            if bv.is_blocked() {
                warn!(ip = %ip, reason = ?bv.reason(), "Blocked by behaviour guard");
                return bv;
            }
        }

        // 2. SQL injection on all parameter values
        if self.config.sql_guard {
            for (key, value) in params {
                let sv = self.sql_guard.check(value);
                if sv.is_blocked() {
                    self.behaviour_guard.record_sqli_attempt(ip);
                    warn!(ip = %ip, key = key, reason = ?sv.reason(), "SQL injection blocked");
                    return sv;
                }
            }
        }

        // 3. Code injection on all parameter values
        if self.config.code_injection_guard {
            for (key, value) in params {
                let cv = self.code_guard.check(value);
                if cv.is_blocked() {
                    warn!(ip = %ip, key = key, reason = ?cv.reason(), "Code injection blocked");
                    return cv;
                }
            }
        }

        debug!(ip = %ip, params = params.len(), "Input checks passed");
        Verdict::Allow
    }

    /// Returns `true` if `check_input` actually scans parameter values.
    /// When this is `false`, callers can skip the work of building a
    /// `Vec<(&str,&str)>` of params entirely (a hot-path allocation
    /// otherwise paid by every request).
    #[inline]
    pub fn needs_input_scan(&self) -> bool {
        self.config.enabled && (self.config.sql_guard || self.config.code_injection_guard)
    }

    /// Returns `true` if the behaviour guard is active.  Cheap per-IP check
    /// that does not depend on params.
    #[inline]
    pub fn needs_behaviour_check(&self) -> bool {
        self.config.enabled && self.config.behaviour_guard
    }

    /// Record a completed request (for behaviour tracking).
    pub fn record_request(&self, ip: IpAddr, was_error: bool) {
        self.behaviour_guard.record_request(ip, was_error);
    }

    /// Manually unblock an IP. Returns `true` if the IP was found and unblocked.
    pub fn unblock_ip(&self, ip: IpAddr) -> bool {
        self.behaviour_guard.unblock_ip(ip)
    }

    /// Returns the list of currently blocked IPs and their remaining block time in seconds.
    pub fn blocked_ips(&self) -> Vec<(IpAddr, Option<u64>)> {
        self.behaviour_guard.blocked_ips()
    }
}

impl Default for SecurityLayer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn localhost() -> IpAddr {
        "127.0.0.1".parse().unwrap()
    }

    fn other_ip() -> IpAddr {
        "10.0.0.1".parse().unwrap()
    }

    // ─── Verdict helpers ─────────────────────────────────────────────────────

    #[test]
    fn verdict_allow_is_not_blocked() {
        assert!(!Verdict::Allow.is_blocked());
        assert!(Verdict::Allow.reason().is_none());
    }

    #[test]
    fn verdict_block_is_blocked() {
        let v = Verdict::Block("test reason".into());
        assert!(v.is_blocked());
        assert_eq!(v.reason(), Some("test reason"));
    }

    #[test]
    fn verdict_equality() {
        assert_eq!(Verdict::Allow, Verdict::Allow);
        assert_eq!(Verdict::Block("x".into()), Verdict::Block("x".into()));
        assert_ne!(Verdict::Allow, Verdict::Block("x".into()));
    }

    // ─── SecurityLayer disabled ──────────────────────────────────────────────

    #[test]
    fn disabled_layer_allows_everything() {
        let layer = SecurityLayer::with_config(SecurityConfig {
            enabled: false,
            ..Default::default()
        });
        let params = &[("q", "1 UNION SELECT * FROM users")];
        assert_eq!(layer.check_input(localhost(), params), Verdict::Allow);
    }

    #[test]
    fn disabled_layer_allows_code_injection() {
        let layer = SecurityLayer::with_config(SecurityConfig {
            enabled: false,
            ..Default::default()
        });
        let params = &[("input", "eval(base64_decode('bWFsaWNpb3Vz'))")];
        assert_eq!(layer.check_input(localhost(), params), Verdict::Allow);
    }

    // ─── Individual guard toggles ────────────────────────────────────────────

    #[test]
    fn sql_guard_disabled_allows_injection() {
        let layer = SecurityLayer::with_config(SecurityConfig {
            enabled: true,
            sql_guard: false,
            code_injection_guard: false,
            behaviour_guard: false,
        });
        let params = &[("id", "1 UNION SELECT * FROM users")];
        assert_eq!(layer.check_input(localhost(), params), Verdict::Allow);
    }

    #[test]
    fn code_guard_disabled_allows_code_injection() {
        let layer = SecurityLayer::with_config(SecurityConfig {
            enabled: true,
            sql_guard: false,
            code_injection_guard: false,
            behaviour_guard: false,
        });
        let params = &[("cmd", "eval(base64_decode('bWFsaWNpb3Vz'))")];
        assert_eq!(layer.check_input(localhost(), params), Verdict::Allow);
    }

    #[test]
    fn sql_guard_enabled_blocks_injection() {
        let layer = SecurityLayer::with_config(SecurityConfig {
            enabled: true,
            sql_guard: true,
            code_injection_guard: false,
            behaviour_guard: false,
        });
        let params = &[("id", "1 UNION SELECT * FROM users")];
        assert!(layer.check_input(localhost(), params).is_blocked());
    }

    #[test]
    fn code_guard_enabled_blocks_code_injection() {
        let layer = SecurityLayer::with_config(SecurityConfig {
            enabled: true,
            sql_guard: false,
            code_injection_guard: true,
            behaviour_guard: false,
        });
        let params = &[("payload", "system('whoami')")];
        assert!(layer.check_input(localhost(), params).is_blocked());
    }

    // ─── Empty / trivial inputs ──────────────────────────────────────────────

    #[test]
    fn empty_params_returns_allow() {
        let layer = SecurityLayer::new();
        assert_eq!(layer.check_input(localhost(), &[]), Verdict::Allow);
    }

    #[test]
    fn safe_params_return_allow() {
        let layer = SecurityLayer::new();
        let params = &[
            ("name", "Jane Doe"),
            ("email", "jane@example.com"),
            ("age", "30"),
        ];
        assert_eq!(layer.check_input(localhost(), params), Verdict::Allow);
    }

    // ─── Multi-param scanning ────────────────────────────────────────────────

    #[test]
    fn sql_in_second_param_is_blocked() {
        let layer = SecurityLayer::with_config(SecurityConfig {
            enabled: true,
            sql_guard: true,
            code_injection_guard: false,
            behaviour_guard: false,
        });
        let params = &[
            ("name", "safe value"),
            ("id", "1 UNION SELECT * FROM users"),
        ];
        assert!(layer.check_input(localhost(), params).is_blocked());
    }

    #[test]
    fn code_in_third_param_is_blocked() {
        let layer = SecurityLayer::with_config(SecurityConfig {
            enabled: true,
            sql_guard: false,
            code_injection_guard: true,
            behaviour_guard: false,
        });
        let params = &[
            ("a", "normal"),
            ("b", "also normal"),
            ("c", "eval(base64_decode('bWFsaWNpb3Vz'))"),
        ];
        assert!(layer.check_input(localhost(), params).is_blocked());
    }

    // ─── Behaviour guard integration ─────────────────────────────────────────

    #[test]
    fn behaviour_guard_disabled_allows_burst() {
        let layer = SecurityLayer::with_behaviour_config(
            SecurityConfig {
                enabled: true,
                sql_guard: false,
                code_injection_guard: false,
                behaviour_guard: false,
            },
            BehaviourConfig {
                max_rps: 1,
                ..Default::default()
            },
        );
        // Fire 500 requests — should all be allowed because behaviour_guard is disabled
        for _ in 0..500 {
            assert_eq!(
                layer.check_input(localhost(), &[("v", "safe")]),
                Verdict::Allow
            );
        }
    }

    #[test]
    fn sql_injection_records_sqli_attempt_for_behaviour_guard() {
        // When SQL injection is detected, check_input calls record_sqli_attempt
        // so that reaching the SQLi threshold blocks the IP even on clean requests.
        let layer = SecurityLayer::with_behaviour_config(
            SecurityConfig {
                enabled: true,
                sql_guard: true,
                code_injection_guard: false,
                behaviour_guard: true,
            },
            BehaviourConfig {
                sqli_block_threshold: 2,
                max_rps: 100_000,
                ..Default::default()
            },
        );
        let ip = other_ip();
        let sqli = &[("id", "1 UNION SELECT 1,2,3")];
        let safe = &[("id", "42")];

        // Each SQL injection also calls record_sqli_attempt internally
        assert!(layer.check_input(ip, sqli).is_blocked()); // 1st SQLi attempt
        assert!(layer.check_input(ip, sqli).is_blocked()); // 2nd — reaches threshold

        // Now even a safe request from this IP is blocked by the behaviour guard
        assert!(
            layer.check_input(ip, safe).is_blocked(),
            "IP should be permanently blocked after 2 SQLi attempts"
        );
    }

    #[test]
    fn record_request_error_propagates_to_behaviour_guard() {
        let layer = SecurityLayer::with_behaviour_config(
            SecurityConfig {
                enabled: true,
                sql_guard: false,
                code_injection_guard: false,
                behaviour_guard: true,
            },
            BehaviourConfig {
                scanning_min_requests: 3,
                scanning_error_rate: 0.6,
                max_rps: 100_000,
                ..Default::default()
            },
        );
        let ip: IpAddr = "172.31.0.1".parse().unwrap();

        for _ in 0..3 {
            layer.check_input(ip, &[("x", "normal")]);
        }
        // Mark all 3 as errors → 100% error rate
        for _ in 0..3 {
            layer.record_request(ip, true);
        }
        // Next request should be flagged as scanning
        let v = layer.check_input(ip, &[("x", "normal")]);
        assert!(
            v.is_blocked(),
            "Scanning should be detected after high error rate"
        );
    }

    // ─── Guard ordering: Behaviour → SQL → Code ──────────────────────────────

    #[test]
    fn blocked_by_behaviour_guard_before_sql_check() {
        let layer = SecurityLayer::with_behaviour_config(
            SecurityConfig {
                enabled: true,
                sql_guard: true,
                code_injection_guard: true,
                behaviour_guard: true,
            },
            BehaviourConfig {
                sqli_block_threshold: 1,
                max_rps: 100_000,
                ..Default::default()
            },
        );
        let ip: IpAddr = "192.168.1.100".parse().unwrap();

        // Trigger behaviour block first
        layer.check_input(ip, &[("id", "1 UNION SELECT 1")]); // records SQLi
                                                              // IP is now blocked. Even with a SQL payload, behaviour guard fires first.
        let v = layer.check_input(ip, &[("id", "1 UNION SELECT 1")]);
        assert!(v.is_blocked());
        // Reason should mention "temporarily blocked" (behaviour guard), not SQL
        let reason = v.reason().unwrap();
        assert!(
            reason.contains("temporarily blocked") || reason.contains("Rate limit"),
            "Expected behaviour-guard reason, got: {reason}"
        );
    }

    // ─── SecurityConfig defaults ─────────────────────────────────────────────

    #[test]
    fn default_config_has_all_guards_enabled() {
        let cfg = SecurityConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.sql_guard);
        assert!(cfg.code_injection_guard);
        assert!(cfg.behaviour_guard);
    }
}
