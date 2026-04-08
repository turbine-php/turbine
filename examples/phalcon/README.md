# Phalcon on Turbine

This example demonstrates how to run a Phalcon PHP application on Turbine.

## Requirements

- PHP 8.1+ with the Phalcon extension (`phalcon.so`)
- PSR extension (`php-psr`) — required by Phalcon 5+

## Quick Start

```bash
# 1. Install Phalcon via PECL (or use Turbine's build.sh)
pecl install psr
pecl install phalcon

# 2. Create a new Phalcon project (or use an existing one)
# Using Phalcon DevTools:
phalcon create-project myapp
cd myapp

# 3. Copy the Turbine configuration
cp /path/to/examples/phalcon/turbine.toml .

# 4. Run with Turbine
turbine --root .
```

Alternatively, use Turbine's `build.sh` to compile PHP with Phalcon included:

```bash
./build.sh
# Select "Phalcon" in the PECL extensions checkbox
```

## How It Works

Turbine auto-detects Phalcon's entry point (`public/index.php`) when `execution_mode = "framework"`. The Phalcon extension is loaded via the `extensions` config.

### Key Configuration Points

- **`extensions = ["psr.so", "phalcon.so"]`** — Phalcon requires both PSR and Phalcon extensions.
- **`execution_mode = "framework"`** — Auto-detects `public/index.php`.
- **`preload_script = "auto"`** — Turbine can preload Phalcon's classes for faster cold starts.
- **`memory_limit = "256M"`** — Phalcon is efficient, but complex apps may need more.

### Phalcon Micro vs Full

Both Phalcon Micro and full MVC applications work with the same Turbine configuration — the entry point is always `public/index.php`.

### Volt Templates

Phalcon's Volt template engine compiles to PHP files. Ensure the compiled templates directory is writable:

```toml
[sandbox]
data_directories = ["cache/", "public/uploads/"]
```

### Phalcon Models & Database

Phalcon's ORM uses PDO internally. Turbine's `sql_guard` provides an additional layer of SQL injection protection on top of Phalcon's parameter binding.

## Sample App

This example includes a minimal Phalcon Micro application to get you started:

```
phalcon/
├── turbine.toml            # Turbine config (dev)
├── turbine-production.toml # Turbine config (prod)
├── public/
│   └── index.php           # Phalcon Micro app
├── app/
│   └── config.php          # Phalcon config
└── cache/                  # Volt compiled templates
```

## File Structure (Full MVC)

```
myapp/
├── turbine.toml
├── public/
│   └── index.php       # Entry point
├── app/
│   ├── config/
│   ├── controllers/
│   ├── models/
│   ├── views/
│   └── migrations/
├── cache/
└── ...
```

## Production Tips

1. **Phalcon + OPcache**: Phalcon compiled classes are already in C, but your app code benefits from OPcache.
2. **Thread mode**: Phalcon works with ZTS PHP. Use `worker_mode = "thread"` for lower memory usage.
3. **Volt cache**: Pre-compile Volt templates in your deployment pipeline.
4. **PSR extension**: Always load `psr.so` **before** `phalcon.so` in the extensions list.
5. **Disabled functions**: Phalcon doesn't need `exec()` family — keep them disabled.
