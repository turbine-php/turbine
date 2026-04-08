# Turbine Examples

Ready-to-use examples for running PHP applications on Turbine.

Each example includes a `turbine.toml` configuration file and (where applicable) a `turbine-production.toml` variant with locked-down production settings.

## Quick Start

```bash
# Run any example
cd examples/<example>
turbine --root .

# Or with a specific config
turbine --root . --config turbine-production.toml
```

## Examples

### Raw PHP

Standalone PHP applications without any framework:

| Example | Description |
|---------|-------------|
| [raw-php/hello-world](raw-php/hello-world/) | Simplest possible app — HTML page with query parameters |
| [raw-php/rest-api](raw-php/rest-api/) | JSON REST API with routing and CRUD operations |
| [raw-php/session-auth](raw-php/session-auth/) | Login/logout flow with session handling |
| [raw-php/file-upload](raw-php/file-upload/) | File upload with Turbine's sandbox protections |
| [raw-php/database-crud](raw-php/database-crud/) | SQLite CRUD API with PDO and pagination |
| [raw-php/websocket-sse](raw-php/websocket-sse/) | Server-Sent Events (SSE) real-time streaming |

### Frameworks

| Example | Description |
|---------|-------------|
| [laravel](laravel/) | Laravel configuration with dev and production variants |
| [symfony](symfony/) | Symfony configuration with preloading support |
| [wordpress](wordpress/) | WordPress with upload protection and wp-admin worker pools |
| [phalcon](phalcon/) | Phalcon Micro app with PSR extension setup |

## Configuration Variants

Each framework example includes two configuration files:

- **`turbine.toml`** — Development settings: verbose logging, file watcher, relaxed security
- **`turbine-production.toml`** — Production settings: TLS, auto-scaling, strict security, dashboard token

## Common Patterns

### Development

```toml
[server]
workers = 2
listen = "127.0.0.1:8080"

[logging]
level = "debug"

[watcher]
enabled = true

[dashboard]
enabled = true
statistics = true
```

### Production

```toml
[server]
workers = 0              # Auto-detect CPU cores
listen = "0.0.0.0:8080"
worker_mode = "thread"   # Lower memory usage
auto_scale = true

[server.tls]
enabled = true

[logging]
level = "warn"

[watcher]
enabled = false

[dashboard]
enabled = true
token = "my-secret-token"
```

### Worker Pools

Route specific URL patterns to dedicated worker groups:

```toml
[[worker_pools]]
match_path = "/api/*"
min_workers = 2
max_workers = 8
name = "api"
```

## Documentation

- [Configuration Reference](../docs/config.md)
- [Compilation Guide](../docs/compile.md)
- [Security](../docs/security.md)
