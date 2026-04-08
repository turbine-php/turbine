# Laravel on Turbine

This example demonstrates how to run a Laravel application on Turbine.

## Requirements

- PHP 8.2+ with extensions: mbstring, openssl, pdo_sqlite (or pdo_mysql), tokenizer, xml, ctype, json, bcmath
- Composer

## Quick Start

```bash
# 1. Create a new Laravel project (or use an existing one)
composer create-project laravel/laravel myapp
cd myapp

# 2. Copy the Turbine configuration
cp /path/to/examples/laravel/turbine.toml .

# 3. Run with Turbine
turbine --root .
```

## How It Works

Turbine auto-detects Laravel's entry point (`public/index.php`) when `execution_mode = "framework"`. No code changes are needed — your existing Laravel app works as-is.

### Key Configuration Points

- **`execution_mode = "framework"`** — Turbine finds `public/index.php` automatically.
- **`preload_script = "auto"`** — Turbine generates an OPcache preload script for Laravel's vendor files.
- **`data_directories`** — `storage/` and `bootstrap/cache/` are protected from direct PHP execution.
- **`disabled_functions`** — Customize if your app uses `exec()`, `proc_open()`, etc. (e.g., for Artisan).

### Sessions

Turbine handles PHP sessions natively. For Laravel's session driver, you can use:

- `file` — works out of the box with Turbine's session config
- `database` — requires a database connection
- `redis` — requires the Redis PHP extension
- `cookie` — works without changes

### Worker Pools

Use `[[worker_pools]]` to route heavy API endpoints to dedicated workers:

```toml
[[worker_pools]]
match_path = "/api/reports/*"
min_workers = 1
max_workers = 4
name = "heavy-reports"
```

## File Structure

```
myapp/                  # Your Laravel app root
├── turbine.toml        # Turbine configuration (this example)
├── public/
│   └── index.php       # Laravel entry point (auto-detected)
├── app/
├── config/
├── routes/
├── storage/
└── ...
```

## Production Tips

1. **OPcache preloading**: Set `preload_script = "auto"` to let Turbine generate an optimized preloader.
2. **Environment**: Set `APP_ENV=production` and `APP_DEBUG=false` in `.env`.
3. **TLS**: Enable `[server.tls]` or use a reverse proxy (Nginx, Caddy).
4. **Workers**: Start with `workers = 0` (auto-detect) and tune based on load.
5. **Rate limiting**: Turbine's built-in rate limiter complements Laravel's ThrottleRequests middleware.
