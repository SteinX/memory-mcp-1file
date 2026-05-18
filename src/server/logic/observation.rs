use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::{json, Map, Value};

use crate::config::AppState;
use crate::embedding::ContentHasher;
use crate::server::params::MemoryObservationCreateParams;
use crate::storage::StorageBackend;
use crate::types::{EmbeddingState, ExportIdentity, Memory, MemoryType};

use super::contracts::{export_contract_meta, summary_collection_response, with_surface_guidance};
use super::{error_response, strip_embedding, success_json};

const ALLOWED_PREFIXES: [&str; 7] = [
    "PROJECT:",
    "EPIC:",
    "TASK:",
    "RESEARCH:",
    "DECISION:",
    "CONTEXT:",
    "USER:",
];

fn observation_contract_json() -> Value {
    let contract = with_surface_guidance(
        export_contract_meta(
            ExportIdentity {
                stable_memory_id: None,
                stable_node_ids: true,
                node_ids_are_project_scoped: false,
                node_id_semantics: Some("stable_public_memory_id".to_string()),
                ..Default::default()
            },
            None,
        ),
        &["id", "memory", "contract", "summary"],
        &["observation_summary"],
        &[],
    );
    serde_json::to_value(contract).unwrap_or_else(|_| json!({}))
}

fn normalize_prefixed_content(content: &str) -> anyhow::Result<(String, bool)> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("content must not be empty"));
    }

    if ALLOWED_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        Ok((trimmed.to_string(), false))
    } else {
        Ok((format!("CONTEXT: {trimmed}"), true))
    }
}

fn validate_non_empty(value: &str, field: &str) -> anyhow::Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(anyhow::anyhow!("{field} must not be empty"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn normalize_confidence(confidence: Option<f32>) -> anyhow::Result<Option<f32>> {
    match confidence {
        Some(value) if !value.is_finite() => Err(anyhow::anyhow!("confidence must be finite")),
        Some(value) => Ok(Some(value.clamp(0.0, 1.0))),
        None => Ok(None),
    }
}

fn build_metadata(params: &MemoryObservationCreateParams) -> anyhow::Result<Value> {
    let mut metadata = match params.metadata.clone() {
        Some(Value::Object(map)) => map,
        Some(value) => {
            let mut map = Map::new();
            map.insert("source_metadata".to_string(), value);
            map
        }
        None => Map::new(),
    };

    let source = validate_non_empty(&params.source, "source")?;
    let event_type = validate_non_empty(&params.event_type, "event_type")?;
    let confidence = normalize_confidence(params.confidence)?;
    let redaction_state = params
        .redaction_state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();

    metadata.insert(
        "observation".to_string(),
        json!({
            "schema_version": 1,
            "source": source,
            "event_type": event_type,
            "confidence": confidence,
            "redaction_state": redaction_state,
            "created_from": "memory_observation_create",
        }),
    );

    Ok(Value::Object(metadata))
}

pub async fn memory_observation_create(
    state: &Arc<AppState>,
    params: MemoryObservationCreateParams,
) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);

    let (content, prefix_added) = match normalize_prefixed_content(&params.content) {
        Ok(value) => value,
        Err(error) => return Ok(error_response(error)),
    };
    let metadata = match build_metadata(&params) {
        Ok(value) => value,
        Err(error) => return Ok(error_response(error)),
    };
    let embedding = state.embedding.embed(&content).await?;
    let now = crate::types::Datetime::default();

    let memory = Memory {
        content: content.clone(),
        embedding: Some(embedding),
        memory_type: MemoryType::Episodic,
        user_id: params.user_id,
        agent_id: params.agent_id,
        run_id: params.run_id,
        namespace: params.namespace,
        metadata: Some(metadata),
        event_time: now,
        ingestion_time: now,
        valid_from: now,
        importance_score: 1.0,
        content_hash: Some(ContentHasher::hash(&content)),
        embedding_state: EmbeddingState::Ready,
        ..Default::default()
    };

    let id = match state.storage.create_memory(memory).await {
        Ok(id) => id,
        Err(error) => return Ok(error_response(error)),
    };
    let mut stored = match state.storage.get_memory(&id).await {
        Ok(Some(memory)) => memory,
        Ok(None) => return Ok(error_response("created observation could not be read back")),
        Err(error) => return Ok(error_response(error)),
    };

    state.memory_search.upsert_memory(stored.clone()).await;
    strip_embedding(&mut stored);

    Ok(success_json(json!({
        "id": id,
        "memory": stored,
        "observation_summary": {
            "schema_version": 1,
            "source": validate_non_empty(&params.source, "source").unwrap_or_default(),
            "event_type": validate_non_empty(&params.event_type, "event_type").unwrap_or_default(),
            "prefix_added": prefix_added,
            "memory_type": "episodic",
            "promotion": {
                "automatic": false,
                "reason": "v1_observation_is_evidence_only"
            }
        },
        "contract": observation_contract_json(),
        "summary": summary_collection_response(
            "memory_observation_create",
            1,
            Some(1),
            false,
            Some("Observation stored as evidence; no automatic promotion or consolidation was performed.".to_string())
        )
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::params::ListMemoriesParams;
    use crate::test_utils::TestContext;

    fn text_result(result: &CallToolResult) -> Value {
        match &result.content[0].raw {
            rmcp::model::RawContent::Text(text) => serde_json::from_str(&text.text).unwrap(),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    #[tokio::test]
    async fn observation_adds_context_prefix_when_missing() {
        let ctx = TestContext::new().await;

        let result = memory_observation_create(
            &ctx.state,
            MemoryObservationCreateParams {
                content: "hook saw compact recovery".to_string(),
                source: "codex-hook".to_string(),
                event_type: "session_start".to_string(),
                namespace: Some("test".to_string()),
                user_id: None,
                agent_id: None,
                run_id: None,
                confidence: Some(0.8),
                redaction_state: Some("redacted".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();

        let json = text_result(&result);
        assert_eq!(json["observation_summary"]["prefix_added"], true);
        assert!(json["memory"]["content"]
            .as_str()
            .unwrap()
            .starts_with("CONTEXT: "));
    }

    #[tokio::test]
    async fn observation_metadata_is_stored_and_readable() {
        let ctx = TestContext::new().await;

        let result = memory_observation_create(
            &ctx.state,
            MemoryObservationCreateParams {
                content: "CONTEXT: hook captured manual continue".to_string(),
                source: "plugin".to_string(),
                event_type: "manual_continue".to_string(),
                namespace: Some("obs".to_string()),
                user_id: None,
                agent_id: None,
                run_id: None,
                confidence: Some(0.9),
                redaction_state: Some("none".to_string()),
                metadata: Some(json!({"hook": "startup"})),
            },
        )
        .await
        .unwrap();
        let id = text_result(&result)["id"].as_str().unwrap().to_string();

        let list = super::super::memory::list_memories(
            &ctx.state,
            ListMemoriesParams {
                limit: Some(10),
                offset: None,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: Some("obs".to_string()),
                memory_type: None,
                metadata_filter: None,
                valid_at: None,
                event_after: None,
                event_before: None,
                ingestion_after: None,
                ingestion_before: None,
            },
        )
        .await
        .unwrap();
        let listed = text_result(&list);
        let memory = listed["memories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|memory| memory["id"]["id"].as_str() == Some(id.as_str()))
            .or_else(|| listed["memories"].as_array().unwrap().first())
            .expect("observation should be listed");

        assert_eq!(
            memory["metadata"]["observation"]["created_from"].as_str(),
            Some("memory_observation_create")
        );
        assert_eq!(
            memory["metadata"]["observation"]["source"].as_str(),
            Some("plugin")
        );
        assert_eq!(
            memory["metadata"]["observation"]["event_type"].as_str(),
            Some("manual_continue")
        );
    }
}
