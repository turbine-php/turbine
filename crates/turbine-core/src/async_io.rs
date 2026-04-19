//! Async I/O primitives exposed to PHP userland.
//!
//! # What's actually useful
//!
//! PHP is single-threaded per request.  Calling a "non-blocking" helper
//! from PHP still blocks the PHP worker while it waits for the HTTP
//! response.  So there is **no gain** from async-ifying a single
//! `fopen`/`fread` — the PHP worker sits on the curl handle either way.
//!
//! The real wins are:
//!
//! 1. **Batch concurrency.**  Submit N operations in one call and let
//!    Rust run them in parallel via `tokio::join!`.  PHP waits once
//!    for the whole batch; the wall-clock cost is `max(op_i)` instead
//!    of `sum(op_i)`.
//! 2. **Deferred work.**  Schedule a task-queue push to fire after a
//!    delay, without tying up a PHP worker for the duration.  This is
//!    the basis for retry, rate-limit cooldown, and debouncing
//!    patterns that would otherwise need Redis + a cron job.
//!
//! Everything else (single read, single write) is offered as building
//! blocks for the batch executor, not as a "make PHP async" promise.
//!
//! # Security
//!
//! File I/O only succeeds if the resolved, canonicalised target path
//! lives under one of the configured `allowed_roots`.  Symlink
//! traversal out of those roots is rejected, as are `..` segments
//! after canonicalisation.  Path escapes are the #1 footgun for a
//! primitive like this; defaults are deliberately conservative
//! (`allowed_roots = []` + `enabled = false`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::task_queue::TaskQueue;

#[derive(Debug, Error)]
pub enum AsyncIoError {
    #[error("path is outside allowed_roots")]
    PathNotAllowed,
    #[error("payload exceeds max_io_bytes ({0})")]
    TooLarge(usize),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("timer requires [task_queue] enabled")]
    TimerWithoutQueue,
    #[error("invalid delay (max {0} ms)")]
    DelayTooLong(u64),
}

/// Runtime handle backing the async_io HTTP endpoints.
pub struct AsyncIo {
    allowed_roots: Vec<PathBuf>,
    max_io_bytes: usize,
    max_timer_ms: u64,
    timers_scheduled: AtomicU64,
    timers_fired: AtomicU64,
    reads: AtomicU64,
    writes: AtomicU64,
}

impl AsyncIo {
    pub fn new(allowed_roots: Vec<PathBuf>, max_io_bytes: usize, max_timer_ms: u64) -> Self {
        // Canonicalise at startup so per-request checks do one fewer
        // filesystem call and don't drift if the roots change under us.
        let allowed_roots = allowed_roots
            .into_iter()
            .filter_map(|p| std::fs::canonicalize(&p).ok())
            .collect();
        Self {
            allowed_roots,
            max_io_bytes,
            max_timer_ms,
            timers_scheduled: AtomicU64::new(0),
            timers_fired: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            writes: AtomicU64::new(0),
        }
    }

    /// Resolve and verify that `path` is allowed.  Returns a
    /// canonicalised absolute path on success.
    ///
    /// For writes, the target file may not exist yet — in that case we
    /// canonicalise the parent directory and re-join the filename.
    fn resolve(&self, path: &str, allow_create: bool) -> Result<PathBuf, AsyncIoError> {
        if self.allowed_roots.is_empty() {
            return Err(AsyncIoError::PathNotAllowed);
        }
        let p = Path::new(path);
        let canon = if let Ok(c) = std::fs::canonicalize(p) {
            c
        } else if allow_create {
            let parent = p.parent().ok_or(AsyncIoError::PathNotAllowed)?;
            let file_name = p.file_name().ok_or(AsyncIoError::PathNotAllowed)?;
            let parent_canon =
                std::fs::canonicalize(parent).map_err(|_| AsyncIoError::PathNotAllowed)?;
            parent_canon.join(file_name)
        } else {
            return Err(AsyncIoError::PathNotAllowed);
        };
        if self
            .allowed_roots
            .iter()
            .any(|root| canon.starts_with(root))
        {
            Ok(canon)
        } else {
            Err(AsyncIoError::PathNotAllowed)
        }
    }

    /// Read up to `length` bytes from `offset`.  `length == 0` means
    /// "read to EOF" (still capped by `max_io_bytes`).
    pub async fn read(
        &self,
        path: &str,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, AsyncIoError> {
        let target = self.resolve(path, false)?;
        let mut file = fs::File::open(&target).await?;
        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(offset)).await?;
        }
        let cap = if length == 0 {
            self.max_io_bytes
        } else {
            length.min(self.max_io_bytes)
        };
        let mut buf = Vec::with_capacity(cap.min(64 * 1024));
        // `take` so a pathological file-size does not blow memory.
        let n = (&mut file).take(cap as u64).read_to_end(&mut buf).await?;
        buf.truncate(n);
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(buf)
    }

    pub async fn write(
        &self,
        path: &str,
        data: &[u8],
        append: bool,
    ) -> Result<usize, AsyncIoError> {
        if data.len() > self.max_io_bytes {
            return Err(AsyncIoError::TooLarge(self.max_io_bytes));
        }
        let target = self.resolve(path, true)?;
        let mut opts = fs::OpenOptions::new();
        opts.create(true).write(true);
        if append {
            opts.append(true);
        } else {
            opts.truncate(true);
        }
        let mut file = opts.open(&target).await?;
        file.write_all(data).await?;
        file.flush().await?;
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(data.len())
    }

    /// Schedule a push to `channel` after `delay`.  Requires the task
    /// queue to be enabled — callers pass the queue handle explicitly
    /// to avoid a hard dep from async_io on the queue config surface.
    pub fn schedule_timer(
        self: &Arc<Self>,
        queue: Option<Arc<TaskQueue>>,
        channel: String,
        payload: Vec<u8>,
        delay: Duration,
    ) -> Result<(), AsyncIoError> {
        let queue = queue.ok_or(AsyncIoError::TimerWithoutQueue)?;
        let delay_ms = delay.as_millis() as u64;
        if delay_ms > self.max_timer_ms {
            return Err(AsyncIoError::DelayTooLong(self.max_timer_ms));
        }
        self.timers_scheduled.fetch_add(1, Ordering::Relaxed);
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // Best-effort: if the channel is full we drop the timer
            // payload.  Callers who need durability should push
            // immediately and implement their own retry.
            let _ = queue.push(&channel, payload);
            this.timers_fired.fetch_add(1, Ordering::Relaxed);
        });
        Ok(())
    }

    pub fn stats(&self) -> AsyncIoStats {
        AsyncIoStats {
            reads: self.reads.load(Ordering::Relaxed),
            writes: self.writes.load(Ordering::Relaxed),
            timers_scheduled: self.timers_scheduled.load(Ordering::Relaxed),
            timers_fired: self.timers_fired.load(Ordering::Relaxed),
            allowed_roots: self.allowed_roots.len(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AsyncIoStats {
    pub reads: u64,
    pub writes: u64,
    pub timers_scheduled: u64,
    pub timers_fired: u64,
    pub allowed_roots: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make(io_roots: Vec<PathBuf>) -> Arc<AsyncIo> {
        Arc::new(AsyncIo::new(io_roots, 1_048_576, 60_000))
    }

    #[tokio::test]
    async fn read_inside_allowed_root_succeeds() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"hello").unwrap();
        let io = make(vec![dir.path().to_path_buf()]);
        let out = io.read(file.to_str().unwrap(), 0, 0).await.unwrap();
        assert_eq!(out, b"hello");
    }

    #[tokio::test]
    async fn read_outside_allowed_root_fails() {
        let dir = tempdir().unwrap();
        let other = tempdir().unwrap();
        let file = other.path().join("secret");
        std::fs::write(&file, b"no").unwrap();
        let io = make(vec![dir.path().to_path_buf()]);
        let err = io.read(file.to_str().unwrap(), 0, 0).await.unwrap_err();
        assert!(matches!(err, AsyncIoError::PathNotAllowed));
    }

    #[tokio::test]
    async fn read_respects_length_cap() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"hello world").unwrap();
        let io = make(vec![dir.path().to_path_buf()]);
        let out = io.read(file.to_str().unwrap(), 0, 5).await.unwrap();
        assert_eq!(out, b"hello");
    }

    #[tokio::test]
    async fn read_respects_offset() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"hello world").unwrap();
        let io = make(vec![dir.path().to_path_buf()]);
        let out = io.read(file.to_str().unwrap(), 6, 0).await.unwrap();
        assert_eq!(out, b"world");
    }

    #[tokio::test]
    async fn write_truncates_by_default() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        let io = make(vec![dir.path().to_path_buf()]);
        io.write(file.to_str().unwrap(), b"first", false)
            .await
            .unwrap();
        io.write(file.to_str().unwrap(), b"hi", false)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"hi");
    }

    #[tokio::test]
    async fn write_append_extends_file() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        let io = make(vec![dir.path().to_path_buf()]);
        io.write(file.to_str().unwrap(), b"one\n", false)
            .await
            .unwrap();
        io.write(file.to_str().unwrap(), b"two\n", true)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"one\ntwo\n");
    }

    #[tokio::test]
    async fn write_outside_allowed_root_fails() {
        let dir = tempdir().unwrap();
        let other = tempdir().unwrap();
        let io = make(vec![dir.path().to_path_buf()]);
        let err = io
            .write(other.path().join("x").to_str().unwrap(), b"nope", false)
            .await
            .unwrap_err();
        assert!(matches!(err, AsyncIoError::PathNotAllowed));
    }

    #[tokio::test]
    async fn write_rejects_oversize_payload() {
        let dir = tempdir().unwrap();
        let io = Arc::new(AsyncIo::new(vec![dir.path().to_path_buf()], 4, 60_000));
        let err = io
            .write(dir.path().join("big").to_str().unwrap(), b"too big", false)
            .await
            .unwrap_err();
        assert!(matches!(err, AsyncIoError::TooLarge(4)));
    }

    #[tokio::test]
    async fn empty_allowed_roots_rejects_everything() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").unwrap();
        let io = make(vec![]);
        let err = io.read(file.to_str().unwrap(), 0, 0).await.unwrap_err();
        assert!(matches!(err, AsyncIoError::PathNotAllowed));
    }

    #[tokio::test]
    async fn timer_without_queue_fails() {
        let dir = tempdir().unwrap();
        let io = make(vec![dir.path().to_path_buf()]);
        let err = io
            .schedule_timer(
                None,
                "c".to_string(),
                b"p".to_vec(),
                Duration::from_millis(1),
            )
            .unwrap_err();
        assert!(matches!(err, AsyncIoError::TimerWithoutQueue));
    }

    #[tokio::test]
    async fn timer_fires_onto_queue_after_delay() {
        let dir = tempdir().unwrap();
        let io = make(vec![dir.path().to_path_buf()]);
        let q = Arc::new(TaskQueue::new(4, 16));
        io.schedule_timer(
            Some(q.clone()),
            "c".to_string(),
            b"payload".to_vec(),
            Duration::from_millis(50),
        )
        .unwrap();
        // Nothing in queue yet.
        assert_eq!(q.size("c"), 0);
        let job = q.pop("c", Duration::from_millis(500)).await.unwrap();
        assert_eq!(job.payload, b"payload");
    }

    #[tokio::test]
    async fn timer_rejects_delay_over_limit() {
        let dir = tempdir().unwrap();
        let io = Arc::new(AsyncIo::new(vec![dir.path().to_path_buf()], 1024, 100));
        let q = Arc::new(TaskQueue::new(4, 16));
        let err = io
            .schedule_timer(
                Some(q),
                "c".to_string(),
                b"p".to_vec(),
                Duration::from_millis(500),
            )
            .unwrap_err();
        assert!(matches!(err, AsyncIoError::DelayTooLong(100)));
    }
}
