use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use tokio::sync::{watch, RwLock, Semaphore};

use crate::embedding::{AdaptiveEmbeddingQueue, EmbeddingService, EmbeddingStore};
use crate::search::CodeSearchEngine;
use crate::storage::SurrealStorage;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub model: String,
    pub cache_size: usize,
    pub batch_size: usize,
    pub timeout_ms: u64,
    pub log_level: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            data_dir: dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("memory-mcp"),
            model: "qwen3".to_string(),
            cache_size: 1000,
            batch_size: 8,
            timeout_ms: 30000,
            log_level: "info".to_string(),
        }
    }
}

pub struct IndexMonitor {
    pub total_files: AtomicU32,
    pub indexed_files: AtomicU32,
    pub current_file: std::sync::RwLock<String>,
}

impl Default for IndexMonitor {
    fn default() -> Self {
        Self {
            total_files: AtomicU32::new(0),
            indexed_files: AtomicU32::new(0),
            current_file: std::sync::RwLock::new(String::new()),
        }
    }
}

pub struct IndexProgressTracker {
    projects: RwLock<HashMap<String, Arc<IndexMonitor>>>,
}

impl IndexProgressTracker {
    pub fn new() -> Self {
        Self {
            projects: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_or_create(&self, project_id: &str) -> Arc<IndexMonitor> {
        {
            let projects = self.projects.read().await;
            if let Some(monitor) = projects.get(project_id) {
                return monitor.clone();
            }
        }
        let mut projects = self.projects.write().await;
        projects
            .entry(project_id.to_string())
            .or_insert_with(|| Arc::new(IndexMonitor::default()))
            .clone()
    }

    pub async fn get(&self, project_id: &str) -> Option<Arc<IndexMonitor>> {
        self.projects.read().await.get(project_id).cloned()
    }

    pub async fn remove(&self, project_id: &str) {
        self.projects.write().await.remove(project_id);
    }
}

impl Default for IndexProgressTracker {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AppState {
    pub config: AppConfig,
    pub storage: Arc<SurrealStorage>,
    pub embedding: Arc<EmbeddingService>,
    pub embedding_store: Arc<EmbeddingStore>,
    pub embedding_queue: AdaptiveEmbeddingQueue,
    pub progress: IndexProgressTracker,
    /// Semaphore to limit concurrent DB operations (prevents SurrealKV channel exhaustion)
    pub db_semaphore: Arc<Semaphore>,
    /// In-memory BM25 index for code chunks (replaces broken SurrealDB FTS)
    pub code_search: Arc<CodeSearchEngine>,
    /// Atomic lock: set of project IDs currently being (re-)indexed.
    /// Insert returns `false` if the ID is already present → concurrent request is rejected.
    /// Removed when indexing finishes (success or failure).
    pub indexing_projects: Arc<std::sync::Mutex<HashSet<String>>>,
    /// Shutdown signal sender. Send `true` to request graceful shutdown of background loops.
    pub shutdown_tx: watch::Sender<bool>,
}

impl AppState {
    /// Returns a new receiver subscribed to the shutdown channel.
    /// Background tasks should select on `rx.changed()` and exit when `*rx.borrow() == true`.
    pub fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }
}
