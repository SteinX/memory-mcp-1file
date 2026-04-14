use super::{Datetime, SurrealValue, Thing};
use serde::{Deserialize, Serialize};

use super::EmbeddingState;

fn default_memory_type() -> MemoryType {
    MemoryType::Semantic
}

fn default_importance() -> f32 {
    1.0
}

fn default_datetime() -> Datetime {
    Datetime::default()
}

fn default_content() -> String {
    String::new()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, SurrealValue)]
pub struct Memory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Thing>,

    #[serde(default = "default_content")]
    pub content: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,

    #[serde(default = "default_memory_type")]
    pub memory_type: MemoryType,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    #[serde(default = "default_datetime")]
    pub event_time: Datetime,

    #[serde(default = "default_datetime")]
    pub ingestion_time: Datetime,

    #[serde(default = "default_datetime")]
    pub valid_from: Datetime,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<Datetime>,

    #[serde(default = "default_importance")]
    pub importance_score: f32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidation_reason: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_hash: Option<String>,

    #[serde(default)]
    pub embedding_state: EmbeddingState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Episodic,
    #[default]
    Semantic,
    Procedural,
}

impl std::str::FromStr for MemoryType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "episodic" => Ok(Self::Episodic),
            "semantic" => Ok(Self::Semantic),
            "procedural" => Ok(Self::Procedural),
            _ => Err(format!("Unknown memory type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<MemoryType>,

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
    pub metadata: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_state: Option<EmbeddingState>,
}

impl Memory {
    pub fn new(content: String) -> Self {
        let now = Datetime::default();
        Self {
            id: None,
            content,
            embedding: None,
            memory_type: MemoryType::Semantic,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            metadata: None,
            event_time: now,
            ingestion_time: now,
            valid_from: now,
            valid_until: None,
            importance_score: 1.0,
            invalidation_reason: None,
            superseded_by: None,
            content_hash: None,
            embedding_state: EmbeddingState::default(),
        }
    }

    pub fn with_type(mut self, memory_type: MemoryType) -> Self {
        self.memory_type = memory_type;
        self
    }

    pub fn with_user_id(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    pub fn with_agent_id(mut self, agent_id: String) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    pub fn with_run_id(mut self, run_id: String) -> Self {
        self.run_id = Some(run_id);
        self
    }

    pub fn with_namespace(mut self, namespace: String) -> Self {
        self.namespace = Some(namespace);
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}
