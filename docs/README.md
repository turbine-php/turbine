# Turbine Documentation

Turbine is a high-performance PHP application server written in Rust. It embeds PHP directly via the embed SAPI, eliminating the overhead of PHP-FPM, Nginx, and inter-process communication.

## Architecture

Turbine replaces the traditional stack (Nginx + PHP-FPM + OPcache) with a single binary:

```
Traditional:  Client → Nginx → PHP-FPM → OPcache → PHP
Turbine:      Client → Turbine (HTTP + PHP + OPcache)
```

## Worker Modes — The Core Choice

Before anything else, choose your worker backend and execution model. These are the two most important architectural decisions in Turbine:

| Mode | Config | PHP Build | IPC | Best For |
|------|--------|-----------|-----|----------|
| **Process per-request** (default) | `worker_mode = "process"` | NTS or ZTS | Pipes | Safety, legacy extensions, WordPress |
| **Process persistent** | `worker_mode = "process"` + `persistent_workers = true` | NTS or ZTS | Pipes | Framework apps (Laravel, Symfony) |
| **Thread per-request** | `worker_mode = "thread"` | **ZTS only** | **Channels (zero syscalls)** | Maximum throughput, simple APIs |
| **Thread persistent** | `worker_mode = "thread"` + `persistent_workers = true` | **ZTS only** | Pipes | Framework apps on ZTS PHP |

- **Process mode** is the default. Zero configuration required, works with any PHP extension, crash in one worker does not affect others.
- **Thread mode** requires PHP compiled with `--enable-zts`. Lower memory, but all workers share one process — a fatal crash takes down all threads.
- **Per-request** (`persistent_workers = false`): Each request runs `php_execute_script` with full lifecycle. Still benefits from embedded SAPI and OPcache (faster than PHP-FPM).
- **Persistent** (`persistent_workers = true`): Workers bootstrap once and handle thousands of requests. Eliminates framework boot overhead from every request.

See [Worker Mode](worker.md) for the full guide, benchmarks, and decision flow.

## Key Features

- **Persistent Workers** — PHP bootstraps once (Laravel, Symfony, Phalcon), serves thousands of requests without re-initialization. Graceful recycling via `0xFF` shutdown signal ensures zero errors during respawn.
- **Two Worker Backends** — Process mode (default, any PHP) or Thread mode (ZTS, maximum throughput)
- **Configurable Persistent Mode** — `persistent_workers = true` enables bootstrap-once execution; omit for per-request mode
- **Tokio Async Tuning** — `tokio_worker_threads` controls async I/O threads for optimal HTTP connection handling
- **Config-Driven** — Everything configured via `turbine.toml`, no framework auto-detection magic
- **Built-in HTTP/1.1 & HTTP/2** — Powered by Hyper (Rust), no external web server needed
- **Embedded PHP 8.4/8.5** — Native embed SAPI, shared OPcache, JIT compilation
- **OWASP Security Guards** — SQL injection, code injection, path traversal, and rate limiting built into the request pipeline
- **Brotli, Zstd, Gzip Compression** — Automatic response compression with configurable algorithms
- **Auto-scaling Workers** — Dynamic worker pool that scales up/down based on load
- **ACME Auto-TLS** — Automatic Let's Encrypt certificate provisioning and renewal
- **Early Hints (103)** — Native HTTP 103 support for faster page loads
- **X-Sendfile / X-Accel-Redirect** — Efficient large file serving
- **Hot Reload** — File watcher with automatic worker restart during development
- **Structured Logging** — JSON logs from PHP via `turbine_log()`, compatible with Datadog, Loki, Elastic
- **OPcache Preload** — Configurable preload scripts for warm startup
- **Embed App in Binary** — Pack your PHP app into the Turbine binary for single-file deployment
- **Named Worker Pools** — Route-based worker pool splitting for different workloads
- **CORS** — Built-in Cross-Origin Resource Sharing configuration
- **Response Cache** — In-memory caching with TTL

## Quick Start

```bash
# Build PHP embed SAPI
./scripts/build-php-embed.sh

# Build Turbine
PHP_CONFIG=$PWD/vendor/php-embed/bin/php-config cargo build --release

# Generate default config
./target/release/turbine init

# Start server
DYLD_LIBRARY_PATH="$PWD/vendor/php-embed/lib" ./target/release/turbine serve --root ./my-app
```

## CLI Commands

```bash
turbine init                    # Generate default turbine.toml
turbine check [OPTIONS]         # Validate turbine.toml (errors + warnings)
turbine serve [OPTIONS]         # Start the server
turbine config                  # Display current configuration
turbine info                    # Show PHP engine information
turbine status [ADDRESS]        # Query running server status
turbine cache-clear [ADDRESS]   # Clear response cache
```

### Check Options

| Flag | Description | Example |
|------|-------------|----------|
| `--config` | Path to turbine.toml | `--config /etc/turbine.toml` |

`turbine check` validates the configuration file and reports:

- **Errors** (exit code 1): problems that will prevent Turbine from running correctly — invalid `worker_mode`, TLS enabled without certificate files, `strict` mode without whitelist, worker pool `min_workers > max_workers`.
- **Warnings** (exit code 0): suboptimal settings — persistent workers without recycling, all security guards disabled, compression level out of range, ACME + manual TLS conflict, etc.

Example output:

```
Turbine Configuration Check
  File: turbine.toml

Settings:
  workers          = 4
  worker_mode      = process
  persistent       = true
  listen           = 127.0.0.1:8080
  request_timeout  = 30s
  max_requests     = 10000
  security         = true
  compression      = true
  cache            = false
  tls              = false

✓ Configuration is valid. No errors or warnings.
```

### Serve Options

| Flag | Description | Example |
|------|-------------|---------|
| `--listen` | Address to bind | `--listen 0.0.0.0:8080` |
| `--workers` | Number of worker processes | `--workers 8` |
| `--config` | Path to turbine.toml | `--config /etc/turbine.toml` |
| `--root` | Application root directory | `--root ./my-laravel-app` |
| `--tls-cert` | Path to TLS certificate | `--tls-cert cert.pem` |
| `--tls-key` | Path to TLS private key | `--tls-key key.pem` |
| `--request-timeout` | Request timeout in seconds | `--request-timeout 60` |
| `--access-log` | Path to access log file | `--access-log /var/log/access.log` |

## Documentation Index

| Document | Description |
|----------|-------------|
| [Configuration](config.md) | Complete `turbine.toml` reference |
| [**Worker Mode**](worker.md) | **Process vs Thread — the most important choice. Persistent workers, auto-scaling, named pools, Tokio tuning** |
| [PHP Extensions](extensions.md) | Adding static and dynamic PHP extensions |
| [Security](security.md) | OWASP guards, sandbox, rate limiting |
| [Phalcon](phalcon.md) | Phalcon native support with persistent workers |
| [WordPress](wordpress.md) | WordPress native support with auto-security |
| [Laravel](laravel.md) | Laravel integration and Octane comparison |
| [Performance](performance.md) | Benchmarks, tuning, optimization |
| [Compile from Source](compile.md) | Building PHP embed and Turbine |
| [Early Hints](early-hints.md) | HTTP 103 Early Hints support |
| [X-Sendfile](x-sendfile.md) | Efficient file serving |
| [Structured Logging](logging.md) | JSON logging from PHP |
| [Compression](compression.md) | Brotli, Zstd, Gzip configuration |
| [Hot Reload](hot-reload.md) | File watching for development |
| [TLS & ACME](tls.md) | HTTPS, Let's Encrypt auto-provisioning |
| [**Virtual Hosting**](virtual-hosts.md) | **Multiple domains on one server, SNI, ACME auto-TLS** |
| [Embed](embed.md) | Pack PHP app into Turbine binary |
| [CORS](cors.md) | Cross-Origin Resource Sharing |
| [Migrate from PHP-FPM](migrate.md) | Migration guide from traditional stacks |
| [Known Issues](known-issues.md) | Compatibility notes and limitations |

## Requirements

- Rust 1.75+ (for building)
- PHP 8.4+ source (for embed SAPI compilation)
- macOS (Apple Silicon/Intel) or Linux (x86_64/aarch64)

## License

Licensed under the [GNU General Public License v3.0](../LICENSE) (GPL-3.0).
