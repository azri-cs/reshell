use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cumulative telemetry counters for rsh operations.
pub struct Metrics {
    pub total_execs: AtomicU64,
    pub total_failures: AtomicU64,
    pub successful_recoveries: AtomicU64,
    pub auto_retries_r22: AtomicU64,
    pub auto_retries_r25: AtomicU64,
    pub false_positives: AtomicU64,
    pub total_output_bytes_raw: AtomicU64,
    pub total_output_bytes_compacted: AtomicU64,
    pub total_recovery_time_ms: AtomicU64,
    pub recovery_count: AtomicU64,
}

impl Metrics {
    pub const fn new() -> Self {
        Self {
            total_execs: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            successful_recoveries: AtomicU64::new(0),
            auto_retries_r22: AtomicU64::new(0),
            auto_retries_r25: AtomicU64::new(0),
            false_positives: AtomicU64::new(0),
            total_output_bytes_raw: AtomicU64::new(0),
            total_output_bytes_compacted: AtomicU64::new(0),
            total_recovery_time_ms: AtomicU64::new(0),
            recovery_count: AtomicU64::new(0),
        }
    }

    pub fn record_exec(&self) {
        self.total_execs.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.total_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_recovery(&self, duration_ms: u64, successful: bool) {
        self.total_recovery_time_ms
            .fetch_add(duration_ms, Ordering::Relaxed);
        self.recovery_count.fetch_add(1, Ordering::Relaxed);
        if successful {
            self.successful_recoveries.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_auto_retry_r22(&self) {
        self.auto_retries_r22.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_auto_retry_r25(&self) {
        self.auto_retries_r25.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_output_savings(&self, raw_bytes: u64, compacted_bytes: u64) {
        self.total_output_bytes_raw
            .fetch_add(raw_bytes, Ordering::Relaxed);
        self.total_output_bytes_compacted
            .fetch_add(compacted_bytes, Ordering::Relaxed);
    }

    pub fn record_false_positive(&self) {
        self.false_positives.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Serialize)]
pub struct MetricsSnapshot {
    pub total_execs: u64,
    pub total_failures: u64,
    pub recovery_rate: f64,
    pub false_positive_rate: f64,
    pub context_savings_pct: f64,
    pub avg_recovery_time_ms: f64,
    pub auto_retries_r22: u64,
    pub auto_retries_r25: u64,
}

impl Metrics {
    pub fn snapshot(&self) -> MetricsSnapshot {
        let total = self.total_execs.load(Ordering::Relaxed);
        let failures = self.total_failures.load(Ordering::Relaxed);
        let recoveries = self.successful_recoveries.load(Ordering::Relaxed);
        let recovery_count = self.recovery_count.load(Ordering::Relaxed);
        let raw = self.total_output_bytes_raw.load(Ordering::Relaxed);
        let compacted = self.total_output_bytes_compacted.load(Ordering::Relaxed);

        let recovery_rate = if failures > 0 {
            recoveries as f64 / failures as f64
        } else {
            0.0
        };

        let false_positive_rate = if failures > 0 {
            self.false_positives.load(Ordering::Relaxed) as f64 / failures as f64
        } else {
            0.0
        };

        let context_savings_pct = if raw > 0 {
            ((raw - compacted) as f64 / raw as f64) * 100.0
        } else {
            0.0
        };

        let avg_recovery_time_ms = if recovery_count > 0 {
            self.total_recovery_time_ms.load(Ordering::Relaxed) as f64 / recovery_count as f64
        } else {
            0.0
        };

        MetricsSnapshot {
            total_execs: total,
            total_failures: failures,
            recovery_rate,
            false_positive_rate,
            context_savings_pct,
            avg_recovery_time_ms,
            auto_retries_r22: self.auto_retries_r22.load(Ordering::Relaxed),
            auto_retries_r25: self.auto_retries_r25.load(Ordering::Relaxed),
        }
    }
}
