//! High-level safe wrapper around the PHP embed SAPI.
//!
//! Provides a safe Rust API for initializing, executing, and shutting down
//! the embedded PHP runtime.

mod engine;
mod error;
pub mod output;

pub use engine::{PhpEngine, PhpIniOverrides, PhpResponse};
pub use error::EngineError;
pub use output::{clear_output_buffer, output_len, take_headers, take_output, take_response_code};

/// Register an uploaded file so that `is_uploaded_file()`/`move_uploaded_file()` recognize it.
///
/// # Safety
/// Must only be called from the PHP thread, after engine init and during an active request.
pub unsafe fn register_uploaded_file(path: &str) {
    turbine_php_sys::register_uploaded_file(path);
}
