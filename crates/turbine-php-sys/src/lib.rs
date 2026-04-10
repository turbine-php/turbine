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

/// PHP extension directory detected at build time.
pub const PHP_EXTENSION_DIR: &str = env!("PHP_EXTENSION_DIR");

/// PHP installation prefix detected at build time.
pub const PHP_PREFIX: &str = env!("PHP_PREFIX");
