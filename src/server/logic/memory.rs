use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::embedding::ContentHasher;
use crate::server::params::{
    ConsolidateMemoryParams, DeleteMemoryParams, GetMemoryParams, GetValidAtParams,
    GetValidParams, InvalidateParams, ListMemoriesParams, StoreMemoryParams, UpdateMemoryParams,
};
use crate::storage::StorageBackend;
use crate::types::EmbeddingState;
use crate::types::{record_key_to_string, Memory, MemoryType, MemoryUpdate};

use super::{error_response, normalize_limit, strip_embedding, strip_embeddings, success_json};

fn normalize_importance_score(value: Option<f32>) -> anyhow::Result<Option<f32>> {
    match value {
        Some(score) if !score.is_finite() => {
            Err(anyhow::anyhow!("importance_score must be a finite number"))
        }
        Some(score) => Ok(Some(score.clamp(0.1, 5.0))),
        None => Ok(None),
    }
}

fn parse_memory_type(value: Option<&str>) -> anyhow::Result<MemoryType> {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => value
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid memory_type: '{}'", value)),
        None => Ok(MemoryType::default()),
    }
}

async fn create_and_sync_memory(state: &Arc<AppState>, memory: Memory) -> anyhow::Result<String> {
    let id = state.storage.create_memory(memory).await?;
    if let Ok(Some(created)) = state.storage.get_memory(&id).await {
        state.memory_search.upsert_memory(created).await;
    }
    Ok(id)
}

async fn invalidate_and_sync_memory(
    state: &Arc<AppState>,
    id: &str,
    reason: Option<&str>,
    superseded_by: Option<&str>,
) -> anyhow::Result<bool> {
    let success = state.storage.invalidate(id, reason, superseded_by).await?;
    if success {
        if let Ok(Some(memory)) = state.storage.get_memory(id).await {
            state.memory_search.upsert_memory(memory).await;
        }
    }
    Ok(success)
}

pub async fn store_memory(
    state: &Arc<AppState>,
    params: StoreMemoryParams,
) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);

    let embedding = state.embedding.embed(&params.content).await?;

    let mem_type = match parse_memory_type(params.memory_type.as_deref()) {
        Ok(memory_type) => memory_type,
        Err(e) => return Ok(error_response(e)),
    };

    let content_hash = ContentHasher::hash(&params.content);
    let now = crate::types::Datetime::default();
    let importance_score = match normalize_importance_score(params.importance_score) {
        Ok(Some(score)) => score,
        Ok(None) => 1.0,
        Err(e) => return Ok(error_response(e)),
    };
    let memory = Memory {
        content: params.content,
        embedding: Some(embedding),
        memory_type: mem_type,
        user_id: params.user_id,
        agent_id: params.agent_id,
        run_id: params.run_id,
        namespace: params.namespace,
        metadata: params.metadata,
        event_time: now,
        ingestion_time: now,
        valid_from: now,
        importance_score,
        content_hash: Some(content_hash),
        embedding_state: EmbeddingState::Ready,
        ..Default::default()
    };

    match create_and_sync_memory(state, memory).await {
        Ok(id) => Ok(success_json(json!({ "id": id }))),
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn consolidate_memory(
    state: &Arc<AppState>,
    params: ConsolidateMemoryParams,
) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);

    let mem_type = match parse_memory_type(params.memory_type.as_deref()) {
        Ok(memory_type) => memory_type,
        Err(e) => return Ok(error_response(e)),
    };
    let filters = match params.to_memory_query() {
        Ok(filters) => filters,
        Err(e) => return Ok(error_response(e)),
    };
    let importance_score = match normalize_importance_score(params.importance_score) {
        Ok(Some(score)) => score,
        Ok(None) => 1.0,
        Err(e) => return Ok(error_response(e)),
    };
    let content_hash = ContentHasher::hash(&params.content);

    let total = match state.storage.count_memories_filtered(&filters).await {
        Ok(total) => total,
        Err(e) => return Ok(error_response(e)),
    };
    let existing = match state
        .storage
        .list_memories(&filters, total.max(1), 0)
        .await
    {
        Ok(memories) => memories,
        Err(e) => return Ok(error_response(e)),
    };

    let duplicate_ids: Vec<String> = existing
        .into_iter()
        .filter(|memory| {
            memory.content_hash.as_deref() == Some(content_hash.as_str()) || memory.content == params.content
        })
        .filter_map(|memory| {
            memory
                .id
                .as_ref()
                .map(|thing| record_key_to_string(&thing.key))
        })
        .collect();

    let embedding = match state.embedding.embed(&params.content).await {
        Ok(embedding) => embedding,
        Err(e) => return Ok(error_response(e)),
    };
    let now = crate::types::Datetime::default();
    let memory = Memory {
        content: params.content,
        embedding: Some(embedding),
        memory_type: mem_type,
        user_id: params.user_id,
        agent_id: params.agent_id,
        run_id: params.run_id,
        namespace: params.namespace,
        metadata: params.metadata,
        event_time: now,
        ingestion_time: now,
        valid_from: now,
        importance_score,
        content_hash: Some(content_hash.clone()),
        embedding_state: EmbeddingState::Ready,
        ..Default::default()
    };

    let replacement_id = match create_and_sync_memory(state, memory).await {
        Ok(id) => id,
        Err(e) => return Ok(error_response(e)),
    };

    let reason = params
        .reason
        .as_deref()
        .filter(|reason| !reason.trim().is_empty())
        .or(Some("exact_duplicate_consolidated"));

    let mut superseded_ids = Vec::new();
    for duplicate_id in duplicate_ids {
        if duplicate_id == replacement_id {
            continue;
        }
        match invalidate_and_sync_memory(state, &duplicate_id, reason, Some(&replacement_id)).await {
            Ok(true) => superseded_ids.push(duplicate_id),
            Ok(false) => {}
            Err(e) => return Ok(error_response(e)),
        }
    }

    Ok(success_json(json!({
        "id": replacement_id,
        "content_hash": content_hash,
        "superseded_ids": superseded_ids,
        "superseded_count": superseded_ids.len(),
        "filters": filters.describe(),
        "reason": reason,
    })))
}

pub async fn get_memory(
    state: &Arc<AppState>,
    params: GetMemoryParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.get_memory(&params.id).await {
        Ok(Some(mut memory)) => {
            strip_embedding(&mut memory);
            Ok(success_json(
                serde_json::to_value(&memory).unwrap_or_default(),
            ))
        }
        Ok(None) => Ok(error_response(format!("Memory not found: {}", params.id))),
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn update_memory(
    state: &Arc<AppState>,
    params: UpdateMemoryParams,
) -> anyhow::Result<CallToolResult> {
    let (embedding, content_hash, embedding_state) = if let Some(ref new_content) = params.content {
        let old_memory = state.storage.get_memory(&params.id).await?;
        let old_hash = old_memory.as_ref().and_then(|m| m.content_hash.as_deref());

        if ContentHasher::needs_reembed(old_hash, new_content) {
            let emb = state.embedding.embed(new_content).await?;
            let hash = ContentHasher::hash(new_content);
            (Some(emb), Some(hash), Some(EmbeddingState::Ready))
        } else {
            (None, None, None)
        }
    } else {
        (None, None, None)
    };

    let update = MemoryUpdate {
        content: params.content,
        memory_type: match &params.memory_type {
            Some(s) => Some(
                s.parse()
                    .map_err(|_| anyhow::anyhow!("Invalid memory_type: '{}'", s))?,
            ),
            None => None,
        },
        user_id: params.user_id,
        agent_id: params.agent_id,
        run_id: params.run_id,
        namespace: params.namespace,
        importance_score: match normalize_importance_score(params.importance_score) {
            Ok(score) => score,
            Err(e) => return Ok(error_response(e)),
        },
        metadata: params.metadata,
        embedding,
        content_hash,
        embedding_state,
    };

    match state.storage.update_memory(&params.id, update).await {
        Ok(mut memory) => {
            state.memory_search.upsert_memory(memory.clone()).await;
            strip_embedding(&mut memory);
            Ok(success_json(
                serde_json::to_value(&memory).unwrap_or_default(),
            ))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn delete_memory(
    state: &Arc<AppState>,
    params: DeleteMemoryParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.delete_memory(&params.id).await {
        Ok(deleted) => {
            if deleted {
                state.memory_search.remove_memory(&params.id).await;
            }
            Ok(success_json(json!({ "deleted": deleted })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn list_memories(
    state: &Arc<AppState>,
    params: ListMemoriesParams,
) -> anyhow::Result<CallToolResult> {
    let limit = normalize_limit(params.limit);
    let offset = params.offset.unwrap_or(0);
    let filters = match params.to_memory_query() {
        Ok(filters) => filters,
        Err(e) => return Ok(error_response(e)),
    };

    let mut memories = match state.storage.list_memories(&filters, limit, offset).await {
        Ok(m) => m,
        Err(e) => return Ok(error_response(e)),
    };

    strip_embeddings(&mut memories);
    let total = state
        .storage
        .count_memories_filtered(&filters)
        .await
        .unwrap_or(0);

    Ok(success_json(json!({
        "memories": memories,
        "total": total,
        "limit": limit,
        "offset": offset,
        "filters": filters.describe(),
        "metadata_filter_diagnostics": {
            "enabled": filters.uses_metadata_post_filter(),
            "mode": if filters.uses_metadata_post_filter() { "post_query_subset_match" } else { "disabled" },
            "notes": if filters.uses_metadata_post_filter() {
                "metadata_filter is applied after DB retrieval; total reflects post-filtered list results."
            } else {
                "metadata_filter not used for this request."
            }
        }
    })))
}

pub async fn get_valid(
    state: &Arc<AppState>,
    params: GetValidParams,
) -> anyhow::Result<CallToolResult> {
    let limit = normalize_limit(params.limit);
    let filters = match params.to_memory_query() {
        Ok(filters) => filters,
        Err(e) => return Ok(error_response(e)),
    };

    match state.storage.get_valid(&filters, limit).await {
        Ok(mut memories) => {
            strip_embeddings(&mut memories);
            Ok(success_json(json!({
                "memories": memories,
                "count": memories.len(),
                "filters": filters.describe(),
                "metadata_filter_diagnostics": {
                    "enabled": filters.uses_metadata_post_filter(),
                    "mode": if filters.uses_metadata_post_filter() { "post_query_subset_match" } else { "disabled" },
                    "notes": if filters.uses_metadata_post_filter() {
                        "metadata_filter is applied after DB retrieval; count reflects post-filtered valid memories."
                    } else {
                        "metadata_filter not used for this request."
                    }
                }
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn get_valid_at(
    state: &Arc<AppState>,
    params: GetValidAtParams,
) -> anyhow::Result<CallToolResult> {
    let limit = normalize_limit(params.limit);
    let filters = match params.to_memory_query() {
        Ok(filters) => filters,
        Err(e) => return Ok(error_response(e)),
    };

    match state.storage.get_valid_at(&filters, limit).await {
        Ok(mut memories) => {
            strip_embeddings(&mut memories);
            Ok(success_json(json!({
                "memories": memories,
                "count": memories.len(),
                "timestamp": params.timestamp,
                "filters": filters.describe(),
                "metadata_filter_diagnostics": {
                    "enabled": filters.uses_metadata_post_filter(),
                    "mode": if filters.uses_metadata_post_filter() { "post_query_subset_match" } else { "disabled" },
                    "notes": if filters.uses_metadata_post_filter() {
                        "metadata_filter is applied after DB retrieval for point-in-time reads; count reflects post-filtered results."
                    } else {
                        "metadata_filter not used for this request."
                    }
                }
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn invalidate(
    state: &Arc<AppState>,
    params: InvalidateParams,
) -> anyhow::Result<CallToolResult> {
    match invalidate_and_sync_memory(
        state,
        &params.id,
        params.reason.as_deref(),
        params.superseded_by.as_deref(),
    )
    .await
    {
        Ok(success) => {
            Ok(success_json(json!({ "invalidated": success })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;

    #[tokio::test]
    async fn test_memory_crud_logic() {
        let ctx = TestContext::new().await;

        // 1. Store
        let params = StoreMemoryParams {
            content: "Logic test memory".to_string(),
            memory_type: Some("semantic".to_string()),
            user_id: Some("user1".to_string()),
            agent_id: Some("agent-a".to_string()),
            run_id: Some("run-1".to_string()),
            namespace: Some("project-alpha".to_string()),
            importance_score: Some(2.5),
            metadata: None,
        };
        let result = store_memory(&ctx.state, params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let id = json["id"].as_str().unwrap().to_string();

        // 2. Get
        let get_params = GetMemoryParams { id: id.clone() };
        let result = get_memory(&ctx.state, get_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let memory_json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(memory_json["content"], "Logic test memory");
        assert_eq!(memory_json["agent_id"], "agent-a");
        assert_eq!(memory_json["namespace"], "project-alpha");
        assert_eq!(memory_json["importance_score"], 2.5);

        // 2.1 Invalidate with superseded_by and verify read model preserves it
        let invalidate_params = InvalidateParams {
            id: id.clone(),
            reason: Some("superseded".to_string()),
            superseded_by: Some("replacement-123".to_string()),
        };
        let _ = invalidate(&ctx.state, invalidate_params).await.unwrap();

        let result = get_memory(&ctx.state, GetMemoryParams { id: id.clone() }).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let invalidated_json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(invalidated_json["superseded_by"], "replacement-123");

        // 3. List
        let list_params = ListMemoriesParams {
            limit: Some(10),
            offset: None,
            user_id: Some("user1".to_string()),
            agent_id: None,
            run_id: None,
            namespace: Some("project-alpha".to_string()),
            memory_type: None,
            metadata_filter: None,
            valid_at: None,
            event_after: None,
            event_before: None,
            ingestion_after: None,
            ingestion_before: None,
        };
        let result = list_memories(&ctx.state, list_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let list_json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(list_json["memories"].as_array().unwrap().len(), 0);
        assert_eq!(list_json["total"], 0);
        assert_eq!(list_json["filters"]["userId"], "user1");
    }

    #[tokio::test]
    async fn test_consolidate_memory_logic() {
        let ctx = TestContext::new().await;

        let original = StoreMemoryParams {
            content: "Exact duplicate content".to_string(),
            memory_type: Some("semantic".to_string()),
            user_id: Some("user-dedup".to_string()),
            agent_id: None,
            run_id: None,
            namespace: Some("project-dedup".to_string()),
            importance_score: Some(1.0),
            metadata: None,
        };
        let result = store_memory(&ctx.state, original).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let original_json: serde_json::Value = serde_json::from_str(text).unwrap();
        let original_id = original_json["id"].as_str().unwrap().to_string();

        let consolidate_params = ConsolidateMemoryParams {
            content: "Exact duplicate content".to_string(),
            memory_type: Some("semantic".to_string()),
            user_id: Some("user-dedup".to_string()),
            agent_id: None,
            run_id: None,
            namespace: Some("project-dedup".to_string()),
            importance_score: Some(2.0),
            reason: Some("deduplicated".to_string()),
            metadata: Some(json!({"source": "consolidated"})),
        };
        let result = consolidate_memory(&ctx.state, consolidate_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let consolidate_json: serde_json::Value = serde_json::from_str(text).unwrap();
        let replacement_id = consolidate_json["id"].as_str().unwrap().to_string();

        assert_ne!(replacement_id, original_id);
        assert_eq!(consolidate_json["superseded_count"], 1);
        assert_eq!(consolidate_json["superseded_ids"][0], original_id);

        let result = get_memory(&ctx.state, GetMemoryParams { id: original_id.clone() })
            .await
            .unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let old_memory_json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(old_memory_json["superseded_by"], replacement_id);
        assert_eq!(old_memory_json["invalidation_reason"], "deduplicated");

        let list_params = ListMemoriesParams {
            limit: Some(10),
            offset: None,
            user_id: Some("user-dedup".to_string()),
            agent_id: None,
            run_id: None,
            namespace: Some("project-dedup".to_string()),
            memory_type: Some("semantic".to_string()),
            metadata_filter: None,
            valid_at: None,
            event_after: None,
            event_before: None,
            ingestion_after: None,
            ingestion_before: None,
        };
        let result = list_memories(&ctx.state, list_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let list_json: serde_json::Value = serde_json::from_str(text).unwrap();
        let memories = list_json["memories"].as_array().unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(list_json["total"], 1);
        let listed_id = memories[0]["id"]["key"]["String"]
            .as_str()
            .or_else(|| memories[0]["id"].as_str())
            .expect("list memory id should be readable");
        assert_eq!(listed_id, replacement_id);
        assert_eq!(memories[0]["content_hash"], consolidate_json["content_hash"]);
    }
}
