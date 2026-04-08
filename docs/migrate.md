# Migrate from PHP-FPM

This guide helps you migrate from a traditional Nginx + PHP-FPM stack to Turbine.

## Architecture Comparison

```
Traditional:  Client → Nginx → FastCGI → PHP-FPM pool → OPcache → PHP
Turbine:      Client → Turbine (HTTP + PHP + OPcache + Security)
```

Turbine replaces **both** Nginx and PHP-FPM with a single binary.

## Configuration Mapping

### PHP-FPM → Turbine

| PHP-FPM (pool.d/www.conf) | Turbine (turbine.toml) |
|----------------------------|------------------------|
| `pm.max_children = 10` | `[server] workers = 10` |
| `pm.start_servers = 4` | `[server] workers = 4` |
| `pm = dynamic` | `[server] auto_scale = true` |
| `pm.min_spare_servers = 2` | `[server] min_workers = 2` |
| `pm.max_spare_servers = 8` | `[server] max_workers = 8` |
| `pm.max_requests = 500` | `[server] worker_max_requests = 500` |
| `request_terminate_timeout = 30` | `[server] request_timeout = 30` |
| `listen = /run/php/fpm.sock` | `[server] listen = "0.0.0.0:8080"` |

### Nginx → Turbine

| Nginx Feature | Turbine Equivalent |
|---------------|-------------------|
| `gzip on` | `[compression] enabled = true` |
| `ssl_certificate` | `[server.tls] cert_file` |
| `ssl_certificate_key` | `[server.tls] key_file` |
| `limit_req zone=api` | `[security] max_requests_per_second` |
| `add_header X-Frame-Options` | `[php.ini]` or PHP code |
| `location ~ \.php$` | `[sandbox] execution_mode` |
| `try_files $uri /index.php` | Automatic (framework mode) |
| `access_log` | `[logging] access_log` |

### php.ini → Turbine

| php.ini Directive | Turbine Configuration |
|-------------------|----------------------|
| `memory_limit = 256M` | `[php] memory_limit = "256M"` |
| `max_execution_time = 30` | `[php] max_execution_time = 30` |
| `upload_max_filesize = 64M` | `[php] upload_max_filesize = "64M"` |
| `post_max_size = 64M` | `[php] post_max_size = "64M"` |
| `opcache.memory_consumption = 128` | `[php] opcache_memory = 128` |
| `opcache.jit_buffer_size = 64M` | `[php] jit_buffer_size = "64M"` |
| `session.save_path` | `[session] save_path` |
| `disable_functions = exec,...` | `[sandbox] disabled_functions` |
| `open_basedir = /var/www` | `[sandbox] enforce_open_basedir = true` |
| Custom directives | `[php.ini] key = "value"` |

## Migration Steps

### 1. Install Turbine

```bash
# Build PHP embed SAPI
./scripts/build-php-embed.sh

# Build Turbine
PHP_CONFIG=$PWD/vendor/php-embed/bin/php-config cargo build --release
```

### 2. Create Configuration

```bash
# Generate default turbine.toml
./target/release/turbine init
```

Edit `turbine.toml` to match your current PHP-FPM and Nginx settings.

### 3. Test

```bash
# Start Turbine
DYLD_LIBRARY_PATH="$PWD/vendor/php-embed/lib" \
  ./target/release/turbine serve --root /var/www/myapp

# Test a request
curl http://localhost:8080/
```

### 4. Benchmark Comparison

```bash
# Test PHP-FPM
wrk -t4 -c50 -d15s http://localhost:80/

# Test Turbine
wrk -t4 -c50 -d15s http://localhost:8080/
```

### 5. Deploy

Replace Nginx + PHP-FPM with Turbine. If using a reverse proxy (e.g., Cloudflare, load balancer), point it to Turbine's port.

## What You Gain

| Feature | PHP-FPM + Nginx | Turbine |
|---------|:---------------:|:-------:|
| Persistent workers | No | Yes |
| Worker mode choice | Process only | Process **or** Thread (ZTS) |
| Built-in security guards | No | Yes |
| Built-in compression | Nginx config | Automatic |
| Auto-scaling workers | Limited | Yes |
| ACME auto-TLS | Certbot cron | Built-in |
| Single binary deployment | No | Yes (embed) |
| Process count | 3+ (Nginx + FPM master + workers) | 1 (Turbine) |

> **Worker mode:** PHP-FPM is always process-based. Turbine defaults to process mode too (`worker_mode = "process"`), so the migration is drop-in. You can optionally upgrade to thread mode later if you need higher throughput — see [Worker Mode](worker.md).

## What Changes

- **No `.htaccess`**: Turbine doesn't read Apache/Nginx config files. Use `turbine.toml`.
- **No FastCGI**: PHP runs inside Turbine, not as a separate process pool.
- **No socket files**: Turbine binds TCP ports directly.
- **Single config file**: No separate `nginx.conf`, `php-fpm.conf`, `php.ini` — everything is in `turbine.toml`.

## What Stays the Same

- Your PHP code works unchanged
- Composer dependencies work as-is
- Laravel/Symfony applications work without modifications
- PHP extensions work the same way
- Session files are compatible
