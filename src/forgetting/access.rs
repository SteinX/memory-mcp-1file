use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, watch};

use crate::forgetting::config::ForgettingConfig;
use crate::storage::MemoryStorage;
use crate::Result;

#[derive(Debug, Clone)]
pub struct AccessEvent {
    pub memory_id: String,
    pub accessed_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AccessTracker {
    sender: mpsc::Sender<AccessEvent>,
}

impl AccessTracker {
    pub fn new(sender: mpsc::Sender<AccessEvent>) -> Self {
        Self { sender }
    }

    /// Fire-and-forget: send access event without blocking.
    /// Silently drops if channel is full or closed.
    pub fn track(&self, memory_id: impl Into<String>) {
        let event = AccessEvent {
            memory_id: memory_id.into(),
            accessed_at: Utc::now(),
        };
        let _ = self.sender.try_send(event);
    }
}

pub struct AccessWriter {
    receiver: mpsc::Receiver<AccessEvent>,
    config: ForgettingConfig,
}

impl AccessWriter {
    pub fn new(receiver: mpsc::Receiver<AccessEvent>, config: ForgettingConfig) -> Self {
        Self { receiver, config }
    }

    /// Run the background writer loop.
    /// Reads events from channel, writes access_count++ and last_accessed_at to DB.
    /// Stops when shutdown signal received.
    pub async fn run(
        mut self,
        db: Arc<dyn MemoryStorage + Send + Sync>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let _ = &self.config;

        loop {
            tokio::select! {
                Some(event) = self.receiver.recv() => {
                    let _ = self.write_access(&db, event).await;
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }

    async fn write_access(
        &self,
        db: &Arc<dyn MemoryStorage + Send + Sync>,
        event: AccessEvent,
    ) -> Result<()> {
        db.record_memory_access(event.memory_id, event.accessed_at)
            .await
    }
}

pub fn create_access_channel(config: ForgettingConfig) -> (AccessTracker, AccessWriter) {
    let (sender, receiver) = mpsc::channel(1024);
    let tracker = AccessTracker::new(sender);
    let writer = AccessWriter::new(receiver, config);
    (tracker, writer)
}
