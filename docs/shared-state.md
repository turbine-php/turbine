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

---

# Task Queue

In-process async job queue. Lets request handlers offload slow work
(emails, webhooks, image resizing, cache warmups) to dedicated PHP CLI
consumers without pulling in Redis, SQS, or RabbitMQ.

This is the second half of Turbine's Swoole-like primitives. Together
with `SharedTable` it covers the majority of coordination patterns
framework users reach for.

## Enable

```toml
[task_queue]
enabled          = true
max_channels     = 64       # distinct named queues
channel_capacity = 10_000   # FIFO depth per channel
max_wait_ms      = 30_000   # hard ceiling for long-poll pop
```

## PHP API

```php
turbine_task_push(string $channel, string $payload): ?int          // returns job id
turbine_task_pop(string $channel, int $wait_ms = 0): ?array        // ['id'=>int,'payload'=>string]
turbine_task_size(string $channel): int
turbine_task_stats(): array                                        // channels/pushed/popped/rejected
```

`push` is fire-and-forget for the producer — it returns as soon as the
job lands in the queue. `pop` supports long-polling: give it `wait_ms`
and the request will block server-side (no PHP CPU used) until a job
arrives or the wait elapses. Returns `null` on timeout.

### Producer (inside a request handler)

```php
$id = turbine_task_push('email', json_encode([
    'to'      => 'user@example.com',
    'subject' => 'Welcome',
]));
if ($id === null) {
    // queue full or too many channels
}
```

### Consumer (a CLI script — run N copies under systemd/supervisor)

```php
// consumer.php
while (true) {
    $job = turbine_task_pop('email', 10_000);   // 10s long-poll
    if ($job === null) continue;
    try {
        $data = json_decode($job['payload'], true);
        send_email($data['to'], $data['subject']);
    } catch (\Throwable $e) {
        error_log("job {$job['id']} failed: {$e->getMessage()}");
    }
}
```

Run multiple consumers for parallelism:

```sh
for i in 1 2 3 4; do php consumer.php & done
```

## HTTP API

| Method | Path | Query | Body | Response |
|---|---|---|---|---|
| `POST`   | `/_/task/push`  | `channel` | raw payload | `{"id":<u64>}` |
| `POST`   | `/_/task/pop`   | `channel`, `wait_ms?` | — | 200 + raw body + `X-Task-Id`, 204 on timeout |
| `GET`    | `/_/task/size`  | `channel` | — | `{"size":<usize>}` |
| `GET`    | `/_/task/stats` | — | — | `{"channels":N,"pushed":N,"popped":N,"rejected":N}` |
| `DELETE` | `/_/task/clear` | `channel` | — | `{"cleared":<usize>}` |

## Design notes

- **Per-channel FIFO** guarded by a short `parking_lot::Mutex` critical
  section — no await under lock.
- **Long-poll** powered by `tokio::sync::Notify`; consumers park on the
  server without burning CPU.
- **Bounded:** both channel count and per-channel depth are capped.
  Push past the cap returns 507 Insufficient Storage rather than
  silently dropping jobs.
- **No at-least-once:** a crash during `pop` → process loses the job.
  If the producer requires guaranteed delivery, use a real broker.
- **Long-poll is clamped** to `max_wait_ms`; this prevents a buggy
  consumer from tying up a connection indefinitely.

## Limits

- Single-process only (same as `SharedTable`).
- No priorities, no delayed jobs, no retry scheduling. Build these on
  top using `SharedTable` for bookkeeping if you need them.
- Payload size is capped at `max(max_body_bytes, 1 MB)`.

---

## Design notes (shared table)

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
