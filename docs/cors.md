# CORS (Cross-Origin Resource Sharing)

Turbine includes built-in CORS support for APIs consumed by frontend applications on different domains.

## Configuration

```toml
[cors]
enabled = true
allow_origins = ["https://myapp.com", "https://admin.myapp.com"]
allow_credentials = true
allow_methods = ["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS"]
allow_headers = ["Content-Type", "Authorization", "X-Requested-With"]
expose_headers = ["X-Total-Count", "X-Page-Count"]
max_age = 86400
```

## Options

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `false` | Enable CORS headers |
| `allow_origins` | array | `[]` | Allowed origins. Use `["*"]` for any origin |
| `allow_credentials` | bool | `false` | Allow cookies/auth headers |
| `allow_methods` | array | `["GET", "POST", ...]` | Allowed HTTP methods |
| `allow_headers` | array | `["Content-Type", ...]` | Allowed request headers |
| `expose_headers` | array | `[]` | Headers exposed to JavaScript |
| `max_age` | integer | `86400` | Preflight cache duration (seconds) |

## Examples

### Public API (any origin)

```toml
[cors]
enabled = true
allow_origins = ["*"]
```

### Authenticated API

```toml
[cors]
enabled = true
allow_origins = ["https://myapp.com"]
allow_credentials = true
allow_headers = ["Content-Type", "Authorization"]
```

> **Note:** `allow_origins = ["*"]` cannot be combined with `allow_credentials = true`. Specify exact origins when credentials are needed.

### Development (allow localhost)

```toml
[cors]
enabled = true
allow_origins = ["http://localhost:3000", "http://localhost:5173"]
```

## How It Works

1. **Preflight requests** (`OPTIONS`): Turbine responds with CORS headers immediately, without forwarding to PHP
2. **Simple requests**: CORS headers are added to the PHP response
3. **Credentials**: When enabled, `Access-Control-Allow-Credentials: true` is added

This is faster than handling CORS in PHP middleware because preflight requests never reach the PHP worker.
