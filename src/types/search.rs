use serde::{Deserialize, Serialize};

use super::code::{ChunkType, Language};
use super::memory::MemoryType;
use super::SurrealValue;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct SearchResult {
    pub id: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
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
