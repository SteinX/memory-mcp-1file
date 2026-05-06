use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize};
use std::sync::Arc;
use std::str::FromStr;

use tokio::sync::{watch, RwLock, Semaphore};

use crate::codebase::{ProjectRegistry, SessionBindingStore};
use crate::embedding::{AdaptiveEmbeddingQueue, EmbeddingService, EmbeddingStore};
use crate::forgetting::access::AccessTracker;
use crate::forgetting::config::ForgettingConfig;
use crate::search::{CodeSearchEngine, MemorySearchEngine};
use crate::storage::SurrealStorage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIndexPipelineMode {
    Legacy,
    Staged,
}

impl Default for CodeIndexPipelineMode {
    fn default() -> Self {
        // Rollout stays rollback-safe until benchmark validation flips it.
        Self::Legacy
    }
}

impl FromStr for CodeIndexPipelineMode {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "legacy" => Ok(Self::Legacy),
            "staged" => Ok(Self::Staged),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIndexBm25Mode {
    FinalRebuild,
    Incremental,
}

impl Default for CodeIndexBm25Mode {
    fn default() -> Self {
        Self::FinalRebuild
    }
}

impl FromStr for CodeIndexBm25Mode {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "final_rebuild" => Ok(Self::FinalRebuild),
            "incremental" => Ok(Self::Incremental),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodeIndexConfig {
    pub pipeline_mode: CodeIndexPipelineMode,
    pub read_workers: usize,
    pub parse_workers: usize,
    pub commit_batch_size: usize,
    pub max_inflight_files: usize,
    pub max_inflight_bytes: usize,
    pub status_flush_ms: u64,
    pub relation_batch_size: usize,
    pub bm25_mode: CodeIndexBm25Mode,
}

impl CodeIndexConfig {
    fn parse_env<T, F>(lookup: &F, key: &str) -> Option<T>
    where
        T: FromStr,
        F: Fn(&str) -> Option<String>,
    {
        lookup(key)?.parse().ok()
    }

    fn parse_env_min<T, F>(lookup: &F, key: &str, min: T, default: T) -> T
    where
        T: FromStr + Ord + Copy,
        F: Fn(&str) -> Option<String>,
    {
        Self::parse_env::<T, F>(lookup, key)
            .map(|value| value.max(min))
            .unwrap_or(default)
    }

    pub fn from_env() -> Self {
        Self::from_env_with(|key| std::env::var(key).ok())
    }

    pub fn from_env_with<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let defaults = Self::default();

        Self {
            pipeline_mode: Self::parse_env::<CodeIndexPipelineMode, _>(&lookup, "CODE_INDEX_PIPELINE_MODE")
                .unwrap_or(defaults.pipeline_mode),
            read_workers: Self::parse_env_min(&lookup, "CODE_INDEX_READ_WORKERS", 1, defaults.read_workers),
            // Keep parse workers bounded for Docker-sized machines.
            parse_workers: Self::parse_env_min(&lookup, "CODE_INDEX_PARSE_WORKERS", 2, defaults.parse_workers),
            commit_batch_size: Self::parse_env_min(&lookup, "CODE_INDEX_COMMIT_BATCH_SIZE", 1, defaults.commit_batch_size),
            max_inflight_files: Self::parse_env_min(&lookup, "CODE_INDEX_MAX_INFLIGHT_FILES", 1, defaults.max_inflight_files),
            max_inflight_bytes: Self::parse_env_min(&lookup, "CODE_INDEX_MAX_INFLIGHT_BYTES", 1, defaults.max_inflight_bytes),
            status_flush_ms: Self::parse_env_min(&lookup, "CODE_INDEX_STATUS_FLUSH_MS", 1, defaults.status_flush_ms),
            relation_batch_size: Self::parse_env_min(&lookup, "CODE_INDEX_RELATION_BATCH_SIZE", 1, defaults.relation_batch_size),
            bm25_mode: Self::parse_env::<CodeIndexBm25Mode, _>(&lookup, "CODE_INDEX_BM25_MODE")
                .unwrap_or(defaults.bm25_mode),
        }
    }
}

impl Default for CodeIndexConfig {
    fn default() -> Self {
        Self {
            // Conservative defaults for bounded Docker-sized deployments.
            pipeline_mode: CodeIndexPipelineMode::default(),
            read_workers: 2,
            parse_workers: std::cmp::max(2, std::cmp::min(num_cpus::get() / 2, 4)),
            commit_batch_size: 100,
            max_inflight_files: 64,
            max_inflight_bytes: 128 * 1024 * 1024,
            status_flush_ms: 1000,
            relation_batch_size: 5000,
            bm25_mode: CodeIndexBm25Mode::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub model: String,
    pub cache_size: usize,
    pub batch_size: usize,
    pub timeout_ms: u64,
    pub log_level: String,
    /// Maximum time (ms) a tool call will block waiting for the embedding model
    /// to finish loading. This allows the MCP session to survive the first-time
    /// model download on a fresh machine without the client seeing a timeout.
    /// Default: 600_000 ms (10 minutes) — longer than any realistic download.
    pub model_load_timeout_ms: u64,

    // ── Embedding pipeline ────────────────────────────────────────────────────
    /// Capacity of the `tokio::sync::mpsc` channel used to buffer embedding
    /// requests before the worker picks them up. Higher values reduce
    /// back-pressure during ingestion bursts.
    /// Default: 256.
    pub embedding_queue_capacity: usize,
    /// Minimum number of pending embedding requests before the worker flushes
    /// a batch eagerly (without waiting for the deadline).
    /// Default: 8.
    pub embedding_batch_size: usize,

    // ── Codebase IndexWorker ──────────────────────────────────────────────────
    /// Maximum number of file-system events to accumulate before forcing an
    /// incremental-index flush. Larger values reduce DB round-trips on heavy
    /// save storms.
    /// Default: 20.
    pub index_batch_size: usize,
    /// Debounce window (ms): how long the IndexWorker waits for more jobs
    /// before flushing an incomplete batch.
    /// Default: 2 000 ms.
    pub index_debounce_ms: u64,
    /// How often (minutes) the periodic manifest-diff task runs to catch
    /// changes missed by the file-system watcher.
    /// Default: 10 minutes.
    pub manifest_diff_interval_mins: u64,
    /// Optional allowlist for project roots visible to code-intelligence registration paths.
    /// When configured, `index_project` and startup registration must stay inside one of these roots.
    pub allowed_project_roots: Option<Vec<PathBuf>>,
    /// Maximum number of managed projects in the in-memory lifecycle registry.
    /// Default: 5.
    pub max_managed_projects: usize,
    /// Conservative code-index pipeline defaults and rollout switches.
    pub code_index: CodeIndexConfig,
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
            model_load_timeout_ms: 600_000,
            embedding_queue_capacity: 256,
            embedding_batch_size: 8,
            index_batch_size: 20,
            index_debounce_ms: 2_000,
            manifest_diff_interval_mins: 10,
            allowed_project_roots: None,
            max_managed_projects: 5,
            code_index: CodeIndexConfig::default(),
        }
    }
}

pub struct IndexMonitor {
    pub total_files: AtomicU32,
    pub indexed_files: AtomicU32,
    pub current_file: std::sync::RwLock<String>,
    /// Same-process operation id for a manually queued one-shot full index.
    pub operation_id: std::sync::RwLock<Option<String>>,
    /// One-shot task state: queued | running | completed | failed | unknown_after_restart.
    pub task_state: std::sync::RwLock<String>,
    /// Last task-level error for the one-shot index runner.
    pub last_error: std::sync::RwLock<Option<String>>,
}

impl Default for IndexMonitor {
    fn default() -> Self {
        Self {
            total_files: AtomicU32::new(0),
            indexed_files: AtomicU32::new(0),
            current_file: std::sync::RwLock::new(String::new()),
            operation_id: std::sync::RwLock::new(None),
            task_state: std::sync::RwLock::new("idle".to_string()),
            last_error: std::sync::RwLock::new(None),
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
    pub forgetting_config: ForgettingConfig,
    pub access_tracker: AccessTracker,
    pub storage: Arc<SurrealStorage>,
    pub embedding: Arc<EmbeddingService>,
    pub embedding_store: Arc<EmbeddingStore>,
    pub embedding_queue: AdaptiveEmbeddingQueue,
    pub progress: IndexProgressTracker,
    /// Semaphore to limit concurrent DB operations (prevents SurrealKV channel exhaustion)
    pub db_semaphore: Arc<Semaphore>,
    /// In-memory BM25 index for code chunks (replaces broken SurrealDB FTS)
    pub code_search: Arc<CodeSearchEngine>,
    /// In-memory lexical index for memories. Warmed from DB and kept in sync
    /// by memory CRUD/invalidation flows to avoid rebuilding BM25 per request.
    pub memory_search: Arc<MemorySearchEngine>,
    /// Atomic lock: set of project IDs currently being (re-)indexed.
    /// Insert returns `false` if the ID is already present → concurrent request is rejected.
    /// Removed when indexing finishes (success or failure).
    pub indexing_projects: Arc<std::sync::Mutex<HashSet<String>>>,
    /// Shutdown signal sender. Send `true` to request graceful shutdown of background loops.
    pub shutdown_tx: watch::Sender<bool>,
    /// Per-project pending job counters shared with [`IndexJobSender`] instances.
    ///
    /// Key = project_id.  Each `Arc<AtomicUsize>` is also held inside the
    /// corresponding `IndexJobSender` so both sides share the same counter
    /// without `AppState` needing to import the `codebase` crate.
    pub index_pending: Arc<RwLock<HashMap<String, Arc<AtomicUsize>>>>,
    /// Per-project code-intelligence lifecycle registry.
    ///
    /// T10 makes `AppState` the owner of the registry so later startup and
    /// manual-indexing tasks can route through one lifecycle coordinator without
    /// changing search breadth or public MCP tool schemas in this step.
    pub project_registry: Arc<ProjectRegistry>,
    /// In-memory HTTP/MCP session to project binding store.
    ///
    /// Key = `mcp-session-id`. Value = optional project binding state plus update timestamp.
    /// This is intentionally process-local and does not persist across restarts.
    pub session_bindings: Arc<SessionBindingStore>,
    /// Ephemeral in-process projection registry.
    ///
    /// Key = opaque locator string. Value = latest on-demand export-only
    /// projection document for same-process re-read. Entries are intentionally
    /// non-persistent and may disappear on restart or be replaced after later
    /// rebuilds.
    pub projection_registry: Arc<RwLock<HashMap<String, crate::types::ExportedProjectProjection>>>,
}

impl AppState {
    /// Returns a new receiver subscribed to the shutdown channel.
    /// Background tasks should select on `rx.changed()` and exit when `*rx.borrow() == true`.
    pub fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_index_defaults_are_conservative() {
        let config = CodeIndexConfig::default();

        assert_eq!(config.pipeline_mode, CodeIndexPipelineMode::Legacy);
        assert_eq!(config.read_workers, 2);
        assert_eq!(
            config.parse_workers,
            std::cmp::max(2, std::cmp::min(num_cpus::get() / 2, 4))
        );
        assert_eq!(config.commit_batch_size, 100);
        assert_eq!(config.max_inflight_files, 64);
        assert_eq!(config.max_inflight_bytes, 128 * 1024 * 1024);
        assert_eq!(config.status_flush_ms, 1000);
        assert_eq!(config.relation_batch_size, 5000);
        assert_eq!(config.bm25_mode, CodeIndexBm25Mode::FinalRebuild);
    }

    #[test]
    fn code_index_env_overrides_apply_and_invalid_values_fallback() {
        let config = CodeIndexConfig::from_env_with(|key| match key {
            "CODE_INDEX_PIPELINE_MODE" => Some("staged".to_string()),
            "CODE_INDEX_READ_WORKERS" => Some("4".to_string()),
            "CODE_INDEX_PARSE_WORKERS" => Some("0".to_string()),
            "CODE_INDEX_COMMIT_BATCH_SIZE" => Some("256".to_string()),
            "CODE_INDEX_MAX_INFLIGHT_FILES" => Some("128".to_string()),
            "CODE_INDEX_MAX_INFLIGHT_BYTES" => Some("not-a-number".to_string()),
            "CODE_INDEX_STATUS_FLUSH_MS" => Some("250".to_string()),
            "CODE_INDEX_RELATION_BATCH_SIZE" => Some("9000".to_string()),
            "CODE_INDEX_BM25_MODE" => Some("incremental".to_string()),
            _ => None,
        });

        assert_eq!(config.pipeline_mode, CodeIndexPipelineMode::Staged);
        assert_eq!(config.read_workers, 4);
        assert_eq!(config.parse_workers, 2);
        assert_eq!(config.commit_batch_size, 256);
        assert_eq!(config.max_inflight_files, 128);
        assert_eq!(config.max_inflight_bytes, 128 * 1024 * 1024);
        assert_eq!(config.status_flush_ms, 250);
        assert_eq!(config.relation_batch_size, 9000);
        assert_eq!(config.bm25_mode, CodeIndexBm25Mode::Incremental);
    }

    #[test]
    fn code_index_legacy_mode_is_accepted() {
        let config = CodeIndexConfig::from_env_with(|key| match key {
            "CODE_INDEX_PIPELINE_MODE" => Some("legacy".to_string()),
            _ => None,
        });

        assert_eq!(config.pipeline_mode, CodeIndexPipelineMode::Legacy);
    }
}
