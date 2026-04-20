# Compile from Source

## Requirements

- Rust 1.75+ (`rustup install stable`)
- PHP 8.3, 8.4 or 8.5 source (downloaded automatically)
- macOS: Homebrew
- Linux: apt/yum build dependencies

## Step 1: Build PHP Embed SAPI

The build script downloads and compiles PHP with the embed SAPI:

```bash
./scripts/build-php-embed.sh
```

This installs PHP to `vendor/php-embed/` with:
- `lib/libphp.dylib` (macOS) or `lib/libphp.so` (Linux)
- `bin/php` — PHP CLI binary
- `bin/php-config` — Build configuration tool
- `bin/pecl` — Extension installer

### Build Options

```bash
# Specific PHP version
./scripts/build-php-embed.sh 8.5.4

# Thread-Safe (ZTS) build for multi-threaded mode
ZTS_BUILD=1 ./scripts/build-php-embed.sh 8.5.4
```

### NTS vs ZTS

> **This choice determines which `worker_mode` you can use.** You cannot switch worker mode without recompiling PHP.

| Build | Default | Worker Mode available | OPcache |
|-------|:-------:|----------------------|:--------|
| **NTS** (Non-Thread-Safe) | Yes | `process` only | Shared via SHM |
| **ZTS** (Zend Thread-Safe) | No | `process` **and** `thread` | Shared in-process |

- **NTS** is the safe default. Works with virtually all PHP extensions. Use `worker_mode = "process"`.
- **ZTS** unlocks `worker_mode = "thread"` — OS threads instead of forked processes, lower memory, higher throughput. Requires all loaded extensions to also be compiled as ZTS.

> **Easiest path to ZTS:** run `./build.sh` and select **Thread mode (ZTS)** or **Both**. The script handles everything including PECL extension compilation against the ZTS build.

NTS is recommended for most workloads. ZTS enables true thread-pool mode with zero-syscall IPC but requires all PHP extensions to be thread-safe. See [Worker Mode](worker.md) for the full comparison.

### macOS Dependencies

Installed automatically via Homebrew:

```
autoconf automake bison re2c pkg-config
libxml2 sqlite openssl@3 zlib curl libpng
oniguruma libzip icu4c libiconv
libsodium gmp libffi
libjpeg-turbo webp freetype
```

### Linux Dependencies (Debian/Ubuntu)

```bash
apt-get install -y \
    build-essential autoconf automake bison re2c pkg-config \
    libxml2-dev libsqlite3-dev libssl-dev zlib1g-dev libcurl4-openssl-dev \
    libpng-dev libonig-dev libzip-dev libicu-dev libjpeg-dev \
    libwebp-dev libfreetype6-dev libsodium-dev libgmp-dev libffi-dev
```

## Step 2: Build Turbine

```bash
PHP_CONFIG=$PWD/vendor/php-embed/bin/php-config cargo build --release
```

The binary is produced at `target/release/turbine` (~8.3 MB).

### Build Features

```bash
# With ACME auto-TLS support
cargo build --release --features acme

# With embedded PHP application
TURBINE_EMBED_DIR=./my-app cargo build --release --features embed

# All features
TURBINE_EMBED_DIR=./my-app cargo build --release --features "acme,embed"
```

## Step 3: Run

```bash
# macOS
DYLD_LIBRARY_PATH="$PWD/vendor/php-embed/lib" ./target/release/turbine serve --root ./my-app

# Linux
LD_LIBRARY_PATH="$PWD/vendor/php-embed/lib" ./target/release/turbine serve --root ./my-app
```

## Project Structure

```
rustphp/
├── Cargo.toml                # Workspace root
├── crates/
│   ├── turbine-core/         # Main server binary
│   │   └── src/
│   │       ├── main.rs       # CLI, HTTP server, request pipeline
│   │       ├── config.rs     # Configuration structs and TOML parsing
│   │       ├── features.rs   # Early Hints, X-Sendfile, Structured Logging
│   │       ├── acme.rs       # Let's Encrypt auto-TLS
│   │       └── embed.rs      # Embedded app extraction
│   ├── turbine-php-sys/      # PHP C FFI bindings
│   ├── turbine-engine/       # PHP embed SAPI wrapper
│   ├── turbine-worker/       # Multi-process worker pool
│   ├── turbine-security/     # Sandbox, heuristic input filter, behaviour guard
│   ├── turbine-metrics/      # Prometheus metrics
│   └── turbine-cache/        # Response caching
├── scripts/
│   ├── build-php-embed.sh    # PHP embed build script
│   ├── build-embed.sh        # Alternative embed build
│   ├── build-static-musl.sh  # Static Linux build
│   └── test_server.sh        # Server test script
├── vendor/
│   ├── php-embed/            # Compiled PHP installation
│   └── php-build/            # PHP source (build artifact)
└── test-app/                 # Test PHP application
```

## Running Tests

```bash
PHP_CONFIG=$PWD/vendor/php-embed/bin/php-config cargo test --release
```

## Adding PHP Extensions at Build Time

Edit `scripts/build-php-embed.sh` to add `./configure` flags. See [PHP Extensions](extensions.md) for details.

## Cross-Compilation

### Static Linux Binary (musl)

For portable Linux binaries without shared library dependencies:

```bash
# Requires musl cross-compiler
rustup target add x86_64-unknown-linux-musl
PHP_CONFIG=$PWD/vendor/php-embed/bin/php-config \
    cargo build --release --target x86_64-unknown-linux-musl
```

### Docker Build

```dockerfile
FROM rust:1.91.0 AS builder
COPY . /app
WORKDIR /app
RUN ./scripts/build-php-embed.sh 8.5.4
RUN PHP_CONFIG=/app/vendor/php-embed/bin/php-config cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/turbine /usr/local/bin/
COPY --from=builder /app/vendor/php-embed/lib/libphp.so /usr/local/lib/
RUN ldconfig
ENTRYPOINT ["turbine"]
```
