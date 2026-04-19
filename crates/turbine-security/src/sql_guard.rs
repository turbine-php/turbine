//! SQL Injection Guard using Aho-Corasick multi-pattern matching.
//!
//! **Heuristic filter, not a WAF.**  Patterns here are cheap string matches
//! with case-insensitive compare — no URL decoding, no comment removal, no
//! MySQL versioned-comment handling.  Trivial obfuscations bypass them.
//! For real SQL injection protection use a WAF upstream (Caddy + coraza,
//! libmodsecurity + OWASP CRS, or a CDN WAF).
//!
//! Two-phase detection:
//! 1. Fast Aho-Corasick scan (~150ns) against known SQL injection patterns
//! 2. Results cached by xxHash of the input, with a size cap (evicts when
//!    `MAX_CACHE_ENTRIES` is reached to prevent unbounded growth under
//!    attacker-controlled unique payloads).
//!
//! Patterns are split into paranoia tiers.  PL1 (default) contains only
//! very high-signal attack strings.  PL2+ adds common patterns that
//! produce false positives on user-generated content (documentation,
//! comments, bug trackers, code snippet editors).

use aho_corasick::AhoCorasick;
use dashmap::DashMap;
use xxhash_rust::xxh3::xxh3_64;

use crate::Verdict;

/// Paranoia-level 1 patterns.  High confidence, low false-positive rate.
/// Includes classic injection syntax, blind-injection timing functions,
/// and filesystem-writing primitives — all of which have no legitimate
/// reason to appear verbatim in user input for the vast majority of
/// applications.
const SQL_PATTERNS_PL1: &[&str] = &[
    // Classic injection
    "union select",
    "union all select",
    "' or '1'='1",
    "' or 1=1",
    "\" or 1=1",
    "or 1=1--",
    "or 1=1#",
    "' or ''='",
    // Blind injection — timing attacks
    "sleep(",
    "benchmark(",
    "waitfor delay",
    "pg_sleep(",
    // Filesystem-writing primitives (high-severity)
    "load_file(",
    "into outfile",
    "into dumpfile",
    // Hex/concat obfuscation used almost exclusively in injection payloads
    "char(0x",
    "concat(0x",
    // Advanced attack functions rarely used in app code
    "extractvalue(",
    "updatexml(",
    "exp(~(",
];

/// Additional PL2 patterns.  These have higher false-positive rates on
/// user-generated content (bug trackers mentioning "drop table", forum
/// posts about SQL, CMS comments with stack traces) so they're opt-in.
const SQL_PATTERNS_PL2: &[&str] = &[
    "drop table",
    "drop database",
    "truncate table",
    "group_concat(",
    "/**/",
    "-- -",
    // Stacked queries — legitimate in very few places
    "; drop",
    "; delete",
    "; insert",
    "; update",
];

/// Additional PL3 patterns.  Very aggressive — will fire on any site with
/// technical documentation, Q&A threads, or admin SQL consoles.
const SQL_PATTERNS_PL3: &[&str] = &[
    "delete from",
    "insert into",
    "update set",
    "information_schema",
    "table_name",
    "column_name",
];

/// Maximum number of cached scan results before eviction.  The cache is a
/// simple hit accelerator for hot URLs — under attacker-controlled unique
/// payloads it must not grow without bound.  When the entry count reaches
/// this threshold we drop the whole cache, which costs a one-shot re-scan
/// of whatever is still live but keeps memory use bounded.
const MAX_CACHE_ENTRIES: usize = 8192;

pub struct SqlGuard {
    automaton: AhoCorasick,
    /// Cache: xxHash of input → was_safe (true = safe, false = blocked).
    cache: DashMap<u64, bool>,
}

impl SqlGuard {
    /// Build a guard with paranoia-level 1 patterns.
    pub fn new() -> Self {
        Self::with_paranoia(1)
    }

    /// Build a guard with patterns appropriate for the given paranoia
    /// level (`0` = no patterns loaded → every input is `Allow`,
    /// `1`/`2`/`3` = progressively larger pattern sets).  Levels `>3`
    /// clamp to 3.
    pub fn with_paranoia(level: u8) -> Self {
        let mut patterns: Vec<&str> = Vec::new();
        if level >= 1 {
            patterns.extend_from_slice(SQL_PATTERNS_PL1);
        }
        if level >= 2 {
            patterns.extend_from_slice(SQL_PATTERNS_PL2);
        }
        if level >= 3 {
            patterns.extend_from_slice(SQL_PATTERNS_PL3);
        }
        let automaton = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&patterns)
            .expect("SQL patterns are valid");
        SqlGuard {
            automaton,
            cache: DashMap::with_capacity(1024),
        }
    }

    /// Check a string for SQL injection patterns.
    ///
    /// Returns `Verdict::Block` if injection detected, `Verdict::Allow` otherwise.
    /// Results are cached by content hash (~50ns for cache hit) up to
    /// `MAX_CACHE_ENTRIES` entries; beyond that the cache is purged to
    /// bound memory use under payload-shuffling attacks.
    pub fn check(&self, input: &str) -> Verdict {
        if input.is_empty() {
            return Verdict::Allow;
        }

        // Check cache first.
        let hash = xxh3_64(input.as_bytes());
        if let Some(cached) = self.cache.get(&hash) {
            return if *cached {
                Verdict::Allow
            } else {
                Verdict::Block("SQL injection (cached)".into())
            };
        }

        // Bound cache growth before inserting a new entry.
        if self.cache.len() >= MAX_CACHE_ENTRIES {
            self.cache.clear();
        }

        // Run Aho-Corasick scan.
        if let Some(mat) = self.automaton.find(input) {
            // The matched pattern index is into the concatenated slice we
            // built at construction time, so we can't look it up in a
            // single static array — format from the matched text instead.
            let matched_text = &input[mat.start()..mat.end()];
            self.cache.insert(hash, false);
            Verdict::Block(format!("SQL injection pattern: {matched_text}"))
        } else {
            self.cache.insert(hash, true);
            Verdict::Allow
        }
    }

    /// Number of cached scan results.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    /// Clear the scan cache.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}

impl Default for SqlGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_normal_queries() {
        let guard = SqlGuard::with_paranoia(3);
        assert_eq!(guard.check("John Doe"), Verdict::Allow);
        assert_eq!(guard.check("hello@example.com"), Verdict::Allow);
        assert_eq!(guard.check("SELECT products"), Verdict::Allow);
        assert_eq!(guard.check("123"), Verdict::Allow);
        assert_eq!(guard.check(""), Verdict::Allow);
    }

    #[test]
    fn blocks_union_select() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 UNION SELECT * FROM users");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_or_1_equals_1() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("admin' OR 1=1--");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_drop_table() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1; DROP TABLE users;");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_sleep_injection() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 AND SLEEP(5)");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_case_insensitive() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 uNiOn SeLeCt * FROM users");
        assert!(v.is_blocked());
    }

    #[test]
    fn cache_hit_returns_same() {
        let guard = SqlGuard::with_paranoia(3);
        let input = "admin' OR 1=1--";
        let v1 = guard.check(input);
        let v2 = guard.check(input);
        assert!(v1.is_blocked());
        assert!(v2.is_blocked());
        assert_eq!(guard.cache_size(), 1);
    }

    #[test]
    fn blocks_information_schema() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 AND 1=1 UNION SELECT table_name FROM information_schema.tables");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_stacked_queries() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1; DELETE FROM sessions");
        assert!(v.is_blocked());
    }

    // ─── Additional pattern coverage ─────────────────────────────────────────

    #[test]
    fn blocks_load_file() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 AND LOAD_FILE('/etc/passwd')");
        assert!(v.is_blocked(), "load_file( should be blocked");
    }

    #[test]
    fn blocks_into_outfile() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 INTO OUTFILE '/var/www/shell.php'");
        assert!(v.is_blocked(), "into outfile should be blocked");
    }

    #[test]
    fn blocks_concat_hex() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 AND EXTRACTVALUE(1,CONCAT(0x7e,(SELECT version())))");
        assert!(v.is_blocked(), "concat(0x should be blocked");
    }

    #[test]
    fn blocks_extractvalue() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("EXTRACTVALUE(1,CONCAT(0x7e,version()))");
        assert!(v.is_blocked(), "extractvalue( should be blocked");
    }

    #[test]
    fn blocks_updatexml() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("UPDATEXML(1,CONCAT(0x7e,(SELECT user())),1)");
        assert!(v.is_blocked(), "updatexml( should be blocked");
    }

    #[test]
    fn blocks_waitfor_delay() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1'; WAITFOR DELAY '0:0:5'--");
        assert!(v.is_blocked(), "waitfor delay should be blocked (MSSQL)");
    }

    #[test]
    fn blocks_pg_sleep() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 AND PG_SLEEP(5)");
        assert!(v.is_blocked(), "pg_sleep( should be blocked");
    }

    #[test]
    fn blocks_benchmark() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 AND BENCHMARK(10000000,MD5('x'))");
        assert!(v.is_blocked(), "benchmark( should be blocked");
    }

    #[test]
    fn blocks_group_concat() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 AND GROUP_CONCAT(username,':',password) FROM users");
        assert!(v.is_blocked(), "group_concat( should be blocked");
    }

    #[test]
    fn blocks_char_hex() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("CHAR(0x41,0x42,0x43)");
        assert!(v.is_blocked(), "char(0x should be blocked");
    }

    #[test]
    fn blocks_comment_bypass() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1/**/UNION/**/SELECT/**/1,2,3");
        assert!(v.is_blocked(), "/**/ comment bypass should be blocked");
    }

    #[test]
    fn blocks_drop_database() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("'; DROP DATABASE production;--");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_truncate() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("'; TRUNCATE TABLE users;--");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_delete_from() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1; DELETE FROM users WHERE 1=1");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_insert_injection() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("', 'hacked'); INSERT INTO users VALUES ('x");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_column_name() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 UNION SELECT column_name FROM information_schema.columns");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_into_dumpfile() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 INTO DUMPFILE '/tmp/out.txt'");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_exp_blind() {
        let guard = SqlGuard::with_paranoia(3);
        // exp(~(SELECT*FROM(SELECT...)x)) — overflow-based blind injection
        let v = guard.check("1 AND EXP(~(SELECT * FROM users))");
        assert!(v.is_blocked(), "exp(~( should be blocked");
    }

    #[test]
    fn allows_normal_select_without_keywords() {
        let guard = SqlGuard::with_paranoia(3);
        // "select" alone is not blocked — only dangerous combinations
        assert_eq!(guard.check("products"), Verdict::Allow);
        assert_eq!(guard.check("user profile"), Verdict::Allow);
        assert_eq!(guard.check("orderby=price"), Verdict::Allow);
    }

    #[test]
    fn block_reason_contains_pattern_name() {
        let guard = SqlGuard::with_paranoia(3);
        let v = guard.check("1 UNION SELECT 1,2,3");
        assert!(v.is_blocked());
        let reason = v.reason().expect("blocked verdict must have reason");
        // Reason preserves the original casing of the matched substring.
        assert!(
            reason.to_lowercase().contains("union select"),
            "reason should name the matched pattern, got: {reason}"
        );
    }

    #[test]
    fn cached_safe_input_stays_safe() {
        let guard = SqlGuard::with_paranoia(3);
        let input = "safe plain text";
        assert_eq!(guard.check(input), Verdict::Allow);
        assert_eq!(guard.check(input), Verdict::Allow);
        assert_eq!(guard.cache_size(), 1);
    }

    #[test]
    fn cache_cleared_then_rescanned() {
        let guard = SqlGuard::with_paranoia(3);
        let input = "1 UNION SELECT 1";
        assert!(guard.check(input).is_blocked());
        assert_eq!(guard.cache_size(), 1);
        guard.clear_cache();
        assert_eq!(guard.cache_size(), 0);
        // Re-scan after clear must still block
        assert!(guard.check(input).is_blocked());
        assert_eq!(guard.cache_size(), 1);
    }
}
