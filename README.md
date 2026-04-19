# Turbine

[![Tests](https://github.com/turbine-php/turbine/actions/workflows/test.yml/badge.svg?branch=master)](https://github.com/turbine-php/turbine/actions/workflows/test.yml)
[![Docker Release](https://github.com/turbine-php/turbine/actions/workflows/docker-release.yml/badge.svg)](https://github.com/turbine-php/turbine/actions/workflows/docker-release.yml)
[![OpenSSF Scorecard](https://api.securityscorecards.dev/projects/github.com/turbine-php/turbine/badge)](https://securityscorecards.dev/viewer/?uri=github.com/turbine-php/turbine)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Docker Pulls](https://img.shields.io/docker/pulls/katisuhara/turbine-php)](https://hub.docker.com/r/katisuhara/turbine-php)

<p align="center">
  <img src="assets/logo.png" alt="Turbine" width="400" />
</p>

High-performance PHP application server written in Rust, powered by the PHP embed SAPI — with a built-in in-process sandbox (execution whitelist, upload hardening, path-traversal guard, PHP INI hardening) plus a lightweight heuristic input filter and per-IP behaviour guard.

Turbine replaces the traditional **Nginx + PHP-FPM + OPcache** stack with a single binary that embeds PHP directly, eliminating inter-process communication overhead and reducing latency. The security layer runs inside the same process — no extra hop, no extra service — but it is intentionally *not* a WAF: if you need OWASP CRS-level rule coverage, put a real WAF (Cloudflare, Coraza, Caddy + coraza, libmodsecurity) in front of Turbine.

> [!WARNING]
> **This project is under active development and is not yet ready for production use.**
> APIs, configuration format, and behaviour may change without notice.
> You are welcome to try it out using the example projects in [`examples/`](examples/) and [`laravel-test/`](laravel-test/).
> Bug reports, feedback, and pull requests are very welcome — contributions of any kind are greatly appreciated.

## Features

- **Single binary** — no Nginx, no PHP-FPM, no reverse proxy
- **Persistent workers** — process and thread modes with automatic scaling
- **Zero-copy IPC** — in-memory channels for thread mode (ZTS), minimal overhead for high-throughput workloads
- **Built-in sandbox** — execution whitelist, data-dir guard, path-traversal guard, PHP INI hardening, heuristic SQL/code input filter, per-IP behaviour guard (rate limit + scan detection) — all in Rust, ~500 ns overhead
- **ACME auto-TLS** — automatic Let's Encrypt certificates
- **Virtual hosting** — multiple domains on one server, SNI per-host TLS
- **OPcache preload** — bytecode compiled once and kept in memory across all workers (not just per-process OPcache)
- **Hot reload** — file watcher for development
- **Framework support** — Laravel, Symfony, Phalcon, WordPress
- **Structured logging** — JSON output, Datadog/Loki compatible
- **Compression** — Brotli, Zstd, Gzip
- **Early Hints** — HTTP 103 support
- **X-Sendfile** — efficient large file delivery
- **App embedding** — pack your entire PHP application into a single self-contained binary for distribution
- **Built-in observability** — Prometheus metrics, live dashboard, per-IP blocked request log

## Security — Built-in Sandbox

Turbine includes a multi-layered in-process security sandbox written in Rust. It is *not* a WAF and does *not* claim OWASP Top 10 coverage — for full rule coverage put Cloudflare, Coraza, or libmodsecurity + OWASP CRS in front of Turbine.

```
Request → Execution Whitelist → Data Directory Guard → Path Guard
        → Heuristic Input Filter (SQL/code patterns) → Behaviour Guard
        → PHP Execution
```

Each stage is an Aho-Corasick scan or O(1) check. Total overhead is ~500 ns per request — negligible compared to PHP execution.

| Layer | What it does | Overhead |
|-------|---------------|----------|
| **Execution whitelist** | Only the detected entry point (or an explicit whitelist) can be executed via HTTP | O(1) hash |
| **Data-dir guard** | Blocks PHP execution inside `uploads/`, `storage/`, etc. even if an attacker drops a `.php` file there | O(1) |
| **Path guard** | Rejects `../`, null bytes, double-encoding | ~50 ns |
| **SQL input filter** | Heuristic Aho-Corasick scan for high-signal tokens (`UNION SELECT`, `SLEEP(`, `LOAD_FILE(`, `INTO OUTFILE`, stacked queries, hex obfuscation, …). Tiered by `paranoia_level`. | ~150 ns |
| **Code input filter** | Heuristic scan for `eval(`, `system(`, `shell_exec(`, backtick, obfuscation chains (`eval(base64_decode(…))`), `ReflectionFunction`, `$$`, …. Tiered by `paranoia_level`. | ~100–200 ns |
| **Behaviour guard** | Per-IP scanning detection (high 4xx rate) and SQLi-attempt accumulation → temporary IP block. Optional rate limit (off by default). | ~200 ns |

POST bodies (JSON and form-encoded) are scanned as well as query strings (first 8 KB of JSON).

### Honest caveats

- The SQL/code input filters are **heuristic** — they match substrings, not parsed tokens. They will miss clever obfuscation and can produce false positives on legitimate technical content. Tune with `paranoia_level` (0–3, default 1) and `exclude_paths`.
- `max_requests_per_second` defaults to `0` (rate limit disabled). Set it explicitly if you want a hard cap per IP.
- Path-traversal, execution whitelist, data-dir guard, and PHP INI hardening are deterministic and always safe to leave on.

All guards are enabled by default. Toggle individually in `turbine.toml`:

```toml
[security]
enabled                  = true
sql_guard                = true
code_injection_guard     = true
path_traversal_guard     = true
behaviour_guard          = true
paranoia_level           = 1         # 0=off, 1=high-signal (default), 2=moderate, 3=aggressive (high FP)
exclude_paths            = ["/admin", "/api/docs"]  # skip input filter (behaviour guard still runs)
max_requests_per_second  = 0         # 0 = disabled (opt-in)
sqli_block_threshold     = 3         # IP blocked after N heuristic SQLi matches
```

> **Try it live:** the [`examples/raw-php/security-demo`](examples/raw-php/security-demo/) example ships an interactive browser UI where you can fire every attack type and watch them blocked in real time.

See [docs/security.md](docs/security.md) for the full reference.

## Docker

Pre-built images are available on Docker Hub for every release:

```bash
docker pull katisuhara/turbine-php
```

### Available Tags

| Tag | PHP | Thread Safety |
|-----|-----|---------------|
| `latest` | 8.4 | NTS |
| `latest-php8.4-nts` | 8.4 | NTS |
| `latest-php8.4-zts` | 8.4 | ZTS |
| `latest-php8.5-nts` | 8.5 | NTS |
| `latest-php8.5-zts` | 8.5 | ZTS |
| `<version>-php8.4-nts` | 8.4 | NTS |
| `<version>-php8.4-zts` | 8.4 | ZTS |
| `<version>-php8.5-nts` | 8.5 | NTS |
| `<version>-php8.5-zts` | 8.5 | ZTS |

All images include **Phalcon** and **Redis** extensions pre-compiled.

### Quick Run

```bash
# NTS (default)
docker run -d -p 8080:8080 -e PORT=8080 \
  -v ./my-app:/var/www/html \
  katisuhara/turbine-php

# ZTS (thread mode)
docker run -d -p 8080:8080 -e PORT=8080 \
  -v ./my-app:/var/www/html \
  katisuhara/turbine-php:latest-php8.4-zts
```

See [docker/README.md](docker/README.md) for Docker Compose examples, configuration, and build customization.

## Quick Start

```bash
# Create a PHP project directory
mkdir myapp && cd myapp

# Generate default configuration
turbine init

# Create a test page
echo '<?php echo "Hello from Turbine!";' > index.php

# Start the server
turbine serve --root .
```

Open http://127.0.0.1:8080 in your browser.

### Runtime library path

On macOS, set the library path before running:

```bash
# NTS
export DYLD_LIBRARY_PATH="/path/to/vendor/php-embed/lib"

# ZTS
export DYLD_LIBRARY_PATH="/path/to/vendor/php-embed-zts/lib"
```

On Linux, use `LD_LIBRARY_PATH` instead.

## Worker Modes

> **The most important choice before deploying Turbine.** Choose at build time — process and thread modes require different PHP builds (NTS vs ZTS).

Turbine has two worker backends:

| Mode | `worker_mode` | PHP Build | Isolation | When to use |
|------|--------------|-----------|-----------|-------------|
| **Process** (default) | `"process"` | NTS or ZTS | Each worker = separate OS process | Default. Any extension, full crash isolation |
| **Thread** | `"thread"` | **ZTS only** | One process, N threads (TSRM) | Maximum throughput, ZTS-safe extensions |

```toml
[server]
# Process mode — default, works with any PHP extension
worker_mode = "process"

# Thread mode — ZTS PHP required, highest throughput
# worker_mode = "thread"
```

**Process mode** forks one OS process per worker. A crash in one worker is isolated — others keep running. Works with NTS or ZTS PHP.

**Thread mode** spawns OS threads sharing one address space. Uses in-memory channels (zero `pipe(2)` syscalls) instead of pipes, and workers communicate as Rust structs rather than serialized bytes. Requires PHP compiled with `--enable-zts` (use `./build.sh` → *Thread mode (ZTS)*). A PHP segfault takes down all threads.

See [docs/worker.md](docs/worker.md) for the full guide, ZTS extension compatibility, and benchmarks.

## Requirements

- **Rust** 1.75+ (install via [rustup](https://rustup.rs))
- **PHP 8.1+** source or embed SAPI library
- **macOS** or **Linux** (x86_64 / aarch64)

### macOS build dependencies

```bash
brew install autoconf automake bison re2c pkg-config \
    openssl@3 libxml2 icu4c libzip oniguruma curl \
    libsodium libpng libjpeg-turbo freetype gd
```

### Linux build dependencies (Debian/Ubuntu)

```bash
apt install build-essential autoconf automake bison re2c pkg-config \
    libssl-dev libxml2-dev libicu-dev libzip-dev libonig-dev libcurl4-openssl-dev \
    libsodium-dev libpng-dev libjpeg-dev libfreetype-dev libgd-dev
```

## Building

The easiest way to build Turbine is with the interactive build script:

```bash
./build.sh
```

The script walks you through every step with a keyboard-driven UI:

1. **Build mode** — choose with arrow keys + Enter:
   - `Process mode (NTS)` — fork-based workers, max compatibility
   - `Thread mode (ZTS)` — in-memory channel workers, max throughput
   - `Both (NTS + ZTS)` — build both variants

2. **PHP version** — defaults to `8.4.6`, enter any `x.y.z` release

3. **PECL extensions** — toggle with Space, confirm with Enter:
   - `Phalcon` — high-performance PHP framework (C extension)
   - `Redis` — PHP Redis client
   - `Imagick` — ImageMagick bindings
   - `APCu` — user data cache
   - `Xdebug` — debugger and profiler (dev only)

4. **Release or debug** build

After confirmation, the script downloads the PHP source, compiles it with the embed SAPI and 45+ extensions, installs PECL extensions, and builds Turbine. Output:

| Build | PHP install path | Turbine binary |
|-------|-----------------|----------------|
| NTS | `vendor/php-embed/` | `target/release/turbine` |
| ZTS | `vendor/php-embed-zts/` | `target/release/turbine` |

> **Note:** `vendor/` is excluded from git. Every developer runs `./build.sh` once after cloning.

### Manual build

If you prefer to build manually:

```bash
# 1. Compile PHP with embed SAPI
# NTS
./scripts/build-php-embed.sh
# ZTS
ZTS_BUILD=1 ./scripts/build-php-embed.sh

# 2. Build Turbine
# NTS
PHP_CONFIG=$PWD/vendor/php-embed/bin/php-config cargo build --release
# ZTS
PHP_CONFIG=$PWD/vendor/php-embed-zts/bin/php-config cargo build --release
```

The binary is at `target/release/turbine`.

## Configuration

Create a `turbine.toml` in your project root:

```toml
[server]
workers = 0               # 0 = auto-detect (CPU cores)
listen = "127.0.0.1:8080"
worker_mode = "thread"     # "process" or "thread" (ZTS required)
request_timeout = 30       # seconds, 0 = no timeout
worker_max_requests = 1000 # respawn after N requests (0 = never)

[php]
extension_dir = ""         # auto-detected if empty
ini = { "memory_limit" = "256M", "upload_max_filesize" = "50M" }

[security]
enabled = true
sql_guard = true
path_traversal_guard = true
code_injection_guard = true
behaviour_guard = true

[sandbox]
seccomp = true             # Linux only
execution_mode = "framework"

[logging]
level = "info"             # trace, debug, info, warn, error

[dashboard]
enabled = true             # /_/dashboard HTML page
statistics = true          # /_/metrics and /_/status endpoints
# token = "my-secret"     # Bearer token for all /_/* endpoints (header only)
```

See [docs/config.md](docs/config.md) for the full configuration reference.

## TLS

```bash
# Manual certificates
turbine serve --tls-cert cert.pem --tls-key key.pem

# Automatic Let's Encrypt (configure in turbine.toml)
```

See [docs/tls.md](docs/tls.md) for ACME auto-TLS setup.

## Documentation

| Topic | Link |
|-------|------|
| Architecture | [docs/README.md](docs/README.md) |
| **Worker modes** | [**docs/worker.md**](docs/worker.md) — process vs thread, the key choice |
| Configuration reference | [docs/config.md](docs/config.md) |
| Building from source | [docs/compile.md](docs/compile.md) |
| **Security model** | [**docs/security.md**](docs/security.md) — sandbox layers, heuristic filters, PHP hardening |
| **Dashboard & Internal API** | [**docs/dashboard.md**](docs/dashboard.md) — UI panels, blocked IPs, Prometheus, cache clear |
| Performance | [docs/performance.md](docs/performance.md) |
| Laravel integration | [docs/laravel.md](docs/laravel.md) |
| TLS & ACME | [docs/tls.md](docs/tls.md) |
| **Virtual hosting** | [**docs/virtual-hosts.md**](docs/virtual-hosts.md) — multiple domains, SNI, ACME |
| PHP extensions | [docs/extensions.md](docs/extensions.md) |
| Migration from Nginx | [docs/migrate.md](docs/migrate.md) |

## Project structure

```
crates/
  turbine-core/       Main server, CLI, HTTP handling
  turbine-php-sys/    PHP FFI bindings, embed SAPI integration
  turbine-engine/     PHP engine lifecycle management
  turbine-worker/     Worker pool (process & thread modes)
  turbine-security/   Sandbox, heuristic input filter, per-IP behaviour guard
  turbine-metrics/    Performance metrics
  turbine-cache/      Response caching
```

## License

Turbine is licensed under the [Apache License 2.0](LICENSE).

You are free to use, modify, and distribute this software in both open-source and proprietary projects, subject to the terms of the Apache 2.0 License.
