# Shared State (SharedTable)

In-process, cross-worker key/value store for PHP userland. This is Turbine's
equivalent of `Swoole\Table` — a small concurrent hash map that lives inside
the Turbine server and is reachable from every PHP worker without touching
Redis or Memcached.

## What it is (and is not)

- ✅ **Is:** a bounded, TTL-aware, lock-free key/value store shared by all
  workers in a single Turbine process. Values are arbitrary byte strings.
  Counters (`incr`/`decr`) are atomic.
- ❌ **Is not:** a Redis replacement. There is no persistence, no pub/sub,
  no Lua, no cluster replication. Survives reloads only if the Turbine
  process itself survives.
- ❌ **Is not:** microsecond-grade. Access from PHP goes through a local
  HTTP round-trip (typically 200–500 µs on Linux with keep-alive curl).
  For hot inner loops prefer APCu. For cross-worker coordination this
  is fine.

## Enable

`turbine.toml`:

```toml
[shared_table]
enabled = true
max_entries = 65536       # hard cap; insert after this returns "full"
sweep_interval_secs = 5   # background TTL eviction cadence
```

When disabled (the default) no memory is allocated and the PHP helpers
are not injected into the bootstrap.

## PHP API

All helpers are injected automatically when `shared_table.enabled = true`.
They require `ext-curl`.

```php
turbine_table_set(string $key, string $value, int $ttl_ms = 0): bool
turbine_table_get(string $key): ?string
turbine_table_del(string $key): bool
turbine_table_exists(string $key): bool
turbine_table_incr(string $key, int $delta = 1): ?int   // atomic
turbine_table_size(): int
```

Values are binary-safe. Keys are UTF-8 strings (URL-encoded on the wire).
`ttl_ms = 0` means "no expiry".

### Example: per-IP rate limiter

```php
$ip  = $_SERVER['REMOTE_ADDR'];
$key = "rl:{$ip}";
$n   = turbine_table_incr($key, 1);
if ($n === 1) {
    turbine_table_set($key, pack('q', 1), 60_000); // 60 s window
}
if ($n > 100) {
    http_response_code(429);
    exit('too many requests');
}
```

### Example: feature flag

```php
if (turbine_table_get('feature:new_checkout') === '1') {
    // ...
}
```

## HTTP API (for external callers)

The helpers are thin wrappers around these endpoints on the Turbine
listener. They require `[dashboard] token` authentication if configured.

| Method | Path | Query | Body | Response |
|---|---|---|---|---|
| `GET`    | `/_/table/get`    | `key` | — | raw value, 404 if missing |
| `POST`   | `/_/table/set`    | `key`, `ttl_ms?` | raw value | 204 No Content |
| `DELETE` | `/_/table/del`    | `key` | — | `{"deleted":true\|false}` |
| `GET`    | `/_/table/exists` | `key` | — | 200/404 |
| `POST`   | `/_/table/incr`   | `key`, `delta?` | — | `{"value":<i64>}` |
| `GET`    | `/_/table/size`   | — | — | `{"size":<usize>}` |
| `DELETE` | `/_/table/clear`  | — | — | 204 |

## Design notes

- **Backend:** [`dashmap`](https://crates.io/crates/dashmap) (sharded
  `parking_lot` write locks). No global mutex.
- **TTL:** monotonic `Instant` deadlines; lazy eviction on read plus a
  background sweeper on `sweep_interval_secs`.
- **Counters:** 8-byte little-endian `i64` stored as the value. `incr`
  takes the entry lock so the read-modify-write is atomic.
- **Capacity:** `max_entries` is a hard cap on new keys. Updates to
  existing keys always succeed. Expired entries are freed by the sweeper
  or by a subsequent `get`.
- **Security:** the endpoints sit under `/_/` and honour the same bearer
  token as the dashboard. Do not expose `/_/*` to the public internet.

## Limits

- Single-process only. Two Turbine processes on the same host do **not**
  share state. Use Redis for multi-node.
- No atomic multi-key transactions.
- `del` is fire-and-forget from PHP's point of view (no error on missing
  key — returns `false`).
