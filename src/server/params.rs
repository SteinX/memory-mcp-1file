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
    #[serde(skip_serializing_if = "Option::is_none", alias = "project_id")]
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
    /// For: create_relation (required). Entity ID returned by create_entity, not display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_entity: Option<String>,
    /// For: create_relation (required). Entity ID returned by create_entity, not display name.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_plan_fingerprint: Option<String>,
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
pub struct PreviewConsolidateMemoryParams {
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

impl PreviewConsolidateMemoryParams {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "projectId")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_full_restart_fallback: Option<bool>,
    /// Force re-index (default: false)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// Required together with force=true when retrying a previously failed full index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_failed_restart: Option<bool>,
    /// Glob patterns for files to include (replaces config default when Some).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_patterns: Option<Vec<String>>,
    /// Glob patterns for files to exclude (replaces config default when Some).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_patterns: Option<Vec<String>>,
}

#[cfg(test)]
mod index_project_param_tests {
    use super::IndexProjectParams;

    #[test]
    fn index_project_params_legacy_deserializes() {
        let params: IndexProjectParams = serde_json::from_str(r#"{"path":"/tmp/p"}"#)
            .expect("legacy index_project payload should deserialize");

        assert_eq!(params.path.as_deref(), Some("/tmp/p"));
        assert_eq!(params.project_id, None);
        assert_eq!(params.resume, None);
        assert_eq!(params.job_id, None);
        assert_eq!(params.resume_token, None);
        assert_eq!(params.allow_full_restart_fallback, None);
        assert_eq!(params.force, None);
        assert_eq!(params.confirm_failed_restart, None);
        assert_eq!(params.include_patterns, None);
        assert_eq!(params.exclude_patterns, None);
    }

    #[test]
    fn index_project_params_resume_deserializes() {
        let params: IndexProjectParams = serde_json::from_value(serde_json::json!({
            "project_id": "project",
            "resume": true,
            "job_id": "job-123",
            "resume_token": "resume-token-456",
            "allow_full_restart_fallback": false
        }))
        .expect("resume index_project payload should deserialize");

        assert_eq!(params.path, None);
        assert_eq!(params.project_id.as_deref(), Some("project"));
        assert_eq!(params.resume, Some(true));
        assert_eq!(params.job_id.as_deref(), Some("job-123"));
        assert_eq!(params.resume_token.as_deref(), Some("resume-token-456"));
        assert_eq!(params.allow_full_restart_fallback, Some(false));
        assert_eq!(params.force, None);
        assert_eq!(params.confirm_failed_restart, None);
        assert_eq!(params.include_patterns, None);
        assert_eq!(params.exclude_patterns, None);
    }

    #[test]
    fn index_project_params_with_filter_deserializes() {
        let params: IndexProjectParams = serde_json::from_value(serde_json::json!({
            "path": "/tmp/p",
            "include_patterns": ["src/**/*.rs"],
            "exclude_patterns": ["**/target/**"]
        }))
        .expect("filter index_project payload should deserialize");

        assert_eq!(params.path.as_deref(), Some("/tmp/p"));
        assert_eq!(
            params.include_patterns,
            Some(vec!["src/**/*.rs".to_string()])
        );
        assert_eq!(
            params.exclude_patterns,
            Some(vec!["**/target/**".to_string()])
        );
        assert_eq!(params.force, None);
        assert_eq!(params.confirm_failed_restart, None);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct SearchCodeParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none", alias = "projectId")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct ProjectInfoParams {
    /// list|index|status|stats|projection|projection_by_locator|bind|unbind|binding_status
    pub action: String,
    /// For: status, stats, projection, bind (required)
    #[serde(skip_serializing_if = "Option::is_none", alias = "projectId")]
    pub project_id: Option<String>,
    /// For: index (required). Server-visible path to index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// For: index. Force full re-index (default: false).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// For: index. Required together with force=true when retrying a previously failed full index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm_failed_restart: Option<bool>,
    /// For: projection_by_locator (required)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    /// For: projection (optional). all|calls|imports|type_links|none
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_scope: Option<String>,
    /// For: projection (optional). canonical
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_mode: Option<String>,
    /// For: cancel_index (required). Durable job ID to cancel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// For: index. Glob patterns for files to include (replaces config default when Some).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_patterns: Option<Vec<String>>,
    /// For: index. Glob patterns for files to exclude (replaces config default when Some).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_patterns: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct DeleteProjectParams {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
#[schemars(title = "")]
pub struct ExportMemoryParams {
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_invalidated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
#[schemars(title = "")]
pub struct ImportMemoryParams {
    pub project_id: String,
    pub jsonl: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_invalidated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserve_project_id: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none", alias = "projectId")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProjectProjectionParams {
    pub project_id: String,
    pub relation_scope: Option<String>,
    pub sort_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetProjectionByLocatorParams {
    pub locator: String,
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

// ============================================================================
// Learning memory tool parameter structs
// ============================================================================

/// Parameters for `learning_memory_create`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryCreateParams {
    /// The learning content to store.
    pub content: String,
    /// Kind of learning: user_preference | project_lesson | project_pattern | project_pitfall | workflow_rule
    pub kind: String,
    /// Lifecycle status (default: candidate): candidate | confirmed | rule
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Confidence score in [0.0, 1.0].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Scope level: global | project | workspace | mode | agent | session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Optional project_id for project-scoped learning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Source of the learning: manual | plugin | migration | promotion | supersede | consolidation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Supporting evidence or examples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<String>>,
    /// Contexts where this learning applies (e.g. file globs, tool names).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<Vec<String>>,
    /// Phrases or patterns that should trigger recall of this learning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_hints: Option<Vec<String>>,
    /// Constraints or caveats on applicability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Vec<String>>,
}

/// Parameters for `learning_memory_get`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryGetParams {
    /// Memory record ID.
    pub id: String,
}

/// Parameters for `learning_memory_list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryListParams {
    /// Filter object (include_status, exclude_status, include_invalidated, audit, fallback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "any_value_schema")]
    pub filter: Option<serde_json::Value>,
    /// Scope level: global | project | workspace | mode | agent | session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Optional project_id for project-scoped listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Maximum number of results (default: 20, max: 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Pagination offset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

/// Parameters for `learning_memory_search`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemorySearchParams {
    /// Semantic search query.
    pub query: String,
    /// Filter object (include_status, exclude_status, include_invalidated, audit, fallback).
    /// Defaults to confirmed+rule only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "any_value_schema")]
    pub filter: Option<serde_json::Value>,
    /// Scope level: global | project | workspace | mode | agent | session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Optional project_id for project-scoped search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Maximum number of results (default: 20, max: 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Parameters for `learning_memory_update`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryUpdateParams {
    /// Memory record ID.
    pub id: String,
    /// Updated content (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Updated confidence score in [0.0, 1.0] (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Updated evidence list (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<String>>,
}

/// Parameters for `learning_memory_promote`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryPromoteParams {
    /// Memory record ID.
    pub id: String,
    /// Target status: confirmed | rule
    pub target_status: String,
    /// Optional target kind override: user_preference | project_lesson | project_pattern | project_pitfall | workflow_rule
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
}

/// Parameters for `learning_memory_reject`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryRejectParams {
    /// Memory record ID.
    pub id: String,
    /// Optional human-readable reason for rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Parameters for `learning_memory_archive`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryArchiveParams {
    /// Memory record ID.
    pub id: String,
    /// Optional human-readable reason for archiving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Parameters for `learning_memory_supersede`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemorySupersededParams {
    /// ID of the record being superseded.
    pub id: String,
    /// ID of the replacement record.
    pub replacement_id: String,
    /// Optional human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Parameters for `learning_memory_migrate_legacy`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryMigrateLegacyParams {
    /// Memory content prefix allowlist (e.g. ["LEARNING:", "RULE:", "PATTERN:"]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_allowlist: Option<Vec<String>>,
    /// Scope level to assign migrated records: global | project | workspace
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Optional project_id for project-scoped migration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Dry-run mode — preview without writing (default: true).
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,
    /// Maximum number of records to migrate per call (default: 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Include invalidated legacy records in the migration scan (default: false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_invalidated: Option<bool>,
    /// In apply mode, invalidate migrated source records with reason migration_replaced (default: false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidate_source: Option<bool>,
    /// Explicitly allow RESEARCH: records to be converted into project_lesson candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_research_lessons: Option<bool>,
}

fn default_dry_run() -> bool {
    true
}

/// Parameters for `learning_memory_delete` (compatibility shim).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "")]
pub struct LearningMemoryDeleteParams {
    /// Memory record ID.
    pub id: String,
    /// Soft-delete mode: soft_reject | soft_archive (default: soft_archive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        ExportMemoryParams, ImportMemoryParams, ProjectInfoParams, RecallCodeParams,
        SearchSymbolsParams, normalize_project_id,
    };

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

    #[test]
    fn project_info_params_deserialize_projection_without_options() {
        let params: ProjectInfoParams =
            serde_json::from_str(r#"{"action":"projection","project_id":"proj"}"#).unwrap();

        assert_eq!(params.action, "projection");
        assert_eq!(params.project_id, Some("proj".to_string()));
        assert_eq!(params.locator, None);
        assert_eq!(params.relation_scope, None);
        assert_eq!(params.sort_mode, None);
    }

    #[test]
    fn project_info_params_deserialize_projection_with_relation_scope() {
        let params: ProjectInfoParams = serde_json::from_str(
            r#"{"action":"projection","project_id":"proj","relation_scope":"imports","sort_mode":"canonical"}"#,
        )
        .unwrap();

        assert_eq!(params.action, "projection");
        assert_eq!(params.project_id, Some("proj".to_string()));
        assert_eq!(params.locator, None);
        assert_eq!(params.relation_scope, Some("imports".to_string()));
        assert_eq!(params.sort_mode, Some("canonical".to_string()));
    }

    #[test]
    fn project_info_params_deserialize_projection_with_type_links_scope() {
        let params: ProjectInfoParams = serde_json::from_str(
            r#"{"action":"projection","project_id":"proj","relation_scope":"type_links"}"#,
        )
        .unwrap();

        assert_eq!(params.action, "projection");
        assert_eq!(params.project_id, Some("proj".to_string()));
        assert_eq!(params.locator, None);
        assert_eq!(params.relation_scope, Some("type_links".to_string()));
        assert_eq!(params.sort_mode, None);
    }

    #[test]
    fn project_info_params_deserialize_projection_by_locator() {
        let params: ProjectInfoParams = serde_json::from_str(
            r#"{"action":"projection_by_locator","locator":"projection:demo:123"}"#,
        )
        .unwrap();

        assert_eq!(params.action, "projection_by_locator");
        assert_eq!(params.project_id, None);
        assert_eq!(params.locator, Some("projection:demo:123".to_string()));
        assert_eq!(params.relation_scope, None);
        assert_eq!(params.sort_mode, None);
    }

    #[test]
    fn project_info_binding_params_deserialize_bind() {
        let params: ProjectInfoParams =
            serde_json::from_str(r#"{"action":"bind","project_id":"proj"}"#).unwrap();

        assert_eq!(params.action, "bind");
        assert_eq!(params.project_id, Some("proj".to_string()));
        assert_eq!(params.locator, None);
        assert_eq!(params.relation_scope, None);
        assert_eq!(params.sort_mode, None);
    }

    #[test]
    fn project_info_binding_params_deserialize_unbind() {
        let params: ProjectInfoParams = serde_json::from_str(r#"{"action":"unbind"}"#).unwrap();

        assert_eq!(params.action, "unbind");
        assert_eq!(params.project_id, None);
        assert_eq!(params.locator, None);
        assert_eq!(params.relation_scope, None);
        assert_eq!(params.sort_mode, None);
    }

    #[test]
    fn project_info_binding_params_deserialize_binding_status() {
        let params: ProjectInfoParams =
            serde_json::from_str(r#"{"action":"binding_status"}"#).unwrap();

        assert_eq!(params.action, "binding_status");
        assert_eq!(params.project_id, None);
        assert_eq!(params.locator, None);
        assert_eq!(params.relation_scope, None);
        assert_eq!(params.sort_mode, None);
    }

    #[test]
    fn recall_code_params_accept_snake_case_project_id_alias() {
        let params: RecallCodeParams = serde_json::from_str(
            r#"{"query":"Container","project_id":"reddoc_true_dev","limit":5}"#,
        )
        .unwrap();

        assert_eq!(params.project_id, Some("reddoc_true_dev".to_string()));
        assert_eq!(params.limit, Some(5));
    }

    #[test]
    fn project_info_params_accept_camel_case_project_id_alias() {
        let params: ProjectInfoParams =
            serde_json::from_str(r#"{"action":"stats","projectId":"reddoc_true_dev"}"#).unwrap();

        assert_eq!(params.action, "stats");
        assert_eq!(params.project_id, Some("reddoc_true_dev".to_string()));
    }

    #[test]
    fn migration_params_do_not_accept_filesystem_paths() {
        let export_schema: serde_json::Value =
            serde_json::to_value(schemars::schema_for!(ExportMemoryParams)).unwrap();
        let export_object = export_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("export schema object");
        for forbidden in ["path", "url", "file", "overwrite", "reset", "replace"] {
            assert!(
                !export_object.contains_key(forbidden),
                "export params unexpectedly expose {forbidden}"
            );
        }

        let import_schema: serde_json::Value =
            serde_json::to_value(schemars::schema_for!(ImportMemoryParams)).unwrap();
        let import_object = import_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("import schema object");
        for forbidden in ["path", "url", "file", "overwrite", "reset", "replace"] {
            assert!(
                !import_object.contains_key(forbidden),
                "import params unexpectedly expose {forbidden}"
            );
        }

        assert!(
            export_object.contains_key("projectId") || export_object.contains_key("project_id")
        );
        assert!(
            import_object.contains_key("projectId") || import_object.contains_key("project_id")
        );
        assert!(import_object.contains_key("jsonl"));
    }
}
