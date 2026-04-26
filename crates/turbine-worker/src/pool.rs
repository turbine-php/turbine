use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, RawFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use nix::sys::wait::{waitpid, WaitPidFlag};
use nix::unistd::{fork, ForkResult};
use tracing::{debug, error, info, warn};

use crate::error::WorkerError;
use crate::worker::{Worker, WorkerState};

/// Create a CString from bytes, stripping any interior null bytes rather than
/// silently returning an empty string.  This prevents malicious null-byte
/// injection from causing silent data loss in FFI calls.
///
/// Fast path: the vast majority of inputs (URIs, header names/values, method,
/// script paths) contain no NUL bytes.  `memchr::memchr` is SIMD-accelerated
/// via libc and avoids a full copy+filter pass when there are no NULs.
pub fn safe_cstring(bytes: &[u8]) -> std::ffi::CString {
    // Fast path — no NUL found, do a single copy via CString::new.
    if !bytes.contains(&0) {
        return std::ffi::CString::new(bytes).unwrap_or_default();
    }
    // Slow path — strip NULs.
    let cleaned: Vec<u8> = bytes.iter().copied().filter(|&b| b != 0).collect();
    std::ffi::CString::new(cleaned).unwrap_or_default()
}

/// Worker backend mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerMode {
    /// Fork-based workers (default). Works with NTS and ZTS PHP.
    Process,
    /// Thread-based workers. Requires ZTS PHP.
    Thread,
}

impl WorkerMode {
    /// Parse from a config string. Returns Process for unknown values.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "thread" | "threads" => WorkerMode::Thread,
            _ => WorkerMode::Process,
        }
    }
}

impl std::fmt::Display for WorkerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerMode::Process => write!(f, "process"),
            WorkerMode::Thread => write!(f, "thread"),
        }
    }
}

/// Configuration for the worker pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Number of worker processes/threads.
    pub workers: usize,
    /// Maximum requests per worker before recycling (0 = unlimited).
    pub max_requests: u64,
    /// Worker backend mode (process or thread).
    pub mode: WorkerMode,
    /// Pin each worker to a specific CPU core (Linux only, no-op elsewhere).
    pub pin_workers: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        PoolConfig {
            workers: cpus,
            max_requests: 10_000,
            mode: WorkerMode::Process,
            pin_workers: false,
        }
    }
}

/// Pin the calling thread/process to logical CPU `cpu`.  Linux-only;
/// no-op on macOS and other systems.
///
/// Called inside each worker after fork/spawn to reduce cache
/// thrashing from the scheduler bouncing hot PHP processes across
/// cores.  Only meaningful when `worker_count ≤ core_count` on a
/// dedicated host; otherwise disables work stealing for no benefit.
#[inline]
pub fn pin_to_core(cpu: usize) {
    #[cfg(target_os = "linux")]
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(cpu, &mut set);
        // pid=0 means "current thread" when set via pthread_setaffinity_np,
        // but sched_setaffinity with pid=0 binds the calling thread too.
        let _ = libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cpu;
}

/// Global thread ID counter for thread-mode workers.
static THREAD_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Protocol for master ↔ worker communication over pipes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerCmd {
    /// Execute a PHP script (path follows as length-prefixed string).
    Execute = 1,
    /// Graceful shutdown.
    Shutdown = 2,
    /// Execute a script via the native SAPI path (binary request follows).
    ExecuteNative = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerResp {
    /// Request completed successfully.
    Ok = 1,
    /// Request failed.
    Error = 2,
    /// Worker is ready for next request.
    Ready = 3,
}

/// The worker pool manages forking, lifecycle, and communication
/// with worker processes.
pub struct WorkerPool {
    config: PoolConfig,
    workers: Vec<Worker>,
    idle_queue: VecDeque<usize>,
}

impl WorkerPool {
    /// Create a new worker pool with the given configuration.
    ///
    /// Workers are NOT forked yet — call `spawn_workers()` after
    /// the master process has initialized PHP and populated shared memory.
    pub fn new(config: PoolConfig) -> Self {
        info!(
            workers = config.workers,
            max_requests = config.max_requests,
            "Worker pool created"
        );

        WorkerPool {
            config,
            workers: Vec::new(),
            idle_queue: VecDeque::new(),
        }
    }

    /// Fork all worker processes.
    ///
    /// This must be called AFTER:
    /// 1. PHP is initialized (php_embed_init)
    /// 2. OPcodes are compiled and stored in shared memory
    /// 3. Shared memory is sealed (mprotect PROT_READ)
    ///
    /// Returns `Ok(true)` in the master process, `Ok(false)` in a worker.
    pub fn spawn_workers<F>(&mut self, worker_main: F) -> Result<bool, WorkerError>
    where
        F: Fn(RawFd, RawFd) + Copy,
    {
        info!(count = self.config.workers, "Spawning worker processes");

        for i in 0..self.config.workers {
            let is_master = self.spawn_one(i, worker_main)?;
            if !is_master {
                return Ok(false); // We're in a child — return immediately
            }
        }

        info!(
            spawned = self.workers.len(),
            "All workers spawned successfully"
        );
        Ok(true)
    }

    /// Fork a single worker process.
    ///
    /// Returns `Ok(true)` if we're the master, `Ok(false)` if we're the child.
    fn spawn_one<F>(&mut self, index: usize, worker_main: F) -> Result<bool, WorkerError>
    where
        F: Fn(RawFd, RawFd),
    {
        // Create pipes using raw libc to avoid OwnedFd IO safety issues with fork
        let mut cmd_pipe = [0i32; 2];
        let mut resp_pipe = [0i32; 2];
        if unsafe { libc::pipe(cmd_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        if unsafe { libc::pipe(resp_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        let (cmd_read, cmd_write) = (cmd_pipe[0], cmd_pipe[1]);
        let (resp_read, resp_write) = (resp_pipe[0], resp_pipe[1]);

        match unsafe { fork() }.map_err(WorkerError::Fork)? {
            ForkResult::Parent { child } => {
                // Master: close child's ends
                unsafe {
                    libc::close(cmd_read);
                    libc::close(resp_write);
                }

                // NOTE: we intentionally leave master-side fds in blocking
                // mode here.  The initial ready-signal handshake (persistent
                // workers) relies on blocking `libc::read`.  `AsyncPipe::new`
                // flips the fd to non-blocking on first async use.

                let worker = Worker::new(child, self.config.max_requests, cmd_write, resp_read);

                // Master keeps cmd_write and resp_read (no forget needed)
                self.idle_queue.push_back(self.workers.len());
                self.workers.push(worker);

                debug!(
                    pid = child.as_raw(),
                    index = index,
                    "Worker forked successfully"
                );
                Ok(true)
            }
            ForkResult::Child => {
                // Worker: close master's ends
                unsafe {
                    libc::close(cmd_write);
                    libc::close(resp_read);
                }

                // Optionally pin this worker to a specific CPU core.
                if self.config.pin_workers {
                    let ncpus = std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(1);
                    pin_to_core(index % ncpus);
                }

                // Enter the worker event loop
                worker_main(cmd_read, resp_write);

                // Worker should never return from worker_main
                std::process::exit(0);
            }
        }
    }

    /// Get the next idle worker index, if available.
    ///
    /// Uses **LIFO** (pop from back) so the most-recently-returned worker
    /// is reused first — its OPcache, Zend arenas and CPU caches are
    /// still hot.  This matches the `ThreadDispatch::get_idle` policy.
    pub fn get_idle_worker(&mut self) -> Option<usize> {
        // Find the most-recently-returned idle worker that is still alive
        while let Some(idx) = self.idle_queue.pop_back() {
            if idx < self.workers.len() && self.workers[idx].state() == WorkerState::Idle {
                return Some(idx);
            }
        }
        None
    }

    /// Return a worker to the idle queue after completing a request.
    pub fn return_worker(&mut self, index: usize) {
        if index < self.workers.len() {
            let should_recycle = self.workers[index].mark_idle();
            if should_recycle {
                info!(
                    pid = self.workers[index].pid().as_raw(),
                    "Recycling worker after max requests"
                );
                let _ = self.workers[index].terminate();
                // A new worker would be spawned by the reaper
            } else {
                self.idle_queue.push_back(index);
            }
        }
    }

    /// Return a persistent worker to the idle queue, respawning inline if
    /// it has reached `max_requests`.
    ///
    /// Sends the persistent-protocol shutdown byte (`0xFF`) via the cmd pipe
    /// so the worker exits cleanly (no SIGTERM), then forks a replacement
    /// immediately so the worker slot is never left empty.
    pub fn return_worker_persistent(
        &mut self,
        index: usize,
        app_root: &str,
        worker_boot: Option<&str>,
        worker_handler: Option<&str>,
        worker_cleanup: Option<&str>,
    ) {
        if index < self.workers.len() {
            let should_recycle = self.workers[index].mark_idle();
            if should_recycle {
                info!(
                    pid = self.workers[index].pid().as_raw(),
                    "Recycling persistent worker — graceful shutdown + inline respawn"
                );
                // Send 0xFF shutdown via pipe — the worker will read it and exit(0)
                // instead of being killed by SIGTERM mid-read.
                let cmd_fd = self.workers[index].cmd_fd();
                let shutdown_byte: [u8; 1] = [0xFF];
                let _ = unsafe {
                    libc::write(cmd_fd, shutdown_byte.as_ptr() as *const libc::c_void, 1)
                };
                // Wait for the worker process to exit (non-blocking poll, then blocking).
                let pid = self.workers[index].pid();
                // Brief non-blocking reap attempts (up to 10ms), then SIGTERM fallback.
                let mut reaped = false;
                for _ in 0..10 {
                    match nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
                        Ok(nix::sys::wait::WaitStatus::Exited(_, _))
                        | Ok(nix::sys::wait::WaitStatus::Signaled(_, _, _)) => {
                            reaped = true;
                            break;
                        }
                        _ => {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                    }
                }
                if !reaped {
                    // Fallback to SIGTERM if graceful didn't work in 10ms
                    let _ = self.workers[index].terminate();
                    let _ = nix::sys::wait::waitpid(pid, None);
                }
                if let Err(e) = self.respawn_persistent_at(
                    index,
                    app_root.to_string(),
                    worker_boot.map(|s| s.to_string()),
                    worker_handler.map(|s| s.to_string()),
                    worker_cleanup.map(|s| s.to_string()),
                ) {
                    error!(index = index, error = %e, "Failed to inline respawn persistent worker");
                }
            } else {
                self.idle_queue.push_back(index);
            }
        }
    }

    /// Get a mutable reference to a worker by index.
    pub fn worker_mut(&mut self, index: usize) -> Option<&mut Worker> {
        self.workers.get_mut(index)
    }

    /// Reap exited workers and respawn replacements.
    ///
    /// Call this periodically from the master's event loop.
    pub fn reap_and_respawn<F>(&mut self, worker_main: F) -> Result<(), WorkerError>
    where
        F: Fn(RawFd, RawFd) + Copy,
    {
        let mut to_respawn = Vec::new();

        for (idx, worker) in self.workers.iter_mut().enumerate() {
            if !worker.is_alive() {
                debug!(
                    pid = worker.pid().as_raw(),
                    index = idx,
                    "Worker exited — will respawn"
                );
                to_respawn.push(idx);
            }
        }

        for idx in to_respawn {
            // The old worker is dead, respawn a replacement
            let is_master = self.spawn_one(idx, worker_main)?;
            if !is_master {
                // We're in the new child
                return Ok(());
            }
        }

        Ok(())
    }

    /// Shutdown all workers gracefully.
    pub fn shutdown(&mut self) {
        info!(workers = self.workers.len(), "Shutting down worker pool");

        for worker in &mut self.workers {
            let _ = worker.terminate();
        }

        // Wait for all workers with a timeout
        for worker in &mut self.workers {
            let pid = worker.pid();
            match waitpid(pid, Some(WaitPidFlag::empty())) {
                Ok(_) => debug!(pid = pid.as_raw(), "Worker exited cleanly"),
                Err(e) => warn!(pid = pid.as_raw(), error = %e, "Error waiting for worker"),
            }
        }

        self.workers.clear();
        self.idle_queue.clear();
        info!("Worker pool shut down");
    }

    /// Number of currently managed workers.
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Number of idle workers.
    pub fn idle_count(&self) -> usize {
        self.idle_queue.len()
    }

    /// Access the pool configuration.
    pub fn config(&self) -> &PoolConfig {
        &self.config
    }

    /// Push a freshly spawned worker into the pool and mark it idle.
    ///
    /// Used by modules that fork workers outside of `spawn_one` (e.g. persistent).
    pub(crate) fn push_worker(&mut self, worker: Worker) {
        let idx = self.workers.len();
        self.idle_queue.push_back(idx);
        self.workers.push(worker);
    }

    /// Replace a dead worker at the given index and mark it idle.
    ///
    /// Used by persistent worker respawning.
    pub(crate) fn replace_worker(&mut self, index: usize, worker: Worker) {
        if index < self.workers.len() {
            self.workers[index] = worker;
            self.idle_queue.push_back(index);
        }
    }

    /// Read-only access to workers for health checking.
    pub fn workers_slice(&self) -> &[Worker] {
        &self.workers
    }

    /// Mutable access to the workers vec (for reap_and_respawn_persistent).
    pub(crate) fn workers_mut(&mut self) -> &mut Vec<Worker> {
        &mut self.workers
    }

    /// Spawn one additional worker (for auto-scaling up).
    /// Returns `Ok(true)` in master, `Ok(false)` in child.
    pub fn spawn_additional<F>(&mut self, worker_main: F) -> Result<bool, WorkerError>
    where
        F: Fn(RawFd, RawFd),
    {
        let index = self.workers.len();
        let is_master = self.spawn_one(index, worker_main)?;
        if is_master {
            info!(
                total = self.workers.len(),
                "Scaled up: spawned additional worker"
            );
        }
        Ok(is_master)
    }

    /// Terminate the last idle worker (for auto-scaling down).
    /// Returns true if a worker was removed.
    pub fn shrink_one(&mut self) -> bool {
        // Find the last idle worker in the queue and terminate it
        if let Some(pos) = self.idle_queue.iter().rposition(|_| true) {
            let idx = self.idle_queue.remove(pos).unwrap();
            if idx < self.workers.len() && self.workers[idx].state() == WorkerState::Idle {
                info!(
                    pid = self.workers[idx].pid().as_raw(),
                    index = idx,
                    total = self.workers.len(),
                    "Scaled down: terminating idle worker"
                );
                let _ = self.workers[idx].terminate();
                return true;
            }
        }
        false
    }

    /// Count of workers that are currently busy.
    pub fn busy_count(&self) -> usize {
        self.workers
            .iter()
            .filter(|w| w.state() == WorkerState::Busy)
            .count()
    }

    /// Count of workers that are alive (idle or busy).
    pub fn alive_count(&self) -> usize {
        self.workers
            .iter()
            .filter(|w| matches!(w.state(), WorkerState::Idle | WorkerState::Busy))
            .count()
    }

    /// Get the pool's worker mode.
    pub fn mode(&self) -> WorkerMode {
        self.config.mode
    }

    // ─────────────────────────────────────────────────────────────────
    // Thread-mode spawning (ZTS PHP required)
    // ─────────────────────────────────────────────────────────────────

    /// Spawn all workers as OS threads instead of forked processes.
    ///
    /// Each thread runs `worker_main(cmd_fd, resp_fd)` using the same pipe-based
    /// IPC protocol as process-mode workers. Requires PHP compiled with ZTS.
    ///
    /// Unlike `spawn_workers()`, this always returns `Ok(())` (no fork parent/child
    /// distinction) since threads share the same address space.
    pub fn spawn_workers_threaded<F>(&mut self, worker_main: F) -> Result<(), WorkerError>
    where
        F: Fn(RawFd, RawFd) + Send + 'static + Clone,
    {
        info!(
            count = self.config.workers,
            mode = "thread",
            "Spawning worker threads"
        );

        // Validate ZTS at runtime
        let is_zts = unsafe { turbine_php_sys::turbine_php_is_thread_safe() };
        if is_zts == 0 {
            error!("Thread worker mode requires PHP compiled with ZTS (--enable-zts). Current PHP is NTS.");
            return Err(WorkerError::Fork(nix::Error::ENOTSUP));
        }

        for i in 0..self.config.workers {
            self.spawn_one_thread(i, worker_main.clone())?;
        }

        info!(spawned = self.workers.len(), "All worker threads spawned");
        Ok(())
    }

    /// Spawn a single worker thread.
    fn spawn_one_thread<F>(&mut self, index: usize, worker_main: F) -> Result<(), WorkerError>
    where
        F: Fn(RawFd, RawFd) + Send + 'static,
    {
        // Create pipes (same as process mode)
        let mut cmd_pipe = [0i32; 2];
        let mut resp_pipe = [0i32; 2];
        if unsafe { libc::pipe(cmd_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        if unsafe { libc::pipe(resp_pipe.as_mut_ptr()) } != 0 {
            return Err(WorkerError::Pipe(nix::Error::last()));
        }
        let (cmd_read, cmd_write) = (cmd_pipe[0], cmd_pipe[1]);
        let (resp_read, resp_write) = (resp_pipe[0], resp_pipe[1]);

        // Thread liveness flag
        let alive = Arc::new(AtomicBool::new(true));
        let alive_clone = alive.clone();
        let thread_id = THREAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pin = self.config.pin_workers;

        // Spawn the worker thread
        std::thread::Builder::new()
            .name(format!("turbine-worker-{index}"))
            .spawn(move || {
                // Optionally pin to core (Linux only).
                if pin {
                    let ncpus = std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(1);
                    pin_to_core(index % ncpus);
                }

                // Initialize TSRM context for this thread (ZTS only)
                let init_rc = unsafe { turbine_php_sys::turbine_thread_init() };
                if init_rc != 0 {
                    error!(
                        thread_id = thread_id,
                        "Failed to initialize TSRM context for worker thread"
                    );
                    alive_clone.store(false, Ordering::Release);
                    unsafe {
                        libc::close(cmd_read);
                        libc::close(resp_write);
                    }
                    return;
                }

                // Run the event loop (same function as process mode)
                worker_main(cmd_read, resp_write);

                // Clean up TSRM context
                unsafe {
                    turbine_php_sys::turbine_thread_cleanup();
                }

                // Close our ends of the pipes
                unsafe {
                    libc::close(cmd_read);
                    libc::close(resp_write);
                }

                // Signal that we're done
                alive_clone.store(false, Ordering::Release);
                debug!(thread_id = thread_id, "Worker thread exited");
            })
            .map_err(|e| {
                error!(error = %e, "Failed to spawn worker thread");
                WorkerError::Fork(nix::Error::ENOMEM)
            })?;

        // Master keeps cmd_write and resp_read
        let worker = Worker::new_thread(
            alive,
            thread_id,
            self.config.max_requests,
            cmd_write,
            resp_read,
        );
        self.idle_queue.push_back(self.workers.len());
        self.workers.push(worker);

        debug!(
            thread_id = thread_id,
            index = index,
            "Worker thread spawned"
        );
        Ok(())
    }

    /// Spawn one additional worker thread (for auto-scaling up in thread mode).
    pub fn spawn_additional_thread<F>(&mut self, worker_main: F) -> Result<(), WorkerError>
    where
        F: Fn(RawFd, RawFd) + Send + 'static,
    {
        let index = self.workers.len();
        self.spawn_one_thread(index, worker_main)?;
        info!(
            total = self.workers.len(),
            "Scaled up: spawned additional worker thread"
        );
        Ok(())
    }

    /// Reap dead thread-mode workers and respawn them.
    pub fn reap_and_respawn_threaded<F>(&mut self, worker_main: F) -> Result<(), WorkerError>
    where
        F: Fn(RawFd, RawFd) + Send + 'static + Clone,
    {
        let mut to_respawn = Vec::new();

        for (idx, worker) in self.workers.iter_mut().enumerate() {
            if !worker.is_alive() {
                debug!(index = idx, "Worker thread exited — will respawn");
                to_respawn.push(idx);
            }
        }

        for idx in to_respawn {
            // Spawn a new thread worker and replace the dead one
            let mut cmd_pipe = [0i32; 2];
            let mut resp_pipe = [0i32; 2];
            if unsafe { libc::pipe(cmd_pipe.as_mut_ptr()) } != 0 {
                return Err(WorkerError::Pipe(nix::Error::last()));
            }
            if unsafe { libc::pipe(resp_pipe.as_mut_ptr()) } != 0 {
                return Err(WorkerError::Pipe(nix::Error::last()));
            }
            let (cmd_read, cmd_write) = (cmd_pipe[0], cmd_pipe[1]);
            let (resp_read, resp_write) = (resp_pipe[0], resp_pipe[1]);

            let alive = Arc::new(AtomicBool::new(true));
            let alive_clone = alive.clone();
            let thread_id = THREAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
            let wm = worker_main.clone();

            std::thread::Builder::new()
                .name(format!("turbine-worker-{idx}"))
                .spawn(move || {
                    let init_rc = unsafe { turbine_php_sys::turbine_thread_init() };
                    if init_rc != 0 {
                        alive_clone.store(false, Ordering::Release);
                        unsafe {
                            libc::close(cmd_read);
                            libc::close(resp_write);
                        }
                        return;
                    }
                    wm(cmd_read, resp_write);
                    unsafe {
                        turbine_php_sys::turbine_thread_cleanup();
                    }
                    unsafe {
                        libc::close(cmd_read);
                        libc::close(resp_write);
                    }
                    alive_clone.store(false, Ordering::Release);
                })
                .map_err(|_| WorkerError::Fork(nix::Error::ENOMEM))?;

            let worker = Worker::new_thread(
                alive,
                thread_id,
                self.config.max_requests,
                cmd_write,
                resp_read,
            );
            self.replace_worker(idx, worker);
            info!(
                thread_id = thread_id,
                index = idx,
                "Worker thread respawned"
            );
        }

        Ok(())
    }

    /// Collect (cmd_fd, resp_fd) for all workers (used by ThreadDispatch).
    pub fn worker_fds(&self) -> Vec<(RawFd, RawFd)> {
        self.workers
            .iter()
            .map(|w| (w.cmd_fd(), w.resp_fd()))
            .collect()
    }

    /// Register a thread worker that uses in-memory channels instead of pipes.
    ///
    /// The worker is tracked for lifecycle purposes (alive flag, counts) but
    /// has dummy fds (-1, -1) since IPC goes through `ThreadDispatch` channels.
    pub fn register_channel_thread(&mut self, alive: Arc<AtomicBool>, thread_id: u64) {
        let worker = Worker::new_thread(alive, thread_id, self.config.max_requests, -1, -1);
        self.idle_queue.push_back(self.workers.len());
        self.workers.push(worker);
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        if !self.workers.is_empty() {
            self.shutdown();
        }
    }
}

/// Write a raw byte buffer to a pipe fd.
///
/// Used to send requests to workers outside of a pool lock.
pub fn write_to_fd(fd: RawFd, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::mem::ManuallyDrop;
    let mut file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.write_all(data)
}

/// The worker-side event loop.
///
/// This runs inside the forked child process. After fork(), the child
/// inherits the initialized PHP engine via Copy-on-Write. It reads commands
/// from the master via `cmd_fd`, executes PHP code, captures output, and
/// sends the response (including output bytes) via `resp_fd`.
///
/// Protocol (master → worker):
///   [1 byte cmd] [4 byte code_len LE] [code bytes]
///
/// Protocol (worker → master):
///   [1 byte status] [4 byte output_len LE] [output bytes]
pub fn worker_event_loop(cmd_fd: RawFd, resp_fd: RawFd) {
    debug!(pid = std::process::id(), "Worker event loop started");

    let cmd_file = unsafe { std::fs::File::from_raw_fd(cmd_fd) };
    let mut cmd_reader = std::io::BufReader::with_capacity(8192, cmd_file);
    let mut resp_writer = unsafe { std::fs::File::from_raw_fd(resp_fd) };

    // The PHP engine was initialized in the parent before fork().
    // After fork(), the child inherits the PHP state via CoW.
    // We use the FFI directly to avoid the AtomicBool guard.
    use turbine_engine::output;

    // Re-install our custom ub_write/header_handler in the child process.
    // The parent's php_embed_init sets them up, but after fork the child
    // must reinstall to ensure its own thread-local buffers are used.
    unsafe {
        output::install_output_capture();
    }

    // Signal ready (with zero-length output)
    let _ = write_response(&mut resp_writer, WorkerResp::Ready, &[]);

    loop {
        // Read command byte
        let mut cmd_buf = [0u8; 1];
        match cmd_reader.read_exact(&mut cmd_buf) {
            Ok(_) => {}
            Err(e) => {
                debug!(error = %e, "Command pipe closed — shutting down");
                break;
            }
        }

        match cmd_buf[0] {
            x if x == WorkerCmd::Execute as u8 => {
                // Read PHP code length (u32 LE)
                let mut len_buf = [0u8; 4];
                if cmd_reader.read_exact(&mut len_buf).is_err() {
                    break;
                }
                let code_len = u32::from_le_bytes(len_buf) as usize;

                // Read PHP code
                let mut code_buf = vec![0u8; code_len];
                if cmd_reader.read_exact(&mut code_buf).is_err() {
                    break;
                }
                let code = String::from_utf8_lossy(&code_buf);

                debug!(
                    pid = std::process::id(),
                    code_len = code_len,
                    "Executing PHP"
                );

                // Execute PHP and capture output
                output::clear_output_buffer();

                let c_code = match std::ffi::CString::new(code.as_ref()) {
                    Ok(c) => c,
                    Err(_) => {
                        let err_msg = b"Error: PHP code contains null byte";
                        let _ = write_response(&mut resp_writer, WorkerResp::Error, err_msg);
                        continue;
                    }
                };
                let c_name = std::ffi::CString::new("turbine_worker").expect("static string");

                let result = unsafe {
                    turbine_php_sys::zend_eval_string(
                        c_code.as_ptr(),
                        std::ptr::null_mut(),
                        c_name.as_ptr(),
                    )
                };

                // Flush PHP output buffers BEFORE taking output.
                // Some PHP applications call ob_start() internally; the ob
                // buffers are only flushed to our ub_write callback during
                // php_request_shutdown(). We must shutdown first, then read
                // the Rust output buffer, then start the next request.
                unsafe {
                    turbine_php_sys::php_request_shutdown(std::ptr::null_mut());
                }

                let captured = output::take_output();

                // Start next request cycle immediately so the worker is ready.
                // IMPORTANT: php_request_startup() resets sapi_module.ub_write
                // back to the embed SAPI default (stdout). We must re-install
                // our custom capture hook after every startup.
                unsafe {
                    turbine_php_sys::php_request_startup();
                    output::install_output_capture();
                }

                if result == turbine_php_sys::SUCCESS {
                    let _ = write_response(&mut resp_writer, WorkerResp::Ok, &captured);
                } else {
                    // Include captured output (PHP may have printed errors)
                    let mut err_output = b"PHP Error\n".to_vec();
                    err_output.extend_from_slice(&captured);
                    let _ = write_response(&mut resp_writer, WorkerResp::Error, &err_output);
                }
            }
            x if x == WorkerCmd::Shutdown as u8 => {
                info!(pid = std::process::id(), "Worker received shutdown command");
                break;
            }
            other => {
                warn!(cmd = other, "Unknown command byte");
                let _ = write_response(&mut resp_writer, WorkerResp::Error, b"Unknown command");
            }
        }
    }

    debug!(pid = std::process::id(), "Worker event loop exiting");
}

/// Native SAPI worker event loop — uses php_execute_script() instead of zend_eval_string().
///
/// Key differences from `worker_event_loop`:
///   - Receives a binary request (script path + HTTP metadata) instead of PHP code
///   - Populates SG(request_info) via C helper, letting PHP auto-populate superglobals
///   - Calls php_execute_script() which uses OPcache (cached bytecodes)
///   - Captures headers via SAPI header_handler + status from SG(sapi_headers)
///   - Response includes structured status + headers + body (not raw output)
///
/// Binary request protocol (master → worker):
///   [1 byte cmd=ExecuteNative]
///   [4 byte total_len LE]
///   --- within total_len: ---
///   [2 byte script_path_len LE][script_path bytes]
///   [2 byte method_len LE][method bytes]
///   [2 byte uri_len LE][uri bytes]
///   [2 byte query_string_len LE][query_string bytes]
///   [2 byte content_type_len LE][content_type bytes]
///   [4 byte content_length LE] (as i32, -1 = no body)
///   [2 byte cookie_len LE][cookie bytes]
///   [2 byte document_root_len LE][document_root bytes]
///   [2 byte remote_addr_len LE][remote_addr bytes]
///   [2 byte remote_port LE]
///   [2 byte server_port LE]
///   [1 byte is_https]
///   [2 byte path_info_len LE][path_info bytes]
///   [2 byte script_name_len LE][script_name bytes]
///   [4 byte body_len LE][body bytes]
///   [2 byte header_count LE]
///   for each header:
///     [2 byte key_len LE][key bytes][2 byte val_len LE][val bytes]
///
/// Response protocol (worker → master):
///   [1 byte status (Ok/Error)]
///   [2 byte http_status LE]
///   [2 byte header_count LE]
///   for each header:
///     [2 byte key_len LE][key bytes][2 byte val_len LE][val bytes]
///   [4 byte body_len LE][body bytes]
pub fn worker_event_loop_native(cmd_fd: RawFd, resp_fd: RawFd) {
    debug!(
        pid = std::process::id(),
        "Native SAPI worker event loop started"
    );

    let cmd_file = unsafe { std::fs::File::from_raw_fd(cmd_fd) };
    let mut cmd_reader = std::io::BufReader::with_capacity(8192, cmd_file);
    let mut resp_writer = unsafe { std::fs::File::from_raw_fd(resp_fd) };

    use turbine_engine::output;

    // Install Turbine SAPI hooks (read_post, read_cookies, register_server_variables)
    // and our output capture hooks (ub_write, header_handler).
    unsafe {
        turbine_php_sys::turbine_sapi_install_hooks();
        output::install_output_capture();
    }

    // Signal ready
    let _ = write_native_response(&mut resp_writer, WorkerResp::Ready, 200, &[], &[]);

    loop {
        // Read command byte
        let mut cmd_buf = [0u8; 1];
        match cmd_reader.read_exact(&mut cmd_buf) {
            Ok(_) => {}
            Err(e) => {
                debug!(error = %e, "Command pipe closed — shutting down");
                break;
            }
        }

        match cmd_buf[0] {
            x if x == WorkerCmd::ExecuteNative as u8 => {
                // Read total payload length
                let mut len_buf = [0u8; 4];
                if cmd_reader.read_exact(&mut len_buf).is_err() {
                    break;
                }
                let total_len = u32::from_le_bytes(len_buf) as usize;

                // Read the entire binary payload
                let mut payload = vec![0u8; total_len];
                if cmd_reader.read_exact(&mut payload).is_err() {
                    break;
                }

                // Parse binary request
                let req = match NativeRequest::decode(&payload) {
                    Some(r) => r,
                    None => {
                        let _ = write_native_response(
                            &mut resp_writer,
                            WorkerResp::Error,
                            500,
                            &[],
                            b"Failed to decode native request",
                        );
                        continue;
                    }
                };

                debug!(pid = std::process::id(), script = ?req.script_path, "Executing via native SAPI");

                // CStrings without allocations (zero-copy references from binary protocol)
                let c_method = req.method;
                let c_uri = req.uri;
                let c_qs = req.query_string;
                let c_ct = req.content_type;
                let c_cookie = req.cookie;
                let c_script = req.script_path;
                let c_docroot = req.document_root;
                let c_addr = req.remote_addr;
                let c_pathinfo = req.path_info;
                let c_scriptname = req.script_name;

                // Headers as raw (ptr, len) — no per-request CString allocation.
                let key_ptrs: Vec<*const std::ffi::c_char> =
                    req.headers.iter().map(|(k, _)| k.as_ptr()).collect();
                let key_lens: Vec<usize> = req
                    .headers
                    .iter()
                    .map(|(k, _)| k.to_bytes().len())
                    .collect();
                let val_ptrs: Vec<*const std::ffi::c_char> =
                    req.headers.iter().map(|(_, v)| v.as_ptr()).collect();
                let val_lens: Vec<usize> = req
                    .headers
                    .iter()
                    .map(|(_, v)| v.to_bytes().len())
                    .collect();

                let content_length: libc::c_long = if req.body.is_empty() {
                    -1
                } else {
                    req.body.len() as libc::c_long
                };

                unsafe {
                    // 1. Set SAPI request info (BEFORE php_request_startup)
                    turbine_php_sys::turbine_sapi_set_request(
                        c_method.as_ptr(),
                        c_uri.as_ptr(),
                        c_qs.as_ptr(),
                        if req.content_type.to_bytes().is_empty() {
                            std::ptr::null()
                        } else {
                            c_ct.as_ptr()
                        },
                        content_length,
                        if req.cookie.to_bytes().is_empty() {
                            std::ptr::null()
                        } else {
                            c_cookie.as_ptr()
                        },
                        c_script.as_ptr(),
                        c_docroot.as_ptr(),
                        c_addr.as_ptr(),
                        req.remote_port as libc::c_int,
                        req.server_port as libc::c_int,
                        req.is_https as libc::c_int,
                        c_pathinfo.as_ptr(),
                        c_scriptname.as_ptr(),
                        if req.body.is_empty() {
                            std::ptr::null()
                        } else {
                            req.body.as_ptr() as *const _
                        },
                        req.body.len(),
                        req.headers.len() as libc::c_int,
                        if key_ptrs.is_empty() {
                            std::ptr::null()
                        } else {
                            key_ptrs.as_ptr()
                        },
                        if key_lens.is_empty() {
                            std::ptr::null()
                        } else {
                            key_lens.as_ptr()
                        },
                        if val_ptrs.is_empty() {
                            std::ptr::null()
                        } else {
                            val_ptrs.as_ptr()
                        },
                        if val_lens.is_empty() {
                            std::ptr::null()
                        } else {
                            val_lens.as_ptr()
                        },
                    );

                    // 2. php_request_startup — PHP auto-populates $_SERVER, $_GET, $_POST, $_COOKIE
                    turbine_php_sys::php_request_startup();

                    // 3. Install our output capture (ub_write + header_handler) AFTER startup resets them
                    output::install_output_capture();
                    output::clear_output_buffer();

                    // 4. php_execute_script — uses OPcache, standard Zend VM path
                    let result = turbine_php_sys::turbine_execute_script(c_script.as_ptr());

                    // 5. Full request shutdown — resets all PHP state for next request.
                    //
                    // CRITICAL: shutdown must run BEFORE take_output(). PHP's
                    // internal output_buffering (default 4096 bytes in php.ini)
                    // only flushes chunks to our ub_write callback during
                    // php_output_end_all() inside php_request_shutdown.
                    // Collecting before shutdown truncates any response larger
                    // than output_buffering — manifests as partial/empty bodies
                    // for large payloads (mirrors the persistent-worker path
                    // that already documents this invariant).
                    turbine_php_sys::php_request_shutdown(std::ptr::null_mut());

                    // 6. Now collect the full output.
                    let body = output::take_output();
                    let headers = output::take_headers();
                    let status = output::take_response_code();

                    if result == turbine_php_sys::SUCCESS {
                        let _ = write_native_response(
                            &mut resp_writer,
                            WorkerResp::Ok,
                            status,
                            &headers,
                            &body,
                        );
                    } else {
                        let _ = write_native_response(
                            &mut resp_writer,
                            WorkerResp::Error,
                            status,
                            &headers,
                            &body,
                        );
                    }
                }
            }
            x if x == WorkerCmd::Shutdown as u8 => {
                info!(pid = std::process::id(), "Worker received shutdown command");
                break;
            }
            other => {
                warn!(cmd = other, "Unknown command byte");
                let _ = write_native_response(
                    &mut resp_writer,
                    WorkerResp::Error,
                    500,
                    &[],
                    b"Unknown command",
                );
            }
        }
    }

    debug!(pid = std::process::id(), "Native worker event loop exiting");
}

/// Channel-based native SAPI worker event loop for thread mode.
///
/// Identical to `worker_event_loop_native` but uses in-memory channels
/// instead of pipe fds.  This eliminates 4 syscalls per request (2 pipe
/// reads + 2 pipe writes) and the serialisation overhead of the wire
/// protocol for the *response* — `NativeResponse` is returned directly
/// as a Rust struct with zero-copy.
///
/// The `response_fn` callback is invoked with the `NativeResponse` for
/// each completed request.  In practice this is a
/// `tokio::sync::mpsc::UnboundedSender::send()` that wakes up the async
/// dispatcher without `spawn_blocking`.
pub fn worker_event_loop_channel(
    request_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    response_fn: impl Fn(NativeResponse) + Send + 'static,
) {
    debug!("Channel-based native SAPI worker event loop started");

    use turbine_engine::output;

    unsafe {
        turbine_php_sys::turbine_sapi_install_hooks();
        output::install_output_capture();
    }

    // Signal ready via the response channel
    response_fn(NativeResponse {
        success: true,
        http_status: 200,
        headers: Vec::new(),
        body: Vec::new(),
    });

    loop {
        let payload = match request_rx.recv() {
            Ok(p) => p,
            Err(_) => {
                debug!("Request channel closed — shutting down");
                break;
            }
        };

        // The payload contains: [1 byte cmd] [4 byte len LE] [request bytes]
        if payload.is_empty() {
            break; // empty = shutdown signal
        }

        let cmd = payload[0];
        if cmd == WorkerCmd::Shutdown as u8 {
            info!("Worker received shutdown via channel");
            break;
        }

        if cmd == WorkerCmd::ExecuteNative as u8 && payload.len() > 5 {
            let total_len =
                u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]) as usize;
            let request_data = &payload[5..5 + total_len.min(payload.len() - 5)];

            let req = match NativeRequest::decode(request_data) {
                Some(r) => r,
                None => {
                    response_fn(NativeResponse {
                        success: false,
                        http_status: 500,
                        headers: Vec::new(),
                        body: b"Failed to decode native request".to_vec(),
                    });
                    continue;
                }
            };

            let c_method = req.method;
            let c_uri = req.uri;
            let c_qs = req.query_string;
            let c_ct = req.content_type;
            let c_cookie = req.cookie;
            let c_script = req.script_path;
            let c_docroot = req.document_root;
            let c_addr = req.remote_addr;
            let c_pathinfo = req.path_info;
            let c_scriptname = req.script_name;

            // Headers as raw (ptr, len) — no per-request CString allocation.
            let key_ptrs: Vec<*const std::ffi::c_char> =
                req.headers.iter().map(|(k, _)| k.as_ptr()).collect();
            let key_lens: Vec<usize> = req
                .headers
                .iter()
                .map(|(k, _)| k.to_bytes().len())
                .collect();
            let val_ptrs: Vec<*const std::ffi::c_char> =
                req.headers.iter().map(|(_, v)| v.as_ptr()).collect();
            let val_lens: Vec<usize> = req
                .headers
                .iter()
                .map(|(_, v)| v.to_bytes().len())
                .collect();

            let content_length: libc::c_long = if req.body.is_empty() {
                -1
            } else {
                req.body.len() as libc::c_long
            };

            unsafe {
                turbine_php_sys::turbine_sapi_set_request(
                    c_method.as_ptr(),
                    c_uri.as_ptr(),
                    c_qs.as_ptr(),
                    if req.content_type.to_bytes().is_empty() {
                        std::ptr::null()
                    } else {
                        c_ct.as_ptr()
                    },
                    content_length,
                    if req.cookie.to_bytes().is_empty() {
                        std::ptr::null()
                    } else {
                        c_cookie.as_ptr()
                    },
                    c_script.as_ptr(),
                    c_docroot.as_ptr(),
                    c_addr.as_ptr(),
                    req.remote_port as libc::c_int,
                    req.server_port as libc::c_int,
                    req.is_https as libc::c_int,
                    c_pathinfo.as_ptr(),
                    c_scriptname.as_ptr(),
                    if req.body.is_empty() {
                        std::ptr::null()
                    } else {
                        req.body.as_ptr() as *const _
                    },
                    req.body.len(),
                    req.headers.len() as libc::c_int,
                    if key_ptrs.is_empty() {
                        std::ptr::null()
                    } else {
                        key_ptrs.as_ptr()
                    },
                    if key_lens.is_empty() {
                        std::ptr::null()
                    } else {
                        key_lens.as_ptr()
                    },
                    if val_ptrs.is_empty() {
                        std::ptr::null()
                    } else {
                        val_ptrs.as_ptr()
                    },
                    if val_lens.is_empty() {
                        std::ptr::null()
                    } else {
                        val_lens.as_ptr()
                    },
                );

                turbine_php_sys::php_request_startup();
                output::install_output_capture();
                output::clear_output_buffer();

                let result = turbine_php_sys::turbine_execute_script(c_script.as_ptr());

                // Full request shutdown FIRST — flushes PHP's internal output
                // buffer (output_buffering, default 4096) into our ub_write
                // callback via php_output_end_all(). Collecting before shutdown
                // truncates any response larger than output_buffering (empty /
                // partial bodies under load). Mirrors the invariant documented
                // in the persistent-worker path.
                turbine_php_sys::php_request_shutdown(std::ptr::null_mut());

                // Now collect the full output.
                let body = output::take_output();
                let headers = output::take_headers();
                let status = output::take_response_code();

                response_fn(NativeResponse {
                    success: result == turbine_php_sys::SUCCESS,
                    http_status: status,
                    headers,
                    body,
                });
            }
        }
    }

    debug!("Channel-based native worker event loop exiting");
}

/// Binary request data for the native SAPI worker.
struct NativeRequest<'a> {
    script_path: &'a std::ffi::CStr,
    method: &'a std::ffi::CStr,
    uri: &'a std::ffi::CStr,
    query_string: &'a std::ffi::CStr,
    content_type: &'a std::ffi::CStr,
    cookie: &'a std::ffi::CStr,
    document_root: &'a std::ffi::CStr,
    remote_addr: &'a std::ffi::CStr,
    remote_port: u16,
    server_port: u16,
    is_https: bool,
    path_info: &'a std::ffi::CStr,
    script_name: &'a std::ffi::CStr,
    body: &'a [u8],
    headers: Vec<(&'a std::ffi::CStr, &'a std::ffi::CStr)>,
}

impl<'a> NativeRequest<'a> {
    /// Decode from binary payload (after cmd byte + total_len).
    fn decode(data: &'a [u8]) -> Option<Self> {
        let mut pos = 0;

        let read_cstr = |data: &'a [u8], pos: &mut usize| -> Option<&'a std::ffi::CStr> {
            if *pos + 2 > data.len() {
                return None;
            }
            let len = u16::from_le_bytes([data[*pos], data[*pos + 1]]) as usize;
            *pos += 2;
            if *pos + len + 1 > data.len() {
                return None;
            }
            let slice = &data[*pos..*pos + len + 1];
            *pos += len + 1;
            std::ffi::CStr::from_bytes_with_nul(slice).ok()
        };

        let script_path = read_cstr(data, &mut pos)?;
        let method = read_cstr(data, &mut pos)?;
        let uri = read_cstr(data, &mut pos)?;
        let query_string = read_cstr(data, &mut pos)?;
        let content_type = read_cstr(data, &mut pos)?;

        if pos + 4 > data.len() {
            return None;
        }
        let _content_length =
            i32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        pos += 4;

        let cookie = read_cstr(data, &mut pos)?;
        let document_root = read_cstr(data, &mut pos)?;
        let remote_addr = read_cstr(data, &mut pos)?;

        if pos + 2 > data.len() {
            return None;
        }
        let remote_port = u16::from_le_bytes([data[pos], data[pos + 1]]);
        pos += 2;

        if pos + 2 > data.len() {
            return None;
        }
        let server_port = u16::from_le_bytes([data[pos], data[pos + 1]]);
        pos += 2;

        if pos + 1 > data.len() {
            return None;
        }
        let is_https = data[pos] != 0;
        pos += 1;

        let path_info = read_cstr(data, &mut pos)?;
        let script_name = read_cstr(data, &mut pos)?;

        // Body
        if pos + 4 > data.len() {
            return None;
        }
        let body_len =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + body_len > data.len() {
            return None;
        }
        let body = &data[pos..pos + body_len];
        pos += body_len;

        // Headers
        if pos + 2 > data.len() {
            return None;
        }
        let header_count = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;

        let mut headers = Vec::with_capacity(header_count);
        for _ in 0..header_count {
            let k = read_cstr(data, &mut pos)?;
            let v = read_cstr(data, &mut pos)?;
            headers.push((k, v));
        }

        Some(NativeRequest {
            script_path,
            method,
            uri,
            query_string,
            content_type,
            cookie,
            document_root,
            remote_addr,
            remote_port,
            server_port,
            is_https,
            path_info,
            script_name,
            body,
            headers,
        })
    }
}

/// Encode a native request into the given buffer (cleared first).
/// Hot-path variant: reuses caller-provided `Vec<u8>` to avoid allocations.
#[allow(clippy::too_many_arguments)]
pub fn encode_native_request_into(
    msg: &mut Vec<u8>,
    script_path: &str,
    method: &str,
    uri: &str,
    query_string: &str,
    content_type: &str,
    content_length: i32,
    cookie: &str,
    document_root: &str,
    remote_addr: &str,
    remote_port: u16,
    server_port: u16,
    is_https: bool,
    path_info: &str,
    script_name: &str,
    body: &[u8],
    headers: &[(&str, &str)],
) {
    msg.clear();

    // Reserve: cmd(1) + total_len(4) + conservative payload estimate
    let est = 1
        + 4
        + 256
        + body.len()
        + script_path.len()
        + uri.len()
        + query_string.len()
        + document_root.len()
        + remote_addr.len()
        + headers
            .iter()
            .map(|(k, v)| k.len() + v.len() + 6)
            .sum::<usize>();
    msg.reserve(est);

    // Prepend cmd byte and placeholder for total length; we'll patch length at the end.
    msg.push(WorkerCmd::ExecuteNative as u8);
    msg.extend_from_slice(&[0u8; 4]);
    let payload_start = msg.len();

    let write_str = |buf: &mut Vec<u8>, s: &str| {
        buf.extend_from_slice(&(s.len() as u16).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
    };

    write_str(msg, script_path);
    write_str(msg, method);
    write_str(msg, uri);
    write_str(msg, query_string);
    write_str(msg, content_type);
    msg.extend_from_slice(&content_length.to_le_bytes());
    write_str(msg, cookie);
    write_str(msg, document_root);
    write_str(msg, remote_addr);
    msg.extend_from_slice(&remote_port.to_le_bytes());
    msg.extend_from_slice(&server_port.to_le_bytes());
    msg.push(is_https as u8);
    write_str(msg, path_info);
    write_str(msg, script_name);

    // Body
    msg.extend_from_slice(&(body.len() as u32).to_le_bytes());
    msg.extend_from_slice(body);

    // Headers
    msg.extend_from_slice(&(headers.len() as u16).to_le_bytes());
    for (k, v) in headers {
        write_str(msg, k);
        write_str(msg, v);
    }

    // Patch total payload length
    let payload_len = (msg.len() - payload_start) as u32;
    msg[1..5].copy_from_slice(&payload_len.to_le_bytes());
}

/// Encode a native request for the binary protocol (allocating).
/// Kept for tests and non-hot-path callers.
#[allow(clippy::too_many_arguments)]
pub fn encode_native_request(
    script_path: &str,
    method: &str,
    uri: &str,
    query_string: &str,
    content_type: &str,
    content_length: i32,
    cookie: &str,
    document_root: &str,
    remote_addr: &str,
    remote_port: u16,
    server_port: u16,
    is_https: bool,
    path_info: &str,
    script_name: &str,
    body: &[u8],
    headers: &[(&str, &str)],
) -> Vec<u8> {
    let mut msg = Vec::with_capacity(1024);
    encode_native_request_into(
        &mut msg,
        script_path,
        method,
        uri,
        query_string,
        content_type,
        content_length,
        cookie,
        document_root,
        remote_addr,
        remote_port,
        server_port,
        is_https,
        path_info,
        script_name,
        body,
        headers,
    );
    msg
}

/// Write a structured native response:
///   [1 byte status][2 byte http_status LE][2 byte header_count LE]
///   for each header: [2 byte key_len LE][key][2 byte val_len LE][val]
///   [4 byte body_len LE][body]
fn write_native_response(
    writer: &mut std::fs::File,
    status: WorkerResp,
    http_status: u16,
    headers: &[(String, String)],
    body: &[u8],
) -> std::io::Result<()> {
    let cap = 128 + headers.len() * 64 + if body.len() <= 8192 { body.len() } else { 0 };
    let mut buf = Vec::with_capacity(cap);
    buf.push(status as u8);
    buf.extend_from_slice(&http_status.to_le_bytes());
    buf.extend_from_slice(&(headers.len() as u16).to_le_bytes());
    for (k, v) in headers {
        buf.extend_from_slice(&(k.len() as u16).to_le_bytes());
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(&(v.len() as u16).to_le_bytes());
        buf.extend_from_slice(v.as_bytes());
    }
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    if body.is_empty() {
        writer.write_all(&buf)?;
    } else if body.len() <= 8192 {
        buf.extend_from_slice(body);
        writer.write_all(&buf)?;
    } else {
        writer.write_all(&buf)?;
        writer.write_all(body)?;
    }
    writer.flush()
}

/// Decoded response from a native SAPI worker.
pub struct NativeResponse {
    pub success: bool,
    pub http_status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Read a native response from a worker pipe.
pub fn read_native_response_from_fd(resp_fd: RawFd) -> std::io::Result<NativeResponse> {
    struct RawReader(RawFd);
    impl Read for RawReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                let ret = unsafe { libc::read(self.0, buf.as_mut_ptr() as *mut _, buf.len()) };
                if ret < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(err);
                }
                return Ok(ret as usize);
            }
        }
    }

    let mut r = std::io::BufReader::with_capacity(8192, RawReader(resp_fd));

    let mut status_buf = [0u8; 1];
    r.read_exact(&mut status_buf)?;
    let success = status_buf[0] == WorkerResp::Ok as u8 || status_buf[0] == WorkerResp::Ready as u8;

    let mut http_buf = [0u8; 2];
    r.read_exact(&mut http_buf)?;
    let http_status = u16::from_le_bytes(http_buf);

    let mut hcount_buf = [0u8; 2];
    r.read_exact(&mut hcount_buf)?;
    let header_count = u16::from_le_bytes(hcount_buf) as usize;

    let mut headers = Vec::with_capacity(header_count);
    for _ in 0..header_count {
        let mut kl = [0u8; 2];
        r.read_exact(&mut kl)?;
        let key_len = u16::from_le_bytes(kl) as usize;
        let mut key = vec![0u8; key_len];
        if key_len > 0 {
            r.read_exact(&mut key)?;
        }

        let mut vl = [0u8; 2];
        r.read_exact(&mut vl)?;
        let val_len = u16::from_le_bytes(vl) as usize;
        let mut val = vec![0u8; val_len];
        if val_len > 0 {
            r.read_exact(&mut val)?;
        }

        headers.push((
            String::from_utf8_lossy(&key).into_owned(),
            String::from_utf8_lossy(&val).into_owned(),
        ));
    }

    let mut blen = [0u8; 4];
    r.read_exact(&mut blen)?;
    let body_len = u32::from_le_bytes(blen) as usize;
    let mut body = vec![0u8; body_len];
    if body_len > 0 {
        r.read_exact(&mut body)?;
    }

    Ok(NativeResponse {
        success,
        http_status,
        headers,
        body,
    })
}

/// Async variant of `read_native_response_from_fd` — reads from an
/// `AsyncRead` source so the caller can await the pipe without spending
/// a `spawn_blocking` thread per in-flight request.
pub async fn read_native_response_async<R>(r: &mut R) -> std::io::Result<NativeResponse>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let mut r = tokio::io::BufReader::with_capacity(8192, r);

    let mut status_buf = [0u8; 1];
    r.read_exact(&mut status_buf).await?;
    let success = status_buf[0] == WorkerResp::Ok as u8 || status_buf[0] == WorkerResp::Ready as u8;

    let mut http_buf = [0u8; 2];
    r.read_exact(&mut http_buf).await?;
    let http_status = u16::from_le_bytes(http_buf);

    let mut hcount_buf = [0u8; 2];
    r.read_exact(&mut hcount_buf).await?;
    let header_count = u16::from_le_bytes(hcount_buf) as usize;

    let mut headers = Vec::with_capacity(header_count);
    for _ in 0..header_count {
        let mut kl = [0u8; 2];
        r.read_exact(&mut kl).await?;
        let key_len = u16::from_le_bytes(kl) as usize;
        let mut key = vec![0u8; key_len];
        if key_len > 0 {
            r.read_exact(&mut key).await?;
        }

        let mut vl = [0u8; 2];
        r.read_exact(&mut vl).await?;
        let val_len = u16::from_le_bytes(vl) as usize;
        let mut val = vec![0u8; val_len];
        if val_len > 0 {
            r.read_exact(&mut val).await?;
        }

        headers.push((
            String::from_utf8_lossy(&key).into_owned(),
            String::from_utf8_lossy(&val).into_owned(),
        ));
    }

    let mut blen = [0u8; 4];
    r.read_exact(&mut blen).await?;
    let body_len = u32::from_le_bytes(blen) as usize;
    let mut body = vec![0u8; body_len];
    if body_len > 0 {
        r.read_exact(&mut body).await?;
    }

    Ok(NativeResponse {
        success,
        http_status,
        headers,
        body,
    })
}

/// Async write-all to a non-blocking fd registered with the tokio reactor.
///
/// Callers must have flipped the fd to non-blocking (via
/// `async_io::set_nonblocking`) before calling this; otherwise writes will
/// park the tokio worker thread on a short kernel buffer.
pub async fn write_to_fd_async(
    pipe: &mut crate::async_io::AsyncPipe,
    data: &[u8],
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    pipe.write_all(data).await
}

/// Read a worker response directly from a raw resp_fd WITHOUT holding any lock.
///
/// This allows the caller to release the `WorkerPool` mutex before blocking
/// on the pipe read, enabling true concurrent execution across multiple workers.
///
/// Protocol: [1 byte status][4 byte payload_len LE][payload bytes]
pub fn read_response_from_fd(
    resp_fd: std::os::unix::io::RawFd,
) -> std::io::Result<(bool, Vec<u8>)> {
    use std::io::Read;

    struct RawReader(std::os::unix::io::RawFd);
    impl Read for RawReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                let ret = unsafe { libc::read(self.0, buf.as_mut_ptr() as *mut _, buf.len()) };
                if ret < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(err);
                }
                return Ok(ret as usize);
            }
        }
    }

    let mut r = std::io::BufReader::with_capacity(8192, RawReader(resp_fd));

    let mut status_buf = [0u8; 1];
    r.read_exact(&mut status_buf)?;

    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let payload_len = u32::from_le_bytes(len_buf) as usize;

    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        r.read_exact(&mut payload)?;
    }

    let success = status_buf[0] == WorkerResp::Ok as u8 || status_buf[0] == WorkerResp::Ready as u8;

    Ok((success, payload))
}

/// Write a response to the master: [1 byte status][4 byte len LE][payload].
fn write_response(
    writer: &mut std::fs::File,
    status: WorkerResp,
    payload: &[u8],
) -> std::io::Result<()> {
    let cap = 5 + if payload.len() <= 8192 {
        payload.len()
    } else {
        0
    };
    let mut buf = Vec::with_capacity(cap);
    buf.push(status as u8);
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    if payload.len() <= 8192 {
        buf.extend_from_slice(payload);
        writer.write_all(&buf)?;
    } else {
        writer.write_all(&buf)?;
        writer.write_all(payload)?;
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── safe_cstring ────────────────────────────────────────────────

    #[test]
    fn safe_cstring_normal_string() {
        let c = safe_cstring(b"hello world");
        assert_eq!(c.to_str().unwrap(), "hello world");
    }

    #[test]
    fn safe_cstring_strips_null_bytes() {
        let c = safe_cstring(b"hel\0lo\0");
        assert_eq!(c.to_str().unwrap(), "hello");
    }

    #[test]
    fn safe_cstring_empty() {
        let c = safe_cstring(b"");
        assert_eq!(c.to_str().unwrap(), "");
    }

    #[test]
    fn safe_cstring_only_nulls() {
        let c = safe_cstring(b"\0\0\0");
        assert_eq!(c.to_str().unwrap(), "");
    }

    #[test]
    fn safe_cstring_utf8() {
        let c = safe_cstring("café".as_bytes());
        assert_eq!(c.to_str().unwrap(), "café");
    }

    #[test]
    fn safe_cstring_with_embedded_nulls_preserves_rest() {
        let c = safe_cstring(b"/var/www\0/index.php");
        assert_eq!(c.to_str().unwrap(), "/var/www/index.php");
    }

    // ── WorkerMode ──────────────────────────────────────────────────

    #[test]
    fn worker_mode_from_str_process() {
        assert_eq!(WorkerMode::from_str("process"), WorkerMode::Process);
        assert_eq!(WorkerMode::from_str("Process"), WorkerMode::Process);
        assert_eq!(WorkerMode::from_str("PROCESS"), WorkerMode::Process);
    }

    #[test]
    fn worker_mode_from_str_thread() {
        assert_eq!(WorkerMode::from_str("thread"), WorkerMode::Thread);
        assert_eq!(WorkerMode::from_str("Thread"), WorkerMode::Thread);
        assert_eq!(WorkerMode::from_str("THREAD"), WorkerMode::Thread);
        assert_eq!(WorkerMode::from_str("threads"), WorkerMode::Thread);
    }

    #[test]
    fn worker_mode_unknown_defaults_to_process() {
        assert_eq!(WorkerMode::from_str(""), WorkerMode::Process);
        assert_eq!(WorkerMode::from_str("fork"), WorkerMode::Process);
        assert_eq!(WorkerMode::from_str("unknown"), WorkerMode::Process);
    }

    #[test]
    fn worker_mode_display() {
        assert_eq!(format!("{}", WorkerMode::Process), "process");
        assert_eq!(format!("{}", WorkerMode::Thread), "thread");
    }

    // ── PoolConfig ──────────────────────────────────────────────────

    #[test]
    fn pool_config_default() {
        let config = PoolConfig::default();
        assert!(config.workers > 0);
        assert_eq!(config.max_requests, 10_000);
        assert_eq!(config.mode, WorkerMode::Process);
    }

    #[test]
    fn pool_config_custom() {
        let config = PoolConfig {
            workers: 16,
            max_requests: 50_000,
            mode: WorkerMode::Thread,
            pin_workers: false,
        };
        assert_eq!(config.workers, 16);
        assert_eq!(config.max_requests, 50_000);
        assert_eq!(config.mode, WorkerMode::Thread);
    }

    // ── WorkerPool ──────────────────────────────────────────────────

    #[test]
    fn worker_pool_new() {
        let pool = WorkerPool::new(PoolConfig {
            workers: 4,
            max_requests: 1000,
            mode: WorkerMode::Process,
            pin_workers: false,
        });
        assert_eq!(pool.config().workers, 4);
        assert_eq!(pool.config().max_requests, 1000);
        assert_eq!(pool.worker_count(), 0); // no workers spawned yet
    }

    // ── WorkerCmd/WorkerResp enum values ────────────────────────────

    #[test]
    fn worker_cmd_values() {
        assert_eq!(WorkerCmd::Execute as u8, 1);
        assert_eq!(WorkerCmd::Shutdown as u8, 2);
        assert_eq!(WorkerCmd::ExecuteNative as u8, 3);
    }

    #[test]
    fn worker_resp_values() {
        assert_eq!(WorkerResp::Ok as u8, 1);
        assert_eq!(WorkerResp::Error as u8, 2);
        assert_eq!(WorkerResp::Ready as u8, 3);
    }

    // ── NativeRequest encode / decode round-trip ────────────────────

    #[test]
    fn native_request_roundtrip_basic() {
        let encoded = encode_native_request(
            "/var/www/index.php",
            "GET",
            "/",
            "",
            "",
            -1,
            "",
            "/var/www",
            "127.0.0.1",
            0,
            8080,
            false,
            "/",
            "/index.php",
            &[],
            &[],
        );

        // Skip cmd byte (1) + length (4)
        assert_eq!(encoded[0], WorkerCmd::ExecuteNative as u8);
        let total_len =
            u32::from_le_bytes([encoded[1], encoded[2], encoded[3], encoded[4]]) as usize;
        let payload = &encoded[5..5 + total_len];

        let decoded = NativeRequest::decode(payload).expect("decode failed");
        assert_eq!(decoded.script_path.to_str().unwrap(), "/var/www/index.php");
        assert_eq!(decoded.method.to_str().unwrap(), "GET");
        assert_eq!(decoded.uri.to_str().unwrap(), "/");
        assert_eq!(decoded.query_string.to_str().unwrap(), "");
        assert_eq!(decoded.document_root.to_str().unwrap(), "/var/www");
        assert_eq!(decoded.remote_addr.to_str().unwrap(), "127.0.0.1");
        assert_eq!(decoded.server_port, 8080);
        assert!(!decoded.is_https);
        assert_eq!(decoded.path_info.to_str().unwrap(), "/");
        assert_eq!(decoded.script_name.to_str().unwrap(), "/index.php");
        assert!(decoded.body.is_empty());
        assert!(decoded.headers.is_empty());
    }

    #[test]
    fn native_request_roundtrip_full() {
        let body = b"name=test&value=hello";
        let headers = [
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Host", "example.com"),
            ("Accept", "text/html"),
        ];
        let encoded = encode_native_request(
            "/app/public/index.php",
            "POST",
            "/api/submit?debug=1",
            "debug=1",
            "application/x-www-form-urlencoded",
            body.len() as i32,
            "session=abc123; lang=en",
            "/app/public",
            "10.0.0.1",
            54321,
            443,
            true,
            "/api/submit",
            "/index.php",
            body,
            &headers,
        );

        let total_len =
            u32::from_le_bytes([encoded[1], encoded[2], encoded[3], encoded[4]]) as usize;
        let payload = &encoded[5..5 + total_len];

        let decoded = NativeRequest::decode(payload).expect("decode failed");
        assert_eq!(
            decoded.script_path.to_str().unwrap(),
            "/app/public/index.php"
        );
        assert_eq!(decoded.method.to_str().unwrap(), "POST");
        assert_eq!(decoded.uri.to_str().unwrap(), "/api/submit?debug=1");
        assert_eq!(decoded.query_string.to_str().unwrap(), "debug=1");
        assert_eq!(
            decoded.content_type.to_str().unwrap(),
            "application/x-www-form-urlencoded"
        );
        assert_eq!(decoded.cookie.to_str().unwrap(), "session=abc123; lang=en");
        assert_eq!(decoded.document_root.to_str().unwrap(), "/app/public");
        assert_eq!(decoded.remote_addr.to_str().unwrap(), "10.0.0.1");
        assert_eq!(decoded.remote_port, 54321);
        assert_eq!(decoded.server_port, 443);
        assert!(decoded.is_https);
        assert_eq!(decoded.path_info.to_str().unwrap(), "/api/submit");
        assert_eq!(decoded.script_name.to_str().unwrap(), "/index.php");
        assert_eq!(decoded.body, body);
        assert_eq!(decoded.headers.len(), 3);
        assert_eq!(decoded.headers[0].0.to_str().unwrap(), "Content-Type");
        assert_eq!(
            decoded.headers[0].1.to_str().unwrap(),
            "application/x-www-form-urlencoded"
        );
        assert_eq!(decoded.headers[1].0.to_str().unwrap(), "Host");
        assert_eq!(decoded.headers[1].1.to_str().unwrap(), "example.com");
        assert_eq!(decoded.headers[2].0.to_str().unwrap(), "Accept");
        assert_eq!(decoded.headers[2].1.to_str().unwrap(), "text/html");
    }

    #[test]
    fn native_request_roundtrip_binary_body() {
        let body: Vec<u8> = (0..=255).collect();
        let encoded = encode_native_request(
            "/upload.php",
            "PUT",
            "/upload",
            "",
            "application/octet-stream",
            body.len() as i32,
            "",
            "/",
            "::1",
            0,
            80,
            false,
            "/upload",
            "/upload.php",
            &body,
            &[],
        );

        let total_len =
            u32::from_le_bytes([encoded[1], encoded[2], encoded[3], encoded[4]]) as usize;
        let payload = &encoded[5..5 + total_len];

        let decoded = NativeRequest::decode(payload).expect("decode failed");
        assert_eq!(decoded.body.len(), 256);
        assert_eq!(decoded.body[0], 0);
        assert_eq!(decoded.body[255], 255);
    }

    #[test]
    fn native_request_decode_truncated_returns_none() {
        // Truncated data should return None
        let result = NativeRequest::decode(&[0x00, 0x01]);
        assert!(result.is_none());
    }

    #[test]
    fn native_request_decode_empty_returns_none() {
        let result = NativeRequest::decode(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn native_request_roundtrip_many_headers() {
        let headers: Vec<(&str, &str)> = (0..20)
            .map(|i| {
                // Leak strings for stable references — ok in tests
                let k: &str = Box::leak(format!("X-Header-{i}").into_boxed_str());
                let v: &str = Box::leak(format!("value-{i}").into_boxed_str());
                (k, v)
            })
            .collect();

        let encoded = encode_native_request(
            "/test.php",
            "GET",
            "/",
            "",
            "",
            -1,
            "",
            "/",
            "127.0.0.1",
            0,
            80,
            false,
            "/",
            "/test.php",
            &[],
            &headers,
        );

        let total_len =
            u32::from_le_bytes([encoded[1], encoded[2], encoded[3], encoded[4]]) as usize;
        let payload = &encoded[5..5 + total_len];

        let decoded = NativeRequest::decode(payload).expect("decode failed");
        assert_eq!(decoded.headers.len(), 20);
        assert_eq!(decoded.headers[0].0.to_str().unwrap(), "X-Header-0");
        assert_eq!(decoded.headers[19].0.to_str().unwrap(), "X-Header-19");
    }

    // ── NativeResponse via pipes ────────────────────────────────────

    #[test]
    fn native_response_roundtrip_via_pipe() {
        let mut pipe = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let (resp_read, resp_write) = (pipe[0], pipe[1]);

        // Write a structured native response
        let mut writer = unsafe { std::fs::File::from_raw_fd(resp_write) };
        let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        write_native_response(&mut writer, WorkerResp::Ok, 200, &headers, b"{\"ok\":true}")
            .expect("write failed");
        // Don't close — from_raw_fd owns it
        std::mem::forget(writer);

        let resp = read_native_response_from_fd(resp_read).expect("read failed");
        assert!(resp.success);
        assert_eq!(resp.http_status, 200);
        assert_eq!(resp.headers.len(), 1);
        assert_eq!(resp.headers[0].0, "Content-Type");
        assert_eq!(resp.body, b"{\"ok\":true}");

        unsafe {
            libc::close(resp_read);
            libc::close(resp_write);
        }
    }

    #[test]
    fn native_response_error_via_pipe() {
        let mut pipe = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let (resp_read, resp_write) = (pipe[0], pipe[1]);

        let mut writer = unsafe { std::fs::File::from_raw_fd(resp_write) };
        write_native_response(&mut writer, WorkerResp::Error, 500, &[], b"Fatal error")
            .expect("write failed");
        std::mem::forget(writer);

        let resp = read_native_response_from_fd(resp_read).expect("read failed");
        assert!(!resp.success);
        assert_eq!(resp.http_status, 500);
        assert!(resp.headers.is_empty());
        assert_eq!(resp.body, b"Fatal error");

        unsafe {
            libc::close(resp_read);
            libc::close(resp_write);
        }
    }
}
