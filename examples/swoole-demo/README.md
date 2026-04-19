# Swoole-Style Demo — All Four Primitives in One Request

This example combines every Swoole-style primitive Turbine ships:

| Primitive   | Used for in this demo                                    |
|-------------|----------------------------------------------------------|
| SharedTable | per-IP rate limiting + warm feature flag cache           |
| TaskQueue   | fire-and-forget email dispatch                           |
| WebSocket   | realtime activity feed published to `events` channel     |
| AsyncIO     | parallel config fetch + retry timer                      |

Everything is opt-in via `turbine.toml`.  Disabling any block just
removes the corresponding feature from the response — the app never
crashes.

## Running

```bash
# 1. Start the server (from repo root)
./target/release/turbine serve --config examples/swoole-demo/turbine.toml \
    --root examples/swoole-demo

# 2. In another shell, start an email consumer
php examples/task-consumer/consumer.php emails

# 3. In a third shell, subscribe to the realtime feed
curl -N "http://127.0.0.1:8080/_/ws/subscribe?channel=events" \
    -H "Upgrade: websocket" -H "Connection: upgrade" \
    -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
    -H "Sec-WebSocket-Version: 13"

# 4. Hammer the app
for i in $(seq 1 20); do curl -s http://127.0.0.1:8080/ ; done
```

## Metrics

Scrape `/_/metrics` with Prometheus:

```
curl http://127.0.0.1:8080/_/metrics
```

You will see counters like:

```
turbine_shared_table_size 3
turbine_task_queue_pushed_total 17
turbine_ws_published_total 17
turbine_async_reads_total 17
```

## What the request does

For every incoming request, `app.php`:

1. Rate-limits the remote IP via `turbine_table_incr` (shared across all
   workers, TTL = 60 s window).
2. Warms or reads a feature flag from the shared table.
3. Schedules an email job on the `emails` channel (processed by the
   consumer in step 2 above).
4. Broadcasts an `"event"` JSON blob on the `events` WebSocket channel.
5. Reads two config files in parallel via `turbine_async_parallel` to
   prove non-blocking I/O works.
6. Schedules a retry timer 5 s out via `turbine_async_timer` that will
   push a follow-up job onto `emails`.

All of that happens in well under a millisecond of PHP CPU because the
heavy lifting is on the Rust side.
