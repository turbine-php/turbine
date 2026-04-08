# Security

Turbine includes a **multi-layered OWASP security system** written entirely in Rust that runs inside the process. There is no external WAF, no ModSecurity, no additional network hop — all guards execute in the same address space as the HTTP server, with ~500 ns total overhead per request.

## Try it live

The [`examples/raw-php/security-demo`](../examples/raw-php/security-demo/) example ships an interactive browser UI with pre-built attack payloads for every guard:

```bash
cd examples/raw-php/security-demo
turbine serve
# open http://localhost:8083/
```

The demo page lets you pick any attack from a dropdown (SQL injection, code injection, obfuscation chains, behaviour attacks), send it with GET or POST (JSON / form-encoded), and watch the HTTP 403 response with the exact matched pattern — PHP never executes.

---

## Architecture: The Fortress Model

Turbine's security operates in 6 layers (called "Camadas"):

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

The OWASP guards are implemented as Aho-Corasick automata compiled once at startup. Each request is scanned in a single O(n) pass regardless of the number of patterns. Results are cached by xxHash-64 so identical inputs pay ~50 ns on cache hit.

**Coverage includes GET query strings, POST `application/x-www-form-urlencoded` bodies, and POST `application/json` bodies** (first 8 KB scanned).

### SQL Injection Guard

```toml
[security]
sql_guard = true
```

36 patterns matched case-insensitively:

| Category | Patterns |
|----------|----------|
| Classic | `union select`, `union all select`, `' or '1'='1`, `or 1=1--`, `or 1=1#` |
| Destructive | `drop table`, `drop database`, `truncate table`, `delete from`, `insert into`, `update set` |
| Comment bypass | `/**/`, `-- -` |
| Schema discovery | `information_schema`, `table_name`, `column_name` |
| Time-based blind | `sleep(`, `benchmark(`, `waitfor delay`, `pg_sleep(` |
| Stacked queries | `; drop`, `; delete`, `; insert`, `; update` |
| File exfiltration | `load_file(`, `into outfile`, `into dumpfile` |
| Error-based | `extractvalue(`, `updatexml(`, `exp(~(` |
| Aggregation | `group_concat(` |
| Encoding tricks | `char(0x`, `concat(0x` |

When triggered:
- Returns **HTTP 403** with `403 Forbidden: SQL injection pattern: <name>`
- Calls `behaviour_guard.record_sqli_attempt(ip)` — IP is permanently blocked after `sqli_block_threshold` attempts

### Code Injection Guard

```toml
[security]
code_injection_guard = true
```

Two-phase detection — obfuscation chains are checked first (higher severity), then basic patterns:

**Obfuscation chains** (phase 1, 7 patterns):

| Pattern | Technique |
|---------|-----------|
| `base64_decode(base64_decode(` | Double base64 |
| `eval(base64_decode(` | Classic webshell |
| `eval(gzinflate(base64_decode(` | Triple-layer (gzip + base64 + eval) |
| `assert(base64_decode(` | assert-based eval bypass |
| `eval(str_rot13(` | ROT13 obfuscation |
| `preg_replace("/.*/e"` | Deprecated `/e` modifier exec |
| `create_function(""` | Anonymous function exec |

**Basic patterns** (phase 2, 36 patterns) — includes `eval(`, `assert(`, `system(`, `exec(`, `shell_exec(`, `passthru(`, `popen(`, `proc_open(`, `pcntl_exec(`, `` ` `` (backtick), `base64_decode(`, `str_rot13(`, `gzinflate(`, `gzuncompress(`, `gzdecode(`, `chr(`, `pack(`, `str_replace(`, `include(`, `include_once(`, `require(`, `require_once(`, `$_GET[`, `$_POST[`, `$_REQUEST[`, `$_COOKIE[`, `$$`, `ReflectionFunction`, `call_user_func(`, `call_user_func_array(`, `create_function(`, `->__construct(`, `::__callStatic(`, `eval(eval(`.

When triggered:
- Returns **HTTP 403** with `403 Forbidden: Code injection (obfuscation chain): <pattern>` or `Code injection pattern: <pattern>`

### Behaviour Guard

```toml
[security]
behaviour_guard              = true
max_requests_per_second      = 100   # rate limit per IP
rate_limit_window            = 60    # window in seconds
sqli_block_threshold         = 3     # permanent block after N SQLi attempts
```

Per-IP profiles tracked in a lock-free `DashMap`. Three detection mechanisms:

| Mechanism | Condition | Response |
|-----------|-----------|----------|
| **Rate limiting** | `req/s > max_requests_per_second` after 10-req warm-up | 403, increments `total_blocked` |
| **Scanning detection** | `error_count / request_count > 0.5` after 20 requests | 403, IP blocked 5 minutes |
| **SQLi accumulation** | `sqli_attempts >= sqli_block_threshold` | 403, IP blocked 10 minutes |

The SQL guard automatically calls `record_sqli_attempt(ip)` on every blocked SQL injection — no manual wiring needed. An IP that fires 3 SQLi attempts in any time window is blocked even for subsequent clean requests.

Total blocked counter: `GET /_/metrics` exposes `turbine_security_blocks_total`.

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

All guards run in-process with Aho-Corasick automata compiled at startup. Pattern matching is O(n) in input length regardless of the number of patterns.

| Guard | Mechanism | Cache hit | Cache miss |
|-------|-----------|-----------|------------|
| SQL Injection | Aho-Corasick + xxHash-64 cache | ~50 ns | ~150 ns |
| Code Injection | Aho-Corasick (2 phase) | — | ~100–200 ns |
| Path traversal | String scan + canonicalise | — | ~50 ns |
| Behaviour (rate limit) | DashMap per-IP profile | — | ~200 ns |
| **Total** | | **~50 ns** (cache hit) | **~500 ns** |

POST JSON body: first 8 KB scanned (cap avoids CPU waste on large uploads). Benchmark on a 100 KB JSON body showed **no measurable difference** vs a GET request — the 8 KB window costs ~4 µs vs the ~10 ms PHP execution floor.

This is negligible compared to PHP execution time (typically 1–50 ms).

## Comparison with PHP-FPM + Nginx

| Feature | PHP-FPM + Nginx | Turbine |
|---------|-----------------|---------|
| SQL injection | Not built-in (needs ModSecurity) | Built-in, 36 patterns |
| Code injection | Not built-in | Built-in, 36 patterns + 7 obfuscation chains |
| Rate limiting | Nginx `limit_req` (coarse) | Per-IP, per-window, configurable |
| SQLi IP banning | Not built-in | Automatic after N attempts |
| Execution whitelist | Nginx `location` rules (manual) | Auto-configured from app structure |
| Upload scanning | Not built-in | Extension + content signature scan |
| Path traversal | Nginx rules | Built-in, always active |
| POST body scanning | Not built-in | JSON + form bodies scanned (8 KB cap) |
| `disable_functions` | php.ini (manual) | Auto-configured |
| Security overhead | External process / network | ~500 ns in-process |
| `open_basedir` | php.ini | Auto-configured |
