use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("Failed to create shared memory segment: {0}")]
    SharedMemoryCreate(#[source] nix::Error),

    #[error("Failed to seal shared memory with mprotect: {0}")]
    SharedMemorySeal(#[source] nix::Error),

    #[error("Failed to fork worker process: {0}")]
    Fork(#[source] nix::Error),

    #[error("Worker process exited unexpectedly: pid={pid}, status={status}")]
    WorkerExited { pid: i32, status: i32 },

    #[error("Worker pool is full (max={max})")]
    PoolFull { max: usize },

    #[error("Worker {pid} timed out after {timeout_ms}ms")]
    Timeout { pid: i32, timeout_ms: u64 },

    #[error("Pipe creation failed: {0}")]
    Pipe(#[source] nix::Error),

    #[error("Signal error: {0}")]
    Signal(#[source] nix::Error),
}
