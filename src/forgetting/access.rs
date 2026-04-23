use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, watch};

use crate::forgetting::config::ForgettingConfig;
use crate::storage::MemoryStorage;
use crate::Result;

/// An access event queued for background persistence.
#[derive(Debug, Clone)]
pub struct AccessEvent {
    pub memory_id: String,
    pub accessed_at: DateTime<Utc>,
}

/// Lightweight sender used to record accesses without blocking callers.
#[derive(Clone)]
pub struct AccessTracker {
    sender: mpsc::Sender<AccessEvent>,
}

impl AccessTracker {
    /// Create a tracker bound to the given channel sender.
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

/// Background writer that persists access counts and timestamps.
pub struct AccessWriter {
    receiver: mpsc::Receiver<AccessEvent>,
    config: ForgettingConfig,
}

impl AccessWriter {
    /// Create a writer from a receiver and the active forgetting configuration.
    pub fn new(receiver: mpsc::Receiver<AccessEvent>, config: ForgettingConfig) -> Self {
        Self { receiver, config }
    }

    /// Run the background writer loop.
    /// Reads events from the channel, updates access metadata, and exits on shutdown.
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

/// Create the access tracking channel pair used by the forgetting subsystem.
///
/// The channel is intentionally bounded so access tracking remains fire-and-forget.
pub fn create_access_channel(config: ForgettingConfig) -> (AccessTracker, AccessWriter) {
    let (sender, receiver) = mpsc::channel(1024);
    let tracker = AccessTracker::new(sender);
    let writer = AccessWriter::new(receiver, config);
    (tracker, writer)
}
