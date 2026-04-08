//! Request metrics collection — lock-free, zero-allocation hot path.
//!
//! Uses atomics for global counters and DashMap for per-endpoint histograms.
//! Designed for < 50ns overhead per `record()` call.

mod collector;
mod histogram;

pub use collector::MetricsCollector;
pub use histogram::LatencyHistogram;
