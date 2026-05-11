use super::{Datetime, SurrealValue, Thing};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_datetime() -> Datetime {
    Datetime::default()
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct CodeChunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Thing>,

    pub file_path: String,
    pub content: String,

    #[serde(default)]
    pub language: Language,

    pub start_line: u32,
    pub end_line: u32,

    #[serde(default)]
    pub chunk_type: ChunkType,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Hierarchical breadcrumb path from AST (e.g. "impl:AuthService > fn:login")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_path: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,

    pub content_hash: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,

    /// Structural generation that produced this row. Missing generation means
    /// a legacy row that belongs to active generation 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,

    #[serde(default = "default_datetime")]
    pub indexed_at: Datetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChunkType {
    Function,
    Class,
    Struct,
    Module,
    Impl,
    #[default]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Java,
    Dart,
    C,
    Cpp,
    Swift,
    Kotlin,
    ObjectiveC,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct IndexStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Thing>,

    pub project_id: String,

    /// Canonical project root path used to derive project_id.
    /// Stored so list/stats can prove which workspace a project belongs to
    /// even after process restart, when the in-memory registry is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,

    pub status: IndexState,

    #[serde(default)]
    pub total_files: u32,

    #[serde(default)]
    pub indexed_files: u32,

    #[serde(default)]
    pub total_chunks: u32,

    #[serde(default)]
    pub total_symbols: u32,

    #[serde(default = "default_datetime")]
    pub started_at: Datetime,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<Datetime>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    #[serde(default)]
    pub failed_files: Vec<String>,

    #[serde(default)]
    pub failed_embeddings: u32,

    /// Tracks embedding model/pooling version for migration detection.
    /// When code changes pooling strategy, bump EMBEDDING_VERSION in manager.rs
    /// to trigger automatic re-embedding on next startup.
    #[serde(default)]
    pub embedding_version: u32,

    /// Structural readiness of canonical code facts (files/chunks/symbols/observed relations).
    #[serde(default)]
    pub structural_state: StructuralState,

    /// Semantic readiness of derived embedding-backed capabilities.
    #[serde(default)]
    pub semantic_state: SemanticState,

    /// Freshness of exported/materialized projections relative to canonical facts.
    /// Projection generation is not implemented yet, so the system currently stays stale.
    #[serde(default)]
    pub projection_state: ProjectionState,

    /// Monotonic counter for canonical structural facts (files/chunks/symbols/observed relations).
    #[serde(default)]
    pub structural_generation: u64,

    /// Monotonic counter for semantic enrichment aligned to structural generations.
    #[serde(default)]
    pub semantic_generation: u64,

    /// Capability-level readiness summaries for degraded serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<CapabilityReadiness>>,

    /// Serving generations for the different code-intelligence pipelines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving: Option<ServingGenerationMetadata>,

    /// Active include glob patterns for this index generation. Empty = no whitelist.
    #[serde(default)]
    pub include_patterns: Vec<String>,

    /// Active exclude glob patterns for this index generation. Empty = no extra excludes.
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    ProjectInfo,
    Bm25,
    Vector,
    Symbols,
    Graph,
    Semantic,
    Projection,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFreshness {
    Fresh,
    Stale,
    Partial,
    Missing,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct CapabilityReadiness {
    pub capability: CapabilityKind,

    pub freshness: CapabilityFreshness,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving_generation: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct ServingGenerationMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structural: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bm25: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbols: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<u64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexing: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IndexState {
    Indexing,
    EmbeddingPending,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexJobState {
    Queued,
    Running,
    Paused,
    Interrupted,
    Resumable,
    Completed,
    Failed,
    CancelRequested,
    Cancelled,
    Abandoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexJobPhase {
    Discover,
    Parse,
    Chunk,
    Symbols,
    Relations,
    Embed,
    EmbedEnqueue,
    Bm25,
    Finalize,
    Promote,
    Cleanup,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexJobReasonCode {
    CancelledByUser,
    InterruptedByShutdown,
    LostSameProcessTask,
    StorageError,
    ParseError,
    EmbeddingError,
    Bm25Error,
    Unknown,
    ActiveIndexRunning,
    ResumableInterruptedJob,
    LostOneShotIndexingTaskAfterRestart,
    CheckpointGenerationMissing,
    WorkspaceChangedSinceCheckpoint,
    StaleGeneration,
    IndexStorageCorrupt,
    IllegalStateTransition,
    ResumeTokenRequired,
    ForceRestartConfirmationRequired,
    CancellationRequested,
    CleanupRequested,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexJobRequestMode {
    StartNew,
    Resume,
    ForceRestart,
    Cancel,
    Cleanup,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct InvalidIndexJobTransition {
    pub from: IndexJobState,
    pub to: IndexJobState,
    pub reason_code: IndexJobReasonCode,
}

impl std::fmt::Display for InvalidIndexJobTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "illegal index job transition: {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl std::error::Error for InvalidIndexJobTransition {}

pub fn validate_index_job_transition(
    current: IndexJobState,
    proposed: IndexJobState,
) -> Result<(), InvalidIndexJobTransition> {
    use IndexJobState::*;

    let legal = matches!(
        (&current, &proposed),
        (Queued, Running)
            | (Running, Interrupted)
            | (Running, Completed)
            | (Running, Failed)
            | (Running, CancelRequested)
            | (Interrupted, Resumable)
            | (Resumable, Running)
            | (CancelRequested, Cancelled)
            | (Cancelled, Abandoned)
    );

    if legal {
        Ok(())
    } else {
        Err(InvalidIndexJobTransition {
            from: current,
            to: proposed,
            reason_code: IndexJobReasonCode::IllegalStateTransition,
        })
    }
}

pub fn validate_job_transition(
    from: IndexJobState,
    to: IndexJobState,
) -> Result<(), String> {
    validate_index_job_transition(from, to).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, JsonSchema, PartialEq, Eq)]
pub struct IndexJobError {
    pub code: IndexJobReasonCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, JsonSchema, PartialEq, Eq)]
pub struct IndexJobResumeState {
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_if_not_supported: Option<IndexJobReasonCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, JsonSchema, Default, PartialEq, Eq)]
pub struct IndexJobProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_files: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered_files: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed_files: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed_files: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedded_chunks: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, PartialEq, Eq)]
pub struct IndexJobRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Thing>,
    pub job_id: String,
    pub project_id: String,
    #[serde(default)]
    pub target_generation: u64,
    #[serde(default)]
    pub workspace_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_fingerprint: Option<String>,
    pub structural_generation: u64,
    pub state: IndexJobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_phase: Option<IndexJobPhase>,
    pub phase: IndexJobPhase,
    #[serde(default)]
    pub resume_token: String,
    #[serde(default = "default_datetime")]
    pub created_at: Datetime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<Datetime>,
    #[serde(default = "default_datetime")]
    pub updated_at: Datetime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<Datetime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<IndexJobError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<IndexJobResumeState>,
    #[serde(default)]
    pub completed_files_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_files_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<IndexJobReasonCode>,
    #[serde(default)]
    pub progress: IndexJobProgress,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, PartialEq, Eq)]
pub struct IndexFileCheckpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Thing>,
    #[serde(default)]
    pub job_id: String,
    pub project_id: String,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub relative_file_path: String,
    pub file_path: String,
    pub content_hash: String,
    pub checkpoint_generation: u64,
    pub phase: IndexJobPhase,
    #[serde(default)]
    pub completed: bool,
    #[serde(default = "default_datetime")]
    pub completed_at: Datetime,
    #[serde(default)]
    pub chunks_written: u64,
    #[serde(default)]
    pub symbols_written: u64,
    #[serde(default = "default_datetime")]
    pub updated_at: Datetime,
}

#[cfg(test)]
mod tests {
    use super::{
        validate_index_job_transition, validate_job_transition, CapabilityFreshness,
        CapabilityKind, CapabilityReadiness, IndexJobReasonCode, IndexJobState, IndexStatus,
    };

    #[test]
    fn index_job_state_machine_allows_legal_transitions() {
        let transitions = [
            (IndexJobState::Queued, IndexJobState::Running),
            (IndexJobState::Running, IndexJobState::Interrupted),
            (IndexJobState::Running, IndexJobState::Completed),
            (IndexJobState::Running, IndexJobState::Failed),
            (IndexJobState::Running, IndexJobState::CancelRequested),
            (IndexJobState::Interrupted, IndexJobState::Resumable),
            (IndexJobState::Resumable, IndexJobState::Running),
            (IndexJobState::CancelRequested, IndexJobState::Cancelled),
            (IndexJobState::Cancelled, IndexJobState::Abandoned),
        ];

        for (from, to) in transitions {
            assert!(
                validate_index_job_transition(from.clone(), to.clone()).is_ok(),
                "expected {from:?} -> {to:?} to be legal"
            );
        }
    }

    #[test]
    fn index_job_state_machine_rejects_invalid_transition() {
        let error = validate_job_transition(IndexJobState::Completed, IndexJobState::Running)
            .expect_err("completed jobs are terminal and cannot return to running");

        assert!(error.contains("Completed"));
        assert!(error.contains("Running"));
    }

    #[test]
    fn index_job_state_machine_rejects_invalid_transition_with_typed_error() {
        let error = validate_index_job_transition(IndexJobState::Completed, IndexJobState::Running)
            .expect_err("completed jobs are terminal and cannot return to running");

        assert_eq!(error.from, IndexJobState::Completed);
        assert_eq!(error.to, IndexJobState::Running);
        assert_eq!(
            error.reason_code,
            IndexJobReasonCode::IllegalStateTransition
        );
    }

    #[test]
    fn index_status_old_json_deserializes_without_filter_fields() {
        let json = r#"{
            "project_id": "proj-abc",
            "status": "completed",
            "structural_generation": 3,
            "semantic_generation": 3
        }"#;
        let status: IndexStatus = serde_json::from_str(json).expect("deserialize");
        assert!(status.include_patterns.is_empty());
        assert!(status.exclude_patterns.is_empty());
    }

    #[test]
    fn capability_readiness_model_defaults() {
        let json = r#"{
            "project_id": "proj-abc",
            "status": "completed",
            "structural_state": "pending",
            "semantic_state": "pending",
            "projection_state": "stale",
            "structural_generation": 3,
            "semantic_generation": 3,
            "include_patterns": [],
            "exclude_patterns": []
        }"#;

        let status: IndexStatus = serde_json::from_str(json).expect("deserialize legacy status");

        assert!(status.capabilities.is_none());
        assert!(status.serving.is_none());
    }

    #[test]
    fn capability_readiness_model_serializes_contract_values() {
        let readiness = CapabilityReadiness {
            capability: CapabilityKind::ProjectInfo,
            freshness: CapabilityFreshness::Stale,
            serving_generation: Some(7),
            reason: Some("waiting for refresh".to_string()),
            reason_code: Some("stale".to_string()),
        };

        let json = serde_json::to_string(&readiness).expect("serialize readiness");

        assert!(json.contains(r#""freshness":"stale""#));
        assert!(json.contains(r#""capability":"project_info""#));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuralState {
    #[default]
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticState {
    #[default]
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionState {
    #[default]
    Stale,
    Current,
}

impl std::fmt::Display for IndexState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexState::Indexing => write!(f, "indexing"),
            IndexState::EmbeddingPending => write!(f, "embedding_pending"),
            IndexState::Completed => write!(f, "completed"),
            IndexState::Failed => write!(f, "failed"),
        }
    }
}

impl std::fmt::Display for StructuralState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StructuralState::Pending => write!(f, "pending"),
            StructuralState::Ready => write!(f, "ready"),
            StructuralState::Failed => write!(f, "failed"),
        }
    }
}

impl std::fmt::Display for SemanticState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SemanticState::Pending => write!(f, "pending"),
            SemanticState::Ready => write!(f, "ready"),
            SemanticState::Failed => write!(f, "failed"),
        }
    }
}

impl std::fmt::Display for ProjectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionState::Stale => write!(f, "stale"),
            ProjectionState::Current => write!(f, "current"),
        }
    }
}

impl IndexStatus {
    pub fn new(project_id: String) -> Self {
        let mut status = Self {
            id: None,
            project_id,
            root_path: None,
            status: IndexState::Indexing,
            total_files: 0,
            indexed_files: 0,
            total_chunks: 0,
            total_symbols: 0,
            started_at: Datetime::default(),
            completed_at: None,
            error_message: None,
            failed_files: Vec::new(),
            failed_embeddings: 0,
            embedding_version: 0,
            structural_state: StructuralState::Pending,
            semantic_state: SemanticState::Pending,
            projection_state: ProjectionState::Stale,
            structural_generation: 0,
            semantic_generation: 0,
            capabilities: None,
            serving: None,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
        };
        status.refresh_lifecycle_states();
        status
    }

    pub fn refresh_lifecycle_states(&mut self) {
        match self.status {
            IndexState::Indexing => {
                self.structural_state = StructuralState::Pending;
                self.semantic_state = SemanticState::Pending;
                self.projection_state = ProjectionState::Stale;
            }
            IndexState::EmbeddingPending => {
                self.structural_state = StructuralState::Ready;
                self.semantic_state = SemanticState::Pending;
                self.projection_state = ProjectionState::Stale;
            }
            IndexState::Completed => {
                self.structural_state = StructuralState::Ready;
                self.semantic_state = SemanticState::Ready;
                if self.projection_state != ProjectionState::Current {
                    self.projection_state = ProjectionState::Stale;
                }
            }
            IndexState::Failed => {
                self.structural_state = StructuralState::Failed;
                self.semantic_state = SemanticState::Failed;
                self.projection_state = ProjectionState::Stale;
            }
        }
    }

    pub fn mark_projection_stale(&mut self) {
        self.projection_state = ProjectionState::Stale;
    }

    pub fn mark_projection_current(&mut self) {
        self.projection_state = if self.status == IndexState::Completed {
            ProjectionState::Current
        } else {
            ProjectionState::Stale
        };
    }

    pub fn mark_structural_generation_advanced(&mut self) {
        self.structural_generation = self.structural_generation.saturating_add(1);
        if self.semantic_generation > self.structural_generation {
            self.semantic_generation = self.structural_generation;
        }
        self.mark_projection_stale();
    }

    pub fn mark_semantic_generation_caught_up(&mut self) {
        self.semantic_generation = self.structural_generation;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ManifestEntry
// ─────────────────────────────────────────────────────────────────────────────

/// A single entry in the file manifest for a project.
/// Used to detect deleted files between indexing runs.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct ManifestEntry {
    pub project_id: String,
    pub file_path: String,
    #[serde(default = "default_datetime")]
    pub last_seen_at: Datetime,
}
