//! Raw FFI bindings to libphp (PHP embed SAPI).
//!
//! This crate provides the lowest-level bindings to the PHP engine.
//! It is not meant to be used directly — use `turbine-engine` instead.

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

pub mod embed;
pub mod sapi;
pub mod zend;

pub use embed::*;
pub use sapi::*;
pub use zend::*;

/// PHP version detected at build time.
pub const PHP_VERSION: &str = env!("PHP_VERSION");

/// PHP version as an integer (e.g. "8.5.4" → 80504), computed at
/// compile time from `PHP_VERSION`. Callers can compare against
/// thresholds without re-parsing the string at runtime.
pub const PHP_VERSION_ID: u32 = compute_php_version_id(PHP_VERSION);

const fn compute_php_version_id(s: &str) -> u32 {
    // Parse "MAJOR.MINOR.PATCH[...]" into MAJOR*10000 + MINOR*100 + PATCH.
    // Anything we can't parse falls back to 0 so callers default to the
    // "old" behaviour rather than claiming a new PHP feature exists.
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut parts = [0u32; 3];
    let mut idx = 0;
    while i < bytes.len() && idx < 3 {
        let mut n = 0u32;
        let mut any = false;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            n = n * 10 + (bytes[i] - b'0') as u32;
            i += 1;
            any = true;
        }
        if !any {
            return 0;
        }
        parts[idx] = n;
        idx += 1;
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    parts[0] * 10000 + parts[1] * 100 + parts[2]
}

/// PHP extension directory detected at build time.
pub const PHP_EXTENSION_DIR: &str = env!("PHP_EXTENSION_DIR");

/// PHP installation prefix detected at build time.
pub const PHP_PREFIX: &str = env!("PHP_PREFIX");
