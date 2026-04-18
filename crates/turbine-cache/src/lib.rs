//! Full-response cache for PHP output.
//!
//! Cache key = (method, path). Stored alongside the content hash (xxh3)
//! of the PHP source — if the source changes, the cache entry is invalidated.
//!
//! TTL-based expiration with configurable default (30s).
//! Lock-free reads via DashMap.
//!
//! Also provides [`Coalescer`] for request coalescing (singleflight):
//! when N concurrent requests arrive for the same missing cache key,
//! only one PHP execution runs and the rest receive a clone of its
//! output.

mod coalescer;
mod store;

pub use coalescer::{Coalescer, Follower, LeaderGuard, LeaderOrFollower};
pub use store::{CacheConfig, CachedResponse, ResponseCache};
