# Known Issues

Issues are marked by which worker mode they affect:

- **[both]** — process mode and thread mode
- **[process]** — process mode only (`worker_mode = "process"`, NTS or ZTS PHP)
- **[thread]** — thread mode only (`worker_mode = "thread"`, ZTS PHP required)

---

## Worker Crash Isolation

**[process]** — Each worker runs in its own forked process. A PHP fatal error, segfault, or unhandled exception in one worker kills only that worker; the master process automatically respawns a replacement. Other workers continue serving requests unaffected.

**[thread]** — All workers share one process. A PHP segfault or fatal that escapes thread boundaries (e.g. a corrupt PHP extension corrupting the heap) will bring down all workers at once. Turbine detects dead threads via the `alive` atomic flag and respawns them, but a hard crash will kill the whole server. Use process mode if you need strict crash isolation.

---

## pcntl Functions

**[both]** — `pcntl_signal()` and `pcntl_alarm()` work correctly in both modes.

**[process]** — `pcntl_fork()` must not be called from inside a worker. Each worker is already a forked child; a nested fork creates a grandchild process that inherits open pipes and PHP state, leading to pipe corruption and undefined behavior. Grandchild processes will not be reaped by Turbine.

**[thread]** — **`pcntl_fork()` is strictly forbidden in thread mode.** Calling `fork(2)` from inside a TSRM-initialized thread is undefined behavior: the child inherits only the calling thread but all mutexes, memory allocators, and TSRM state from all threads — the result is an immediate deadlock or crash. `pcntl_exec()` is disabled via `disabled_functions` for the same reason.

---

## Static PHP Variables

**[both]** — PHP `static` variables and module-level globals are **per-worker**, not shared across workers. In both process and thread mode, each worker has its own PHP interpreter context (forked address space for process mode; separate TSRM context per thread in thread mode). Code that assumes a singleton `static $cache` will be shared across requests has a single worker as its scope, not the whole pool.

---

## OPcache

### File Changes Not Detected

**[both]** — OPcache caches compiled scripts in shared memory. After deploying code changes, workers must be restarted to pick up the new files:

```bash
# Send SIGHUP to restart workers gracefully
kill -HUP $(cat /var/run/turbine.pid)
# or simply
turbine serve   # restart the server
```

Enable the file watcher during development to restart workers automatically:

```toml
[watcher]
enabled = true
paths = ["app/", "src/", "config/"]
```

### OPcache Preload

**[both]** — OPcache preloading (`preload_script`) runs once before any worker is forked or spawned. In process mode, preloaded classes are shared via copy-on-write in the child processes' address space. In thread mode, preloaded classes are shared directly across threads (no copy). Both modes benefit equally from preloading.

### JIT and eval()

**[both]** — JIT compilation does not apply to code executed via `eval()`. This is a PHP limitation, not a Turbine issue.

---

## Memory Growth

**[process]** — Memory allocated in one worker process is isolated. When a worker reaches `worker_max_requests`, Turbine sends it a shutdown signal and forks a fresh replacement. Memory leaks are bounded to one worker at a time.

**[thread]** — All worker threads share the same heap. A memory leak in PHP code (circular references, growing static caches) accumulates in the shared process memory and affects all threads. `worker_max_requests` still applies — a thread that reaches the limit exits and is respawned, releasing its TSRM context — but PHP-level allocations that bypass TSRM (e.g. extension-level malloc outside ZMM) may not be freed. Zero `worker_max_requests` is inadvisable in thread mode for long-running servers.

```toml
[server]
worker_max_requests = 5000   # Lower in thread mode if memory growth is observed
```

---

## Persistent Worker Degradation Without Recycling

**[both]** — When `persistent_workers = true` and `worker_max_requests = 0` (recycling disabled), persistent workers degrade in throughput over sustained load. In benchmarks, RPS can drop from ~42K to ~17K after extended operation due to accumulated PHP internal state, fragmentation, and GC pressure.

**Always set `worker_max_requests` to a positive value** (10,000–50,000 recommended) when using persistent workers. Recycling is graceful: a `0xFF` shutdown byte is sent via the worker's pipe, the worker finishes its current work and exits cleanly, and a fresh worker is respawned inline with zero dropped requests.

---

## Persistent Worker IPC Overhead

**[process]** **[thread+persistent]** — Persistent workers (both process and thread mode) communicate with the Tokio runtime via pipes using the binary wire protocol. Each request incurs a pipe round-trip (~33µs). For trivial scripts (e.g. `echo "Hello"`), this overhead means per-request workers can outperform persistent workers. For real applications with >1ms execution time, the bootstrap savings of persistent mode vastly outweigh the IPC cost.

**[thread+per-request]** — Thread mode **without** persistence uses in-memory channels (`std::sync::mpsc`) instead of pipes. This eliminates all pipe syscalls and returns responses as Rust structs (zero-copy). This is the fastest IPC path in Turbine — there is no pipe overhead at all.

---

## FFI (Foreign Function Interface)

**[both]** — FFI works for basic C function calls. Ensure `ffi.enable` is set:

```toml
[php.ini]
"ffi.enable" = "true"
```

**[thread]** — C libraries loaded via FFI must be thread-safe. FFI calls into non-reentrant C libraries (those using global state without locks) will produce data races in thread mode. Use process mode if you rely on non-thread-safe C libraries via FFI.

---

## Sessions

**[both]** — Session files are stored in `session.save_path`. The directory must be writable:

```bash
mkdir -p /tmp/turbine-sessions
chmod 1733 /tmp/turbine-sessions
```

**[thread]** — Multiple threads can receive concurrent requests with the same session ID. PHP's file-based session handler uses `flock()` to serialize access, so concurrent session reads/writes are safe but may introduce brief lock contention under high concurrency. If this is a bottleneck, use a Redis session handler with proper atomic operations.

---

## `output_buffering` is forced to 0

**[both]** — Turbine captures PHP output via a custom `ub_write` SAPI callback that is drained during `php_request_shutdown`. Any non-zero `output_buffering` would retain response bytes inside PHP's internal buffer beyond the capture window and truncate responses larger than the buffer size.

The generated `php.ini` sets `output_buffering=0` and Turbine actively rejects overrides from `[php.ini]` in `turbine.toml` with a warn-level log entry. This is intentional and not configurable.

If you need output buffering in userland, use `ob_start()` / `ob_get_clean()` in PHP — they compose correctly with the SAPI capture.

---

## Runtime Library Path

**[both]** — Turbine links against `libphp.dylib` (macOS) or `libphp.so` (Linux) at runtime. The library must be locatable by the dynamic linker.

```bash
# macOS
export DYLD_LIBRARY_PATH="$PWD/vendor/php-embed-zts/lib"

# Linux
export LD_LIBRARY_PATH="$PWD/vendor/php-embed-zts/lib"
# or install to system path and run: ldconfig
```

The `build.sh` script sets `-Wl,-rpath` so that the installed `turbine` binary finds `libphp` relative to the path it was compiled against. If you move the `vendor/` directory after building, update the rpath or re-export the library path variable.

### macOS System Integrity Protection (SIP)

**[both]** — macOS SIP strips `DYLD_LIBRARY_PATH` when launching processes via `sudo` or certain system launchers. Run Turbine directly from a terminal session without `sudo`. For launchd/systemd service files, set the environment variable explicitly in the service definition.

---

## Unsupported Features

| Feature | Status | Workaround |
|---------|--------|------------|
| WebSockets | Not supported | Use a dedicated WebSocket server or reverse proxy |
| HTTP/3 (QUIC) | Not supported | Front with Caddy or Cloudflare |
| Mercure | Not built-in | Use a standalone Mercure hub |
| `pcntl_fork()` | Forbidden in thread mode; unsafe in process mode | Do not use inside workers |
| Shared PHP globals across workers | Not possible (by design) | Use Redis, database, or shared memory |
