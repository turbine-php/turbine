//! Full-response cache for PHP output.
//!
//! Cache key = (method, path). Stored alongside the content hash (xxh3)
//! of the PHP source — if the source changes, the cache entry is invalidated.
//!
//! TTL-based expiration with configurable default (30s).
//! Lock-free reads via DashMap.

mod store;

pub use store::{ResponseCache, CacheConfig, CachedResponse};
