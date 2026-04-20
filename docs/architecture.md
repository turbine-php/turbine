# How Turbine Works

This document explains Turbine's architecture end-to-end: what runs in Rust, what runs in PHP, and how the two talk.

## High-Level View

Turbine is a full PHP runtime, not just an HTTP server in front of PHP-FPM. It embeds `libphp` directly in the process — no FastCGI, no proxying, no inter-process serialization of request/response bodies. From PHP's point of view, Turbine is the SAPI, the same way FrankenPHP or Swoole embed `libphp`.

```
Traditional:  Client → Nginx → PHP-FPM → OPcache → PHP
Turbine:      Client → Turbine (HTTP + PHP + OPcache + security + metrics)
```

A single binary terminates TLS, parses HTTP, applies middleware, executes PHP, and writes the response back to the socket.

## Two Layers

### Rust Layer (tokio + hyper)

Everything non-PHP lives in Rust and runs async:

- TCP accept, TLS termination (rustls)
- HTTP/1.1 and HTTP/2 parsing (hyper)
- Routing, virtual hosts
- Middleware: CORS, compression (gzip/br/zstd), rate limiting, security sandbox checks, response cache, early hints, X-Sendfile
- Static file serving
- Metrics, health checks, dashboard
- Hot reload file watcher
- ACME certificate renewal

A single tokio runtime drives all non-blocking I/O. The number of tokio async threads is independent of the PHP worker count and is tuned via `tokio_worker_threads` in `turbine.toml`.

### PHP Worker Pool

A fixed number of PHP workers, configured in `turbine.toml`. Each worker owns an isolated PHP interpreter. Two backends are available:

**Process mode** (default)
Workers are forked processes. Each worker has its own NTS or ZTS interpreter, fully isolated. A crash in one worker does not affect the others. This is the only mode that is safe for non-thread-safe extensions like Phalcon, Xdebug, and various PECL packages. IPC between Rust and the worker uses pipes.

**Thread mode** (requires PHP compiled with `--enable-zts`)
Workers are OS threads inside the main process. Each thread holds its own TSRM context, so each has a logically separate interpreter, but they share the process address space. Memory footprint is lower and IPC uses in-memory channels (no syscalls), but a fatal crash in any worker takes down the whole process, and non-thread-safe extensions cannot be used.

Both modes support two execution models:

- **Per-request** — standard `php_execute_script` lifecycle per request. Still faster than PHP-FPM because the SAPI is embedded and OPcache stays warm.
- **Persistent** (`persistent_workers = true`) — the worker boots the framework once and serves many requests against the already-booted app. See [Worker Lifecycle](worker-lifecycle.md).

## Request Flow

Step by step, what happens when a request arrives:

1. **Accept & TLS** — hyper accepts the TCP connection on the tokio runtime. If TLS is configured, rustls terminates it here.
2. **HTTP parsing** — hyper decodes the HTTP/1.1 or HTTP/2 request into headers, method, path, and a streaming body.
3. **Routing & middleware** — Turbine applies virtual host matching, CORS, rate limiting, response cache lookup, security sandbox checks, and any other middleware. This all runs in Rust, async, without touching PHP.
4. **Worker selection** — a PHP worker is picked from the pool (named pools are supported for route-based splitting).
5. **Dispatch** — the async task that received the request serializes the request into a `NativeRequest` struct and sends it to the selected worker through a tokio mpsc channel.
6. **PHP execution** — the worker picks up the message and runs PHP synchronously on its interpreter. Superglobals (`$_GET`, `$_POST`, `$_SERVER`, `$_COOKIE`) are populated from the `NativeRequest`. The PHP script runs to completion, either through the full request lifecycle or through the lightweight handler in persistent mode.
7. **Response** — the worker sends the response (status, headers, body) back through a return channel.
8. **Write** — the async task receives the response, applies compression and any post-middleware, and writes it to the socket via hyper.

Async stays on the Rust side. PHP runs synchronously inside its own thread or process. No request blocks another, because accept and I/O never wait on PHP — only the specific worker handling that request is busy until it completes.

## Shared State

Because workers run in parallel, state shared across requests cannot live in PHP globals (each worker has its own copy). Turbine exposes explicit primitives that live on the Rust side and are accessed from PHP as an extension:

- **SharedTable** — concurrent key/value store, safe across workers
- **AtomicCounter** — lock-free counters
- **TaskQueue** — fire-and-forget background jobs
- **WebSocket broadcast** — publish to all connected clients from any worker
- **Async I/O primitives** — parallel HTTP, non-blocking file operations

These primitives bypass PHP-level locking entirely. Synchronization is done by the Rust runtime using standard concurrency tools (DashMap, atomics, tokio channels).

See [Shared State](shared-state.md).

## Security Sandbox

The in-process security sandbox is a Rust middleware, not a PHP library:

- Execution whitelist (only listed `.php` files can be executed)
- Path traversal guard
- PHP `ini` hardening
- Heuristic SQL/code input filter (tiered by `paranoia_level`)
- Per-IP behaviour guard (scan detection, optional rate limit)

It runs in the same process as everything else with very low overhead per request. It is deliberately not a WAF — for full rule coverage, put Cloudflare, Coraza, or libmodsecurity + OWASP CRS in front of Turbine.

See [Security](security.md).

## Observability

- **Prometheus `/metrics`** — request counts, latency histograms, worker states, memory per worker, queue depth
- **Health checks** — `/healthz`, `/ready`
- **Structured JSON logs** — by default, from both Rust and PHP (`turbine_log()`)
- **Dashboard** — live view of workers and in-flight requests

See [Dashboard](dashboard.md), [Logging](logging.md).

## Worker Lifecycle in Persistent Mode

In persistent mode the worker doesn't fully restart between requests. Instead:

1. **Boot (once per worker)** — the worker loads `vendor/autoload.php`, runs `worker_boot.php` to boot the framework, and stores the app instance in `$GLOBALS`.
2. **Request loop** — superglobals are rearmed from the incoming request, `worker_handler.php` is included to dispatch through the booted app, optional `worker_cleanup.php` resets per-request state (sessions, auth, scoped services).
3. **Recycle** — after `worker_max_requests` requests, the worker is gracefully replaced, reclaiming any leaked memory.

See [Worker Lifecycle](worker-lifecycle.md).

## Configuration

Everything is in a single `turbine.toml`: listener, TLS, worker count and mode, PHP `ini` overrides, extensions, limits, logging, sandbox, compression, CORS, cache, virtual hosts. One file, version-controllable, diff-friendly.

See [Config Reference](config.md).

## Summary

- Rust handles I/O, HTTP, TLS, routing, middleware, and shared state.
- PHP runs in an isolated worker (process or thread) driven by a channel.
- Async lives in Rust, PHP stays synchronous — no PHP code has to change.
- Shared state is exposed as a Rust-backed PHP extension.
- One binary, one config file, no FPM, no external web server.
