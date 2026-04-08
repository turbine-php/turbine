use std::io::{Read, Write};
use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nix::sys::signal::{self, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use tracing::{debug, info, warn};

use crate::pool::WorkerResp;
use crate::WorkerError;

/// A wrapper around a RawFd that implements Read/Write but does NOT close
/// the fd on drop. Used for the cmd/resp pipes which are owned by Worker.
struct ManualFd {
    fd: RawFd,
}

impl ManualFd {
    unsafe fn new(fd: RawFd) -> Self {
        ManualFd { fd }
    }
}

impl Read for ManualFd {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let ret = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if ret < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(ret as usize)
        }
    }
}

impl Write for ManualFd {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let ret = unsafe { libc::write(self.fd, buf.as_ptr() as *const _, buf.len()) };
        if ret < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(ret as usize)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// State of a worker process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState {
    /// Worker is idle, waiting for a request.
    Idle,
    /// Worker is currently processing a request.
    Busy,
    /// Worker is shutting down.
    Stopping,
    /// Worker has exited.
    Exited(i32),
}

/// The backend kind for a worker — either a forked process or an OS thread.
#[derive(Debug)]
pub enum WorkerKind {
    /// Fork-based worker (default). Uses waitpid/signals for lifecycle.
    Process(Pid),
    /// Thread-based worker (ZTS required). Uses atomic flag for liveness.
    Thread {
        /// Shared flag — the thread sets this to `false` before exiting.
        alive: Arc<AtomicBool>,
        /// Thread ID for logging purposes.
        thread_id: u64,
    },
}

/// A single worker — either a forked process or an OS thread.
///
/// Both kinds use the same pipe-based IPC protocol (cmd_fd/resp_fd).
/// The difference is lifecycle management: processes use signals (SIGTERM/SIGKILL),
/// threads use the shutdown command via the pipe.
pub struct Worker {
    /// Worker backend kind (process with PID or thread with liveness flag).
    kind: WorkerKind,
    /// Current state.
    state: WorkerState,
    /// Number of requests handled.
    requests_handled: u64,
    /// Maximum requests before recycling (0 = unlimited).
    max_requests: u64,
    /// Pipe fd for master → worker communication.
    cmd_fd: RawFd,
    /// Pipe fd for worker → master communication.
    resp_fd: RawFd,
}

impl Worker {
    /// Create a Worker for a freshly forked child process.
    pub(crate) fn new(pid: Pid, max_requests: u64, cmd_fd: RawFd, resp_fd: RawFd) -> Self {
        info!(pid = pid.as_raw(), "Worker process created");
        Worker {
            kind: WorkerKind::Process(pid),
            state: WorkerState::Idle,
            requests_handled: 0,
            max_requests,
            cmd_fd,
            resp_fd,
        }
    }

    /// Create a Worker backed by an OS thread (ZTS mode).
    pub(crate) fn new_thread(alive: Arc<AtomicBool>, thread_id: u64, max_requests: u64, cmd_fd: RawFd, resp_fd: RawFd) -> Self {
        info!(thread_id = thread_id, "Worker thread created");
        Worker {
            kind: WorkerKind::Thread { alive, thread_id },
            state: WorkerState::Idle,
            requests_handled: 0,
            max_requests,
            cmd_fd,
            resp_fd,
        }
    }

    /// Get the worker's PID (process mode) or 0 (thread mode).
    pub fn pid(&self) -> Pid {
        match &self.kind {
            WorkerKind::Process(pid) => *pid,
            WorkerKind::Thread { .. } => Pid::from_raw(0),
        }
    }

    /// Get the worker kind.
    pub fn kind(&self) -> &WorkerKind {
        &self.kind
    }

    /// Whether this is a thread-based worker.
    pub fn is_thread(&self) -> bool {
        matches!(self.kind, WorkerKind::Thread { .. })
    }

    /// Get the worker's current state.
    pub fn state(&self) -> WorkerState {
        self.state
    }

    /// Get the number of requests this worker has handled.
    pub fn requests_handled(&self) -> u64 {
        self.requests_handled
    }

    /// Mark the worker as busy (processing a request).
    pub fn mark_busy(&mut self) {
        self.state = WorkerState::Busy;
    }

    /// Mark the worker as idle and increment the request counter.
    /// Returns `true` if the worker should be recycled.
    pub fn mark_idle(&mut self) -> bool {
        self.requests_handled += 1;
        self.state = WorkerState::Idle;

        if self.max_requests > 0 && self.requests_handled >= self.max_requests {
            debug!(
                pid = self.pid().as_raw(),
                requests = self.requests_handled,
                max = self.max_requests,
                "Worker reached max requests — scheduling recycle"
            );
            return true;
        }
        false
    }

    /// Check if the worker process/thread is still alive (non-blocking).
    pub fn is_alive(&mut self) -> bool {
        match &self.kind {
            WorkerKind::Process(pid) => {
                match waitpid(*pid, Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::StillAlive) => true,
                    Ok(WaitStatus::Exited(_, code)) => {
                        self.state = WorkerState::Exited(code);
                        false
                    }
                    Ok(WaitStatus::Signaled(_, sig, _)) => {
                        warn!(pid = pid.as_raw(), signal = ?sig, "Worker killed by signal");
                        self.state = WorkerState::Exited(-1);
                        false
                    }
                    Ok(_) => true, // Stopped/Continued
                    Err(_) => {
                        self.state = WorkerState::Exited(-1);
                        false
                    }
                }
            }
            WorkerKind::Thread { alive, thread_id } => {
                if alive.load(Ordering::Acquire) {
                    true
                } else {
                    debug!(thread_id = thread_id, "Worker thread has exited");
                    self.state = WorkerState::Exited(0);
                    false
                }
            }
        }
    }

    /// Send SIGTERM (process) or shutdown command (thread) for graceful stop.
    pub fn terminate(&mut self) -> Result<(), WorkerError> {
        self.state = WorkerState::Stopping;
        match &self.kind {
            WorkerKind::Process(pid) => {
                info!(pid = pid.as_raw(), "Sending SIGTERM to worker");
                signal::kill(*pid, Signal::SIGTERM).map_err(WorkerError::Signal)
            }
            WorkerKind::Thread { thread_id, .. } => {
                info!(thread_id = thread_id, "Sending shutdown to worker thread");
                // Send shutdown command via pipe — the thread's event loop handles it
                let _ = self.send_shutdown();
                Ok(())
            }
        }
    }

    /// Send SIGKILL (process) or close pipe (thread) for forceful stop.
    pub fn kill(&mut self) -> Result<(), WorkerError> {
        self.state = WorkerState::Stopping;
        match &self.kind {
            WorkerKind::Process(pid) => {
                warn!(pid = pid.as_raw(), "Sending SIGKILL to worker");
                signal::kill(*pid, Signal::SIGKILL).map_err(WorkerError::Signal)
            }
            WorkerKind::Thread { thread_id, .. } => {
                warn!(thread_id = thread_id, "Force-closing worker thread pipe");
                // Close the command pipe — the thread will see EOF and exit
                unsafe { libc::close(self.cmd_fd); }
                // Set cmd_fd to -1 to prevent double-close in Drop
                self.cmd_fd = -1;
                Ok(())
            }
        }
    }

    /// Get the command pipe fd (master writes, worker reads).
    pub fn cmd_fd(&self) -> RawFd {
        self.cmd_fd
    }

    /// Get the response pipe fd (worker writes, master reads).
    pub fn resp_fd(&self) -> RawFd {
        self.resp_fd
    }

    /// Send a PHP code string to the worker for execution.
    ///
    /// Protocol: [1 byte cmd=Execute] [4 byte code_len LE] [code bytes]
    pub fn send_execute(&self, php_code: &str) -> std::io::Result<()> {
        let mut file = unsafe { ManualFd::new(self.cmd_fd) };
        let code_bytes = php_code.as_bytes();

        file.write_all(&[crate::pool::WorkerCmd::Execute as u8])?;
        file.write_all(&(code_bytes.len() as u32).to_le_bytes())?;
        file.write_all(code_bytes)?;
        file.flush()
    }

    /// Read the response from the worker after an execute command.
    ///
    /// Protocol: [1 byte status] [4 byte payload_len LE] [payload bytes]
    ///
    /// Returns (success: bool, output: Vec<u8>).
    pub fn read_response(&self) -> std::io::Result<(bool, Vec<u8>)> {
        let mut file = unsafe { ManualFd::new(self.resp_fd) };

        // Read status byte
        let mut status_buf = [0u8; 1];
        file.read_exact(&mut status_buf)?;

        // Read payload length
        let mut len_buf = [0u8; 4];
        file.read_exact(&mut len_buf)?;
        let payload_len = u32::from_le_bytes(len_buf) as usize;

        // Read payload
        let mut payload = vec![0u8; payload_len];
        if payload_len > 0 {
            file.read_exact(&mut payload)?;
        }

        let success = status_buf[0] == WorkerResp::Ok as u8
            || status_buf[0] == WorkerResp::Ready as u8;

        Ok((success, payload))
    }

    /// Send a shutdown command to the worker.
    pub fn send_shutdown(&self) -> std::io::Result<()> {
        let mut file = unsafe { ManualFd::new(self.cmd_fd) };
        file.write_all(&[crate::pool::WorkerCmd::Shutdown as u8])?;
        file.flush()
    }

    /// Send a pre-encoded binary request to a persistent worker.
    ///
    /// Used with the persistent binary protocol — the payload is built by
    /// `turbine_worker::persistent::encode_request()`.
    pub fn send_request(&self, data: &[u8]) -> std::io::Result<()> {
        let mut file = unsafe { ManualFd::new(self.cmd_fd) };
        file.write_all(data)?;
        file.flush()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.state != WorkerState::Exited(-1) && self.state != WorkerState::Exited(0) {
            // Close pipe fds (skip if already closed, i.e. fd == -1)
            unsafe {
                if self.cmd_fd >= 0 { libc::close(self.cmd_fd); }
                if self.resp_fd >= 0 { libc::close(self.resp_fd); }
            }
            // Try graceful then force
            if let WorkerState::Idle | WorkerState::Busy = self.state {
                let _ = self.terminate();
            }
        }
    }
}
