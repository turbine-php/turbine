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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Output Buffer ───────────────────────────────────────────────

    #[test]
    fn clear_output_buffer_resets_all() {
        // Set some state
        OUTPUT_BUFFER.with(|buf| buf.borrow_mut().extend_from_slice(b"hello"));
        HEADER_BUFFER.with(|buf| buf.borrow_mut().push(("X-Test".to_string(), "val".to_string())));
        RESPONSE_CODE.with(|rc| *rc.borrow_mut() = 404);

        clear_output_buffer();

        assert_eq!(output_len(), 0);
        HEADER_BUFFER.with(|buf| assert!(buf.borrow().is_empty()));
        RESPONSE_CODE.with(|rc| assert_eq!(*rc.borrow(), 200));
    }

    #[test]
    fn take_output_returns_and_clears() {
        clear_output_buffer();
        OUTPUT_BUFFER.with(|buf| buf.borrow_mut().extend_from_slice(b"test output"));

        let output = take_output();
        assert_eq!(output, b"test output");
        assert_eq!(output_len(), 0); // buffer is now empty
    }

    #[test]
    fn take_output_empty() {
        clear_output_buffer();
        let output = take_output();
        assert!(output.is_empty());
    }

    #[test]
    fn take_headers_returns_and_clears() {
        clear_output_buffer();
        HEADER_BUFFER.with(|buf| {
            let mut b = buf.borrow_mut();
            b.push(("Content-Type".to_string(), "text/html".to_string()));
            b.push(("X-Powered-By".to_string(), "Turbine".to_string()));
        });

        let headers = take_headers();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].0, "Content-Type");
        assert_eq!(headers[1].0, "X-Powered-By");

        // Buffer should be empty after take
        HEADER_BUFFER.with(|buf| assert!(buf.borrow().is_empty()));
    }

    #[test]
    fn take_headers_empty() {
        clear_output_buffer();
        let headers = take_headers();
        assert!(headers.is_empty());
    }

    #[test]
    fn take_response_code_default() {
        clear_output_buffer();
        let code = take_response_code();
        assert_eq!(code, 200);
    }

    #[test]
    fn take_response_code_custom() {
        clear_output_buffer();
        RESPONSE_CODE.with(|rc| *rc.borrow_mut() = 302);
        let code = take_response_code();
        assert_eq!(code, 302);
        // Should reset to 200 after take
        RESPONSE_CODE.with(|rc| assert_eq!(*rc.borrow(), 200));
    }

    #[test]
    fn output_len_tracks_buffer_size() {
        clear_output_buffer();
        assert_eq!(output_len(), 0);

        OUTPUT_BUFFER.with(|buf| buf.borrow_mut().extend_from_slice(b"12345"));
        assert_eq!(output_len(), 5);

        OUTPUT_BUFFER.with(|buf| buf.borrow_mut().extend_from_slice(b"67890"));
        assert_eq!(output_len(), 10);
    }

    // ── turbine_ub_write callback ───────────────────────────────────

    #[test]
    fn ub_write_captures_output() {
        clear_output_buffer();
        let data = b"Hello from PHP!";
        unsafe {
            turbine_ub_write(data.as_ptr() as *const c_char, data.len());
        }
        let output = take_output();
        assert_eq!(output, b"Hello from PHP!");
    }

    #[test]
    fn ub_write_multiple_calls_append() {
        clear_output_buffer();
        let d1 = b"Hello ";
        let d2 = b"World";
        unsafe {
            turbine_ub_write(d1.as_ptr() as *const c_char, d1.len());
            turbine_ub_write(d2.as_ptr() as *const c_char, d2.len());
        }
        let output = take_output();
        assert_eq!(output, b"Hello World");
    }

    #[test]
    fn ub_write_null_pointer_safe() {
        clear_output_buffer();
        unsafe {
            let result = turbine_ub_write(std::ptr::null(), 0);
            assert_eq!(result, 0);
        }
        assert_eq!(output_len(), 0);
    }

    #[test]
    fn ub_write_zero_length_safe() {
        clear_output_buffer();
        let data = b"data";
        unsafe {
            let result = turbine_ub_write(data.as_ptr() as *const c_char, 0);
            assert_eq!(result, 0);
        }
        assert_eq!(output_len(), 0);
    }

    #[test]
    fn ub_write_returns_length() {
        clear_output_buffer();
        let data = b"test";
        unsafe {
            let result = turbine_ub_write(data.as_ptr() as *const c_char, data.len());
            assert_eq!(result, 4);
        }
    }

    #[test]
    fn ub_write_binary_data() {
        clear_output_buffer();
        let data: Vec<u8> = (0..=255).collect();
        unsafe {
            turbine_ub_write(data.as_ptr() as *const c_char, data.len());
        }
        let output = take_output();
        assert_eq!(output.len(), 256);
        assert_eq!(output[0], 0);
        assert_eq!(output[255], 255);
    }

    // ── turbine_header_handler callback ─────────────────────────────

    fn make_sapi_header(s: &str) -> (sapi_header_struct, Vec<u8>) {
        let bytes: Vec<u8> = s.as_bytes().to_vec();
        let header = sapi_header_struct {
            header: bytes.as_ptr() as *mut c_char,
            header_len: bytes.len(),
        };
        (header, bytes) // return bytes to keep alive
    }

    #[test]
    fn header_handler_captures_header() {
        clear_output_buffer();
        let (mut hdr, _bytes) = make_sapi_header("Content-Type: text/html");
        unsafe {
            turbine_header_handler(
                &mut hdr as *mut _ as *mut c_void,
                SAPI_HEADER_REPLACE,
                std::ptr::null_mut(),
            );
        }
        let headers = take_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "Content-Type");
        assert_eq!(headers[0].1, "text/html");
    }

    #[test]
    fn header_handler_add_allows_duplicates() {
        clear_output_buffer();
        let (mut hdr1, _b1) = make_sapi_header("Set-Cookie: a=1");
        let (mut hdr2, _b2) = make_sapi_header("Set-Cookie: b=2");
        unsafe {
            turbine_header_handler(&mut hdr1 as *mut _ as *mut c_void, SAPI_HEADER_ADD, std::ptr::null_mut());
            turbine_header_handler(&mut hdr2 as *mut _ as *mut c_void, SAPI_HEADER_ADD, std::ptr::null_mut());
        }
        let headers = take_headers();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].1, "a=1");
        assert_eq!(headers[1].1, "b=2");
    }

    #[test]
    fn header_handler_replace_removes_previous() {
        clear_output_buffer();
        let (mut hdr1, _b1) = make_sapi_header("Content-Type: text/plain");
        let (mut hdr2, _b2) = make_sapi_header("Content-Type: application/json");
        unsafe {
            turbine_header_handler(&mut hdr1 as *mut _ as *mut c_void, SAPI_HEADER_REPLACE, std::ptr::null_mut());
            turbine_header_handler(&mut hdr2 as *mut _ as *mut c_void, SAPI_HEADER_REPLACE, std::ptr::null_mut());
        }
        let headers = take_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].1, "application/json");
    }

    #[test]
    fn header_handler_delete_all() {
        clear_output_buffer();
        HEADER_BUFFER.with(|buf| {
            let mut b = buf.borrow_mut();
            b.push(("A".to_string(), "1".to_string()));
            b.push(("B".to_string(), "2".to_string()));
        });
        unsafe {
            turbine_header_handler(std::ptr::null_mut(), SAPI_HEADER_DELETE_ALL, std::ptr::null_mut());
        }
        let headers = take_headers();
        assert!(headers.is_empty());
    }

    #[test]
    fn header_handler_delete_specific() {
        clear_output_buffer();
        HEADER_BUFFER.with(|buf| {
            let mut b = buf.borrow_mut();
            b.push(("Content-Type".to_string(), "text/html".to_string()));
            b.push(("X-Custom".to_string(), "value".to_string()));
        });
        let (mut hdr, _bytes) = make_sapi_header("content-type");
        unsafe {
            turbine_header_handler(&mut hdr as *mut _ as *mut c_void, SAPI_HEADER_DELETE, std::ptr::null_mut());
        }
        let headers = take_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "X-Custom");
    }

    #[test]
    fn header_handler_http_status_line() {
        clear_output_buffer();
        let (mut hdr, _bytes) = make_sapi_header("HTTP/1.1 302 Found");
        unsafe {
            turbine_header_handler(&mut hdr as *mut _ as *mut c_void, SAPI_HEADER_REPLACE, std::ptr::null_mut());
        }
        RESPONSE_CODE.with(|rc| {
            assert_eq!(*rc.borrow(), 302);
        });
        // HTTP status line should NOT be added as a header
        let headers = take_headers();
        assert!(headers.is_empty());
    }

    #[test]
    fn header_handler_null_safe() {
        clear_output_buffer();
        unsafe {
            let result = turbine_header_handler(std::ptr::null_mut(), SAPI_HEADER_REPLACE, std::ptr::null_mut());
            assert_eq!(result, SAPI_HEADER_SENT_SUCCESSFULLY);
        }
        // Should not crash or add anything
        let headers = take_headers();
        assert!(headers.is_empty());
    }

    // ── Thread safety (each thread has its own buffer) ──────────────

    #[test]
    fn buffers_are_thread_local() {
        clear_output_buffer();
        OUTPUT_BUFFER.with(|buf| buf.borrow_mut().extend_from_slice(b"main thread"));

        let handle = std::thread::spawn(|| {
            // Different thread should have its own empty buffer
            let len = output_len();
            assert_eq!(len, 0);

            OUTPUT_BUFFER.with(|buf| buf.borrow_mut().extend_from_slice(b"child thread"));
            let output = take_output();
            assert_eq!(output, b"child thread");
        });
        handle.join().unwrap();

        // Main thread's buffer should be untouched
        let output = take_output();
        assert_eq!(output, b"main thread");
    }
}
