//! Worker pool management — fork, CoW shared memory, state reset between requests.
//!
//! This crate implements:
//! - Shared memory segment creation and sealing (mmap + mprotect)
//! - Process forking with Copy-on-Write memory sharing
//! - Thread-based workers with TSRM (ZTS PHP required)
//! - Worker lifecycle management (spawn, recycle, kill)
//! - Surgical state reset between HTTP requests
//! - Persistent PHP worker mode (bootstrap once, handle N requests)

mod error;
pub mod persistent;
pub mod pool;
mod shared_mem;
mod worker;

pub use error::WorkerError;
pub use persistent::{decode_response, encode_request, PersistentRequest, PersistentResponse};
pub use pool::{
    encode_native_request, read_native_response_from_fd, safe_cstring, worker_event_loop_channel,
    write_to_fd, NativeResponse, WorkerMode, WorkerPool,
};
pub use shared_mem::SharedMemory;
pub use worker::{Worker, WorkerKind, WorkerState};
