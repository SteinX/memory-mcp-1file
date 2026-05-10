use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::embedding::ContentHasher;
use crate::storage::StorageBackend;
use crate::server::params::{
    LearningMemoryArchiveParams, LearningMemoryCreateParams, LearningMemoryDeleteParams,
    LearningMemoryGetParams, LearningMemoryListParams, LearningMemoryMigrateLegacyParams,
    LearningMemoryPromoteParams, LearningMemoryRejectParams, LearningMemorySearchParams,
    LearningMemorySupersededParams, LearningMemoryUpdateParams,
};
use crate::types::{
    learning::{
        validate_learning_metadata, CreatedFrom, LearningKind, LearningMetadata, LearningScope,
        LearningSource, LearningStatus, ScopeLevel,
    },
    Datetime, EmbeddingState, ExportIdentity, Memory, MemoryQuery, MemoryType, MemoryUpdate,
    record_key_to_string,
};

use super::{
    error_response, normalize_limit, strip_embedding, strip_embeddings, success_json,
    success_serialize,
};
use super::learning_filters::{apply_learning_filter, default_list_filter, LearningFilter};
use super::learning_lifecycle::{derive_lifecycle_state, LearningLifecycleState};
use super::learning_response::build_learning_response;
use super::contracts::{export_contract_meta, summary_collection_response, with_surface_guidance};

// ─── memory_type mapping ──────────────────────────────────────────────────────

fn kind_status_to_memory_type(kind: &LearningKind, status: &LearningStatus) -> MemoryType {
    if *kind == LearningKind::WorkflowRule || *status == LearningStatus::Rule {
        return MemoryType::Procedural;
    }
    match kind {
        LearningKind::ProjectLesson => MemoryType::Episodic,
        LearningKind::UserPreference
        | LearningKind::ProjectPattern
        | LearningKind::ProjectPitfall => MemoryType::Semantic,
        LearningKind::WorkflowRule => MemoryType::Procedural,
    }
}

// ─── contract helpers ─────────────────────────────────────────────────────────

fn learning_contract_json(memory_id: Option<&str>) -> serde_json::Value {
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
        &["learning", "memory", "contract", "summary"],
        &["count", "total", "filters"],
        &[],
    );
    serde_json::to_value(contract).unwrap_or_else(|_| json!({}))
}

fn learning_collection_contract_json() -> serde_json::Value {
    learning_contract_json(None)
}

// ─── extract learning fields from a Memory record ────────────────────────────

fn extract_learning_fields(
    memory: &Memory,
) -> Option<(LearningKind, LearningStatus, LearningScope, u32)> {
    let meta = memory.metadata.as_ref()?;
    let learning_val = meta.get("learning")?;
    let lm: LearningMetadata = serde_json::from_value(learning_val.clone()).ok()?;
    Some((lm.kind, lm.status, lm.scope, lm.schema_version))
}

// ─── create ──────────────────────────────────────────────────────────────────

pub async fn create(
    state: &AppState,
    params: LearningMemoryCreateParams,
) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);

    let kind: LearningKind = match serde_json::from_value(json!(params.kind)) {
        Ok(k) => k,
        Err(_) => return Ok(error_response(format!("Invalid kind: '{}'", params.kind))),
    };

    let status: LearningStatus = match params.status.as_deref() {
        Some(s) => match serde_json::from_value(json!(s)) {
            Ok(st) => st,
            Err(_) => return Ok(error_response(format!("Invalid status: '{}'", s))),
        },
        None => LearningStatus::Candidate,
    };

    let scope_level: ScopeLevel = match params.scope.as_deref() {
        Some(s) => match serde_json::from_value(json!(s)) {
            Ok(sl) => sl,
            Err(_) => return Ok(error_response(format!("Invalid scope: '{}'", s))),
        },
        None => ScopeLevel::Global,
    };

    let created_from: CreatedFrom = match params.source.as_deref() {
        Some(s) => match serde_json::from_value(json!(s)) {
            Ok(cf) => cf,
            Err(_) => return Ok(error_response(format!("Invalid source: '{}'", s))),
        },
        None => CreatedFrom::Manual,
    };

    let confidence = params.confidence.unwrap_or(0.5) as f64;
    if !(0.0..=1.0).contains(&confidence) {
        return Ok(error_response("confidence must be in [0.0, 1.0]"));
    }

    let scope = LearningScope {
        level: scope_level,
        project_id: params.project_id.clone(),
        workspace: None,
        mode: None,
        user_id: None,
        agent_id: None,
        run_id: None,
        namespace: None,
    };

    let learning_meta = LearningMetadata {
        schema_version: 1,
        kind: kind.clone(),
        status: status.clone(),
        confidence,
        scope: scope.clone(),
        source: LearningSource {
            created_from,
            client: None,
            source_memory_ids: vec![],
        },
        evidence: params.evidence.unwrap_or_default(),
        applies_to: params.applies_to.unwrap_or_default(),
        trigger_hints: params.trigger_hints.unwrap_or_default(),
        supersedes: vec![],
        constraints: params.constraints.unwrap_or_default(),
    };

    let learning_val = match serde_json::to_value(&learning_meta) {
        Ok(v) => v,
        Err(e) => return Ok(error_response(e)),
    };
    if let Err(e) = validate_learning_metadata(&learning_val) {
        return Ok(error_response(e));
    }

    let metadata = json!({ "learning": learning_val });
    let memory_type = kind_status_to_memory_type(&kind, &status);
    let embedding = state.embedding.embed(&params.content).await?;
    let content_hash = ContentHasher::hash(&params.content);
    let now = Datetime::default();

    let memory = Memory {
        content: params.content,
        embedding: Some(embedding),
        memory_type,
        metadata: Some(metadata),
        event_time: now,
        ingestion_time: now,
        valid_from: now,
        importance_score: 1.0,
        content_hash: Some(content_hash),
        embedding_state: EmbeddingState::Ready,
        ..Default::default()
    };

    let id = match state.storage.create_memory(memory).await {
        Ok(id) => id,
        Err(e) => return Ok(error_response(e)),
    };

    if let Ok(Some(created)) = state.storage.get_memory(&id).await {
        state.memory_search.upsert_memory(created).await;
    }

    let stored = match state.storage.get_memory(&id).await {
        Ok(Some(m)) => m,
        Ok(None) => return Ok(error_response(format!("Memory not found after create: {id}"))),
        Err(e) => return Ok(error_response(e)),
    };

    let mut stored = stored;
    strip_embedding(&mut stored);
    let lifecycle_state = derive_lifecycle_state(&stored);
    let record_json = serde_json::to_value(&stored).unwrap_or(json!({}));
    let contract = learning_contract_json(Some(&id));
    let summary = summary_collection_response("memory", 1, Some(1), false, None);
    let summary_val = serde_json::to_value(summary).unwrap_or(json!({}));

    let response = build_learning_response(
        record_json,
        kind,
        status,
        scope,
        lifecycle_state,
        1,
        contract,
        summary_val,
    );

    Ok(success_serialize(&response))
}

// ─── get ─────────────────────────────────────────────────────────────────────

pub async fn get(
    state: &AppState,
    params: LearningMemoryGetParams,
) -> anyhow::Result<CallToolResult> {
    let mut memory = match state.storage.get_memory(&params.id).await {
        Ok(Some(m)) => m,
        Ok(None) => return Ok(error_response(format!("Memory not found: {}", params.id))),
        Err(e) => return Ok(error_response(e)),
    };

    strip_embedding(&mut memory);

    let (kind, status, scope, schema_version) = match extract_learning_fields(&memory) {
        Some(fields) => fields,
        None => {
            return Ok(error_response(format!(
                "Memory '{}' does not have valid learning metadata",
                params.id
            )))
        }
    };

    let lifecycle_state = derive_lifecycle_state(&memory);
    let record_json = serde_json::to_value(&memory).unwrap_or(json!({}));
    let contract = learning_contract_json(Some(&params.id));
    let summary = summary_collection_response("memory", 1, Some(1), false, None);
    let summary_val = serde_json::to_value(summary).unwrap_or(json!({}));

    let response = build_learning_response(
        record_json,
        kind,
        status,
        scope,
        lifecycle_state,
        schema_version,
        contract,
        summary_val,
    );

    Ok(success_serialize(&response))
}

// ─── list ─────────────────────────────────────────────────────────────────────

pub async fn list(
    state: &AppState,
    params: LearningMemoryListParams,
) -> anyhow::Result<CallToolResult> {
    let limit = normalize_limit(params.limit);
    let offset = params.offset.unwrap_or(0);

    let mut filter = default_list_filter();
    if let Some(filter_val) = params.filter {
        let caller_filter: LearningFilter = match serde_json::from_value(filter_val) {
            Ok(f) => f,
            Err(e) => return Ok(error_response(format!("Invalid filter: {e}"))),
        };
        if caller_filter.include_status.is_some() {
            filter.include_status = caller_filter.include_status;
        }
        if caller_filter.exclude_status.is_some() {
            filter.exclude_status = caller_filter.exclude_status;
        }
        if caller_filter.include_invalidated {
            filter.include_invalidated = true;
        }
        if caller_filter.audit {
            filter.audit = true;
        }
        filter.fallback = caller_filter.fallback;
    }

    let mut query = MemoryQuery::default();
    apply_learning_filter(&mut query, &filter);

    let mut memories = match state.storage.list_memories(&query, limit, offset).await {
        Ok(m) => m,
        Err(e) => return Ok(error_response(e)),
    };

    strip_embeddings(&mut memories);

    let total = state
        .storage
        .count_memories_filtered(&query)
        .await
        .unwrap_or(0);

    let records: Vec<serde_json::Value> = memories
        .iter()
        .filter_map(|memory| {
            let (kind, status, scope, schema_version) = extract_learning_fields(memory)?;
            let lifecycle_state = derive_lifecycle_state(memory);
            let record_json = serde_json::to_value(memory).ok()?;
            let response = build_learning_response(
                record_json,
                kind,
                status,
                scope,
                lifecycle_state,
                schema_version,
                json!({}),
                json!({}),
            );
            serde_json::to_value(response).ok()
        })
        .collect();

    let summary = summary_collection_response("collection", records.len(), Some(total), false, None);
    let summary_val = serde_json::to_value(summary).unwrap_or(json!({}));

    Ok(success_json(json!({
        "records": records,
        "summary": summary_val,
        "contract": learning_collection_contract_json(),
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

pub async fn search(
    _state: &AppState,
    _params: LearningMemorySearchParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_search: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn update(
    state: &AppState,
    params: LearningMemoryUpdateParams,
) -> anyhow::Result<CallToolResult> {
    let memory = match state.storage.get_memory(&params.id).await? {
        Some(m) => m,
        None => return Ok(error_response(format!("Record not found: {}", params.id))),
    };

    let lifecycle = derive_lifecycle_state(&memory);
    if matches!(
        lifecycle,
        LearningLifecycleState::Rejected
            | LearningLifecycleState::Archived
            | LearningLifecycleState::Superseded
            | LearningLifecycleState::Invalidated
    ) {
        return Ok(error_response(format!(
            "Cannot update record in lifecycle state {:?}",
            lifecycle
        )));
    }

    let mut learning_meta = memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .cloned()
        .unwrap_or(json!({}));

    if let Some(confidence) = params.confidence {
        if !(0.0..=1.0).contains(&confidence) {
            return Ok(error_response("confidence must be in [0.0, 1.0]"));
        }
        learning_meta["confidence"] = json!(confidence);
    }
    if let Some(evidence) = params.evidence {
        learning_meta["evidence"] = json!(evidence);
    }

    let mut full_metadata = memory.metadata.clone().unwrap_or(json!({}));
    full_metadata["learning"] = learning_meta;

    let update = MemoryUpdate {
        content: params.content.clone(),
        memory_type: None,
        user_id: None,
        agent_id: None,
        run_id: None,
        namespace: None,
        importance_score: None,
        metadata: Some(full_metadata),
        embedding: None,
        content_hash: None,
        embedding_state: None,
    };

    let mut updated = match state.storage.update_memory(&params.id, update).await {
        Ok(m) => m,
        Err(e) => return Ok(error_response(e)),
    };

    state.memory_search.upsert_memory(updated.clone()).await;
    strip_embedding(&mut updated);

    let learning_raw = updated
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .cloned()
        .unwrap_or(json!({}));

    let lm = match validate_learning_metadata(&learning_raw) {
        Ok(m) => m,
        Err(e) => return Ok(error_response(format!("Invalid learning metadata: {}", e))),
    };

    let lifecycle = derive_lifecycle_state(&updated);
    let record_value = serde_json::to_value(&updated).unwrap_or_default();
    let contract = learning_contract_json(
        updated.id.as_ref().map(|id| record_key_to_string(&id.key)).as_deref(),
    );
    let summary = json!({ "result_kind": "learning_memory", "counts": { "returned": 1 } });

    let resp = build_learning_response(
        record_value,
        lm.kind,
        lm.status,
        lm.scope,
        lifecycle,
        lm.schema_version,
        contract,
        summary,
    );

    Ok(success_serialize(&resp))
}

pub async fn promote(
    state: &AppState,
    params: LearningMemoryPromoteParams,
) -> anyhow::Result<CallToolResult> {
    let target_status: LearningStatus = match serde_json::from_value(json!(params.target_status)) {
        Ok(s) => s,
        Err(_) => {
            return Ok(error_response(format!(
                "Invalid target_status: '{}'. Must be 'confirmed' or 'rule'",
                params.target_status
            )));
        }
    };

    if !matches!(target_status, LearningStatus::Confirmed | LearningStatus::Rule) {
        return Ok(error_response(format!(
            "Invalid promotion target '{}': only 'confirmed' and 'rule' are valid",
            params.target_status
        )));
    }

    let memory = match state.storage.get_memory(&params.id).await? {
        Some(m) => m,
        None => return Ok(error_response(format!("Record not found: {}", params.id))),
    };

    let lifecycle = derive_lifecycle_state(&memory);
    if matches!(
        lifecycle,
        LearningLifecycleState::Rejected
            | LearningLifecycleState::Archived
            | LearningLifecycleState::Superseded
            | LearningLifecycleState::Invalidated
            | LearningLifecycleState::Unknown
    ) {
        return Ok(error_response(format!(
            "Cannot promote record in lifecycle state {:?}",
            lifecycle
        )));
    }

    let current_status = memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .and_then(|l| l.get("status"))
        .and_then(|s| serde_json::from_value::<LearningStatus>(s.clone()).ok())
        .unwrap_or(LearningStatus::Candidate);

    let allowed = matches!(
        (&current_status, &target_status),
        (LearningStatus::Candidate, LearningStatus::Confirmed)
            | (LearningStatus::Candidate, LearningStatus::Rule)
            | (LearningStatus::Confirmed, LearningStatus::Rule)
    );

    if !allowed {
        return Ok(error_response(format!(
            "Invalid promotion: {:?} → {:?} is not permitted",
            current_status, target_status
        )));
    }

    let mut learning_meta = memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .cloned()
        .unwrap_or(json!({}));

    learning_meta["status"] = serde_json::to_value(&target_status).unwrap_or(json!("confirmed"));

    if let Some(ref target_kind_str) = params.target_kind {
        let target_kind: LearningKind = match serde_json::from_value(json!(target_kind_str)) {
            Ok(k) => k,
            Err(_) => {
                return Ok(error_response(format!(
                    "Invalid target_kind: '{}'",
                    target_kind_str
                )));
            }
        };
        learning_meta["kind"] = serde_json::to_value(&target_kind).unwrap_or(json!("user_preference"));
    }

    let mut full_metadata = memory.metadata.clone().unwrap_or(json!({}));
    full_metadata["learning"] = learning_meta;

    let new_memory_type = if matches!(target_status, LearningStatus::Rule) {
        Some(MemoryType::Procedural)
    } else {
        None
    };

    let update = MemoryUpdate {
        content: None,
        memory_type: new_memory_type,
        user_id: None,
        agent_id: None,
        run_id: None,
        namespace: None,
        importance_score: None,
        metadata: Some(full_metadata),
        embedding: None,
        content_hash: None,
        embedding_state: None,
    };

    let mut updated = match state.storage.update_memory(&params.id, update).await {
        Ok(m) => m,
        Err(e) => return Ok(error_response(e)),
    };

    state.memory_search.upsert_memory(updated.clone()).await;
    strip_embedding(&mut updated);

    let learning_raw = updated
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .cloned()
        .unwrap_or(json!({}));

    let lm = match validate_learning_metadata(&learning_raw) {
        Ok(m) => m,
        Err(e) => return Ok(error_response(format!("Invalid learning metadata after promote: {}", e))),
    };

    let lifecycle = derive_lifecycle_state(&updated);
    let record_value = serde_json::to_value(&updated).unwrap_or_default();
    let contract = learning_contract_json(
        updated.id.as_ref().map(|id| record_key_to_string(&id.key)).as_deref(),
    );
    let summary = json!({ "result_kind": "learning_memory", "counts": { "returned": 1 } });

    let resp = build_learning_response(
        record_value,
        lm.kind,
        lm.status,
        lm.scope,
        lifecycle,
        lm.schema_version,
        contract,
        summary,
    );

    Ok(success_serialize(&resp))
}

pub async fn reject(
    _state: &AppState,
    _params: LearningMemoryRejectParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_reject: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn archive(
    _state: &AppState,
    _params: LearningMemoryArchiveParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_archive: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn supersede(
    _state: &AppState,
    _params: LearningMemorySupersededParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_supersede: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn migrate_legacy(
    _state: &AppState,
    _params: LearningMemoryMigrateLegacyParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_migrate_legacy: logic will be implemented in Tasks 6-9"
    })))
}

pub async fn delete(
    _state: &AppState,
    _params: LearningMemoryDeleteParams,
) -> anyhow::Result<CallToolResult> {
    Ok(success_json(serde_json::json!({
        "status": "not_implemented",
        "message": "learning_memory_delete: compatibility shim; logic will be implemented in Tasks 6-9"
    })))
}

#[cfg(test)]
mod tests {
    use crate::server::logic::learning_lifecycle::{derive_lifecycle_state, LearningLifecycleState};
    use crate::types::{learning::{LearningKind, LearningStatus}, Memory, MemoryType};
    use serde_json::json;

    use super::{extract_learning_fields, kind_status_to_memory_type};

    fn make_memory_with_status(status: LearningStatus) -> Memory {
        let mut m = Memory::new("test content".to_string());
        m.metadata = Some(json!({
            "learning": {
                "schema_version": 1,
                "kind": "user_preference",
                "status": serde_json::to_value(&status).unwrap(),
                "confidence": 0.8,
                "scope": { "level": "global" },
                "source": { "created_from": "manual" }
            }
        }));
        m
    }

    fn make_invalidated_memory(reason: &str) -> Memory {
        let mut m = make_memory_with_status(LearningStatus::Candidate);
        m.valid_until = Some(surrealdb::types::Datetime::from(chrono::Utc::now()));
        m.invalidation_reason = Some(reason.to_string());
        m
    }

    fn validate_promote_transition(
        current: LearningStatus,
        target: LearningStatus,
    ) -> Result<(), String> {
        let allowed = matches!(
            (&current, &target),
            (LearningStatus::Candidate, LearningStatus::Confirmed)
                | (LearningStatus::Candidate, LearningStatus::Rule)
                | (LearningStatus::Confirmed, LearningStatus::Rule)
        );
        if allowed {
            Ok(())
        } else {
            Err(format!("Invalid promotion: {:?} → {:?}", current, target))
        }
    }

    #[test]
    fn promote_candidate_to_confirmed_allowed() {
        assert!(validate_promote_transition(LearningStatus::Candidate, LearningStatus::Confirmed).is_ok());
    }

    #[test]
    fn promote_candidate_to_rule_allowed() {
        assert!(validate_promote_transition(LearningStatus::Candidate, LearningStatus::Rule).is_ok());
    }

    #[test]
    fn promote_confirmed_to_rule_allowed() {
        assert!(validate_promote_transition(LearningStatus::Confirmed, LearningStatus::Rule).is_ok());
    }

    #[test]
    fn promote_confirmed_to_candidate_rejected() {
        assert!(validate_promote_transition(LearningStatus::Confirmed, LearningStatus::Candidate).is_err());
    }

    #[test]
    fn promote_rule_to_confirmed_rejected() {
        assert!(validate_promote_transition(LearningStatus::Rule, LearningStatus::Confirmed).is_err());
    }

    #[test]
    fn promote_rule_to_candidate_rejected() {
        assert!(validate_promote_transition(LearningStatus::Rule, LearningStatus::Candidate).is_err());
    }

    #[test]
    fn rejected_memory_has_rejected_lifecycle() {
        let m = make_invalidated_memory("learning_rejected");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Rejected);
    }

    #[test]
    fn archived_memory_has_archived_lifecycle() {
        let m = make_invalidated_memory("learning_archived");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Archived);
    }

    #[test]
    fn superseded_memory_has_superseded_lifecycle() {
        let m = make_invalidated_memory("superseded");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Superseded);
    }

    #[test]
    fn candidate_memory_has_candidate_lifecycle() {
        let m = make_memory_with_status(LearningStatus::Candidate);
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Candidate);
    }

    #[test]
    fn confirmed_memory_has_active_lifecycle() {
        let m = make_memory_with_status(LearningStatus::Confirmed);
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Active);
    }

    #[test]
    fn rule_memory_has_active_lifecycle() {
        let m = make_memory_with_status(LearningStatus::Rule);
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Active);
    }

    #[test]
    fn promote_to_rule_requires_procedural_memory_type() {
        let new_memory_type = if matches!(LearningStatus::Rule, LearningStatus::Rule) {
            Some(MemoryType::Procedural)
        } else {
            None
        };
        assert_eq!(new_memory_type, Some(MemoryType::Procedural));
    }

    #[test]
    fn promote_to_confirmed_does_not_change_memory_type() {
        let new_memory_type: Option<MemoryType> = if matches!(LearningStatus::Confirmed, LearningStatus::Rule) {
            Some(MemoryType::Procedural)
        } else {
            None
        };
        assert_eq!(new_memory_type, None);
    }

    #[test]
    fn workflow_rule_kind_maps_to_procedural() {
        assert_eq!(
            kind_status_to_memory_type(&LearningKind::WorkflowRule, &LearningStatus::Candidate),
            MemoryType::Procedural
        );
    }

    #[test]
    fn rule_status_maps_to_procedural_regardless_of_kind() {
        assert_eq!(
            kind_status_to_memory_type(&LearningKind::UserPreference, &LearningStatus::Rule),
            MemoryType::Procedural
        );
        assert_eq!(
            kind_status_to_memory_type(&LearningKind::ProjectLesson, &LearningStatus::Rule),
            MemoryType::Procedural
        );
    }

    #[test]
    fn project_lesson_kind_maps_to_episodic() {
        assert_eq!(
            kind_status_to_memory_type(&LearningKind::ProjectLesson, &LearningStatus::Candidate),
            MemoryType::Episodic
        );
    }

    #[test]
    fn user_preference_kind_maps_to_semantic() {
        assert_eq!(
            kind_status_to_memory_type(&LearningKind::UserPreference, &LearningStatus::Confirmed),
            MemoryType::Semantic
        );
    }

    #[test]
    fn project_pattern_kind_maps_to_semantic() {
        assert_eq!(
            kind_status_to_memory_type(&LearningKind::ProjectPattern, &LearningStatus::Confirmed),
            MemoryType::Semantic
        );
    }

    #[test]
    fn project_pitfall_kind_maps_to_semantic() {
        assert_eq!(
            kind_status_to_memory_type(&LearningKind::ProjectPitfall, &LearningStatus::Candidate),
            MemoryType::Semantic
        );
    }

    #[test]
    fn extract_learning_fields_returns_none_when_no_metadata() {
        let m = Memory::new("content".to_string());
        assert!(extract_learning_fields(&m).is_none());
    }

    #[test]
    fn extract_learning_fields_returns_none_when_no_learning_key() {
        let mut m = Memory::new("content".to_string());
        m.metadata = Some(json!({ "other": "value" }));
        assert!(extract_learning_fields(&m).is_none());
    }

    #[test]
    fn extract_learning_fields_returns_none_on_invalid_learning_value() {
        let mut m = Memory::new("content".to_string());
        m.metadata = Some(json!({ "learning": "not_an_object" }));
        assert!(extract_learning_fields(&m).is_none());
    }

    #[test]
    fn extract_learning_fields_parses_valid_metadata() {
        let m = make_memory_with_status(LearningStatus::Confirmed);
        let result = extract_learning_fields(&m);
        assert!(result.is_some());
        let (kind, status, _scope, schema_version) = result.unwrap();
        assert_eq!(kind, LearningKind::UserPreference);
        assert_eq!(status, LearningStatus::Confirmed);
        assert_eq!(schema_version, 1);
    }

    #[test]
    fn extract_learning_fields_returns_correct_kind_for_workflow_rule() {
        let mut m = Memory::new("content".to_string());
        m.metadata = Some(json!({
            "learning": {
                "schema_version": 1,
                "kind": "workflow_rule",
                "status": "rule",
                "confidence": 0.9,
                "scope": { "level": "global" },
                "source": { "created_from": "manual" }
            }
        }));
        let (kind, status, _, _) = extract_learning_fields(&m).unwrap();
        assert_eq!(kind, LearningKind::WorkflowRule);
        assert_eq!(status, LearningStatus::Rule);
    }
}
