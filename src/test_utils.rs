use std::sync::Arc;
use tempfile::TempDir;

use crate::config::{AppConfig, AppState};
use crate::forgetting::{create_access_channel, ForgettingConfig};
use crate::embedding::{
    AdaptiveEmbeddingQueue, EmbeddingConfig, EmbeddingMetrics, EmbeddingService, EmbeddingStore,
    ModelType,
};
use crate::search::{CodeSearchEngine, MemorySearchEngine};
use crate::storage::SurrealStorage;

pub struct TestContext {
    pub state: Arc<AppState>,
    pub _temp_dir: TempDir, // Kept to ensure directory lives as long as context
}

impl TestContext {
    pub async fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let db_path = temp_dir.path();

        // Initialize Storage
        let storage = Arc::new(
            SurrealStorage::new(db_path, 768)
                .await
                .expect("Failed to init storage"),
        );

        // Initialize Mock Embedding
        let embedding_config = EmbeddingConfig {
            model: ModelType::Mock,
            cache_size: 100,
            batch_size: 10,
            mrl_dim: None,

            cache_dir: None,
        };
        let embedding = Arc::new(EmbeddingService::new(embedding_config));
        embedding.start_loading();

        let mut attempts = 0;
        while !embedding.is_ready() {
            if attempts > 10 {
                panic!("Mock embedding service failed to start");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            attempts += 1;
        }

        let embedding_store =
            Arc::new(EmbeddingStore::new(db_path, "mock").expect("Failed to init embedding store"));
        let metrics = Arc::new(EmbeddingMetrics::new());
        let (queue_tx, _queue_rx) = tokio::sync::mpsc::channel(1000);
        let adaptive_queue = AdaptiveEmbeddingQueue::with_defaults(queue_tx, metrics);

        let config = AppConfig {
            data_dir: db_path.to_path_buf(),
            model: "mock".to_string(),
            cache_size: 100,
            batch_size: 10,
            timeout_ms: 5000,
            log_level: "debug".to_string(),
            // Tests use Mock model which is always instantly ready; any value works.
            model_load_timeout_ms: 30_000,
            embedding_queue_capacity: 256,
            embedding_batch_size: 8,
            index_batch_size: 20,
            index_debounce_ms: 2_000,
            manifest_diff_interval_mins: 10,
        };

        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        let state = Arc::new(AppState {
            config,
            storage,
            embedding,
            embedding_store,
            embedding_queue: adaptive_queue,
            progress: crate::config::IndexProgressTracker::new(),
            db_semaphore: Arc::new(tokio::sync::Semaphore::new(10)),
            code_search: Arc::new(CodeSearchEngine::new()),
            memory_search: Arc::new(MemorySearchEngine::new()),
            indexing_projects: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            shutdown_tx,
            index_pending: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            projection_registry: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            forgetting_config: ForgettingConfig::default(),
            access_tracker: {
                let (tracker, _writer) = create_access_channel(ForgettingConfig::default());
                tracker
            },
        });

        Self {
            state,
            _temp_dir: temp_dir,
        }
    }
}
