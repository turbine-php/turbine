//! io_uring backend scaffold (Linux-only, experimental).
//!
//! # Status
//!
//! **Not yet wired into the runtime.**  Enabling the `io-uring` feature
//! compiles this module but the request path still uses `AsyncPipe`
//! (epoll/kqueue via `tokio::io::unix::AsyncFd`).
//!
//! # Why this exists
//!
//! io_uring can eliminate the `read`/`write` syscalls on the
//! master ↔ worker pipe path by submitting I/O entries in batch through
//! a memory-mapped queue.  In `SQPOLL` mode, a kernel thread drains
//! the submission queue without any userspace syscall at all — zero
//! syscalls per request in the steady state.
//!
//! Cloudflare Pingora reported ~30% throughput improvement after
//! switching from epoll to io_uring for its core pipe path; ScyllaDB
//! uses io_uring for all I/O.
//!
//! # What a full implementation would entail
//!
//! 1. Pull in `tokio-uring` (or `glommio`) as a Linux-only dep.
//! 2. Build a thread-per-core runtime where each worker is pinned to
//!    one core along with its io_uring SQ/CQ.
//! 3. Register the master-side cmd_fd/resp_fd of every worker at
//!    startup (`io_uring_register_files`).  Registered fds skip the
//!    per-syscall fd lookup.
//! 4. Replace [`crate::main::handle_request_inner`]'s pipe write/read
//!    with linked `IORING_OP_WRITEV` + `IORING_OP_READV` SQEs.
//! 5. Graceful fallback when:
//!    - Kernel is older than 5.15 (feature probing at boot).
//!    - Running inside a seccomp-restricted environment that blocks
//!      `io_uring_setup` (common in many container runtimes — Docker
//!      default denies io_uring since 2023 CVE).
//!
//! # Why not implemented yet
//!
//! tokio-uring is not a drop-in replacement for tokio; it has its own
//! runtime and uses completion-based futures that don't compose with
//! the existing hyper + rustls stack.  Doing this right requires
//! either isolating the PHP-dispatch runtime on a dedicated thread
//! pool (and coordinating with the hyper reactor via channels), or
//! waiting for tokio-uring to land its in-flight compatibility layer.
//!
//! Target milestone: Turbine 0.3.

#![cfg(all(feature = "io-uring", target_os = "linux"))]

/// Marker type — presence indicates the feature is compiled in.
pub struct IoUringBackend;

impl IoUringBackend {
    /// Probe whether the kernel supports io_uring at all.  Returns
    /// `false` on any failure (kernel too old, seccomp filter,
    /// `io_uring_disabled` sysctl, etc.).
    pub fn kernel_supported() -> bool {
        // Real implementation would call `io_uring_setup` with a
        // 1-entry ring and close it immediately.  For now, a
        // conservative "not available" until the full backend lands.
        false
    }
}
