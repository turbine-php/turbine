# Embed PHP App in Binary

Turbine can embed your entire PHP application into the binary, creating a single-file deployment.

## How It Works

1. At **build time**, the PHP application directory is packed into a tar.gz archive and embedded in the binary via `include_bytes!()`
2. At **runtime**, the archive is extracted to a temporary directory (or configured path)
3. A hash-based marker prevents re-extraction on subsequent startups

## Building

```bash
# Set the directory to embed and enable the feature
TURBINE_EMBED_DIR=./my-laravel-app cargo build --release --features embed
```

The resulting binary contains your PHP application — no separate files needed.

## Configuration

```toml
[embed]
enabled = true
# Custom extraction directory (default: /tmp/turbine-embedded-app)
extract_dir = "/opt/myapp"
```

## Running

```bash
# The binary contains the app — just run it
./turbine serve --listen 0.0.0.0:8080
```

On first run, the app is extracted. Subsequent runs detect the existing extraction via checksum and skip it.

## What to Include

Include your entire deployment-ready application:

```bash
# Laravel example: prepare for production
cd my-laravel-app
composer install --no-dev --optimize-autoloader
php artisan config:cache
php artisan route:cache
php artisan view:cache

# Build the binary
cd ..
TURBINE_EMBED_DIR=./my-laravel-app cargo build --release --features embed
```

## Security

- Path traversal is blocked during extraction
- Symbolic links are skipped
- Archive integrity is verified via xxh3 hash
- Extraction directory is created with standard permissions

## Use Cases

- **Single-binary deployment**: Ship one file instead of a tarball
- **Edge/IoT deployment**: Minimal footprint, no filesystem setup
- **Container optimization**: Smaller Docker images (no COPY of app files)
