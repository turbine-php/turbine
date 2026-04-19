# Symfony on Turbine

This example demonstrates how to run a Symfony application on Turbine.

## Requirements

- PHP 8.2+ with extensions: intl, mbstring, xml, ctype, iconv, pdo_sqlite (or pdo_mysql)
- Composer

## Quick Start

```bash
# 1. Create a new Symfony project
composer create-project symfony/skeleton myapp
cd myapp

# 2. (Optional) Add the web application pack
composer require webapp

# 3. Copy the Turbine configuration
cp /path/to/examples/symfony/turbine.toml .

# 4. Run with Turbine
turbine --root .
```

## How It Works

Turbine auto-detects Symfony's entry point (`public/index.php`) when `execution_mode = "framework"`. No changes to Symfony's front controller are needed.

### Key Differences from Apache/Nginx

1. **No `.htaccess`** — Turbine routes all non-static requests to `public/index.php` automatically.
2. **Built-in compression** — No need for `mod_deflate` or Nginx gzip config.
3. **Built-in TLS** — Direct HTTPS without a reverse proxy.
4. **OPcache preloading** — Use `preload_script = "auto"` or point to Symfony's `config/preload.php`.

### Sessions

Symfony's session handling works seamlessly. You can use:

- `framework.session.handler_id: session.handler.native_file` — uses Turbine's session path
- `framework.session.handler_id: ~` (null) — uses PHP's default handler (managed by Turbine)
- Redis, Memcached, or database handlers — requires corresponding PHP extensions

### Symfony Messenger

For background processing with Symfony Messenger, use dedicated worker pools:

```toml
[[worker_pools]]
match_path = "/api/async/*"
min_workers = 2
max_workers = 8
name = "async-api"
```

## File Structure

```
myapp/                  # Your Symfony app root
├── turbine.toml        # Turbine configuration (this example)
├── public/
│   └── index.php       # Symfony front controller (auto-detected)
├── src/
├── config/
├── templates/
├── var/
└── ...
```

## Production Tips

1. **Preloading**: Use `preload_script = "config/preload.php"` for Symfony's built-in preloader.
2. **Environment**: Set `APP_ENV=prod` and `APP_DEBUG=0` in `.env.local`.
3. **DEFAULT_URI** (persistent workers): Symfony's router needs `DEFAULT_URI` when the kernel is booted without a live request context. Add `DEFAULT_URI=http://localhost/` (or your canonical URL) to `.env.local` to avoid `EnvNotFoundException: "Environment variable not found: DEFAULT_URI"` on every request.
4. **Cache warmup**: Run `php bin/console cache:warmup --env=prod` before starting Turbine.
5. **Var directory**: Ensure `var/cache/` and `var/log/` are writable.
6. **Disabled functions**: If you use `Process` component, remove `proc_open` from `disabled_functions`.
