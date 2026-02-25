use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::metrics::EmbeddingMetrics;
use crate::embedding::EmbeddingRequest;
use crate::Result;

const DEFAULT_CAPACITY: usize = 64;
const HIGH_WATERMARK: f32 = 0.5;
const THROTTLE_DELAY_MS: u64 = 500;

pub struct AdaptiveQueueConfig {
    pub capacity: usize,
    pub high_watermark: f32,
    pub throttle_delay: Duration,
}

impl Default for AdaptiveQueueConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CAPACITY,
            high_watermark: HIGH_WATERMARK,
            throttle_delay: Duration::from_millis(THROTTLE_DELAY_MS),
        }
    }
}

pub struct AdaptiveEmbeddingQueue {
    sender: mpsc::Sender<EmbeddingRequest>,
    metrics: Arc<EmbeddingMetrics>,
    config: AdaptiveQueueConfig,
}

impl AdaptiveEmbeddingQueue {
    pub fn new(
        sender: mpsc::Sender<EmbeddingRequest>,
        metrics: Arc<EmbeddingMetrics>,
        config: AdaptiveQueueConfig,
    ) -> Self {
        Self {
            sender,
            metrics,
            config,
        }
    }

    pub fn with_defaults(
        sender: mpsc::Sender<EmbeddingRequest>,
        metrics: Arc<EmbeddingMetrics>,
    ) -> Self {
        Self::new(sender, metrics, AdaptiveQueueConfig::default())
    }

    pub async fn send(&self, req: EmbeddingRequest) -> Result<()> {
        let queue_depth = self.metrics.get_queue_depth();
        let utilization = queue_depth as f32 / self.config.capacity as f32;

        if utilization > self.config.high_watermark {
            tracing::debug!(
                utilization = %format!("{:.1}%", utilization * 100.0),
                queue_depth,
                "Queue pressure, throttling"
            );
            tokio::time::sleep(self.config.throttle_delay).await;
        }

        self.sender
            .send(req)
            .await
            .map_err(|_| crate::AppError::Internal("Embedding queue closed".into()))?;
        self.metrics.inc_queue();

        Ok(())
    }

    pub fn try_send(&self, req: EmbeddingRequest) -> Result<()> {
        self.metrics.inc_queue();
        self.sender.try_send(req).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                self.metrics.dec_queue();
                crate::AppError::Internal("Embedding queue full".into())
            }
            mpsc::error::TrySendError::Closed(_) => {
                self.metrics.dec_queue();
                crate::AppError::Internal("Embedding queue closed".into())
            }
        })
    }

    pub fn metrics(&self) -> &EmbeddingMetrics {
        &self.metrics
    }

    pub fn utilization(&self) -> f32 {
        self.metrics.get_queue_depth() as f32 / self.config.capacity as f32
    }

    pub fn is_healthy(&self) -> bool {
        self.utilization() < self.config.high_watermark
    }
}

impl Clone for AdaptiveEmbeddingQueue {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            metrics: Arc::clone(&self.metrics),
            config: AdaptiveQueueConfig {
                capacity: self.config.capacity,
                high_watermark: self.config.high_watermark,
                throttle_delay: self.config.throttle_delay,
            },
        }
    }
}
