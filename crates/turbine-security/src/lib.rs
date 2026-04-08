//! OWASP Top 10 security guards — in-process, zero-network-overhead.
//!
//! All guards run inside the process with < 2μs total overhead per request.
//! No external WAF needed.

mod error;
mod sql_guard;
mod code_guard;
mod behaviour_guard;

pub use error::SecurityError;
pub use sql_guard::SqlGuard;
pub use code_guard::CodeGuard;
pub use behaviour_guard::{BehaviourGuard, BehaviourConfig};

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
        }

        // 1. Behaviour guard — rate limit + scanning detection (cheapest)
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

    /// Record a completed request (for behaviour tracking).
    pub fn record_request(&self, ip: IpAddr, was_error: bool) {
        self.behaviour_guard.record_request(ip, was_error);
    }
}

impl Default for SecurityLayer {
    fn default() -> Self {
        Self::new()
    }
}
