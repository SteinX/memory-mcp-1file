use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::embedding::ContentHasher;
use crate::server::params::{
    LearningMemoryArchiveParams, LearningMemoryCreateParams, LearningMemoryDeleteParams,
    LearningMemoryGetParams, LearningMemoryListParams, LearningMemoryMigrateLegacyParams,
    LearningMemoryPromoteParams, LearningMemoryRejectParams, LearningMemorySearchParams,
    LearningMemorySupersededParams, LearningMemoryUpdateParams,
};
use crate::storage::traits::MemoryGcFilter;
use crate::storage::StorageBackend;
use crate::types::{
    learning::{
        validate_learning_metadata, CreatedFrom, LearningKind, LearningMetadata, LearningScope,
        LearningSource, LearningStatus, ScopeLevel,
    },
    record_key_to_string, Datetime, EmbeddingState, ExportIdentity, Memory, MemoryQuery,
    MemoryType, MemoryUpdate,
};

use super::contracts::{export_contract_meta, summary_collection_response, with_surface_guidance};
use super::learning_filters::{
    apply_learning_filter, default_list_filter, default_search_filter, LearningFilter,
};
use super::learning_lifecycle::{derive_lifecycle_state, LearningLifecycleState};
use super::learning_response::build_learning_response;
use super::{
    error_response, normalize_limit, strip_embedding, strip_embeddings, success_json,
    success_serialize,
};

#[derive(Debug, Clone, PartialEq)]
struct LegacyMigrationClassification {
    kind: Option<LearningKind>,
    status: Option<LearningStatus>,
    scope: Option<LearningScope>,
    outcome: &'static str,
    reason: &'static str,
}

#[derive(Debug, Clone)]
struct AlreadyMigratedMatch {
    confidence: &'static str,
    matched_memory_id: String,
}

#[derive(Debug, Default)]
struct MigrationCounts {
    scanned: usize,
    eligible: usize,
    created: usize,
    skipped: usize,
    ambiguous: usize,
    already_migrated: usize,
    invalidated_skipped: usize,
}

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

fn memory_id(memory: &Memory) -> Option<String> {
    memory.id.as_ref().map(|id| record_key_to_string(&id.key))
}

fn has_learning_schema(memory: &Memory) -> bool {
    memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .and_then(|l| l.get("schema_version"))
        .and_then(|v| v.as_u64())
        .is_some()
}

fn metadata_text_contains(memory: &Memory, needles: &[&str]) -> bool {
    let Some(metadata) = &memory.metadata else {
        return false;
    };
    let text = metadata.to_string().to_ascii_lowercase();
    needles.iter().any(|needle| text.contains(needle))
}

fn legacy_scope(params: &LearningMemoryMigrateLegacyParams) -> Result<LearningScope, String> {
    let level: ScopeLevel = match params.scope.as_deref() {
        Some(s) => serde_json::from_value(json!(s)).map_err(|_| format!("Invalid scope: '{s}'"))?,
        None if params.project_id.is_some() => ScopeLevel::Project,
        None => ScopeLevel::Global,
    };

    Ok(LearningScope {
        level,
        project_id: params.project_id.clone(),
        workspace: None,
        mode: None,
        user_id: None,
        agent_id: None,
        run_id: None,
        namespace: None,
    })
}

fn classify_legacy_memory(
    memory: &Memory,
    params: &LearningMemoryMigrateLegacyParams,
) -> LegacyMigrationClassification {
    if has_learning_schema(memory) {
        return LegacyMigrationClassification {
            kind: None,
            status: None,
            scope: None,
            outcome: "already_migrated",
            reason: "record already has metadata.learning.schema_version",
        };
    }

    if let Some(prefix_allowlist) = &params.prefix_allowlist {
        if !prefix_allowlist
            .iter()
            .any(|prefix| memory.content.starts_with(prefix))
        {
            return LegacyMigrationClassification {
                kind: None,
                status: None,
                scope: None,
                outcome: "skipped",
                reason: "content prefix is not in prefix_allowlist",
            };
        }
    }

    let scope = match legacy_scope(params) {
        Ok(scope) => scope,
        Err(_) => {
            return LegacyMigrationClassification {
                kind: None,
                status: None,
                scope: None,
                outcome: "skipped",
                reason: "invalid target scope",
            };
        }
    };

    let content = memory.content.trim_start();
    let classification = if content.starts_with("USER — Preference:") {
        Some((
            LearningKind::UserPreference,
            LearningStatus::Confirmed,
            "USER — Preference prefix",
        ))
    } else if content.starts_with("USER:") {
        if metadata_text_contains(
            memory,
            &["preference", "workflow_rule", "workflow rule", "rule"],
        ) {
            if metadata_text_contains(memory, &["workflow_rule", "workflow rule", "rule"]) {
                Some((
                    LearningKind::WorkflowRule,
                    LearningStatus::Rule,
                    "USER metadata rule hint",
                ))
            } else {
                Some((
                    LearningKind::UserPreference,
                    LearningStatus::Confirmed,
                    "USER metadata preference hint",
                ))
            }
        } else {
            return LegacyMigrationClassification {
                kind: None,
                status: None,
                scope: None,
                outcome: "ambiguous",
                reason: "USER prefix requires preference/rule metadata or caller rule",
            };
        }
    } else if content.starts_with("CONTEXT:") && matches!(scope.level, ScopeLevel::Project) {
        Some((
            LearningKind::ProjectPattern,
            LearningStatus::Candidate,
            "CONTEXT project scope",
        ))
    } else if content.starts_with("RESEARCH:") && params.extract_research_lessons.unwrap_or(false) {
        Some((
            LearningKind::ProjectLesson,
            LearningStatus::Candidate,
            "RESEARCH explicit lesson extraction",
        ))
    } else if content.starts_with("TASK:") || content.starts_with("EPIC:") {
        return LegacyMigrationClassification {
            kind: None,
            status: None,
            scope: None,
            outcome: "skipped",
            reason: "TASK and EPIC records are excluded by default",
        };
    } else {
        return LegacyMigrationClassification {
            kind: None,
            status: None,
            scope: None,
            outcome: "ambiguous",
            reason: "no eligible legacy learning prefix matched",
        };
    };

    let (kind, status, reason) = classification.expect("classification is set above");
    LegacyMigrationClassification {
        kind: Some(kind),
        status: Some(status),
        scope: Some(scope),
        outcome: "eligible",
        reason,
    }
}

fn find_already_migrated_match(
    legacy_id: &str,
    legacy: &Memory,
    classification: &LegacyMigrationClassification,
    existing_learning: &[Memory],
) -> Option<AlreadyMigratedMatch> {
    for candidate in existing_learning {
        let Some(metadata) = candidate
            .metadata
            .as_ref()
            .and_then(|m| m.get("learning"))
            .and_then(|raw| validate_learning_metadata(raw).ok())
        else {
            continue;
        };

        let Some(candidate_id) = memory_id(candidate) else {
            continue;
        };

        if metadata.source.created_from == CreatedFrom::Migration
            && metadata
                .source
                .source_memory_ids
                .iter()
                .any(|source_id| source_id == legacy_id)
        {
            return Some(AlreadyMigratedMatch {
                confidence: "primary",
                matched_memory_id: candidate_id,
            });
        }

        let Some(kind) = &classification.kind else {
            continue;
        };
        let Some(scope) = &classification.scope else {
            continue;
        };
        let legacy_hash = legacy
            .content_hash
            .clone()
            .unwrap_or_else(|| ContentHasher::hash(&legacy.content));
        if candidate.content_hash.as_deref() == Some(legacy_hash.as_str())
            && &metadata.kind == kind
            && &metadata.scope == scope
        {
            return Some(AlreadyMigratedMatch {
                confidence: "secondary",
                matched_memory_id: candidate_id,
            });
        }
    }

    None
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
        Ok(None) => {
            return Ok(error_response(format!(
                "Memory not found after create: {id}"
            )));
        }
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
            )));
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

    let summary =
        summary_collection_response("collection", records.len(), Some(total), false, None);
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

// ─── Ranking helpers ─────────────────────────────────────────────────────────

/// Status-based score multiplier.
/// rule=1.3x, confirmed=1.1x, candidate=0.9x (candidate excluded by default).
fn status_boost(status: &LearningStatus) -> f32 {
    match status {
        LearningStatus::Rule => 1.3,
        LearningStatus::Confirmed => 1.1,
        LearningStatus::Candidate => 0.9,
        // Rejected/superseded/archived are excluded by default; multiplier is
        // irrelevant but defined for completeness.
        _ => 0.5,
    }
}

/// Scope-match multiplier.
/// exact match = 1.2x, global fallback = 1.0x (no boost).
fn scope_boost(
    record_scope: &LearningScope,
    requested_scope: Option<&str>,
    requested_project_id: Option<&str>,
) -> f32 {
    let requested_level =
        requested_scope.and_then(|s| serde_json::from_value::<ScopeLevel>(json!(s)).ok());
    match &requested_level {
        Some(level) if *level == record_scope.level => {
            // Also check project_id match for project-scoped records.
            if *level == ScopeLevel::Project {
                if requested_project_id == record_scope.project_id.as_deref() {
                    1.2
                } else {
                    1.0
                }
            } else {
                1.2
            }
        }
        _ => 1.0,
    }
}

/// Confidence multiplier clamped to [0.5, 2.0].
/// Maps confidence [0.0, 1.0] → multiplier [0.5, 1.5].
fn confidence_multiplier(confidence: f64) -> f32 {
    let m = 0.5 + confidence as f32;
    m.clamp(0.5, 2.0)
}

/// Importance multiplier clamped to [0.5, 2.0].
/// importance_score is typically [0.0, 5.0]; we normalise to [0.5, 2.0].
fn importance_multiplier(importance: f32) -> f32 {
    let m = 0.5 + (importance / 5.0) * 1.5;
    m.clamp(0.5, 2.0)
}

pub async fn search(
    state: &AppState,
    params: LearningMemorySearchParams,
) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);

    // ── 1. Parse filter ───────────────────────────────────────────────────────
    let mut filter = default_search_filter();
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
            filter.include_status = None;
            filter.exclude_status = None;
        }
        filter.fallback = caller_filter.fallback;
    }

    let include_global = filter.fallback.include_global;

    // ── 2. Build MemoryQuery with learning + status filters ───────────────────
    let mut query = MemoryQuery::default();

    // Require schema_version to be present (learning records only).
    let schema_filter = json!({
        "field": "metadata.learning.schema_version",
        "op": "exists",
        "value": true
    });
    query.metadata_filter = Some(schema_filter);

    // Apply status/invalidation filter (merges with existing metadata_filter).
    apply_learning_filter(&mut query, &filter);

    // ── 3. Scope filter ───────────────────────────────────────────────────────
    // We do NOT restrict at the storage level by scope — we handle scope
    // boosting in post-processing. Global fallback is controlled by
    // `include_global` flag applied after ranking.

    let limit = normalize_limit(params.limit);
    let fetch_limit = (limit * 4).max(50); // overfetch for post-ranking

    // ── 4. Vector search ──────────────────────────────────────────────────────
    let query_embedding = state.embedding.embed(&params.query).await?;

    let mut prefilter_query = query.clone();
    prefilter_query.metadata_filter = None;

    let vector_results: Vec<crate::types::SearchResult> = state
        .storage
        .vector_search(&query_embedding, &prefilter_query, fetch_limit)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            r.metadata
                .as_ref()
                .and_then(|m| m.get("learning"))
                .and_then(|l| l.get("schema_version"))
                .is_some()
        })
        .collect();

    // ── 5. BM25 search ────────────────────────────────────────────────────────
    // Collect candidate records.
    //
    // NOTE: `list_memories` filters by `valid_until` at the SQL level, so
    // invalidated (rejected/archived) records are excluded even when
    // `include_invalidated=true`.  We work around this by:
    //   a) Always using a bare query (no metadata_filter) for `list_memories`
    //      so the Rust-side `metadata_matches` (which does not understand the
    //      {field,op,value} DSL) does not discard everything.
    //   b) Applying the status/invalidation filter ourselves in Rust after
    //      fetching.
    //   c) In audit mode, additionally fetching each vector-result record via
    //      `get_memory` (which has no `valid_until` restriction) so that
    //      invalidated records can be included.
    let include_invalidated = filter.include_invalidated || filter.audit;

    // Build a bare query (schema_version existence only, no status filter) so
    // that `list_memories` returns all valid learning records.
    let bare_query = MemoryQuery::default();
    // No metadata_filter — we post-filter in Rust.

    let listed_valid = state
        .storage
        .list_memories(&bare_query, fetch_limit, 0)
        .await
        .unwrap_or_default();

    // Helper: check whether a Memory passes the status filter.
    let status_allowed = |m: &Memory| -> bool {
        let status = m
            .metadata
            .as_ref()
            .and_then(|md| md.get("learning"))
            .and_then(|l| l.get("status"))
            .and_then(|s| serde_json::from_value::<LearningStatus>(s.clone()).ok());

        // Must be a learning record.
        let status = match status {
            Some(s) => s,
            None => return false,
        };

        // include_status whitelist.
        if let Some(ref include) = filter.include_status {
            if !include.is_empty() && !include.contains(&status) {
                return false;
            }
        }
        // exclude_status blacklist.
        if let Some(ref exclude) = filter.exclude_status {
            if exclude.contains(&status) {
                return false;
            }
        }
        true
    };

    // Filter valid records by status.
    let listed: Vec<Memory> = listed_valid
        .into_iter()
        .filter(|m| {
            // Must be a learning record (has schema_version).
            m.metadata
                .as_ref()
                .and_then(|md| md.get("learning"))
                .and_then(|l| l.get("schema_version"))
                .is_some()
                && status_allowed(m)
        })
        .collect();

    let mut allowed_ids: std::collections::HashSet<String> = listed
        .iter()
        .filter_map(|m| m.id.as_ref().map(|r| record_key_to_string(&r.key)))
        .collect();

    // ── 5b. Audit mode: include invalidated records from vector results ────────
    // `list_memories` and `vector_search` both exclude records with
    // `valid_until <= now`.  In audit mode we fetch each vector-result record
    // individually via `get_memory` (no `valid_until` restriction) and add
    // qualifying ones to the pool.  We also seed them into `score_map` with a
    // floor score so they appear even when neither vector nor BM25 returns them.
    let mut id_to_memory: std::collections::HashMap<String, Memory> =
        std::collections::HashMap::new();
    let mut audit_seeded_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    if include_invalidated {
        for r in &vector_results {
            if allowed_ids.contains(&r.id) {
                continue; // already in the valid pool
            }
            if let Ok(Some(m)) = state.storage.get_memory(&r.id).await {
                if m.metadata
                    .as_ref()
                    .and_then(|md| md.get("learning"))
                    .and_then(|l| l.get("schema_version"))
                    .is_some()
                    && status_allowed(&m)
                {
                    allowed_ids.insert(r.id.clone());
                    audit_seeded_ids.insert(r.id.clone());
                    if let Some(ref rec) = m.id {
                        id_to_memory.insert(record_key_to_string(&rec.key), m);
                    }
                }
            }
        }
        // Also search BM25 for the query text and fetch any matching invalidated
        // records that weren't found by vector search.
        let bm25_audit_candidates = state
            .memory_search
            .search(&params.query, None, fetch_limit)
            .await;
        for r in &bm25_audit_candidates {
            if allowed_ids.contains(&r.id) {
                continue;
            }
            if let Ok(Some(m)) = state.storage.get_memory(&r.id).await {
                if m.metadata
                    .as_ref()
                    .and_then(|md| md.get("learning"))
                    .and_then(|l| l.get("schema_version"))
                    .is_some()
                    && status_allowed(&m)
                {
                    allowed_ids.insert(r.id.clone());
                    audit_seeded_ids.insert(r.id.clone());
                    if let Some(ref rec) = m.id {
                        id_to_memory.insert(record_key_to_string(&rec.key), m);
                    }
                }
            }
        }

        // The in-memory BM25 index may not contain old invalidated rows after a
        // restart. Audit mode is explicitly for lifecycle inspection, so scan
        // invalidated storage rows and seed query-matching learning records.
        let query_lc = params.query.to_ascii_lowercase();
        let page_size = fetch_limit.max(50);
        let mut offset = 0usize;
        while audit_seeded_ids.len() < fetch_limit {
            let invalidated_page = state
                .storage
                .list_invalidated_memories_for_gc(&MemoryGcFilter::default(), page_size, offset)
                .await
                .unwrap_or_default();
            if invalidated_page.is_empty() {
                break;
            }

            for m in invalidated_page {
                let Some(id) = memory_id(&m) else {
                    continue;
                };
                if allowed_ids.contains(&id)
                    || !m.content.to_ascii_lowercase().contains(&query_lc)
                    || !has_learning_schema(&m)
                    || !status_allowed(&m)
                {
                    continue;
                }
                allowed_ids.insert(id.clone());
                audit_seeded_ids.insert(id.clone());
                id_to_memory.insert(id, m);
                if audit_seeded_ids.len() >= fetch_limit {
                    break;
                }
            }

            offset += page_size;
        }
    }

    let bm25_results: Vec<crate::types::SearchResult> = if !allowed_ids.is_empty() {
        state
            .memory_search
            .search(&params.query, Some(&allowed_ids), fetch_limit)
            .await
    } else {
        vec![]
    };

    // ── 6. Merge: build a score map (best score per ID) ───────────────────────
    let mut score_map: std::collections::HashMap<String, (f32, Option<serde_json::Value>)> =
        std::collections::HashMap::new();

    for r in &vector_results {
        if !allowed_ids.contains(&r.id) {
            continue; // not in the allowed set
        }
        let entry = score_map
            .entry(r.id.clone())
            .or_insert((0.0, r.metadata.clone()));
        if r.score > entry.0 {
            entry.0 = r.score;
        }
    }
    for r in &bm25_results {
        let entry = score_map
            .entry(r.id.clone())
            .or_insert((0.0, r.metadata.clone()));
        if r.score > entry.0 {
            entry.0 = r.score;
        }
    }

    for id in &audit_seeded_ids {
        score_map.entry(id.clone()).or_insert((0.1, None));
    }

    // ── 7. Fetch full Memory records for scoring ──────────────────────────────
    // We need the full Memory to extract learning metadata for boosting.
    // Use the listed memories (already fetched) as the primary source.
    // (id_to_memory may already contain audit-mode invalidated records from 5b.)
    for m in listed {
        if let Some(ref rec) = m.id {
            id_to_memory.insert(record_key_to_string(&rec.key), m);
        }
    }

    // ── 8. Apply learning-specific ranking multipliers ────────────────────────
    let mut scored: Vec<(f32, Memory)> = Vec::new();

    for (id, (base_score, _)) in &score_map {
        let memory = match id_to_memory.remove(id) {
            Some(m) => m,
            None => continue,
        };

        let lm: LearningMetadata = match memory
            .metadata
            .as_ref()
            .and_then(|m| m.get("learning"))
            .and_then(|v| serde_json::from_value(v.clone()).ok())
        {
            Some(lm) => lm,
            None => continue, // not a learning record
        };

        // Scope filter: exclude non-global records when scope doesn't match,
        // unless include_global is true.
        let scope_level = &lm.scope.level;
        if *scope_level != ScopeLevel::Global && !include_global {
            // Only include if scope matches requested scope.
            if let Some(req_scope) = params.scope.as_deref() {
                let req_level: Option<ScopeLevel> = serde_json::from_value(json!(req_scope)).ok();
                if req_level.as_ref() != Some(scope_level) {
                    continue;
                }
            }
            // If no scope requested, include project/workspace records too
            // (they are not global but still relevant).
        }

        // Global records: only include if include_global=true OR no scope requested.
        // Actually: global records are always included unless caller restricts scope.
        // (The default is to include global records in all searches.)

        let s_boost = status_boost(&lm.status);
        let sc_boost = scope_boost(
            &lm.scope,
            params.scope.as_deref(),
            params.project_id.as_deref(),
        );
        let conf_mult = confidence_multiplier(lm.confidence);
        let imp_mult = importance_multiplier(memory.importance_score);

        let final_score = base_score * s_boost * sc_boost * conf_mult * imp_mult;
        scored.push((final_score, memory));
    }

    // ── 9. Sort and limit ─────────────────────────────────────────────────────
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    // ── 10. Build response ────────────────────────────────────────────────────
    let records: Vec<serde_json::Value> = scored
        .into_iter()
        .filter_map(|(_score, mut memory)| {
            strip_embedding(&mut memory);
            let memory_id = memory.id.as_ref().map(|r| record_key_to_string(&r.key));
            let (kind, status, scope, schema_version) = extract_learning_fields(&memory)?;
            let lifecycle_state = derive_lifecycle_state(&memory);
            let record_json = serde_json::to_value(&memory).ok()?;
            let record_contract = learning_contract_json(memory_id.as_deref());
            let response = build_learning_response(
                record_json,
                kind,
                status,
                scope,
                lifecycle_state,
                schema_version,
                record_contract,
                json!({}),
            );
            serde_json::to_value(response).ok()
        })
        .collect();

    let contract = learning_collection_contract_json();
    let summary = summary_collection_response(
        "collection",
        records.len(),
        Some(records.len()),
        false,
        None,
    );
    let summary_val = serde_json::to_value(summary).unwrap_or(json!({}));

    Ok(success_json(json!({
        "records": records,
        "summary": summary_val,
        "contract": contract,
        "count": records.len(),
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
        updated
            .id
            .as_ref()
            .map(|id| record_key_to_string(&id.key))
            .as_deref(),
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

    if !matches!(
        target_status,
        LearningStatus::Confirmed | LearningStatus::Rule
    ) {
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
        learning_meta["kind"] =
            serde_json::to_value(&target_kind).unwrap_or(json!("user_preference"));
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
        Err(e) => {
            return Ok(error_response(format!(
                "Invalid learning metadata after promote: {}",
                e
            )));
        }
    };

    let lifecycle = derive_lifecycle_state(&updated);
    let record_value = serde_json::to_value(&updated).unwrap_or_default();
    let contract = learning_contract_json(
        updated
            .id
            .as_ref()
            .map(|id| record_key_to_string(&id.key))
            .as_deref(),
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
    state: &AppState,
    params: LearningMemoryRejectParams,
) -> anyhow::Result<CallToolResult> {
    let memory = match state.storage.get_memory(&params.id).await? {
        Some(m) => m,
        None => return Ok(error_response(format!("Record not found: {}", params.id))),
    };

    let (kind, _status, scope, schema_version) = match extract_learning_fields(&memory) {
        Some(fields) => fields,
        None => {
            return Ok(error_response(format!(
                "Memory '{}' does not have valid learning metadata",
                params.id
            )));
        }
    };

    let mut learning_meta = memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .cloned()
        .unwrap_or(json!({}));
    learning_meta["status"] =
        serde_json::to_value(&LearningStatus::Rejected).unwrap_or(json!("rejected"));

    let mut full_metadata = memory.metadata.clone().unwrap_or(json!({}));
    full_metadata["learning"] = learning_meta;

    let update = MemoryUpdate {
        content: None,
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

    match state.storage.update_memory(&params.id, update).await {
        Ok(_) => {}
        Err(e) => return Ok(error_response(e)),
    };

    match state
        .storage
        .invalidate(&params.id, Some("learning_rejected"), None)
        .await
    {
        Ok(_) => {}
        Err(e) => return Ok(error_response(e)),
    };

    if let Ok(Some(updated)) = state.storage.get_memory(&params.id).await {
        state.memory_search.upsert_memory(updated).await;
    }

    let mut final_memory = match state.storage.get_memory(&params.id).await? {
        Some(m) => m,
        None => {
            return Ok(error_response(format!(
                "Record not found after reject: {}",
                params.id
            )));
        }
    };
    strip_embedding(&mut final_memory);

    let lifecycle = derive_lifecycle_state(&final_memory);
    let record_value = serde_json::to_value(&final_memory).unwrap_or_default();
    let contract = learning_contract_json(Some(&params.id));
    let summary = json!({ "result_kind": "learning_memory", "counts": { "returned": 1 } });

    let resp = build_learning_response(
        record_value,
        kind,
        LearningStatus::Rejected,
        scope,
        lifecycle,
        schema_version,
        contract,
        summary,
    );

    Ok(success_serialize(&resp))
}

pub async fn archive(
    state: &AppState,
    params: LearningMemoryArchiveParams,
) -> anyhow::Result<CallToolResult> {
    let memory = match state.storage.get_memory(&params.id).await? {
        Some(m) => m,
        None => return Ok(error_response(format!("Record not found: {}", params.id))),
    };

    let (kind, _status, scope, schema_version) = match extract_learning_fields(&memory) {
        Some(fields) => fields,
        None => {
            return Ok(error_response(format!(
                "Memory '{}' does not have valid learning metadata",
                params.id
            )));
        }
    };

    let mut learning_meta = memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .cloned()
        .unwrap_or(json!({}));
    learning_meta["status"] =
        serde_json::to_value(&LearningStatus::Archived).unwrap_or(json!("archived"));

    let mut full_metadata = memory.metadata.clone().unwrap_or(json!({}));
    full_metadata["learning"] = learning_meta;

    let update = MemoryUpdate {
        content: None,
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

    match state.storage.update_memory(&params.id, update).await {
        Ok(_) => {}
        Err(e) => return Ok(error_response(e)),
    };

    match state
        .storage
        .invalidate(&params.id, Some("learning_archived"), None)
        .await
    {
        Ok(_) => {}
        Err(e) => return Ok(error_response(e)),
    };

    if let Ok(Some(updated)) = state.storage.get_memory(&params.id).await {
        state.memory_search.upsert_memory(updated).await;
    }

    let mut final_memory = match state.storage.get_memory(&params.id).await? {
        Some(m) => m,
        None => {
            return Ok(error_response(format!(
                "Record not found after archive: {}",
                params.id
            )));
        }
    };
    strip_embedding(&mut final_memory);

    let lifecycle = derive_lifecycle_state(&final_memory);
    let record_value = serde_json::to_value(&final_memory).unwrap_or_default();
    let contract = learning_contract_json(Some(&params.id));
    let summary = json!({ "result_kind": "learning_memory", "counts": { "returned": 1 } });

    let resp = build_learning_response(
        record_value,
        kind,
        LearningStatus::Archived,
        scope,
        lifecycle,
        schema_version,
        contract,
        summary,
    );

    Ok(success_serialize(&resp))
}

pub async fn supersede(
    state: &AppState,
    params: LearningMemorySupersededParams,
) -> anyhow::Result<CallToolResult> {
    let memory = match state.storage.get_memory(&params.id).await? {
        Some(m) => m,
        None => return Ok(error_response(format!("Record not found: {}", params.id))),
    };

    let (kind, _status, scope, schema_version) = match extract_learning_fields(&memory) {
        Some(fields) => fields,
        None => {
            return Ok(error_response(format!(
                "Memory '{}' does not have valid learning metadata",
                params.id
            )));
        }
    };

    let lifecycle = derive_lifecycle_state(&memory);
    if matches!(
        lifecycle,
        LearningLifecycleState::Rejected
            | LearningLifecycleState::Archived
            | LearningLifecycleState::Superseded
    ) {
        return Ok(error_response(format!(
            "Record '{}' is already in a terminal state ({:?}) and cannot be superseded",
            params.id, lifecycle
        )));
    }

    if state
        .storage
        .get_memory(&params.replacement_id)
        .await?
        .is_none()
    {
        return Ok(error_response(format!(
            "Replacement record not found: {}",
            params.replacement_id
        )));
    }

    let mut learning_meta = memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .cloned()
        .unwrap_or(json!({}));
    learning_meta["status"] =
        serde_json::to_value(&LearningStatus::Superseded).unwrap_or(json!("superseded"));
    learning_meta["superseded_by"] = json!(&params.replacement_id);
    if let Some(ref reason) = params.reason {
        learning_meta["supersede_reason"] = json!(reason);
    }

    let mut full_metadata = memory.metadata.clone().unwrap_or(json!({}));
    full_metadata["learning"] = learning_meta;

    let update = MemoryUpdate {
        content: None,
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

    match state.storage.update_memory(&params.id, update).await {
        Ok(_) => {}
        Err(e) => return Ok(error_response(e)),
    };

    match state
        .storage
        .invalidate(&params.id, Some("superseded"), Some(&params.replacement_id))
        .await
    {
        Ok(_) => {}
        Err(e) => return Ok(error_response(e)),
    };

    if let Ok(Some(updated)) = state.storage.get_memory(&params.id).await {
        state.memory_search.upsert_memory(updated).await;
    }

    let mut final_memory = match state.storage.get_memory(&params.id).await? {
        Some(m) => m,
        None => {
            return Ok(error_response(format!(
                "Record not found after supersede: {}",
                params.id
            )));
        }
    };
    strip_embedding(&mut final_memory);

    let lifecycle = derive_lifecycle_state(&final_memory);
    let record_value = serde_json::to_value(&final_memory).unwrap_or_default();
    let contract = learning_contract_json(Some(&params.id));
    let summary = json!({ "result_kind": "learning_memory", "counts": { "returned": 1 } });

    let resp = build_learning_response(
        record_value,
        kind,
        LearningStatus::Superseded,
        scope,
        lifecycle,
        schema_version,
        contract,
        summary,
    );

    Ok(success_serialize(&resp))
}

pub async fn migrate_legacy(
    state: &AppState,
    params: LearningMemoryMigrateLegacyParams,
) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);

    if let Err(e) = legacy_scope(&params) {
        return Ok(error_response(e));
    }

    let dry_run = params.dry_run;
    let include_invalidated = params.include_invalidated.unwrap_or(false);
    let invalidate_source = params.invalidate_source.unwrap_or(false);
    let limit = params.limit.unwrap_or(50).min(100);

    let scan_query = if include_invalidated {
        MemoryQuery {
            valid_at: Some(Datetime::default()),
            ..Default::default()
        }
    } else {
        MemoryQuery::default()
    };
    let learning_query = MemoryQuery {
        metadata_filter: Some(json!({ "learning": { "schema_version": 1 } })),
        valid_at: if include_invalidated {
            Some(Datetime::default())
        } else {
            None
        },
        ..Default::default()
    };

    let existing_learning = match state.storage.list_memories(&learning_query, 100, 0).await {
        Ok(records) => records,
        Err(e) => return Ok(error_response(e)),
    };
    let candidates = match state.storage.list_memories(&scan_query, limit, 0).await {
        Ok(records) => records,
        Err(e) => return Ok(error_response(e)),
    };

    let mut counts = MigrationCounts::default();
    let mut previews = Vec::new();
    let mut created_records = Vec::new();

    for legacy in candidates {
        counts.scanned += 1;
        let Some(legacy_id) = memory_id(&legacy) else {
            counts.skipped += 1;
            previews.push(json!({
                "legacy_id": null,
                "outcome": "skipped",
                "reason": "legacy record has no stable id"
            }));
            continue;
        };

        if legacy.valid_until.is_some() && !include_invalidated {
            counts.invalidated_skipped += 1;
            previews.push(json!({
                "legacy_id": legacy_id,
                "outcome": "invalidated_skipped",
                "reason": "invalidated source excluded by default"
            }));
            continue;
        }

        let classification = classify_legacy_memory(&legacy, &params);
        if classification.outcome == "already_migrated" {
            counts.already_migrated += 1;
            previews.push(json!({
                "legacy_id": legacy_id,
                "outcome": "already_migrated",
                "reason": classification.reason,
                "match_confidence": "primary"
            }));
            continue;
        }
        if classification.outcome == "ambiguous" {
            counts.ambiguous += 1;
            previews.push(json!({
                "legacy_id": legacy_id,
                "outcome": "ambiguous",
                "reason": classification.reason,
            }));
            continue;
        }
        if classification.outcome == "skipped" {
            counts.skipped += 1;
            previews.push(json!({
                "legacy_id": legacy_id,
                "outcome": "skipped",
                "reason": classification.reason,
            }));
            continue;
        }

        if let Some(migration_match) =
            find_already_migrated_match(&legacy_id, &legacy, &classification, &existing_learning)
        {
            counts.already_migrated += 1;
            previews.push(json!({
                "legacy_id": legacy_id,
                "outcome": "already_migrated",
                "reason": "matching migrated learning record already exists",
                "match_confidence": migration_match.confidence,
                "matched_memory_id": migration_match.matched_memory_id,
            }));
            continue;
        }

        counts.eligible += 1;
        let kind = classification
            .kind
            .clone()
            .expect("eligible classification has kind");
        let status = classification
            .status
            .clone()
            .expect("eligible classification has status");
        let scope = classification
            .scope
            .clone()
            .expect("eligible classification has scope");
        let memory_type = kind_status_to_memory_type(&kind, &status);
        let learning_meta = LearningMetadata {
            schema_version: 1,
            kind: kind.clone(),
            status: status.clone(),
            confidence: 0.5,
            scope: scope.clone(),
            source: LearningSource {
                created_from: CreatedFrom::Migration,
                client: None,
                source_memory_ids: vec![legacy_id.clone()],
            },
            evidence: vec![format!("Migrated from legacy memory {legacy_id}")],
            applies_to: vec![],
            trigger_hints: vec![],
            supersedes: vec![],
            constraints: vec![
                "Legacy migration classification should be reviewed before promotion.".to_string(),
            ],
        };
        let learning_val = match serde_json::to_value(&learning_meta) {
            Ok(v) => v,
            Err(e) => return Ok(error_response(e)),
        };
        if let Err(e) = validate_learning_metadata(&learning_val) {
            return Ok(error_response(e));
        }

        if dry_run {
            previews.push(json!({
                "legacy_id": legacy_id,
                "outcome": "eligible",
                "reason": classification.reason,
                "kind": kind,
                "status": status,
                "scope": scope,
                "would_create": true,
                "would_invalidate_source": invalidate_source,
            }));
            continue;
        }

        let embedding = state.embedding.embed(&legacy.content).await?;
        let now = Datetime::default();
        let memory = Memory {
            content: legacy.content.clone(),
            embedding: Some(embedding),
            memory_type,
            metadata: Some(json!({ "learning": learning_val })),
            event_time: now,
            ingestion_time: now,
            valid_from: now,
            importance_score: legacy.importance_score,
            content_hash: Some(ContentHasher::hash(&legacy.content)),
            embedding_state: EmbeddingState::Ready,
            ..Default::default()
        };

        let created_id = match state.storage.create_memory(memory).await {
            Ok(id) => id,
            Err(e) => return Ok(error_response(e)),
        };
        counts.created += 1;
        if let Ok(Some(created)) = state.storage.get_memory(&created_id).await {
            state.memory_search.upsert_memory(created.clone()).await;
            let mut stripped = created;
            strip_embedding(&mut stripped);
            created_records.push(serde_json::to_value(&stripped).unwrap_or(json!({})));
        }

        if invalidate_source {
            match state
                .storage
                .invalidate(&legacy_id, Some("migration_replaced"), Some(&created_id))
                .await
            {
                Ok(_) => {}
                Err(e) => return Ok(error_response(e)),
            };
            if let Ok(Some(updated_source)) = state.storage.get_memory(&legacy_id).await {
                state.memory_search.upsert_memory(updated_source).await;
            }
        }

        previews.push(json!({
            "legacy_id": legacy_id,
            "outcome": "created",
            "reason": classification.reason,
            "kind": kind,
            "status": status,
            "scope": scope,
            "created_memory_id": created_id,
            "source_invalidated": invalidate_source,
        }));
    }

    let counts_json = json!({
        "scanned": counts.scanned,
        "eligible": counts.eligible,
        "created": counts.created,
        "skipped": counts.skipped,
        "ambiguous": counts.ambiguous,
        "already_migrated": counts.already_migrated,
        "invalidated_skipped": counts.invalidated_skipped,
    });

    Ok(success_json(json!({
        "dry_run": dry_run,
        "limit": limit,
        "counts": counts_json,
        "previews": previews,
        "created_records": created_records,
        "summary": {
            "result_kind": "learning_migration",
            "counts": counts_json,
            "partial": serde_json::Value::Null,
        },
        "contract": learning_collection_contract_json(),
    })))
}

/// Compatibility shim: soft-delete via archive (default) or reject (mode = "soft_reject").
/// Never performs a hard delete.
pub async fn delete(
    state: &AppState,
    params: LearningMemoryDeleteParams,
) -> anyhow::Result<CallToolResult> {
    let use_reject = params.mode.as_deref() == Some("soft_reject");

    if use_reject {
        reject(
            state,
            LearningMemoryRejectParams {
                id: params.id,
                reason: None,
            },
        )
        .await
    } else {
        archive(
            state,
            LearningMemoryArchiveParams {
                id: params.id,
                reason: None,
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::server::logic::learning_lifecycle::{
        derive_lifecycle_state, LearningLifecycleState,
    };
    use crate::server::params::LearningMemoryMigrateLegacyParams;
    use crate::types::{
        learning::{LearningKind, LearningStatus, ScopeLevel},
        Memory, MemoryType,
    };
    use serde_json::json;

    use super::{classify_legacy_memory, extract_learning_fields, kind_status_to_memory_type};

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

    fn migrate_params() -> LearningMemoryMigrateLegacyParams {
        LearningMemoryMigrateLegacyParams {
            prefix_allowlist: None,
            scope: Some("project".to_string()),
            project_id: Some("project-alpha".to_string()),
            dry_run: true,
            limit: None,
            include_invalidated: None,
            invalidate_source: None,
            extract_research_lessons: None,
        }
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
        assert!(
            validate_promote_transition(LearningStatus::Candidate, LearningStatus::Confirmed)
                .is_ok()
        );
    }

    #[test]
    fn promote_candidate_to_rule_allowed() {
        assert!(
            validate_promote_transition(LearningStatus::Candidate, LearningStatus::Rule).is_ok()
        );
    }

    #[test]
    fn promote_confirmed_to_rule_allowed() {
        assert!(
            validate_promote_transition(LearningStatus::Confirmed, LearningStatus::Rule).is_ok()
        );
    }

    #[test]
    fn promote_confirmed_to_candidate_rejected() {
        assert!(
            validate_promote_transition(LearningStatus::Confirmed, LearningStatus::Candidate)
                .is_err()
        );
    }

    #[test]
    fn promote_rule_to_confirmed_rejected() {
        assert!(
            validate_promote_transition(LearningStatus::Rule, LearningStatus::Confirmed).is_err()
        );
    }

    #[test]
    fn promote_rule_to_candidate_rejected() {
        assert!(
            validate_promote_transition(LearningStatus::Rule, LearningStatus::Candidate).is_err()
        );
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
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Superseded
        );
    }

    #[test]
    fn candidate_memory_has_candidate_lifecycle() {
        let m = make_memory_with_status(LearningStatus::Candidate);
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Candidate
        );
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
        let new_memory_type: Option<MemoryType> =
            if matches!(LearningStatus::Confirmed, LearningStatus::Rule) {
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

    #[test]
    fn migrate_classifies_user_preference_prefix_as_confirmed() {
        let m = Memory::new("USER — Preference: Prefer concise replies".to_string());
        let result = classify_legacy_memory(&m, &migrate_params());
        assert_eq!(result.outcome, "eligible");
        assert_eq!(result.kind, Some(LearningKind::UserPreference));
        assert_eq!(result.status, Some(LearningStatus::Confirmed));
    }

    #[test]
    fn migrate_marks_plain_user_prefix_ambiguous_without_metadata_hint() {
        let m = Memory::new("USER: Likes fast answers".to_string());
        let result = classify_legacy_memory(&m, &migrate_params());
        assert_eq!(result.outcome, "ambiguous");
    }

    #[test]
    fn migrate_classifies_plain_user_prefix_with_preference_metadata() {
        let mut m = Memory::new("USER: Likes fast answers".to_string());
        m.metadata = Some(json!({ "legacy_kind": "preference" }));
        let result = classify_legacy_memory(&m, &migrate_params());
        assert_eq!(result.outcome, "eligible");
        assert_eq!(result.kind, Some(LearningKind::UserPreference));
        assert_eq!(result.status, Some(LearningStatus::Confirmed));
    }

    #[test]
    fn migrate_classifies_context_as_project_pattern_only_with_project_scope() {
        let m = Memory::new("CONTEXT: Rust modules keep logic in src/server/logic".to_string());
        let result = classify_legacy_memory(&m, &migrate_params());
        assert_eq!(result.outcome, "eligible");
        assert_eq!(result.kind, Some(LearningKind::ProjectPattern));
        assert_eq!(result.status, Some(LearningStatus::Candidate));
        assert_eq!(result.scope.unwrap().level, ScopeLevel::Project);

        let mut global_params = migrate_params();
        global_params.scope = Some("global".to_string());
        global_params.project_id = None;
        let result = classify_legacy_memory(&m, &global_params);
        assert_eq!(result.outcome, "ambiguous");
    }

    #[test]
    fn migrate_research_requires_explicit_lesson_extraction() {
        let m = Memory::new("RESEARCH: The cache warmed fastest with small batches".to_string());
        let result = classify_legacy_memory(&m, &migrate_params());
        assert_eq!(result.outcome, "ambiguous");

        let mut params = migrate_params();
        params.extract_research_lessons = Some(true);
        let result = classify_legacy_memory(&m, &params);
        assert_eq!(result.outcome, "eligible");
        assert_eq!(result.kind, Some(LearningKind::ProjectLesson));
        assert_eq!(result.status, Some(LearningStatus::Candidate));
    }

    #[test]
    fn migrate_excludes_task_and_epic_by_default() {
        let task = Memory::new("TASK: WP01 in progress".to_string());
        let epic = Memory::new("EPIC: Learning memory".to_string());
        assert_eq!(
            classify_legacy_memory(&task, &migrate_params()).outcome,
            "skipped"
        );
        assert_eq!(
            classify_legacy_memory(&epic, &migrate_params()).outcome,
            "skipped"
        );
    }

    #[test]
    fn migrate_marks_existing_learning_schema_as_already_migrated() {
        let m = make_memory_with_status(LearningStatus::Confirmed);
        let result = classify_legacy_memory(&m, &migrate_params());
        assert_eq!(result.outcome, "already_migrated");
    }

    #[test]
    fn learning_reject_sets_rejected_lifecycle() {
        let m = make_invalidated_memory("learning_rejected");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Rejected);
        assert!(m.valid_until.is_some());
        assert_eq!(m.invalidation_reason.as_deref(), Some("learning_rejected"));
    }

    #[test]
    fn learning_archive_sets_archived_lifecycle() {
        let m = make_invalidated_memory("learning_archived");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Archived);
        assert!(m.valid_until.is_some());
        assert_eq!(m.invalidation_reason.as_deref(), Some("learning_archived"));
    }

    #[test]
    fn rejected_memory_excluded_from_default_list_and_search() {
        use crate::server::logic::learning_response::compute_default_inclusion;
        let m = make_invalidated_memory("learning_rejected");
        let lifecycle = derive_lifecycle_state(&m);
        let (list, search, inject) =
            compute_default_inclusion(&LearningStatus::Rejected, &lifecycle);
        assert!(!list);
        assert!(!search);
        assert!(!inject);
    }

    #[test]
    fn archived_memory_excluded_from_default_list_and_search() {
        use crate::server::logic::learning_response::compute_default_inclusion;
        let m = make_invalidated_memory("learning_archived");
        let lifecycle = derive_lifecycle_state(&m);
        let (list, search, inject) =
            compute_default_inclusion(&LearningStatus::Archived, &lifecycle);
        assert!(!list);
        assert!(!search);
        assert!(!inject);
    }

    #[test]
    fn learning_delete_shim_defaults_to_archive_behavior() {
        let m = make_invalidated_memory("learning_archived");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Archived);
        assert!(m.valid_until.is_some(), "soft delete must set valid_until");
    }

    #[test]
    fn learning_delete_shim_soft_reject_mode() {
        let m = make_invalidated_memory("learning_rejected");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Rejected);
        assert!(m.valid_until.is_some(), "soft delete must set valid_until");
    }

    #[test]
    fn audit_filter_retrieves_rejected_via_invalidation_reason() {
        let m = make_invalidated_memory("learning_rejected");
        let lifecycle = derive_lifecycle_state(&m);
        assert_eq!(lifecycle, LearningLifecycleState::Rejected);
        assert!(m.invalidation_reason.is_some());
    }

    #[test]
    fn audit_filter_retrieves_archived_via_invalidation_reason() {
        let m = make_invalidated_memory("learning_archived");
        let lifecycle = derive_lifecycle_state(&m);
        assert_eq!(lifecycle, LearningLifecycleState::Archived);
        assert!(m.invalidation_reason.is_some());
    }

    #[test]
    fn superseded_memory_has_superseded_lifecycle_via_invalidation_reason() {
        let m = make_invalidated_memory("superseded");
        let lifecycle = derive_lifecycle_state(&m);
        assert_eq!(lifecycle, LearningLifecycleState::Superseded);
        assert!(m.valid_until.is_some());
        assert!(m.invalidation_reason.is_some());
    }

    #[test]
    fn terminal_states_are_rejected_archived_superseded() {
        for (reason, expected) in [
            ("learning_rejected", LearningLifecycleState::Rejected),
            ("learning_archived", LearningLifecycleState::Archived),
            ("superseded", LearningLifecycleState::Superseded),
        ] {
            let m = make_invalidated_memory(reason);
            assert_eq!(derive_lifecycle_state(&m), expected, "reason={reason}");
            assert!(
                matches!(
                    derive_lifecycle_state(&m),
                    LearningLifecycleState::Rejected
                        | LearningLifecycleState::Archived
                        | LearningLifecycleState::Superseded
                ),
                "reason={reason} should be terminal"
            );
        }
    }

    #[test]
    fn active_states_are_not_terminal() {
        for status in [
            LearningStatus::Candidate,
            LearningStatus::Confirmed,
            LearningStatus::Rule,
        ] {
            let m = make_memory_with_status(status.clone());
            let lifecycle = derive_lifecycle_state(&m);
            assert!(
                !matches!(
                    lifecycle,
                    LearningLifecycleState::Rejected
                        | LearningLifecycleState::Archived
                        | LearningLifecycleState::Superseded
                ),
                "status={status:?} should not be terminal"
            );
        }
    }

    use super::{confidence_multiplier, importance_multiplier, scope_boost, status_boost};
    use crate::types::learning::{CreatedFrom, LearningScope, LearningSource};

    fn make_global_scope() -> LearningScope {
        LearningScope {
            level: ScopeLevel::Global,
            project_id: None,
            workspace: None,
            mode: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
        }
    }

    fn make_project_scope(project_id: &str) -> LearningScope {
        LearningScope {
            level: ScopeLevel::Project,
            project_id: Some(project_id.to_string()),
            workspace: None,
            mode: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
        }
    }

    #[test]
    fn rule_status_has_highest_boost() {
        assert!(status_boost(&LearningStatus::Rule) > status_boost(&LearningStatus::Confirmed));
        assert!(
            status_boost(&LearningStatus::Confirmed) > status_boost(&LearningStatus::Candidate)
        );
    }

    #[test]
    fn rule_boost_is_1_3() {
        assert!((status_boost(&LearningStatus::Rule) - 1.3).abs() < f32::EPSILON);
    }

    #[test]
    fn confirmed_boost_is_1_1() {
        assert!((status_boost(&LearningStatus::Confirmed) - 1.1).abs() < f32::EPSILON);
    }

    #[test]
    fn candidate_boost_is_0_9() {
        assert!((status_boost(&LearningStatus::Candidate) - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn exact_scope_match_gives_1_2_boost() {
        let scope = make_global_scope();
        let boost = scope_boost(&scope, Some("global"), None);
        assert!((boost - 1.2).abs() < f32::EPSILON);
    }

    #[test]
    fn no_scope_match_gives_1_0_boost() {
        let scope = make_global_scope();
        let boost = scope_boost(&scope, Some("project"), None);
        assert!((boost - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn project_scope_exact_match_with_same_project_id_gives_1_2() {
        let scope = make_project_scope("my-project");
        let boost = scope_boost(&scope, Some("project"), Some("my-project"));
        assert!((boost - 1.2).abs() < f32::EPSILON);
    }

    #[test]
    fn project_scope_exact_match_with_different_project_id_gives_1_0() {
        let scope = make_project_scope("my-project");
        let boost = scope_boost(&scope, Some("project"), Some("other-project"));
        assert!((boost - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_1_0_gives_multiplier_1_5() {
        assert!((confidence_multiplier(1.0) - 1.5).abs() < 1e-5);
    }

    #[test]
    fn confidence_0_0_gives_multiplier_0_5() {
        assert!((confidence_multiplier(0.0) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn confidence_0_5_gives_multiplier_1_0() {
        assert!((confidence_multiplier(0.5) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn importance_0_gives_multiplier_0_5() {
        assert!((importance_multiplier(0.0) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn importance_5_gives_multiplier_2_0() {
        assert!((importance_multiplier(5.0) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn importance_multiplier_clamped_above_5() {
        assert!((importance_multiplier(10.0) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn confidence_multiplier_clamped_below_0() {
        assert!((confidence_multiplier(-1.0) - 0.5).abs() < 1e-5);
    }
}
