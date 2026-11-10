//! Bounded process-local latency accounting. No user or destination labels.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;

pub const LATENCY_BUCKET_UPPER_US: [u64; 6] = [100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000];

#[derive(Debug, Serialize)]
pub struct WaitSnapshot {
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub total_us: u64,
    pub max_us: u64,
    /// Disjoint buckets, followed by an unbounded overflow bucket.
    pub bucket_counts: [u64; 7],
}

pub struct WaitMetrics {
    completed: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
    total_us: AtomicU64,
    max_us: AtomicU64,
    buckets: [AtomicU64; 7],
}

fn add(counter: &AtomicU64, value: u64) {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        match counter.compare_exchange_weak(
            current,
            current.saturating_add(value),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

impl Default for WaitMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl WaitMetrics {
    pub const fn new() -> Self {
        Self {
            completed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            cancelled: AtomicU64::new(0),
            total_us: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            buckets: [const { AtomicU64::new(0) }; 7],
        }
    }

    pub fn start(&self) -> WaitTimer<'_> {
        WaitTimer {
            metrics: self,
            started: Instant::now(),
            completed: false,
        }
    }

    pub fn snapshot(&self) -> WaitSnapshot {
        WaitSnapshot {
            completed: self.completed.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            cancelled: self.cancelled.load(Ordering::Relaxed),
            total_us: self.total_us.load(Ordering::Relaxed),
            max_us: self.max_us.load(Ordering::Relaxed),
            bucket_counts: std::array::from_fn(|i| self.buckets[i].load(Ordering::Relaxed)),
        }
    }

    fn record(&self, elapsed: Duration, outcome: u8) {
        let micros = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        add(&self.total_us, micros);
        self.max_us.fetch_max(micros, Ordering::Relaxed);
        let bucket = LATENCY_BUCKET_UPPER_US.partition_point(|limit| micros > *limit);
        add(&self.buckets[bucket], 1);
        match outcome {
            0 => add(&self.completed, 1),
            1 => add(&self.failed, 1),
            _ => add(&self.cancelled, 1),
        }
    }
}

pub struct WaitTimer<'a> {
    metrics: &'a WaitMetrics,
    started: Instant,
    completed: bool,
}

impl WaitTimer<'_> {
    pub fn finish(mut self, success: bool) {
        self.completed = true;
        self.metrics
            .record(self.started.elapsed(), u8::from(!success));
    }
}

impl Drop for WaitTimer<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.metrics.record(self.started.elapsed(), 2);
        }
    }
}

pub static QUERY_POOL_WAIT: WaitMetrics = WaitMetrics::new();
pub static COORDINATION_POOL_WAIT: WaitMetrics = WaitMetrics::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_wait_accounting_keeps_fixed_buckets_and_cancelled_waits() {
        let metrics = WaitMetrics::new();
        metrics.record(Duration::from_micros(100), 0);
        metrics.record(Duration::from_micros(101), 1);
        metrics.record(Duration::from_secs(11), 2);
        drop(metrics.start());
        let snapshot = metrics.snapshot();
        assert_eq!(
            (snapshot.completed, snapshot.failed, snapshot.cancelled),
            (1, 1, 2)
        );
        assert_eq!(snapshot.bucket_counts.iter().sum::<u64>(), 4);
        assert_eq!(snapshot.bucket_counts[6], 1);
        assert!(snapshot.total_us >= 11_000_201);
        assert!(snapshot.max_us >= 11_000_000);
    }
}
