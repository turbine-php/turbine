# Worker Mode

> **This is the most important architectural choice when deploying Turbine.** Read this page before configuring anything else.

Turbine supports **four worker combinations** from two independent axes: the worker backend (`worker_mode`: process or thread) and the execution model (`persistent_workers`: true or false). Each combination has different IPC mechanisms, isolation guarantees, and performance characteristics.

## Quick Decision Guide

```
What kind of application are you running?

  Framework app (Laravel, Symfony, WordPress)?
    └─ persistent_workers = true
    └─ worker_mode = "process" (default, safest)
    └─ Or worker_mode = "thread" if ZTS + all extensions are thread-safe

  Simple scripts / APIs with fast execution (<1ms)?
    └─ persistent_workers = false (per-request)
    └─ worker_mode = "thread" for maximum throughput (zero-syscall channels)
    └─ Or worker_mode = "process" for safety with NTS PHP

  Legacy code, pcntl-heavy, or unstable extensions?
    └─ worker_mode = "process" (always)
    └─ persistent_workers = false (safest)

  Not sure?
    └─ Start with: worker_mode = "process" + persistent_workers = true
    └─ This works everywhere and gives the biggest performance win
```

| Mode | `worker_mode` | `persistent_workers` | PHP required | IPC | Throughput |
|------|--------------|---------------------|--------------|-----|------------|
| Process per-request | `"process"` | `false` | NTS or ZTS | Pipes | Good |
| Process persistent | `"process"` | `true` | NTS or ZTS | Pipes | High |
| Thread per-request | `"thread"` | `false` | **ZTS only** | **Channels (zero syscalls)** | **Highest** |
| Thread persistent | `"thread"` | `true` | **ZTS only** | Pipes | High |

> **How to get ZTS PHP:** Run `./build.sh` and select **Thread mode (ZTS)** in the build mode selector. The script compiles PHP with `--enable-zts` and installs it to `vendor/php-embed-zts/`.

See also: [Known Issues — Worker Crash Isolation](known-issues.md#worker-crash-isolation) and [Compile from Source](compile.md#nts-vs-zts).

## The Four Worker Combinations

Turbine has **two independent axes**: worker backend (`worker_mode`) and execution model (`persistent_workers`). This produces four distinct combinations:

| | `persistent_workers = false` | `persistent_workers = true` |
|-|:----------------------------:|:---------------------------:|
| **`worker_mode = "process"`** | Process + per-request | Process + persistent |
| **`worker_mode = "thread"`** | Thread + per-request | Thread + persistent |

### Summary

| Combination | IPC | PHP Execution | Bootstrap | Best For |
|-------------|-----|--------------|-----------|----------|
| **Process + per-request** | Pipes | `php_execute_script` with full lifecycle | Every request | Default — safe, simple |
| **Process + persistent** | Pipes | `php_execute_script` with warm OPcache | Once per worker | Framework apps (Laravel, Symfony) |
| **Thread + per-request** | In-memory channels (zero syscalls) | `php_execute_script` with full lifecycle | Every request | Maximum throughput, simple scripts |
| **Thread + persistent** | Pipes | `php_execute_script` with warm OPcache | Once per thread | Framework apps on ZTS PHP |

> **Key insight**: Thread + per-request uses lock-free `std::sync::mpsc` channels (no pipe syscalls at all), making it the fastest mode for lightweight endpoints. Thread + persistent still uses pipes for the binary protocol but eliminates the framework bootstrap overhead.

## How It Works

### Traditional PHP-FPM

```
Request 1: FastCGI → Boot PHP → Load Composer → Boot Framework → Handle → Destroy
Request 2: FastCGI → Boot PHP → Load Composer → Boot Framework → Handle → Destroy
Request 3: FastCGI → Boot PHP → Load Composer → Boot Framework → Handle → Destroy
```

### Turbine Per-Request Workers (persistent_workers = false)

```
Request 1: [embedded] php_execute_script → OPcache hit → Handle → Destroy
Request 2: [embedded] php_execute_script → OPcache hit → Handle → Destroy
Request 3: [embedded] php_execute_script → OPcache hit → Handle → Destroy
```

Even without persistent workers, Turbine is faster than PHP-FPM because:
- **No FastCGI IPC** — PHP is embedded in the same process (function calls, not socket I/O)
- **Shared OPcache** — compiled bytecodes persist across requests (no recompilation)
- **No process creation** — workers are pre-forked or pre-spawned, not created per request

The trade-off: framework boot (autoloader, service container, routes) still happens on every request.

### Turbine Persistent Workers (persistent_workers = true)

```
Worker Boot: Boot PHP → Load Composer → Boot Framework (once)
Request 1: Handle → Reset superglobals
Request 2: Handle → Reset superglobals
Request 3: Handle → Reset superglobals
... (10,000 requests before graceful respawn)
```

The bootstrap phase (autoloader, service container, config, routes) happens **once**. Each subsequent request only executes the request-specific code. This is the primary performance advantage for framework-based applications.

## Worker Configuration

```toml
[server]
# Number of worker processes/threads
workers = 8
# Worker backend: "process" (fork, default) or "thread" (ZTS required)
worker_mode = "process"
# Enable persistent workers (bootstrap once, handle many requests)
persistent_workers = true
# Max requests per worker before respawn (prevents memory leaks)
worker_max_requests = 10000
# Request timeout in seconds
request_timeout = 30
# Max queue wait time before 503 (0 = use request_timeout)
max_wait_time = 5
# Number of Tokio async I/O threads (default = CPU cores)
# tokio_worker_threads = 6
```

### Persistent Workers

When `persistent_workers = true`, workers bootstrap the application (autoloader, framework, service container) **once** and then handle thousands of requests without re-initialization. This eliminates the framework boot overhead on every request.

Without `persistent_workers` (or when set to `false`), each request runs `php_execute_script` with full PHP lifecycle (startup → execute → shutdown). This is faster than PHP-FPM (no FastCGI, shared OPcache) but still boots the framework on every request.

```toml
[server]
persistent_workers = true
worker_max_requests = 10000  # Recycle after 10K requests
```

> **Important**: Persistent workers should always use `worker_max_requests > 0`. Without recycling, PHP state accumulates over time and throughput degrades. A value of 10,000–50,000 is recommended.

### Worker Mode

Turbine supports two worker backends, selectable via `worker_mode` in `turbine.toml`:

| Mode | Backend | PHP Requirement | Isolation | Memory | Use Case |
|------|---------|----------------|-----------|--------|----------|
| `"process"` | `fork()` + Copy-on-Write | NTS or ZTS | Full (separate processes) | Higher (per-process page tables) | Default, safest |
| `"thread"` | `std::thread` + TSRM | **ZTS only** | Shared address space | Lower (shared memory) | High-throughput, ZTS-safe extensions |

**Process mode** (default): Each worker is a separate OS process created via `fork()`. OPcache bytecodes are shared via Copy-on-Write memory. A crash in one worker cannot affect others. Works with any PHP build (NTS or ZTS). IPC is always pipe-based.

**Thread mode**: Each worker is an OS thread sharing the same address space. Requires PHP compiled with `--enable-zts` (Zend Thread Safety). Lower memory overhead.

Thread mode IPC depends on the execution model:
- **Per-request** (`persistent_workers = false`): Uses in-memory `std::sync::mpsc` channels — **zero pipe syscalls**, lock-free dispatch, responses returned as Rust structs (zero-copy). This is the fastest IPC path.
- **Persistent** (`persistent_workers = true`): Uses pipes (same binary protocol as process+persistent) because the persistent event loop relies on the pipe-based wire protocol for the bootstrap-once model.

A crash in any thread can take down all workers. Only use with ZTS-safe extensions.

```toml
# Thread mode example (requires ZTS PHP)
[server]
worker_mode = "thread"
workers = 16
```

If thread mode is selected but PHP was compiled without ZTS, Turbine will exit with a clear error:
```
Thread worker mode requires PHP compiled with ZTS (--enable-zts). Current PHP is NTS.
```

> **Note**: Many PHP C extensions (including Phalcon) are not ZTS-safe. Verify your extensions support ZTS before enabling thread mode.

### Choosing Worker Count

| Workload | Recommended Workers |
|----------|-------------------|
| CPU-bound (computation) | CPU cores |
| I/O-bound (database, API calls) | CPU cores × 2 |
| Mixed | CPU cores × 1.5 |
| Auto-detect | `workers = 0` |

Setting `workers = 0` auto-detects based on available CPU cores.

## Worker Lifecycle

### Process Mode (default)

```
┌─────────────────────────────────────────┐
│ Master Process                          │
│                                         │
│  ┌──────────┐  ┌──────────┐  ┌───────┐ │
│  │ Worker 1 │  │ Worker 2 │  │ ... N │ │
│  │ (fork)   │  │ (fork)   │  │(fork) │ │
│  └──────────┘  └──────────┘  └───────┘ │
│       ↕              ↕            ↕     │
│     pipes          pipes        pipes   │
│                                         │
│  HTTP Listener → Route → Dispatch       │
└─────────────────────────────────────────┘
```

1. **Master process** binds the TCP port and accepts connections
2. **Worker processes** are forked at startup, each inheriting the PHP interpreter via CoW
3. Requests are dispatched to available workers via **pipes** (binary protocol)
4. Workers execute `php_execute_script` with full PHP lifecycle (startup → execute → shutdown)
5. With `persistent_workers = true`, bootstrap happens once; state resets between requests
6. After `worker_max_requests`, a worker is recycled (graceful respawn)

### Thread Mode — Per-Request (zero-syscall fast path)

```
┌─────────────────────────────────────────┐
│ Single Process                          │
│                                         │
│  ┌──────────┐  ┌──────────┐  ┌───────┐ │
│  │ Thread 1 │  │ Thread 2 │  │ ... N │ │
│  │ (TSRM)   │  │ (TSRM)   │  │(TSRM) │ │
│  └──────────┘  └──────────┘  └───────┘ │
│       ↕              ↕            ↕     │
│   channels       channels     channels  │
│  (lock-free)    (lock-free)  (lock-free)│
│                                         │
│  HTTP Listener → Route → Dispatch       │
└─────────────────────────────────────────┘
```

1. **Single process** binds the TCP port and accepts connections
2. **Worker threads** are spawned at startup, each with its own TSRM interpreter context
3. Requests are dispatched via **in-memory channels** (`std::sync::mpsc`) — no pipe syscalls
4. Responses are returned as Rust structs (zero-copy, no serialization)
5. This is the fastest IPC path: lock-free, no kernel involvement
6. After `worker_max_requests`, a thread exits and is respawned

### Thread Mode — Persistent (bootstrap-once)

```
┌─────────────────────────────────────────┐
│ Single Process                          │
│                                         │
│  ┌──────────┐  ┌──────────┐  ┌───────┐ │
│  │ Thread 1 │  │ Thread 2 │  │ ... N │ │
│  │ (TSRM)   │  │ (TSRM)   │  │(TSRM) │ │
│  └──────────┘  └──────────┘  └───────┘ │
│       ↕              ↕            ↕     │
│     pipes          pipes        pipes   │
│  (persistent     (persistent   (persist │
│   protocol)       protocol)    proto)   │
│                                         │
│  HTTP Listener → Route → Dispatch       │
└─────────────────────────────────────────┘
```

1. **Worker threads** bootstrap the application once (autoloader, framework, service container)
2. Each thread sends a `0xAA` ready signal back to the master after bootstrap
3. Requests use the **persistent binary protocol** over pipes (same as process+persistent)
4. OPcache stays warm across requests; PHP state resets between requests
5. Thread+persistent uses pipes because the persistent event loop relies on the pipe-based wire protocol

## Auto-scaling

Turbine can dynamically adjust the worker pool based on load:

```toml
[server]
auto_scale = true
min_workers = 2
max_workers = 16
scale_down_idle_secs = 5
```

- When all workers are busy, new workers are spawned (up to `max_workers`)
- When workers are idle for `scale_down_idle_secs`, excess workers are terminated (down to `min_workers`)
- Scaling decisions happen in a background task

## Named Worker Pools

Route specific URL patterns to dedicated worker groups. Useful for separating fast and slow endpoints:

```toml
# Fast API handlers (default pool handles these)

[[worker_pools]]
match_path = "/api/reports/*"
min_workers = 2
max_workers = 4
name = "reports"

[[worker_pools]]
match_path = "/webhook"
min_workers = 1
max_workers = 2
name = "webhooks"

[[worker_pools]]
match_path = "/api/export/*"
min_workers = 1
max_workers = 3
name = "exports"
```

### How Routing Works

1. Request arrives at Turbine
2. Named pools are checked first (in order)
3. Pattern matching: `/api/reports/*` matches `/api/reports/monthly`, `/api/reports/q1/sales`
4. Exact match: `/webhook` matches only `/webhook`
5. If no named pool matches, the request goes to the default pool

This prevents slow endpoints (report generation, file exports) from blocking fast API responses.

## Request Queue (max_wait_time)

When all workers are busy, requests are queued. The `max_wait_time` setting controls how long a request waits before receiving a 503:

```toml
[server]
max_wait_time = 5  # Return 503 after 5 seconds in queue
```

Set to `0` to use the `request_timeout` value as the queue limit.

## Worker Respawn

Workers are automatically respawned after `worker_max_requests` to prevent memory leaks.

### Per-Request Workers (persistent_workers = false)

Per-request workers complete the current request, exit, and are replaced:
- **Process mode**: A new process is forked by the master
- **Thread mode**: A new thread is spawned with a fresh TSRM context

### Persistent Workers — Graceful Recycling (persistent_workers = true)

Persistent workers (both process and thread mode) use a **graceful shutdown protocol** to avoid dropping in-flight requests:

1. Turbine sends a `0xFF` shutdown byte via the worker's pipe (not SIGTERM)
2. The worker reads the shutdown signal in its event loop and exits cleanly
3. Turbine polls `waitpid` with `WNOHANG` for up to 10ms
4. If the worker hasn't exited, a `SIGTERM` fallback is sent
5. A fresh worker is respawned inline in the same pool slot

This ensures **zero errors during recycling** — no request is lost because the old worker finishes its current work before exiting.

```toml
[server]
worker_max_requests = 10000  # Respawn after 10k requests
```

Set to `0` to disable automatic respawn (not recommended — persistent workers degrade without recycling).

## Monitoring Workers

Check worker status via the CLI:

```bash
turbine status 127.0.0.1:8080
```

This shows:
- Active workers and their state
- Requests processed per worker
- Memory usage
- Queue depth

## Tokio Async Threads

Turbine's HTTP stack runs on a multi-threaded Tokio runtime. The `tokio_worker_threads` option controls how many OS threads handle async I/O (connection accept, HTTP parsing, response writing).

```toml
[server]
tokio_worker_threads = 6  # Default: number of CPU cores
```

**Guidelines:**
- With few PHP workers (≤4), fewer Tokio threads (4–6) may be optimal
- With many PHP workers (≥8), keep the default (CPU core count) for best performance
- Reducing too much (e.g., 2) can bottleneck connection handling and reduce throughput by 30%+
- The Tokio threads and PHP workers compete for CPU cores — balance accordingly
