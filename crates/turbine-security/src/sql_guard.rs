//! SQL Injection Guard using Aho-Corasick multi-pattern matching.
//!
//! Two-phase detection:
//! 1. Fast Aho-Corasick scan (~150ns) against known SQL injection patterns
//! 2. Results cached by xxHash of the input to avoid re-scanning identical queries
//!
//! The Aho-Corasick automaton matches all patterns in a single O(n) pass
//! regardless of the number of patterns.

use aho_corasick::AhoCorasick;
use dashmap::DashMap;
use xxhash_rust::xxh3::xxh3_64;

use crate::Verdict;

/// SQL injection patterns to detect.
///
/// These are normalised to lowercase for case-insensitive matching.
const SQL_PATTERNS: &[&str] = &[
    // Classic injection
    "union select",
    "union all select",
    "' or '1'='1",
    "' or 1=1",
    "\" or 1=1",
    "or 1=1--",
    "or 1=1#",
    "' or ''='",
    // Destructive
    "drop table",
    "drop database",
    "truncate table",
    "delete from",
    "insert into",
    "update set",
    // Comment-based
    "/**/",
    "-- -",
    // Information disclosure
    "information_schema",
    "table_name",
    "column_name",
    // Blind injection
    "sleep(",
    "benchmark(",
    "waitfor delay",
    "pg_sleep(",
    // Stacked queries
    "; drop",
    "; delete",
    "; insert",
    "; update",
    // Function-based
    "load_file(",
    "into outfile",
    "into dumpfile",
    "char(0x",
    "concat(0x",
    // Advanced
    "extractvalue(",
    "updatexml(",
    "exp(~(",
    "group_concat(",
];

pub struct SqlGuard {
    automaton: AhoCorasick,
    /// Cache: xxHash of input → was_safe (true = safe, false = blocked).
    cache: DashMap<u64, bool>,
}

impl SqlGuard {
    pub fn new() -> Self {
        let automaton = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(SQL_PATTERNS)
            .expect("SQL patterns are valid");

        SqlGuard {
            automaton,
            cache: DashMap::with_capacity(1024),
        }
    }

    /// Check a string for SQL injection patterns.
    ///
    /// Returns `Verdict::Block` if injection detected, `Verdict::Allow` otherwise.
    /// Results are cached by content hash (~50ns for cache hit).
    pub fn check(&self, input: &str) -> Verdict {
        if input.is_empty() {
            return Verdict::Allow;
        }

        // Check cache first
        let hash = xxh3_64(input.as_bytes());
        if let Some(cached) = self.cache.get(&hash) {
            return if *cached {
                Verdict::Allow
            } else {
                Verdict::Block("SQL injection (cached)".into())
            };
        }

        // Run Aho-Corasick scan
        let verdict = if let Some(mat) = self.automaton.find(input) {
            let pattern_idx = mat.pattern().as_usize();
            let pattern = SQL_PATTERNS.get(pattern_idx).unwrap_or(&"unknown");
            self.cache.insert(hash, false);
            Verdict::Block(format!("SQL injection pattern: {pattern}"))
        } else {
            self.cache.insert(hash, true);
            Verdict::Allow
        };

        verdict
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
        let guard = SqlGuard::new();
        assert_eq!(guard.check("John Doe"), Verdict::Allow);
        assert_eq!(guard.check("hello@example.com"), Verdict::Allow);
        assert_eq!(guard.check("SELECT products"), Verdict::Allow);
        assert_eq!(guard.check("123"), Verdict::Allow);
        assert_eq!(guard.check(""), Verdict::Allow);
    }

    #[test]
    fn blocks_union_select() {
        let guard = SqlGuard::new();
        let v = guard.check("1 UNION SELECT * FROM users");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_or_1_equals_1() {
        let guard = SqlGuard::new();
        let v = guard.check("admin' OR 1=1--");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_drop_table() {
        let guard = SqlGuard::new();
        let v = guard.check("1; DROP TABLE users;");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_sleep_injection() {
        let guard = SqlGuard::new();
        let v = guard.check("1 AND SLEEP(5)");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_case_insensitive() {
        let guard = SqlGuard::new();
        let v = guard.check("1 uNiOn SeLeCt * FROM users");
        assert!(v.is_blocked());
    }

    #[test]
    fn cache_hit_returns_same() {
        let guard = SqlGuard::new();
        let input = "admin' OR 1=1--";
        let v1 = guard.check(input);
        let v2 = guard.check(input);
        assert!(v1.is_blocked());
        assert!(v2.is_blocked());
        assert_eq!(guard.cache_size(), 1);
    }

    #[test]
    fn blocks_information_schema() {
        let guard = SqlGuard::new();
        let v = guard.check("1 AND 1=1 UNION SELECT table_name FROM information_schema.tables");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_stacked_queries() {
        let guard = SqlGuard::new();
        let v = guard.check("1; DELETE FROM sessions");
        assert!(v.is_blocked());
    }

    // ─── Additional pattern coverage ─────────────────────────────────────────

    #[test]
    fn blocks_load_file() {
        let guard = SqlGuard::new();
        let v = guard.check("1 AND LOAD_FILE('/etc/passwd')");
        assert!(v.is_blocked(), "load_file( should be blocked");
    }

    #[test]
    fn blocks_into_outfile() {
        let guard = SqlGuard::new();
        let v = guard.check("1 INTO OUTFILE '/var/www/shell.php'");
        assert!(v.is_blocked(), "into outfile should be blocked");
    }

    #[test]
    fn blocks_concat_hex() {
        let guard = SqlGuard::new();
        let v = guard.check("1 AND EXTRACTVALUE(1,CONCAT(0x7e,(SELECT version())))");
        assert!(v.is_blocked(), "concat(0x should be blocked");
    }

    #[test]
    fn blocks_extractvalue() {
        let guard = SqlGuard::new();
        let v = guard.check("EXTRACTVALUE(1,CONCAT(0x7e,version()))");
        assert!(v.is_blocked(), "extractvalue( should be blocked");
    }

    #[test]
    fn blocks_updatexml() {
        let guard = SqlGuard::new();
        let v = guard.check("UPDATEXML(1,CONCAT(0x7e,(SELECT user())),1)");
        assert!(v.is_blocked(), "updatexml( should be blocked");
    }

    #[test]
    fn blocks_waitfor_delay() {
        let guard = SqlGuard::new();
        let v = guard.check("1'; WAITFOR DELAY '0:0:5'--");
        assert!(v.is_blocked(), "waitfor delay should be blocked (MSSQL)");
    }

    #[test]
    fn blocks_pg_sleep() {
        let guard = SqlGuard::new();
        let v = guard.check("1 AND PG_SLEEP(5)");
        assert!(v.is_blocked(), "pg_sleep( should be blocked");
    }

    #[test]
    fn blocks_benchmark() {
        let guard = SqlGuard::new();
        let v = guard.check("1 AND BENCHMARK(10000000,MD5('x'))");
        assert!(v.is_blocked(), "benchmark( should be blocked");
    }

    #[test]
    fn blocks_group_concat() {
        let guard = SqlGuard::new();
        let v = guard.check("1 AND GROUP_CONCAT(username,':',password) FROM users");
        assert!(v.is_blocked(), "group_concat( should be blocked");
    }

    #[test]
    fn blocks_char_hex() {
        let guard = SqlGuard::new();
        let v = guard.check("CHAR(0x41,0x42,0x43)");
        assert!(v.is_blocked(), "char(0x should be blocked");
    }

    #[test]
    fn blocks_comment_bypass() {
        let guard = SqlGuard::new();
        let v = guard.check("1/**/UNION/**/SELECT/**/1,2,3");
        assert!(v.is_blocked(), "/**/ comment bypass should be blocked");
    }

    #[test]
    fn blocks_drop_database() {
        let guard = SqlGuard::new();
        let v = guard.check("'; DROP DATABASE production;--");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_truncate() {
        let guard = SqlGuard::new();
        let v = guard.check("'; TRUNCATE TABLE users;--");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_delete_from() {
        let guard = SqlGuard::new();
        let v = guard.check("1; DELETE FROM users WHERE 1=1");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_insert_injection() {
        let guard = SqlGuard::new();
        let v = guard.check("', 'hacked'); INSERT INTO users VALUES ('x");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_column_name() {
        let guard = SqlGuard::new();
        let v = guard.check("1 UNION SELECT column_name FROM information_schema.columns");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_into_dumpfile() {
        let guard = SqlGuard::new();
        let v = guard.check("1 INTO DUMPFILE '/tmp/out.txt'");
        assert!(v.is_blocked());
    }

    #[test]
    fn blocks_exp_blind() {
        let guard = SqlGuard::new();
        // exp(~(SELECT*FROM(SELECT...)x)) — overflow-based blind injection
        let v = guard.check("1 AND EXP(~(SELECT * FROM users))");
        assert!(v.is_blocked(), "exp(~( should be blocked");
    }

    #[test]
    fn allows_normal_select_without_keywords() {
        let guard = SqlGuard::new();
        // "select" alone is not blocked — only dangerous combinations
        assert_eq!(guard.check("products"), Verdict::Allow);
        assert_eq!(guard.check("user profile"), Verdict::Allow);
        assert_eq!(guard.check("orderby=price"), Verdict::Allow);
    }

    #[test]
    fn block_reason_contains_pattern_name() {
        let guard = SqlGuard::new();
        let v = guard.check("1 UNION SELECT 1,2,3");
        assert!(v.is_blocked());
        let reason = v.reason().expect("blocked verdict must have reason");
        assert!(
            reason.contains("union select"),
            "reason should name the matched pattern, got: {reason}"
        );
    }

    #[test]
    fn cached_safe_input_stays_safe() {
        let guard = SqlGuard::new();
        let input = "safe plain text";
        assert_eq!(guard.check(input), Verdict::Allow);
        assert_eq!(guard.check(input), Verdict::Allow);
        assert_eq!(guard.cache_size(), 1);
    }

    #[test]
    fn cache_cleared_then_rescanned() {
        let guard = SqlGuard::new();
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
