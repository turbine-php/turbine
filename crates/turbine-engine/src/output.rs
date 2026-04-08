//! PHP output capture via SAPI ub_write and header_handler interception.
//!
//! The embed SAPI writes PHP output (echo, print, etc.) to stdout by default.
//! We replace the `ub_write` callback with our own that appends to a
//! process-local buffer, allowing us to capture PHP output from Rust.
//!
//! We also replace `header_handler` to capture HTTP headers set via PHP's
//! `header()`, `setcookie()`, and `http_response_code()`.

use std::cell::RefCell;

use libc::{c_char, c_int, c_void, size_t};
use tracing::trace;

use turbine_php_sys::{sapi_header_struct, sapi_module, SapiUbWrite, read_sapi_response_code};

thread_local! {
    /// Buffer that accumulates PHP output for the current request.
    static OUTPUT_BUFFER: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(4096));

    /// Buffer that accumulates HTTP headers from PHP header() calls.
    static HEADER_BUFFER: RefCell<Vec<(String, String)>> = RefCell::new(Vec::with_capacity(16));

    /// HTTP response status code set by PHP (via http_response_code() or header("HTTP/...")).
    static RESPONSE_CODE: RefCell<u16> = RefCell::new(200);
}

/// The original ub_write callback (saved so we can restore it if needed).
static mut ORIGINAL_UB_WRITE: Option<SapiUbWrite> = None;

/// Custom ub_write that captures PHP output into our buffer.
///
/// # Safety
/// Called from PHP engine via C function pointer. `str` must be valid for
/// `str_length` bytes.
unsafe extern "C" fn turbine_ub_write(str: *const c_char, str_length: size_t) -> size_t {
    if !str.is_null() && str_length > 0 {
        let slice = std::slice::from_raw_parts(str as *const u8, str_length);
        OUTPUT_BUFFER.with(|buf| {
            buf.borrow_mut().extend_from_slice(slice);
        });
        trace!(bytes = str_length, "Captured PHP output");
    }
    str_length
}

/// SAPI header_handler operation constants (from PHP sapi.h).
const SAPI_HEADER_REPLACE: c_int = 0;
const SAPI_HEADER_ADD: c_int = 1;
const SAPI_HEADER_DELETE: c_int = 2;
const SAPI_HEADER_DELETE_ALL: c_int = 3;
const SAPI_HEADER_SET_STATUS: c_int = 4;

/// Return code from header_handler indicating success.
const SAPI_HEADER_SENT_SUCCESSFULLY: c_int = 1;

/// Custom header_handler that captures PHP header() calls.
///
/// # Safety
/// Called from PHP engine via C function pointer.
unsafe extern "C" fn turbine_header_handler(
    sapi_header_ptr: *mut c_void,
    op: c_int,
    _sapi_headers: *mut c_void,
) -> c_int {
    let sapi_header = sapi_header_ptr as *mut sapi_header_struct;
    match op {
        SAPI_HEADER_DELETE_ALL => {
            HEADER_BUFFER.with(|buf| buf.borrow_mut().clear());
            trace!("PHP: cleared all headers");
        }
        SAPI_HEADER_SET_STATUS => {
            // Status code is passed in the header struct's header_len field
            // (or via http_response_code). We'll also parse from "HTTP/..." headers.
        }
        SAPI_HEADER_DELETE => {
            if !sapi_header.is_null() {
                let header_ptr = (*sapi_header).header;
                let header_len = (*sapi_header).header_len;
                if !header_ptr.is_null() && header_len > 0 {
                    let header_bytes = std::slice::from_raw_parts(header_ptr as *const u8, header_len);
                    if let Ok(header_str) = std::str::from_utf8(header_bytes) {
                        let name = header_str.trim().to_lowercase();
                        HEADER_BUFFER.with(|buf| {
                            buf.borrow_mut().retain(|(k, _)| k.to_lowercase() != name);
                        });
                        trace!(header = header_str, "PHP: deleted header");
                    }
                }
            }
        }
        SAPI_HEADER_REPLACE | SAPI_HEADER_ADD => {
            if !sapi_header.is_null() {
                let header_ptr = (*sapi_header).header;
                let header_len = (*sapi_header).header_len;
                if !header_ptr.is_null() && header_len > 0 {
                    let header_bytes = std::slice::from_raw_parts(header_ptr as *const u8, header_len);
                    if let Ok(header_str) = std::str::from_utf8(header_bytes) {
                        // Check for HTTP status line: "HTTP/1.1 302 Found"
                        if header_str.starts_with("HTTP/") {
                            if let Some(code_str) = header_str.split_whitespace().nth(1) {
                                if let Ok(code) = code_str.parse::<u16>() {
                                    RESPONSE_CODE.with(|rc| *rc.borrow_mut() = code);
                                    trace!(code = code, "PHP: set HTTP status code");
                                }
                            }
                        } else if let Some((name, value)) = header_str.split_once(':') {
                            let name = name.trim().to_string();
                            let value = value.trim().to_string();

                            if op == SAPI_HEADER_REPLACE {
                                // Replace existing header with same name
                                HEADER_BUFFER.with(|buf| {
                                    let mut b = buf.borrow_mut();
                                    b.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
                                    b.push((name.clone(), value.clone()));
                                });
                            } else {
                                // Add (allow duplicates, e.g. Set-Cookie)
                                HEADER_BUFFER.with(|buf| {
                                    buf.borrow_mut().push((name.clone(), value.clone()));
                                });
                            }
                            trace!(name = %name, value = %value, "PHP: captured header");
                        }
                    }
                }
            }
        }
        _ => {}
    }
    SAPI_HEADER_SENT_SUCCESSFULLY
}

/// Install our custom ub_write and header handlers, replacing the embed SAPI defaults.
///
/// Must be called after `php_embed_init()`.
///
/// # Safety
/// Modifies the global `sapi_module` struct. Must be called once from the
/// main thread before any concurrent PHP execution.
pub unsafe fn install_output_capture() {
    ORIGINAL_UB_WRITE = sapi_module.ub_write;
    sapi_module.ub_write = Some(turbine_ub_write);
    sapi_module.header_handler = Some(turbine_header_handler);
}

/// Clear the output buffer and header buffer. Call before executing each request.
pub fn clear_output_buffer() {
    OUTPUT_BUFFER.with(|buf| buf.borrow_mut().clear());
    HEADER_BUFFER.with(|buf| buf.borrow_mut().clear());
    RESPONSE_CODE.with(|rc| *rc.borrow_mut() = 200);
}

/// Take the accumulated output, leaving the buffer empty.
pub fn take_output() -> Vec<u8> {
    OUTPUT_BUFFER.with(|buf| {
        std::mem::take(&mut *buf.borrow_mut())
    })
}

/// Take the accumulated HTTP headers, leaving the buffer empty.
pub fn take_headers() -> Vec<(String, String)> {
    HEADER_BUFFER.with(|buf| {
        std::mem::take(&mut *buf.borrow_mut())
    })
}

/// Get the HTTP response status code set by PHP.
///
/// Checks our captured code from header_handler first, then falls back to
/// reading `SG(sapi_headers).http_response_code` directly from PHP globals.
pub fn take_response_code() -> u16 {
    RESPONSE_CODE.with(|rc| {
        let captured = *rc.borrow();
        *rc.borrow_mut() = 200;

        // If our header handler captured a non-200 code (from HTTP/... status line), use it.
        if captured != 200 {
            return captured;
        }

        // Otherwise read from PHP's sapi_globals (handles http_response_code(),
        // header("Location: ...", true, 302), etc.)
        let sg_code = unsafe { read_sapi_response_code() };
        if sg_code > 0 && sg_code != 200 {
            sg_code as u16
        } else {
            captured
        }
    })
}

/// Get the current output buffer length without copying.
pub fn output_len() -> usize {
    OUTPUT_BUFFER.with(|buf| buf.borrow().len())
}
