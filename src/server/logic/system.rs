use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::embedding::EmbeddingStatus;
use crate::server::logic::memory_lifecycle::{
    derive_memory_lifecycle, RetentionPolicy, RETENTION_POLICY_VERSION,
};
use crate::server::params::{GetStatusParams, ResetAllMemoryParams};
use crate::storage::traits::MemoryGcFilter;
use crate::storage::StorageBackend;

use super::{error_response, success_json};

pub async fn get_status(
    state: &Arc<AppState>,
    _params: GetStatusParams,
) -> anyhow::Result<CallToolResult> {
    let memories_count = state.storage.count_memories().await.unwrap_or(0);
    let valid_memories_count = state.storage.count_valid_memories().await.unwrap_or(0);
    let db_healthy = state.storage.health_check().await.unwrap_or(false);
    let embedding_status = state.embedding.status().await;
    let gc_filter = MemoryGcFilter::default();
    let invalidated_memories_count = state
        .storage
        .count_invalidated_memories_for_gc(&gc_filter)
        .await
        .unwrap_or(0);
    let reason_distribution = state
        .storage
        .count_invalidated_memories_by_reason(&gc_filter)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| json!({ "reason": row.reason, "count": row.count }))
        .collect::<Vec<_>>();
    let policy = RetentionPolicy::default();
    let now = chrono::Utc::now();
    let mut purge_eligible_count = 0usize;
    let mut offset = 0usize;
    const PAGE_SIZE: usize = 500;
    loop {
        let page = state
            .storage
            .list_invalidated_memories_for_gc(&gc_filter, PAGE_SIZE, offset)
            .await
            .unwrap_or_default();
        if page.is_empty() {
            break;
        }
        purge_eligible_count += page
            .iter()
            .filter(|memory| derive_memory_lifecycle(memory, now, &policy).purge_eligible)
            .count();
        let page_len = page.len();
        if page_len < PAGE_SIZE {
            break;
        }
        offset += page_len;
    }

    let (overall_status, embedding_json) = match &embedding_status {
        EmbeddingStatus::Ready => (
            "healthy",
            json!({
                "status": "ready",
                "model": format!("{}_{}", state.embedding.model(), state.embedding.dimensions()),
                "dimensions": state.embedding.dimensions()
            }),
        ),
        EmbeddingStatus::Loading {
            phase,
            elapsed_seconds,
            eta_seconds,
            cached,
            progress_percent,
            ..
        } => {
            let mut loading_json = json!({
                "status": "loading",
                "phase": phase.to_string(),
                "elapsed_seconds": elapsed_seconds,
                "eta_seconds": eta_seconds,
                "cached": cached,
                "model": format!("{}_{}", state.embedding.model(), state.embedding.dimensions()),
                "dimensions": state.embedding.dimensions()
            });
            if let Some(pct) = progress_percent {
                loading_json["progress_percent"] = json!(pct);
            }
            ("loading", loading_json)
        }
        EmbeddingStatus::Error { message } => (
            "error",
            json!({
                "status": "error",
                "error": message,
                "model": format!("{}_{}", state.embedding.model(), state.embedding.dimensions()),
                "dimensions": state.embedding.dimensions()
            }),
        ),
    };

    let status = if !db_healthy {
        "degraded"
    } else {
        overall_status
    };

    Ok(success_json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "status": status,
        "memories_count": memories_count,
        "memory_gc": {
            "policy_version": RETENTION_POLICY_VERSION,
            "total": memories_count,
            "valid": valid_memories_count,
            "invalidated": invalidated_memories_count,
            "purge_eligible": purge_eligible_count,
            "reason_distribution": reason_distribution,
        },
        "embedding": embedding_json
    })))
}

pub async fn reset_all_memory(
    state: &Arc<AppState>,
    params: ResetAllMemoryParams,
) -> anyhow::Result<CallToolResult> {
    if !params.confirm {
        return Ok(error_response("Must set confirm=true to reset all data"));
    }

    state.storage.reset_db().await?;

    Ok(success_json(json!({
        "reset": true,
        "warning": "All data has been cleared"
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;
    use crate::types::{Memory, MemoryType};

    #[tokio::test]
    async fn test_system_logic() {
        let ctx = TestContext::new().await;

        // Seed
        ctx.state
            .storage
            .create_memory(Memory {
                id: None,
                content: "To be reset".to_string(),
                embedding: None,
                memory_type: MemoryType::Semantic,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                metadata: None,
                event_time: Default::default(),
                ingestion_time: Default::default(),
                valid_from: Default::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        // 1. Get Status
        let status_params = GetStatusParams {
            _placeholder: false,
        };
        let status_res = get_status(&ctx.state, status_params).await.unwrap();
        let status_val = serde_json::to_value(&status_res).unwrap();
        let status_text = status_val["content"][0]["text"].as_str().unwrap();
        let status_json: serde_json::Value = serde_json::from_str(status_text).unwrap();
        assert_eq!(status_json["memories_count"].as_u64().unwrap(), 1);

        // 2. Reset without confirm
        let reset_params_fail = ResetAllMemoryParams { confirm: false };
        let reset_res_fail = reset_all_memory(&ctx.state, reset_params_fail)
            .await
            .unwrap();
        let fail_val = serde_json::to_value(&reset_res_fail).unwrap();
        let fail_text = fail_val["content"][0]["text"].as_str().unwrap();
        let fail_json: serde_json::Value = serde_json::from_str(fail_text).unwrap();
        assert!(fail_json.get("error").is_some());

        // 3. Reset with confirm
        let reset_params = ResetAllMemoryParams { confirm: true };
        let reset_res = reset_all_memory(&ctx.state, reset_params).await.unwrap();
        let success_val = serde_json::to_value(&reset_res).unwrap();
        let success_text = success_val["content"][0]["text"].as_str().unwrap();
        let success_json: serde_json::Value = serde_json::from_str(success_text).unwrap();
        assert!(success_json.get("reset").is_some());

        assert_eq!(ctx.state.storage.count_memories().await.unwrap(), 0);
    }
}
