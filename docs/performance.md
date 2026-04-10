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

[php]
memory_limit = "512M"
opcache_memory = 256
jit_buffer_size = "128M"

[compression]
enabled = true

[security]
enabled = true

[logging]
level = "warn"

[watcher]
enabled = false
```
