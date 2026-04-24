//! PHP embed SAPI — the primary interface for embedding PHP in Rust.

use libc::{c_char, c_int, c_long, c_void};

use crate::sapi::sapi_module_struct;

extern "C" {
    /// Initialize the PHP embed SAPI.
    ///
    /// `argc` and `argv` are the command-line arguments (can be 0/null).
    /// Returns SUCCESS (0) or FAILURE (-1).
    pub fn php_embed_init(argc: c_int, argv: *mut *mut c_char) -> c_int;

    /// Shut down the PHP embed SAPI.
    /// Cleans up all PHP resources.
    pub fn php_embed_shutdown();

    /// The embed SAPI module struct (same layout as sapi_module_struct).
    /// Must be configured BEFORE calling php_embed_init().
    pub static mut php_embed_module: sapi_module_struct;
}

// --- PHP lifecycle functions ---
extern "C" {
    /// Request startup — called before executing each request.
    pub fn php_request_startup() -> c_int;

    /// Request shutdown — called after each request.
    pub fn php_request_shutdown(dummy: *mut c_void);

    /// Execute a PHP file.
    pub fn php_execute_script(primary_file: *mut c_void) -> c_int;
}

// --- Turbine persistent worker lifecycle (turbine_worker_lifecycle.c) ---
//
// These functions provide lightweight per-request state management that
// preserves PHP global variables ($app, $kernel) across requests in a
// long-lived worker process.
//
// Call order:
//   turbine_worker_boot()                 ← once per worker process
//   loop:
//     turbine_worker_request_startup()    ← before each request
//     zend_eval_string(...)               ← handle the request
//     turbine_worker_request_shutdown()   ← after each request
//   turbine_worker_shutdown()             ← once before process exit
extern "C" {
    /// One-time worker initialization. Calls php_request_startup() for the
    /// bootstrap phase. Returns SUCCESS (0) or FAILURE (-1).
    pub fn turbine_worker_boot() -> c_int;

    /// Lightweight per-request startup. Re-activates SAPI and output buffering
    /// WITHOUT resetting the PHP global variable table. Returns SUCCESS or FAILURE.
    pub fn turbine_worker_request_startup() -> c_int;

    /// Lightweight per-request shutdown. Flushes output, closes session,
    /// deactivates SAPI. Does NOT destroy PHP global variables.
    pub fn turbine_worker_request_shutdown();

    /// One-time worker shutdown. Performs a full php_request_shutdown.
    pub fn turbine_worker_shutdown();
}

// --- Turbine native SAPI request handler (turbine_sapi_handler.c) ---
//
// These functions provide php-fpm-style native SAPI execution that uses
// php_execute_script() instead of zend_eval_string(). This enables OPcache
// and the standard Zend Engine execution path.
extern "C" {
    /// Install Turbine's SAPI hooks (read_post, read_cookies, register_server_variables).
    /// Must be called once per worker process after fork().
    pub fn turbine_sapi_install_hooks();

    /// Populate SG(request_info) with HTTP request metadata.
    /// Must be called BEFORE php_request_startup() for each request.
    pub fn turbine_sapi_set_request(
        method: *const c_char,
        uri: *const c_char,
        query_string: *const c_char,
        content_type: *const c_char,
        content_length: c_long,
        cookie_data: *const c_char,
        script_filename: *const c_char,
        document_root: *const c_char,
        remote_addr: *const c_char,
        remote_port: c_int,
        server_port: c_int,
        is_https: c_int,
        path_info: *const c_char,
        script_name: *const c_char,
        post_body: *const c_char,
        post_body_len: usize,
        header_count: c_int,
        header_keys: *const *const c_char,
        header_key_lens: *const usize,
        header_vals: *const *const c_char,
        header_val_lens: *const usize,
    );

    /// Execute a PHP script using the standard Zend Engine path (OPcache enabled).
    /// Returns SUCCESS (0) or FAILURE (-1).
    pub fn turbine_execute_script(filename: *const c_char) -> c_int;
}

// --- Turbine thread support (turbine_thread_support.c) ---
//
// These functions provide ZTS (Zend Thread Safety) detection and per-thread
// TSRM context management for thread-mode workers.
//
// In NTS builds, `turbine_php_is_thread_safe()` returns 0 and the init/cleanup
// functions are safe no-ops. Thread mode must NOT be used with NTS PHP.
extern "C" {
    /// Check whether PHP was compiled with ZTS (Zend Thread Safety).
    /// Returns 1 if ZTS, 0 if NTS.
    pub fn turbine_php_is_thread_safe() -> c_int;

    /// Initialize a TSRM interpreter context for the calling thread.
    /// Must be called from each worker thread BEFORE any PHP operations.
    /// Returns 0 on success, -1 on failure. No-op in NTS mode.
    pub fn turbine_thread_init() -> c_int;

    /// Clean up the TSRM interpreter context for the calling thread.
    /// Must be called before thread exit. No-op in NTS mode.
    pub fn turbine_thread_cleanup();
}
