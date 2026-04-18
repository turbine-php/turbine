//! Async I/O wrappers around raw pipe file descriptors.
//!
//! The worker pool uses raw `RawFd` pipes inherited via `fork()` to talk to
//! workers.  Originally all pipe reads/writes were done via blocking
//! `libc::read`/`libc::write` inside `tokio::task::spawn_blocking`, which
//! consumes one thread from the blocking pool (default 512) **for every
//! in-flight request**.  Under high concurrency (more than 512 simultaneous
//! requests) the blocking pool saturates and further requests stall even
//! when workers are idle.
//!
//! This module wraps pipe fds with `tokio::io::unix::AsyncFd` + an
//! `AsyncRead`/`AsyncWrite` adapter so pipe I/O integrates natively with
//! the tokio reactor (epoll on Linux / kqueue on BSD/macOS).  Each request
//! only consumes a cheap reactor registration instead of a whole OS thread.
//!
//! # Usage
//! ```ignore
//! // One-time: flip fd to non-blocking so read/write return EAGAIN instead
//! // of parking the thread.
//! set_nonblocking(fd)?;
//! let mut pipe = AsyncPipe::new(fd)?;
//! let resp = decode_response_async(&mut pipe).await?;
//! ```

use std::io;
use std::os::unix::io::RawFd;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Switch a file descriptor to non-blocking mode via `fcntl`.
///
/// This is a prerequisite for using `AsyncFd`: the reactor relies on
/// `read`/`write` returning `EAGAIN` instead of parking the thread when the
/// pipe is empty/full.  Idempotent.
pub fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd is owned by the caller; fcntl with F_GETFL/F_SETFL is
    // documented as safe to call on any valid fd.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if flags & libc::O_NONBLOCK != 0 {
        return Ok(());
    }
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Async adapter around a non-blocking pipe `RawFd`.
///
/// Does NOT take ownership of the fd (wraps it via `AsyncFd` but the
/// underlying fd lifecycle remains controlled by the `WorkerPool`).  When
/// this struct is dropped, the fd is left open — callers keep using the
/// raw fd for other operations or for the worker's lifetime.
pub struct AsyncPipe {
    inner: AsyncFd<BorrowedFd>,
}

/// Tiny newtype so `AsyncFd::new` (which requires `AsRawFd`) wraps our
/// borrowed fd without transferring ownership.
struct BorrowedFd(RawFd);

impl std::os::unix::io::AsRawFd for BorrowedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl AsyncPipe {
    /// Register `fd` with the tokio reactor.  The fd is automatically
    /// flipped to non-blocking mode on construction (idempotent — safe to
    /// call on an fd that is already non-blocking, or on a fresh blocking
    /// pipe fd).  The fd lifecycle is **not** taken over: the caller is
    /// still responsible for closing it when the worker exits.
    pub fn new(fd: RawFd) -> io::Result<Self> {
        set_nonblocking(fd)?;
        let inner = AsyncFd::new(BorrowedFd(fd))?;
        Ok(Self { inner })
    }
}

impl AsyncRead for AsyncPipe {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut guard = match self.inner.poll_read_ready(cx) {
                Poll::Ready(Ok(g)) => g,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };
            let fd = guard.get_inner().0;
            let dst = buf.initialize_unfilled();
            // SAFETY: libc::read is safe on a valid fd with a writable buffer.
            let n = unsafe { libc::read(fd, dst.as_mut_ptr() as *mut _, dst.len()) };
            if n < 0 {
                let err = io::Error::last_os_error();
                match err.kind() {
                    io::ErrorKind::WouldBlock => {
                        guard.clear_ready();
                        continue;
                    }
                    io::ErrorKind::Interrupted => continue,
                    _ => return Poll::Ready(Err(err)),
                }
            }
            buf.advance(n as usize);
            return Poll::Ready(Ok(()));
        }
    }
}

impl AsyncWrite for AsyncPipe {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = match self.inner.poll_write_ready(cx) {
                Poll::Ready(Ok(g)) => g,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            };
            let fd = guard.get_inner().0;
            // SAFETY: libc::write is safe on a valid fd with a readable buffer.
            let n = unsafe { libc::write(fd, buf.as_ptr() as *const _, buf.len()) };
            if n < 0 {
                let err = io::Error::last_os_error();
                match err.kind() {
                    io::ErrorKind::WouldBlock => {
                        guard.clear_ready();
                        continue;
                    }
                    io::ErrorKind::Interrupted => continue,
                    _ => return Poll::Ready(Err(err)),
                }
            }
            return Poll::Ready(Ok(n as usize));
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Pipes have no user-space buffering we control; kernel flushes on write.
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers mirroring the sync binary-protocol readers, but async.
// ─────────────────────────────────────────────────────────────────────────────

use tokio::io::AsyncReadExt;

/// Read exactly `n` bytes asynchronously.
pub async fn read_exact_async<R: AsyncRead + Unpin>(r: &mut R, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Read a single byte asynchronously.
pub async fn read_u8_async<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b).await?;
    Ok(b[0])
}

/// Read a little-endian u16 asynchronously.
pub async fn read_u16_le_async<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b).await?;
    Ok(u16::from_le_bytes(b))
}

/// Read a little-endian u32 asynchronously.
pub async fn read_u32_le_async<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).await?;
    Ok(u32::from_le_bytes(b))
}

/// Read a length-prefixed string asynchronously.
pub async fn read_string_async<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<String> {
    let len = read_u32_le_async(r).await? as usize;
    let bytes = read_exact_async(r, len).await?;
    String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Read a length-prefixed byte slice asynchronously.
pub async fn read_bytes_async<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Vec<u8>> {
    let len = read_u32_le_async(r).await? as usize;
    read_exact_async(r, len).await
}
