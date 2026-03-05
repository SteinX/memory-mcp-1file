use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[derive(Debug, Default)]
pub struct EmbeddingMetrics {
    pub queue_depth: AtomicUsize,
    pub processed_total: AtomicU64,
    pub failed_total: AtomicU64,
}

impl EmbeddingMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn inc_queue(&self) {
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
    }

    pub fn dec_queue(&self) {
        self.queue_depth.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn inc_processed(&self, count: u64) {
        self.processed_total.fetch_add(count, Ordering::Relaxed);
    }

    pub fn inc_failed(&self, count: u64) {
        self.failed_total.fetch_add(count, Ordering::Relaxed);
    }

    pub fn get_queue_depth(&self) -> usize {
        self.queue_depth.load(Ordering::Relaxed)
    }

    pub fn get_failed_total(&self) -> u64 {
        self.failed_total.load(Ordering::Relaxed)
    }
}
