# Task Consumer Example

Long-running PHP CLI that consumes jobs pushed via `turbine_task_push()`.

## Setup

Enable the task queue in `turbine.toml`:

```toml
[task_queue]
enabled          = true
max_channels     = 64
channel_capacity = 10_000
max_wait_ms      = 30_000
```

Start Turbine:

```sh
turbine serve
```

## Producer (in any request handler)

```php
turbine_task_push('emails', json_encode(['to' => 'user@example.com']));
```

## Consumer (run N copies)

```sh
# A single consumer
php consumer.php emails

# Multiple workers
for i in 1 2 3 4; do php consumer.php emails & done
wait
```

Under systemd or supervisor, wrap each copy in its own unit / program.

## Behaviour

- `turbine_task_pop` long-polls the server (no CPU used while waiting).
- Consumers process jobs at-most-once. If your handler crashes mid-job,
  the job is lost — fine for emails / cache warmup, not for payments.
- Back-pressure: when the channel fills up, `turbine_task_push` returns
  `null` and the producer can fall back to inline processing or a real
  broker.
