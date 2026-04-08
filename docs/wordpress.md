# WordPress

Turbine supports **WordPress** with security hardening and optimized URL routing.

## Setup

Turbine detects the root `index.php` front controller pattern. Configure `turbine.toml`:

```toml
[server]
workers = 4
listen = "0.0.0.0:8080"
worker_max_requests = 10000

[security]
enabled = true
sql_guard = true
code_injection_guard = true

[sandbox]
execution_mode = "strict"
execution_whitelist = ["index.php", "wp-login.php", "wp-cron.php", "wp-signup.php", "wp-comments-post.php", "xmlrpc.php", "wp-admin/admin.php", "wp-admin/admin-ajax.php"]
data_directories = ["wp-content/uploads/", "wp-content/cache/", "wp-content/upgrade/"]
```

## How It Works

WordPress uses Turbine's **native per-request execution mode** (not persistent workers). This is intentional — WordPress plugins frequently use `exit()` and `die()`, which would kill persistent worker processes.

Turbine optimizes WordPress through:

1. **OPcache + JIT** — PHP opcodes are cached and JIT-compiled
2. **Correct URL routing** — Pretty permalinks work automatically
3. **Auto-security** — Upload directories are locked down
4. **Static file serving** — CSS, JS, images served directly by Turbine (no PHP overhead)
5. **Document root** — Automatically set to WordPress root (not `public/`)

## URL Routing

Turbine handles WordPress URLs correctly:

| Request | Resolves To |
|---------|-------------|
| `/` | `index.php` (front controller) |
| `/my-page/` | `index.php` (pretty permalink) |
| `/category/news/` | `index.php` (pretty permalink) |
| `/wp-login.php` | `wp-login.php` (direct) |
| `/wp-admin/admin.php` | `wp-admin/admin.php` (direct) |
| `/wp-admin/admin-ajax.php` | `wp-admin/admin-ajax.php` (direct) |
| `/wp-cron.php` | `wp-cron.php` (direct) |
| `/wp-content/uploads/photo.jpg` | Static file (served directly) |

## Security Auto-Configuration

### Execution Whitelist

Configure an explicit whitelist of all legitimate WordPress PHP entry points in `turbine.toml`:

- **Root files**: `index.php`, `wp-login.php`, `wp-cron.php`, `wp-signup.php`, `wp-comments-post.php`, `xmlrpc.php`, etc.
- **Admin files**: `wp-admin/admin.php`, `wp-admin/admin-ajax.php`, and other admin entry points
- **Blocked**: PHP files uploaded to `wp-content/uploads/` are **never** executed

### Data Directories

Configure these directories as data directories (no PHP execution allowed):

| Directory | Purpose |
|-----------|---------|
| `wp-content/uploads/` | Media uploads |
| `wp-content/cache/` | Cache plugins |
| `wp-content/upgrade/` | Update temp files |

This prevents the most common WordPress attack vector — uploading a malicious PHP file to the uploads directory.

## TOML Configuration

```toml
[server]
workers = 4
listen = "0.0.0.0:8080"
worker_max_requests = 10000

[php]
memory_limit = "256M"
max_execution_time = 300    # WordPress updates can take time
upload_tmp_dir = "/tmp"

[php.ini]
upload_max_filesize = "64M"
post_max_size = "64M"
max_input_vars = "5000"     # WordPress + WooCommerce may need this

[security]
enabled = true
sql_guard = true             # Catches SQL injection in query params
code_injection_guard = true

# Optionally add more data directories
[sandbox]
data_directories = ["wp-content/uploads", "wp-content/cache", "wp-content/upgrade"]
```

## Quick Start

```bash
# Download WordPress
curl -O https://wordpress.org/latest.tar.gz
tar xzf latest.tar.gz
cd wordpress

# Configure wp-config.php (copy from sample)
cp wp-config-sample.php wp-config.php
# Edit wp-config.php with your database credentials

# Start Turbine
turbine serve --root . --workers 4
```

Visit `http://localhost:8080` to complete the WordPress installation wizard.

## Static Files

Turbine serves static files (CSS, JS, images, fonts) directly without invoking PHP. This includes:

- `wp-content/themes/*/` — Theme assets
- `wp-content/plugins/*/` — Plugin assets
- `wp-content/uploads/` — Media files
- `wp-includes/css/`, `wp-includes/js/` — WordPress core assets
- `wp-admin/css/`, `wp-admin/js/` — Admin assets

Static files are served with:
- **ETag** headers for cache validation
- **304 Not Modified** responses
- Correct **MIME types**
- Optional **compression** (Brotli/Gzip)

## Multisite

WordPress Multisite is supported. Turbine's URL routing handles subdirectory multisite installations correctly since all non-file requests route through `index.php`.

For **subdomain** multisite, configure DNS and Turbine's listen address accordingly:

```toml
[server]
listen = "0.0.0.0:8080"

[tls]
enabled = true
cert = "/path/to/wildcard.crt"
key = "/path/to/wildcard.key"
```

## Comparison with Traditional Setup

| Feature | Nginx + PHP-FPM | Turbine |
|---------|-----------------|---------|
| Config files | nginx.conf + php-fpm.conf + wp-config.php | turbine.toml + wp-config.php |
| URL rewriting | `try_files` + `.htaccess` rules | Automatic |
| Upload security | Manual `location` rules | Auto data directory guard |
| Static files | Nginx serves directly | Turbine serves directly |
| OPcache | Separate config | Built-in |
| Process management | systemd + php-fpm | Single binary |
| SSL/TLS | Separate (certbot + nginx) | Built-in ACME |

## Known Limitations

- **No persistent workers** — WordPress uses per-request execution due to widespread use of `exit()`/`die()` in plugins. Performance comes from OPcache + JIT instead.
- **xmlrpc.php** — Whitelisted by default. Disable in TOML if not needed:
  ```toml
  [sandbox]
  execution_mode = "strict"
  execution_whitelist = ["index.php", "wp-login.php", "wp-cron.php", "wp-admin/admin.php", "wp-admin/admin-ajax.php"]
  ```
- **WP-CLI** — Use `php wp-cli.phar` directly, not through Turbine (Turbine is an HTTP server, not a CLI runner).
