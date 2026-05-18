use std::sync::Arc;
use tempfile::TempDir;

use crate::config::{AppConfig, AppState};
use crate::embedding::{
    AdaptiveEmbeddingQueue, EmbeddingConfig, EmbeddingMetrics, EmbeddingRequest, EmbeddingService,
    EmbeddingStore, ModelType,
};
use crate::forgetting::{create_access_channel, ForgettingConfig};
use crate::search::{CodeSearchEngine, MemorySearchEngine};
use crate::storage::SurrealStorage;

pub struct TestContext {
    pub state: Arc<AppState>,
    pub _temp_dir: TempDir, // Kept to ensure directory lives as long as context
    pub _embedding_rx: tokio::sync::mpsc::Receiver<EmbeddingRequest>,
}

impl TestContext {
    pub async fn new() -> Self {
        Self::new_with_registry_policy(crate::codebase::ProjectRegistryPolicy::default()).await
    }

    pub async fn new_with_registry_policy(
        project_registry_policy: crate::codebase::ProjectRegistryPolicy,
    ) -> Self {
        Self::new_with_registry_policy_and_code_index_config(
            project_registry_policy,
            crate::config::CodeIndexConfig::default(),
        )
        .await
    }

    pub async fn new_with_code_index_config(config: crate::config::CodeIndexConfig) -> Self {
        Self::new_with_registry_policy_and_code_index_config(
            crate::codebase::ProjectRegistryPolicy::default(),
            config,
        )
        .await
    }

    async fn new_with_registry_policy_and_code_index_config(
        project_registry_policy: crate::codebase::ProjectRegistryPolicy,
        code_index: crate::config::CodeIndexConfig,
    ) -> Self {
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
            allowed_project_roots: None,
            max_managed_projects: 5,
            db_semaphore_size: 24,
            code_index,
        };

        let metrics = Arc::new(EmbeddingMetrics::new());
        let (queue_tx, embedding_rx) = tokio::sync::mpsc::channel(config.embedding_queue_capacity);
        let adaptive_queue = AdaptiveEmbeddingQueue::new(
            queue_tx,
            metrics,
            crate::embedding::AdaptiveQueueConfig {
                capacity: config.embedding_queue_capacity,
                ..Default::default()
            },
        );

        let (shutdown_tx, _) = tokio::sync::watch::channel(false);
        let db_semaphore_size = config.db_semaphore_size;

        let state = Arc::new(AppState {
            config,
            storage,
            embedding,
            embedding_store,
            embedding_queue: adaptive_queue,
            progress: crate::config::IndexProgressTracker::new(),
            db_semaphore: Arc::new(tokio::sync::Semaphore::new(db_semaphore_size)),
            code_search: Arc::new(CodeSearchEngine::new()),
            memory_search: Arc::new(MemorySearchEngine::new()),
            indexing_projects: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            shutdown_tx,
            index_pending: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            project_registry: Arc::new(crate::codebase::ProjectRegistry::with_policy(
                project_registry_policy,
            )),
            session_bindings: Arc::new(crate::codebase::SessionBindingStore::new(1024)),
            projection_registry: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            forgetting_config: ForgettingConfig::default(),
            access_tracker: {
                let (tracker, _writer) = create_access_channel(ForgettingConfig::default());
                tracker
            },
            community_cache: moka::future::Cache::builder()
                .max_capacity(1)
                .time_to_live(std::time::Duration::from_secs(300))
                .build(),
            metrics: crate::metrics::MetricsRecorder::disabled(),
        });

        Self {
            state,
            _temp_dir: temp_dir,
            _embedding_rx: embedding_rx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_context_app_state_owns_project_registry() {
        let context = TestContext::new().await;

        assert!(context.state.project_registry.is_empty().await);
    }
}
