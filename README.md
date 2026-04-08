# Turbine

High-performance PHP application server written in Rust, powered by the PHP embed SAPI.

Turbine replaces the traditional **Nginx + PHP-FPM + OPcache** stack with a single binary that embeds PHP directly, eliminating inter-process communication overhead and reducing latency.

## Features

- **Single binary** — no Nginx, no PHP-FPM, no reverse proxy
- **Persistent workers** — process and thread modes with automatic scaling
- **Zero-copy IPC** — in-memory channels for thread mode (ZTS)
- **OWASP security guards** — SQL injection, XSS, path traversal, code injection
- **ACME auto-TLS** — automatic Let's Encrypt certificates
- **OPcache preload** — scripts compiled once, shared across workers
- **Hot reload** — file watcher for development
- **Framework auto-detection** — Laravel, Symfony, Phalcon, WordPress
- **Structured logging** — JSON output, Datadog/Loki compatible
- **Compression** — Brotli, Zstd, Gzip
- **Early Hints** — HTTP 103 support
- **X-Sendfile** — efficient large file delivery
- **App embedding** — pack your PHP app into the binary

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

## Quick start

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
# token = "my-secret"     # protect internal endpoints with Bearer token
```

See [docs/config.md](docs/config.md) for the full configuration reference.

## Worker Modes

> **The most important choice before deploying Turbine.**

Turbine has two worker backends. Choose once at build time — they require different PHP builds.

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

**Thread mode** spawns OS threads sharing one address space. Lower memory, no `pipe(2)` overhead between request dispatch and worker. Requires PHP compiled with `--enable-zts` (use `./build.sh` → *Thread mode (ZTS)*). A PHP segfault takes down all threads.

See [docs/worker.md](docs/worker.md) for the full guide, ZTS extension compatibility, and benchmarks.

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
| Security model | [docs/security.md](docs/security.md) |
| Performance | [docs/performance.md](docs/performance.md) |
| Laravel integration | [docs/laravel.md](docs/laravel.md) |
| TLS & ACME | [docs/tls.md](docs/tls.md) |
| PHP extensions | [docs/extensions.md](docs/extensions.md) |
| Migration from Nginx | [docs/migrate.md](docs/migrate.md) |

## Project structure

```
crates/
  turbine-core/       Main server, CLI, HTTP handling
  turbine-php-sys/    PHP FFI bindings, embed SAPI integration
  turbine-engine/     PHP engine lifecycle management
  turbine-worker/     Worker pool (process & thread modes)
  turbine-security/   OWASP security guards, sandbox
  turbine-metrics/    Performance metrics
  turbine-cache/      Response caching
```

## License

Licensed under the [GNU General Public License v3.0](LICENSE) (GPL-3.0).
