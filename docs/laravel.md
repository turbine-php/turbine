# Laravel Integration

Turbine works with Laravel applications out of the box. Configure persistent workers to bootstrap the framework once and serve thousands of requests without re-initialization.

## Quick Start

```bash
# From your Laravel project root
DYLD_LIBRARY_PATH="/path/to/vendor/php-embed/lib" \
  turbine serve --root . --workers 8
```

Turbine detects the `public/index.php` front controller pattern, then:
1. Sets document root to `public/`
2. Routes all requests to `public/index.php`
3. Uses persistent workers if `persistent_workers = true` in config

## Configuration

Create `turbine.toml` in your Laravel project root:

```toml
[server]
workers = 8
listen = "0.0.0.0:8080"
persistent_workers = true
request_timeout = 30
worker_max_requests = 10000

[php]
memory_limit = "256M"
# preload_script = "vendor/preload.php"

[security]
enabled = true

[sandbox]
execution_mode = "framework"
# Laravel data directories — no PHP execution allowed
data_directories = ["storage/", "public/uploads/"]

[session]
save_path = "storage/framework/sessions"
cookie_secure = false  # Set true if using HTTPS

[watcher]
# Enable for local development
enabled = false
paths = ["app/", "config/", "routes/", "resources/views/"]
extensions = ["php", "env", "blade.php"]
debounce_ms = 500

[logging]
level = "info"
```

## How Persistent Workers Work with Laravel

### Traditional PHP-FPM

Every request:
1. Load Composer autoloader (~5ms)
2. Create Application instance (~10ms)
3. Boot service providers (~15ms)
4. Load config, routes, middleware (~10ms)
5. Handle request (~5ms)
6. **Total: ~45ms**

### Turbine Persistent Mode

First request only:
1. Load Composer autoloader (once)
2. Create Application instance (once)
3. Boot service providers (once)
4. Load config, routes, middleware (once)

Every subsequent request:
1. Handle request (~5ms)
2. Reset superglobals
3. **Total: ~5ms**

This persistent worker model eliminates framework boot overhead from every request, resulting in significantly lower latency.

## Turbine vs Laravel Octane

Laravel Octane is a first-party package that provides persistent worker mode via Swoole or RoadRunner. Turbine achieves the same benefit without requiring Octane:

| Feature | Turbine | Octane (Swoole) | Octane (RoadRunner) |
|---------|---------|-----------------|---------------------|
| Language | Rust | C (PHP extension) | Go |
| Requires package | No | Yes | Yes |
| Code changes | None | Octane-compatible code | Octane-compatible code |
| Security guards | Built-in | None | None |
| Compression | Built-in (br/zstd/gzip) | Manual | Manual |
| Auto-TLS | Built-in (ACME) | No | No |
| Hot reload | Built-in | `--watch` flag | `--watch` flag |
| Memory management | Auto-respawn | Manual `octane:reload` | Manual |

### Key Difference

With Turbine, you don't need to install any additional packages or modify your Laravel code. Your existing Laravel application works as-is. Turbine handles the persistent worker lifecycle externally.

## Database Connections

Turbine's persistent workers keep database connections alive between requests. Laravel's connection pooling handles this automatically via the `DB` facade.

For optimal performance with persistent workers:

```env
# .env
DB_CONNECTION=mysql
DB_HOST=127.0.0.1
DB_PORT=3306
DB_DATABASE=myapp

# Important for persistent workers:
DB_POOL_SIZE=5
```

## Static Files

Turbine serves static files from `public/` automatically with:
- ETag headers for 304 Not Modified responses
- Content-type detection
- Response compression (if enabled)

No additional configuration needed for CSS, JS, images, or other assets.

## Queues and Artisan

Turbine handles HTTP requests only. For queue workers and artisan commands, use the standard PHP CLI:

```bash
# Queue worker (separate process)
php artisan queue:work

# Scheduled tasks
php artisan schedule:run
```

## Development Setup

For local development with hot reload:

```toml
[server]
workers = 2
listen = "127.0.0.1:8000"

[watcher]
enabled = true
paths = ["app/", "config/", "routes/", "resources/views/"]
extensions = ["php", "env", "blade.php"]

[security]
enabled = false

[logging]
level = "debug"
```

## Production Deployment

```toml
[server]
workers = 0  # Auto-detect CPU cores
listen = "0.0.0.0:8080"
persistent_workers = true
worker_max_requests = 10000

[php]
memory_limit = "512M"
opcache_memory = 256
jit_buffer_size = "128M"

[security]
enabled = true

[compression]
enabled = true
algorithms = ["br", "zstd", "gzip"]

[logging]
level = "warn"

[watcher]
enabled = false
```

## Benchmarks: Laravel

Turbine's persistent worker model eliminates framework boot overhead, resulting in significantly faster response times compared to traditional PHP-FPM setups.

Run your own benchmarks:

```bash
./target/release/turbine serve --root ./my-laravel-app --workers 8
wrk -t4 -c50 -d15s http://127.0.0.1:8080/
```
