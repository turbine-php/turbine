//! Lock-free latency histogram using fixed buckets.
//!
//! Bucket boundaries (in microseconds):
//! 100, 250, 500, 1000, 2500, 5000, 10_000, 25_000, 50_000, 100_000, +Inf
//!
//! Each bucket is an AtomicU64 counter — no locks, no allocations on record.

use std::sync::atomic::{AtomicU64, Ordering};

/// Fixed histogram bucket boundaries in microseconds.
const BUCKETS_US: &[u64] = &[
    100,     // 0.1ms
    250,     // 0.25ms
    500,     // 0.5ms
    1_000,   // 1ms
    2_500,   // 2.5ms
    5_000,   // 5ms
    10_000,  // 10ms
    25_000,  // 25ms
    50_000,  // 50ms
    100_000, // 100ms
];

/// Number of buckets (10 bounded + 1 overflow).
const NUM_BUCKETS: usize = 11;

pub struct LatencyHistogram {
    buckets: [AtomicU64; NUM_BUCKETS],
    sum_us: AtomicU64,
    count: AtomicU64,
}

impl LatencyHistogram {
    pub fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            sum_us: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// Record a latency sample in microseconds.
    #[inline]
    pub fn record(&self, latency_us: u64) {
        let idx = BUCKETS_US
            .iter()
            .position(|&b| latency_us <= b)
            .unwrap_or(BUCKETS_US.len()); // overflow bucket

        self.buckets[idx].fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(latency_us, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Total number of recorded samples.
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Sum of all latencies in microseconds.
    pub fn sum_us(&self) -> u64 {
        self.sum_us.load(Ordering::Relaxed)
    }

    /// Mean latency in microseconds, or 0 if no samples.
    pub fn mean_us(&self) -> u64 {
        let c = self.count();
        if c == 0 {
            0
        } else {
            self.sum_us() / c
        }
    }

    /// Approximate percentile (p50, p90, p99, etc.).
    /// `percentile` is 0.0..=1.0.
    pub fn percentile(&self, percentile: f64) -> u64 {
        let total = self.count();
        if total == 0 {
            return 0;
        }
        let target = (total as f64 * percentile).ceil() as u64;
        let mut cumulative = 0u64;
        for (i, bucket) in self.buckets.iter().enumerate() {
            cumulative += bucket.load(Ordering::Relaxed);
            if cumulative >= target {
                return if i < BUCKETS_US.len() {
                    BUCKETS_US[i]
                } else {
                    // Overflow bucket — estimate from sum
                    self.sum_us() / total
                };
            }
        }
        self.sum_us() / total
    }

    /// Render buckets in Prometheus histogram format.
    pub fn prometheus_buckets(&self, name: &str) -> String {
        let mut out = String::with_capacity(512);
        let mut cumulative = 0u64;
        for (i, &boundary) in BUCKETS_US.iter().enumerate() {
            cumulative += self.buckets[i].load(Ordering::Relaxed);
            out.push_str(&format!(
                "{name}_bucket{{le=\"{le}\"}} {count}\n",
                name = name,
                le = boundary as f64 / 1000.0, // convert to ms
                count = cumulative,
            ));
        }
        cumulative += self.buckets[BUCKETS_US.len()].load(Ordering::Relaxed);
        out.push_str(&format!(
            "{name}_bucket{{le=\"+Inf\"}} {count}\n",
            name = name,
            count = cumulative,
        ));
        out.push_str(&format!(
            "{name}_sum {sum}\n{name}_count {count}\n",
            name = name,
            sum = self.sum_us() as f64 / 1000.0,
            count = self.count(),
        ));
        out
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_count() {
        let h = LatencyHistogram::new();
        h.record(50);
        h.record(200);
        h.record(5000);
        assert_eq!(h.count(), 3);
        assert_eq!(h.sum_us(), 5250);
    }

    #[test]
    fn mean_calculation() {
        let h = LatencyHistogram::new();
        h.record(100);
        h.record(200);
        h.record(300);
        assert_eq!(h.mean_us(), 200);
    }

    #[test]
    fn empty_histogram() {
        let h = LatencyHistogram::new();
        assert_eq!(h.count(), 0);
        assert_eq!(h.mean_us(), 0);
        assert_eq!(h.percentile(0.5), 0);
    }

    #[test]
    fn percentile_p50() {
        let h = LatencyHistogram::new();
        // 10 fast requests (200μs) + 10 slow requests (8000μs)
        for _ in 0..10 {
            h.record(200);
        }
        for _ in 0..10 {
            h.record(8000);
        }
        // p50 should be in the 250μs bucket (200μs fits in ≤250)
        assert_eq!(h.percentile(0.50), 250);
        // p90 should be in the 10_000μs bucket (8000μs fits in ≤10_000)
        assert_eq!(h.percentile(0.90), 10_000);
    }

    #[test]
    fn prometheus_format() {
        let h = LatencyHistogram::new();
        h.record(500);
        let output = h.prometheus_buckets("http_request_duration");
        assert!(output.contains("http_request_duration_bucket{le=\"0.5\"} 1"));
        assert!(output.contains("http_request_duration_bucket{le=\"+Inf\"} 1"));
        assert!(output.contains("http_request_duration_count 1"));
    }
}
