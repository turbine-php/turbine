# WordPress on Turbine

This example demonstrates how to run WordPress on Turbine.

## Requirements

- PHP 8.0+ with extensions: mysqli (or pdo_mysql), gd, mbstring, xml, curl, zip, intl
- MySQL/MariaDB database server

## Quick Start

```bash
# 1. Download WordPress
curl -O https://wordpress.org/latest.tar.gz
tar xzf latest.tar.gz
cd wordpress

# 2. Copy the Turbine configuration
cp /path/to/examples/wordpress/turbine.toml .

# 3. Set up wp-config.php
cp wp-config-sample.php wp-config.php
# Edit wp-config.php with your database credentials

# 4. Run with Turbine
turbine --root .
```

## How It Works

Turbine treats `index.php` in the project root as the entry point. WordPress's `.htaccess` rewrite rules are unnecessary — Turbine routes all requests that don't match a static file to `index.php` automatically.

### Key Configuration Points

- **`execution_mode = "framework"`** — Auto-detects `index.php` as the entry point.
- **`block_url_fopen = false`** — WordPress needs `allow_url_fopen` for plugin/theme updates.
- **`disabled_functions`** — Relaxed to allow WordPress cron, plugin installation, and updates.
- **`data_directories`** — Protects `wp-content/uploads/` from direct PHP execution (shell upload prevention).

### WordPress Multisite

For multisite installations, no additional Turbine configuration is needed. Turbine handles the routing internally through `index.php`.

### Plugins & Themes

Most WordPress plugins and themes work without modification. However:

- **Security plugins** (Wordfence, etc.): Turbine handles security at the server level. You can still use them, but there may be overlap.
- **Cache plugins** (W3 Total Cache, WP Super Cache): Turbine has its own response cache. Consider using Turbine's cache instead.
- **File manager plugins**: May be restricted by `disabled_functions` and sandbox settings.

### wp-cron

WordPress's built-in cron runs via HTTP requests. For production, disable `wp-cron.php` and use a system cron:

```php
// In wp-config.php
define('DISABLE_WP_CRON', true);
```

```bash
# System crontab
*/5 * * * * curl -s http://localhost:8080/wp-cron.php > /dev/null 2>&1
```

## File Structure

```
wordpress/              # WordPress root
├── turbine.toml        # Turbine configuration (this example)
├── index.php           # WordPress entry point
├── wp-config.php       # Database & site configuration
├── wp-admin/
├── wp-content/
│   ├── themes/
│   ├── plugins/
│   └── uploads/        # Protected from PHP execution by Turbine
├── wp-includes/
└── ...
```

## Production Tips

1. **File permissions**: Ensure `wp-content/uploads/` is writable but protected by Turbine's sandbox.
2. **Database**: Use a dedicated MySQL/MariaDB instance. Enable `sql_guard` for extra protection.
3. **Updates**: WordPress needs network access for updates. Keep `block_url_fopen = false`.
4. **PHP memory**: WordPress + plugins can be memory-hungry. Set `memory_limit = "512M"` if needed.
5. **Rate limiting**: Protect `wp-login.php` and `xmlrpc.php` with Turbine's rate limiter.
