//! In-process task queue, accessible from PHP userland.
//!
//! A companion to [`SharedTable`](crate::shared_table) that provides async
//! job dispatch without Redis/SQS/RabbitMQ.  Producers (request handlers)
//! push jobs onto a named channel; consumers (separate PHP CLI workers or
//! the same process) pop jobs off and process them.
//!
//! # Design
//!
//! - One FIFO `VecDeque` per channel, guarded by a `parking_lot::Mutex`
//!   (short critical sections, no await under lock).
//! - A `tokio::sync::Notify` per channel powers the long-poll `pop` — a
//!   consumer may wait up to `wait_ms` for a job to arrive rather than
//!   tight-looping.
//! - Bounded capacity per channel: pushes past the limit are rejected
//!   with `QueueError::Full` rather than silently dropped.
//! - Bounded channel count: creating a new channel past `max_channels`
//!   is rejected.  This prevents a buggy worker from blowing up memory
//!   by pushing to random channel names.
//! - Job IDs are monotonic `u64` from a process-wide counter so
//!   consumers can de-duplicate or correlate with tracing.
//!
//! This is NOT a durable queue.  Jobs live only while the Turbine process
//! is running.  If you need at-least-once delivery across restarts, use
//! a real broker.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::Notify;

/// Errors returned by the task queue.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QueueError {
    /// Channel reached its per-channel capacity.
    #[error("channel '{0}' full (max {1})")]
    Full(String, usize),
    /// Process reached its configured channel count.
    #[error("too many channels (max {0})")]
    TooManyChannels(usize),
}

/// A single enqueued job.
#[derive(Debug, Clone)]
pub struct Job {
    /// Monotonic process-wide identifier.
    pub id: u64,
    /// Opaque byte payload supplied by the producer.
    pub payload: Vec<u8>,
    /// Monotonic timestamp of enqueue — used for queueing-latency metrics.
    #[allow(dead_code)]
    pub enqueued_at: Instant,
}

struct Channel {
    queue: Mutex<VecDeque<Job>>,
    notify: Notify,
    capacity: usize,
}

/// Process-wide task queue.
pub struct TaskQueue {
    channels: DashMap<String, Arc<Channel>>,
    max_channels: usize,
    default_channel_capacity: usize,
    next_id: AtomicU64,
    total_pushed: AtomicU64,
    total_popped: AtomicU64,
    total_rejected: AtomicU64,
}

impl TaskQueue {
    /// Create a new queue with the given bounds.
    pub fn new(max_channels: usize, default_channel_capacity: usize) -> Self {
        Self {
            channels: DashMap::new(),
            max_channels,
            default_channel_capacity,
            next_id: AtomicU64::new(1),
            total_pushed: AtomicU64::new(0),
            total_popped: AtomicU64::new(0),
            total_rejected: AtomicU64::new(0),
        }
    }

    fn channel_or_create(&self, name: &str) -> Result<Arc<Channel>, QueueError> {
        if let Some(ch) = self.channels.get(name) {
            return Ok(ch.clone());
        }
        // Slow path: create channel if we're under the cap.  We racily check
        // `len()` but the worst case is overshooting by a handful — still
        // bounded.
        if self.channels.len() >= self.max_channels {
            return Err(QueueError::TooManyChannels(self.max_channels));
        }
        let ch = Arc::new(Channel {
            queue: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            capacity: self.default_channel_capacity,
        });
        // `entry` avoids a double-create race.
        Ok(self.channels.entry(name.to_string()).or_insert(ch).clone())
    }

    /// Push a job onto `channel`.  Returns the assigned job id.
    pub fn push(&self, channel: &str, payload: Vec<u8>) -> Result<u64, QueueError> {
        let ch = self.channel_or_create(channel)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut q = ch.queue.lock();
            if q.len() >= ch.capacity {
                self.total_rejected.fetch_add(1, Ordering::Relaxed);
                return Err(QueueError::Full(channel.to_string(), ch.capacity));
            }
            q.push_back(Job {
                id,
                payload,
                enqueued_at: Instant::now(),
            });
        }
        ch.notify.notify_one();
        self.total_pushed.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    /// Pop a job from `channel`, waiting up to `wait` for one to arrive.
    /// Returns `None` if the wait elapses without any job.
    pub async fn pop(&self, channel: &str, wait: Duration) -> Option<Job> {
        // Creating a channel on pop would let a pathological consumer
        // allocate channels forever; avoid it.
        let ch = self.channels.get(channel)?.clone();

        // Fast path: grab one if available.
        if let Some(j) = ch.queue.lock().pop_front() {
            self.total_popped.fetch_add(1, Ordering::Relaxed);
            return Some(j);
        }
        if wait.is_zero() {
            return None;
        }
        // Slow path: long-poll.  `Notify` is one-shot-ish so we loop until
        // we actually grab a job or the deadline passes.
        let deadline = Instant::now() + wait;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let notified = ch.notify.notified();
            // Re-check under the lock before sleeping — another pop may
            // have drained the entry between the first check and now.
            if let Some(j) = ch.queue.lock().pop_front() {
                self.total_popped.fetch_add(1, Ordering::Relaxed);
                return Some(j);
            }
            if tokio::time::timeout(remaining, notified).await.is_err() {
                return None;
            }
            if let Some(j) = ch.queue.lock().pop_front() {
                self.total_popped.fetch_add(1, Ordering::Relaxed);
                return Some(j);
            }
        }
    }

    /// Number of jobs waiting on `channel`.  Returns 0 if the channel
    /// does not exist.
    pub fn size(&self, channel: &str) -> usize {
        self.channels
            .get(channel)
            .map(|ch| ch.queue.lock().len())
            .unwrap_or(0)
    }

    /// Drop every job in `channel`.  Returns the number removed.
    pub fn clear(&self, channel: &str) -> usize {
        match self.channels.get(channel) {
            Some(ch) => {
                let mut q = ch.queue.lock();
                let n = q.len();
                q.clear();
                n
            }
            None => 0,
        }
    }

    /// Count of active channels.
    #[allow(dead_code)]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Running totals — cheap to read, used by the HTTP stats endpoint.
    pub fn stats(&self) -> QueueStats {
        QueueStats {
            channels: self.channels.len(),
            pushed: self.total_pushed.load(Ordering::Relaxed),
            popped: self.total_popped.load(Ordering::Relaxed),
            rejected: self.total_rejected.load(Ordering::Relaxed),
        }
    }
}

/// Running totals for the queue.
#[derive(Debug, Clone, Copy)]
pub struct QueueStats {
    pub channels: usize,
    pub pushed: u64,
    pub popped: u64,
    pub rejected: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn push_then_pop_returns_same_payload() {
        let q = TaskQueue::new(8, 16);
        let id = q.push("email", b"hello".to_vec()).unwrap();
        let job = q.pop("email", Duration::ZERO).await.unwrap();
        assert_eq!(job.id, id);
        assert_eq!(job.payload, b"hello");
    }

    #[tokio::test]
    async fn pop_on_missing_channel_is_none() {
        let q = TaskQueue::new(8, 16);
        assert!(q.pop("nope", Duration::ZERO).await.is_none());
    }

    #[tokio::test]
    async fn push_rejects_when_channel_full() {
        let q = TaskQueue::new(8, 2);
        q.push("c", b"a".to_vec()).unwrap();
        q.push("c", b"b".to_vec()).unwrap();
        let err = q.push("c", b"c".to_vec()).unwrap_err();
        assert!(matches!(err, QueueError::Full(_, 2)));
    }

    #[tokio::test]
    async fn push_rejects_beyond_channel_cap() {
        let q = TaskQueue::new(1, 4);
        q.push("a", b"x".to_vec()).unwrap();
        let err = q.push("b", b"y".to_vec()).unwrap_err();
        assert!(matches!(err, QueueError::TooManyChannels(1)));
    }

    #[tokio::test]
    async fn pop_waits_and_wakes_on_push() {
        let q = Arc::new(TaskQueue::new(4, 16));
        // Ensure channel exists first so pop can find it.
        q.push("c", b"first".to_vec()).unwrap();
        let _ = q.pop("c", Duration::ZERO).await.unwrap();

        let q2 = q.clone();
        let h = tokio::spawn(async move { q2.pop("c", Duration::from_millis(500)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        q.push("c", b"late".to_vec()).unwrap();
        let got = h.await.unwrap().expect("pop should have returned");
        assert_eq!(got.payload, b"late");
    }

    #[tokio::test]
    async fn pop_times_out_when_empty() {
        let q = TaskQueue::new(4, 16);
        q.push("c", b"x".to_vec()).unwrap();
        let _ = q.pop("c", Duration::ZERO).await;
        let start = Instant::now();
        let got = q.pop("c", Duration::from_millis(50)).await;
        assert!(got.is_none());
        assert!(start.elapsed() >= Duration::from_millis(40));
    }

    #[tokio::test]
    async fn size_and_clear() {
        let q = TaskQueue::new(4, 16);
        for i in 0..5u8 {
            q.push("c", vec![i]).unwrap();
        }
        assert_eq!(q.size("c"), 5);
        assert_eq!(q.clear("c"), 5);
        assert_eq!(q.size("c"), 0);
    }

    #[tokio::test]
    async fn ids_are_monotonic_across_channels() {
        let q = TaskQueue::new(4, 16);
        let a = q.push("a", b"1".to_vec()).unwrap();
        let b = q.push("b", b"1".to_vec()).unwrap();
        let c = q.push("a", b"2".to_vec()).unwrap();
        assert!(a < b && b < c);
    }

    #[tokio::test]
    async fn stats_track_activity() {
        let q = TaskQueue::new(2, 1);
        q.push("a", b"1".to_vec()).unwrap();
        q.push("a", b"2".to_vec()).unwrap_err();
        let _ = q.pop("a", Duration::ZERO).await.unwrap();
        let s = q.stats();
        assert_eq!(s.pushed, 1);
        assert_eq!(s.popped, 1);
        assert_eq!(s.rejected, 1);
        assert_eq!(s.channels, 1);
    }
}
