# Configuration Reference

Turbine is configured through a `turbine.toml` file. By default, Turbine looks for this file in the application root directory. You can specify a custom path with `--config`.

Generate a default configuration:

```bash
turbine init
```

Validate an existing configuration:

```bash
turbine check
turbine check --config /path/to/turbine.toml
```

## Complete Configuration

```toml
# Turbine Runtime Configuration

[server]
# Number of worker processes/threads (0 = auto-detect based on CPU cores)
workers = 4
# Address to listen on
listen = "127.0.0.1:8080"
# Worker backend: "process" (fork, default) or "thread" (requires ZTS PHP)
worker_mode = "process"
# Enable persistent workers (bootstrap once, handle many requests)
# When true, workers load the autoloader once and handle requests without re-initialization
# When false (default), each request executes php_execute_script with full lifecycle
# persistent_workers = true
# Boot script: executed ONCE per worker at startup (enables lightweight lifecycle)
# Path relative to app root, or absolute. Requires persistent_workers = true.
# worker_boot = "turbine-boot.php"
# Handler script: included on EVERY request (lightweight lifecycle)
# Requires persistent_workers = true and worker_boot to be set.
# worker_handler = "turbine-handler.php"
# Number of Tokio async I/O threads (default = number of CPU cores)
# Increase for more concurrent connections; decrease to leave cores for PHP workers
# tokio_worker_threads = 6
# Request timeout in seconds (0 = no timeout)
request_timeout = 30
# Max PHP requests per worker before respawn (prevents memory leaks)
worker_max_requests = 10000
# Internal channel capacity for single-process mode
channel_capacity = 64
# Max seconds to wait for a free worker before returning 503 (0 = use request_timeout)
# max_wait_time = 5
# PID file path
# pid_file = "/var/run/turbine.pid"
# Enable auto-scaling worker pool
auto_scale = false
# min_workers = 1
# max_workers = 16
# scale_down_idle_secs = 5

[server.tls]
enabled = false
# cert_file = "/path/to/cert.pem"
# key_file = "/path/to/key.pem"

[php]
# Path to PHP extensions directory (auto-detected if empty)
# extension_dir = "/path/to/extensions"
# PHP extensions to load (.so files)
# extensions = ["redis.so", "imagick.so"]
# Zend extensions to load
# zend_extensions = ["xdebug.so"]
memory_limit = "256M"
max_execution_time = 30
upload_max_filesize = "64M"
post_max_size = "64M"
opcache_memory = 128
jit_buffer_size = "64M"
upload_tmp_dir = "/tmp/turbine-uploads"
# OPcache preload script (path to preload file)
# preload_script = "vendor/preload.php"

# Arbitrary php.ini directives
# [php.ini]
# error_reporting = "E_ALL"
# date.timezone = "UTC"

[security]
# Master switch for all OWASP guards
enabled = true
# SQL injection detection (Aho-Corasick, ~150ns overhead)
sql_guard = true
# Code injection detection (eval, system, shell metacharacters)
code_injection_guard = true
# Path traversal prevention (../, null bytes)
path_traversal_guard = true
# Behaviour analysis (rate limiting, scanning detection)
behaviour_guard = true
# Max requests per second per IP (0 = unlimited)
max_requests_per_second = 100
# Rate limit time window in seconds
rate_limit_window = 60
# Block IP permanently after this many SQL injection attempts (resets after block expires)
sqli_block_threshold = 3

[sandbox]
# Execution mode: "framework" (detect entry point) or "strict" (whitelist only)
execution_mode = "framework"
# Files allowed to execute in strict mode
# execution_whitelist = ["public/index.php"]
# Directories where PHP execution is blocked
data_directories = ["storage/", "uploads/", "public/uploads/"]
# File extensions blocked from upload
blocked_upload_extensions = [".php", ".phtml", ".phar", ".php7", ".php8", ".inc", ".phps", ".pht", ".pgif"]
# Scan upload content for PHP code
scan_upload_content = true
# PHP functions disabled at runtime
disabled_functions = ["exec", "system", "passthru", "shell_exec", "proc_open", "popen", "pcntl_exec", "dl", "putenv"]
# Restrict file access to project directories
enforce_open_basedir = true
# Block allow_url_include
block_url_include = true
# Block allow_url_fopen
block_url_fopen = true

[cache]
# Enable in-memory response cache
enabled = true
# Cache TTL in seconds
ttl_seconds = 30
# Maximum cached responses
max_entries = 1024

[logging]
# Log level: trace, debug, info, warn, error
level = "info"
# Access log file (empty = disabled)
# access_log = "/var/log/turbine/access.log"

[compression]
# Enable response compression
enabled = true
# Minimum response size to compress (bytes)
min_size = 1024
# Compression level (1-9)
level = 6
# Algorithm preference order
algorithms = ["br", "zstd", "gzip"]

[session]
enabled = true
save_path = "/tmp/turbine-sessions"
cookie_name = "PHPSESSID"
# 0 = session cookie (until browser close)
cookie_lifetime = 0
cookie_httponly = true
# Auto-enabled when TLS is active
cookie_secure = false
# Lax, Strict, or None
cookie_samesite = "Lax"
# Session garbage collection max lifetime (seconds)
gc_maxlifetime = 1440

[cors]
enabled = false
# allow_origins = ["https://example.com"]  # Use ["*"] for any
allow_credentials = false
allow_methods = ["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS"]
allow_headers = ["Content-Type", "Authorization", "X-Requested-With"]
# expose_headers = ["X-Custom-Header"]
max_age = 86400

[error_pages]
# Custom error page paths (relative to app root)
# not_found = "errors/404.html"
# server_error = "errors/500.html"

[watcher]
# Enable file watching (development only)
enabled = false
paths = ["app/", "config/", "routes/", "src/", "public/"]
extensions = ["php", "env"]
# Debounce delay in milliseconds
debounce_ms = 500

[early_hints]
# Enable HTTP 103 Early Hints
enabled = true

[x_sendfile]
# Enable X-Sendfile / X-Accel-Redirect support
enabled = false
# Base directory for file serving (security boundary)
# root = "private-files/"

[structured_logging]
# Enable turbine_log() PHP function
enabled = true
# Output target: "stdout", "stderr", or file path
output = "stderr"

[acme]
# Enable automatic Let's Encrypt certificates.
# When [[virtual_hosts]] are configured, their domains are auto-collected —
# you do NOT need to list them here. Use 'domains' only for single-site setups
# without virtual hosting.
enabled = false
# domains = ["example.com", "www.example.com"]
# email = "admin@example.com"
cache_dir = "/var/lib/turbine/acme"
# Use Let's Encrypt staging server for testing
staging = false

[embed]
# Enable embedded app extraction
enabled = false
# extract_dir = "/tmp/turbine-app"

[dashboard]
# Enable the /_/dashboard HTML page
enabled = true
# Enable /_/metrics and /_/status endpoints
statistics = true
# Bearer token to protect internal endpoints (comment out for open access)
# token = "my-secret-token"

# Named worker pools for route-based splitting
# [[worker_pools]]
# match_path = "/api/slow/*"
# min_workers = 1
# max_workers = 4
# name = "slow-api"

# Virtual hosting: serve different PHP apps on different domains
# [[virtual_hosts]]
# domain = "xpto.com"
# root = "/var/www/xpto"
# aliases = ["www.xpto.com"]
# # entry_point = "index.php"
# # tls_cert = "/etc/ssl/xpto.com.pem"
# # tls_key = "/etc/ssl/xpto.com-key.pem"
```

## Section Reference

### `[server]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `workers` | integer | CPU cores | Number of PHP worker processes |
| `listen` | string | `"127.0.0.1:9000"` | Bind address |
| `worker_mode` | string | `"process"` | Worker backend: `"process"` (fork) or `"thread"` (ZTS) |
| `persistent_workers` | bool | none | Enable persistent workers (bootstrap once, handle many) |
| `worker_boot` | string | none | Boot script path (once per worker). See [worker-lifecycle.md](worker-lifecycle.md) |
| `worker_handler` | string | none | Handler script path (per request). See [worker-lifecycle.md](worker-lifecycle.md) |
| `tokio_worker_threads` | integer | CPU cores | Number of Tokio async I/O threads |
| `request_timeout` | integer | `30` | Request timeout in seconds (0 = unlimited) |
| `worker_max_requests` | integer | `10000` | Requests per worker before respawn |
| `channel_capacity` | integer | `64` | Channel size for single-process mode |
| `max_wait_time` | integer | `0` | Max queue wait before 503 (0 = request_timeout) |
| `pid_file` | string | none | PID file path |
| `auto_scale` | bool | `false` | Enable dynamic worker scaling |
| `min_workers` | integer | `1` | Minimum workers when scaling |
| `max_workers` | integer | CPUs × 2 | Maximum workers when scaling |
| `scale_down_idle_secs` | integer | `5` | Idle time before scaling down |

### `[server.tls]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `false` | Enable HTTPS |
| `cert_file` | string | none | Path to PEM certificate chain |
| `key_file` | string | none | Path to PEM private key |

### `[php]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `extension_dir` | string | auto | Directory for `.so` extensions |
| `extensions` | array | `[]` | PHP extensions to load |
| `zend_extensions` | array | `[]` | Zend extensions to load |
| `memory_limit` | string | `"256M"` | PHP memory limit |
| `max_execution_time` | integer | `30` | Script timeout (seconds) |
| `upload_max_filesize` | string | `"64M"` | Max upload file size |
| `post_max_size` | string | `"64M"` | Max POST body size |
| `opcache_memory` | integer | `128` | OPcache memory (MB) |
| `jit_buffer_size` | string | `"64M"` | JIT compilation buffer |
| `upload_tmp_dir` | string | `"/tmp/turbine-uploads"` | Upload temp directory |
| `preload_script` | string | none | OPcache preload script |

### `[php.ini]`

Arbitrary php.ini directives as key-value pairs:

```toml
[php.ini]
error_reporting = "E_ALL"
date.timezone = "America/Sao_Paulo"
"session.gc_probability" = "1"
```

### `[security]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Master security switch |
| `sql_guard` | bool | `true` | SQL injection detection |
| `code_injection_guard` | bool | `true` | Code injection detection |
| `path_traversal_guard` | bool | `true` | Path traversal prevention |
| `behaviour_guard` | bool | `true` | Rate limiting & scanning detection |
| `max_requests_per_second` | integer | `100` | Rate limit per IP |
| `rate_limit_window` | integer | `60` | Rate limit window (seconds) |
| `sqli_block_threshold` | integer | `3` | SQLi attempts before permanent IP block |

### `[sandbox]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `execution_mode` | string | `"framework"` | `"framework"` or `"strict"` |
| `execution_whitelist` | array | `["public/index.php"]` | Whitelist for strict mode |
| `data_directories` | array | `["storage/", ...]` | Dirs blocked from PHP execution |
| `blocked_upload_extensions` | array | `[".php", ...]` | Upload extension blacklist |
| `scan_upload_content` | bool | `true` | Scan uploads for PHP code |
| `disabled_functions` | array | `["exec", ...]` | Disabled PHP functions |
| `enforce_open_basedir` | bool | `true` | Restrict file access |
| `block_url_include` | bool | `true` | Block remote includes |
| `block_url_fopen` | bool | `true` | Block remote file opens |

### `[compression]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable response compression |
| `min_size` | integer | `1024` | Min bytes before compressing |
| `level` | integer | `6` | Compression level (1-9) |
| `algorithms` | array | `["br", "zstd", "gzip"]` | Priority order |

### `[session]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable session handling |
| `save_path` | string | `"/tmp/turbine-sessions"` | Session file storage |
| `cookie_name` | string | `"PHPSESSID"` | Session cookie name |
| `cookie_lifetime` | integer | `0` | Cookie TTL (0 = session) |
| `cookie_httponly` | bool | `true` | HttpOnly flag |
| `cookie_secure` | bool | `false` | Secure flag (auto with TLS) |
| `cookie_samesite` | string | `"Lax"` | SameSite policy |
| `gc_maxlifetime` | integer | `1440` | GC max lifetime (seconds) |

### `[cors]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `false` | Enable CORS headers |
| `allow_origins` | array | `[]` | Allowed origins (`["*"]` for any) |
| `allow_credentials` | bool | `false` | Allow credentials |
| `allow_methods` | array | `["GET", "POST", ...]` | Allowed methods |
| `allow_headers` | array | `["Content-Type", ...]` | Allowed headers |
| `expose_headers` | array | `[]` | Exposed headers |
| `max_age` | integer | `86400` | Preflight cache (seconds) |

### `[watcher]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `false` | Enable file watching |
| `paths` | array | `["app/", "config/", ...]` | Directories to watch |
| `extensions` | array | `["php", "env"]` | File extensions to watch |
| `debounce_ms` | integer | `500` | Debounce delay (ms) |

### `[dashboard]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable `/_/dashboard` HTML page |
| `statistics` | bool | `true` | Enable `/_/metrics` and `/_/status` endpoints |
| `token` | string | none | Bearer token to protect internal endpoints |

When `token` is set, all `/_/*` endpoints require `Authorization: Bearer <token>`. See [Dashboard & Internal API](dashboard.md) for the full reference.

### `[acme]`

Automatic TLS certificates via Let's Encrypt. See [TLS & ACME](tls.md) for the full guide.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `false` | Enable ACME auto-TLS |
| `domains` | string[] | `[]` | Domain names (**not needed with `[[virtual_hosts]]`** — domains are auto-collected) |
| `email` | string | none | Contact email for Let's Encrypt notifications |
| `cache_dir` | string | `"/var/lib/turbine/acme"` | Certificate cache directory |
| `staging` | bool | `false` | Use Let's Encrypt staging server (for testing) |

> **With virtual hosting:** Do not set `domains` — Turbine auto-collects `domain` + `aliases` from every `[[virtual_hosts]]` entry. Virtual hosts with their own `tls_cert`/`tls_key` are excluded from ACME.

### `[[worker_pools]]`

Named worker pools route specific URL patterns to dedicated worker groups:

```toml
[[worker_pools]]
match_path = "/api/slow/*"
min_workers = 1
max_workers = 4
name = "slow-api"

[[worker_pools]]
match_path = "/webhook"
min_workers = 2
max_workers = 2
name = "webhooks"
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `match_path` | string | required | URL pattern (`*` = wildcard) |
| `min_workers` | integer | `1` | Minimum pool workers |
| `max_workers` | integer | `4` | Maximum pool workers |
| `name` | string | none | Pool name for logging |

### `[[virtual_hosts]]`

Serve different PHP applications on different domains from a single Turbine instance. See [Virtual Hosting](virtual-hosts.md) for the full guide.

```toml
[[virtual_hosts]]
domain = "xpto.com"
root = "/var/www/xpto"
aliases = ["www.xpto.com"]

[[virtual_hosts]]
domain = "outro.com"
root = "/var/www/outro"
tls_cert = "/etc/ssl/outro.com.pem"
tls_key = "/etc/ssl/outro.com-key.pem"
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `domain` | string | required | Primary domain name (matched against `Host` header) |
| `root` | string | required | Application root directory |
| `aliases` | string[] | `[]` | Additional domains that route to this vhost |
| `entry_point` | string | auto-detected | PHP entry point file |
| `tls_cert` | string | none | PEM certificate (per-host SNI) |
| `tls_key` | string | none | PEM private key (per-host SNI) |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `RUST_LOG` | Override log level (`trace`, `debug`, `info`, `warn`, `error`) |
| `DYLD_LIBRARY_PATH` | macOS: path to `libphp.dylib` |
| `LD_LIBRARY_PATH` | Linux: path to `libphp.so` |
| `PHP_CONFIG` | Path to `php-config` binary (build time) |
| `TURBINE_EMBED_DIR` | Directory to embed in binary (build time) |

## Configuration Precedence

1. CLI flags (highest priority)
2. `turbine.toml` file
3. Built-in defaults (lowest priority)

The `--config` flag specifies the TOML file. If not provided, Turbine looks for `turbine.toml` in the app root directory (specified by `--root`).
