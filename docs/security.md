# Security

Turbine includes a multi-layered security system inspired by the OWASP Top 10. Security is enabled by default and operates in the HTTP request pipeline with minimal performance overhead.

## Architecture: The Fortress Model

Turbine's security operates in 5 layers (called "Camadas"):

```
Request → [Layer 1: Execution Whitelist]
        → [Layer 2: Data Directory Guard]
        → [Layer 3: Path Validation]
        → [Layer 4: OWASP Guards (SQL, Code, Behaviour)]
        → [Layer 5: PHP INI Hardening]
        → PHP Execution
```

Each layer is independent. Disabling one does not affect the others.

## Layer 1: Execution Whitelist

Controls which PHP files can be executed via HTTP requests.

### Framework Mode (default)

Turbine detects the entry point via structural heuristics (`public/index.php` or root `index.php`) and only allows that file:

```toml
[sandbox]
execution_mode = "framework"
```

- Apps with `public/index.php` — Only `public/index.php` is executable
- Apps with root `index.php` — Only `index.php` is executable

All other PHP files return **403 Forbidden**, even if they exist. This prevents direct access to config files, helpers, or migration scripts via URL.

### Strict Mode

Explicitly whitelist which files can be executed:

```toml
[sandbox]
execution_mode = "strict"
execution_whitelist = [
    "public/index.php",
    "api/webhook.php",
    "cron/daily.php",
]
```

## Layer 2: Data Directory Guard

Blocks PHP execution inside directories that should only contain data:

```toml
[sandbox]
data_directories = ["storage/", "uploads/", "public/uploads/"]
```

Even if an attacker uploads a `.php` file to `uploads/`, it cannot be executed. This blocks common web shell attacks.

### Upload Protection

```toml
[sandbox]
blocked_upload_extensions = [".php", ".phtml", ".phar", ".php7", ".php8", ".inc", ".phps", ".pht", ".pgif"]
scan_upload_content = true
```

When `scan_upload_content` is enabled, Turbine inspects uploaded file content for PHP code signatures (`<?php`, `<?=`) regardless of the file extension.

## Layer 3: Path Validation

Prevents path traversal attacks:

- Blocks `../` sequences
- Blocks null bytes (`%00`)
- Canonicalizes paths before execution
- Rejects double-encoded paths

No configuration needed — always active when security is enabled.

## Layer 4: OWASP Guards

### SQL Injection Guard

Detects SQL injection patterns in HTTP parameters using Aho-Corasick multi-pattern matching (~150ns overhead per request):

```toml
[security]
sql_guard = true
```

Detects patterns like:
- `UNION SELECT`, `UNION ALL SELECT`
- `' OR 1=1`, `" OR ""="`
- `DROP TABLE`, `DELETE FROM`
- `EXEC xp_`, `EXECUTE sp_`
- Comment injection (`--`, `/**/`)
- Hex-encoded payloads

When triggered: returns **403 Forbidden** and logs the attempt.

### Code Injection Guard

Detects code injection payloads in HTTP parameters:

```toml
[security]
code_injection_guard = true
```

Detects:
- PHP code markers (`<?php`, `<?=`)
- Dangerous function calls (`eval(`, `system(`, `exec(`)
- Shell metacharacters (`|`, `&`, `;`, `` ` ``)
- Base64-encoded PHP payloads

### Behaviour Guard

Rate limiting and scanning detection per IP:

```toml
[security]
behaviour_guard = true
max_requests_per_second = 100
rate_limit_window = 60
```

Detects:
- Request rate exceeding threshold
- Rapid sequential requests to different URLs (vulnerability scanning)
- Repeated 4xx/5xx responses (fuzzing)
- Repeated SQLi attempts from the same IP

When triggered: returns **429 Too Many Requests**.

## Layer 5: PHP INI Hardening

Turbine configures PHP with secure defaults:

```toml
[sandbox]
disabled_functions = ["exec", "system", "passthru", "shell_exec", "proc_open", "popen", "pcntl_exec", "dl", "putenv"]
enforce_open_basedir = true
block_url_include = true
block_url_fopen = true
```

| Directive | Default | Purpose |
|-----------|---------|---------|
| `disable_functions` | 9 functions | Blocks shell execution |
| `open_basedir` | project root | Restricts file access |
| `allow_url_include` | Off | Blocks remote file inclusion (RFI) |
| `allow_url_fopen` | Off | Blocks remote file reads |
| `display_errors` | Off | Prevents error information disclosure |
| `expose_php` | Off | Hides PHP version from headers |

## Disabling Security

For development or benchmarking:

```toml
[security]
enabled = false

[sandbox]
enforce_open_basedir = false
block_url_include = false
block_url_fopen = false
disabled_functions = []
```

> **Warning:** Never disable security in production.

## Security vs Performance

The security layer adds minimal overhead:

| Guard | Overhead per Request |
|-------|---------------------|
| SQL injection | ~150ns |
| Code injection | ~100ns |
| Path traversal | ~50ns |
| Behaviour (rate limiting) | ~200ns |
| **Total** | **~500ns** (~0.0005ms) |

This is negligible compared to PHP execution time (typically 1-50ms).

## Comparison with PHP-FPM

| Feature | PHP-FPM + Nginx | Turbine |
|---------|-----------------|---------|
| SQL injection | Not built-in (needs ModSecurity) | Built-in |
| Rate limiting | Nginx `limit_req` | Built-in per IP |
| Execution whitelist | Nginx `location` rules | Auto-configured |
| Upload scanning | Not built-in | Built-in |
| Path traversal | Nginx rules | Built-in |
| `disable_functions` | php.ini | Auto-configured |
| `open_basedir` | php.ini | Auto-configured |
