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

Turbine's persistent workers keep PHP processes alive across requests. Laravel still bootstraps on every request (`public/index.php` runs in full), but because the workers are long-lived processes, **OPcache stays warm** — all PHP files are compiled once and cached in memory. There is no per-request `fork()` overhead, and JIT-compiled code is reused across requests.

### Traditional PHP-FPM (pm = dynamic, cold workers)

Every request on a cold worker:
1. Parse and compile PHP files from disk
2. Load Composer autoloader
3. Create Application instance and boot service providers
4. Handle request

### Turbine Persistent Mode

Every request (warm worker):
1. OPcache serves compiled opcodes — no disk reads, no recompilation
2. Load Composer autoloader (from OPcache)
3. Create Application instance and boot service providers (from OPcache)
4. Handle request

The PHP application itself (Application instance, service providers, routes) is **not** persisted across requests. Each request gets a clean PHP state via `php_request_startup`/`php_request_shutdown`. The performance gain comes from OPcache hit rate, warm JIT, and eliminating process fork overhead.

## Turbine vs Laravel Octane

Laravel Octane (Swoole/RoadRunner) keeps the Laravel Application instance alive across requests — service providers boot **once** and the DI container is reused per request. This is a fundamentally different model from Turbine.

Turbine runs each request through the full Laravel bootstrap, accelerated by OPcache. No code changes or additional packages are required.

| Feature | Turbine | Octane (Swoole) | Octane (RoadRunner) |
|---------|---------|-----------------|---------------------|
| Language | Rust | C (PHP extension) | Go |
| Requires package | No | Yes | Yes |
| Code changes | None | Octane-compatible code | Octane-compatible code |
| App bootstrap | Per request (OPcache) | Once per worker | Once per worker |
| Security guards | Built-in | None | None |
| Compression | Built-in (br/zstd/gzip) | Manual | Manual |
| Auto-TLS | Built-in (ACME) | No | No |
| Hot reload | Built-in | `--watch` flag | `--watch` flag |
| Memory management | Auto-respawn | Manual `octane:reload` | Manual |

## Database Connections

Turbine calls `php_request_shutdown` after every request, which destroys all PHP objects including PDO connection handles. Database connections are **not** kept alive between requests unless you use PHP's native persistent connections (`PDO::ATTR_PERSISTENT = true`).

For standard Laravel configuration, no special changes are needed:

```env
# .env
DB_CONNECTION=mysql
DB_HOST=127.0.0.1
DB_PORT=3306
DB_DATABASE=myapp
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

Turbine's persistent workers eliminate per-request `fork()` overhead and keep OPcache warm, resulting in faster response times compared to cold PHP-FPM setups.

Run your own benchmarks:

```bash
./target/release/turbine serve --root ./my-laravel-app --workers 8
wrk -t4 -c50 -d15s http://127.0.0.1:8080/
```
