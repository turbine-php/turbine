//! Request coalescing (aka "singleflight").
//!
//! When N concurrent requests arrive for the same cacheable URL and
//! nothing is in the cache, naive handling invokes PHP N times.
//! Coalescing ensures **only one** PHP execution happens — the other
//! N-1 requests wait on a [`tokio::sync::Notify`] and read the result
//! from the shared slot once the first one finishes.
//!
//! This is what Varnish and nginx call "cache lock" or "cache busy".
//! It's the single biggest server-side optimization you can apply when
//! traffic concentrates on a handful of URLs (the typical Pareto
//! distribution on real web apps).
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use turbine_cache::Coalescer;
//!
//! # async fn example() {
//! let c: Arc<Coalescer<Vec<u8>>> = Arc::new(Coalescer::new());
//!
//! let body = c.run("GET:/home", || async {
//!     // Expensive work — only the first caller runs this.
//!     render_home_page().await
//! }).await;
//! # }
//! # async fn render_home_page() -> Vec<u8> { Vec::new() }
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Notify;

/// A singleflight group that collapses concurrent calls with the same
/// key into a single execution.
///
/// The in-flight map is guarded by a fine-grained [`parking_lot::Mutex`]
/// because the critical section is microseconds; swapping it out for a
/// DashMap would actually slow things down due to shard hashing.
pub struct Coalescer<T: Clone + Send + Sync + 'static> {
    inflight: Mutex<HashMap<String, Arc<Slot<T>>>>,
}

struct Slot<T> {
    notify: Notify,
    result: Mutex<Option<T>>,
}

impl<T: Clone + Send + Sync + 'static> Coalescer<T> {
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// Try to acquire leadership for `key`.
    ///
    /// - Returns `LeaderOrFollower::Leader(guard)` when no other caller
    ///   is in flight. The caller MUST produce a value and call
    ///   `guard.publish(value)` (or drop the guard to abort, which
    ///   wakes followers with `None`).
    /// - Returns `LeaderOrFollower::Follower(future)` when another
    ///   caller is already producing. Awaiting the future yields the
    ///   leader's value (or `None` if the leader aborted).
    ///
    /// This lower-level API lets callers integrate coalescing into an
    /// existing request pipeline without reshaping it into a closure.
    pub fn acquire(self: &Arc<Self>, key: &str) -> LeaderOrFollower<T> {
        let mut map = self.inflight.lock();
        if let Some(existing) = map.get(key) {
            let slot = existing.clone();
            LeaderOrFollower::Follower(Follower { slot })
        } else {
            let slot = Arc::new(Slot {
                notify: Notify::new(),
                result: Mutex::new(None),
            });
            map.insert(key.to_string(), slot.clone());
            LeaderOrFollower::Leader(LeaderGuard {
                coalescer: self.clone(),
                key: key.to_string(),
                slot,
                published: false,
            })
        }
    }

    /// Run `f` for this `key`. If another caller is already running
    /// for the same key, wait for its result and return a clone of it.
    ///
    /// Returns `None` if the producing future is cancelled or its slot
    /// is dropped without producing a value (rare; callers should fall
    /// back to producing the value themselves).
    pub async fn run<F, Fut>(self: &Arc<Self>, key: &str, f: F) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        match self.acquire(key) {
            LeaderOrFollower::Leader(mut guard) => {
                let value = f().await;
                guard.publish(value.clone());
                Some(value)
            }
            LeaderOrFollower::Follower(f) => f.wait().await,
        }
    }

    /// How many distinct keys are currently being coalesced.
    pub fn inflight_count(&self) -> usize {
        self.inflight.lock().len()
    }
}

/// Outcome of calling [`Coalescer::acquire`].
pub enum LeaderOrFollower<T: Clone + Send + Sync + 'static> {
    /// Caller is the first for this key and must produce the value.
    Leader(LeaderGuard<T>),
    /// Caller should wait for the leader to finish.
    Follower(Follower<T>),
}

/// Held by the request that claimed leadership for a coalesced key.
///
/// Dropping without calling `publish` wakes followers with `None`
/// (they can fall back to executing themselves).
pub struct LeaderGuard<T: Clone + Send + Sync + 'static> {
    coalescer: Arc<Coalescer<T>>,
    key: String,
    slot: Arc<Slot<T>>,
    published: bool,
}

impl<T: Clone + Send + Sync + 'static> LeaderGuard<T> {
    /// Publish the produced value and wake all followers.
    pub fn publish(&mut self, value: T) {
        if self.published {
            return;
        }
        self.published = true;
        {
            let mut guard = self.slot.result.lock();
            *guard = Some(value);
        }
        self.coalescer.inflight.lock().remove(&self.key);
        self.slot.notify.notify_waiters();
    }
}

impl<T: Clone + Send + Sync + 'static> Drop for LeaderGuard<T> {
    fn drop(&mut self) {
        if !self.published {
            // Leader aborted — remove slot and wake followers with None.
            self.coalescer.inflight.lock().remove(&self.key);
            self.slot.notify.notify_waiters();
        }
    }
}

/// Handle returned to a coalesced follower.
pub struct Follower<T: Clone + Send + Sync + 'static> {
    slot: Arc<Slot<T>>,
}

impl<T: Clone + Send + Sync + 'static> Follower<T> {
    /// Wait for the leader to publish. Returns `None` if the leader
    /// aborted — caller should fall back to running the work itself.
    pub async fn wait(self) -> Option<T> {
        self.slot.notify.notified().await;
        self.slot.result.lock().clone()
    }
}

impl<T: Clone + Send + Sync + 'static> Default for Coalescer<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn single_call_runs_once() {
        let c: Arc<Coalescer<u64>> = Arc::new(Coalescer::new());
        let calls = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..50 {
            let c = c.clone();
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                c.run("key", || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    42u64
                })
                .await
            }));
        }

        for h in handles {
            assert_eq!(h.await.unwrap(), Some(42));
        }
        // All 50 concurrent calls collapsed to a single execution.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_keys_run_in_parallel() {
        let c: Arc<Coalescer<&'static str>> = Arc::new(Coalescer::new());
        let a = c.run("a", || async { "A" });
        let b = c.run("b", || async { "B" });
        let (ra, rb) = tokio::join!(a, b);
        assert_eq!(ra, Some("A"));
        assert_eq!(rb, Some("B"));
    }

    #[tokio::test]
    async fn inflight_cleared_after_completion() {
        let c: Arc<Coalescer<u64>> = Arc::new(Coalescer::new());
        let _ = c.run("key", || async { 1u64 }).await;
        assert_eq!(c.inflight_count(), 0);
    }
}
