//! Zend Engine types and functions.

use libc::{c_char, c_int, c_long, c_uchar, c_uint, c_void, size_t};

// --- Zend types ---

/// Opaque Zend string type.
#[repr(C)]
pub struct zend_string {
    _opaque: [u8; 0],
}

/// Zend value (zval) — the universal PHP value container.
#[repr(C)]
pub struct zval {
    pub value: zend_value,
    pub u1: zval_u1,
    pub u2: zval_u2,
}

#[repr(C)]
pub union zend_value {
    pub lval: c_long,
    pub dval: f64,
    pub str_: *mut zend_string,
    pub arr: *mut c_void, // HashTable*
    pub obj: *mut c_void, // zend_object*
    pub res: *mut c_void, // zend_resource*
    pub ref_: *mut c_void, // zend_reference*
    pub ptr: *mut c_void,
}

#[repr(C)]
pub struct zval_u1 {
    pub type_info: c_uint,
}

#[repr(C)]
pub struct zval_u2 {
    pub next: c_uint,
}

// --- Zend type constants ---
pub const IS_UNDEF: c_uchar = 0;
pub const IS_NULL: c_uchar = 1;
pub const IS_FALSE: c_uchar = 2;
pub const IS_TRUE: c_uchar = 3;
pub const IS_LONG: c_uchar = 4;
pub const IS_DOUBLE: c_uchar = 5;
pub const IS_STRING: c_uchar = 6;
pub const IS_ARRAY: c_uchar = 7;
pub const IS_OBJECT: c_uchar = 8;
pub const IS_RESOURCE: c_uchar = 9;

// --- Zend Engine functions ---
extern "C" {
    /// Evaluate a PHP string. Returns SUCCESS (0) or FAILURE (-1).
    pub fn zend_eval_string(
        str: *const c_char,
        retval: *mut zval,
        string_name: *const c_char,
    ) -> c_int;

    /// Evaluate a PHP string (extended version with error handling).
    pub fn zend_eval_string_ex(
        str: *const c_char,
        retval: *mut zval,
        string_name: *const c_char,
        handle_exceptions: c_int,
    ) -> c_int;

    /// Hash table operations for registering uploaded files.
    pub fn zend_hash_str_add_empty_element(
        ht: *mut c_void,
        key: *const c_char,
        len: size_t,
    ) -> *mut zval;

    pub fn _zend_hash_init(
        ht: *mut c_void,
        n_size: c_uint,
        p_destructor: *const c_void,
        persistent: c_int,
    );

    // --- Executor globals ---

    /// Get the current executor globals pointer.
    /// Note: In embed SAPI, this is a simple global, not thread-local.
    pub static mut executor_globals: _zend_executor_globals;
}

/// Opaque executor globals — we only need the pointer.
#[repr(C)]
pub struct _zend_executor_globals {
    _opaque: [u8; 0],
}

// --- Zend result constants ---
pub const SUCCESS: c_int = 0;
pub const FAILURE: c_int = -1;

// --- Memory ---
extern "C" {
    pub fn emalloc(size: size_t) -> *mut c_void;
    pub fn efree(ptr: *mut c_void);
}
