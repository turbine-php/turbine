//! SAPI (Server Application Programming Interface) types.
//!
//! Matches the layout of `struct _sapi_module_struct` from PHP's SAPI.h.

use libc::{c_char, c_int, c_uint, c_void, size_t};

use crate::zval;

/// Output write callback type — `size_t (*ub_write)(const char *str, size_t str_length)`
pub type SapiUbWrite = unsafe extern "C" fn(str: *const c_char, str_length: size_t) -> size_t;

/// Flush callback type — `void (*flush)(void *server_context)`
pub type SapiFlush = unsafe extern "C" fn(server_context: *mut c_void);

/// Header handler callback type — matches PHP SAPI header_handler signature.
/// `int (*header_handler)(sapi_header_struct *sapi_header, sapi_header_op_enum op, sapi_headers_struct *sapi_headers)`
pub type SapiHeaderHandler = unsafe extern "C" fn(
    sapi_header: *mut sapi_header_struct,
    op: c_int,
    sapi_headers: *mut c_void,
) -> c_int;

/// Matches PHP's `sapi_header_struct` from SAPI.h.
/// Layout: { char *header; size_t header_len; }
#[repr(C)]
pub struct sapi_header_struct {
    pub header: *mut c_char,
    pub header_len: size_t,
}

/// SAPI module struct — full layout matching PHP 8.x SAPI.h.
///
/// Fields marked `_pad_*` are function pointers we don't use but must
/// occupy the correct offset in memory.
#[repr(C)]
pub struct sapi_module_struct {
    pub name: *mut c_char,
    pub pretty_name: *mut c_char,

    pub startup: Option<unsafe extern "C" fn(*mut sapi_module_struct) -> c_int>,
    pub shutdown: Option<unsafe extern "C" fn(*mut sapi_module_struct) -> c_int>,

    pub activate: Option<unsafe extern "C" fn() -> c_int>,
    pub deactivate: Option<unsafe extern "C" fn() -> c_int>,

    pub ub_write: Option<SapiUbWrite>,
    pub flush: Option<SapiFlush>,

    pub get_stat: Option<unsafe extern "C" fn() -> *mut c_void>,
    pub getenv: Option<unsafe extern "C" fn(name: *const c_char, name_len: size_t) -> *mut c_char>,

    pub sapi_error: Option<unsafe extern "C" fn(r#type: c_int, error_msg: *const c_char, ...)>,

    pub header_handler:
        Option<unsafe extern "C" fn(header: *mut c_void, op: c_int, headers: *mut c_void) -> c_int>,
    pub send_headers: Option<unsafe extern "C" fn(headers: *mut c_void) -> c_int>,
    pub send_header: Option<unsafe extern "C" fn(header: *mut c_void, server_context: *mut c_void)>,

    pub read_post: Option<unsafe extern "C" fn(buffer: *mut c_char, count_bytes: size_t) -> size_t>,
    pub read_cookies: Option<unsafe extern "C" fn() -> *mut c_char>,

    pub register_server_variables: Option<unsafe extern "C" fn(track_vars_array: *mut zval)>,
    pub log_message: Option<unsafe extern "C" fn(message: *const c_char, syslog_type_int: c_int)>,
    pub get_request_time: Option<unsafe extern "C" fn(request_time: *mut f64) -> c_int>,
    pub terminate_process: Option<unsafe extern "C" fn()>,

    pub php_ini_path_override: *mut c_char,

    pub default_post_reader: Option<unsafe extern "C" fn()>,
    pub treat_data:
        Option<unsafe extern "C" fn(arg: c_int, str: *mut c_char, dest_array: *mut zval)>,
    pub executable_location: *mut c_char,

    pub php_ini_ignore: c_int,
    pub php_ini_ignore_cwd: c_int,

    pub get_fd: Option<unsafe extern "C" fn(fd: *mut c_int) -> c_int>,
    pub force_http_10: Option<unsafe extern "C" fn() -> c_int>,
    pub get_target_uid: Option<unsafe extern "C" fn(uid: *mut c_uint) -> c_int>,
    pub get_target_gid: Option<unsafe extern "C" fn(gid: *mut c_uint) -> c_int>,

    pub input_filter: Option<
        unsafe extern "C" fn(
            arg: c_int,
            var: *const c_char,
            val: *mut *mut c_char,
            val_len: size_t,
            new_val_len: *mut size_t,
        ) -> c_uint,
    >,
    pub ini_defaults: Option<unsafe extern "C" fn(configuration_hash: *mut c_void)>,
    pub phpinfo_as_text: c_int,

    pub ini_entries: *const c_char,
    pub additional_functions: *const c_void,
    pub input_filter_init: Option<unsafe extern "C" fn() -> c_uint>,
}

extern "C" {
    pub static mut sapi_module: sapi_module_struct;

    /// Send headers.
    pub fn sapi_send_headers() -> c_int;

    /// ZTS-safe accessor: read SG(sapi_headers).http_response_code
    fn turbine_read_response_code() -> c_int;

    /// ZTS-safe accessor: register file in SG(rfc1867_uploaded_files)
    fn turbine_register_uploaded_file_c(path: *const c_char, path_len: size_t);
}

/// Register a file path as an uploaded file in PHP's `SG(rfc1867_uploaded_files)` hash table.
///
/// This makes `is_uploaded_file()` and `move_uploaded_file()` recognize the file.
///
/// # Safety
/// Must only be called after PHP engine is initialised and during an active request.
pub unsafe fn register_uploaded_file(path: &str) {
    let cleaned: Vec<u8> = path.bytes().filter(|&b| b != 0).collect();
    let c_path = std::ffi::CString::new(cleaned).unwrap_or_default();
    turbine_register_uploaded_file_c(c_path.as_ptr(), path.len());
}

/// Read the HTTP response code from PHP's sapi_globals.sapi_headers.http_response_code.
///
/// Uses a C wrapper that accesses SG() macro, which is safe for both NTS and ZTS builds.
///
/// # Safety
/// Must only be called after PHP engine is initialised and while it's still running.
pub unsafe fn read_sapi_response_code() -> c_int {
    turbine_read_response_code()
}
