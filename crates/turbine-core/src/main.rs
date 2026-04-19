use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use clap::Parser;

/// Use mimalloc as the global allocator.
///
/// mimalloc (Microsoft Research) outperforms glibc malloc and jemalloc on
/// highly-threaded allocation-heavy workloads.  Typical gains: 5-10%
/// throughput, 20-40% lower p99 under concurrent load, smaller RSS due to
/// aggressive segment reuse and better thread-local caches.  Matches what
/// Bun and Deno ship.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use turbine_cache::{CacheConfig, ResponseCache};
use turbine_engine::{PhpEngine, PhpIniOverrides};
use turbine_metrics::MetricsCollector;
use turbine_security::{BehaviourConfig, SecurityConfig as SecConfig, SecurityLayer};
use turbine_worker::persistent::{encode_request, read_ready_signal, PersistentRequest};
use turbine_worker::pool::{
    read_native_response_from_fd, worker_event_loop_channel, worker_event_loop_native, PoolConfig,
    WorkerMode, WorkerPool,
};
use turbine_worker::{encode_native_request, write_to_fd, NativeResponse};

mod acme;
mod cli;
mod compat;
mod config;
mod dashboard;
mod embed;
mod features;
#[cfg(all(feature = "io-uring", target_os = "linux"))]
mod io_uring_backend;
mod path_guard;
mod shared_table;

use path_guard::RequestGuard;
use shared_table::{SharedTable, TableError};

use cli::{Cli, Command};
use compat::{AppDetector, AppStructure, FullHttpRequest};
use config::RuntimeConfig;

const TURBINE_STATUS_MARKER: &str = "__TURBINE_STATUS__\t";
const TURBINE_HEADER_MARKER: &str = "__TURBINE_HEADER__\t";
const TURBINE_BODY_MARKER: &str = "__TURBINE_BODY__\n";

/// Result shared between the singleflight leader and its followers.
///
/// Cloned once per waiter — that's cheap because `body` is already a
/// `Bytes` (refcounted, zero-copy clone) and headers are few and small.
#[derive(Clone)]
struct CoalescedResponse {
    status: u16,
    content_type: String,
    body: Bytes,
    headers: Vec<(String, String)>,
}

/// Advise the kernel that this process's anonymous memory should use
/// transparent huge pages (2 MiB on x86_64/aarch64).  Applied to the
/// full address space via `PR_SET_THP_DISABLE=0` + a broad `madvise`.
///
/// No-op on macOS (kernel has no THP equivalent at this layer) and on
/// Linux hosts whose sysfs is configured `transparent_hugepage = never`.
#[inline]
fn hugepage_hint_process() {
    #[cfg(target_os = "linux")]
    unsafe {
        // Re-enable THP at the process scope if it was disabled (some
        // container runtimes inherit PR_SET_THP_DISABLE=1).
        let _ = libc::prctl(libc::PR_SET_THP_DISABLE, 0, 0, 0, 0);

        // Suggest hugepages for the entire heap range.  The address 0
        // with MADV_HUGEPAGE + length=0 isn't portable, so we instead
        // hint on a large anonymous region.  Future allocations inherit
        // the madvise from the VMA they land in; glibc / mimalloc pick
        // this up when they request fresh arenas.
        extern "C" {
            static __bss_start: libc::c_void;
            static _end: libc::c_void;
        }
        // Best-effort: hint the BSS/data range.  The call is advisory;
        // failures (EINVAL on non-2MiB-aligned ranges) are ignored.
        let start = std::ptr::addr_of!(__bss_start) as usize;
        let end = std::ptr::addr_of!(_end) as usize;
        if end > start {
            let page = 2 * 1024 * 1024usize;
            let aligned_start = start.next_multiple_of(page);
            let aligned_end = end & !(page - 1);
            if aligned_end > aligned_start {
                let _ = libc::madvise(
                    aligned_start as *mut libc::c_void,
                    aligned_end - aligned_start,
                    libc::MADV_HUGEPAGE,
                );
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Init) => cmd_init(),
        Some(Command::Config) => cmd_config(),
        Some(Command::Info) => cmd_info(),
        Some(Command::Check { config }) => cmd_check(config),
        Some(Command::Status { address }) => cmd_status(&address),
        Some(Command::CacheClear { address }) => cmd_cache_clear(&address),
        Some(Command::Serve {
            listen,
            workers,
            config,
            root,
            tls_cert,
            tls_key,
            request_timeout,
            access_log,
        }) => {
            cmd_serve(
                listen,
                workers,
                config,
                root,
                tls_cert,
                tls_key,
                request_timeout,
                access_log,
            );
        }
        None => cmd_serve(None, None, None, None, None, None, None, None),
    }
}

/// `turbine init` — generate a default turbine.toml
fn cmd_init() {
    let path = std::env::current_dir()
        .unwrap_or_default()
        .join("turbine.toml");
    if path.exists() {
        eprintln!("turbine.toml already exists");
        std::process::exit(1);
    }
    std::fs::write(&path, RuntimeConfig::template()).expect("Failed to write turbine.toml");
    println!("Created {}", path.display());
}

/// `turbine check` — validate turbine.toml configuration
fn cmd_check(config_path: Option<String>) {
    let path = config_path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_default()
            .join("turbine.toml")
            .to_string_lossy()
            .to_string()
    });

    // Step 1: Check file exists
    if !std::path::Path::new(&path).exists() {
        eprintln!("\x1b[31m✗\x1b[0m Config file not found: {path}");
        std::process::exit(1);
    }

    // Step 2: Read and parse TOML
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\x1b[31m✗\x1b[0m Failed to read {path}: {e}");
            std::process::exit(1);
        }
    };

    let config: RuntimeConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("\x1b[31m✗\x1b[0m TOML parse error in {path}:");
            eprintln!("  {e}");
            std::process::exit(1);
        }
    };

    // Step 3: Run semantic validation
    let (errors, warnings) = config.check();

    println!("\x1b[1mTurbine Configuration Check\x1b[0m");
    println!("  File: {path}");
    println!();

    // Summary of key settings
    println!("\x1b[1mSettings:\x1b[0m");
    println!("  workers          = {}", config.server.workers);
    println!("  worker_mode      = {}", config.server.worker_mode);
    println!(
        "  persistent       = {}",
        config.server.persistent_workers.unwrap_or(false)
    );
    println!("  listen           = {}", config.server.listen);
    println!("  request_timeout  = {}s", config.server.request_timeout);
    println!("  max_requests     = {}", config.server.worker_max_requests);
    if let Some(t) = config.server.tokio_worker_threads {
        println!("  tokio_threads    = {t}");
    }
    println!("  security         = {}", config.security.enabled);
    println!("  compression      = {}", config.compression.enabled);
    println!("  cache            = {}", config.cache.enabled);
    println!("  tls              = {}", config.server.tls.enabled);
    if !config.virtual_hosts.is_empty() {
        println!("  virtual_hosts    = {}", config.virtual_hosts.len());
        for vhost in &config.virtual_hosts {
            let aliases = if vhost.aliases.is_empty() {
                String::new()
            } else {
                format!(" (+ {})", vhost.aliases.join(", "))
            };
            println!("    {} → {}{}", vhost.domain, vhost.root, aliases);
        }
    }
    println!();

    let mut has_issues = false;

    if !errors.is_empty() {
        has_issues = true;
        println!("\x1b[31m✗ {} error(s):\x1b[0m", errors.len());
        for e in &errors {
            println!("  \x1b[31m•\x1b[0m {e}");
        }
        println!();
    }

    if !warnings.is_empty() {
        has_issues = true;
        println!("\x1b[33m⚠ {} warning(s):\x1b[0m", warnings.len());
        for w in &warnings {
            println!("  \x1b[33m•\x1b[0m {w}");
        }
        println!();
    }

    if has_issues {
        if !errors.is_empty() {
            eprintln!("\x1b[31m✗ Configuration has errors that must be fixed.\x1b[0m");
            std::process::exit(1);
        } else {
            println!("\x1b[33m⚠ Configuration is valid but has warnings.\x1b[0m");
        }
    } else {
        println!("\x1b[32m✓ Configuration is valid. No errors or warnings.\x1b[0m");
    }
}

/// `turbine config` — display current configuration
fn cmd_config() {
    let config = RuntimeConfig::load();
    println!("{config:#?}");
}

/// `turbine info` — show PHP engine information
fn cmd_info() {
    let engine = match PhpEngine::init() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to init PHP: {e}");
            std::process::exit(1);
        }
    };
    println!("PHP version: {}", engine.php_version());
    println!("Embed SAPI:  active");
    println!("Turbine:     v{}", env!("CARGO_PKG_VERSION"));
}

/// `turbine status` — query a running server's status endpoint
fn cmd_status(address: &str) {
    let url = format!("http://{address}/_/status");
    match std::net::TcpStream::connect(address) {
        Ok(mut stream) => {
            use std::io::{BufRead, BufReader, Write};
            let req =
                format!("GET /_/status HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(req.as_bytes());
            let mut response = String::new();
            let mut reader = BufReader::new(&stream);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => response.push_str(&line),
                    Err(_) => break,
                }
            }
            if let Some(body_start) = response.find("\r\n\r\n") {
                print!("{}", &response[body_start + 4..]);
            } else {
                eprintln!("Invalid response from {url}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Cannot connect to {address}: {e}");
            eprintln!("Is the server running? Start with: turbine serve");
            std::process::exit(1);
        }
    }
}

/// `turbine cache:clear` — send cache clear command to running server
fn cmd_cache_clear(address: &str) {
    match std::net::TcpStream::connect(address) {
        Ok(mut stream) => {
            use std::io::{BufRead, BufReader, Write};
            let req = format!(
                "POST /_/cache/clear HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(req.as_bytes());
            let mut response = String::new();
            let mut reader = BufReader::new(&stream);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => response.push_str(&line),
                    Err(_) => break,
                }
            }
            if let Some(body_start) = response.find("\r\n\r\n") {
                print!("{}", &response[body_start + 4..]);
            } else {
                eprintln!("Invalid response");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Cannot connect to {address}: {e}");
            std::process::exit(1);
        }
    }
}

/// Lock-free dispatch for thread-mode workers.
///
/// Replaces the `Mutex<WorkerPool>` → `get_idle_worker()` hot path with a
/// channel-based approach.  Idle worker indices flow through a Semaphore +
/// lock-free queue; dispatch uses in-memory channels with zero pipe syscalls.
///
/// **Critical design**: `get_idle()` uses a `tokio::sync::Semaphore` so that ALL
/// waiting tasks can independently await a permit without holding any lock.
/// The previous design held a `tokio::sync::Mutex` across an `.await` which
/// serialised all waiter wakeups — a classic async anti-pattern.
///
/// Cold paths (reap, respawn, shutdown) still use the pool mutex.
struct ThreadDispatch {
    /// Semaphore with one permit per idle worker.  Tasks acquire a permit
    /// to claim a worker; `return_idle` adds a permit back.
    idle_sem: tokio::sync::Semaphore,
    /// Queue of idle worker indices.  Protected by a brief parking_lot Mutex
    /// (O(1) push/pop, never held across .await).
    idle_queue: parking_lot::Mutex<std::collections::VecDeque<usize>>,
    /// Per-worker pipe fds: `(cmd_fd, resp_fd)`.
    /// RwLock because fds can change on worker respawn (rare cold path).
    /// Empty when using in-memory channels.
    worker_fds: parking_lot::RwLock<Vec<(std::os::unix::io::RawFd, std::os::unix::io::RawFd)>>,
    /// Per-worker request senders (in-memory channel mode).
    /// Empty when using pipe-based mode.
    request_txs: Vec<std::sync::mpsc::Sender<Vec<u8>>>,
    /// Per-worker response receivers (in-memory channel mode).
    /// Empty when using pipe-based mode.
    response_rxs: Vec<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<NativeResponse>>>,
    /// Per-worker request counter (for max_requests enforcement in the hot
    /// path).  Incremented on each successful dispatch.  When a worker
    /// reaches `max_requests`, `get_idle` skips it so the reaper can
    /// recycle it before any new traffic lands on a stale interpreter.
    requests_served: Vec<std::sync::atomic::AtomicU64>,
    /// Per-worker unhealthy flag.  Set to `true` when a send/decode fails
    /// (EPIPE, EOF, decode error).  Workers marked unhealthy are skipped
    /// by `get_idle` until the reaper respawns them and resets the flag.
    /// Prevents the well-known "dead fd still in idle queue" race where
    /// a persistent worker crashed mid-request and the next dispatch
    /// picks the same index, hits the dead pipe, and returns HTTP 502.
    unhealthy: Vec<std::sync::atomic::AtomicBool>,
    /// Max requests per worker before recycling (0 = unlimited, same
    /// semantics as `ServerConfig.worker_max_requests`).
    max_requests_per_worker: u64,
}

impl ThreadDispatch {
    /// Create a pipe-based ThreadDispatch (legacy / persistent workers).
    fn new(fds: Vec<(std::os::unix::io::RawFd, std::os::unix::io::RawFd)>) -> Self {
        let count = fds.len();
        let mut queue = std::collections::VecDeque::with_capacity(count);
        for i in 0..count {
            queue.push_back(i);
        }
        let mut requests_served = Vec::with_capacity(count);
        let mut unhealthy = Vec::with_capacity(count);
        for _ in 0..count {
            requests_served.push(std::sync::atomic::AtomicU64::new(0));
            unhealthy.push(std::sync::atomic::AtomicBool::new(false));
        }
        ThreadDispatch {
            idle_sem: tokio::sync::Semaphore::new(count),
            idle_queue: parking_lot::Mutex::new(queue),
            worker_fds: parking_lot::RwLock::new(fds),
            request_txs: Vec::new(),
            response_rxs: Vec::new(),
            requests_served,
            unhealthy,
            max_requests_per_worker: 0,
        }
    }

    /// Set the max_requests_per_worker threshold used by `get_idle` to
    /// skip workers that have already served their quota (so the reaper
    /// can recycle them without racing with new traffic).
    fn set_max_requests(&mut self, max: u64) {
        self.max_requests_per_worker = max;
    }

    /// Mark a worker as unhealthy (called on send/decode failure). The
    /// worker will be skipped by `get_idle` until the reaper clears the
    /// flag after respawning the underlying process/thread.
    fn mark_unhealthy(&self, idx: usize) {
        if idx < self.unhealthy.len() {
            self.unhealthy[idx].store(true, std::sync::atomic::Ordering::Release);
        }
    }

    /// Clear the unhealthy flag and reset the request counter for a
    /// freshly respawned worker.  Called from the reaper path.
    #[allow(dead_code)]
    fn mark_healthy(&self, idx: usize) {
        if idx < self.unhealthy.len() {
            self.unhealthy[idx].store(false, std::sync::atomic::Ordering::Release);
            self.requests_served[idx].store(0, std::sync::atomic::Ordering::Release);
        }
    }

    /// Increment the per-worker request counter.  Called on every
    /// successful dispatch.
    fn record_served(&self, idx: usize) {
        if idx < self.requests_served.len() {
            self.requests_served[idx].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Returns true when `idx` is still safe to dispatch to.  Used by
    /// `get_idle` to reject dead pipes / quota-exhausted workers.
    fn is_pickable(&self, idx: usize) -> bool {
        if idx >= self.unhealthy.len() {
            return false;
        }
        if self.unhealthy[idx].load(std::sync::atomic::Ordering::Acquire) {
            return false;
        }
        if self.max_requests_per_worker > 0 {
            let served = self.requests_served[idx].load(std::sync::atomic::Ordering::Relaxed);
            if served >= self.max_requests_per_worker {
                return false;
            }
        }
        true
    }

    /// Spawn channel-based worker threads and return a `ThreadDispatch` that
    /// communicates entirely via in-memory channels (zero pipe syscalls).
    ///
    /// Each worker runs `worker_event_loop_channel`.  The caller must also
    /// register workers in the pool via `pool.register_channel_thread()` for
    /// lifecycle tracking.
    ///
    /// Returns `(ThreadDispatch, Vec<(Arc<AtomicBool>, u64)>)` — the dispatch
    /// handle and per-worker `(alive_flag, thread_id)` pairs.
    fn spawn_channel_workers(
        count: usize,
        pin_workers: bool,
    ) -> (Self, Vec<(Arc<std::sync::atomic::AtomicBool>, u64)>) {
        use std::sync::atomic::{AtomicBool, Ordering};

        static CHAN_THREAD_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1_000_000);

        let mut request_txs = Vec::with_capacity(count);
        let mut response_rxs_raw: Vec<tokio::sync::mpsc::UnboundedReceiver<NativeResponse>> =
            Vec::with_capacity(count);
        let mut worker_info: Vec<(Arc<AtomicBool>, u64)> = Vec::with_capacity(count);

        // Validate ZTS at runtime
        let is_zts = unsafe { turbine_php_sys::turbine_php_is_thread_safe() };
        if is_zts == 0 {
            panic!("Thread worker mode requires PHP compiled with ZTS (--enable-zts). Current PHP is NTS.");
        }

        for i in 0..count {
            let (req_tx, req_rx) = std::sync::mpsc::channel::<Vec<u8>>();
            let (resp_tx, resp_rx) = tokio::sync::mpsc::unbounded_channel::<NativeResponse>();

            let alive = Arc::new(AtomicBool::new(true));
            let alive_clone = alive.clone();
            let thread_id = CHAN_THREAD_ID.fetch_add(1, Ordering::Relaxed);

            std::thread::Builder::new()
                .name(format!("turbine-ch-worker-{i}"))
                .spawn(move || {
                    if pin_workers {
                        let ncpus = std::thread::available_parallelism()
                            .map(|n| n.get())
                            .unwrap_or(1);
                        turbine_worker::pin_to_core(i % ncpus);
                    }
                    let init_rc = unsafe { turbine_php_sys::turbine_thread_init() };
                    if init_rc != 0 {
                        tracing::error!(worker = i, "Failed to init TSRM for channel worker");
                        alive_clone.store(false, Ordering::Release);
                        return;
                    }

                    let response_fn = move |resp: NativeResponse| {
                        let _ = resp_tx.send(resp);
                    };
                    worker_event_loop_channel(req_rx, response_fn);

                    unsafe {
                        turbine_php_sys::turbine_thread_cleanup();
                    }
                    alive_clone.store(false, Ordering::Release);
                    tracing::debug!(worker = i, "Channel worker thread exited");
                })
                .expect("Failed to spawn channel worker thread");

            request_txs.push(req_tx);
            response_rxs_raw.push(resp_rx);
            worker_info.push((alive, thread_id));
        }

        // Consume the "ready" signal from each worker.
        let mut idle_queue = std::collections::VecDeque::with_capacity(count);
        for (i, rx) in response_rxs_raw.iter_mut().enumerate() {
            let mut ready = false;
            for _ in 0..500 {
                match rx.try_recv() {
                    Ok(resp) if resp.success => {
                        tracing::debug!(idx = i, "Channel worker ready");
                        idle_queue.push_back(i);
                        ready = true;
                        break;
                    }
                    Ok(_resp) => {
                        tracing::warn!(idx = i, "Channel worker sent non-success ready");
                        idle_queue.push_back(i);
                        ready = true;
                        break;
                    }
                    Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                }
            }
            if !ready {
                tracing::error!(idx = i, "Channel worker failed to send ready signal");
                idle_queue.push_back(i); // add anyway
            }
        }

        let idle_count = idle_queue.len();
        let response_rxs: Vec<_> = response_rxs_raw
            .into_iter()
            .map(|rx| tokio::sync::Mutex::new(rx))
            .collect();

        let mut requests_served = Vec::with_capacity(count);
        let mut unhealthy = Vec::with_capacity(count);
        for _ in 0..count {
            requests_served.push(std::sync::atomic::AtomicU64::new(0));
            unhealthy.push(std::sync::atomic::AtomicBool::new(false));
        }

        let td = ThreadDispatch {
            idle_sem: tokio::sync::Semaphore::new(idle_count),
            idle_queue: parking_lot::Mutex::new(idle_queue),
            worker_fds: parking_lot::RwLock::new(Vec::new()),
            request_txs,
            response_rxs,
            requests_served,
            unhealthy,
            max_requests_per_worker: 0,
        };
        (td, worker_info)
    }

    /// Whether this dispatch uses in-memory channels (true) vs pipe fds (false).
    fn has_channels(&self) -> bool {
        !self.request_txs.is_empty()
    }

    /// Await the next idle worker index with a timeout.
    ///
    /// Uses Semaphore so ALL waiting tasks can independently await a permit
    /// without holding any lock.  When a permit is available, the task pops
    /// the worker index from the queue (O(1) brief lock, never across .await).
    ///
    /// # Dispatch policy: hot-worker-first (LIFO)
    ///
    /// We pop from the **back** of the deque, which means the most
    /// recently returned worker — whose OPcache, Zend arenas, and CPU
    /// L2/L3 caches are still warm — gets the next request.  This is
    /// the same locality trick Go's scheduler uses for its local run
    /// queues.  In this architecture each worker handles exactly one
    /// request at a time, so classical power-of-two-choices (pick 2
    /// random, send to the less loaded) reduces to "all idle workers
    /// have load 0" — useless.  LIFO is the meaningful optimization.
    ///
    /// Cold workers still get recycled naturally: whenever the idle
    /// queue is empty at dispatch time (peak load), new requests fan
    /// out to every worker including cold ones.
    ///
    /// # Health filtering
    ///
    /// Workers flagged unhealthy (send/decode failed last time) or past
    /// their `max_requests` quota are skipped and NOT returned to the
    /// idle queue — the background reaper will respawn them and call
    /// `mark_healthy` to clear the flags.  If every permit we acquire
    /// points to an unhealthy worker we give up and return `None` so
    /// the caller can trigger a reap cycle instead of looping forever.
    async fn get_idle(&self, timeout: std::time::Duration) -> Option<usize> {
        // Cap the number of skips so we can't spin forever if every
        // worker is unhealthy at once (shouldn't happen — the reaper
        // runs every 100ms — but defend against pathological cases).
        let max_skips = self.unhealthy.len().max(1) * 2;
        let mut skipped = 0usize;
        loop {
            let permit = match tokio::time::timeout(timeout, self.idle_sem.acquire()).await {
                Ok(Ok(permit)) => permit,
                _ => return None,
            };
            permit.forget(); // consumed; return_idle will add_permits(1)
            let idx = self.idle_queue.lock().pop_back();
            let idx = match idx {
                Some(i) => i,
                None => {
                    // Safety net: restore the permit if queue is unexpectedly empty
                    self.idle_sem.add_permits(1);
                    return None;
                }
            };

            if self.is_pickable(idx) {
                return Some(idx);
            }

            // Unhealthy or quota-exhausted — drop this worker on the floor
            // (do NOT add the permit back; the reaper will respawn it and
            // call `mark_healthy` + `return_idle` which restores the
            // permit).  This naturally applies back-pressure while the
            // pool is shrinking.
            skipped += 1;
            tracing::debug!(
                worker = idx,
                skipped = skipped,
                "get_idle: skipping unhealthy/exhausted worker"
            );
            if skipped >= max_skips {
                tracing::warn!(
                    skipped = skipped,
                    "get_idle: giving up after too many unhealthy workers"
                );
                return None;
            }
        }
    }

    /// Return a worker to the idle pool.  Pushes to the back so the
    /// next `get_idle()` picks this hot worker first (LIFO — see
    /// `get_idle` docs).
    fn return_idle(&self, idx: usize) {
        self.idle_queue.lock().push_back(idx);
        self.idle_sem.add_permits(1);
    }

    /// Get (cmd_fd, resp_fd) for worker `idx` (pipe mode only).
    fn fds(&self, idx: usize) -> (std::os::unix::io::RawFd, std::os::unix::io::RawFd) {
        self.worker_fds.read()[idx]
    }

    /// Update fds after a worker respawn.
    #[allow(dead_code)]
    fn update_fds(
        &self,
        idx: usize,
        cmd_fd: std::os::unix::io::RawFd,
        resp_fd: std::os::unix::io::RawFd,
    ) {
        let mut fds = self.worker_fds.write();
        if idx < fds.len() {
            fds[idx] = (cmd_fd, resp_fd);
        }
    }

    /// Bulk-refresh all worker fds.  Called by the background reaper after
    /// dead workers have been respawned so the dispatch uses up-to-date
    /// pipe fds instead of the stale ones left behind by the dead PIDs.
    /// If `new_fds` has more entries than before (scale-up) the extra
    /// workers are added to the idle pool.  Shrinks are not applied here
    /// (handled by shrink_one).
    ///
    /// Clears the unhealthy flag for every worker — after a respawn,
    /// the pipe fds are fresh, so any previous "dead pipe" verdict no
    /// longer applies.  Also resets the per-worker request counter so
    /// `max_requests` enforcement starts from zero on the new interpreter.
    fn refresh_fds(&self, new_fds: Vec<(std::os::unix::io::RawFd, std::os::unix::io::RawFd)>) {
        let prev_len = {
            let mut fds = self.worker_fds.write();
            let prev = fds.len();
            *fds = new_fds;
            prev
        };
        let new_len = self.worker_fds.read().len();

        // Reset health/counter state for all known worker slots.
        for i in 0..new_len.min(self.unhealthy.len()) {
            self.unhealthy[i].store(false, std::sync::atomic::Ordering::Release);
            self.requests_served[i].store(0, std::sync::atomic::Ordering::Release);
        }

        if new_len > prev_len {
            // Newly added workers — make them idle and grow the semaphore.
            let growable = new_len.min(self.unhealthy.len()) - prev_len;
            if growable > 0 {
                let mut q = self.idle_queue.lock();
                for i in prev_len..prev_len + growable {
                    q.push_back(i);
                }
                drop(q);
                self.idle_sem.add_permits(growable);
            }
            if new_len > self.unhealthy.len() {
                // Health atomics are sized at startup; scale-ups beyond
                // the initial cap would require reallocation (skipped
                // here — auto_scale is off by default and this branch
                // is best-effort). Log once so this is visible.
                tracing::warn!(
                    requested = new_len,
                    cap = self.unhealthy.len(),
                    "ThreadDispatch: scale-up beyond initial worker cap; health tracking limited to cap"
                );
            }
        }
    }

    /// Send a request payload to worker `idx` via in-memory channel.
    fn send_request(&self, idx: usize, payload: Vec<u8>) -> Result<(), String> {
        self.request_txs[idx]
            .send(payload)
            .map_err(|_| "channel worker dead".to_string())
    }

    /// Await the next response from worker `idx` via in-memory channel.
    async fn recv_response(&self, idx: usize) -> Option<NativeResponse> {
        let mut rx = self.response_rxs[idx].lock().await;
        rx.recv().await
    }
}

/// RAII guard that returns a worker index to the idle pool on drop.
/// Prevents worker leaks when an async task is cancelled (e.g. client disconnect).
struct IdleGuard {
    td: Arc<ThreadDispatch>,
    idx: Option<usize>,
}

impl IdleGuard {
    fn new(td: Arc<ThreadDispatch>, idx: usize) -> Self {
        IdleGuard { td, idx: Some(idx) }
    }
    /// Consume the guard without returning the worker (e.g. on send error
    /// where we manually call return_idle).
    #[allow(dead_code)]
    fn defuse(&mut self) {
        self.idx = None;
    }
}

impl Drop for IdleGuard {
    fn drop(&mut self) {
        if let Some(idx) = self.idx.take() {
            self.td.return_idle(idx);
        }
    }
}

struct ServerState {
    listen: String,
    worker_count: usize,
    is_tls: bool,
    /// Request execution timeout (0 = no timeout).
    request_timeout: std::time::Duration,
    /// Access log file writer (None = disabled).
    access_log: Option<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>,
    /// Gzip/brotli/zstd compression settings.
    compression_enabled: bool,
    compression_min_size: usize,
    compression_level: u32,
    compression_algorithms: Vec<String>,
    /// Pre-loaded custom error page HTML for 404.
    error_page_404: std::sync::RwLock<Option<Vec<u8>>>,
    /// Pre-loaded custom error page HTML for 500.
    error_page_500: std::sync::RwLock<Option<Vec<u8>>>,
    /// CORS configuration.
    cors: config::CorsConfig,
    /// PID file path (for cleanup on shutdown).
    pid_file: Option<String>,
    /// Temporary directory for file uploads.
    upload_tmp_dir: String,
    /// Maximum accepted request body size in bytes (derived from
    /// `php.post_max_size`). Requests exceeding this receive HTTP 413
    /// before the body is read — DoS protection against huge uploads.
    /// `None` = no limit.
    max_body_bytes: Option<usize>,
    /// Sandbox: execution mode ("strict" or "framework").
    execution_mode: String,
    /// Sandbox: whitelist of executable PHP files (strict mode).
    execution_whitelist: Vec<String>,
    /// Sandbox: data directories (no PHP execution allowed).
    data_directories: Vec<String>,
    /// Sandbox: upload security configuration.
    upload_security: compat::UploadSecurityConfig,
    request_guard: RequestGuard,
    security: SecurityLayer,
    metrics: MetricsCollector,
    cache: ResponseCache,
    /// Request coalescer ("singleflight") — collapses concurrent requests
    /// for the same cacheable URL into a single PHP execution.  Only the
    /// leader invokes PHP; followers await and receive a clone of its
    /// response body.  Key is `"METHOD:path"` — same scheme as the
    /// response cache.
    cache_coalescer: Arc<turbine_cache::Coalescer<CoalescedResponse>>,
    app_structure: AppStructure,
    php_bootstrap: String,

    /// Channel to send PHP execution requests to the dedicated PHP thread.
    /// Only used in single-process mode.
    php_tx: Option<tokio::sync::mpsc::Sender<PhpRequest>>,
    /// Worker pool for multi-process mode.
    /// Uses parking_lot::Mutex (no poisoning — won't panic if a thread panicked).
    worker_pool: Option<parking_lot::Mutex<WorkerPool>>,
    /// Worker backend mode (process or thread).
    worker_mode: WorkerMode,
    /// Lock-free dispatch for thread-mode workers (None in process mode).
    thread_dispatch: Option<Arc<ThreadDispatch>>,
    /// Whether workers are running in persistent mode (bootstrap-once protocol).
    persistent_workers: bool,
    /// App root path (for respawning persistent workers).
    persistent_app_root: String,
    /// Worker boot script (executed once per worker at startup).
    worker_boot: Option<String>,
    /// Worker handler script (included per request in lightweight lifecycle).
    worker_handler: Option<String>,
    /// Worker cleanup script (evaluated after each request in persistent mode).
    worker_cleanup: Option<String>,
    /// Whether to call session_start() before PHP execution.
    session_auto_start: bool,
    /// Semaphore limiting concurrent worker usage to worker_count permits.
    /// Requests wait here instead of getting an instant 503 when all workers are busy.
    worker_semaphore: Option<std::sync::Arc<tokio::sync::Semaphore>>,
    /// Auto-scaling configuration.
    auto_scale: bool,
    min_workers: usize,
    max_workers: usize,
    scale_down_idle_secs: u64,
    /// Watcher configuration.
    watcher_config: config::WatcherConfig,
    /// Early Hints (103) support.
    early_hints_enabled: bool,
    /// X-Sendfile / X-Accel-Redirect support.
    x_sendfile_enabled: bool,
    /// X-Sendfile base directory (resolved to absolute path).
    x_sendfile_root: Option<std::path::PathBuf>,
    /// Structured logging from PHP.
    structured_logging_enabled: bool,
    /// Maximum wait time in seconds for a free worker (0 = use request_timeout).
    max_wait_time: u64,
    /// Worker pool route configs for thread pool splitting.
    #[allow(dead_code)]
    worker_pool_routes: Vec<config::WorkerPoolRouteConfig>,
    /// Named worker pools for route-based thread pool splitting.
    named_pools: Vec<NamedWorkerPool>,
    /// ACME HTTP-01 challenge tokens (shared with background renewal task).
    acme_challenge_tokens: acme::ChallengeTokens,
    /// Dashboard and internal endpoints configuration.
    dashboard_enabled: bool,
    statistics_enabled: bool,
    dashboard_token: Option<String>,
    /// Shared in-memory table exposed to PHP via `/_/table/*`. `None` when
    /// the feature is disabled — all endpoints 404 and the PHP helpers are
    /// not injected.
    shared_table: Option<Arc<SharedTable>>,
    /// Virtual host map: lowercase domain → resolved vhost with pre-computed AppStructure.
    /// Empty = no virtual hosting (use global app_structure).
    virtual_hosts: std::collections::HashMap<String, Arc<VhostResolved>>,
}

/// Resolved virtual host — pre-computed at startup for zero-cost per-request lookup.
struct VhostResolved {
    /// Primary domain name (for logging).
    #[allow(dead_code)]
    domain: String,
    /// Pre-computed AppStructure (document_root + entry_point + resolve_path logic).
    app_structure: AppStructure,
}

/// A named worker pool for route-based thread pool splitting.
/// Routes matching a pool's pattern are dispatched to that pool instead of the default.
struct NamedWorkerPool {
    route: config::WorkerPoolRouteConfig,
    pool: parking_lot::Mutex<WorkerPool>,
    semaphore: std::sync::Arc<tokio::sync::Semaphore>,
}

/// Resolved pool reference: either the default pool or a named pool.
struct ResolvedPool<'a> {
    pool: &'a parking_lot::Mutex<WorkerPool>,
    semaphore: Option<&'a std::sync::Arc<tokio::sync::Semaphore>>,
    /// Index into named_pools (None = default pool).
    pool_index: Option<usize>,
}

/// Find the right worker pool for a request path.
/// Checks named pools first (in order), falls back to default.
fn find_pool<'a>(state: &'a ServerState, path: &str) -> Option<ResolvedPool<'a>> {
    // Check named pools first (route-based splitting)
    for (i, np) in state.named_pools.iter().enumerate() {
        if features::matches_pool_route(&np.route.match_path, path) {
            return Some(ResolvedPool {
                pool: &np.pool,
                semaphore: Some(&np.semaphore),
                pool_index: Some(i),
            });
        }
    }
    // Fall back to default pool
    state.worker_pool.as_ref().map(|pm| ResolvedPool {
        pool: pm,
        semaphore: state.worker_semaphore.as_ref(),
        pool_index: None,
    })
}

/// Return a worker to the correct pool (default or named).
fn return_worker_to_pool(state: &ServerState, pool_index: Option<usize>, worker_idx: usize) {
    if let Some(idx) = pool_index {
        if let Some(np) = state.named_pools.get(idx) {
            let mut pool = np.pool.lock();
            if state.persistent_workers {
                pool.return_worker_persistent(
                    worker_idx,
                    &state.persistent_app_root,
                    state.worker_boot.as_deref(),
                    state.worker_handler.as_deref(),
                    state.worker_cleanup.as_deref(),
                );
            } else {
                pool.return_worker(worker_idx);
            }
        }
    } else if let Some(ref pm) = state.worker_pool {
        let mut pool = pm.lock();
        if state.persistent_workers {
            pool.return_worker_persistent(
                worker_idx,
                &state.persistent_app_root,
                state.worker_boot.as_deref(),
                state.worker_handler.as_deref(),
                state.worker_cleanup.as_deref(),
            );
        } else {
            pool.return_worker(worker_idx);
        }
    }
}

/// A request to execute PHP code on the dedicated thread.
struct PhpRequest {
    code: String,
    uploaded_files: Vec<String>,
    response_tx: tokio::sync::oneshot::Sender<PhpResult>,
}

type PhpResult = Result<turbine_engine::PhpResponse, String>;

#[allow(clippy::too_many_arguments)]
fn cmd_serve(
    listen_override: Option<String>,
    workers_override: Option<usize>,
    config_path: Option<String>,
    root_override: Option<String>,
    tls_cert_override: Option<String>,
    tls_key_override: Option<String>,
    request_timeout_override: Option<u64>,
    access_log_override: Option<String>,
) {
    // Advise the kernel to use 2 MiB transparent huge pages for this
    // process's anonymous memory.  Wins are largest for OPcache shared
    // memory and Zend arenas (hot bytecode + object heap fits in ~1-4 MiB
    // per worker, so 4 KiB pages trigger lots of TLB misses).  No-op on
    // macOS and on Linux kernels configured with `transparent_hugepage =
    // never`.  Matches what HHVM does in `HugePagesInit`.
    hugepage_hint_process();

    // Resolve config path to absolute BEFORE chdir so it remains valid after the directory change
    let resolved_config_path: Option<String> = config_path.map(|p| {
        let pb = std::path::PathBuf::from(&p);
        if pb.is_absolute() {
            p
        } else {
            std::env::current_dir()
                .unwrap_or_default()
                .join(&pb)
                .to_string_lossy()
                .into_owned()
        }
    });

    // Change to app root directory if --root is specified
    if let Some(ref root) = root_override {
        let root_path = std::path::Path::new(root);
        if !root_path.exists() {
            eprintln!("Application root does not exist: {root}");
            std::process::exit(1);
        }
        std::env::set_current_dir(root_path).unwrap_or_else(|e| {
            eprintln!("Cannot change to directory {root}: {e}");
            std::process::exit(1);
        });
    }

    let mut config = match resolved_config_path {
        Some(ref path) => RuntimeConfig::load_from(path),
        None => RuntimeConfig::load(),
    };

    // --- Embedded app extraction ---
    // If the binary has an embedded PHP app and no --root was specified,
    // extract it and use as the application root.
    if root_override.is_none() && embed::has_embedded_app() {
        if let Some(extract_dir) = embed::extract_embedded_app(&config.embed) {
            std::env::set_current_dir(&extract_dir).unwrap_or_else(|e| {
                eprintln!(
                    "Cannot change to embedded app directory {}: {e}",
                    extract_dir.display()
                );
                std::process::exit(1);
            });
            // Reload config from extracted directory (if turbine.toml is embedded)
            config = RuntimeConfig::load();
        }
    }

    // CLI overrides
    if let Some(listen) = listen_override {
        config.server.listen = listen;
    }
    if let Some(workers) = workers_override {
        config.server.workers = workers;
    }
    // TLS CLI overrides
    if let Some(cert) = tls_cert_override {
        config.server.tls.cert_file = Some(cert);
        config.server.tls.enabled = true;
    }
    if let Some(key) = tls_key_override {
        config.server.tls.key_file = Some(key);
        config.server.tls.enabled = true;
    }
    if let Some(timeout) = request_timeout_override {
        config.server.request_timeout = timeout;
    }
    if let Some(log_path) = access_log_override {
        config.logging.access_log = Some(log_path);
    }

    config.validate();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.logging.level)),
        )
        .init();

    info!("Turbine Runtime v{}", env!("CARGO_PKG_VERSION"));
    info!(
        workers = config.server.workers,
        listen = %config.server.listen,
        log_level = %config.logging.level,
        "Configuration loaded"
    );

    // --- PID file ---
    let pid_file_path = config.server.pid_file.clone();
    if let Some(ref path) = pid_file_path {
        match std::fs::write(path, format!("{}", std::process::id())) {
            Ok(_) => info!(path = %path, pid = std::process::id(), "PID file written"),
            Err(e) => warn!(path = %path, error = %e, "Failed to write PID file"),
        }
    }

    let app_root = std::env::current_dir().expect("Cannot determine current directory");
    info!(root = %app_root.display(), "Application root");

    // --- Session directory ---
    if config.session.enabled {
        if let Err(e) = std::fs::create_dir_all(&config.session.save_path) {
            warn!(path = %config.session.save_path, error = %e, "Failed to create session directory");
        }
    }

    // --- Upload temp directory ---
    if let Err(e) = std::fs::create_dir_all(&config.php.upload_tmp_dir) {
        warn!(path = %config.php.upload_tmp_dir, error = %e, "Failed to create upload temp directory");
    }

    // --- Initialize PHP embed SAPI ---
    let startup = Instant::now();

    let session_secure = config.session.cookie_secure || config.server.tls.enabled;

    // --- Camada 5: Build open_basedir from project paths (Fortress) ---
    // Always active when enforce_open_basedir=true — restricts ALL PHP filesystem
    // access (fopen, file_get_contents, include, etc.) to these directories only.
    // /dev/fd/ is included so persistent workers can use pipe file descriptors.
    let open_basedir = if config.sandbox.enforce_open_basedir {
        let sys_tmp = std::env::temp_dir().display().to_string();
        let mut paths = vec![
            app_root.display().to_string(),
            config.php.upload_tmp_dir.clone(),
            config.session.save_path.clone(),
            "/tmp/turbine-opcache".to_string(),
            "/dev/fd".to_string(),
            sys_tmp,
        ];
        // Add data directories as absolute paths
        for data_dir in &config.sandbox.data_directories {
            let abs = app_root.join(data_dir);
            paths.push(abs.display().to_string());
        }
        // Add virtual host roots so PHP can access them
        for vhost in &config.virtual_hosts {
            let vhost_root = std::path::Path::new(&vhost.root);
            if vhost_root.is_absolute() {
                paths.push(vhost.root.clone());
            } else {
                paths.push(app_root.join(&vhost.root).display().to_string());
            }
        }
        // Deduplicate paths
        paths.sort();
        paths.dedup();
        let basedir = paths.join(":");
        info!(open_basedir = %basedir, "PHP open_basedir restriction active");
        basedir
    } else {
        String::new()
    };

    let disabled_functions = config.sandbox.disabled_functions.join(",");

    // Resolve OPcache preload script — auto-detect vendor/preload.php
    let preload_script = match config.php.preload_script.as_deref() {
        Some("auto") | None => {
            // Auto-detect common preload scripts
            let candidates = [
                app_root.join("vendor/preload.php"),
                app_root.join("preload.php"),
                app_root.join("config/preload.php"),
            ];
            candidates
                .iter()
                .find(|p| p.exists())
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        }
        Some(path) => path.to_string(),
    };

    let php_ini = PhpIniOverrides {
        memory_limit: config.php.memory_limit.clone(),
        max_execution_time: config.php.max_execution_time,
        upload_max_filesize: config.php.upload_max_filesize.clone(),
        post_max_size: config.php.post_max_size.clone(),
        opcache_memory: config.php.opcache_memory,
        jit_buffer_size: config.php.jit_buffer_size.clone(),
        session_save_path: config.session.save_path.clone(),
        session_cookie_name: config.session.cookie_name.clone(),
        session_cookie_lifetime: config.session.cookie_lifetime,
        session_cookie_httponly: config.session.cookie_httponly,
        session_cookie_secure: session_secure,
        session_cookie_samesite: config.session.cookie_samesite.clone(),
        session_gc_maxlifetime: config.session.gc_maxlifetime,
        open_basedir,
        disabled_functions,
        block_url_include: config.sandbox.block_url_include,
        block_url_fopen: config.sandbox.block_url_fopen,
        preload_script,
        extra_ini: config.php.ini.clone(),
        extensions: config.php.extensions.clone(),
        zend_extensions: config.php.zend_extensions.clone(),
        opcache_validate_timestamps: config.php.opcache_validate_timestamps.unwrap_or(false),
    };
    let mut engine = match PhpEngine::init_with(php_ini) {
        Ok(engine) => engine,
        Err(e) => {
            error!("Failed to initialize PHP engine: {e}");
            std::process::exit(1);
        }
    };
    info!("PHP {} loaded via embed SAPI", engine.php_version());

    if let Some(ref ext_dir) = config.php.extension_dir {
        info!(extension_dir = %ext_dir, "PHP extension directory configured");
    }

    // --- Detect application structure ---
    let mut app_structure = AppDetector::detect(&app_root);
    // Allow config override for front_controller
    if let Some(fc) = config.sandbox.front_controller {
        app_structure.front_controller = fc;
    }
    info!(
        document_root = %app_structure.document_root.display(),
        entry = %app_structure.entry_point,
        front_controller = app_structure.front_controller,
        "Application structure detected"
    );
    let mut php_bootstrap = app_structure.php_bootstrap_code();
    // Inject turbine_log() PHP function if structured logging is enabled
    if config.structured_logging.enabled {
        php_bootstrap = format!("{}{}", features::php_turbine_log_function(), php_bootstrap);
        info!("PHP turbine_log() function injected into bootstrap");
    }
    // Inject turbine_table_* PHP helpers if shared_table is enabled
    if config.shared_table.enabled {
        php_bootstrap = format!(
            "{}{}",
            features::php_turbine_table_functions(),
            php_bootstrap
        );
        info!("PHP turbine_table_*() helpers injected into bootstrap");
    }

    // --- Virtual hosting ---
    let mut virtual_hosts: std::collections::HashMap<String, Arc<VhostResolved>> =
        std::collections::HashMap::new();
    for vhost_cfg in &config.virtual_hosts {
        let vhost_root = std::path::Path::new(&vhost_cfg.root);
        let vhost_root = if vhost_root.is_absolute() {
            vhost_root.to_path_buf()
        } else {
            app_root.join(vhost_root)
        };
        if !vhost_root.exists() {
            warn!(domain = %vhost_cfg.domain, root = %vhost_cfg.root, "Virtual host root directory not found — skipping");
            continue;
        }
        let mut vhost_app = AppDetector::detect(&vhost_root);
        // Override entry_point if specified in config
        if let Some(ref ep) = vhost_cfg.entry_point {
            vhost_app.entry_point = ep.clone();
        }
        info!(
            domain = %vhost_cfg.domain,
            document_root = %vhost_app.document_root.display(),
            entry = %vhost_app.entry_point,
            "Virtual host configured"
        );
        let resolved = Arc::new(VhostResolved {
            domain: vhost_cfg.domain.clone(),
            app_structure: vhost_app,
        });
        // Map domain (lowercase)
        virtual_hosts.insert(vhost_cfg.domain.to_lowercase(), resolved.clone());
        // Map aliases
        for alias in &vhost_cfg.aliases {
            virtual_hosts.insert(alias.to_lowercase(), resolved.clone());
        }
    }
    if !virtual_hosts.is_empty() {
        info!(
            count = config.virtual_hosts.len(),
            domains = virtual_hosts.len(),
            "Virtual hosts loaded"
        );
    }

    // --- RequestGuard ---
    let request_guard = RequestGuard::new(&app_structure.document_root);
    if config.security.path_traversal_guard {
        info!("Path traversal guard active (via RequestGuard)");
    }

    // --- Sandbox: Execution Whitelist (Camada 1: Fortress) ---
    // When execution_mode = "strict", use the explicit whitelist from config.
    // When execution_mode = "framework", use the explicit whitelist if configured,
    // otherwise leave empty (empty whitelist = allow all PHP files).
    let execution_whitelist = if config.sandbox.execution_mode == "strict" {
        let wl = if config.sandbox.execution_whitelist.is_empty() {
            // Strict mode with no explicit whitelist: only allow entry point
            vec![app_structure.entry_point.clone()]
        } else {
            config.sandbox.execution_whitelist.clone()
        };
        info!(
            mode = "strict",
            whitelist = ?wl,
            "Execution whitelist active"
        );
        wl
    } else if !config.sandbox.execution_whitelist.is_empty() {
        // Framework mode with explicit whitelist configured
        info!(
            mode = "framework",
            whitelist = ?config.sandbox.execution_whitelist,
            "Execution whitelist active (user-configured)"
        );
        config.sandbox.execution_whitelist.clone()
    } else {
        // Framework mode, no explicit whitelist: allow all PHP files
        info!(
            mode = "framework",
            "No execution whitelist — all PHP files allowed"
        );
        Vec::new()
    };

    // --- Sandbox: Data Directories (Camada 2: Fortress) ---
    let data_directories = config.sandbox.data_directories.clone();
    info!(
        data_dirs = ?data_directories,
        "Data directories configured (no PHP execution allowed)"
    );

    let startup_elapsed = startup.elapsed();
    info!(
        elapsed_ms = startup_elapsed.as_millis(),
        "Master initialization complete"
    );

    // --- Security layer ---
    let sec_config = SecConfig {
        enabled: config.security.enabled,
        sql_guard: config.security.sql_guard,
        code_injection_guard: config.security.code_injection_guard,
        behaviour_guard: config.security.behaviour_guard,
        paranoia_level: config.security.paranoia_level,
        exclude_paths: config.security.exclude_paths.clone(),
    };
    let behaviour_config = BehaviourConfig {
        max_rps: config.security.max_requests_per_second,
        window_seconds: config.security.rate_limit_window,
        sqli_block_threshold: config.security.sqli_block_threshold,
        ..BehaviourConfig::default()
    };
    let security = SecurityLayer::with_behaviour_config(sec_config, behaviour_config);
    info!(
        enabled = config.security.enabled,
        sql = config.security.sql_guard,
        code_inj = config.security.code_injection_guard,
        behaviour = config.security.behaviour_guard,
        max_rps = config.security.max_requests_per_second,
        "Security layer initialized"
    );

    // --- Metrics ---
    let metrics = MetricsCollector::new();
    info!("Metrics collector initialized");

    // --- Response cache ---
    let cache_config = CacheConfig {
        ttl: std::time::Duration::from_secs(config.cache.ttl_seconds),
        max_entries: config.cache.max_entries,
        enabled: config.cache.enabled,
    };
    let cache = ResponseCache::new(cache_config);
    info!(
        enabled = config.cache.enabled,
        ttl_s = config.cache.ttl_seconds,
        max = config.cache.max_entries,
        "Response cache initialized"
    );

    // --- Database bridge ---
    // PHP handles database connections natively via PDO.

    let listen = config.server.listen.clone();
    let worker_count = config.server.workers;

    // --- Upload Security (Camada 4: Fortress) ---
    let upload_security = compat::UploadSecurityConfig {
        blocked_extensions: config.sandbox.blocked_upload_extensions.clone(),
        scan_content: config.sandbox.scan_upload_content,
    };
    info!(
        blocked_extensions = ?config.sandbox.blocked_upload_extensions,
        scan_content = config.sandbox.scan_upload_content,
        "Upload hardening active"
    );

    // --- Request timeout ---
    let request_timeout = std::time::Duration::from_secs(config.server.request_timeout);
    if config.server.request_timeout > 0 {
        info!(
            timeout_s = config.server.request_timeout,
            "Request timeout configured"
        );
    } else {
        info!("Request timeout disabled");
    }

    // --- Access log ---
    let access_log: Option<std::sync::Mutex<std::io::BufWriter<std::fs::File>>> = if let Some(
        ref path,
    ) =
        config.logging.access_log
    {
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(file) => {
                info!(path = %path, "Access log enabled");
                Some(std::sync::Mutex::new(std::io::BufWriter::new(file)))
            }
            Err(e) => {
                warn!(path = %path, error = %e, "Failed to open access log file, continuing without");
                None
            }
        }
    } else {
        None
    };

    // --- Custom error pages ---
    let error_page_404 =
        config
            .error_pages
            .not_found
            .as_ref()
            .and_then(|path| match std::fs::read(path) {
                Ok(content) => {
                    info!(path = %path, "Custom 404 error page loaded");
                    Some(content)
                }
                Err(e) => {
                    warn!(path = %path, error = %e, "Failed to load custom 404 page");
                    None
                }
            });
    let error_page_500 =
        config
            .error_pages
            .server_error
            .as_ref()
            .and_then(|path| match std::fs::read(path) {
                Ok(content) => {
                    info!(path = %path, "Custom 500 error page loaded");
                    Some(content)
                }
                Err(e) => {
                    warn!(path = %path, error = %e, "Failed to load custom 500 page");
                    None
                }
            });

    // --- X-Sendfile root ---
    let x_sendfile_root = if config.x_sendfile.enabled {
        let root = config.x_sendfile.root.as_deref().unwrap_or(".");
        let abs = app_root.join(root);
        info!(root = %abs.display(), "X-Sendfile / X-Accel-Redirect enabled");
        Some(abs)
    } else {
        None
    };

    // --- Feature logging ---
    if config.early_hints.enabled {
        info!("Early Hints (103) support enabled");
    }
    if config.structured_logging.enabled {
        info!(output = %config.structured_logging.output, "Structured logging from PHP enabled");
    }
    if !config.worker_pools.is_empty() {
        for pool_route in &config.worker_pools {
            info!(
                path = %pool_route.match_path,
                min = pool_route.min_workers,
                max = pool_route.max_workers,
                name = ?pool_route.name,
                "Worker pool route configured"
            );
        }
    }
    if config.server.max_wait_time > 0 {
        info!(
            max_wait_time_s = config.server.max_wait_time,
            "Worker queue timeout configured"
        );
    }

    // --- ACME auto-TLS ---
    // Auto-collect virtual host domains into ACME if not already present
    if config.acme.enabled && !config.virtual_hosts.is_empty() {
        for vhost_cfg in &config.virtual_hosts {
            // Skip vhosts with their own per-host TLS certificates
            if vhost_cfg.tls_cert.is_some() {
                continue;
            }
            let domain = vhost_cfg.domain.to_lowercase();
            if !config
                .acme
                .domains
                .iter()
                .any(|d| d.to_lowercase() == domain)
            {
                info!(domain = %vhost_cfg.domain, "Adding virtual host domain to ACME");
                config.acme.domains.push(vhost_cfg.domain.clone());
            }
            for alias in &vhost_cfg.aliases {
                let alias_lower = alias.to_lowercase();
                if !config
                    .acme
                    .domains
                    .iter()
                    .any(|d| d.to_lowercase() == alias_lower)
                {
                    info!(domain = %alias, "Adding virtual host alias to ACME");
                    config.acme.domains.push(alias.clone());
                }
            }
        }
    }
    let acme_challenge_tokens = acme::new_challenge_store();
    if config.acme.enabled {
        if config.acme.domains.is_empty() {
            warn!("ACME enabled but no domains configured — skipping");
        } else {
            match acme::load_cached_certificate(&config.acme) {
                Some(cert) => {
                    // Use cached certificate
                    config.server.tls.enabled = true;
                    config.server.tls.cert_file = Some(cert.cert_path.display().to_string());
                    config.server.tls.key_file = Some(cert.key_path.display().to_string());
                    info!("Using cached ACME certificate");

                    // Check if renewal is needed and spawn renewal task
                    if acme::needs_renewal(&config.acme) {
                        info!("ACME certificate approaching expiry — renewal will be attempted in background");
                    }
                }
                None => {
                    // Need to provision a new certificate
                    info!("No valid cached certificate — starting ACME provisioning");

                    // Build a temporary tokio runtime for provisioning
                    // (we can't use the main server's runtime because it hasn't started yet)
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to build ACME runtime");

                    // Start temporary HTTP server for ACME challenge on port 80
                    let challenge_tokens_clone = acme_challenge_tokens.clone();
                    let acme_config_clone = config.acme.clone();
                    match rt.block_on(async {
                        // Spawn HTTP-01 challenge server on port 80
                        let challenge_listener =
                            match tokio::net::TcpListener::bind("0.0.0.0:80").await {
                                Ok(l) => l,
                                Err(e) => {
                                    return Err(format!(
                                        "Cannot bind port 80 for ACME challenge: {e}. \
                                    Ensure port 80 is available or use manual TLS."
                                    ));
                                }
                            };
                        let tokens_for_server = challenge_tokens_clone.clone();
                        let challenge_server = tokio::spawn(async move {
                            loop {
                                let (stream, _) = match challenge_listener.accept().await {
                                    Ok(pair) => pair,
                                    Err(_) => continue,
                                };
                                let tokens = tokens_for_server.clone();
                                tokio::spawn(async move {
                                    let io = hyper_util::rt::TokioIo::new(stream);
                                    let svc = hyper::service::service_fn(
                                        move |req: hyper::Request<hyper::body::Incoming>| {
                                            let tokens = tokens.clone();
                                            async move {
                                                let path = req.uri().path().to_string();
                                                if let Some(response) =
                                                    acme::handle_challenge_request(&path, &tokens)
                                                {
                                                    Ok::<_, hyper::Error>(
                                                        hyper::Response::builder()
                                                            .status(200)
                                                            .header("Content-Type", "text/plain")
                                                            .body(http_body_util::Full::new(
                                                                bytes::Bytes::from(response),
                                                            ))
                                                            .unwrap(),
                                                    )
                                                } else {
                                                    Ok(hyper::Response::builder()
                                                        .status(404)
                                                        .body(http_body_util::Full::new(
                                                            bytes::Bytes::from("Not Found"),
                                                        ))
                                                        .unwrap())
                                                }
                                            }
                                        },
                                    );
                                    let _ = hyper::server::conn::http1::Builder::new()
                                        .serve_connection(io, svc)
                                        .await;
                                });
                            }
                        });

                        let result = acme::provision_certificate(
                            &acme_config_clone,
                            &challenge_tokens_clone,
                        )
                        .await;
                        challenge_server.abort();
                        result
                    }) {
                        Ok(cert) => {
                            config.server.tls.enabled = true;
                            config.server.tls.cert_file =
                                Some(cert.cert_path.display().to_string());
                            config.server.tls.key_file = Some(cert.key_path.display().to_string());
                            info!("ACME certificate provisioned and TLS enabled");
                        }
                        Err(e) => {
                            error!(error = %e, "ACME certificate provisioning failed");
                            if !config.server.tls.enabled {
                                warn!("Continuing without TLS — ACME provisioning failed");
                            }
                        }
                    }
                }
            }
        }
    }

    // --- TLS setup ---
    let tls_acceptor = if config.server.tls.enabled {
        let cert_path = config.server.tls.cert_file.as_deref().unwrap_or_else(|| {
            error!("TLS enabled but cert_file not set");
            std::process::exit(1);
        });
        let key_path = config.server.tls.key_file.as_deref().unwrap_or_else(|| {
            error!("TLS enabled but key_file not set");
            std::process::exit(1);
        });
        // Collect per-vhost TLS certs for SNI
        let vhost_certs: Vec<(String, String, String)> = config
            .virtual_hosts
            .iter()
            .filter_map(|v| match (&v.tls_cert, &v.tls_key) {
                (Some(cert), Some(key)) => {
                    let mut domains = vec![(v.domain.to_lowercase(), cert.clone(), key.clone())];
                    for alias in &v.aliases {
                        domains.push((alias.to_lowercase(), cert.clone(), key.clone()));
                    }
                    Some(domains)
                }
                _ => None,
            })
            .flatten()
            .collect();
        if vhost_certs.is_empty() {
            Some(build_tls_acceptor(cert_path, key_path))
        } else {
            Some(build_tls_acceptor_with_sni(
                cert_path,
                key_path,
                &vhost_certs,
            ))
        }
    } else {
        None
    };

    if worker_count >= 1 {
        // --- Multi-worker mode: fork processes or spawn threads ---
        // worker_count >= 1 uses the worker pool (process or thread mode).
        // worker_count == 0 falls through to php_tx (single-process tokio).
        let worker_mode = WorkerMode::from_str(&config.server.worker_mode);
        info!(mode = %worker_mode, workers = worker_count, "Worker mode selected");

        let pool_config = PoolConfig {
            workers: worker_count,
            max_requests: config.server.worker_max_requests,
            mode: worker_mode,
            pin_workers: config.server.pin_workers,
        };
        let mut pool = WorkerPool::new(pool_config);
        let is_thread_mode = worker_mode == WorkerMode::Thread;
        let mut thread_dispatch_prebuilt: Option<ThreadDispatch> = None;

        // Choose whether to use persistent workers (bootstrap-once) or the
        // classic per-request fork+eval model.
        // Persistent workers: controlled by config.server.persistent_workers (default false).
        let use_persistent = config.server.persistent_workers.unwrap_or(false);

        if use_persistent {
            info!(mode = "persistent", worker_mode = %worker_mode, "Persistent workers enabled");
            let app_root_str = app_root.display().to_string();
            let w_boot = config.server.worker_boot.as_deref();
            let w_handler = config.server.worker_handler.as_deref();
            let w_cleanup = config.server.worker_cleanup.as_deref();

            if is_thread_mode {
                // Thread mode: spawn persistent workers as OS threads (ZTS required)
                match pool.spawn_persistent_workers_threaded(
                    &app_root_str,
                    w_boot,
                    w_handler,
                    w_cleanup,
                ) {
                    Ok(()) => {
                        info!(
                            workers = pool.worker_count(),
                            mode = "thread",
                            "Persistent worker thread pool ready"
                        );
                    }
                    Err(e) => {
                        error!("Failed to spawn persistent worker threads: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                // Process mode: spawn persistent workers via fork
                match pool.spawn_persistent_workers(&app_root_str, w_boot, w_handler, w_cleanup) {
                    Ok(true) => {
                        info!(
                            workers = pool.worker_count(),
                            mode = "process",
                            "Persistent worker pool ready"
                        );
                    }
                    Ok(false) => {
                        std::process::exit(0);
                    }
                    Err(e) => {
                        error!("Failed to spawn persistent workers: {e}");
                        std::process::exit(1);
                    }
                }
            }

            // Read ready signal (binary: 0xAA + u32:0) from each worker.
            for idx in 0..pool.worker_count() {
                if let Some(worker) = pool.worker_mut(idx) {
                    match read_ready_signal(worker.resp_fd()) {
                        Ok(true) => debug!(idx = idx, "Persistent worker ready"),
                        Ok(false) => warn!(idx = idx, "Persistent worker bootstrap failed"),
                        Err(e) => {
                            error!(idx = idx, error = %e, "Failed to read persistent ready signal")
                        }
                    }
                }
            }
        } else {
            if is_thread_mode {
                // Thread mode: spawn channel-based worker threads (zero-pipe IPC).
                // ThreadDispatch owns the channels; we register workers in the pool
                // for lifecycle tracking only (dummy fds).
                let (td, worker_info) =
                    ThreadDispatch::spawn_channel_workers(worker_count, config.server.pin_workers);
                // Register workers in pool for alive_count / shutdown tracking.
                for (alive, tid) in &worker_info {
                    pool.register_channel_thread(alive.clone(), *tid);
                }
                // Store the pre-built dispatch (will be moved into Arc later).
                // We stash it in an Option so the later ThreadDispatch build block
                // can distinguish channel mode from pipe mode.
                thread_dispatch_prebuilt = Some(td);
                info!(
                    workers = pool.worker_count(),
                    mode = "thread-channel",
                    "Worker thread pool ready (channel IPC)"
                );
            } else {
                // Process mode: spawn native SAPI workers via fork
                match pool.spawn_workers(worker_event_loop_native) {
                    Ok(true) => {
                        info!(
                            workers = pool.worker_count(),
                            mode = "process",
                            "Worker pool ready"
                        );
                    }
                    Ok(false) => {
                        std::process::exit(0);
                    }
                    Err(e) => {
                        error!("Failed to spawn workers: {e}");
                        std::process::exit(1);
                    }
                }
            }

            // Read ready signals from pipe-based workers (process mode only).
            if !is_thread_mode {
                for idx in 0..pool.worker_count() {
                    if let Some(worker) = pool.worker_mut(idx) {
                        match read_native_response_from_fd(worker.resp_fd()) {
                            Ok(resp) if resp.success => debug!(idx = idx, "Native worker ready"),
                            Ok(resp) => {
                                warn!(
                                    idx = idx,
                                    "Native worker not ready: status={}", resp.http_status
                                );
                            }
                            Err(e) => {
                                error!(idx = idx, error = %e, "Failed to read native worker ready signal");
                            }
                        }
                    }
                }
            }
        }

        // --- Named worker pools (route-based splitting) ---
        let mut named_pools = Vec::new();
        for route_cfg in &config.worker_pools {
            let pool_name = route_cfg.name.as_deref().unwrap_or(&route_cfg.match_path);
            let np_config = PoolConfig {
                workers: route_cfg.min_workers,
                max_requests: config.server.worker_max_requests,
                mode: worker_mode,
                pin_workers: config.server.pin_workers,
            };
            let mut np_pool = WorkerPool::new(np_config);

            let np_spawn_ok = if is_thread_mode {
                match np_pool.spawn_workers_threaded(worker_event_loop_native) {
                    Ok(()) => true,
                    Err(e) => {
                        warn!(name = pool_name, error = %e, "Failed to spawn named thread pool — requests will use default pool");
                        false
                    }
                }
            } else {
                match np_pool.spawn_workers(worker_event_loop_native) {
                    Ok(true) => true,
                    Ok(false) => std::process::exit(0), // child
                    Err(e) => {
                        warn!(name = pool_name, error = %e, "Failed to spawn named pool — requests will use default pool");
                        false
                    }
                }
            };

            if np_spawn_ok {
                info!(name = pool_name, workers = np_pool.worker_count(), route = %route_cfg.match_path, "Named worker pool ready");
                // Read ready signals from named pool workers
                for idx in 0..np_pool.worker_count() {
                    if let Some(worker) = np_pool.worker_mut(idx) {
                        let _ = read_native_response_from_fd(worker.resp_fd());
                    }
                }
                named_pools.push(NamedWorkerPool {
                    route: route_cfg.clone(),
                    pool: parking_lot::Mutex::new(np_pool),
                    semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(
                        route_cfg.min_workers,
                    )),
                });
            }
        }

        // Build lock-free ThreadDispatch for ALL pipe-based worker modes.
        //
        // Originally only the thread-mode workers used `ThreadDispatch` to
        // escape the per-request `pool_mutex.lock()`.  Process-mode workers
        // went through the pool mutex in the hot path which serialised
        // dispatch across all cores.  We now build a pipe-based dispatch
        // for process mode too.
        //
        // For channel mode (non-persistent + thread), the dispatch was
        // already built by `spawn_channel_workers`.
        //
        // Recycle-by-max_requests IS now enforced in the dispatch hot path:
        // `get_idle` checks `ThreadDispatch.requests_served[idx]` vs the
        // configured `worker_max_requests` and skips over workers that
        // have reached quota, letting the reaper respawn them without
        // racing with new traffic.  Dead pipes (EPIPE on send or EOF on
        // decode) are similarly tagged unhealthy in the dispatch handler
        // so the next `get_idle` won't pick the same worker again until
        // it is respawned.  See `ThreadDispatch::is_pickable`.
        let thread_dispatch: Option<Arc<ThreadDispatch>> =
            if let Some(mut td) = thread_dispatch_prebuilt.take() {
                td.set_max_requests(config.server.worker_max_requests);
                Some(Arc::new(td))
            } else {
                // Both thread and process pipe-based modes use pipe fds.
                let fds = pool.worker_fds();
                let mut td = ThreadDispatch::new(fds);
                td.set_max_requests(config.server.worker_max_requests);
                Some(Arc::new(td))
            };

        let state = Arc::new(ServerState {
            listen: listen.clone(),
            worker_count,
            is_tls: tls_acceptor.is_some(),
            request_timeout,
            access_log,
            compression_enabled: config.compression.enabled,
            compression_min_size: config.compression.min_size,
            compression_level: config.compression.level,
            compression_algorithms: config.compression.algorithms.clone(),
            error_page_404: std::sync::RwLock::new(error_page_404),
            error_page_500: std::sync::RwLock::new(error_page_500),
            cors: config.cors.clone(),
            pid_file: config.server.pid_file.clone(),
            upload_tmp_dir: config.php.upload_tmp_dir.clone(),
            max_body_bytes: parse_php_size(&config.php.post_max_size),
            execution_mode: config.sandbox.execution_mode.clone(),
            execution_whitelist,
            data_directories: data_directories.clone(),
            upload_security,
            request_guard,
            security,
            metrics,
            cache,
            cache_coalescer: Arc::new(turbine_cache::Coalescer::new()),
            persistent_app_root: app_root.display().to_string(),
            worker_boot: config.server.worker_boot.clone(),
            worker_handler: config.server.worker_handler.clone(),
            worker_cleanup: config.server.worker_cleanup.clone(),
            session_auto_start: config.session.enabled && config.session.auto_start,
            app_structure,
            php_bootstrap,
            php_tx: None,
            worker_pool: Some(parking_lot::Mutex::new(pool)),
            worker_mode,
            thread_dispatch: thread_dispatch.clone(),
            persistent_workers: use_persistent,
            worker_semaphore: Some(std::sync::Arc::new(tokio::sync::Semaphore::new(
                worker_count,
            ))),
            auto_scale: config.server.auto_scale,
            min_workers: config.server.min_workers,
            max_workers: config.server.max_workers,
            scale_down_idle_secs: config.server.scale_down_idle_secs,
            watcher_config: config.watcher.clone(),
            early_hints_enabled: config.early_hints.enabled,
            x_sendfile_enabled: config.x_sendfile.enabled,
            x_sendfile_root: x_sendfile_root.clone(),
            structured_logging_enabled: config.structured_logging.enabled,
            max_wait_time: config.server.max_wait_time,
            worker_pool_routes: config.worker_pools.clone(),
            named_pools,
            acme_challenge_tokens: acme_challenge_tokens.clone(),
            dashboard_enabled: config.dashboard.enabled,
            statistics_enabled: config.dashboard.statistics,
            dashboard_token: config.dashboard.token.clone(),
            shared_table: if config.shared_table.enabled {
                Some(Arc::new(SharedTable::new(config.shared_table.max_entries)))
            } else {
                None
            },
            virtual_hosts: virtual_hosts.clone(),
        });

        // Spawn ACME renewal task if ACME is enabled
        let mut rt_builder = tokio::runtime::Builder::new_multi_thread();
        rt_builder.enable_all();
        if let Some(n) = config.server.tokio_worker_threads {
            rt_builder.worker_threads(n);
        }
        let rt = rt_builder.build().expect("Failed to build tokio runtime");
        if config.acme.enabled && !config.acme.domains.is_empty() {
            let acme_config = config.acme.clone();
            let tokens = acme_challenge_tokens.clone();
            rt.spawn(async move {
                acme::spawn_renewal_task(acme_config, tokens);
            });
        }

        // Background shared-table sweeper — drops expired entries on a cadence
        // so TTL'd data doesn't rely solely on lazy read-path eviction.
        if let Some(table) = state.shared_table.clone() {
            let sweep = std::time::Duration::from_secs(config.shared_table.sweep_interval_secs);
            rt.spawn(async move {
                let mut interval = tokio::time::interval(sweep);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                // First tick fires immediately; skip it so we don't sweep an empty map at boot.
                interval.tick().await;
                loop {
                    interval.tick().await;
                    let removed = table.sweep_expired();
                    if removed > 0 {
                        debug!(removed, "shared-table sweeper evicted expired entries");
                    }
                }
            });
        }

        // Spawn a background reaper task that keeps the worker pool healthy
        // and the lock-free `ThreadDispatch` in sync with pipe fds.  This is
        // required now that ALL modes go through the dispatch (the mutex-
        // path reap-on-timeout is no longer on the hot path).
        if let (Some(td), true) = (state.thread_dispatch.clone(), state.worker_pool.is_some()) {
            let reaper_state = state.clone();
            rt.spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    // Cold path: lock the pool briefly to reap+respawn
                    // dead workers, then sync dispatch fds.
                    let pm = match reaper_state.worker_pool.as_ref() {
                        Some(pm) => pm,
                        None => break,
                    };
                    let new_fds = {
                        let mut pool = pm.lock();
                        if reaper_state.persistent_workers {
                            if reaper_state.worker_mode == WorkerMode::Thread {
                                let _ = pool.reap_and_respawn_persistent_threaded(
                                    &reaper_state.persistent_app_root,
                                    reaper_state.worker_boot.as_deref(),
                                    reaper_state.worker_handler.as_deref(),
                                    reaper_state.worker_cleanup.as_deref(),
                                );
                            } else {
                                let _ = pool.reap_and_respawn_persistent(
                                    &reaper_state.persistent_app_root,
                                    reaper_state.worker_boot.as_deref(),
                                    reaper_state.worker_handler.as_deref(),
                                    reaper_state.worker_cleanup.as_deref(),
                                );
                            }
                        } else if reaper_state.worker_mode == WorkerMode::Thread {
                            let _ = pool.reap_and_respawn_threaded(
                                turbine_worker::pool::worker_event_loop_native,
                            );
                        } else {
                            let _ = pool
                                .reap_and_respawn(turbine_worker::pool::worker_event_loop_native);
                        }
                        pool.worker_fds()
                    };
                    td.refresh_fds(new_fds);
                }
            });
        }

        let busy_poll_us = config.server.listen_busy_poll_us.unwrap_or(0);
        let reuseport_shards = config.server.listen_reuseport_shards.unwrap_or(0);
        rt.block_on(run_hyper_server(
            state,
            &listen,
            tls_acceptor,
            busy_poll_us,
            reuseport_shards,
        ));
    } else {
        // --- Single-process mode: PHP on a dedicated thread, hyper for HTTP ---
        info!("Running in single-process mode");

        let (php_tx, php_rx) =
            tokio::sync::mpsc::channel::<PhpRequest>(config.server.channel_capacity);

        let state = Arc::new(ServerState {
            listen: listen.clone(),
            worker_count: 1,
            is_tls: tls_acceptor.is_some(),
            request_timeout,
            access_log,
            compression_enabled: config.compression.enabled,
            compression_min_size: config.compression.min_size,
            compression_level: config.compression.level,
            compression_algorithms: config.compression.algorithms.clone(),
            error_page_404: std::sync::RwLock::new(error_page_404),
            error_page_500: std::sync::RwLock::new(error_page_500),
            cors: config.cors.clone(),
            pid_file: config.server.pid_file.clone(),
            upload_tmp_dir: config.php.upload_tmp_dir.clone(),
            max_body_bytes: parse_php_size(&config.php.post_max_size),
            execution_mode: config.sandbox.execution_mode.clone(),
            execution_whitelist,
            data_directories,
            upload_security,
            request_guard,
            security,
            metrics,
            cache,
            cache_coalescer: Arc::new(turbine_cache::Coalescer::new()),
            app_structure,
            php_bootstrap,
            php_tx: Some(php_tx),
            worker_pool: None,
            worker_mode: WorkerMode::Process,
            thread_dispatch: None,
            persistent_workers: false,
            persistent_app_root: String::new(),
            worker_boot: None,
            worker_handler: None,
            worker_cleanup: None,
            session_auto_start: config.session.enabled && config.session.auto_start,
            worker_semaphore: None,
            auto_scale: false,
            min_workers: 1,
            max_workers: 1,
            scale_down_idle_secs: 5,
            watcher_config: config.watcher.clone(),
            early_hints_enabled: config.early_hints.enabled,
            x_sendfile_enabled: config.x_sendfile.enabled,
            x_sendfile_root: x_sendfile_root.clone(),
            structured_logging_enabled: config.structured_logging.enabled,
            max_wait_time: config.server.max_wait_time,
            worker_pool_routes: config.worker_pools.clone(),
            named_pools: Vec::new(),
            acme_challenge_tokens: acme_challenge_tokens.clone(),
            dashboard_enabled: config.dashboard.enabled,
            statistics_enabled: config.dashboard.statistics,
            dashboard_token: config.dashboard.token.clone(),
            shared_table: if config.shared_table.enabled {
                Some(Arc::new(SharedTable::new(config.shared_table.max_entries)))
            } else {
                None
            },
            virtual_hosts: virtual_hosts.clone(),
        });

        let mut rt_builder = tokio::runtime::Builder::new_multi_thread();
        rt_builder.enable_all();
        if let Some(n) = config.server.tokio_worker_threads {
            rt_builder.worker_threads(n);
        }
        let rt = rt_builder.build().expect("Failed to build tokio runtime");

        let busy_poll_us = config.server.listen_busy_poll_us.unwrap_or(0);
        let reuseport_shards = config.server.listen_reuseport_shards.unwrap_or(0);
        rt.block_on(async {
            tokio::task::spawn_blocking(move || {
                php_executor_loop(&mut engine, php_rx);
            });

            run_hyper_server(state, &listen, tls_acceptor, busy_poll_us, reuseport_shards).await;
        });
    }
}

fn php_executor_loop(engine: &mut PhpEngine, mut rx: tokio::sync::mpsc::Receiver<PhpRequest>) {
    while let Some(req) = rx.blocking_recv() {
        for path in &req.uploaded_files {
            unsafe { turbine_engine::register_uploaded_file(path) };
        }
        // Split code into setup (superglobals etc) and include
        // The caller sends the full code with include at the end
        let result = if let Some((setup, include_part)) = req.code.rsplit_once("include '") {
            let include_code = format!("include '{include_part}");
            match engine.eval(setup) {
                Ok(()) => match engine.eval_capture_full(&include_code) {
                    Ok(resp) => {
                        let _ = engine.reset_request();
                        Ok(resp)
                    }
                    Err(e) => {
                        let _ = engine.reset_request();
                        Err(format!("{e}"))
                    }
                },
                Err(e) => {
                    let _ = engine.reset_request();
                    Err(format!("{e}"))
                }
            }
        } else {
            // No include — eval the full code
            match engine.eval_capture_full(&req.code) {
                Ok(resp) => {
                    let _ = engine.reset_request();
                    Ok(resp)
                }
                Err(e) => {
                    let _ = engine.reset_request();
                    Err(format!("{e}"))
                }
            }
        };

        let _ = req.response_tx.send(result);
    }
}

fn build_tls_acceptor(cert_path: &str, key_path: &str) -> TlsAcceptor {
    build_tls_acceptor_with_sni(cert_path, key_path, &[])
}

/// Build a TLS acceptor with optional SNI-based per-host certificates.
/// The default cert/key is used for connections that don't match any SNI name.
fn build_tls_acceptor_with_sni(
    cert_path: &str,
    key_path: &str,
    vhost_certs: &[(String, String, String)], // (domain, cert_path, key_path)
) -> TlsAcceptor {
    use rustls::ServerConfig as RustlsConfig;
    use std::io::BufReader;

    // Helper: load cert+key pair
    fn load_cert_key(
        cert_path: &str,
        key_path: &str,
    ) -> (
        Vec<rustls::pki_types::CertificateDer<'static>>,
        rustls::pki_types::PrivateKeyDer<'static>,
    ) {
        let cert_file = std::fs::File::open(cert_path).unwrap_or_else(|e| {
            error!(path = cert_path, "Failed to open certificate file: {e}");
            std::process::exit(1);
        });
        let key_file = std::fs::File::open(key_path).unwrap_or_else(|e| {
            error!(path = key_path, "Failed to open key file: {e}");
            std::process::exit(1);
        });

        let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
            .filter_map(|r| r.ok())
            .collect();
        if certs.is_empty() {
            error!(path = cert_path, "No certificates found in file");
            std::process::exit(1);
        }

        let key = rustls_pemfile::private_key(&mut BufReader::new(key_file))
            .unwrap_or_else(|e| {
                error!(path = key_path, "Failed to parse private key: {e}");
                std::process::exit(1);
            })
            .unwrap_or_else(|| {
                error!(path = key_path, "No private key found in file");
                std::process::exit(1);
            });

        (certs, key)
    }

    let tls_config = if vhost_certs.is_empty() {
        // Simple path: single certificate
        let (certs, key) = load_cert_key(cert_path, key_path);
        let mut cfg = RustlsConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap_or_else(|e| {
                error!("Invalid TLS certificate/key pair: {e}");
                std::process::exit(1);
            });
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        info!("TLS configured (single cert, ALPN: h2, http/1.1)");
        cfg
    } else {
        // SNI path: multiple certificates per domain
        use rustls::server::ResolvesServerCertUsingSni;
        use rustls::sign::CertifiedKey;

        let mut resolver = ResolvesServerCertUsingSni::new();

        for (domain, vcert_path, vkey_path) in vhost_certs {
            let (certs, key) = load_cert_key(vcert_path, vkey_path);
            let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key)
                .unwrap_or_else(|e| {
                    error!(domain = %domain, "Failed to load signing key: {e}");
                    std::process::exit(1);
                });
            let certified = CertifiedKey::new(certs, signing_key);
            resolver.add(domain, certified).unwrap_or_else(|e| {
                error!(domain = %domain, "Failed to add SNI cert: {e}");
                std::process::exit(1);
            });
            info!(domain = %domain, "SNI certificate loaded");
        }

        // Also add the default cert to the SNI resolver as fallback
        let (default_certs, default_key) = load_cert_key(cert_path, key_path);
        let default_signing = rustls::crypto::aws_lc_rs::sign::any_supported_type(&default_key)
            .unwrap_or_else(|e| {
                error!("Failed to load default signing key: {e}");
                std::process::exit(1);
            });
        let default_certified = Arc::new(CertifiedKey::new(default_certs, default_signing));

        // Build config with SNI resolver + default fallback
        use rustls::server::ResolvesServerCert;
        use std::fmt;
        struct SniWithFallback {
            sni: ResolvesServerCertUsingSni,
            fallback: Arc<CertifiedKey>,
        }
        impl fmt::Debug for SniWithFallback {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct("SniWithFallback").finish()
            }
        }
        impl ResolvesServerCert for SniWithFallback {
            fn resolve(
                &self,
                client_hello: rustls::server::ClientHello<'_>,
            ) -> Option<Arc<CertifiedKey>> {
                // Try SNI lookup first — if the domain has a cert, use it.
                // ResolvesServerCertUsingSni already checks server_name() internally.
                if client_hello.server_name().is_some() {
                    // Clone the server name to avoid borrow conflict
                    let sni_result = self.sni.resolve(client_hello);
                    if sni_result.is_some() {
                        return sni_result;
                    }
                    // SNI name didn't match — fall through to default
                }
                Some(self.fallback.clone())
            }
        }

        let sni_fallback = Arc::new(SniWithFallback {
            sni: resolver,
            fallback: default_certified,
        });

        let mut cfg = RustlsConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(sni_fallback);
        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        info!(
            vhosts = vhost_certs.len(),
            "TLS configured with SNI (ALPN: h2, http/1.1)"
        );
        cfg
    };

    TlsAcceptor::from(Arc::new(tls_config))
}

/// Enable `SO_BUSY_POLL` on a socket fd.
///
/// Linux only. The kernel will spin on the NIC RX queue for up to
/// `us` microseconds before yielding to the scheduler, shaving 20-50µs
/// off p99 latency at the cost of CPU usage.
///
/// Silently no-op on non-Linux and when the setsockopt fails (e.g.
/// insufficient privileges on kernels < 5.7). Returns `true` if the
/// option was applied.
#[cfg(target_os = "linux")]
fn set_busy_poll(fd: std::os::unix::io::RawFd, us: u32) -> bool {
    // SO_BUSY_POLL = 46 on Linux (kernel 3.11+).
    const SO_BUSY_POLL: libc::c_int = 46;
    let val: libc::c_int = us as libc::c_int;
    // SAFETY: fd is a valid socket fd owned by the caller; we only
    // borrow it for the duration of the setsockopt call.
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            SO_BUSY_POLL,
            &val as *const _ as *const libc::c_void,
            std::mem::size_of_val(&val) as libc::socklen_t,
        )
    };
    rc == 0
}

#[cfg(not(target_os = "linux"))]
#[inline]
#[allow(dead_code)]
fn set_busy_poll(_fd: i32, _us: u32) -> bool {
    false
}

/// Bind a single TCP listener with `SO_REUSEPORT` set, so multiple
/// listeners can share the same (addr, port). Linux distributes
/// incoming connections across all such listeners with a flow hash,
/// which is the basis of the accept-per-core pattern.
///
/// Also sets `SO_REUSEADDR`, `SOCK_NONBLOCK`, `SOCK_CLOEXEC`, and a
/// generous listen backlog (1024).
///
/// Returns a `tokio::net::TcpListener` ready for async accept.
#[cfg(target_os = "linux")]
fn bind_reuseport_linux(addr: std::net::SocketAddr) -> std::io::Result<TcpListener> {
    use std::os::unix::io::FromRawFd;

    let domain = if addr.is_ipv6() {
        libc::AF_INET6
    } else {
        libc::AF_INET
    };
    // SAFETY: libc::socket returns -1 on error; we check below. All
    // other raw-fd ops are standard BSD sockets API usage.
    unsafe {
        let fd = libc::socket(
            domain,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        );
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Wrap fd so it gets closed on any early return.
        let owned: std::os::unix::io::OwnedFd = std::os::unix::io::FromRawFd::from_raw_fd(fd);

        let one: libc::c_int = 1;
        let set = |opt: libc::c_int| -> std::io::Result<()> {
            let rc = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                opt,
                &one as *const _ as *const libc::c_void,
                std::mem::size_of_val(&one) as libc::socklen_t,
            );
            if rc != 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        set(libc::SO_REUSEADDR)?;
        set(libc::SO_REUSEPORT)?;

        // Bind
        match addr {
            std::net::SocketAddr::V4(v4) => {
                let sin = libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: v4.port().to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(v4.ip().octets()),
                    },
                    sin_zero: [0; 8],
                };
                let rc = libc::bind(
                    fd,
                    &sin as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                );
                if rc != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            std::net::SocketAddr::V6(v6) => {
                let mut sin6: libc::sockaddr_in6 = std::mem::zeroed();
                sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sin6.sin6_port = v6.port().to_be();
                sin6.sin6_addr.s6_addr = v6.ip().octets();
                sin6.sin6_flowinfo = v6.flowinfo();
                sin6.sin6_scope_id = v6.scope_id();
                let rc = libc::bind(
                    fd,
                    &sin6 as *const _ as *const libc::sockaddr,
                    std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                );
                if rc != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
        }

        let rc = libc::listen(fd, 1024);
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // Convert to std listener (taking ownership), then to tokio.
        let raw = std::os::unix::io::IntoRawFd::into_raw_fd(owned);
        let std_listener = std::net::TcpListener::from_raw_fd(raw);
        TcpListener::from_std(std_listener)
    }
}

async fn run_hyper_server(
    state: Arc<ServerState>,
    listen: &str,
    tls_acceptor: Option<TlsAcceptor>,
    busy_poll_us: u32,
    reuseport_shards: usize,
) {
    let addr: SocketAddr = listen.parse().unwrap_or_else(|_| {
        error!(listen = listen, "Invalid listen address");
        std::process::exit(1);
    });

    // Build one or more listeners. With `reuseport_shards > 1` on Linux,
    // bind N independent sockets with SO_REUSEPORT so the kernel can
    // load-balance accepts across N concurrent accept loops.
    let listeners: Vec<TcpListener> = {
        #[cfg(target_os = "linux")]
        {
            if reuseport_shards > 1 {
                let mut v = Vec::with_capacity(reuseport_shards);
                for i in 0..reuseport_shards {
                    match bind_reuseport_linux(addr) {
                        Ok(l) => v.push(l),
                        Err(e) => {
                            if i == 0 {
                                error!(listen = listen, "Failed to bind (SO_REUSEPORT): {e}");
                                std::process::exit(1);
                            }
                            warn!(
                                shard = i,
                                error = %e,
                                "Failed to bind additional reuseport shard; continuing with fewer"
                            );
                            break;
                        }
                    }
                }
                info!(shards = v.len(), "SO_REUSEPORT accept sharding enabled");
                v
            } else {
                match TcpListener::bind(addr).await {
                    Ok(l) => vec![l],
                    Err(e) => {
                        error!(listen = listen, "Failed to bind: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            if reuseport_shards > 1 {
                debug!(
                    shards = reuseport_shards,
                    "listen_reuseport_shards set but ignored (non-Linux platform)"
                );
            }
            match TcpListener::bind(addr).await {
                Ok(l) => vec![l],
                Err(e) => {
                    error!(listen = listen, "Failed to bind: {e}");
                    std::process::exit(1);
                }
            }
        }
    };

    // Optional: SO_BUSY_POLL on every listening socket (Linux only).
    // Shaves 20-50µs off p99 at the cost of CPU.  No-op on other OSes.
    if busy_poll_us > 0 {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::io::AsRawFd;
            let mut ok = 0usize;
            for l in &listeners {
                if set_busy_poll(l.as_raw_fd(), busy_poll_us) {
                    ok += 1;
                }
            }
            if ok == listeners.len() {
                info!(us = busy_poll_us, "SO_BUSY_POLL enabled on listener(s)");
            } else {
                warn!(
                    us = busy_poll_us,
                    ok = ok,
                    total = listeners.len(),
                    "SO_BUSY_POLL setsockopt failed on some listeners"
                );
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            debug!("listen_busy_poll_us set but ignored (non-Linux platform)");
        }
    }

    let scheme = if tls_acceptor.is_some() {
        "https"
    } else {
        "http"
    };
    let proto = if tls_acceptor.is_some() {
        "HTTP/1.1 + HTTP/2 (TLS)"
    } else {
        "HTTP/1.1 (keep-alive)"
    };
    info!(listen = listen, protocol = proto, "Server listening");
    info!("Try: curl {scheme}://{listen}/");
    info!("Metrics: {scheme}://{listen}/_/metrics  Status: {scheme}://{listen}/_/status");
    info!("Dashboard: {scheme}://{listen}/_/dashboard");

    // SIGHUP handler — hot-reload error pages from turbine.toml
    #[cfg(unix)]
    {
        let reload_state = state.clone();
        tokio::spawn(async move {
            let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
                .expect("Failed to install SIGHUP handler");
            loop {
                sighup.recv().await;
                info!("Received SIGHUP, reloading configuration...");
                let new_config = RuntimeConfig::load();

                // Reload error pages
                let new_404 = new_config
                    .error_pages
                    .not_found
                    .as_ref()
                    .and_then(|path| std::fs::read(path).ok());
                let new_500 = new_config
                    .error_pages
                    .server_error
                    .as_ref()
                    .and_then(|path| std::fs::read(path).ok());
                *reload_state.error_page_404.write().unwrap() = new_404;
                *reload_state.error_page_500.write().unwrap() = new_500;
                info!("Configuration reloaded (error pages updated)");
            }
        });
    }

    // --- Auto-scaling task ---
    if state.auto_scale && state.worker_pool.is_some() {
        let scale_state = state.clone();
        tokio::spawn(async move {
            let check_interval = std::time::Duration::from_secs(1);
            let idle_threshold = std::time::Duration::from_secs(scale_state.scale_down_idle_secs);
            let mut idle_since: Option<tokio::time::Instant> = None;

            loop {
                tokio::time::sleep(check_interval).await;

                if let Some(ref pool_mutex) = scale_state.worker_pool {
                    let pool = pool_mutex.lock();
                    let alive = pool.alive_count();
                    let idle = pool.idle_count();
                    let busy = pool.busy_count();
                    drop(pool);

                    // Scale UP: all workers busy AND below max
                    if idle == 0 && alive < scale_state.max_workers {
                        let mut pool = pool_mutex.lock();
                        if scale_state.persistent_workers {
                            // Persistent workers require spawn_persistent_workers — skip auto-scale for now
                            debug!("Auto-scale up skipped: persistent workers not supported");
                        } else if scale_state.worker_mode == WorkerMode::Thread {
                            match pool.spawn_additional_thread(worker_event_loop_native) {
                                Ok(()) => {
                                    if let Some(ref sem) = scale_state.worker_semaphore {
                                        sem.add_permits(1);
                                    }
                                    info!(
                                        alive = pool.alive_count(),
                                        busy = busy,
                                        mode = "thread",
                                        "Auto-scaled UP"
                                    );
                                    idle_since = None;
                                }
                                Err(e) => warn!(error = %e, "Auto-scale thread spawn failed"),
                            }
                        } else {
                            match pool.spawn_additional(worker_event_loop_native) {
                                Ok(true) => {
                                    // Add a permit to the semaphore
                                    if let Some(ref sem) = scale_state.worker_semaphore {
                                        sem.add_permits(1);
                                    }
                                    info!(
                                        alive = pool.alive_count(),
                                        busy = busy,
                                        "Auto-scaled UP"
                                    );
                                    idle_since = None;
                                }
                                Ok(false) => std::process::exit(0), // child
                                Err(e) => warn!(error = %e, "Auto-scale spawn failed"),
                            }
                        }
                    }
                    // Scale DOWN: excess idle workers beyond min_workers
                    else if idle > 0 && alive > scale_state.min_workers {
                        match idle_since {
                            Some(since) if since.elapsed() >= idle_threshold => {
                                let mut pool = pool_mutex.lock();
                                if pool.shrink_one() {
                                    info!(alive = pool.alive_count(), "Auto-scaled DOWN");
                                    idle_since = None;
                                }
                            }
                            None => {
                                idle_since = Some(tokio::time::Instant::now());
                            }
                            _ => {} // waiting for idle timeout
                        }
                    } else {
                        idle_since = None;
                    }
                }
            }
        });
    }

    // --- File watcher task ---
    if state.watcher_config.enabled && state.worker_pool.is_some() {
        let watch_state = state.clone();
        let watcher_cfg = state.watcher_config.clone();
        tokio::task::spawn_blocking(move || {
            use notify::{Config, RecursiveMode, Watcher};
            use std::sync::mpsc;

            let (tx, rx) = mpsc::channel();
            let mut watcher = match notify::RecommendedWatcher::new(tx, Config::default()) {
                Ok(w) => w,
                Err(e) => {
                    error!(error = %e, "Failed to create file watcher");
                    return;
                }
            };

            let app_root = std::env::current_dir().unwrap_or_default();
            for path in &watcher_cfg.paths {
                let watch_path = app_root.join(path);
                if watch_path.exists() {
                    if let Err(e) = watcher.watch(&watch_path, RecursiveMode::Recursive) {
                        warn!(path = %watch_path.display(), error = %e, "Failed to watch directory");
                    } else {
                        info!(path = %watch_path.display(), "Watching for changes");
                    }
                }
            }

            let debounce = std::time::Duration::from_millis(watcher_cfg.debounce_ms);
            let extensions: Vec<String> = watcher_cfg.extensions.clone();
            let mut last_reload = std::time::Instant::now();

            loop {
                match rx.recv() {
                    Ok(Ok(event)) => {
                        // Only react to file modifications/creations with matching extensions
                        let dominated = event.paths.iter().any(|p| {
                            p.extension()
                                .and_then(|e| e.to_str())
                                .map(|ext| extensions.iter().any(|w| w == ext))
                                .unwrap_or(false)
                        });
                        if dominated && last_reload.elapsed() >= debounce {
                            let changed: Vec<_> = event
                                .paths
                                .iter()
                                .filter_map(|p| p.file_name())
                                .filter_map(|n| n.to_str())
                                .collect();
                            info!(files = ?changed, "File change detected — restarting workers");
                            last_reload = std::time::Instant::now();

                            if let Some(ref pool_mutex) = watch_state.worker_pool {
                                let mut pool = pool_mutex.lock();
                                // Graceful restart: terminate all, then respawn
                                pool.shutdown();
                                if watch_state.persistent_workers {
                                    // For persistent workers, a full respawn is needed
                                    // This is complex — for now, log a warning
                                    warn!(
                                        "Hot reload for persistent workers requires server restart"
                                    );
                                } else if watch_state.worker_mode == WorkerMode::Thread {
                                    if let Err(e) =
                                        pool.spawn_workers_threaded(worker_event_loop_native)
                                    {
                                        error!(error = %e, "Failed to respawn worker threads after file change");
                                    } else {
                                        info!(
                                            workers = pool.worker_count(),
                                            mode = "thread",
                                            "Worker threads restarted after file change"
                                        );
                                    }
                                } else if let Err(e) = pool.spawn_workers(worker_event_loop_native)
                                {
                                    error!(error = %e, "Failed to respawn workers after file change");
                                } else {
                                    info!(
                                        workers = pool.worker_count(),
                                        "Workers restarted after file change"
                                    );
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        warn!(error = %e, "File watcher error");
                    }
                    Err(_) => {
                        debug!("File watcher channel closed");
                        break;
                    }
                }
            }
        });
    }

    // Graceful shutdown signal
    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("Failed to install SIGTERM handler");
            tokio::select! {
                _ = ctrl_c => info!("Received SIGINT, shutting down gracefully..."),
                _ = sigterm.recv() => info!("Received SIGTERM, shutting down gracefully..."),
            }
        }
        #[cfg(not(unix))]
        {
            ctrl_c.await.ok();
            info!("Received shutdown signal, shutting down gracefully...");
        }
    };
    tokio::pin!(shutdown);

    // Track active connections for graceful drain
    let active_connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Fan out one accept loop per listener shard. All shards share the
    // same `active_connections` counter and `state`. Shutdown is
    // broadcast via a `watch` channel.
    let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
    let mut shard_handles = Vec::with_capacity(listeners.len());

    for (shard_id, listener) in listeners.into_iter().enumerate() {
        let state_s = state.clone();
        let tls_s = tls_acceptor.clone();
        let conns_s = active_connections.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (stream, remote_addr) = match result {
                            Ok(pair) => pair,
                            Err(e) => {
                                warn!(shard = shard_id, "Accept error: {e}");
                                continue;
                            }
                        };

                        // Disable Nagle's algorithm — HTTP/1.1 and HTTP/2 both do their
                        // own batching, and Nagle adds up to ~40ms latency on p99 for
                        // small responses (< MSS).  Matches the defaults used by nginx,
                        // Caddy and hyper's own HTTP/2 stack.
                        let _ = stream.set_nodelay(true);

                        // Propagate SO_BUSY_POLL to accepted connections on Linux.
                        // Accepted sockets inherit most options from the listener, but
                        // SO_BUSY_POLL is per-socket, so set it explicitly.
                        #[cfg(target_os = "linux")]
                        if busy_poll_us > 0 {
                            use std::os::unix::io::AsRawFd;
                            let _ = set_busy_poll(stream.as_raw_fd(), busy_poll_us);
                        }

                        let state = state_s.clone();
                        let tls_acceptor = tls_s.clone();
                        let conns = conns_s.clone();
                        conns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                        tokio::task::spawn(async move {
                            let service = service_fn(move |req: Request<Incoming>| {
                                let state = state.clone();
                                async move { handle_request(req, remote_addr, state).await }
                            });

                            if let Some(acceptor) = tls_acceptor {
                                let tls_stream = match acceptor.accept(stream).await {
                                    Ok(s) => s,
                                    Err(e) => {
                                        debug!("TLS handshake error: {e}");
                                        conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                                        return;
                                    }
                                };
                                let io = TokioIo::new(tls_stream);
                                let result = AutoBuilder::new(hyper_util::rt::TokioExecutor::new())
                                    .serve_connection(io, service)
                                    .await;
                                if let Err(e) = result {
                                    let msg = e.to_string();
                                    if !msg.contains("connection closed") && !msg.contains("not connected") {
                                        debug!("TLS connection error: {msg}");
                                    }
                                }
                            } else {
                                let io = TokioIo::new(stream);
                                let conn = http1::Builder::new()
                                    .keep_alive(true)
                                    .serve_connection(io, service)
                                    .with_upgrades();
                                if let Err(e) = conn.await {
                                    if !e.to_string().contains("connection closed") {
                                        debug!("Connection error: {e}");
                                    }
                                }
                            }

                            conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                        });
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });
        shard_handles.push(handle);
    }

    // Wait for OS shutdown signal, then notify all shards.
    (&mut shutdown).await;
    let _ = shutdown_tx.send(true);
    for h in shard_handles {
        let _ = h.await;
    }

    // Phase 1: Wait for in-flight PHP requests (worker draining)
    if let Some(ref pool_mutex) = state.worker_pool {
        let drain_start = Instant::now();
        let worker_drain_timeout = std::time::Duration::from_secs(15);
        loop {
            let busy = {
                let pool = pool_mutex.lock();
                pool.busy_count()
            };
            if busy == 0 {
                info!("All worker requests drained");
                break;
            }
            if drain_start.elapsed() > worker_drain_timeout {
                warn!(busy = busy, "Worker drain timeout reached");
                break;
            }
            info!(
                busy = busy,
                "Waiting for in-flight PHP requests to complete..."
            );
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        // Send shutdown to all workers
        let mut pool = pool_mutex.lock();
        pool.shutdown();
        info!("Worker pool shut down");
    }

    // Shutdown named worker pools
    for np in &state.named_pools {
        let mut pool = np.pool.lock();
        pool.shutdown();
        let name = np.route.name.as_deref().unwrap_or(&np.route.match_path);
        info!(name = name, "Named worker pool shut down");
    }

    // Phase 2: Drain active HTTP connections
    let drain_start = Instant::now();
    let drain_timeout = std::time::Duration::from_secs(10);
    loop {
        let active = active_connections.load(std::sync::atomic::Ordering::Relaxed);
        if active == 0 {
            info!("All connections drained");
            break;
        }
        if drain_start.elapsed() > drain_timeout {
            warn!(active = active, "Drain timeout reached, forcing shutdown");
            break;
        }
        info!(active = active, "Waiting for connections to drain...");
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    // Clean up PID file
    if let Some(ref path) = state.pid_file {
        if let Err(e) = std::fs::remove_file(path) {
            debug!(path = %path, error = %e, "Failed to remove PID file");
        }
    }

    info!("Server stopped");
}

type HyperResponse = Response<Full<Bytes>>;

async fn handle_request(
    req: Request<Incoming>,
    remote_addr: SocketAddr,
    state: Arc<ServerState>,
) -> Result<HyperResponse, hyper::Error> {
    let is_tls = state.is_tls;
    let accept_encoding = req
        .headers()
        .get("accept-encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let min_size = state.compression_min_size;
    let level = state.compression_level;

    let origin = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let is_preflight = req.method() == hyper::Method::OPTIONS && origin.is_some();
    let cors_enabled = state.cors.enabled;
    let cors = &state.cors;

    if cors_enabled && is_preflight {
        if let Some(ref origin_val) = origin {
            if cors_origin_allowed(cors, origin_val) {
                let mut resp = build_response(204, "text/plain", Vec::new(), &[]);
                apply_cors_headers(resp.headers_mut(), cors, origin_val);
                return Ok(resp);
            }
        }
    }

    let mut resp = handle_request_inner(req, remote_addr, state.clone()).await?;

    if is_tls {
        resp.headers_mut().insert(
            "Strict-Transport-Security",
            "max-age=63072000; includeSubDomains; preload"
                .parse()
                .unwrap(),
        );
    }

    if cors_enabled {
        if let Some(ref origin_val) = origin {
            if cors_origin_allowed(cors, origin_val) {
                apply_cors_headers(resp.headers_mut(), cors, origin_val);
            }
        }
    }

    if state.compression_enabled && !accept_encoding.is_empty() {
        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !resp.headers().contains_key("Content-Encoding") && is_compressible_content_type(&ct) {
            use http_body_util::BodyExt;
            let (mut parts, body) = resp.into_parts();
            let collected = body.collect().await.unwrap_or_default();
            let data = collected.to_bytes();
            if data.len() >= min_size {
                // Compression (brotli/zstd/gzip) is CPU-bound and on hot
                // payloads can take several ms.  Run on blocking pool so the
                // tokio reactor stays free for other connections.
                let algorithms = state.compression_algorithms.clone();
                let accept = accept_encoding.clone();
                let data_for_task = data.clone();
                let result = tokio::task::spawn_blocking(move || {
                    negotiate_compression(&accept, &algorithms, &data_for_task, level)
                })
                .await
                .unwrap_or(None);

                if let Some((encoding, compressed)) = result {
                    parts
                        .headers
                        .insert("Content-Encoding", encoding.parse().unwrap());
                    parts
                        .headers
                        .insert("Vary", "Accept-Encoding".parse().unwrap());
                    parts
                        .headers
                        .insert("Content-Length", compressed.len().into());
                    resp = Response::from_parts(parts, Full::new(Bytes::from(compressed)));
                } else {
                    resp = Response::from_parts(parts, Full::new(data));
                }
            } else {
                resp = Response::from_parts(parts, Full::new(data));
            }
        }
    }

    Ok(resp)
}

async fn handle_request_inner(
    req: Request<Incoming>,
    remote_addr: SocketAddr,
    state: Arc<ServerState>,
) -> Result<HyperResponse, hyper::Error> {
    let request_start = Instant::now();
    let req_method = req.method().clone();
    let uri_path = req.uri().path().to_string();
    let clean_path = uri_path.split('?').next().unwrap_or(&uri_path).to_string();

    // --- Internal endpoints ---
    if clean_path.starts_with("/_/") {
        // Dashboard is served without auth — the login screen handles it in-browser.
        if clean_path == "/_/dashboard" && state.dashboard_enabled {
            let body = dashboard::dashboard_html(&state.listen, state.dashboard_token.is_some());
            return Ok(build_response(
                200,
                "text/html; charset=utf-8",
                body.into_bytes(),
                &[],
            ));
        }

        // Token authentication for all other internal endpoints
        if let Some(ref expected_token) = state.dashboard_token {
            let authorized = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|t| t == expected_token.as_str())
                .unwrap_or(false);
            if !authorized {
                return Ok(build_response(
                    401,
                    "application/json",
                    b"{\"error\": \"Unauthorized\"}".to_vec(),
                    &[],
                ));
            }
        }

        if clean_path == "/_/metrics" && state.statistics_enabled {
            let body = state.metrics.prometheus();
            return Ok(build_response(
                200,
                "text/plain; version=0.0.4",
                body.into_bytes(),
                &[],
            ));
        }
        if clean_path == "/_/status" && state.statistics_enabled {
            let body = state.metrics.status_json(state.worker_count);
            return Ok(build_response(
                200,
                "application/json",
                body.into_bytes(),
                &[],
            ));
        }
        if clean_path == "/_/cache/clear" {
            let cleared = state.cache.len();
            state.cache.clear();
            let body = format!("{{\"cleared\": {cleared}}}");
            return Ok(build_response(
                200,
                "application/json",
                body.into_bytes(),
                &[],
            ));
        }

        // GET /_/security/blocked — list currently blocked IPs
        if clean_path == "/_/security/blocked" && req_method == "GET" {
            let blocked = state.security.blocked_ips();
            let entries: Vec<String> = blocked
                .iter()
                .map(|(ip, secs)| match secs {
                    Some(s) => format!("{{\"ip\":\"{ip}\",\"expires_in_secs\":{s}}}"),
                    None => format!("{{\"ip\":\"{ip}\",\"expires_in_secs\":null}}"),
                })
                .collect();
            let body = format!(
                "{{\"blocked\":[{}],\"count\":{}}}",
                entries.join(","),
                blocked.len()
            );
            return Ok(build_response(
                200,
                "application/json",
                body.into_bytes(),
                &[],
            ));
        }

        // POST /_/security/unblock  body: {"ip":"1.2.3.4"}
        if clean_path == "/_/security/unblock" && req_method == "POST" {
            let (inner_req, _) = match FullHttpRequest::from_hyper(
                req,
                remote_addr,
                &state.upload_tmp_dir,
                &state.upload_security,
                Some(8192), // admin endpoint: 8 KB is plenty
            )
            .await
            {
                Ok(pair) => pair,
                Err(compat::RequestBuildError::PayloadTooLarge) => {
                    return Ok(build_response(
                        413,
                        "application/json",
                        b"{\"error\":\"payload too large\"}".to_vec(),
                        &[],
                    ))
                }
                Err(_) => {
                    return Ok(build_response(
                        400,
                        "application/json",
                        b"{\"error\":\"invalid request\"}".to_vec(),
                        &[],
                    ))
                }
            };
            let body_str = String::from_utf8_lossy(&inner_req.body);
            // Parse {"ip":"x.x.x.x"} — minimal JSON extraction without a dep
            let ip_str = body_str
                .split('"')
                .skip_while(|s| *s != "ip")
                .nth(2)
                .unwrap_or("")
                .trim()
                .to_string();
            match ip_str.parse::<std::net::IpAddr>() {
                Ok(ip) => {
                    let found = state.security.unblock_ip(ip);
                    if found {
                        warn!(ip = %ip, "IP manually unblocked via admin API");
                        let body = format!("{{\"unblocked\":true,\"ip\":\"{ip}\"}}");
                        return Ok(build_response(
                            200,
                            "application/json",
                            body.into_bytes(),
                            &[],
                        ));
                    } else {
                        let body = format!("{{\"unblocked\":false,\"ip\":\"{ip}\",\"note\":\"IP was not blocked\"}}");
                        return Ok(build_response(
                            200,
                            "application/json",
                            body.into_bytes(),
                            &[],
                        ));
                    }
                }
                Err(_) => {
                    return Ok(build_response(
                        400,
                        "application/json",
                        b"{\"error\":\"invalid IP address\"}".to_vec(),
                        &[],
                    ));
                }
            }
        }

        // ── Shared table (Swoole\Table equivalent) ───────────────────────
        // These endpoints are active only when `[shared_table] enabled = true`.
        // `key` and `ttl_ms` come from the query string; the value (for SET)
        // is the raw request body so binary-safe payloads work transparently.
        if let Some(table) = state.shared_table.as_ref() {
            if clean_path.starts_with("/_/table") {
                // Parse query parameters manually (no url_form dep for internal API).
                let qs = req.uri().query().unwrap_or("");
                let key = query_param(qs, "key");
                let ttl_ms: u64 = query_param(qs, "ttl_ms")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let delta: i64 = query_param(qs, "delta")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);

                // Dispatch on method + subpath.
                let sub = clean_path.strip_prefix("/_/table").unwrap_or("");
                match (req_method.as_str(), sub) {
                    ("GET", "/size") => {
                        let body = format!(
                            "{{\"size\":{},\"evictions\":{}}}",
                            table.size(),
                            table.evictions()
                        );
                        return Ok(build_response(
                            200,
                            "application/json",
                            body.into_bytes(),
                            &[],
                        ));
                    }
                    ("GET", "/exists") => {
                        let k = match key {
                            Some(k) => k,
                            None => {
                                return Ok(build_response(
                                    400,
                                    "application/json",
                                    b"{\"error\":\"missing key\"}".to_vec(),
                                    &[],
                                ))
                            }
                        };
                        let code = if table.exists(k) { 200 } else { 404 };
                        return Ok(build_response(
                            code,
                            "application/json",
                            format!("{{\"exists\":{}}}", code == 200).into_bytes(),
                            &[],
                        ));
                    }
                    ("GET", "" | "/" | "/get") => {
                        let k = match key {
                            Some(k) => k,
                            None => {
                                return Ok(build_response(
                                    400,
                                    "application/json",
                                    b"{\"error\":\"missing key\"}".to_vec(),
                                    &[],
                                ))
                            }
                        };
                        return Ok(match table.get(k) {
                            Some(v) => build_response(200, "application/octet-stream", v, &[]),
                            None => build_response(404, "text/plain", Vec::new(), &[]),
                        });
                    }
                    ("POST", "/incr") => {
                        let k = match key {
                            Some(k) => k.to_string(),
                            None => {
                                return Ok(build_response(
                                    400,
                                    "application/json",
                                    b"{\"error\":\"missing key\"}".to_vec(),
                                    &[],
                                ))
                            }
                        };
                        return Ok(match table.incr(&k, delta) {
                            Ok(v) => build_response(
                                200,
                                "application/json",
                                format!("{{\"value\":{v}}}").into_bytes(),
                                &[],
                            ),
                            Err(TableError::Full(n)) => build_response(
                                507,
                                "application/json",
                                format!("{{\"error\":\"table full\",\"max_entries\":{n}}}")
                                    .into_bytes(),
                                &[],
                            ),
                            Err(TableError::NotACounter(_)) => build_response(
                                409,
                                "application/json",
                                b"{\"error\":\"key exists but is not a counter\"}".to_vec(),
                                &[],
                            ),
                        });
                    }
                    ("POST", "" | "/" | "/set") => {
                        let k = match key {
                            Some(k) => k.to_string(),
                            None => {
                                return Ok(build_response(
                                    400,
                                    "application/json",
                                    b"{\"error\":\"missing key\"}".to_vec(),
                                    &[],
                                ))
                            }
                        };
                        // Read the request body — capped at `max_body_bytes` so
                        // a rogue PHP worker cannot fill memory with one call.
                        let (inner_req, _) = match FullHttpRequest::from_hyper(
                            req,
                            remote_addr,
                            &state.upload_tmp_dir,
                            &state.upload_security,
                            Some(state.max_body_bytes.unwrap_or(1_048_576).max(1_048_576)),
                        )
                        .await
                        {
                            Ok(pair) => pair,
                            Err(compat::RequestBuildError::PayloadTooLarge) => {
                                return Ok(build_response(
                                    413,
                                    "application/json",
                                    b"{\"error\":\"payload too large\"}".to_vec(),
                                    &[],
                                ))
                            }
                            Err(_) => {
                                return Ok(build_response(
                                    400,
                                    "application/json",
                                    b"{\"error\":\"invalid request\"}".to_vec(),
                                    &[],
                                ))
                            }
                        };
                        let ttl = if ttl_ms > 0 {
                            Some(std::time::Duration::from_millis(ttl_ms))
                        } else {
                            None
                        };
                        return Ok(match table.set(k, inner_req.body, ttl) {
                            Ok(()) => build_response(204, "text/plain", Vec::new(), &[]),
                            Err(TableError::Full(n)) => build_response(
                                507,
                                "application/json",
                                format!("{{\"error\":\"table full\",\"max_entries\":{n}}}")
                                    .into_bytes(),
                                &[],
                            ),
                            Err(TableError::NotACounter(_)) => build_response(
                                500,
                                "application/json",
                                b"{\"error\":\"internal\"}".to_vec(),
                                &[],
                            ),
                        });
                    }
                    ("DELETE", "/clear") => {
                        let n = table.clear();
                        return Ok(build_response(
                            200,
                            "application/json",
                            format!("{{\"cleared\":{n}}}").into_bytes(),
                            &[],
                        ));
                    }
                    ("DELETE", "" | "/" | "/del") => {
                        let k = match key {
                            Some(k) => k,
                            None => {
                                return Ok(build_response(
                                    400,
                                    "application/json",
                                    b"{\"error\":\"missing key\"}".to_vec(),
                                    &[],
                                ))
                            }
                        };
                        let removed = table.del(k);
                        return Ok(build_response(
                            200,
                            "application/json",
                            format!("{{\"deleted\":{removed}}}").into_bytes(),
                            &[],
                        ));
                    }
                    _ => {
                        // Unknown subpath under /_/table — fall through to 404.
                    }
                }
            }
        }

        return Ok(build_response(
            404,
            "text/plain",
            b"Not found".to_vec(),
            &[],
        ));
    }

    // --- ACME HTTP-01 challenge ---
    if clean_path.starts_with("/.well-known/acme-challenge/") {
        if let Some(response) =
            acme::handle_challenge_request(&clean_path, &state.acme_challenge_tokens)
        {
            return Ok(build_response(
                200,
                "text/plain",
                response.into_bytes(),
                &[],
            ));
        }
        return Ok(build_response(
            404,
            "text/plain",
            b"ACME challenge token not found".to_vec(),
            &[],
        ));
    }

    // --- Virtual host resolution (O(1) HashMap lookup) ---
    let vhost = if !state.virtual_hosts.is_empty() {
        req.headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .map(|h| {
                // Strip port from Host header (e.g. "xpto.com:443" → "xpto.com")
                let domain = h.split(':').next().unwrap_or(h);
                domain.to_lowercase()
            })
            .and_then(|domain| state.virtual_hosts.get(&domain).cloned())
    } else {
        None
    };
    let app = vhost
        .as_ref()
        .map(|v| &v.app_structure)
        .unwrap_or(&state.app_structure);

    let if_none_match = req
        .headers()
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    if let Some(resp) = try_serve_static(
        &app.document_root,
        &uri_path,
        &req_method,
        &state.metrics,
        request_start,
        if_none_match.as_deref(),
    ) {
        return Ok(resp);
    }

    let (request, _remote) = match FullHttpRequest::from_hyper(
        req,
        remote_addr,
        &state.upload_tmp_dir,
        &state.upload_security,
        state.max_body_bytes,
    )
    .await
    {
        Ok(pair) => pair,
        Err(compat::RequestBuildError::PayloadTooLarge) => {
            state
                .metrics
                .record_request("", 413, request_start.elapsed().as_micros() as u64, 0);
            return Ok(build_response(
                413,
                "text/plain",
                b"Payload Too Large".to_vec(),
                &[],
            ));
        }
        Err(_) => {
            return Ok(build_response(
                400,
                "text/plain",
                b"Invalid HTTP request".to_vec(),
                &[],
            ));
        }
    };

    debug!(method = %request.method, path = %request.path, "Request received");

    let client_ip = remote_addr.ip();
    // Stringify the client IP once per request — otherwise `client_ip.to_string()`
    // is called ~13× (access log, metrics, worker envelope, security) causing
    // that many heap allocations on every hot path.
    let client_ip_str = client_ip.to_string();

    // Only build the parameter scan list if a guard will actually look at it.
    // Laravel/Symfony apps with security disabled (or only behaviour guard on)
    // were paying a full query+post params parse + raw-body copy + multiple
    // Vec allocations on every single request for no benefit.
    let needs_input_scan = state.security.needs_input_scan();
    let needs_behaviour = state.security.needs_behaviour_check();

    let input_verdict = if needs_input_scan {
        let query_params = request.get_params();
        let post_params = request.post_params();

        // Raw body scan: treat the first BODY_SCAN_LIMIT bytes as a single parameter so
        // that JSON-encoded payloads (e.g. {"q":"1 UNION SELECT *"}) are inspected even
        // when Content-Type is application/json (which post_params() skips).
        // We cap at 8 KB: injection payloads are always near the beginning; scanning
        // a multi-MB upload body would waste CPU without any security benefit.
        const BODY_SCAN_LIMIT: usize = 8192;
        let raw_body_str = if !request.body.is_empty() && post_params.is_empty() {
            let slice = &request.body[..request.body.len().min(BODY_SCAN_LIMIT)];
            // Use from_utf8_lossy — non-UTF-8 bytes become U+FFFD, but patterns won't match them.
            String::from_utf8_lossy(slice).into_owned()
        } else {
            String::new()
        };

        let mut all_params: Vec<(String, String)> = Vec::with_capacity(
            query_params.len() + post_params.len() + if raw_body_str.is_empty() { 0 } else { 1 },
        );
        all_params.extend(query_params);
        all_params.extend(post_params);
        if !raw_body_str.is_empty() {
            all_params.push(("_body".to_string(), raw_body_str));
        }

        let param_refs: Vec<(&str, &str)> = all_params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        state
            .security
            .check_input(client_ip, &request.path, &param_refs)
    } else if needs_behaviour {
        // Only behaviour guard enabled — cheap per-IP check, no param work.
        state.security.check_input(client_ip, &request.path, &[])
    } else {
        turbine_security::Verdict::Allow
    };

    let php_path = app.resolve_path(&request.path);

    // --- Camada 1: Execution Whitelist (Fortress) ---
    // Only files in the whitelist can be executed via HTTP.
    // Empty whitelist = allow all PHP files (framework mode default).
    if !state.execution_whitelist.is_empty() && !state.execution_whitelist.contains(&php_path) {
        warn!(
            ip = %client_ip, path = %php_path, mode = %state.execution_mode,
            "BLOCKED: PHP file not in execution whitelist"
        );
        state.metrics.record_security_block();
        state.metrics.record_request(
            &php_path,
            403,
            request_start.elapsed().as_micros() as u64,
            0,
        );
        return Ok(build_response(
            403,
            "text/plain",
            b"403 Forbidden: file not in execution whitelist".to_vec(),
            &[],
        ));
    }

    // --- Camada 2: Data Directory Guard (Fortress) ---
    // Block execution of any PHP file inside data directories.
    for data_dir in &state.data_directories {
        let normalized = data_dir.trim_end_matches('/');
        if php_path.starts_with(normalized) {
            warn!(
                ip = %client_ip, path = %php_path, data_dir = %data_dir,
                "BLOCKED: PHP execution attempt inside data directory"
            );
            state.metrics.record_security_block();
            state.metrics.record_request(
                &php_path,
                403,
                request_start.elapsed().as_micros() as u64,
                0,
            );
            return Ok(build_response(
                403,
                "text/plain",
                b"403 Forbidden: execution denied in data directory".to_vec(),
                &[],
            ));
        }
    }

    if input_verdict.is_blocked() {
        let reason = input_verdict.reason().unwrap_or("blocked");
        warn!(ip = %client_ip, reason = reason, "Request blocked by security layer");
        state.metrics.record_security_block();
        state.metrics.record_request(
            &php_path,
            403,
            request_start.elapsed().as_micros() as u64,
            0,
        );
        let body = format!("403 Forbidden: {reason}");
        return Ok(build_response(403, "text/plain", body.into_bytes(), &[]));
    }

    if !state.request_guard.exists(&php_path) {
        state.metrics.record_request(
            &php_path,
            404,
            request_start.elapsed().as_micros() as u64,
            0,
        );
        if let Some(ref page) = *state
            .error_page_404
            .read()
            .unwrap_or_else(|e| e.into_inner())
        {
            return Ok(build_response(
                404,
                "text/html; charset=utf-8",
                page.clone(),
                &[],
            ));
        }
        let body = format!("File not found: {php_path}");
        return Ok(build_response(404, "text/plain", body.into_bytes(), &[]));
    }

    // ── Fast path for persistent workers ──────────────────────────
    // The persistent worker already has the application bootstrapped — we only need
    // to send the HTTP request data via the binary protocol.
    //
    // Dispatch is ALWAYS lock-free through `ThreadDispatch` for both
    // thread and process worker modes — server startup guarantees
    // `state.thread_dispatch = Some(...)` whenever `state.worker_pool`
    // is initialised (see ThreadDispatch construction in `run_server`).
    // The previous `else` branch that went through `pool_mutex.lock()`
    // on the hot path was therefore unreachable; removing it shrinks
    // this function and makes it obvious that persistent dispatch is
    // entirely lock-free.
    if state.persistent_workers {
        if let Some(ref td) = state.thread_dispatch {
            let timeout_dur = if state.max_wait_time > 0 {
                std::time::Duration::from_secs(state.max_wait_time)
            } else if state.request_timeout.is_zero() {
                std::time::Duration::from_secs(60)
            } else {
                state.request_timeout
            };

            let worker_idx = match td.get_idle(timeout_dur).await {
                Some(idx) => idx,
                None => {
                    state.metrics.record_request(
                        &php_path,
                        504,
                        request_start.elapsed().as_micros() as u64,
                        0,
                    );
                    return Ok(build_response(
                        504,
                        "text/plain",
                        b"Request timeout waiting for worker".to_vec(),
                        &[],
                    ));
                }
            };

            let server_port = state
                .listen
                .split(':')
                .next_back()
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(8080);
            let full_uri_owned;
            let full_uri: &str = if request.query_string.is_empty() {
                &request.path
            } else {
                full_uri_owned = format!("{}?{}", request.path, request.query_string);
                &full_uri_owned
            };
            let headers_vec: Vec<(&str, &str)> = request
                .headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let content_type = request.content_type.as_deref().unwrap_or("");
            // O(1): request.headers is a HashMap with lowercase keys (see compat.rs).
            let cookie_header = request
                .headers
                .get("cookie")
                .map(String::as_str)
                .unwrap_or("");
            let document_root = &app.document_root_str;
            let script_filename = format!("{}/{}", &app.document_root_str, &php_path);
            let script_name = format!("/{}", &php_path);
            let per = PersistentRequest {
                method: &request.method,
                uri: full_uri,
                body: &request.body,
                client_ip: &client_ip_str,
                port: server_port,
                is_https: state.is_tls,
                headers: &headers_vec,
                script_filename: &script_filename,
                query_string: &request.query_string,
                document_root,
                content_type,
                cookie: cookie_header,
                path_info: &request.path,
                script_name: &script_name,
            };
            let guard = IdleGuard::new(td.clone(), worker_idx);
            let (cmd_fd, resp_fd) = td.fds(worker_idx);
            let write_result = turbine_worker::with_encode_scratch(|buf| {
                turbine_worker::encode_request_into(buf, &per);
                write_to_fd(cmd_fd, buf)
            });
            if let Err(e) = write_result {
                error!(worker = worker_idx, error = %e, "Failed to send to persistent worker (thread dispatch)");
                // Mark pipe as unhealthy so subsequent get_idle skips this
                // worker until the reaper respawns it.  guard returns the
                // idx on drop so the semaphore permit is restored, but
                // the flag ensures the next dispatch bypasses this slot.
                td.mark_unhealthy(worker_idx);
                return Ok(build_response(
                    502,
                    "text/plain",
                    b"Worker communication error".to_vec(),
                    &[],
                ));
            }

            // AsyncFd-based read: no spawn_blocking thread consumed.
            let bin_result: std::io::Result<_> =
                match turbine_worker::async_io::AsyncPipe::new(resp_fd) {
                    Ok(mut pipe) => turbine_worker::decode_response_async(&mut pipe).await,
                    Err(e) => Err(e),
                };
            // If decode failed, tag the worker unhealthy before releasing
            // the guard so the next get_idle() can't pick it again.
            if bin_result.is_err() {
                td.mark_unhealthy(worker_idx);
            } else {
                td.record_served(worker_idx);
            }
            drop(guard);

            match bin_result {
                Ok(resp) => {
                    let mut body = resp.body;
                    let mut status_code = resp.status;
                    let elapsed_us = request_start.elapsed().as_micros() as u64;
                    let php_content_type = resp
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
                        .map(|(_, v)| v.as_str());
                    let mut content_type = php_content_type
                        .unwrap_or_else(|| detect_content_type(&body))
                        .to_string();
                    let mut resp_headers = resp.headers;
                    postprocess_php_response(
                        &state,
                        &mut body,
                        &mut status_code,
                        &mut content_type,
                        &mut resp_headers,
                    );
                    state.security.record_request(client_ip, false);
                    state.metrics.record_request(
                        &php_path,
                        status_code,
                        elapsed_us,
                        body.len() as u64,
                    );
                    write_access_log(
                        &state,
                        &request.method,
                        &request.path,
                        status_code,
                        request_start,
                        &client_ip_str,
                    );
                    let extra_headers: Vec<(&str, &str)> = resp_headers
                        .iter()
                        .filter(|(k, _)| {
                            !k.eq_ignore_ascii_case("Content-Type")
                                && !k.eq_ignore_ascii_case("Content-Length")
                        })
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();
                    return Ok(build_response(
                        status_code,
                        &content_type,
                        body,
                        &extra_headers,
                    ));
                }
                Err(e) => {
                    error!(worker = worker_idx, error = %e, "Persistent worker response decode error");
                    state.metrics.record_request(
                        &php_path,
                        502,
                        request_start.elapsed().as_micros() as u64,
                        0,
                    );
                    return Ok(build_response(
                        502,
                        "text/plain",
                        format!("Worker error: {e}").into_bytes(),
                        &[],
                    ));
                }
            }
        }
        // No ThreadDispatch but persistent_workers=true is a startup bug
        // (the dispatch is always built when persistent workers spawn
        // successfully).  Fail loud rather than silently falling through.
        error!(
            "persistent dispatch: state.thread_dispatch is None — \
             persistent workers were never ready"
        );
        return Ok(build_response(
            500,
            "text/plain",
            b"Server configuration error: persistent workers unavailable".to_vec(),
            &[],
        ));
    }

    // --- Validate request path ---
    if let Err(e) = state.request_guard.validate(&php_path) {
        let body = format!("403 Forbidden: {e}");
        state.metrics.record_security_block();
        state.metrics.record_request(
            &php_path,
            403,
            request_start.elapsed().as_micros() as u64,
            0,
        );
        return Ok(build_response(403, "text/plain", body.into_bytes(), &[]));
    }

    // --- Read source from disk ---
    let source = match std::fs::read(state.request_guard.root().join(&php_path)) {
        Ok(s) => s,
        Err(e) => {
            let body = format!("File read error: {e}");
            return Ok(build_response(500, "text/plain", body.into_bytes(), &[]));
        }
    };

    let source_hash = ResponseCache::hash_source(&source);
    if let Some(cached) = state.cache.get(&request.method, &request.path, source_hash) {
        let elapsed = request_start.elapsed();
        state.metrics.record_cache_hit();
        state.metrics.record_request(
            &php_path,
            cached.status,
            elapsed.as_micros() as u64,
            cached.body.len() as u64,
        );
        state.security.record_request(client_ip, false);
        debug!(path = %request.path, elapsed_us = elapsed.as_micros(), "Cache hit");
        return Ok(build_response(
            cached.status,
            &cached.content_type,
            cached.body.clone(),
            &[],
        ));
    }
    state.metrics.record_cache_miss();

    // ── Request coalescing (singleflight) ─────────────────────────────
    // When N concurrent requests arrive for the same cacheable URL and
    // nothing is in the cache yet, invoke PHP only once — followers
    // wait on the leader's result.  Saves server CPU roughly in
    // proportion to traffic concentration (often 50% on real apps).
    //
    // Only GETs with cache enabled are eligible — the cache only stores
    // GET/200 anyway, so coalescing other methods would just add
    // latency for no hit rate.
    let mut coalesce_guard: Option<turbine_cache::LeaderGuard<CoalescedResponse>> =
        if request.method == "GET" && state.cache.is_enabled() {
            let key = format!("GET:{}", request.path);
            match state.cache_coalescer.acquire(&key) {
                turbine_cache::LeaderOrFollower::Leader(guard) => Some(guard),
                turbine_cache::LeaderOrFollower::Follower(follower) => {
                    if let Some(shared) = follower.wait().await {
                        let elapsed = request_start.elapsed();
                        state.metrics.record_request(
                            &php_path,
                            shared.status,
                            elapsed.as_micros() as u64,
                            shared.body.len() as u64,
                        );
                        state.security.record_request(client_ip, false);
                        debug!(
                            path = %request.path,
                            elapsed_us = elapsed.as_micros(),
                            "Coalesced response served"
                        );
                        let hdrs: Vec<(&str, &str)> = shared
                            .headers
                            .iter()
                            .map(|(k, v)| (k.as_str(), v.as_str()))
                            .collect();
                        return Ok(build_response(
                            shared.status,
                            &shared.content_type,
                            shared.body,
                            &hdrs,
                        ));
                    }
                    // Leader aborted — fall through and execute ourselves.
                    None
                }
            }
        } else {
            None
        };

    let app_root = std::env::current_dir().unwrap_or_default();
    let server_port = state
        .listen
        .split(':')
        .next_back()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);
    let superglobals = request.php_superglobals_code(
        &app_root,
        &php_path,
        &client_ip_str,
        server_port,
        state.is_tls,
    );
    let abs_php_path = app.document_root.join(&php_path);
    let include_path = abs_php_path.display().to_string().replace('\'', "\\'");
    let session_code = if state.session_auto_start {
        "session_start(); "
    } else {
        ""
    };
    let full_code = format!(
        "{superglobals}{session}{bootstrap}include '{include_path}';",
        superglobals = superglobals,
        session = session_code,
        bootstrap = state.php_bootstrap,
    );

    let uploaded_files: Vec<String> = request.files.iter().map(|f| f.tmp_path.clone()).collect();

    if let Some(ref php_tx) = state.php_tx {
        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
        let php_req = PhpRequest {
            code: full_code,
            uploaded_files,
            response_tx: resp_tx,
        };

        if php_tx.send(php_req).await.is_err() {
            return Ok(build_response(
                500,
                "text/plain",
                b"PHP executor unavailable".to_vec(),
                &[],
            ));
        }

        let php_result = if state.request_timeout.is_zero() {
            resp_rx.await
        } else {
            match tokio::time::timeout(state.request_timeout, resp_rx).await {
                Ok(result) => result,
                Err(_) => {
                    warn!(method = %request.method, path = %request.path, timeout_s = state.request_timeout.as_secs(), "Request timeout");
                    state.metrics.record_request(
                        &php_path,
                        504,
                        request_start.elapsed().as_micros() as u64,
                        0,
                    );
                    write_access_log(
                        &state,
                        &request.method,
                        &request.path,
                        504,
                        request_start,
                        &client_ip_str,
                    );
                    return Ok(build_response(
                        504,
                        "text/plain",
                        b"Request timeout".to_vec(),
                        &[],
                    ));
                }
            }
        };

        match php_result {
            Ok(Ok(mut response)) => {
                if let Some((status_code, headers, body)) =
                    parse_turbine_response_envelope(&response.body)
                {
                    response.status_code = status_code;
                    response.headers = headers;
                    response.body = body;
                }

                let elapsed = request_start.elapsed();
                let elapsed_us = elapsed.as_micros() as u64;
                let php_content_type = response
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
                    .map(|(_, v)| v.as_str());
                let mut content_type = php_content_type
                    .unwrap_or_else(|| detect_content_type(&response.body))
                    .to_string();
                let mut status_code = response.status_code;

                postprocess_php_response(
                    &state,
                    &mut response.body,
                    &mut status_code,
                    &mut content_type,
                    &mut response.headers,
                );

                state.security.record_request(client_ip, false);
                state.metrics.record_request(
                    &php_path,
                    status_code,
                    elapsed_us,
                    response.body.len() as u64,
                );
                if !response_prevents_caching(&response.headers) {
                    state.cache.put(
                        &request.method,
                        &request.path,
                        source_hash,
                        status_code,
                        &content_type,
                        &response.body,
                    );
                }
                if let Some(ref mut g) = coalesce_guard {
                    g.publish(CoalescedResponse {
                        status: status_code,
                        content_type: content_type.clone(),
                        body: Bytes::copy_from_slice(&response.body),
                        headers: response.headers.clone(),
                    });
                }

                info!(method = %request.method, path = %request.path, status = status_code, elapsed_us = elapsed_us, "Request completed");
                write_access_log(
                    &state,
                    &request.method,
                    &request.path,
                    status_code,
                    request_start,
                    &client_ip_str,
                );

                let extra_headers: Vec<(&str, &str)> = response
                    .headers
                    .iter()
                    .filter(|(k, _)| {
                        !k.eq_ignore_ascii_case("Content-Type")
                            && !k.eq_ignore_ascii_case("Content-Length")
                    })
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();

                Ok(build_response(
                    status_code,
                    &content_type,
                    response.body,
                    &extra_headers,
                ))
            }
            Ok(Err(e)) => {
                state.security.record_request(client_ip, true);
                state.metrics.record_request(
                    &php_path,
                    500,
                    request_start.elapsed().as_micros() as u64,
                    0,
                );
                if let Some(ref page) = *state
                    .error_page_500
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                {
                    Ok(build_response(
                        500,
                        "text/html; charset=utf-8",
                        page.clone(),
                        &[],
                    ))
                } else {
                    let body = format!("PHP Error: {e}");
                    Ok(build_response(500, "text/plain", body.into_bytes(), &[]))
                }
            }
            Err(_) => Ok(build_response(
                500,
                "text/plain",
                b"PHP executor channel closed".to_vec(),
                &[],
            )),
        }
    } else if let Some(td) = state
        .thread_dispatch
        .as_ref()
        .filter(|_| find_pool(&state, &clean_path).is_some_and(|r| r.pool_index.is_none()))
    {
        // ── Thread-mode classic dispatch (lock-free) ─────────────────
        let timeout_dur = if state.max_wait_time > 0 {
            std::time::Duration::from_secs(state.max_wait_time)
        } else if state.request_timeout.is_zero() {
            std::time::Duration::from_secs(60)
        } else {
            state.request_timeout
        };

        let worker_idx = match td.get_idle(timeout_dur).await {
            Some(idx) => idx,
            None => {
                state.metrics.record_request(
                    &php_path,
                    504,
                    request_start.elapsed().as_micros() as u64,
                    0,
                );
                return Ok(build_response(
                    504,
                    "text/plain",
                    b"Request timeout waiting for worker".to_vec(),
                    &[],
                ));
            }
        };

        // Build the native request payload
        let full_uri_owned;
        let full_uri: &str = if request.query_string.is_empty() {
            &request.path
        } else {
            full_uri_owned = format!("{}?{}", request.path, request.query_string);
            &full_uri_owned
        };
        let headers_vec: Vec<(&str, &str)> = request
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let content_type_str = request.content_type.as_deref().unwrap_or("");
        // O(1): request.headers is a HashMap with lowercase keys (see compat.rs).
        let cookie_header = request
            .headers
            .get("cookie")
            .map(String::as_str)
            .unwrap_or("");
        let content_length: i32 = if request.body.is_empty() {
            -1
        } else {
            request.body.len() as i32
        };
        let document_root = &app.document_root_str;
        let script_path_native = abs_php_path.display().to_string();
        let script_name = format!("/{}", &php_path);

        // ── Send request and receive response ─────────────────────
        // IdleGuard ensures the worker index returns to idle even if
        // the task is cancelled (e.g. client disconnect).
        let guard = IdleGuard::new(td.clone(), worker_idx);

        let native_result: Result<NativeResponse, String> = if td.has_channels() {
            // In-memory channel IPC (zero syscalls); must own Vec<u8>.
            let encoded = encode_native_request(
                &script_path_native,
                &request.method,
                full_uri,
                &request.query_string,
                content_type_str,
                content_length,
                cookie_header,
                document_root,
                &client_ip_str,
                0,
                server_port,
                state.is_tls,
                &request.path,
                &script_name,
                &request.body,
                &headers_vec,
            );
            if let Err(e) = td.send_request(worker_idx, encoded) {
                error!(worker = worker_idx, error = %e, "Channel send failed (thread dispatch)");
                td.mark_unhealthy(worker_idx);
                // guard will return_idle on drop
                return Ok(build_response(
                    502,
                    "text/plain",
                    b"Worker communication error".to_vec(),
                    &[],
                ));
            }
            match td.recv_response(worker_idx).await {
                Some(resp) => Ok(resp),
                None => {
                    td.mark_unhealthy(worker_idx);
                    Err("channel worker died".to_string())
                }
            }
        } else {
            // Pipe-based IPC (legacy / persistent fallback)
            let (cmd_fd, resp_fd) = td.fds(worker_idx);
            let write_result = turbine_worker::with_encode_scratch(|buf| {
                turbine_worker::encode_native_request_into(
                    buf,
                    &script_path_native,
                    &request.method,
                    full_uri,
                    &request.query_string,
                    content_type_str,
                    content_length,
                    cookie_header,
                    document_root,
                    &client_ip_str,
                    0,
                    server_port,
                    state.is_tls,
                    &request.path,
                    &script_name,
                    &request.body,
                    &headers_vec,
                );
                write_to_fd(cmd_fd, buf)
            });
            if let Err(e) = write_result {
                error!(worker = worker_idx, error = %e, "Failed to send to worker (thread dispatch)");
                td.mark_unhealthy(worker_idx);
                return Ok(build_response(
                    502,
                    "text/plain",
                    b"Worker communication error".to_vec(),
                    &[],
                ));
            }
            // AsyncFd-based read: reactor handles readiness, no blocking
            // pool thread is consumed per in-flight request.
            let result: std::io::Result<NativeResponse> =
                match turbine_worker::async_io::AsyncPipe::new(resp_fd) {
                    Ok(mut pipe) => turbine_worker::read_native_response_async(&mut pipe).await,
                    Err(e) => Err(e),
                };
            if result.is_err() {
                td.mark_unhealthy(worker_idx);
            }
            result.map_err(|e| e.to_string())
        };

        // Explicitly drop the guard now to return worker to idle pool.
        drop(guard);

        if native_result.is_ok() {
            td.record_served(worker_idx);
        }

        match native_result {
            Ok(resp) => {
                let mut body = resp.body;
                let mut status_code = resp.http_status;
                let elapsed_us = request_start.elapsed().as_micros() as u64;
                let php_content_type = resp
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
                    .map(|(_, v)| v.as_str());
                let mut content_type = php_content_type
                    .unwrap_or_else(|| detect_content_type(&body))
                    .to_string();
                let mut resp_headers = resp.headers;
                postprocess_php_response(
                    &state,
                    &mut body,
                    &mut status_code,
                    &mut content_type,
                    &mut resp_headers,
                );
                state.security.record_request(client_ip, false);
                state
                    .metrics
                    .record_request(&php_path, status_code, elapsed_us, body.len() as u64);
                if !response_prevents_caching(&resp_headers) {
                    state.cache.put(
                        &request.method,
                        &request.path,
                        source_hash,
                        status_code,
                        &content_type,
                        &body,
                    );
                }
                if let Some(ref mut g) = coalesce_guard {
                    g.publish(CoalescedResponse {
                        status: status_code,
                        content_type: content_type.clone(),
                        body: Bytes::copy_from_slice(&body),
                        headers: resp_headers.clone(),
                    });
                }
                write_access_log(
                    &state,
                    &request.method,
                    &request.path,
                    status_code,
                    request_start,
                    &client_ip_str,
                );
                let extra_headers: Vec<(&str, &str)> = resp_headers
                    .iter()
                    .filter(|(k, _)| {
                        !k.eq_ignore_ascii_case("Content-Type")
                            && !k.eq_ignore_ascii_case("Content-Length")
                    })
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                Ok(build_response(
                    status_code,
                    &content_type,
                    body,
                    &extra_headers,
                ))
            }
            Err(e) => {
                state.metrics.record_request(
                    &php_path,
                    502,
                    request_start.elapsed().as_micros() as u64,
                    0,
                );
                Ok(build_response(
                    502,
                    "text/plain",
                    format!("Worker error: {e}").into_bytes(),
                    &[],
                ))
            }
        }
    } else if let Some(resolved) = find_pool(&state, &clean_path) {
        let pool_mutex = resolved.pool;
        let pool_index = resolved.pool_index;
        // Acquire a semaphore permit — this queues the request (blocks the async task)
        // until a worker slot is available. At most N concurrent PHP executions.
        // Use acquire_owned() so the OwnedSemaphorePermit is 'static and safe across .await.
        let permit = if let Some(sem) = resolved.semaphore {
            let sem_arc = sem.clone();
            let timeout_dur = if state.max_wait_time > 0 {
                std::time::Duration::from_secs(state.max_wait_time)
            } else if state.request_timeout.is_zero() {
                std::time::Duration::from_secs(60)
            } else {
                state.request_timeout
            };
            match tokio::time::timeout(timeout_dur, sem_arc.acquire_owned()).await {
                Ok(Ok(permit)) => Some(permit),
                Ok(Err(_)) => {
                    return Ok(build_response(
                        503,
                        "text/plain",
                        b"Worker pool closed".to_vec(),
                        &[],
                    ));
                }
                Err(_) => {
                    state.metrics.record_request(
                        &php_path,
                        504,
                        request_start.elapsed().as_micros() as u64,
                        0,
                    );
                    return Ok(build_response(
                        504,
                        "text/plain",
                        b"Request timeout waiting for worker".to_vec(),
                        &[],
                    ));
                }
            }
        } else {
            None
        };

        // Step 1: Claim a worker, send the request, capture resp_fd — then RELEASE the lock.
        // The blocking pipe-read must happen OUTSIDE the mutex so other workers can
        // serve concurrent requests in parallel.
        let (worker_idx, resp_fd) = {
            let mut pool = pool_mutex.lock();

            let worker_idx = match pool.get_idle_worker() {
                Some(idx) => idx,
                None => {
                    if state.worker_mode == WorkerMode::Thread {
                        let _ = pool.reap_and_respawn_threaded(worker_event_loop_native);
                    } else {
                        let _ = pool.reap_and_respawn(worker_event_loop_native);
                    }
                    match pool.get_idle_worker() {
                        Some(idx) => idx,
                        None => {
                            return Ok(build_response(
                                503,
                                "text/plain",
                                b"All workers busy".to_vec(),
                                &[],
                            ));
                        }
                    }
                }
            };

            let resp_fd = if let Some(worker) = pool.worker_mut(worker_idx) {
                worker.mark_busy();

                let send_result = if state.persistent_workers {
                    // Build the URI (path + query string) for the binary protocol.
                    let full_uri_owned;
                    let full_uri: &str = if request.query_string.is_empty() {
                        &request.path
                    } else {
                        full_uri_owned = format!("{}?{}", request.path, request.query_string);
                        &full_uri_owned
                    };
                    let headers_vec: Vec<(&str, &str)> = request
                        .headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();
                    let content_type = request.content_type.as_deref().unwrap_or("");
                    // O(1): request.headers is a HashMap with lowercase keys (see compat.rs).
                    let cookie_header = request
                        .headers
                        .get("cookie")
                        .map(String::as_str)
                        .unwrap_or("");
                    let document_root = &app.document_root_str;
                    let script_filename = abs_php_path.display().to_string();
                    let script_name = format!("/{}", &php_path);
                    let per = PersistentRequest {
                        method: &request.method,
                        uri: full_uri,
                        body: &request.body,
                        client_ip: &client_ip_str,
                        port: server_port,
                        is_https: state.is_tls,
                        headers: &headers_vec,
                        script_filename: &script_filename,
                        query_string: &request.query_string,
                        document_root,
                        content_type,
                        cookie: cookie_header,
                        path_info: &request.path,
                        script_name: &script_name,
                    };
                    let encoded = encode_request(&per);
                    worker.send_request(&encoded)
                } else {
                    // Native SAPI path: send binary request with script path + HTTP metadata
                    let full_uri_owned_native;
                    let full_uri_native: &str = if request.query_string.is_empty() {
                        &request.path
                    } else {
                        full_uri_owned_native =
                            format!("{}?{}", request.path, request.query_string);
                        &full_uri_owned_native
                    };
                    let headers_vec: Vec<(&str, &str)> = request
                        .headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();
                    let content_type = request.content_type.as_deref().unwrap_or("");
                    // O(1): request.headers is a HashMap with lowercase keys (see compat.rs).
                    let cookie_header = request
                        .headers
                        .get("cookie")
                        .map(String::as_str)
                        .unwrap_or("");
                    let content_length: i32 = if request.body.is_empty() {
                        -1
                    } else {
                        request.body.len() as i32
                    };
                    let document_root = &app.document_root_str;
                    let script_path_native = abs_php_path.display().to_string();
                    let script_name = format!("/{}", &php_path);

                    let encoded = encode_native_request(
                        &script_path_native,
                        &request.method,
                        full_uri_native,
                        &request.query_string,
                        content_type,
                        content_length,
                        cookie_header,
                        document_root,
                        &client_ip_str,
                        0, // remote_port
                        server_port,
                        state.is_tls,
                        &request.path,
                        &script_name,
                        &request.body,
                        &headers_vec,
                    );
                    worker.send_request(&encoded)
                };

                if let Err(e) = send_result {
                    error!(worker = worker_idx, error = %e, "Failed to send to worker");
                    pool.return_worker(worker_idx);
                    return Ok(build_response(
                        502,
                        "text/plain",
                        b"Worker communication error".to_vec(),
                        &[],
                    ));
                }
                worker.resp_fd()
            } else {
                return Ok(build_response(
                    502,
                    "text/plain",
                    b"Worker unavailable".to_vec(),
                    &[],
                ));
            };

            (worker_idx, resp_fd)
        }; // ← MUTEX RELEASED HERE — other workers can now handle concurrent requests

        // Keep a copy for logging after the spawned task consumes worker_idx
        let worker_idx_log = worker_idx;

        // Step 2: Read the PHP response WITHOUT holding the mutex.
        // Everything from here (read + return_worker + permit) runs inside
        // tokio::spawn so it always completes even if the parent is cancelled
        // (e.g. client disconnects), preventing worker starvation.
        let is_persistent = state.persistent_workers;
        let return_state = state.clone();

        // Use a single spawned task for both persistent and classic paths.
        // The permit is moved into the task so it's held until the worker finishes.
        enum WorkerResult {
            Persistent(Result<turbine_worker::persistent::PersistentResponse, std::io::Error>),
            Native(Result<NativeResponse, std::io::Error>),
        }
        let reader_handle = tokio::spawn(async move {
            // Hold permit until task completes.
            let _permit_guard = permit;
            // AsyncFd-based read: no blocking pool thread consumed per request.
            let result = match turbine_worker::async_io::AsyncPipe::new(resp_fd) {
                Ok(mut pipe) => {
                    if is_persistent {
                        WorkerResult::Persistent(
                            turbine_worker::decode_response_async(&mut pipe).await,
                        )
                    } else {
                        WorkerResult::Native(
                            turbine_worker::read_native_response_async(&mut pipe).await,
                        )
                    }
                }
                Err(e) => {
                    if is_persistent {
                        WorkerResult::Persistent(Err(e))
                    } else {
                        WorkerResult::Native(Err(e))
                    }
                }
            };
            // Always return the worker after reading
            return_worker_to_pool(&return_state, pool_index, worker_idx);
            result
        });

        let worker_result = reader_handle
            .await
            .unwrap_or_else(|e| WorkerResult::Native(Err(std::io::Error::other(e.to_string()))));

        match worker_result {
            WorkerResult::Persistent(bin_result) => match bin_result {
                Ok(resp) => {
                    let mut body = resp.body;
                    let mut status_code = resp.status;

                    let elapsed = request_start.elapsed();
                    let elapsed_us = elapsed.as_micros() as u64;
                    let php_content_type = resp
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
                        .map(|(_, v)| v.as_str());
                    let mut content_type = php_content_type
                        .unwrap_or_else(|| detect_content_type(&body))
                        .to_string();
                    let mut resp_headers = resp.headers;

                    postprocess_php_response(
                        &state,
                        &mut body,
                        &mut status_code,
                        &mut content_type,
                        &mut resp_headers,
                    );

                    state.security.record_request(client_ip, false);
                    state.metrics.record_request(
                        &php_path,
                        status_code,
                        elapsed_us,
                        body.len() as u64,
                    );
                    if !response_prevents_caching(&resp_headers) {
                        state.cache.put(
                            &request.method,
                            &request.path,
                            source_hash,
                            status_code,
                            &content_type,
                            &body,
                        );
                    }
                    if let Some(ref mut g) = coalesce_guard {
                        g.publish(CoalescedResponse {
                            status: status_code,
                            content_type: content_type.clone(),
                            body: Bytes::copy_from_slice(&body),
                            headers: resp_headers.clone(),
                        });
                    }

                    info!(method = %request.method, path = %request.path, worker = worker_idx_log, status = status_code, elapsed_us = elapsed_us, bytes = body.len(), "Request completed");
                    write_access_log(
                        &state,
                        &request.method,
                        &request.path,
                        status_code,
                        request_start,
                        &client_ip_str,
                    );

                    let extra_headers: Vec<(&str, &str)> = resp_headers
                        .iter()
                        .filter(|(k, _)| {
                            !k.eq_ignore_ascii_case("Content-Type")
                                && !k.eq_ignore_ascii_case("Content-Length")
                        })
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();

                    Ok(build_response(
                        status_code,
                        &content_type,
                        body,
                        &extra_headers,
                    ))
                }
                Err(e) => {
                    state.security.record_request(client_ip, true);
                    state.metrics.record_request(
                        &php_path,
                        502,
                        request_start.elapsed().as_micros() as u64,
                        0,
                    );
                    error!(worker = worker_idx_log, error = %e, "Failed to read persistent worker response");
                    Ok(build_response(
                        502,
                        "text/plain",
                        b"Worker response error".to_vec(),
                        &[],
                    ))
                }
            },
            WorkerResult::Native(native_result) => match native_result {
                Ok(resp) => {
                    let mut body = resp.body;
                    let mut status_code = if resp.http_status == 0 {
                        200
                    } else {
                        resp.http_status
                    };

                    let elapsed = request_start.elapsed();
                    let elapsed_us = elapsed.as_micros() as u64;
                    let php_content_type = resp
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
                        .map(|(_, v)| v.as_str());
                    let mut content_type = php_content_type
                        .unwrap_or_else(|| detect_content_type(&body))
                        .to_string();
                    let mut resp_headers = resp.headers;

                    postprocess_php_response(
                        &state,
                        &mut body,
                        &mut status_code,
                        &mut content_type,
                        &mut resp_headers,
                    );

                    state.security.record_request(client_ip, !resp.success);
                    state.metrics.record_request(
                        &php_path,
                        status_code,
                        elapsed_us,
                        body.len() as u64,
                    );
                    if !response_prevents_caching(&resp_headers) {
                        state.cache.put(
                            &request.method,
                            &request.path,
                            source_hash,
                            status_code,
                            &content_type,
                            &body,
                        );
                    }
                    if let Some(ref mut g) = coalesce_guard {
                        g.publish(CoalescedResponse {
                            status: status_code,
                            content_type: content_type.clone(),
                            body: Bytes::copy_from_slice(&body),
                            headers: resp_headers.clone(),
                        });
                    }

                    info!(method = %request.method, path = %request.path, worker = worker_idx_log, status = status_code, elapsed_us = elapsed_us, bytes = body.len(), "Request completed");
                    write_access_log(
                        &state,
                        &request.method,
                        &request.path,
                        status_code,
                        request_start,
                        &client_ip_str,
                    );

                    let extra_headers: Vec<(&str, &str)> = resp_headers
                        .iter()
                        .filter(|(k, _)| {
                            !k.eq_ignore_ascii_case("Content-Type")
                                && !k.eq_ignore_ascii_case("Content-Length")
                        })
                        .map(|(k, v)| (k.as_str(), v.as_str()))
                        .collect();

                    Ok(build_response(
                        status_code,
                        &content_type,
                        body,
                        &extra_headers,
                    ))
                }
                Err(e) => {
                    state.security.record_request(client_ip, true);
                    state.metrics.record_request(
                        &php_path,
                        502,
                        request_start.elapsed().as_micros() as u64,
                        0,
                    );
                    error!(worker = worker_idx_log, error = %e, "Failed to read native worker response");
                    Ok(build_response(
                        502,
                        "text/plain",
                        b"Worker response error".to_vec(),
                        &[],
                    ))
                }
            },
        }
    } else {
        Ok(build_response(
            500,
            "text/plain",
            b"No PHP executor configured".to_vec(),
            &[],
        ))
    }
}

/// Post-process a PHP response: extract structured logs, handle X-Sendfile,
/// add Early Hints Link headers.
/// Returns true when the PHP response headers indicate the response must not
/// be stored in a shared cache (Cache-Control: no-store / no-cache / private).
fn response_prevents_caching(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("Cache-Control")
            && (v.contains("no-store") || v.contains("no-cache") || v.contains("private"))
    })
}

fn postprocess_php_response(
    state: &ServerState,
    body: &mut Vec<u8>,
    status_code: &mut u16,
    content_type: &mut String,
    headers: &mut Vec<(String, String)>,
) {
    // 1. Structured logging: extract __TURBINE_LOG__ markers from body
    if state.structured_logging_enabled {
        let (cleaned, entries) = features::extract_structured_logs(body);
        if !entries.is_empty() {
            *body = cleaned;
            for entry in &entries {
                features::emit_log_entry(entry);
            }
        }
    }

    // 2. Early Hints: extract Link headers and include in final response
    if state.early_hints_enabled {
        let hints = features::extract_early_hints(headers);
        // Link headers are already present in the headers vec — they'll be
        // forwarded as-is. Nothing extra to do for HTTP/1.1.
        // For HTTP/2, we'd send 103 frames here.
        if !hints.is_empty() {
            debug!(hints = ?hints, "Early Hints detected (Link headers preserved)");
        }
    }

    // 3. X-Sendfile / X-Accel-Redirect: replace body with file contents
    if state.x_sendfile_enabled {
        if let Some(sendfile_path) = features::check_x_sendfile(headers) {
            if let Some(ref root) = state.x_sendfile_root {
                if let Some(resolved) = features::resolve_sendfile_path(&sendfile_path, root) {
                    if let Some((file_ct, file_body)) = features::serve_sendfile(&resolved) {
                        *body = file_body;
                        *content_type = file_ct;
                        *status_code = 200;
                        // Remove X-Accel-Redirect / X-Sendfile headers from response
                        headers.retain(|(k, _)| {
                            !k.eq_ignore_ascii_case("X-Accel-Redirect")
                                && !k.eq_ignore_ascii_case("X-Sendfile")
                        });
                    }
                }
            }
        }
    }
}

#[allow(clippy::type_complexity)]
fn parse_turbine_response_envelope(body: &[u8]) -> Option<(u16, Vec<(String, String)>, Vec<u8>)> {
    let status_marker = TURBINE_STATUS_MARKER.as_bytes();
    let body_marker = TURBINE_BODY_MARKER.as_bytes();

    // Scan for status marker - may not be at position 0 if PHP emitted warnings/notices first
    let envelope_start = body.windows(status_marker.len())
        .position(|w| w == status_marker)
        .or_else(|| {
            // Debug: log first 80 bytes when marker not found
            let preview = &body[..body.len().min(80)];
            debug!(preview = ?String::from_utf8_lossy(preview), "Turbine envelope marker not found");
            None
        })?;

    let envelope = &body[envelope_start..];
    let body_marker_pos = envelope
        .windows(body_marker.len())
        .position(|w| w == body_marker)?;

    let meta = std::str::from_utf8(&envelope[..body_marker_pos]).ok()?;
    let payload = envelope[body_marker_pos + body_marker.len()..].to_vec();

    let mut status_code = 200u16;
    let mut headers = Vec::new();

    for line in meta.lines() {
        if let Some(rest) = line.strip_prefix(TURBINE_STATUS_MARKER) {
            status_code = rest.trim().parse().unwrap_or(200);
            continue;
        }

        if let Some(rest) = line.strip_prefix(TURBINE_HEADER_MARKER) {
            let mut parts = rest.splitn(2, '\t');
            let name = match parts.next() {
                Some(n) => n.trim(),
                None => continue,
            };
            let value = match parts.next() {
                Some(v) => v.trim(),
                None => continue,
            };
            headers.push((name.to_string(), value.to_string()));
        }
    }

    Some((status_code, headers, payload))
}

/// Compress response body with gzip if it exceeds min_size and the content type is compressible.
fn is_compressible_content_type(content_type: &str) -> bool {
    content_type.contains("text/")
        || content_type.contains("application/json")
        || content_type.contains("application/javascript")
        || content_type.contains("application/xml")
        || content_type.contains("image/svg+xml")
        || content_type.contains("application/manifest+json")
}

fn gzip_compress(data: &[u8], level: u32) -> Vec<u8> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(level));
    if encoder.write_all(data).is_err() {
        return data.to_vec();
    }
    encoder.finish().unwrap_or_else(|_| data.to_vec())
}

fn brotli_compress(data: &[u8], level: u32) -> Vec<u8> {
    let mut output = Vec::new();
    let quality = level.min(11);
    let params = brotli::enc::BrotliEncoderParams {
        quality: quality as i32,
        ..Default::default()
    };
    if brotli::BrotliCompress(&mut &data[..], &mut output, &params).is_err() {
        return data.to_vec();
    }
    output
}

fn zstd_compress(data: &[u8], level: u32) -> Vec<u8> {
    let zstd_level = level.min(19) as i32;
    zstd::bulk::compress(data, zstd_level).unwrap_or_else(|_| data.to_vec())
}

/// Negotiate the best compression algorithm based on client's Accept-Encoding
/// and server's preferred order. Returns (encoding_name, compressed_data).
fn negotiate_compression(
    accept_encoding: &str,
    server_prefs: &[String],
    data: &[u8],
    level: u32,
) -> Option<(&'static str, Vec<u8>)> {
    let ae = accept_encoding.to_lowercase();
    for pref in server_prefs {
        match pref.as_str() {
            "br" if ae.contains("br") => {
                return Some(("br", brotli_compress(data, level)));
            }
            "zstd" if ae.contains("zstd") => {
                return Some(("zstd", zstd_compress(data, level)));
            }
            "gzip" if ae.contains("gzip") => {
                return Some(("gzip", gzip_compress(data, level)));
            }
            _ => {}
        }
    }
    None
}

/// Check if a request origin is allowed by the CORS config.
fn cors_origin_allowed(cors: &config::CorsConfig, origin: &str) -> bool {
    cors.allow_origins.iter().any(|o| o == "*" || o == origin)
}

/// Apply CORS headers to a response.
fn apply_cors_headers(
    headers: &mut hyper::header::HeaderMap,
    cors: &config::CorsConfig,
    origin: &str,
) {
    use hyper::header::HeaderValue;

    let origin_value = if cors.allow_origins.iter().any(|o| o == "*") && !cors.allow_credentials {
        "*"
    } else {
        origin
    };
    if let Ok(val) = HeaderValue::from_str(origin_value) {
        headers.insert("Access-Control-Allow-Origin", val);
    }

    if cors.allow_credentials {
        headers.insert(
            "Access-Control-Allow-Credentials",
            HeaderValue::from_static("true"),
        );
    }

    let methods = cors.allow_methods.join(", ");
    if let Ok(val) = HeaderValue::from_str(&methods) {
        headers.insert("Access-Control-Allow-Methods", val);
    }

    let allow_headers = cors.allow_headers.join(", ");
    if let Ok(val) = HeaderValue::from_str(&allow_headers) {
        headers.insert("Access-Control-Allow-Headers", val);
    }

    if !cors.expose_headers.is_empty() {
        let expose = cors.expose_headers.join(", ");
        if let Ok(val) = HeaderValue::from_str(&expose) {
            headers.insert("Access-Control-Expose-Headers", val);
        }
    }

    if cors.max_age > 0 {
        headers.insert("Access-Control-Max-Age", cors.max_age.into());
    }
}

/// Parse PHP-style size strings like "64M", "2G", "128K", "1024" into bytes.
/// Returns `None` for "0", empty, or unparseable input (= no limit).
fn parse_php_size(s: &str) -> Option<usize> {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed == "0" {
        return None;
    }
    let (num_part, mult): (&str, usize) = match trimmed.chars().last() {
        Some('K') | Some('k') => (&trimmed[..trimmed.len() - 1], 1024),
        Some('M') | Some('m') => (&trimmed[..trimmed.len() - 1], 1024 * 1024),
        Some('G') | Some('g') => (&trimmed[..trimmed.len() - 1], 1024 * 1024 * 1024),
        _ => (trimmed, 1),
    };
    num_part
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_mul(mult))
}

/// Extract the first raw value of `name` from a `k=v&k=v`-style query
/// string.  No percent-decoding is performed — callers on the PHP side
/// either restrict keys to `[A-Za-z0-9_.-]` or base64-url-encode them.
fn query_param<'a>(qs: &'a str, name: &str) -> Option<&'a str> {
    for pair in qs.split('&') {
        let mut it = pair.splitn(2, '=');
        if it.next()? == name {
            return it.next();
        }
    }
    None
}

fn build_response(
    status: u16,
    content_type: &str,
    body: impl Into<Bytes>,
    extra_headers: &[(&str, &str)],
) -> HyperResponse {
    let body: Bytes = body.into();
    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let content_length = body.len();
    let mut builder = Response::builder()
        .status(status_code)
        .header("Content-Type", content_type)
        .header("Content-Length", content_length)
        .header("Server", format!("Turbine/{}", env!("CARGO_PKG_VERSION")))
        // Security headers
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "SAMEORIGIN")
        .header("X-XSS-Protection", "0")
        .header("Referrer-Policy", "strict-origin-when-cross-origin")
        .header(
            "Permissions-Policy",
            "camera=(), microphone=(), geolocation=()",
        );

    for (name, value) in extra_headers {
        // Skip invalid header names/values to prevent panics from PHP code
        // that emit pseudo-headers like "Status: 200 OK"
        if hyper::header::HeaderName::from_bytes(name.as_bytes()).is_ok()
            && hyper::header::HeaderValue::from_str(value).is_ok()
        {
            builder = builder.header(*name, *value);
        }
    }

    builder.body(Full::new(body)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Full::new(Bytes::from("Internal response build error")))
            .unwrap()
    })
}

/// Write an access log entry in Combined Log Format.
fn write_access_log(
    state: &ServerState,
    method: &str,
    path: &str,
    status: u16,
    request_start: Instant,
    client_ip: &str,
) {
    if let Some(ref log_mutex) = state.access_log {
        use std::io::Write;
        let elapsed_ms = request_start.elapsed().as_millis();

        // Format: IP - - [timestamp] "METHOD PATH" STATUS elapsed_ms
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let line = format!("{client_ip} - - [{now}] \"{method} {path}\" {status} {elapsed_ms}ms\n");

        if let Ok(mut writer) = log_mutex.lock() {
            let _ = writer.write_all(line.as_bytes());
            let _ = writer.flush();
        }
    }
}

/// Try to serve a static file. Returns Some(Response) if the file was served.
fn try_serve_static(
    document_root: &std::path::Path,
    uri_path: &str,
    method: &hyper::Method,
    metrics: &MetricsCollector,
    request_start: Instant,
    if_none_match: Option<&str>,
) -> Option<HyperResponse> {
    let clean = uri_path.split('?').next().unwrap_or(uri_path);
    if clean.ends_with(".php") || clean == "/" {
        return None;
    }

    let relative = clean.trim_start_matches('/');
    if relative.is_empty() || relative.contains("..") {
        return None;
    }

    let file_path = document_root.join(relative);

    if let (Ok(resolved), Ok(root)) = (file_path.canonicalize(), document_root.canonicalize()) {
        if !resolved.starts_with(&root) {
            return None;
        }

        if resolved.is_file() {
            match std::fs::read(&resolved) {
                Ok(body) => {
                    let content_type = mime_type_for_extension(relative);

                    // ETag: xxh3 hash of file content
                    let hash = xxhash_rust::xxh3::xxh3_64(&body);
                    let etag = format!("\"{hash:x}\"");

                    // 304 Not Modified: check If-None-Match
                    if let Some(client_etag) = if_none_match {
                        if client_etag == etag || client_etag.trim() == etag {
                            let elapsed = request_start.elapsed();
                            let elapsed_us = elapsed.as_micros() as u64;
                            metrics.record_request(relative, 304, elapsed_us, 0);
                            info!(method = %method, path = uri_path, status = 304, elapsed_us = elapsed_us, "Not modified");
                            return Some(build_response(
                                304,
                                content_type,
                                Vec::new(),
                                &[("ETag", &etag)],
                            ));
                        }
                    }

                    let elapsed = request_start.elapsed();
                    let elapsed_us = elapsed.as_micros() as u64;
                    metrics.record_request(relative, 200, elapsed_us, body.len() as u64);

                    let cache_header = if relative.contains("/assets/") {
                        "public, max-age=31536000, immutable"
                    } else {
                        "public, max-age=3600"
                    };

                    info!(method = %method, path = uri_path, status = 200, elapsed_us = elapsed_us, bytes = body.len(), "Static file served");

                    Some(build_response(
                        200,
                        content_type,
                        body,
                        &[("Cache-Control", cache_header), ("ETag", &etag)],
                    ))
                }
                Err(_) => None,
            }
        } else {
            None
        }
    } else {
        None
    }
}

/// Detect content type from PHP output.
fn detect_content_type(output: &[u8]) -> &'static str {
    let prefix = &output[..output.len().min(256)];
    if prefix.starts_with(b"{") || prefix.starts_with(b"[") {
        "application/json"
    } else if prefix.windows(6).any(|w| w == b"<html>" || w == b"<HTML>")
        || prefix
            .windows(9)
            .any(|w| w == b"<!DOCTYPE" || w == b"<!doctype")
    {
        "text/html; charset=utf-8"
    } else {
        "text/plain; charset=utf-8"
    }
}

/// Map file extension to MIME type for static file serving.
fn mime_type_for_extension(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json",
        "html" | "htm" => "text/html; charset=utf-8",
        "xml" => "application/xml",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}
