//! Response cache store — DashMap + TTL + content hash invalidation.

use std::time::{Duration, Instant};

use dashmap::DashMap;
use tracing::debug;
use xxhash_rust::xxh3::xxh3_64;

/// Configuration for the response cache.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Default TTL for cached responses.
    pub ttl: Duration,
    /// Maximum number of entries. Oldest evicted on overflow.
    pub max_entries: usize,
    /// Enable/disable the cache entirely.
    pub enabled: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(30),
            max_entries: 1024,
            enabled: true,
        }
    }
}

/// A cached response entry.
#[derive(Clone)]
pub struct CachedResponse {
    pub body: Vec<u8>,
    pub content_type: String,
    pub status: u16,
    /// xxh3 hash of the PHP source that produced this response.
    source_hash: u64,
    /// When this entry was created.
    created_at: Instant,
    /// TTL for this entry.
    ttl: Duration,
}

impl CachedResponse {
    /// Check if this entry is still valid.
    pub fn is_valid(&self, current_source_hash: u64) -> bool {
        self.source_hash == current_source_hash
            && self.created_at.elapsed() < self.ttl
    }

    /// Remaining TTL in seconds.
    pub fn remaining_ttl_secs(&self) -> u64 {
        self.ttl
            .checked_sub(self.created_at.elapsed())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Full-response cache.
///
/// Key: `"{method}:{path}"` (e.g. `"GET:/index.php"`).
/// Thread-safe via DashMap.
pub struct ResponseCache {
    store: DashMap<String, CachedResponse>,
    config: CacheConfig,
}

impl ResponseCache {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            store: DashMap::with_capacity(config.max_entries.min(256)),
            config,
        }
    }

    /// Check if the cache is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Build cache key from method and path.
    fn cache_key(method: &str, path: &str) -> String {
        format!("{method}:{path}")
    }

    /// Hash PHP source content for invalidation.
    pub fn hash_source(source: &[u8]) -> u64 {
        xxh3_64(source)
    }

    /// Look up a cached response.
    ///
    /// Returns `Some(CachedResponse)` if a valid entry exists for this
    /// method+path and the source hash still matches.
    pub fn get(&self, method: &str, path: &str, source_hash: u64) -> Option<CachedResponse> {
        if !self.config.enabled {
            return None;
        }

        let key = Self::cache_key(method, path);
        if let Some(entry) = self.store.get(&key) {
            if entry.is_valid(source_hash) {
                debug!(key = %key, remaining_ttl = entry.remaining_ttl_secs(), "Cache hit");
                return Some(entry.clone());
            }
            // Stale or hash changed — remove
            drop(entry);
            self.store.remove(&key);
        }
        None
    }

    /// Store a response in the cache.
    pub fn put(
        &self,
        method: &str,
        path: &str,
        source_hash: u64,
        status: u16,
        content_type: &str,
        body: &[u8],
    ) {
        if !self.config.enabled {
            return;
        }

        // Only cache successful GET responses
        if method != "GET" || status != 200 {
            return;
        }

        // Evict if over capacity (simple: remove oldest)
        if self.store.len() >= self.config.max_entries {
            self.evict_oldest();
        }

        let key = Self::cache_key(method, path);
        self.store.insert(
            key,
            CachedResponse {
                body: body.to_vec(),
                content_type: content_type.to_string(),
                status,
                source_hash,
                created_at: Instant::now(),
                ttl: self.config.ttl,
            },
        );
    }

    /// Invalidate a specific path (all methods).
    pub fn invalidate(&self, path: &str) {
        // Remove GET entry (the only one we cache)
        let key = format!("GET:{path}");
        self.store.remove(&key);
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        self.store.clear();
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// Evict the oldest entry.
    fn evict_oldest(&self) {
        let mut oldest_key: Option<String> = None;
        let mut oldest_age = Duration::ZERO;

        for entry in self.store.iter() {
            let age = entry.value().created_at.elapsed();
            if age > oldest_age {
                oldest_age = age;
                oldest_key = Some(entry.key().clone());
            }
        }
        if let Some(key) = oldest_key {
            self.store.remove(&key);
            debug!(key = %key, age_ms = oldest_age.as_millis(), "Evicted oldest cache entry");
        }
    }

    /// Purge all expired entries. Call periodically for housekeeping.
    pub fn purge_expired(&self) -> usize {
        let mut removed = 0;
        self.store.retain(|_key, entry| {
            if entry.created_at.elapsed() >= entry.ttl {
                removed += 1;
                false
            } else {
                true
            }
        });
        removed
    }
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new(CacheConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn put_and_get() {
        let cache = ResponseCache::default();
        let source = b"<?php echo 'hello';";
        let hash = ResponseCache::hash_source(source);
        cache.put("GET", "/index.php", hash, 200, "text/html", b"hello");
        let hit = cache.get("GET", "/index.php", hash);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().body, b"hello");
    }

    #[test]
    fn miss_on_empty() {
        let cache = ResponseCache::default();
        assert!(cache.get("GET", "/nope", 0).is_none());
    }

    #[test]
    fn invalidate_on_source_change() {
        let cache = ResponseCache::default();
        let old_hash = ResponseCache::hash_source(b"<?php echo 'v1';");
        let new_hash = ResponseCache::hash_source(b"<?php echo 'v2';");
        cache.put("GET", "/test.php", old_hash, 200, "text/html", b"v1");

        // With old hash — hit
        assert!(cache.get("GET", "/test.php", old_hash).is_some());
        // With new hash — miss (source changed)
        assert!(cache.get("GET", "/test.php", new_hash).is_none());
    }

    #[test]
    fn ttl_expiration() {
        let config = CacheConfig {
            ttl: Duration::from_millis(50),
            ..Default::default()
        };
        let cache = ResponseCache::new(config);
        let hash = ResponseCache::hash_source(b"test");
        cache.put("GET", "/", hash, 200, "text/html", b"cached");

        assert!(cache.get("GET", "/", hash).is_some());
        thread::sleep(Duration::from_millis(60));
        assert!(cache.get("GET", "/", hash).is_none());
    }

    #[test]
    fn only_caches_get_200() {
        let cache = ResponseCache::default();
        let hash = ResponseCache::hash_source(b"test");
        cache.put("POST", "/api", hash, 200, "application/json", b"{}");
        assert!(cache.get("POST", "/api", hash).is_none());

        cache.put("GET", "/err", hash, 500, "text/plain", b"error");
        assert!(cache.get("GET", "/err", hash).is_none());
    }

    #[test]
    fn explicit_invalidate() {
        let cache = ResponseCache::default();
        let hash = ResponseCache::hash_source(b"test");
        cache.put("GET", "/page", hash, 200, "text/html", b"content");
        assert!(cache.get("GET", "/page", hash).is_some());

        cache.invalidate("/page");
        assert!(cache.get("GET", "/page", hash).is_none());
    }

    #[test]
    fn purge_expired_entries() {
        let config = CacheConfig {
            ttl: Duration::from_millis(20),
            ..Default::default()
        };
        let cache = ResponseCache::new(config);
        let hash = ResponseCache::hash_source(b"test");
        cache.put("GET", "/a", hash, 200, "text/html", b"a");
        cache.put("GET", "/b", hash, 200, "text/html", b"b");
        assert_eq!(cache.len(), 2);

        thread::sleep(Duration::from_millis(30));
        let purged = cache.purge_expired();
        assert_eq!(purged, 2);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn eviction_at_capacity() {
        let config = CacheConfig {
            max_entries: 2,
            ..Default::default()
        };
        let cache = ResponseCache::new(config);
        let hash = ResponseCache::hash_source(b"test");
        cache.put("GET", "/1", hash, 200, "text/html", b"1");
        thread::sleep(Duration::from_millis(5)); // ensure different ages
        cache.put("GET", "/2", hash, 200, "text/html", b"2");
        thread::sleep(Duration::from_millis(5));
        cache.put("GET", "/3", hash, 200, "text/html", b"3");

        // Should have evicted /1 (oldest)
        assert_eq!(cache.len(), 2);
        assert!(cache.get("GET", "/1", hash).is_none());
    }

    #[test]
    fn disabled_cache_is_noop() {
        let config = CacheConfig {
            enabled: false,
            ..Default::default()
        };
        let cache = ResponseCache::new(config);
        let hash = ResponseCache::hash_source(b"test");
        cache.put("GET", "/", hash, 200, "text/html", b"data");
        assert!(cache.get("GET", "/", hash).is_none());
        assert_eq!(cache.len(), 0);
    }
}
