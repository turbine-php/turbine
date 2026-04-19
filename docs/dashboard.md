# Dashboard & Internal API

Turbine ships with a built-in web dashboard and a set of internal management endpoints under the `/_/` prefix. These endpoints are protected by an optional Bearer token and are never routed to PHP workers.

## Dashboard UI — `/_/dashboard`

Open `http://<listen>/_/dashboard` in your browser after starting the server.

The dashboard is a single self-contained HTML page that auto-refreshes every 2 seconds. No external dependencies — everything is embedded in the binary.

### Panels

| Panel | Refresh | Description |
|-------|---------|-------------|
| **Requests** | 2 s | Total requests and current requests/sec |
| **Latency** | 2 s | Mean, p50, p99 in ms |
| **Cache Hit Ratio** | 2 s | Response cache hits vs misses |
| **Security Blocks** | 2 s | Cumulative number of requests blocked by the sandbox and heuristic filters |
| **Workers** | 2 s | Active worker count |
| **Bytes Out** | 2 s | Total bytes sent |
| **Endpoints** | 2 s | Per-path breakdown: requests, errors, mean, p99, relative load bar |
| **Status Codes** | 2 s | 2xx / 4xx / 5xx counters |
| **Blocked IPs** | 5 s | IPs currently banned by the Behaviour Guard, with expiry countdown and per-IP unblock button |

### Authentication in the UI

When `[dashboard] token` is set in `turbine.toml`, the dashboard JavaScript automatically attaches the token as an `Authorization: Bearer <token>` header on all API calls. There is no login screen — access the dashboard only from trusted networks or via a reverse proxy, or use `token` to restrict public access.

---

## Internal API Reference

All endpoints live under the `/_/` namespace. Requests to `/_/` paths are handled entirely inside Turbine and are **never forwarded to PHP workers**.

### Authentication

When `[dashboard] token` is configured, every `/_/` request must include:

```
Authorization: Bearer <token>
```

Requests without a valid header receive `401 Unauthorized`. Query-parameter auth (`?token=…`) is **not** supported.

---

### `GET /_/status`

Returns a JSON snapshot of runtime metrics.

**Requires**: `[dashboard] statistics = true`

```bash
curl http://127.0.0.1:8080/_/status \
  -H "Authorization: Bearer my-secret"
```

**Response** (`200 OK`):
```json
{
  "uptime_seconds": 3712,
  "total_requests": 184920,
  "requests_per_second": 312.4,
  "workers": 4,
  "bytes_out": 2147483648,
  "latency_ms": { "mean": 2.31, "p50": 1.80, "p99": 12.40 },
  "cache": { "hits": 91200, "misses": 3600, "hit_ratio": 0.962 },
  "security": { "blocks": 42 },
  "status_codes": { "2xx": 183100, "4xx": 1780, "5xx": 40 },
  "endpoints": [
    { "path": "/", "requests": 120000, "errors": 5, "mean_ms": 1.8, "p99_ms": 8.2 },
    { "path": "/api/users", "requests": 64920, "errors": 35, "mean_ms": 3.1, "p99_ms": 14.6 }
  ]
}
```

---

### `GET /_/metrics`

Returns metrics in **Prometheus text format**.

**Requires**: `[dashboard] statistics = true`

```bash
curl http://127.0.0.1:8080/_/metrics \
  -H "Authorization: Bearer my-secret"
```

**Response** (`200 OK`, `text/plain; version=0.0.4`):
```
# HELP turbine_requests_total Total HTTP requests handled
# TYPE turbine_requests_total counter
turbine_requests_total 184920
...
```

---

### `GET /_/security/blocked`

Returns the list of IPs currently banned by the [Behaviour Guard](security.md#behaviour-guard--ip-banning).

```bash
curl http://127.0.0.1:8080/_/security/blocked \
  -H "Authorization: Bearer my-secret"
```

**Response** (`200 OK`):
```json
{
  "blocked": [
    { "ip": "203.0.113.42", "expires_in_secs": 547 },
    { "ip": "198.51.100.7",  "expires_in_secs": null }
  ],
  "count": 2
}
```

| Field | Description |
|-------|-------------|
| `ip` | The banned IP address |
| `expires_in_secs` | Seconds until automatic unban. `null` = permanent ban |
| `count` | Total number of entries in the list |

---

### `POST /_/security/unblock`

Manually unban an IP address, clearing its block state and accumulated SQLi attempt counter.

```bash
curl -X POST http://127.0.0.1:8080/_/security/unblock \
  -H "Authorization: Bearer my-secret" \
  -H "Content-Type: application/json" \
  -d '{"ip":"203.0.113.42"}'
```

**Response** (`200 OK`):
```json
{ "unblocked": true, "ip": "203.0.113.42" }
```

| Field | Description |
|-------|-------------|
| `unblocked` | `true` if the IP was found and cleared; `false` if IP was not in the block list |
| `ip` | The IP address that was processed |

---

### `POST /_/cache/clear`

Clears the in-memory response cache.

```bash
curl -X POST http://127.0.0.1:8080/_/cache/clear \
  -H "Authorization: Bearer my-secret"
```

**Response** (`200 OK`):
```json
{ "cleared": 512 }
```

`cleared` is the number of cached entries removed.

---

## Configuration

```toml
[dashboard]
enabled    = true           # /_/dashboard HTML page
statistics = true           # /_/status and /_/metrics
token      = "my-secret"    # Bearer token for all /_/* endpoints
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable `/_/dashboard` |
| `statistics` | bool | `true` | Enable `/_/status` and `/_/metrics` |
| `token` | string | none | If set, all `/_/` requests must include `Authorization: Bearer <token>` |

See [config.md](config.md) for the full configuration reference.

---

## Endpoint Summary

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/_/dashboard` | GET | optional | HTML admin dashboard |
| `/_/status` | GET | optional | JSON runtime metrics |
| `/_/metrics` | GET | optional | Prometheus metrics |
| `/_/cache/clear` | POST | optional | Clear response cache |
| `/_/security/blocked` | GET | optional | List banned IPs |
| `/_/security/unblock` | POST | optional | Unban an IP |

> "optional" means the endpoint is open when no `token` is configured. In production, always set a `token`.
