use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::{Datetime, MemoryQuery, MemoryType};

pub fn normalize_project_id(project_id: Option<String>) -> Option<String> {
    project_id.and_then(|project_id| {
        let trimmed = project_id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Schema override: serde_json::Value generates boolean `true` via schemars,
/// which Claude Code's Zod validator rejects. We emit `{}` (empty object schema) instead.
/// See: https://github.com/anthropics/claude-code/issues/17742
fn any_value_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({})
}

fn parse_optional_datetime(value: Option<&str>, field: &str) -> anyhow::Result<Option<Datetime>> {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => {
            let ts: chrono::DateTime<chrono::Utc> = value
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid {} format. Use ISO 8601", field))?;
            Ok(Some(Datetime::from(ts)))
        }
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct StoreMemoryParams {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "any_value_schema")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct GetMemoryParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct UpdateMemoryParams {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "any_value_schema")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct DeleteMemoryParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(title = "")]
pub struct ListMemoriesParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "any_value_schema")]
    pub metadata_filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(title = "")]
pub struct SearchParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// vector|bm25 (default: vector)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Minimum score threshold in [0.0, 1.0] applied after ranking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "any_value_schema")]
    pub metadata_filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(title = "")]
pub struct RecallParams {
    pub query: String,
    /// Default: 10
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Tune RRF: vector channel (default: 0.50)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_weight: Option<f32>,
    /// Tune RRF: BM25 channel (default: 0.20)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_weight: Option<f32>,
    /// Tune RRF: graph PPR channel (default: 0.30)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ppr_weight: Option<f32>,
    /// Minimum fused score threshold in [0.0, 1.0] applied after fusion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "any_value_schema")]
    pub metadata_filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(title = "")]
pub struct RecallCodeParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Default: 10
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// vector|hybrid (default: hybrid)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Vector weight (default: 0.50)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_weight: Option<f32>,
    /// BM25 weight (default: 0.20)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_weight: Option<f32>,
    /// Graph PPR weight (default: 0.30)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ppr_weight: Option<f32>,
    /// Path prefix filter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    /// Language filter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Chunk type filter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct KnowledgeGraphParams {
    /// create_entity|create_relation|get_related|detect_communities
    pub action: String,
    /// For: create_entity (required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// For: create_entity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// For: create_entity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// For: create_entity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// For: create_relation (required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_entity: Option<String>,
    /// For: create_relation (required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_entity: Option<String>,
    /// For: create_relation (required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_type: Option<String>,
    /// For: create_relation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    /// For: get_related (required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    /// For: get_related (default: 1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    /// For: get_related (in|out|both)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct GetValidParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "any_value_schema")]
    pub metadata_filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct GetValidAtParams {
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "any_value_schema")]
    pub metadata_filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl SearchParams {
    pub fn to_memory_query(&self) -> anyhow::Result<MemoryQuery> {
        Ok(MemoryQuery {
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            namespace: self.namespace.clone(),
            memory_type: parse_memory_type(self.memory_type.as_deref())?,
            metadata_filter: self.metadata_filter.clone(),
            valid_at: parse_optional_datetime(self.valid_at.as_deref(), "valid_at")?,
            event_after: parse_optional_datetime(self.event_after.as_deref(), "event_after")?,
            event_before: parse_optional_datetime(self.event_before.as_deref(), "event_before")?,
            ingestion_after: parse_optional_datetime(
                self.ingestion_after.as_deref(),
                "ingestion_after",
            )?,
            ingestion_before: parse_optional_datetime(
                self.ingestion_before.as_deref(),
                "ingestion_before",
            )?,
        })
    }
}

impl RecallParams {
    pub fn to_memory_query(&self) -> anyhow::Result<MemoryQuery> {
        Ok(MemoryQuery {
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            namespace: self.namespace.clone(),
            memory_type: parse_memory_type(self.memory_type.as_deref())?,
            metadata_filter: self.metadata_filter.clone(),
            valid_at: parse_optional_datetime(self.valid_at.as_deref(), "valid_at")?,
            event_after: parse_optional_datetime(self.event_after.as_deref(), "event_after")?,
            event_before: parse_optional_datetime(self.event_before.as_deref(), "event_before")?,
            ingestion_after: parse_optional_datetime(
                self.ingestion_after.as_deref(),
                "ingestion_after",
            )?,
            ingestion_before: parse_optional_datetime(
                self.ingestion_before.as_deref(),
                "ingestion_before",
            )?,
        })
    }
}

impl GetValidParams {
    pub fn to_memory_query(&self) -> anyhow::Result<MemoryQuery> {
        Ok(MemoryQuery {
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            namespace: self.namespace.clone(),
            memory_type: parse_memory_type(self.memory_type.as_deref())?,
            metadata_filter: self.metadata_filter.clone(),
            valid_at: parse_optional_datetime(self.timestamp.as_deref(), "timestamp")?,
            event_after: parse_optional_datetime(self.event_after.as_deref(), "event_after")?,
            event_before: parse_optional_datetime(self.event_before.as_deref(), "event_before")?,
            ingestion_after: parse_optional_datetime(
                self.ingestion_after.as_deref(),
                "ingestion_after",
            )?,
            ingestion_before: parse_optional_datetime(
                self.ingestion_before.as_deref(),
                "ingestion_before",
            )?,
        })
    }
}

impl GetValidAtParams {
    pub fn to_memory_query(&self) -> anyhow::Result<MemoryQuery> {
        Ok(MemoryQuery {
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            namespace: self.namespace.clone(),
            memory_type: parse_memory_type(self.memory_type.as_deref())?,
            metadata_filter: self.metadata_filter.clone(),
            valid_at: parse_optional_datetime(Some(&self.timestamp), "timestamp")?,
            event_after: parse_optional_datetime(self.event_after.as_deref(), "event_after")?,
            event_before: parse_optional_datetime(self.event_before.as_deref(), "event_before")?,
            ingestion_after: parse_optional_datetime(
                self.ingestion_after.as_deref(),
                "ingestion_after",
            )?,
            ingestion_before: parse_optional_datetime(
                self.ingestion_before.as_deref(),
                "ingestion_before",
            )?,
        })
    }
}

impl ListMemoriesParams {
    pub fn to_memory_query(&self) -> anyhow::Result<MemoryQuery> {
        Ok(MemoryQuery {
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            namespace: self.namespace.clone(),
            memory_type: parse_memory_type(self.memory_type.as_deref())?,
            metadata_filter: self.metadata_filter.clone(),
            valid_at: parse_optional_datetime(self.valid_at.as_deref(), "valid_at")?,
            event_after: parse_optional_datetime(self.event_after.as_deref(), "event_after")?,
            event_before: parse_optional_datetime(self.event_before.as_deref(), "event_before")?,
            ingestion_after: parse_optional_datetime(
                self.ingestion_after.as_deref(),
                "ingestion_after",
            )?,
            ingestion_before: parse_optional_datetime(
                self.ingestion_before.as_deref(),
                "ingestion_before",
            )?,
        })
    }
}

fn parse_memory_type(value: Option<&str>) -> anyhow::Result<Option<MemoryType>> {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| anyhow::anyhow!("Invalid memory_type: '{}'", value)),
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct InvalidateParams {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct ConsolidateMemoryParams {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "any_value_schema")]
    pub metadata: Option<serde_json::Value>,
}

impl ConsolidateMemoryParams {
    pub fn to_memory_query(&self) -> anyhow::Result<MemoryQuery> {
        Ok(MemoryQuery {
            user_id: self.user_id.clone(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            namespace: self.namespace.clone(),
            memory_type: parse_memory_type(self.memory_type.as_deref())?,
            metadata_filter: None,
            valid_at: None,
            event_after: None,
            event_before: None,
            ingestion_after: None,
            ingestion_before: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct GetStatusParams {
    #[serde(skip)]
    pub _placeholder: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct IndexProjectParams {
    pub path: String,
    /// Force re-index (default: false)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// Required together with force=true when retrying a previously failed full index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm_failed_restart: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct SearchCodeParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct ProjectInfoParams {
    /// list|status|stats
    pub action: String,
    /// For: status, stats (required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct DeleteProjectParams {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct ResetAllMemoryParams {
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct DetectCommunitiesParams {
    #[serde(skip)]
    pub _placeholder: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct SearchSymbolsParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct SymbolGraphParams {
    pub symbol_id: String,
    /// callers|callees|related
    pub action: String,
    /// For: related (default: 1, max: 5)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
    /// For: related (in|out|both)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct GetProjectStatsParams {
    pub project_id: String,
}

// --- Internal params (used by logic layer, not exposed as MCP tools) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetIndexStatusParams {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListProjectsParams {
    #[serde(skip)]
    pub _placeholder: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEntityParams {
    pub name: String,
    pub entity_type: Option<String>,
    pub description: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRelationParams {
    pub from_entity: String,
    pub to_entity: String,
    pub relation_type: String,
    pub weight: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRelatedParams {
    pub entity_id: String,
    pub depth: Option<usize>,
    pub direction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct HowToUseParams {
    /// Placeholder. Always pass true.
    #[serde(default)]
    pub _placeholder: bool,
}

#[cfg(test)]
mod tests {
    use super::normalize_project_id;

    #[test]
    fn normalize_project_id_converts_empty_and_whitespace_to_none() {
        assert_eq!(normalize_project_id(None), None);
        assert_eq!(normalize_project_id(Some(String::new())), None);
        assert_eq!(normalize_project_id(Some("   \t\n  ".to_string())), None);
    }

    #[test]
    fn normalize_project_id_trims_non_empty_values() {
        assert_eq!(
            normalize_project_id(Some("  reddoc_dev  ".to_string())),
            Some("reddoc_dev".to_string())
        );
    }
}
