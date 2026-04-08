//! Central metrics collector.
//!
//! All counters are lock-free atomics. Per-endpoint data uses DashMap.
//! Safe to call from any process/thread without synchronization.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use dashmap::DashMap;

use crate::LatencyHistogram;

/// Per-endpoint metrics.
pub struct EndpointMetrics {
    pub requests: AtomicU64,
    pub errors: AtomicU64,
    pub bytes_out: AtomicU64,
    pub latency: LatencyHistogram,
}

impl EndpointMetrics {
    fn new() -> Self {
        Self {
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            latency: LatencyHistogram::new(),
        }
    }
}

/// Global metrics collector — one per runtime instance.
pub struct MetricsCollector {
    // Global counters
    pub total_requests: AtomicU64,
    pub total_errors: AtomicU64,
    pub total_bytes_out: AtomicU64,
    pub total_2xx: AtomicU64,
    pub total_3xx: AtomicU64,
    pub total_4xx: AtomicU64,
    pub total_5xx: AtomicU64,
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub security_blocks: AtomicU64,

    // Global latency histogram
    pub latency: LatencyHistogram,

    // Per-endpoint breakdown
    pub endpoints: DashMap<String, EndpointMetrics>,

    // Uptime
    started_at: Instant,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            total_bytes_out: AtomicU64::new(0),
            total_2xx: AtomicU64::new(0),
            total_3xx: AtomicU64::new(0),
            total_4xx: AtomicU64::new(0),
            total_5xx: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            security_blocks: AtomicU64::new(0),
            latency: LatencyHistogram::new(),
            endpoints: DashMap::new(),
            started_at: Instant::now(),
        }
    }

    /// Record a completed request.
    #[inline]
    pub fn record_request(
        &self,
        path: &str,
        status: u16,
        latency_us: u64,
        bytes: u64,
    ) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_out.fetch_add(bytes, Ordering::Relaxed);
        self.latency.record(latency_us);

        match status {
            200..=299 => { self.total_2xx.fetch_add(1, Ordering::Relaxed); }
            300..=399 => { self.total_3xx.fetch_add(1, Ordering::Relaxed); }
            400..=499 => { self.total_4xx.fetch_add(1, Ordering::Relaxed); }
            500..=599 => {
                self.total_5xx.fetch_add(1, Ordering::Relaxed);
                self.total_errors.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }

        // Per-endpoint
        let ep = self
            .endpoints
            .entry(path.to_string())
            .or_insert_with(EndpointMetrics::new);
        ep.requests.fetch_add(1, Ordering::Relaxed);
        ep.bytes_out.fetch_add(bytes, Ordering::Relaxed);
        ep.latency.record(latency_us);
        if status >= 500 {
            ep.errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a cache hit.
    #[inline]
    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss.
    #[inline]
    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a security block.
    #[inline]
    pub fn record_security_block(&self) {
        self.security_blocks.fetch_add(1, Ordering::Relaxed);
    }

    /// Uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Requests per second (average since start).
    pub fn rps(&self) -> f64 {
        let secs = self.uptime_secs();
        if secs == 0 {
            return self.total_requests.load(Ordering::Relaxed) as f64;
        }
        self.total_requests.load(Ordering::Relaxed) as f64 / secs as f64
    }

    /// Cache hit ratio (0.0 - 1.0).
    pub fn cache_hit_ratio(&self) -> f64 {
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            return 0.0;
        }
        hits as f64 / total as f64
    }

    /// Render all metrics in Prometheus text exposition format.
    pub fn prometheus(&self) -> String {
        let mut out = String::with_capacity(4096);

        // Global counters
        out.push_str(&format!(
            "# HELP turbine_requests_total Total HTTP requests\n\
             # TYPE turbine_requests_total counter\n\
             turbine_requests_total {}\n\n",
            self.total_requests.load(Ordering::Relaxed),
        ));

        out.push_str(&format!(
            "# HELP turbine_errors_total Total server errors (5xx)\n\
             # TYPE turbine_errors_total counter\n\
             turbine_errors_total {}\n\n",
            self.total_errors.load(Ordering::Relaxed),
        ));

        out.push_str(&format!(
            "# HELP turbine_bytes_out_total Total bytes sent\n\
             # TYPE turbine_bytes_out_total counter\n\
             turbine_bytes_out_total {}\n\n",
            self.total_bytes_out.load(Ordering::Relaxed),
        ));

        // Status code breakdown
        out.push_str(&format!(
            "# HELP turbine_http_status HTTP responses by status class\n\
             # TYPE turbine_http_status counter\n\
             turbine_http_status{{class=\"2xx\"}} {}\n\
             turbine_http_status{{class=\"3xx\"}} {}\n\
             turbine_http_status{{class=\"4xx\"}} {}\n\
             turbine_http_status{{class=\"5xx\"}} {}\n\n",
            self.total_2xx.load(Ordering::Relaxed),
            self.total_3xx.load(Ordering::Relaxed),
            self.total_4xx.load(Ordering::Relaxed),
            self.total_5xx.load(Ordering::Relaxed),
        ));

        // Cache
        out.push_str(&format!(
            "# HELP turbine_cache_hits_total Response cache hits\n\
             # TYPE turbine_cache_hits_total counter\n\
             turbine_cache_hits_total {}\n\n\
             # HELP turbine_cache_misses_total Response cache misses\n\
             # TYPE turbine_cache_misses_total counter\n\
             turbine_cache_misses_total {}\n\n",
            self.cache_hits.load(Ordering::Relaxed),
            self.cache_misses.load(Ordering::Relaxed),
        ));

        // Security
        out.push_str(&format!(
            "# HELP turbine_security_blocks_total Requests blocked by security\n\
             # TYPE turbine_security_blocks_total counter\n\
             turbine_security_blocks_total {}\n\n",
            self.security_blocks.load(Ordering::Relaxed),
        ));

        // Latency histogram
        out.push_str(
            "# HELP turbine_request_duration_ms Request latency in milliseconds\n\
             # TYPE turbine_request_duration_ms histogram\n",
        );
        out.push_str(&self.latency.prometheus_buckets("turbine_request_duration_ms"));
        out.push('\n');

        // Uptime
        out.push_str(&format!(
            "# HELP turbine_uptime_seconds Runtime uptime\n\
             # TYPE turbine_uptime_seconds gauge\n\
             turbine_uptime_seconds {}\n",
            self.uptime_secs(),
        ));

        out
    }

    /// Render a JSON status summary for the dashboard endpoint.
    pub fn status_json(&self, workers: usize) -> String {
        let total = self.total_requests.load(Ordering::Relaxed);
        let errors = self.total_errors.load(Ordering::Relaxed);

        // Collect top endpoints by request count
        let mut eps: Vec<_> = self
            .endpoints
            .iter()
            .map(|e| {
                let path = e.key().clone();
                let reqs = e.value().requests.load(Ordering::Relaxed);
                let errs = e.value().errors.load(Ordering::Relaxed);
                let mean = e.value().latency.mean_us();
                let p99 = e.value().latency.percentile(0.99);
                (path, reqs, errs, mean, p99)
            })
            .collect();
        eps.sort_by(|a, b| b.1.cmp(&a.1)); // sort by requests descending

        let mut json = String::with_capacity(2048);
        json.push_str("{\n");
        json.push_str(&format!("  \"uptime_seconds\": {},\n", self.uptime_secs()));
        json.push_str(&format!("  \"workers\": {},\n", workers));
        json.push_str(&format!("  \"total_requests\": {},\n", total));
        json.push_str(&format!("  \"total_errors\": {},\n", errors));
        json.push_str(&format!("  \"requests_per_second\": {:.1},\n", self.rps()));
        json.push_str(&format!(
            "  \"bytes_out\": {},\n",
            self.total_bytes_out.load(Ordering::Relaxed),
        ));
        json.push_str(&format!(
            "  \"latency_ms\": {{\n\
             \x20   \"mean\": {:.2},\n\
             \x20   \"p50\": {:.2},\n\
             \x20   \"p90\": {:.2},\n\
             \x20   \"p99\": {:.2}\n\
             \x20 }},\n",
            self.latency.mean_us() as f64 / 1000.0,
            self.latency.percentile(0.50) as f64 / 1000.0,
            self.latency.percentile(0.90) as f64 / 1000.0,
            self.latency.percentile(0.99) as f64 / 1000.0,
        ));
        json.push_str(&format!(
            "  \"cache\": {{\n\
             \x20   \"hits\": {},\n\
             \x20   \"misses\": {},\n\
             \x20   \"hit_ratio\": {:.3}\n\
             \x20 }},\n",
            self.cache_hits.load(Ordering::Relaxed),
            self.cache_misses.load(Ordering::Relaxed),
            self.cache_hit_ratio(),
        ));
        json.push_str(&format!(
            "  \"security\": {{\n\
             \x20   \"blocks\": {}\n\
             \x20 }},\n",
            self.security_blocks.load(Ordering::Relaxed),
        ));
        json.push_str(&format!(
            "  \"status_codes\": {{\n\
             \x20   \"2xx\": {},\n\
             \x20   \"3xx\": {},\n\
             \x20   \"4xx\": {},\n\
             \x20   \"5xx\": {}\n\
             \x20 }},\n",
            self.total_2xx.load(Ordering::Relaxed),
            self.total_3xx.load(Ordering::Relaxed),
            self.total_4xx.load(Ordering::Relaxed),
            self.total_5xx.load(Ordering::Relaxed),
        ));

        // Endpoints array
        json.push_str("  \"endpoints\": [\n");
        for (i, (path, reqs, errs, mean, p99)) in eps.iter().enumerate() {
            json.push_str(&format!(
                "    {{\n\
                 \x20     \"path\": \"{path}\",\n\
                 \x20     \"requests\": {reqs},\n\
                 \x20     \"errors\": {errs},\n\
                 \x20     \"mean_ms\": {mean:.2},\n\
                 \x20     \"p99_ms\": {p99:.2}\n\
                 \x20   }}",
                path = path,
                reqs = reqs,
                errs = errs,
                mean = *mean as f64 / 1000.0,
                p99 = *p99 as f64 / 1000.0,
            ));
            if i + 1 < eps.len() {
                json.push(',');
            }
            json.push('\n');
        }
        json.push_str("  ]\n}\n");
        json
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_count() {
        let m = MetricsCollector::new();
        m.record_request("/", 200, 500, 1024);
        m.record_request("/api", 200, 1200, 512);
        m.record_request("/api", 500, 50000, 128);

        assert_eq!(m.total_requests.load(Ordering::Relaxed), 3);
        assert_eq!(m.total_2xx.load(Ordering::Relaxed), 2);
        assert_eq!(m.total_5xx.load(Ordering::Relaxed), 1);
        assert_eq!(m.total_errors.load(Ordering::Relaxed), 1);
        assert_eq!(m.latency.count(), 3);
    }

    #[test]
    fn per_endpoint_tracking() {
        let m = MetricsCollector::new();
        m.record_request("/index.php", 200, 300, 2048);
        m.record_request("/index.php", 200, 400, 2048);
        m.record_request("/api.php", 200, 1000, 512);

        assert_eq!(m.endpoints.len(), 2);
        let idx = m.endpoints.get("/index.php").unwrap();
        assert_eq!(idx.requests.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn cache_hit_ratio() {
        let m = MetricsCollector::new();
        m.record_cache_hit();
        m.record_cache_hit();
        m.record_cache_hit();
        m.record_cache_miss();
        assert!((m.cache_hit_ratio() - 0.75).abs() < 0.001);
    }

    #[test]
    fn security_block_counter() {
        let m = MetricsCollector::new();
        m.record_security_block();
        m.record_security_block();
        assert_eq!(m.security_blocks.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn prometheus_output() {
        let m = MetricsCollector::new();
        m.record_request("/", 200, 500, 1024);
        let output = m.prometheus();
        assert!(output.contains("turbine_requests_total 1"));
        assert!(output.contains("turbine_request_duration_ms_count 1"));
        assert!(output.contains("turbine_uptime_seconds"));
    }

    #[test]
    fn status_json_output() {
        let m = MetricsCollector::new();
        m.record_request("/", 200, 500, 1024);
        let json = m.status_json(4);
        assert!(json.contains("\"total_requests\": 1"));
        assert!(json.contains("\"workers\": 4"));
        assert!(json.contains("\"endpoints\""));
    }
}
