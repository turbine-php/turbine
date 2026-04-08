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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn make_pipe() -> (i32, i32) {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    // ── Worker State Machine ────────────────────────────────────────

    #[test]
    fn worker_starts_idle() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);
        assert_eq!(worker.state(), WorkerState::Idle);
        assert_eq!(worker.requests_handled(), 0);
        // Clean up fds
        unsafe { libc::close(cmd_r); libc::close(resp_w); }
        // Drop will try to close cmd_w and resp_r
    }

    #[test]
    fn worker_mark_busy_changes_state() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let mut worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);
        worker.mark_busy();
        assert_eq!(worker.state(), WorkerState::Busy);
        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn worker_mark_idle_increments_counter() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let mut worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);

        worker.mark_busy();
        let should_recycle = worker.mark_idle();
        assert!(!should_recycle);
        assert_eq!(worker.requests_handled(), 1);
        assert_eq!(worker.state(), WorkerState::Idle);

        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn worker_recycle_at_max_requests() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let mut worker = Worker::new(nix::unistd::Pid::from_raw(99999), 3, cmd_w, resp_r);

        assert!(!worker.mark_idle()); // 1
        assert!(!worker.mark_idle()); // 2
        assert!(worker.mark_idle());  // 3 -- should recycle

        assert_eq!(worker.requests_handled(), 3);
        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn worker_no_recycle_when_max_is_zero() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let mut worker = Worker::new(nix::unistd::Pid::from_raw(99999), 0, cmd_w, resp_r);

        for _ in 0..100 {
            assert!(!worker.mark_idle());
        }
        assert_eq!(worker.requests_handled(), 100);
        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    // ── WorkerKind ──────────────────────────────────────────────────

    #[test]
    fn worker_process_kind() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let worker = Worker::new(nix::unistd::Pid::from_raw(12345), 100, cmd_w, resp_r);
        assert!(!worker.is_thread());
        assert_eq!(worker.pid().as_raw(), 12345);
        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn worker_thread_kind() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let alive = Arc::new(AtomicBool::new(true));
        let worker = Worker::new_thread(alive.clone(), 42, 100, cmd_w, resp_r);
        assert!(worker.is_thread());
        assert_eq!(worker.pid().as_raw(), 0); // thread has no PID
        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn worker_thread_alive_flag() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let alive = Arc::new(AtomicBool::new(true));
        let mut worker = Worker::new_thread(alive.clone(), 1, 100, cmd_w, resp_r);

        assert!(worker.is_alive());

        // Simulate thread exit
        alive.store(false, Ordering::Release);
        assert!(!worker.is_alive());
        assert_eq!(worker.state(), WorkerState::Exited(0));

        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    // ── Pipe Communication ──────────────────────────────────────────

    #[test]
    fn worker_send_execute_and_read_response() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);

        // Send PHP code
        worker.send_execute("echo 'hello';").unwrap();

        // Verify what was written to the pipe
        let mut cmd_buf = [0u8; 1];
        unsafe {
            assert!(libc::read(cmd_r, cmd_buf.as_mut_ptr() as *mut _, 1) > 0);
        }
        assert_eq!(cmd_buf[0], crate::pool::WorkerCmd::Execute as u8);

        let mut len_buf = [0u8; 4];
        unsafe {
            assert!(libc::read(cmd_r, len_buf.as_mut_ptr() as *mut _, 4) > 0);
        }
        let code_len = u32::from_le_bytes(len_buf) as usize;
        assert_eq!(code_len, "echo 'hello';".len());

        let mut code_buf = vec![0u8; code_len];
        unsafe {
            assert!(libc::read(cmd_r, code_buf.as_mut_ptr() as *mut _, code_len) > 0);
        }
        assert_eq!(&code_buf, "echo 'hello';".as_bytes());

        // Simulate a worker response
        let mut resp = Vec::new();
        resp.push(WorkerResp::Ok as u8);
        let payload = b"hello";
        resp.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        resp.extend_from_slice(payload);
        unsafe {
            libc::write(resp_w, resp.as_ptr() as *const _, resp.len());
        }

        let (success, output) = worker.read_response().unwrap();
        assert!(success);
        assert_eq!(output, b"hello");

        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn worker_send_shutdown() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);

        worker.send_shutdown().unwrap();

        let mut buf = [0u8; 1];
        unsafe {
            assert!(libc::read(cmd_r, buf.as_mut_ptr() as *mut _, 1) > 0);
        }
        assert_eq!(buf[0], crate::pool::WorkerCmd::Shutdown as u8);

        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn worker_send_request_binary() {
        let (cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);

        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        worker.send_request(&data).unwrap();

        let mut read_buf = [0u8; 5];
        unsafe {
            assert_eq!(libc::read(cmd_r, read_buf.as_mut_ptr() as *mut _, 5), 5);
        }
        assert_eq!(&read_buf, &[0x01, 0x02, 0x03, 0x04, 0x05]);

        unsafe { libc::close(cmd_r); libc::close(resp_w); }
    }

    // ── Response status interpretation ──────────────────────────────

    #[test]
    fn read_response_ok_is_success() {
        let (_cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);

        let mut resp = Vec::new();
        resp.push(WorkerResp::Ok as u8);
        resp.extend_from_slice(&0u32.to_le_bytes());
        unsafe { libc::write(resp_w, resp.as_ptr() as *const _, resp.len()); }

        let (success, payload) = worker.read_response().unwrap();
        assert!(success);
        assert!(payload.is_empty());

        unsafe { libc::close(_cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn read_response_ready_is_success() {
        let (_cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);

        let mut resp = Vec::new();
        resp.push(WorkerResp::Ready as u8);
        resp.extend_from_slice(&0u32.to_le_bytes());
        unsafe { libc::write(resp_w, resp.as_ptr() as *const _, resp.len()); }

        let (success, _) = worker.read_response().unwrap();
        assert!(success);

        unsafe { libc::close(_cmd_r); libc::close(resp_w); }
    }

    #[test]
    fn read_response_error_is_failure() {
        let (_cmd_r, cmd_w) = make_pipe();
        let (resp_r, resp_w) = make_pipe();
        let worker = Worker::new(nix::unistd::Pid::from_raw(99999), 100, cmd_w, resp_r);

        let mut resp = Vec::new();
        resp.push(WorkerResp::Error as u8);
        let payload = b"Fatal error";
        resp.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        resp.extend_from_slice(payload);
        unsafe { libc::write(resp_w, resp.as_ptr() as *const _, resp.len()); }

        let (success, output) = worker.read_response().unwrap();
        assert!(!success);
        assert_eq!(output, b"Fatal error");

        unsafe { libc::close(_cmd_r); libc::close(resp_w); }
    }

    // ── WorkerState enum ────────────────────────────────────────────

    #[test]
    fn worker_state_equality() {
        assert_eq!(WorkerState::Idle, WorkerState::Idle);
        assert_eq!(WorkerState::Busy, WorkerState::Busy);
        assert_eq!(WorkerState::Stopping, WorkerState::Stopping);
        assert_eq!(WorkerState::Exited(0), WorkerState::Exited(0));
        assert_ne!(WorkerState::Exited(0), WorkerState::Exited(1));
        assert_ne!(WorkerState::Idle, WorkerState::Busy);
    }
}
