# Security

Turbine includes a **multi-layered in-process security sandbox** written entirely in Rust that runs inside the same process as the HTTP server, with ~500 ns total overhead per request. There is no external WAF, no ModSecurity, no additional network hop.

> [!NOTE]
> Turbine is **not a WAF** and does **not** claim OWASP Top 10 coverage. The SQL/code input filters are lightweight heuristic Aho-Corasick scans — useful as a first line of defence, but not a substitute for a real rule-based WAF. If you need full WAF coverage (OWASP CRS, protocol validation, virtual patching), put Cloudflare, Coraza, Caddy + coraza, or libmodsecurity + OWASP CRS **in front of** Turbine.

## Try it live

The [`examples/raw-php/security-demo`](../examples/raw-php/security-demo/) example ships an interactive browser UI with pre-built attack payloads for every guard:

```bash
cd examples/raw-php/security-demo
turbine serve
# open http://localhost:8083/
```

The demo page lets you pick any attack from a dropdown (SQL injection, code injection, obfuscation chains, behaviour attacks), send it with GET or POST (JSON / form-encoded), and watch the HTTP 403 response with the matched pattern — PHP never executes.

---

## Architecture: The Fortress Model

Turbine's security operates in 5 layers (called "Camadas"):

```
Request → [Layer 1: Execution Whitelist]
        → [Layer 2: Data Directory Guard]
        → [Layer 3: Path Validation]
        → [Layer 4: Heuristic Input Filter + Behaviour Guard]
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

## Layer 4: Heuristic Input Filter + Behaviour Guard

Layer 4 is a **heuristic** input filter implemented as Aho-Corasick automata compiled once at startup, plus a per-IP behaviour guard. Each request is scanned in a single O(n) pass regardless of the number of patterns. Results are cached by xxHash-64 so identical inputs pay ~50 ns on cache hit. The cache is bounded at 8 192 entries.

**Coverage includes GET query strings, POST `application/x-www-form-urlencoded` bodies, and POST `application/json` bodies** (first 8 KB scanned).

> [!IMPORTANT]
> This is a heuristic substring matcher, not a parser. It will miss clever obfuscation and can produce false positives on legitimate technical content (documentation, admin tooling, query builders). Tune with `paranoia_level` and `exclude_paths`.

### Paranoia levels

Both the SQL and code input filters are tiered by `paranoia_level` (0–3). Each level is **cumulative** — level 2 includes level 1 patterns, level 3 includes 1 + 2.

| Level | Signal | False-positive risk | When to use |
|-------|--------|---------------------|-------------|
| **0** | — | — | Filter disabled (path/exec/INI guards still run) |
| **1** (default) | Very high | Very low | General web apps, default safe setting |
| **2** | High | Moderate | Closed-surface APIs, no admin tooling |
| **3** | Any hit | High | Demo / honeypot / testing only |

```toml
[security]
paranoia_level = 1              # default
exclude_paths  = ["/admin", "/api/docs"]
```

`exclude_paths` skips the input filter for requests whose path starts with any listed prefix. The behaviour guard, path guard, execution whitelist, and data-dir guard **still run** on excluded paths.

### SQL input filter

```toml
[security]
sql_guard = true
```

Patterns are case-insensitive, checked in a single Aho-Corasick pass:

| Level | Category | Examples |
|-------|----------|----------|
| **1** | Classic injection | `union select`, `union all select`, `or 1=1`, `or '1'='1` |
| **1** | Time-based blind | `sleep(`, `benchmark(`, `waitfor delay`, `pg_sleep(` |
| **1** | File primitives | `load_file(`, `into outfile`, `into dumpfile` |
| **1** | Error-based | `extractvalue(`, `updatexml(`, `exp(~(` |
| **1** | Hex obfuscation | `char(0x`, `concat(0x` |
| **2** | Destructive DDL | `drop table`, `drop database`, `truncate table` |
| **2** | Stacked queries | `; drop`, `; delete`, `; insert`, `; update` |
| **2** | Comment bypass | `/**/`, `-- -` |
| **2** | Aggregation | `group_concat(` |
| **3** | Destructive DML | `delete from`, `insert into`, `update set` |
| **3** | Schema discovery | `information_schema`, `table_name`, `column_name` |

When triggered:
- Returns **HTTP 403** with a reason containing the matched substring.
- At `paranoia_level >= 2`, calls `behaviour_guard.record_sqli_attempt(ip)` — IP is temporarily blocked after `sqli_block_threshold` matches. At the default `paranoia_level = 1` this coupling is intentionally off to avoid FP-driven bans.

### Code input filter

```toml
[security]
code_injection_guard = true
```

Two-phase detection — obfuscation chains are checked first (high severity), then the paranoia-tiered patterns:

**Obfuscation chains** (always loaded when the guard is on):

| Pattern | Technique |
|---------|-----------|
| `base64_decode(base64_decode(` | Double base64 |
| `eval(base64_decode(` | Classic webshell |
| `eval(gzinflate(base64_decode(` | Triple-layer (gzip + base64 + eval) |
| `assert(base64_decode(` | `assert`-based eval bypass |
| `eval(str_rot13(` | ROT13 obfuscation |
| `create_function(""` | Anonymous-function exec |
| `eval(eval(` | Nested eval |

**Tiered patterns:**

| Level | Category | Examples |
|-------|----------|----------|
| **1** | Direct exec | `eval(`, `assert(`, `create_function(`, `exec(`, `shell_exec(`, `system(`, `passthru(`, `popen(`, `proc_open(`, `pcntl_exec(` |
| **2** | Indirect / decoders | `call_user_func(`, `base64_decode(`, `gzinflate`, `gzuncompress`, `gzdecode`, `str_rot13`, `chr(`, `pack(`, `` ` ``, `$$`, `ReflectionFunction` |
| **3** | Language primitives (high FP) | `include(`, `include_once(`, `require(`, `require_once(`, `str_replace(`, `$_GET[`, `$_POST[`, `$_REQUEST[`, `$_COOKIE[`, `->__construct(`, `::__callStatic(` |

> [!TIP]
> Level 3 patterns appear routinely in legitimate PHP source, documentation, and tooling. Keep `paranoia_level = 1` unless you have a closed request surface and have tested your traffic.

### Behaviour guard

```toml
[security]
behaviour_guard              = true
max_requests_per_second      = 0     # 0 = disabled (opt-in)
rate_limit_window            = 60    # window in seconds
sqli_block_threshold         = 3     # temporary block after N heuristic SQLi matches
```

Per-IP profiles tracked in a lock-free `DashMap` with atomic counters. Three detection mechanisms:

| Mechanism | Condition | Response |
|-----------|-----------|----------|
| **Rate limiting** | `req/s > max_requests_per_second` after 10-req warm-up (only when `max_requests_per_second > 0`) | 403, increments `total_blocked` |
| **Scanning detection** | `error_count / request_count > 0.5` after 20 requests | 403, IP blocked 5 minutes |
| **SQLi accumulation** | `sqli_attempts >= sqli_block_threshold` (only when `paranoia_level >= 2`) | 403, IP blocked 10 minutes |

The rate limit defaults to **disabled** (`max_requests_per_second = 0`) — the previous default of 100 r/s was too aggressive for normal traffic bursts. Set a value explicitly only if you want a hard per-IP cap.

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
| SQL input filter | Aho-Corasick + xxHash-64 cache (bounded 8 192 entries) | ~50 ns | ~150 ns |
| Code input filter | Aho-Corasick (2 phase) | — | ~100–200 ns |
| Path traversal | String scan + canonicalise | — | ~50 ns |
| Behaviour guard | Lock-free DashMap per-IP profile (atomic counters) | — | ~200 ns |
| **Total** | | **~50 ns** (cache hit) | **~500 ns** |

POST JSON body: first 8 KB scanned (cap avoids CPU waste on large uploads). Benchmark on a 100 KB JSON body showed **no measurable difference** vs a GET request — the 8 KB window costs ~4 µs vs the ~10 ms PHP execution floor.

This is negligible compared to PHP execution time (typically 1–50 ms).

## Comparison with PHP-FPM + Nginx

| Feature | PHP-FPM + Nginx | Turbine |
|---------|-----------------|---------|
| SQL injection filter | Not built-in (needs ModSecurity) | Built-in heuristic, tiered by paranoia level |
| Code injection filter | Not built-in | Built-in heuristic, tiered + obfuscation chains |
| Rate limiting | Nginx `limit_req` (coarse) | Per-IP, per-window, opt-in (off by default) |
| Scan detection | Not built-in | Automatic (temporary IP block on high 4xx ratio) |
| Execution whitelist | Nginx `location` rules (manual) | Auto-configured from app structure |
| Upload scanning | Not built-in | Extension + content signature scan |
| Path traversal | Nginx rules | Built-in, always active |
| POST body scanning | Not built-in | JSON + form bodies scanned (8 KB cap) |
| `disable_functions` | php.ini (manual) | Auto-configured |
| Security overhead | External process / network | ~500 ns in-process |
| `open_basedir` | php.ini | Auto-configured |
| **WAF-grade rule coverage** (OWASP CRS, protocol validation) | Needs ModSecurity / Coraza | **Not provided** — put a real WAF upstream |
