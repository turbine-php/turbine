# Performance & Tuning

## Architecture Advantages

### 1. Zero IPC Overhead

Traditional PHP servers (PHP-FPM) communicate via FastCGI sockets, adding serialization and context-switching overhead. Turbine embeds PHP directly in the same process — function calls instead of socket I/O.

### 2. Persistent Bootstrap

Turbine's persistent workers bootstrap Laravel/Symfony once and reuse the application instance across requests. The framework boot phase (autoloader, config, routes, service providers) is eliminated from every request.

Even **without** persistent workers (`persistent_workers = false`), Turbine is faster than PHP-FPM because:
- PHP is embedded in-process (no FastCGI socket IPC)
- OPcache is shared across all workers (no recompilation)
- Workers are pre-forked/pre-spawned (no process creation per request)

### 3. Rust HTTP Stack

Turbine uses Hyper (Rust) for HTTP parsing and connection management — one of the fastest HTTP implementations available.

### 4. Shared OPcache

All workers share the same OPcache with JIT compilation. Bytecode is compiled once and reused across all workers, with L2 file cache for warm restarts.

### 5. Worker Mode

**This is the single biggest performance lever after persistent workers.**

| Combination | IPC Mechanism | Memory per worker | Throughput |
|-------------|--------------|:-----------------:|:----------:|
| Process + non-persistent | `pipe(2)` per request | ~10–30 MB (CoW) | High |
| Process + persistent | `pipe(2)` per request | ~10–30 MB (CoW) | Higher (no bootstrap) |
| Thread + non-persistent | **In-memory channel (zero syscalls)** | ~2–5 MB (TSRM) | **Highest** |
| Thread + persistent | `pipe(2)` per request | ~2–5 MB (TSRM) | Higher (no bootstrap) |

Thread + non-persistent is the fastest mode for simple scripts: it uses lock-free `std::sync::mpsc` channels with zero pipe syscalls and returns responses as Rust structs (zero-copy). Thread + persistent still uses pipes for the binary protocol but benefits from bootstrap-once execution.

> Thread mode requires PHP compiled with `--enable-zts`. Run `./build.sh` → select **Thread mode (ZTS)**. See [Worker Mode](worker.md) for the full guide.

### 6. Persistent Workers

When `persistent_workers = true`, workers bootstrap the application once and handle thousands of requests without re-initialization. This is the primary throughput advantage over PHP-FPM for framework-based applications.

```toml
[server]
persistent_workers = true
worker_max_requests = 10000
```

**Important notes:**
- Always set `worker_max_requests > 0` with persistent workers. Without recycling, PHP state accumulates and throughput degrades over time (up to 50% loss over sustained load).
- Persistent worker recycling is **graceful** — a `0xFF` shutdown byte is sent via pipe (not SIGTERM), so in-flight requests complete before respawn. This ensures zero errors during recycling.
- For trivial scripts (e.g. `echo "Hello"`), process workers may outperform persistent workers due to the pipe IPC overhead (~33µs per request). For real applications with >1ms execution time, the bootstrap savings dominate.

## Tuning Guide

### Worker Count

```toml
[server]
workers = 8  # Start with CPU core count
```

| CPU Cores | Workers (CPU-bound) | Workers (I/O-bound) |
|:---------:|:------------------:|:-------------------:|
| 2 | 2 | 4 |
| 4 | 4 | 8 |
| 8 | 8 | 16 |
| 16 | 16 | 32 |

For I/O-bound workloads (database queries, API calls), use 2x cores. Workers spend time waiting on I/O, so more workers keep the CPU busy.

### OPcache & JIT

```toml
[php]
opcache_memory = 256     # More memory for large apps
jit_buffer_size = "128M" # Larger JIT buffer
```

OPcache is configured for maximum performance:
- `validate_timestamps = 0` — Never stat files (changes require restart)
- `file_cache = /tmp/turbine-opcache` — L2 disk cache for warm restarts
- `jit = function` — JIT compiles hot functions to native code

#### JIT tuning (PHP 8.0+)

`jit_buffer_size` controls how much memory the JIT gets for native code.
Tune based on framework size:

| Workload                         | Recommended            |
|:---------------------------------|:-----------------------|
| Simple API / microservice         | `"32M"` (default)      |
| Laravel / Symfony / WordPress     | `"128M"`               |
| Data crunching / template engines | `"256M"` + `jit=tracing` |

`jit = tracing` (the full tracing JIT) is significantly faster than the
default `function` mode for CPU-bound PHP (template engines, ORM query
builders, compute loops) — expect **2-3× speed-up** on arithmetic-heavy
code. It costs a bit more memory and has a warm-up period where it
discovers hot traces, which is why you typically combine it with
**preload** so the warm-up runs at boot instead of at first request.

#### OPcache preload

Preload lets PHP parse + link every class in a framework **once, at
master boot**, before any `fork()`. Combined with Turbine's CoW worker
model this means:

- All workers share **one physical copy** of Laravel's class graph —
  RSS per worker drops 30-50 %.
- First-request latency drops dramatically. On Laravel, cold boot goes
  from ~80 ms → ~5 ms per request because routing, middleware, and
  container reflection are already linked.
- JIT tracing has time to warm up during preload instead of burning
  CPU on the first real user's request.

Turbine auto-detects the following files at startup (in order):

1. `vendor/preload.php`
2. `preload.php`
3. `config/preload.php`

To point at a custom location:

```toml
[php]
preload_script = "bootstrap/preload.php"
```

A minimal Laravel preload file:

```php
<?php
$root = __DIR__ . '/..';
require $root . '/vendor/autoload.php';

$classes = [
    // Framework core — adjust to your app's hot classes
    \Illuminate\Foundation\Application::class,
    \Illuminate\Http\Request::class,
    \Illuminate\Http\Response::class,
    \Illuminate\Routing\Router::class,
    \Illuminate\Routing\UrlGenerator::class,
    \Illuminate\Container\Container::class,
    \Illuminate\Database\Eloquent\Model::class,
    \Illuminate\View\Factory::class,
];

foreach ($classes as $cls) {
    opcache_compile_file((new ReflectionClass($cls))->getFileName());
}
```

> **Caveat:** preload requires `opcache.validate_timestamps = 0`. Code
> changes to preloaded files require a full restart — which is already
> how Turbine deploys work (`SIGHUP` or systemd restart).

### Response Cache

For endpoints that return the same content:

```toml
[cache]
enabled = true
ttl_seconds = 30
max_entries = 1024
```

The cache stores complete HTTP responses keyed by URL. Cache invalidation is TTL-based.

### Compression

```toml
[compression]
enabled = true
min_size = 1024          # Don't compress tiny responses
level = 6               # Balance speed vs ratio
algorithms = ["br", "zstd", "gzip"]  # Brotli preferred
```

Algorithm performance (compressing 50KB HTML):

| Algorithm | Ratio | Speed |
|-----------|:-----:|:-----:|
| Brotli | ~85% | Fast |
| Zstd | ~80% | Fastest |
| Gzip | ~75% | Fast |

Turbine negotiates the best algorithm based on the client's `Accept-Encoding` header.

### Request Timeout & Queue

```toml
[server]
request_timeout = 30  # Kill slow requests
max_wait_time = 5     # Don't queue too long
```

Set `max_wait_time` to prevent cascading failures. When all workers are busy and the queue exceeds this timeout, new requests get 503 immediately instead of piling up.

### Tokio Async Threads

The `tokio_worker_threads` option controls how many OS threads handle the async HTTP stack (connection accept, parsing, response writing).

```toml
[server]
tokio_worker_threads = 6  # Default: CPU core count
```

| PHP Workers | Recommended Tokio Threads | Notes |
|:-----------:|:-------------------------:|-------|
| ≤4 | 4–6 | Fewer threads avoid contention |
| ≥8 | Default (CPU cores) | More PHP workers need more async capacity |

Reducing Tokio threads below 4 can bottleneck connection handling (−30% throughput). Test with your workload to find the optimum.

## Production Checklist

- [ ] Set `workers` based on CPU cores and workload type
- [ ] Choose `worker_mode` (`"process"` or `"thread"`)
- [ ] Enable `persistent_workers = true` for framework apps
- [ ] Set `worker_max_requests` (10,000–50,000 for persistent workers)
- [ ] Tune `tokio_worker_threads` if needed (default is usually fine)
- [ ] Enable compression (`br` preferred for web traffic)
- [ ] Set appropriate `request_timeout` and `max_wait_time`
- [ ] Increase `opcache_memory` for large applications
- [ ] Set log level to `warn` or `error`
- [ ] Disable file watcher
- [ ] Enable security guards
- [ ] Configure TLS (or use a reverse proxy)

```toml
[server]
workers = 0
worker_mode = "process"
persistent_workers = true
request_timeout = 30
max_wait_time = 5
worker_max_requests = 10000
# tokio_worker_threads = 6
# --- Linux-only tuning (safe to leave commented on macOS / small boxes) ---
# pin_workers = true
# listen_busy_poll_us = 50
# listen_reuseport_shards = 8

[php]
memory_limit = "512M"
opcache_memory = 256
jit_buffer_size = "128M"

[compression]
enabled = true

[cache]
enabled = true       # also activates request coalescing (singleflight)
ttl_seconds = 30

[security]
enabled = true

[logging]
level = "warn"

[watcher]
enabled = false
```

## Advanced / experimental

### Always-on optimizations (no config needed)

Turbine enables the following by default — listed here so you know what you
are benchmarking:

- **mimalloc** as the global allocator (Microsoft Research). Typical
  wins on allocation-heavy threaded workloads: 5–10 % throughput,
  20–40 % lower p99 under concurrency, smaller RSS due to aggressive
  segment reuse and better thread-local caches. Matches what Bun and
  Deno ship.
- **Transparent huge pages hint** (`madvise(MADV_HUGEPAGE)` +
  `PR_SET_THP_DISABLE=0`) applied at process start on Linux. Reduces
  TLB misses on OPcache SHM and Zend arenas. No-op on macOS and on
  hosts configured with `transparent_hugepage = never`.
- **LIFO hot-worker dispatch.** When a worker returns to the idle pool,
  it is pushed to the back; the next request pops from the back. The
  most-recently-used worker — whose OPcache, Zend arenas, and CPU
  L1/L2 caches are still warm — gets the next request. Cold workers
  still recycle naturally at peak load when the idle queue is empty.
  Same trick Go's scheduler uses on local run queues.
- **Request coalescing (singleflight)** on cacheable `GET`s. When N
  concurrent requests hit the same URL and the response cache is
  enabled but empty, only one PHP execution runs; the other N-1
  callers wait on a `Notify` and receive a clone of the leader's
  response. Saves server CPU roughly in proportion to traffic
  concentration (often 50 %+ on real sites with a Pareto URL
  distribution). Requires `[cache] enabled = true`.
- **Zero-copy cache hits.** Cache bodies are stored as refcounted
  `Bytes`. Serving a hit clones an 8-byte refcount instead of copying
  the body — a 50 KB response served to 1000 concurrent clients
  allocates 50 KB once, not 50 MB.
- **`TCP_NODELAY` on every accepted stream.** Kills up to ~40 ms of
  Nagle latency on p99 for small responses.
- **Async pipe I/O via `AsyncFd`** (tokio reactor directly, no
  `spawn_blocking`). Eliminates the 512-thread blocking-pool ceiling
  on concurrent PHP requests.

None of these are config-gated — they are part of the default build.

### CPU pinning (Linux only)

```toml
[server]
pin_workers = true
```

Binds each worker to a fixed logical core (`worker_N → core N % ncpus`).
Wins come from avoiding the scheduler bouncing hot PHP processes
between cores, which invalidates L2/L3 caches and OPcache hot pages.

Only enable when all of these are true:
- Running on a **dedicated host** (no noisy neighbours).
- `worker_count ≤ physical_core_count` (oversubscription negates the win).
- Workload is latency-sensitive (tail latency matters more than throughput).

No-op on macOS and in environments that don't allow `sched_setaffinity`
(most cgroup-restricted containers).

### `SO_BUSY_POLL` (Linux only)

```toml
[server]
listen_busy_poll_us = 50   # spin budget in microseconds
```

Applied to the listener and every accepted connection. The kernel
spins on the NIC RX queue for up to `listen_busy_poll_us` µs before
yielding to the scheduler. Shaves 20–50 µs off p99 latency when the
host is otherwise idle, but **wastes CPU on oversubscribed hosts** —
the spin is real.

Typical values: `50` (latency-sensitive APIs) to `200` (extreme
low-latency trading-style). Silently ignored on macOS and when
`setsockopt` fails (e.g. insufficient privileges on kernels < 5.7).

### `SO_REUSEPORT` accept sharding (Linux only)

```toml
[server]
listen_reuseport_shards = 8   # typically = tokio_worker_threads
```

Binds N independent listener sockets to the same `(addr, port)` and
runs one accept loop per shard. The Linux kernel distributes new
connections across them using a per-flow hash, removing contention on
the single accept queue that otherwise becomes the bottleneck above
~100 k connections/sec.

Recommended value: match `tokio_worker_threads` (typically `ncpus`).
The gain is largest on connection-churn-heavy workloads (short-lived
HTTP/1 without keep-alive, WebSocket upgrades). Negligible on
keep-alive-heavy workloads where connection setup is amortised.

`None` / `0` / `1` keeps single-listener behaviour. Silently falls
back to a single listener on non-Linux and when additional shards
fail to bind.

### Profile-Guided Optimization (PGO)

`scripts/build-pgo.sh` automates the three-stage PGO workflow:

1. **Instrumented build** — compiles Turbine with
   `-Cprofile-generate` to emit counter hooks for every branch and
   function call.
2. **Workload run** — you start the instrumented binary and hit it
   with `wrk` (or production traffic mirrored through it) against a
   representative sample app. Counters accumulate in `.profraw`
   files.
3. **Optimized build** — consumes `llvm-profdata merge`'d profile to
   reorder functions for instruction-cache locality, bias branch
   prediction, and inline hot paths that cost-model alone would skip.

Typical gain on HTTP-heavy code: **8–15 % throughput, 5–10 % lower
p99**. Requires `rustup component add llvm-tools-preview`.

```bash
# Stage 1-2 (instrumented + workload)
scripts/build-pgo.sh http://127.0.0.1:8080/
# → prompts you to run `wrk` then press Enter

# The script then merges .profraw files and does the optimized build.
# Final binary: target/release/turbine
```

Combine with `pin_workers` + `listen_busy_poll_us` on a dedicated
bare-metal host for the best numbers Turbine can produce today.

### IRQ affinity (sysadmin, outside Turbine)

For the extreme end: route the NIC interrupt to cores that do NOT run
PHP workers. On a 16-core host you might reserve cores 0-1 for IRQs +
tokio reactor, cores 2-15 for `pin_workers`.

```bash
# Example: move ethN interrupts to cores 0-1
echo 3 | sudo tee /proc/irq/$(grep ethN /proc/interrupts | awk -F: '{print $1}' | tr -d ' ')/smp_affinity
```

Combine with `isolcpus=2-15` on the kernel boot line to fully isolate
worker cores from general-purpose scheduling. Typical gain on latency
p99: **30-50%**. This is stock ScyllaDB / Redis-Benchmark tuning.

### io_uring backend (stub — not yet active)

Turbine's `io-uring` Cargo feature compiles a placeholder module but
does **not** yet replace the epoll-based pipe I/O. The dispatch path
still uses `tokio::io::unix::AsyncFd` (fast, but does incur a syscall
per read/write).

A full io_uring backend with `SQPOLL` would eliminate those syscalls
entirely — Cloudflare Pingora reported ~30 % throughput improvement
after the same switch. The implementation is non-trivial because
`tokio-uring` uses completion-based futures that don't freely compose
with hyper/rustls, so it requires a dedicated runtime on a thread
isolated from the HTTP reactor. Tracking milestone: Turbine 0.3.

## A note on `output_buffering`

Turbine captures PHP output via a custom `ub_write` SAPI callback.
That callback is only drained during `php_request_shutdown`, so
`output_buffering` **must stay at 0** (the default Turbine ships). Any
non-zero value would cause responses larger than the buffer to be
truncated on the hot path.

Turbine enforces `output_buffering=0` in the generated php.ini and
actively rejects overrides from `[php.ini]` in `turbine.toml` with a
warn-level log message. Don't try to work around this — if you need
buffered output in userland, use `ob_start()` / `ob_get_clean()` in
PHP, which compose correctly with the SAPI capture.
