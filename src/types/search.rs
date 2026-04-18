use serde::{Deserialize, Serialize};

use super::code::{ChunkType, Language};
use super::memory::MemoryType;
use super::Datetime;
use super::SurrealValue;

fn default_importance_score() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<MemoryType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<Datetime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_after: Option<Datetime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_before: Option<Datetime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_after: Option<Datetime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_before: Option<Datetime>,
}

impl MemoryQuery {
    pub fn uses_metadata_post_filter(&self) -> bool {
        self.metadata_filter.is_some()
    }

    pub fn is_unfiltered(&self) -> bool {
        self.user_id.is_none()
            && self.agent_id.is_none()
            && self.run_id.is_none()
            && self.namespace.is_none()
            && self.memory_type.is_none()
            && self.metadata_filter.is_none()
            && self.valid_at.is_none()
            && self.event_after.is_none()
            && self.event_before.is_none()
            && self.ingestion_after.is_none()
            && self.ingestion_before.is_none()
    }

    pub fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "userId": self.user_id,
            "agentId": self.agent_id,
            "runId": self.run_id,
            "namespace": self.namespace,
            "memoryType": self.memory_type.as_ref().map(|t| match t {
                MemoryType::Episodic => "episodic",
                MemoryType::Semantic => "semantic",
                MemoryType::Procedural => "procedural",
            }),
            "metadataFilter": self.metadata_filter,
            "validAt": self.valid_at,
            "eventAfter": self.event_after,
            "eventBefore": self.event_before,
            "ingestionAfter": self.ingestion_after,
            "ingestionBefore": self.ingestion_before,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct SearchResult {
    pub id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub memory_type: MemoryType,
    pub score: f32,
    #[serde(default = "default_importance_score")]
    pub importance_score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<Datetime>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidation_reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consolidation_trace: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_lineage: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention_summary: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_summary: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
    pub memories: Vec<ScoredMemory>,
    pub query: String,
    pub subgraph_nodes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredMemory {
    pub id: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub score: f32,
    pub vector_score: f32,
    pub bm25_score: f32,
    pub ppr_score: f32,
    pub importance_score: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<Datetime>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidation_reason: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consolidation_trace: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_lineage: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention_summary: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_summary: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSearchResult {
    pub results: Vec<ScoredCodeChunk>,
    pub count: usize,
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct ScoredCodeChunk {
    pub id: String,
    pub file_path: String,
    pub content: String,
    pub language: Language,
    pub start_line: u32,
    pub end_line: u32,
    pub chunk_type: ChunkType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Hierarchical breadcrumb path from AST (e.g. "impl:AuthService > fn:login")
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_path: Option<String>,
    pub score: f32,
}
