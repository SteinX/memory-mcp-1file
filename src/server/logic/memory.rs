use std::collections::HashSet;
use std::sync::Arc;

use blake3;
use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::embedding::ContentHasher;
use crate::server::params::{
    ConsolidateMemoryParams, DeleteMemoryParams, GetMemoryParams, GetValidAtParams,
    GetValidParams, InvalidateParams, ListMemoriesParams, PreviewConsolidateMemoryParams,
    StoreMemoryParams, UpdateMemoryParams,
};
use crate::storage::StorageBackend;
use crate::types::EmbeddingState;
use crate::types::{record_key_to_string, ExportIdentity, Memory, MemoryType, MemoryUpdate};

use super::contracts::{export_contract_meta, summary_collection_response, with_surface_guidance};
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

fn memory_contract_json(memory_id: Option<&str>) -> serde_json::Value {
    let contract = with_surface_guidance(
        export_contract_meta(
            ExportIdentity {
                stable_memory_id: memory_id.map(|id| id.to_string()),
                stable_node_ids: true,
                node_ids_are_project_scoped: false,
                stable_edge_ids: false,
                edge_ids_are_local_only: true,
                node_id_semantics: Some("stable_public_memory_id".to_string()),
                edge_id_semantics: Some("no_public_edge_ids".to_string()),
                ..Default::default()
            },
            None,
        ),
        &["memory", "memories", "contract", "summary"],
        &["count", "total", "filters", "metadata_filter_diagnostics"],
        &[],
    );
    serde_json::to_value(contract).unwrap_or_else(|_| json!({}))
}

fn memory_collection_contract_json() -> serde_json::Value {
    memory_contract_json(None)
}

fn resolved_consolidation_reason(reason: Option<&str>) -> &str {
    reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("exact_duplicate_consolidated")
}

fn build_consolidation_plan_fingerprint(
    content_hash: &str,
    filters: &crate::types::MemoryQuery,
    matched_ids: &[String],
    reason: &str,
    memory_type: &MemoryType,
    user_id: Option<&str>,
    agent_id: Option<&str>,
    run_id: Option<&str>,
    namespace: Option<&str>,
    importance_score: f32,
    metadata: Option<&serde_json::Value>,
) -> String {
    let payload = json!({
        "contentHash": content_hash,
        "filters": filters.describe(),
        "matchedIds": matched_ids,
        "reason": reason,
        "replacement": {
            "memoryType": serde_json::to_value(memory_type).unwrap_or(serde_json::Value::Null),
            "userId": user_id,
            "agentId": agent_id,
            "runId": run_id,
            "namespace": namespace,
            "importanceScore": importance_score,
            "metadata": metadata.cloned(),
        }
    });

    blake3::hash(payload.to_string().as_bytes()).to_hex().to_string()
}

fn plan_diagnostics(
    content_hash: &str,
    filters: &crate::types::MemoryQuery,
    matched_ids: &[String],
    reason: &str,
    memory_type: &MemoryType,
    user_id: Option<&str>,
    agent_id: Option<&str>,
    run_id: Option<&str>,
    namespace: Option<&str>,
    importance_score: f32,
    metadata: Option<&serde_json::Value>,
) -> serde_json::Value {
    json!({
        "content_hash": content_hash,
        "filters": filters.describe(),
        "matched_ids": matched_ids,
        "matched_count": matched_ids.len(),
        "reason": reason,
        "replacement": {
            "memory_type": serde_json::to_value(memory_type).unwrap_or(serde_json::Value::Null),
            "user_id": user_id,
            "agent_id": agent_id,
            "run_id": run_id,
            "namespace": namespace,
            "importance_score": importance_score,
            "metadata": metadata.cloned(),
        }
    })
}

fn duplicate_match_summary(
    memory: &Memory,
    content: &str,
    content_hash: &str,
    lookup_source: &str,
) -> Option<serde_json::Value> {
    let hash_match = memory.content_hash.as_deref() == Some(content_hash);
    let content_match = memory.content == content;

    if !hash_match && !content_match {
        return None;
    }

    let id = memory
        .id
        .as_ref()
        .map(|thing| record_key_to_string(&thing.key))?;

    let mut matched_by = Vec::new();
    if hash_match {
        matched_by.push("content_hash");
    }
    if content_match {
        matched_by.push("exact_content");
    }

    Some(json!({
        "id": id,
        "matched_by": matched_by,
        "lookup_source": lookup_source,
    }))
}

fn exact_duplicate_matches(
    memories: Vec<Memory>,
    content: &str,
    content_hash: &str,
    lookup_source: &str,
) -> Vec<serde_json::Value> {
    memories
        .into_iter()
        .filter_map(|memory| duplicate_match_summary(&memory, content, content_hash, lookup_source))
        .collect()
}

fn lookup_diagnostics(
    hash_candidate_count: usize,
    hash_match_count: usize,
    fallback_scanned: bool,
    fallback_match_count: usize,
) -> serde_json::Value {
    json!({
        "hash_first_candidate_count": hash_candidate_count,
        "hash_first_match_count": hash_match_count,
        "fallback_scanned": fallback_scanned,
        "exact_content_fallback_match_count": fallback_match_count,
        "used_hash_first": hash_match_count > 0,
        "used_exact_content_fallback": fallback_match_count > 0,
    })
}

async fn find_duplicate_matches(
    state: &Arc<AppState>,
    filters: &crate::types::MemoryQuery,
    content: &str,
    content_hash: &str,
) -> anyhow::Result<(Vec<serde_json::Value>, serde_json::Value)> {
    let hash_matches = state
        .storage
        .find_memories_by_content_hash(filters, content_hash)
        .await?;

    let hash_candidate_count = hash_matches.len();
    let mut matches = exact_duplicate_matches(hash_matches, content, content_hash, "hash_first");
    let hash_match_count = matches.len();
    let mut seen_ids: HashSet<String> = duplicate_ids_from_matches(&matches).into_iter().collect();

    if matches.is_empty() {
        let total = state.storage.count_memories_filtered(filters).await?;
        let existing = state.storage.list_memories(filters, total.max(1), 0).await?;
        matches = exact_duplicate_matches(existing, content, content_hash, "exact_content_fallback");
        let fallback_match_count = matches.len();
        return Ok((
            matches,
            lookup_diagnostics(
                hash_candidate_count,
                hash_match_count,
                true,
                fallback_match_count,
            ),
        ));
    }

    let mut fallback_match_count = 0;
    if !seen_ids.is_empty() {
        let total = state.storage.count_memories_filtered(filters).await?;
        let existing = state.storage.list_memories(filters, total.max(1), 0).await?;
        for candidate in exact_duplicate_matches(existing, content, content_hash, "exact_content_fallback") {
            if let Some(id) = candidate["id"].as_str() {
                if seen_ids.insert(id.to_string()) {
                    fallback_match_count += 1;
                    matches.push(candidate);
                }
            }
        }
    }

    Ok((
        matches,
        lookup_diagnostics(
            hash_candidate_count,
            hash_match_count,
            fallback_match_count > 0,
            fallback_match_count,
        ),
    ))
}

fn duplicate_ids_from_matches(matches: &[serde_json::Value]) -> Vec<String> {
    matches
        .iter()
        .filter_map(|value| value["id"].as_str().map(ToOwned::to_owned))
        .collect()
}

fn attention_summary(
    matched_count: usize,
    superseded_count: Option<usize>,
    replacement_lineage: Option<&serde_json::Value>,
    fingerprint_checked: bool,
) -> serde_json::Value {
    let lineage_depth = replacement_lineage
        .and_then(|value| value["depth"].as_u64())
        .unwrap_or(0);
    let cycle_detected = replacement_lineage
        .and_then(|value| value["cycle_detected"].as_bool())
        .unwrap_or(false);
    let truncated = replacement_lineage
        .and_then(|value| value["truncated"].as_bool())
        .unwrap_or(false);

    let mut attention_flags = Vec::new();
    if matched_count > 1 {
        attention_flags.push("multiple_matches");
    }
    if superseded_count.is_some_and(|count| count < matched_count) {
        attention_flags.push("partial_supersede");
    }
    if cycle_detected {
        attention_flags.push("lineage_cycle_detected");
    }
    if truncated {
        attention_flags.push("lineage_truncated");
    }

    let requires_operator_attention = !attention_flags.is_empty();

    json!({
        "requires_operator_attention": requires_operator_attention,
        "attention_flags": attention_flags,
        "multiple_matches": matched_count > 1,
        "partial_supersede": superseded_count.is_some_and(|count| count < matched_count),
        "lineage_cycle_detected": cycle_detected,
        "lineage_truncated": truncated,
        "lineage_depth": lineage_depth,
        "fingerprint_checked": fingerprint_checked,
    })
}

fn operator_summary(
    stage: &str,
    attention_summary: &serde_json::Value,
    plan_diagnostics: Option<&serde_json::Value>,
    lookup_diagnostics: Option<&serde_json::Value>,
    consolidation_trace: Option<&serde_json::Value>,
    replacement_lineage: Option<&serde_json::Value>,
) -> serde_json::Value {
    let requires_operator_attention = attention_summary["requires_operator_attention"]
        .as_bool()
        .unwrap_or(false);
    let lifecycle_status = consolidation_trace
        .and_then(|value| value["status"].as_str())
        .unwrap_or("not_applicable");
    let lineage_depth = replacement_lineage
        .and_then(|value| value["depth"].as_u64())
        .unwrap_or(0);

    let primary_signal = if requires_operator_attention {
        "attention_summary"
    } else if plan_diagnostics.is_some() {
        "plan_diagnostics"
    } else if lookup_diagnostics.is_some() {
        "lookup_diagnostics"
    } else if consolidation_trace.is_some() {
        "consolidation_trace"
    } else {
        "none"
    };

    let mut available_sections = Vec::new();
    if plan_diagnostics.is_some() {
        available_sections.push("plan_diagnostics");
    }
    if lookup_diagnostics.is_some() {
        available_sections.push("lookup_diagnostics");
    }
    if consolidation_trace.is_some() {
        available_sections.push("consolidation_trace");
    }
    if replacement_lineage.is_some() {
        available_sections.push("replacement_lineage");
    }
    available_sections.push("attention_summary");

    json!({
        "stage": stage,
        "primary_signal": primary_signal,
        "requires_operator_attention": requires_operator_attention,
        "attention_flags": attention_summary["attention_flags"].clone(),
        "lifecycle_status": lifecycle_status,
        "lineage_depth": lineage_depth,
        "available_sections": available_sections,
    })
}

fn consolidation_trace(memory: &Memory) -> serde_json::Value {
    let invalidated = memory.valid_until.is_some() || memory.invalidation_reason.is_some();
    let replacement_kind = match (invalidated, memory.superseded_by.as_ref()) {
        (true, Some(_)) => "superseded",
        (true, None) => "invalidated",
        (false, Some(_)) => "replacement_linked",
        (false, None) => "active",
    };

    json!({
        "status": replacement_kind,
        "invalidated": invalidated,
        "invalidation_reason": memory.invalidation_reason,
        "superseded_by": memory.superseded_by,
        "has_replacement": memory.superseded_by.is_some(),
    })
}

async fn replacement_lineage(state: &Arc<AppState>, memory: &Memory) -> serde_json::Value {
    const MAX_LINEAGE_DEPTH: usize = 16;

    let Some(first_replacement) = memory.superseded_by.clone() else {
        return json!({
            "chain_ids": [],
            "depth": 0,
            "terminal_replacement_id": serde_json::Value::Null,
            "cycle_detected": false,
            "truncated": false,
        });
    };

    let mut chain_ids = Vec::new();
    let mut seen = HashSet::new();
    let mut current_id = Some(first_replacement);
    let mut cycle_detected = false;
    let mut truncated = false;

    while let Some(id) = current_id.take() {
        if !seen.insert(id.clone()) {
            cycle_detected = true;
            break;
        }

        chain_ids.push(id.clone());

        if chain_ids.len() >= MAX_LINEAGE_DEPTH {
            truncated = true;
            break;
        }

        current_id = match state.storage.get_memory(&id).await {
            Ok(Some(next_memory)) => next_memory.superseded_by.clone(),
            Ok(None) | Err(_) => None,
        };
    }

    let terminal_replacement_id = chain_ids.last().cloned();

    json!({
        "chain_ids": chain_ids,
        "depth": chain_ids.len(),
        "terminal_replacement_id": terminal_replacement_id,
        "cycle_detected": cycle_detected,
        "truncated": truncated,
    })
}

async fn memory_with_trace(state: &Arc<AppState>, memory: &Memory) -> serde_json::Value {
    let mut value = serde_json::to_value(memory).unwrap_or_default();
    let lineage = replacement_lineage(state, memory).await;
    let trace = consolidation_trace(memory);
    let attention = attention_summary(0, None, Some(&lineage), false);
    if let serde_json::Value::Object(ref mut map) = value {
        map.insert("consolidation_trace".to_string(), trace.clone());
        map.insert("replacement_lineage".to_string(), lineage.clone());
        map.insert("attention_summary".to_string(), attention.clone());
        map.insert(
            "operator_summary".to_string(),
            operator_summary(
                "read",
                &attention,
                None,
                None,
                Some(&trace),
                Some(&lineage),
            ),
        );
    }
    value
}

async fn memories_with_trace(state: &Arc<AppState>, memories: &[Memory]) -> Vec<serde_json::Value> {
    let mut traced = Vec::with_capacity(memories.len());
    for memory in memories {
        traced.push(memory_with_trace(state, memory).await);
    }
    traced
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

    let (duplicate_matches, lookup_diagnostics) = match find_duplicate_matches(state, &filters, &params.content, &content_hash).await {
        Ok(value) => value,
        Err(e) => return Ok(error_response(e)),
    };
    let duplicate_ids = duplicate_ids_from_matches(&duplicate_matches);

    let reason = resolved_consolidation_reason(params.reason.as_deref());
    let plan_fingerprint = build_consolidation_plan_fingerprint(
        &content_hash,
        &filters,
        &duplicate_ids,
        reason,
        &mem_type,
        params.user_id.as_deref(),
        params.agent_id.as_deref(),
        params.run_id.as_deref(),
        params.namespace.as_deref(),
        importance_score,
        params.metadata.as_ref(),
    );
    let plan_diagnostics = plan_diagnostics(
        &content_hash,
        &filters,
        &duplicate_ids,
        reason,
        &mem_type,
        params.user_id.as_deref(),
        params.agent_id.as_deref(),
        params.run_id.as_deref(),
        params.namespace.as_deref(),
        importance_score,
        params.metadata.as_ref(),
    );

    if let Some(expected) = params
        .expected_plan_fingerprint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if expected != plan_fingerprint {
            return Ok(error_response(format!(
                "Consolidation preview is stale. expected_plan_fingerprint does not match current plan (expected={}, current={}). Re-run preview_consolidate_memory and inspect plan_diagnostics.",
                expected, plan_fingerprint
            )));
        }
    }
    let fingerprint_checked = params
        .expected_plan_fingerprint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();

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

    let mut superseded_ids = Vec::new();
    let mut skipped_ids = Vec::new();
    for duplicate_id in duplicate_ids {
        if duplicate_id == replacement_id {
            skipped_ids.push(duplicate_id);
            continue;
        }
        match invalidate_and_sync_memory(state, &duplicate_id, Some(reason), Some(&replacement_id)).await {
            Ok(true) => superseded_ids.push(duplicate_id),
            Ok(false) => {}
            Err(e) => return Ok(error_response(e)),
        }
    }

    let attention = attention_summary(
        duplicate_matches.len(),
        Some(superseded_ids.len()),
        None,
        fingerprint_checked,
    );

    Ok(success_json(json!({
        "id": replacement_id,
        "content_hash": content_hash,
        "plan_fingerprint": plan_fingerprint,
        "plan_diagnostics": plan_diagnostics,
        "superseded_ids": superseded_ids,
        "superseded_count": superseded_ids.len(),
        "filters": filters.describe(),
        "reason": reason,
        "lookup_diagnostics": lookup_diagnostics,
        "matched_summary": duplicate_matches,
        "execution_summary": {
            "replacement_id": replacement_id,
            "attempted_match_count": duplicate_matches.len(),
            "superseded_ids": superseded_ids,
            "superseded_count": superseded_ids.len(),
            "skipped_ids": skipped_ids,
            "used_plan_fingerprint": plan_fingerprint,
        },
        "attention_summary": attention.clone(),
        "operator_summary": operator_summary(
            "apply",
            &attention,
            Some(&plan_diagnostics),
            Some(&lookup_diagnostics),
            None,
            None,
        )
    })))
}

pub async fn preview_consolidate_memory(
    state: &Arc<AppState>,
    params: PreviewConsolidateMemoryParams,
) -> anyhow::Result<CallToolResult> {
    let filters = match params.to_memory_query() {
        Ok(filters) => filters,
        Err(e) => return Ok(error_response(e)),
    };
    let importance_score = match normalize_importance_score(params.importance_score) {
        Ok(Some(score)) => score,
        Ok(None) => 1.0,
        Err(e) => return Ok(error_response(e)),
    };
    let memory_type = match parse_memory_type(params.memory_type.as_deref()) {
        Ok(memory_type) => memory_type,
        Err(e) => return Ok(error_response(e)),
    };
    let content_hash = ContentHasher::hash(&params.content);
    let (matched_summary, lookup_diagnostics) = match find_duplicate_matches(state, &filters, &params.content, &content_hash).await {
        Ok(value) => value,
        Err(e) => return Ok(error_response(e)),
    };
    let matched_ids = duplicate_ids_from_matches(&matched_summary);
    let reason = resolved_consolidation_reason(params.reason.as_deref());
    let plan_fingerprint = build_consolidation_plan_fingerprint(
        &content_hash,
        &filters,
        &matched_ids,
        reason,
        &memory_type,
        params.user_id.as_deref(),
        params.agent_id.as_deref(),
        params.run_id.as_deref(),
        params.namespace.as_deref(),
        importance_score,
        params.metadata.as_ref(),
    );
    let plan_diagnostics = plan_diagnostics(
        &content_hash,
        &filters,
        &matched_ids,
        reason,
        &memory_type,
        params.user_id.as_deref(),
        params.agent_id.as_deref(),
        params.run_id.as_deref(),
        params.namespace.as_deref(),
        importance_score,
        params.metadata.as_ref(),
    );

    let attention = attention_summary(matched_ids.len(), None, None, false);

    Ok(success_json(json!({
        "mode": "preview",
        "content_hash": content_hash,
        "plan_fingerprint": plan_fingerprint,
        "plan_diagnostics": plan_diagnostics,
        "matched_ids": matched_ids,
        "matched_count": matched_ids.len(),
        "matched_summary": matched_summary,
        "filters": filters.describe(),
        "reason": reason,
        "lookup_diagnostics": lookup_diagnostics,
        "replacement": {
            "memory_type": serde_json::to_value(&memory_type).unwrap_or(serde_json::Value::Null),
            "user_id": params.user_id,
            "agent_id": params.agent_id,
            "run_id": params.run_id,
            "namespace": params.namespace,
            "importance_score": importance_score,
            "metadata": params.metadata,
        },
        "attention_summary": attention.clone(),
        "operator_summary": operator_summary(
            "preview",
            &attention,
            Some(&plan_diagnostics),
            Some(&lookup_diagnostics),
            None,
            None,
        ),
        "notes": [
            "Preview does not write any data.",
            "Exact duplicates are matched by content_hash or exact content equality inside the same optional scope/type boundary.",
            "lookup_diagnostics explains whether matching came from hash-first narrowing or exact-content fallback for legacy no-hash data.",
            "matched_summary explains why each candidate matched, and plan_fingerprint can be supplied back to consolidate_memory for alignment checking.",
            "plan_diagnostics echoes the normalized fingerprint inputs so operators can explain why a plan changed before apply."
        ]
    })))
}

pub async fn get_memory(
    state: &Arc<AppState>,
    params: GetMemoryParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.get_memory(&params.id).await {
        Ok(Some(mut memory)) => {
            strip_embedding(&mut memory);
            let memory_json = memory_with_trace(state, &memory).await;
            let mut response = json!({
                "memory": memory_json.clone(),
                "summary": summary_collection_response("memory", 1, Some(1), false, None),
                "contract": memory_contract_json(Some(&params.id))
            });

            if let (Some(response_map), Some(memory_map)) = (
                response.as_object_mut(),
                memory_json.as_object(),
            ) {
                for (key, value) in memory_map {
                    response_map.insert(key.clone(), value.clone());
                }
            }

            Ok(success_json(response))
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
    let memories_json = memories_with_trace(state, &memories).await;

    Ok(success_json(json!({
        "memories": memories_json,
        "summary": summary_collection_response("collection", memories.len(), Some(total), false, None),
        "contract": memory_collection_contract_json(),
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
            let memories_json = memories_with_trace(state, &memories).await;
            Ok(success_json(json!({
                "memories": memories_json,
                "summary": summary_collection_response("collection", memories.len(), Some(memories.len()), false, None),
                "contract": memory_collection_contract_json(),
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
            let memories_json = memories_with_trace(state, &memories).await;
            Ok(success_json(json!({
                "memories": memories_json,
                "summary": summary_collection_response("collection", memories.len(), Some(memories.len()), false, None),
                "contract": memory_collection_contract_json(),
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

    fn lineage_ids(trace_value: &serde_json::Value) -> Vec<String> {
        trace_value["replacement_lineage"]["chain_ids"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

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
        assert_eq!(memory_json["memory"]["content"], "Logic test memory");
        assert_eq!(memory_json["memory"]["agent_id"], "agent-a");
        assert_eq!(memory_json["memory"]["namespace"], "project-alpha");
        assert_eq!(memory_json["memory"]["importance_score"], 2.5);
        assert_eq!(memory_json["memory"]["consolidation_trace"]["status"], "active");
        assert_eq!(memory_json["memory"]["operator_summary"]["stage"], "read");
        assert_eq!(memory_json["memory"]["operator_summary"]["primary_signal"], "consolidation_trace");
        assert_eq!(memory_json["contract"]["compatibility"]["mode"], "additive_first");
        assert_eq!(memory_json["contract"]["compatibility"]["clients_must_ignore_unknown_fields"], true);
        assert_eq!(memory_json["summary"]["result_kind"], "memory");
        assert!(memory_json["summary"]["partial"]["reason_code"].is_null());
        assert!(memory_json["summary"]["partial"]["reason"].is_null());
        assert_eq!(memory_json["contract"]["identity"]["stable_memory_id"], id);
        assert_eq!(memory_json["contract"]["identity"]["node_id_semantics"], "stable_public_memory_id");

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
        assert_eq!(invalidated_json["memory"]["superseded_by"], "replacement-123");
        assert_eq!(invalidated_json["memory"]["consolidation_trace"]["status"], "superseded");
        assert_eq!(invalidated_json["memory"]["consolidation_trace"]["has_replacement"], true);
        assert_eq!(invalidated_json["memory"]["replacement_lineage"]["depth"], 1);
        assert_eq!(invalidated_json["memory"]["replacement_lineage"]["terminal_replacement_id"], "replacement-123");
        assert_eq!(invalidated_json["memory"]["attention_summary"]["lineage_depth"], 1);
        assert_eq!(invalidated_json["memory"]["attention_summary"]["requires_operator_attention"], false);
        assert_eq!(invalidated_json["memory"]["operator_summary"]["stage"], "read");
        assert_eq!(invalidated_json["memory"]["operator_summary"]["lifecycle_status"], "superseded");
        assert_eq!(lineage_ids(&invalidated_json["memory"]), vec!["replacement-123".to_string()]);

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
        assert_eq!(list_json["contract"]["compatibility"]["mode"], "additive_first");
        assert_eq!(list_json["summary"]["result_kind"], "collection");
        assert!(list_json["summary"]["partial"]["reason_code"].is_null());
        assert!(list_json["summary"]["partial"]["reason"].is_null());
        assert_eq!(list_json["contract"]["identity"]["node_id_semantics"], "stable_public_memory_id");
    }

    #[tokio::test]
    async fn memory_collection_surfaces_expose_contract_metadata() {
        let ctx = TestContext::new().await;

        let store_result = store_memory(
            &ctx.state,
            StoreMemoryParams {
                content: "Contract memory item".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-contract".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-contract".to_string()),
                importance_score: Some(1.0),
                metadata: None,
            },
        )
        .await
        .unwrap();
        let store_val = serde_json::to_value(&store_result).unwrap();
        let store_text = store_val["content"][0]["text"].as_str().unwrap();
        let store_json: serde_json::Value = serde_json::from_str(store_text).unwrap();
        let id = store_json["id"].as_str().unwrap().to_string();

        let get_result = get_memory(&ctx.state, GetMemoryParams { id: id.clone() }).await.unwrap();
        let get_val = serde_json::to_value(&get_result).unwrap();
        let get_text = get_val["content"][0]["text"].as_str().unwrap();
        let get_json: serde_json::Value = serde_json::from_str(get_text).unwrap();
        assert_eq!(get_json["contract"]["schema_version"], 1);
        assert_eq!(get_json["contract"]["identity"]["stable_memory_id"], id);
        assert_eq!(get_json["contract"]["identity"]["node_id_semantics"], "stable_public_memory_id");
        assert_eq!(get_json["contract"]["compatibility"]["mode"], "additive_first");
        assert!(get_json["summary"]["partial"]["reason_code"].is_null());
        assert!(get_json["summary"]["partial"]["reason"].is_null());
        assert_eq!(get_json["summary"]["result_kind"], "memory");

        let list_result = list_memories(
            &ctx.state,
            ListMemoriesParams {
                limit: Some(10),
                offset: None,
                user_id: Some("user-contract".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-contract".to_string()),
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
        let list_val = serde_json::to_value(&list_result).unwrap();
        let list_text = list_val["content"][0]["text"].as_str().unwrap();
        let list_json: serde_json::Value = serde_json::from_str(list_text).unwrap();
        assert_eq!(list_json["contract"]["schema_version"], 1);
        assert_eq!(list_json["contract"]["identity"]["stable_node_ids"], true);
        assert_eq!(list_json["contract"]["identity"]["node_ids_are_project_scoped"], false);
        assert_eq!(list_json["contract"]["identity"]["node_id_semantics"], "stable_public_memory_id");
        assert_eq!(list_json["contract"]["compatibility"]["clients_must_ignore_unknown_fields"], true);
        assert_eq!(list_json["summary"]["result_kind"], "collection");
        assert_eq!(list_json["summary"]["counts"]["results"], 1);

        let valid_result = get_valid(
            &ctx.state,
            GetValidParams {
                limit: Some(10),
                timestamp: None,
                user_id: Some("user-contract".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-contract".to_string()),
                memory_type: None,
                metadata_filter: None,
                event_after: None,
                event_before: None,
                ingestion_after: None,
                ingestion_before: None,
            },
        )
        .await
        .unwrap();
        let valid_val = serde_json::to_value(&valid_result).unwrap();
        let valid_text = valid_val["content"][0]["text"].as_str().unwrap();
        let valid_json: serde_json::Value = serde_json::from_str(valid_text).unwrap();
        assert_eq!(valid_json["contract"]["identity"]["node_id_semantics"], "stable_public_memory_id");
        assert_eq!(valid_json["summary"]["result_kind"], "collection");
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
            expected_plan_fingerprint: None,
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
        assert!(consolidate_json["plan_fingerprint"].as_str().is_some());
        assert_eq!(
            consolidate_json["plan_diagnostics"]["content_hash"],
            consolidate_json["content_hash"]
        );
        assert_eq!(
            consolidate_json["plan_diagnostics"]["matched_count"],
            1
        );
        assert_eq!(
            consolidate_json["plan_diagnostics"]["matched_ids"][0],
            original_id
        );
        assert_eq!(
            consolidate_json["plan_diagnostics"]["replacement"]["memory_type"],
            "semantic"
        );
        assert_eq!(consolidate_json["matched_summary"][0]["id"], original_id);
        assert!(consolidate_json["matched_summary"][0]["matched_by"]
            .as_array()
            .unwrap()
            .len()
            >= 1);
        assert_eq!(consolidate_json["execution_summary"]["replacement_id"], replacement_id);
        assert_eq!(consolidate_json["execution_summary"]["used_plan_fingerprint"], consolidate_json["plan_fingerprint"]);
        assert_eq!(consolidate_json["attention_summary"]["requires_operator_attention"], false);
        assert_eq!(consolidate_json["attention_summary"]["fingerprint_checked"], false);
        assert_eq!(consolidate_json["lookup_diagnostics"]["used_hash_first"], true);
        assert_eq!(consolidate_json["lookup_diagnostics"]["used_exact_content_fallback"], false);
        assert_eq!(consolidate_json["operator_summary"]["stage"], "apply");
        assert_eq!(consolidate_json["operator_summary"]["primary_signal"], "plan_diagnostics");

        let result = get_memory(&ctx.state, GetMemoryParams { id: original_id.clone() })
            .await
            .unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let old_memory_json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(old_memory_json["superseded_by"], replacement_id);
        assert_eq!(old_memory_json["invalidation_reason"], "deduplicated");
        assert_eq!(old_memory_json["consolidation_trace"]["status"], "superseded");
        assert_eq!(old_memory_json["replacement_lineage"]["depth"], 1);
        assert_eq!(old_memory_json["replacement_lineage"]["terminal_replacement_id"], replacement_id);
        assert_eq!(old_memory_json["operator_summary"]["lifecycle_status"], "superseded");
        assert_eq!(lineage_ids(&old_memory_json), vec![replacement_id.clone()]);

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
        assert_eq!(memories[0]["consolidation_trace"]["status"], "active");
        assert_eq!(memories[0]["replacement_lineage"]["depth"], 0);
        assert_eq!(memories[0]["operator_summary"]["stage"], "read");
    }

    #[tokio::test]
    async fn test_preview_consolidate_memory_logic() {
        let ctx = TestContext::new().await;

        let original = StoreMemoryParams {
            content: "Preview duplicate content".to_string(),
            memory_type: Some("semantic".to_string()),
            user_id: Some("user-preview".to_string()),
            agent_id: None,
            run_id: None,
            namespace: Some("project-preview".to_string()),
            importance_score: Some(1.0),
            metadata: None,
        };
        let result = store_memory(&ctx.state, original).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let original_json: serde_json::Value = serde_json::from_str(text).unwrap();
        let original_id = original_json["id"].as_str().unwrap().to_string();

        let preview_params = PreviewConsolidateMemoryParams {
            content: "Preview duplicate content".to_string(),
            memory_type: Some("semantic".to_string()),
            user_id: Some("user-preview".to_string()),
            agent_id: None,
            run_id: None,
            namespace: Some("project-preview".to_string()),
            importance_score: Some(2.0),
            reason: Some("deduplicated".to_string()),
            metadata: Some(json!({"source": "preview"})),
        };

        let result = preview_consolidate_memory(&ctx.state, preview_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let preview_json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(preview_json["mode"], "preview");
        assert_eq!(preview_json["matched_count"], 1);
        assert_eq!(preview_json["matched_ids"][0], original_id);
        assert_eq!(preview_json["matched_summary"][0]["id"], original_id);
        let matched_by = preview_json["matched_summary"][0]["matched_by"]
            .as_array()
            .unwrap();
        assert!(matched_by.iter().any(|value| value == "content_hash" || value == "exact_content"));
        assert_eq!(preview_json["reason"], "deduplicated");
        let fingerprint = preview_json["plan_fingerprint"].as_str().unwrap().to_string();
        assert_eq!(preview_json["lookup_diagnostics"]["used_hash_first"], true);
        assert_eq!(preview_json["lookup_diagnostics"]["used_exact_content_fallback"], false);
        assert_eq!(preview_json["plan_diagnostics"]["content_hash"], preview_json["content_hash"]);
        assert_eq!(preview_json["plan_diagnostics"]["matched_count"], 1);
        assert_eq!(preview_json["plan_diagnostics"]["matched_ids"][0], original_id);
        assert_eq!(preview_json["plan_diagnostics"]["replacement"]["memory_type"], "semantic");
        assert_eq!(preview_json["plan_diagnostics"]["replacement"]["importance_score"], 2.0);
        assert_eq!(preview_json["replacement"]["memory_type"], "semantic");
        assert_eq!(preview_json["replacement"]["importance_score"], 2.0);
        assert_eq!(preview_json["attention_summary"]["requires_operator_attention"], false);
        assert_eq!(preview_json["attention_summary"]["multiple_matches"], false);
        assert_eq!(preview_json["operator_summary"]["stage"], "preview");
        assert_eq!(preview_json["operator_summary"]["primary_signal"], "plan_diagnostics");

        let preview_list_result = list_memories(
            &ctx.state,
            ListMemoriesParams {
                limit: Some(10),
                offset: None,
                user_id: Some("user-preview".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-preview".to_string()),
                memory_type: Some("semantic".to_string()),
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
        let preview_list_val = serde_json::to_value(&preview_list_result).unwrap();
        let preview_list_text = preview_list_val["content"][0]["text"].as_str().unwrap();
        let preview_list_json: serde_json::Value = serde_json::from_str(preview_list_text).unwrap();
        assert_eq!(preview_list_json["total"], 1);

        let preview_get_result =
            get_memory(&ctx.state, GetMemoryParams { id: original_id.clone() }).await.unwrap();
        let preview_get_val = serde_json::to_value(&preview_get_result).unwrap();
        let preview_get_text = preview_get_val["content"][0]["text"].as_str().unwrap();
        let original_after_preview: serde_json::Value = serde_json::from_str(preview_get_text).unwrap();
        assert!(original_after_preview["superseded_by"].is_null());
        assert!(original_after_preview["invalidation_reason"].is_null());
        assert_eq!(original_after_preview["consolidation_trace"]["status"], "active");
        assert_eq!(original_after_preview["replacement_lineage"]["depth"], 0);
        assert_eq!(original_after_preview["operator_summary"]["lifecycle_status"], "active");

        let execute_result = consolidate_memory(
            &ctx.state,
            ConsolidateMemoryParams {
                content: "Preview duplicate content".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-preview".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-preview".to_string()),
                importance_score: Some(2.0),
                reason: Some("deduplicated".to_string()),
                expected_plan_fingerprint: Some(fingerprint.clone()),
                metadata: Some(json!({"source": "preview"})),
            },
        )
        .await
        .unwrap();
        let execute_val = serde_json::to_value(&execute_result).unwrap();
        let execute_text = execute_val["content"][0]["text"].as_str().unwrap();
        let execute_json: serde_json::Value = serde_json::from_str(execute_text).unwrap();
        assert_eq!(execute_json["plan_fingerprint"], fingerprint);
        assert_eq!(execute_json["execution_summary"]["used_plan_fingerprint"], fingerprint);
        assert_eq!(execute_json["execution_summary"]["superseded_ids"][0], original_id);
        assert_eq!(execute_json["attention_summary"]["fingerprint_checked"], true);
        assert_eq!(execute_json["attention_summary"]["requires_operator_attention"], false);
        assert_eq!(execute_json["operator_summary"]["stage"], "apply");
        assert_eq!(execute_json["operator_summary"]["primary_signal"], "plan_diagnostics");

        let stale_result = consolidate_memory(
            &ctx.state,
            ConsolidateMemoryParams {
                content: "Preview duplicate content".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-preview".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-preview".to_string()),
                importance_score: Some(2.0),
                reason: Some("deduplicated".to_string()),
                expected_plan_fingerprint: Some("stale-plan".to_string()),
                metadata: Some(json!({"source": "preview"})),
            },
        )
        .await
        .unwrap();
        let stale_val = serde_json::to_value(&stale_result).unwrap();
        let stale_text = stale_val["content"][0]["text"].as_str().unwrap();
        assert!(stale_text.contains("Consolidation preview is stale"));

        let result = list_memories(
            &ctx.state,
            ListMemoriesParams {
                limit: Some(10),
                offset: None,
                user_id: Some("user-preview".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-preview".to_string()),
                memory_type: Some("semantic".to_string()),
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
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let list_json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(list_json["total"], 1);

        let result = get_memory(&ctx.state, GetMemoryParams { id: original_id }).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let original_after_preview: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(original_after_preview["superseded_by"].is_string());
        assert_eq!(original_after_preview["invalidation_reason"], "deduplicated");
        assert_eq!(original_after_preview["consolidation_trace"]["status"], "superseded");
        assert_eq!(original_after_preview["replacement_lineage"]["depth"], 1);
        assert_eq!(original_after_preview["replacement_lineage"]["terminal_replacement_id"], execute_json["id"]);
        assert_eq!(original_after_preview["operator_summary"]["lifecycle_status"], "superseded");
    }

    #[tokio::test]
    async fn test_content_hash_lifecycle_contract() {
        let ctx = TestContext::new().await;

        let store_result = store_memory(
            &ctx.state,
            StoreMemoryParams {
                content: "Lifecycle original content".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-hash".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-hash".to_string()),
                importance_score: Some(1.0),
                metadata: None,
            },
        )
        .await
        .unwrap();
        let store_val = serde_json::to_value(&store_result).unwrap();
        let store_text = store_val["content"][0]["text"].as_str().unwrap();
        let store_json: serde_json::Value = serde_json::from_str(store_text).unwrap();
        let id = store_json["id"].as_str().unwrap().to_string();

        let get_initial = get_memory(&ctx.state, GetMemoryParams { id: id.clone() })
            .await
            .unwrap();
        let get_initial_val = serde_json::to_value(&get_initial).unwrap();
        let get_initial_text = get_initial_val["content"][0]["text"].as_str().unwrap();
        let initial_json: serde_json::Value = serde_json::from_str(get_initial_text).unwrap();
        let initial_hash = initial_json["content_hash"].as_str().unwrap().to_string();
        assert!(!initial_hash.is_empty());

        let update_result = update_memory(
            &ctx.state,
            UpdateMemoryParams {
                id: id.clone(),
                content: Some("Lifecycle updated content".to_string()),
                memory_type: None,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                importance_score: None,
                metadata: None,
            },
        )
        .await
        .unwrap();
        let update_val = serde_json::to_value(&update_result).unwrap();
        let update_text = update_val["content"][0]["text"].as_str().unwrap();
        let update_json: serde_json::Value = serde_json::from_str(update_text).unwrap();
        let updated_hash = update_json["content_hash"].as_str().unwrap().to_string();
        assert_ne!(updated_hash, initial_hash);

        let preview_result = preview_consolidate_memory(
            &ctx.state,
            PreviewConsolidateMemoryParams {
                content: "Lifecycle updated content".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-hash".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-hash".to_string()),
                importance_score: Some(2.0),
                reason: Some("hash-lifecycle".to_string()),
                metadata: Some(json!({"source": "hash-lifecycle"})),
            },
        )
        .await
        .unwrap();
        let preview_val = serde_json::to_value(&preview_result).unwrap();
        let preview_text = preview_val["content"][0]["text"].as_str().unwrap();
        let preview_json: serde_json::Value = serde_json::from_str(preview_text).unwrap();
        let fingerprint = preview_json["plan_fingerprint"].as_str().unwrap().to_string();
        assert_eq!(preview_json["content_hash"], updated_hash);
        assert_eq!(preview_json["matched_count"], 1);
        assert_eq!(preview_json["matched_ids"][0], id);

        let execute_result = consolidate_memory(
            &ctx.state,
            ConsolidateMemoryParams {
                content: "Lifecycle updated content".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-hash".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-hash".to_string()),
                importance_score: Some(2.0),
                reason: Some("hash-lifecycle".to_string()),
                expected_plan_fingerprint: Some(fingerprint),
                metadata: Some(json!({"source": "hash-lifecycle"})),
            },
        )
        .await
        .unwrap();
        let execute_val = serde_json::to_value(&execute_result).unwrap();
        let execute_text = execute_val["content"][0]["text"].as_str().unwrap();
        let execute_json: serde_json::Value = serde_json::from_str(execute_text).unwrap();
        assert_eq!(execute_json["content_hash"], updated_hash);
        assert_eq!(execute_json["superseded_ids"][0], id);

        let old_memory = get_memory(&ctx.state, GetMemoryParams { id: id.clone() })
            .await
            .unwrap();
        let old_memory_val = serde_json::to_value(&old_memory).unwrap();
        let old_memory_text = old_memory_val["content"][0]["text"].as_str().unwrap();
        let old_memory_json: serde_json::Value = serde_json::from_str(old_memory_text).unwrap();
        assert_eq!(old_memory_json["content_hash"], updated_hash);
        assert_eq!(old_memory_json["superseded_by"], execute_json["id"]);
        assert_eq!(old_memory_json["invalidation_reason"], "hash-lifecycle");
    }

    #[tokio::test]
    async fn test_consolidate_memory_falls_back_when_existing_hash_missing() {
        let ctx = TestContext::new().await;

        let legacy_memory = Memory {
            content: "Legacy duplicate content".to_string(),
            embedding: Some(vec![0.0; 768]),
            memory_type: MemoryType::Semantic,
            user_id: Some("user-legacy".to_string()),
            namespace: Some("project-legacy".to_string()),
            content_hash: None,
            embedding_state: EmbeddingState::Ready,
            ..Default::default()
        };

        let legacy_id = create_and_sync_memory(&ctx.state, legacy_memory).await.unwrap();

        let result = consolidate_memory(
            &ctx.state,
            ConsolidateMemoryParams {
                content: "Legacy duplicate content".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-legacy".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-legacy".to_string()),
                importance_score: Some(2.0),
                reason: Some("legacy-fallback".to_string()),
                expected_plan_fingerprint: None,
                metadata: None,
            },
        )
        .await
        .unwrap();

        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let consolidate_json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(consolidate_json["superseded_count"], 1);
        assert_eq!(consolidate_json["superseded_ids"][0], legacy_id);
        assert_eq!(consolidate_json["lookup_diagnostics"]["used_hash_first"], false);
        assert_eq!(consolidate_json["lookup_diagnostics"]["used_exact_content_fallback"], true);
        assert_eq!(consolidate_json["lookup_diagnostics"]["exact_content_fallback_match_count"], 1);
        assert_eq!(consolidate_json["operator_summary"]["primary_signal"], "plan_diagnostics");
        assert!(consolidate_json["matched_summary"][0]["matched_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "exact_content"));

        let legacy_after = get_memory(&ctx.state, GetMemoryParams { id: legacy_id.clone() })
            .await
            .unwrap();
        let legacy_after_val = serde_json::to_value(&legacy_after).unwrap();
        let legacy_after_text = legacy_after_val["content"][0]["text"].as_str().unwrap();
        let legacy_after_json: serde_json::Value = serde_json::from_str(legacy_after_text).unwrap();

        assert_eq!(legacy_after_json["superseded_by"], consolidate_json["id"]);
        assert_eq!(legacy_after_json["invalidation_reason"], "legacy-fallback");
    }

    #[tokio::test]
    async fn test_store_memory_allows_exact_duplicates_with_same_hash() {
        let ctx = TestContext::new().await;

        let params = StoreMemoryParams {
            content: "store duplicate behavior".to_string(),
            memory_type: Some("semantic".to_string()),
            user_id: Some("user-store-dup".to_string()),
            agent_id: None,
            run_id: None,
            namespace: Some("project-store-dup".to_string()),
            importance_score: Some(1.0),
            metadata: None,
        };

        let first_result = store_memory(&ctx.state, params.clone()).await.unwrap();
        let first_val = serde_json::to_value(&first_result).unwrap();
        let first_text = first_val["content"][0]["text"].as_str().unwrap();
        let first_json: serde_json::Value = serde_json::from_str(first_text).unwrap();
        let first_id = first_json["id"].as_str().unwrap().to_string();

        let second_result = store_memory(&ctx.state, params).await.unwrap();
        let second_val = serde_json::to_value(&second_result).unwrap();
        let second_text = second_val["content"][0]["text"].as_str().unwrap();
        let second_json: serde_json::Value = serde_json::from_str(second_text).unwrap();
        let second_id = second_json["id"].as_str().unwrap().to_string();

        assert_ne!(first_id, second_id);

        let first_get = get_memory(&ctx.state, GetMemoryParams { id: first_id.clone() })
            .await
            .unwrap();
        let first_get_val = serde_json::to_value(&first_get).unwrap();
        let first_get_text = first_get_val["content"][0]["text"].as_str().unwrap();
        let first_memory_json: serde_json::Value = serde_json::from_str(first_get_text).unwrap();

        let second_get = get_memory(&ctx.state, GetMemoryParams { id: second_id.clone() })
            .await
            .unwrap();
        let second_get_val = serde_json::to_value(&second_get).unwrap();
        let second_get_text = second_get_val["content"][0]["text"].as_str().unwrap();
        let second_memory_json: serde_json::Value = serde_json::from_str(second_get_text).unwrap();

        assert_eq!(first_memory_json["content"], "store duplicate behavior");
        assert_eq!(second_memory_json["content"], "store duplicate behavior");
        assert_eq!(first_memory_json["content_hash"], second_memory_json["content_hash"]);

        let list_result = list_memories(
            &ctx.state,
            ListMemoriesParams {
                limit: Some(10),
                offset: None,
                user_id: Some("user-store-dup".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-store-dup".to_string()),
                memory_type: Some("semantic".to_string()),
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
        let list_val = serde_json::to_value(&list_result).unwrap();
        let list_text = list_val["content"][0]["text"].as_str().unwrap();
        let list_json: serde_json::Value = serde_json::from_str(list_text).unwrap();

        assert_eq!(list_json["total"], 2);
        assert_eq!(list_json["memories"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_consolidate_memory_handles_short_noise_like_content() {
        let ctx = TestContext::new().await;

        let original_result = store_memory(
            &ctx.state,
            StoreMemoryParams {
                content: ".".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-noise".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-noise".to_string()),
                importance_score: Some(1.0),
                metadata: None,
            },
        )
        .await
        .unwrap();
        let original_val = serde_json::to_value(&original_result).unwrap();
        let original_text = original_val["content"][0]["text"].as_str().unwrap();
        let original_json: serde_json::Value = serde_json::from_str(original_text).unwrap();
        let original_id = original_json["id"].as_str().unwrap().to_string();

        let preview_result = preview_consolidate_memory(
            &ctx.state,
            PreviewConsolidateMemoryParams {
                content: ".".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-noise".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-noise".to_string()),
                importance_score: Some(2.0),
                reason: Some("noise-dedup".to_string()),
                metadata: None,
            },
        )
        .await
        .unwrap();
        let preview_val = serde_json::to_value(&preview_result).unwrap();
        let preview_text = preview_val["content"][0]["text"].as_str().unwrap();
        let preview_json: serde_json::Value = serde_json::from_str(preview_text).unwrap();

        assert_eq!(preview_json["matched_count"], 1);
        assert_eq!(preview_json["matched_ids"][0], original_id);

        let consolidate_result = consolidate_memory(
            &ctx.state,
            ConsolidateMemoryParams {
                content: ".".to_string(),
                memory_type: Some("semantic".to_string()),
                user_id: Some("user-noise".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-noise".to_string()),
                importance_score: Some(2.0),
                reason: Some("noise-dedup".to_string()),
                expected_plan_fingerprint: None,
                metadata: None,
            },
        )
        .await
        .unwrap();
        let consolidate_val = serde_json::to_value(&consolidate_result).unwrap();
        let consolidate_text = consolidate_val["content"][0]["text"].as_str().unwrap();
        let consolidate_json: serde_json::Value = serde_json::from_str(consolidate_text).unwrap();

        assert_eq!(consolidate_json["superseded_count"], 1);
        assert_eq!(consolidate_json["superseded_ids"][0], original_id);
        assert_eq!(consolidate_json["lookup_diagnostics"]["used_hash_first"], true);
        assert_eq!(consolidate_json["lookup_diagnostics"]["used_exact_content_fallback"], false);
    }
}
