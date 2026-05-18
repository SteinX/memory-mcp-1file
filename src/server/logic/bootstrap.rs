use std::collections::BTreeMap;
use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::{json, Value};

use crate::config::AppState;
use crate::server::params::{MemoryAuditParams, MemoryBootstrapParams};
use crate::storage::traits::MemoryGcFilter;
use crate::storage::StorageBackend;
use crate::types::{record_key_to_string, ExportIdentity, Memory, MemoryQuery, MemoryType};

use super::contracts::{export_contract_meta, summary_collection_response, with_surface_guidance};
use super::memory_lifecycle::{derive_memory_lifecycle, MemoryLifecycleState, RetentionPolicy};
use super::{error_response, normalize_limit, success_json};

const BOOTSTRAP_FETCH_LIMIT: usize = 200;
const DEFAULT_BOOTSTRAP_TOKEN_BUDGET: usize = 4_000;
const RECOVERY_SIMILARITY_THRESHOLD: f32 = 0.72;

fn bootstrap_contract_json() -> Value {
    let contract = with_surface_guidance(
        export_contract_meta(ExportIdentity::default(), None),
        &[
            "active_tasks",
            "stable_context",
            "recovery",
            "project",
            "memory_health",
            "selection_summary",
            "contract",
            "summary",
        ],
        &[],
        &[],
    );
    serde_json::to_value(contract).unwrap_or_else(|_| json!({}))
}

fn memory_id(memory: &Memory) -> Option<String> {
    memory.id.as_ref().map(|id| record_key_to_string(&id.key))
}

fn memory_type_value(memory_type: &MemoryType) -> Value {
    serde_json::to_value(memory_type).unwrap_or_else(|_| json!("semantic"))
}

fn estimate_tokens(content: &str) -> usize {
    (content.chars().count() / 4).max(1)
}

fn memory_entry(memory: &Memory, category: &str, reason: &str) -> Value {
    json!({
        "id": memory_id(memory),
        "content": memory.content,
        "memory_type": memory_type_value(&memory.memory_type),
        "namespace": memory.namespace,
        "user_id": memory.user_id,
        "agent_id": memory.agent_id,
        "run_id": memory.run_id,
        "category": category,
        "reason": reason,
        "estimated_tokens": estimate_tokens(&memory.content),
        "ingestion_time": memory.ingestion_time,
        "importance_score": memory.importance_score,
    })
}

fn content_prefix(content: &str) -> Option<&'static str> {
    let trimmed = content.trim_start();
    [
        "TASK:",
        "DECISION:",
        "USER:",
        "RESEARCH:",
        "PROJECT:",
        "CONTEXT:",
        "EPIC:",
    ]
    .into_iter()
    .find(|prefix| trimmed.starts_with(prefix))
}

fn stable_group(prefix: &str) -> &'static str {
    match prefix {
        "DECISION:" => "decision",
        "USER:" => "user",
        "RESEARCH:" => "research",
        "PROJECT:" => "project",
        "CONTEXT:" => "context",
        "EPIC:" => "epic",
        _ => "other",
    }
}

fn is_active_task_candidate(memory: &Memory) -> bool {
    if content_prefix(&memory.content) != Some("TASK:") {
        return false;
    }
    let lower = memory.content.to_lowercase();
    lower.contains("status: in_progress")
        || lower.contains("status=in_progress")
        || lower.contains("in_progress")
        || lower.contains("current:")
        || lower.contains("[ ]")
}

fn task_priority(memory: &Memory) -> usize {
    let lower = memory.content.to_lowercase();
    usize::from(lower.contains("in_progress")) * 4
        + usize::from(lower.contains("current:")) * 2
        + usize::from(lower.contains("[ ]"))
}

fn words(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .filter_map(|part| {
            let trimmed = part.trim().to_lowercase();
            (trimmed.len() >= 3).then_some(trimmed)
        })
        .collect()
}

fn token_similarity(left: &str, right: &str) -> f32 {
    let left_words: std::collections::BTreeSet<_> = words(left).into_iter().collect();
    let right_words: std::collections::BTreeSet<_> = words(right).into_iter().collect();
    if left_words.is_empty() || right_words.is_empty() {
        return 0.0;
    }
    let intersection = left_words.intersection(&right_words).count() as f32;
    let union = left_words.union(&right_words).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn is_recovery_detail(memory: &Memory, prompt: Option<&str>) -> bool {
    let content = memory.content.as_str();
    let lower = content.to_lowercase();
    lower.contains("command:")
        || lower.contains("path:")
        || lower.contains("current:")
        || lower.contains("verification")
        || lower.contains("failed")
        || lower.contains("blocker")
        || lower.contains("updated:")
        || lower.contains("todo")
        || lower.contains("continue")
        || content.contains('`')
        || content.contains("/")
        || prompt
            .map(|prompt| token_similarity(prompt, content) > 0.12)
            .unwrap_or(false)
}

fn compact_repeats(memory: &Memory, compact_summary: Option<&str>) -> bool {
    compact_summary
        .map(|summary| token_similarity(&memory.content, summary) >= RECOVERY_SIMILARITY_THRESHOLD)
        .unwrap_or(false)
}

#[derive(Default)]
struct SelectionBudget {
    limit: usize,
    token_budget: usize,
    used_tokens: usize,
    truncated_by_limit: usize,
    truncated_by_token_budget: usize,
}

impl SelectionBudget {
    fn new(limit: usize, token_budget: usize) -> Self {
        Self {
            limit,
            token_budget,
            ..Default::default()
        }
    }

    fn select(&mut self, candidates: Vec<Value>) -> Vec<Value> {
        let mut out = Vec::new();
        for candidate in candidates {
            if out.len() >= self.limit {
                self.truncated_by_limit += 1;
                continue;
            }
            let tokens = candidate
                .get("estimated_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(1) as usize;
            if self.used_tokens + tokens > self.token_budget {
                self.truncated_by_token_budget += 1;
                continue;
            }
            self.used_tokens += tokens;
            out.push(candidate);
        }
        out
    }
}

async fn project_readiness(state: &Arc<AppState>, project_id: Option<&str>) -> Value {
    match project_id {
        Some(project_id) => match state.storage.get_index_status(project_id).await {
            Ok(Some(status)) => json!({
                "project_id": project_id,
                "status": status.status.to_string(),
                "total_files": status.total_files,
                "indexed_files": status.indexed_files,
                "total_chunks": status.total_chunks,
                "total_symbols": status.total_symbols,
                "structural_state": status.structural_state.to_string(),
                "semantic_state": status.semantic_state.to_string(),
                "projection_state": status.projection_state.to_string(),
                "error_message": status.error_message,
            }),
            Ok(None) => json!({
                "project_id": project_id,
                "status": "missing",
                "reason_code": "missing",
                "message": "No persisted index status found for project_id."
            }),
            Err(error) => json!({
                "project_id": project_id,
                "status": "degraded",
                "reason_code": "degraded",
                "message": error.to_string(),
            }),
        },
        None => match state.storage.list_index_statuses().await {
            Ok(statuses) => json!({
                "project_id": Value::Null,
                "known_projects": statuses.len(),
                "ready_projects": statuses.iter().filter(|status| {
                    status.status == crate::types::IndexState::Completed
                }).count(),
                "reason_code": "partial",
                "message": "No project_id supplied; returning aggregate index readiness only."
            }),
            Err(error) => json!({
                "project_id": Value::Null,
                "known_projects": 0,
                "reason_code": "degraded",
                "message": error.to_string(),
            }),
        },
    }
}

async fn memory_health(state: &Arc<AppState>, namespace: Option<&str>) -> Value {
    let gc_filter = MemoryGcFilter {
        namespace: namespace.map(ToOwned::to_owned),
        memory_type: None,
        invalidation_reason: None,
        invalidated_before: None,
    };
    let invalidated = state
        .storage
        .count_invalidated_memories_for_gc(&gc_filter)
        .await
        .unwrap_or(0);
    let invalidated_sample = state
        .storage
        .list_invalidated_memories_for_gc(&gc_filter, 100, 0)
        .await
        .unwrap_or_default();
    let now = chrono::Utc::now();
    let policy = RetentionPolicy::default();
    let mut purge_eligible = 0usize;
    let mut pinned = 0usize;
    for memory in &invalidated_sample {
        let lifecycle = derive_memory_lifecycle(memory, now, &policy);
        if lifecycle.purge_eligible {
            purge_eligible += 1;
        }
        if lifecycle.pinned {
            pinned += 1;
        }
    }
    let reason_code = if state.embedding.is_ready() {
        "ok"
    } else {
        "partial"
    };

    json!({
        "gc_backlog": {
            "invalidated_count": invalidated,
            "sampled_purge_eligible_count": purge_eligible,
            "sampled_pinned_count": pinned,
        },
        "learning_memory": {
            "readiness": "available",
            "promotion_is_explicit": true,
            "tools": ["learning_memory_create", "learning_memory_promote", "learning_memory_supersede", "learning_memory_reject", "learning_memory_archive"]
        },
        "embedding": {
            "ready": state.embedding.is_ready()
        },
        "partial": {
            "is_partial": !state.embedding.is_ready(),
            "reason_code": reason_code,
            "reason": if state.embedding.is_ready() { "ready" } else { "embedding_model_not_ready" },
            "message": if state.embedding.is_ready() {
                "Memory search dependencies are ready."
            } else {
                "Embedding model is not ready; bootstrap still returns storage-backed context."
            }
        }
    })
}

fn base_query(params: &MemoryBootstrapParams) -> anyhow::Result<MemoryQuery> {
    params.to_memory_query()
}

pub async fn memory_bootstrap(
    state: &Arc<AppState>,
    params: MemoryBootstrapParams,
) -> anyhow::Result<CallToolResult> {
    let limit = normalize_limit(params.limit).min(25);
    let token_budget = params
        .token_budget
        .unwrap_or(DEFAULT_BOOTSTRAP_TOKEN_BUDGET)
        .min(20_000)
        .max(256);
    let filters = match base_query(&params) {
        Ok(filters) => filters,
        Err(error) => return Ok(error_response(error)),
    };

    let fetch_limit = BOOTSTRAP_FETCH_LIMIT.max(limit * 8);
    let memories = match state.storage.get_valid(&filters, fetch_limit).await {
        Ok(memories) => memories,
        Err(error) => return Ok(error_response(error)),
    };

    let mut active_task_candidates: Vec<_> = memories
        .iter()
        .filter(|memory| is_active_task_candidate(memory))
        .cloned()
        .collect();
    active_task_candidates.sort_by(|a, b| task_priority(b).cmp(&task_priority(a)));

    let mut stable_context_candidates: BTreeMap<&'static str, Vec<Value>> = BTreeMap::new();
    for memory in &memories {
        if let Some(prefix) = content_prefix(&memory.content) {
            if prefix == "TASK:" {
                continue;
            }
            let group = stable_group(prefix);
            stable_context_candidates
                .entry(group)
                .or_default()
                .push(memory_entry(memory, group, "prefix_group"));
        }
    }

    let recovery_candidates: Vec<_> = memories
        .iter()
        .filter(|memory| {
            is_recovery_detail(memory, params.prompt.as_deref())
                && !compact_repeats(memory, params.compact_summary.as_deref())
        })
        .map(|memory| memory_entry(memory, "recovery", "operational_detail"))
        .collect();

    let mut active_budget = SelectionBudget::new(limit, token_budget / 2);
    let active_tasks = active_budget.select(
        active_task_candidates
            .iter()
            .map(|memory| memory_entry(memory, "active_task", "active_task_priority"))
            .collect(),
    );

    let mut stable_groups = serde_json::Map::new();
    let mut stable_truncated_limit = 0usize;
    let mut stable_truncated_tokens = 0usize;
    let stable_group_budget = (token_budget / 3).max(128);
    for group in ["decision", "user", "research", "project", "context", "epic"] {
        let candidates = stable_context_candidates.remove(group).unwrap_or_default();
        let mut budget = SelectionBudget::new(limit, stable_group_budget);
        let selected = budget.select(candidates);
        stable_truncated_limit += budget.truncated_by_limit;
        stable_truncated_tokens += budget.truncated_by_token_budget;
        stable_groups.insert(group.to_string(), Value::Array(selected));
    }

    let mut recovery_budget = SelectionBudget::new(limit, token_budget / 3);
    let recovery = recovery_budget.select(recovery_candidates);

    let selected_count = active_tasks.len()
        + stable_groups
            .values()
            .filter_map(Value::as_array)
            .map(Vec::len)
            .sum::<usize>()
        + recovery.len();
    let truncated_by_limit = active_budget.truncated_by_limit
        + stable_truncated_limit
        + recovery_budget.truncated_by_limit;
    let truncated_by_token_budget = active_budget.truncated_by_token_budget
        + stable_truncated_tokens
        + recovery_budget.truncated_by_token_budget;
    let is_partial = truncated_by_limit > 0 || truncated_by_token_budget > 0;

    Ok(success_json(json!({
        "active_tasks": active_tasks,
        "stable_context": stable_groups,
        "recovery": recovery,
        "project": project_readiness(state, params.project_id.as_deref()).await,
        "memory_health": memory_health(state, params.namespace.as_deref()).await,
        "selection_summary": {
            "limit": limit,
            "token_budget": token_budget,
            "estimated_tokens_used": active_budget.used_tokens + recovery_budget.used_tokens,
            "candidate_count": memories.len(),
            "returned_count": selected_count,
            "truncated_by_limit": truncated_by_limit,
            "truncated_by_token_budget": truncated_by_token_budget,
            "compact_summary_filter": {
                "enabled": params.compact_summary.is_some(),
                "similarity_threshold": RECOVERY_SIMILARITY_THRESHOLD,
            },
            "groups": {
                "active_tasks": active_tasks.len(),
                "stable_context": stable_groups.iter().map(|(key, value)| (key.clone(), value.as_array().map(Vec::len).unwrap_or(0))).collect::<BTreeMap<_, _>>(),
                "recovery": recovery.len(),
            }
        },
        "contract": bootstrap_contract_json(),
        "summary": summary_collection_response(
            "memory_bootstrap",
            selected_count,
            Some(memories.len()),
            is_partial,
            if is_partial {
                Some("Bootstrap selection was truncated by limit or token_budget.".to_string())
            } else {
                None
            }
        )
    })))
}

fn audit_query(params: &MemoryAuditParams) -> anyhow::Result<MemoryQuery> {
    Ok(MemoryQuery {
        user_id: None,
        agent_id: None,
        run_id: None,
        namespace: params.namespace.clone(),
        memory_type: params.memory_type_filter()?,
        metadata_filter: None,
        valid_at: None,
        event_after: None,
        event_before: None,
        ingestion_after: None,
        ingestion_before: None,
    })
}

fn audit_contract_json() -> Value {
    let contract = with_surface_guidance(
        export_contract_meta(ExportIdentity::default(), None),
        &[
            "lifecycle_counts",
            "purge_readiness",
            "observation_counts",
            "operator_signals",
            "contract",
            "summary",
        ],
        &[],
        &[],
    );
    serde_json::to_value(contract).unwrap_or_else(|_| json!({}))
}

fn increment_nested_count(
    map: &mut BTreeMap<String, BTreeMap<String, usize>>,
    source: &str,
    event_type: &str,
) {
    *map.entry(source.to_string())
        .or_default()
        .entry(event_type.to_string())
        .or_default() += 1;
}

fn observation_counts(memories: &[Memory]) -> Value {
    let mut by_source_event: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut total = 0usize;
    for memory in memories {
        let Some(observation) = memory
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("observation"))
        else {
            continue;
        };
        let source = observation
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let event_type = observation
            .get("event_type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        increment_nested_count(&mut by_source_event, source, event_type);
        total += 1;
    }

    json!({
        "total": total,
        "by_source_event_type": by_source_event,
    })
}

pub async fn memory_audit(
    state: &Arc<AppState>,
    params: MemoryAuditParams,
) -> anyhow::Result<CallToolResult> {
    let limit = normalize_limit(params.limit);
    let filters = match audit_query(&params) {
        Ok(filters) => filters,
        Err(error) => return Ok(error_response(error)),
    };
    let gc_filter = MemoryGcFilter {
        namespace: params.namespace.clone(),
        memory_type: params.memory_type_filter().unwrap_or(None),
        invalidation_reason: None,
        invalidated_before: None,
    };
    let active_memories = state
        .storage
        .get_valid(&filters, limit)
        .await
        .unwrap_or_default();
    let active_count = active_memories.len();
    let invalidated_memories = state
        .storage
        .list_invalidated_memories_for_gc(&gc_filter, limit, 0)
        .await
        .unwrap_or_default();
    let invalidated_count = state
        .storage
        .count_invalidated_memories_for_gc(&gc_filter)
        .await
        .unwrap_or(invalidated_memories.len());

    let now = chrono::Utc::now();
    let policy = RetentionPolicy::default();
    let mut superseded_count = 0usize;
    let mut archived_count = 0usize;
    let mut rejected_count = 0usize;
    let mut purge_eligible = 0usize;
    let mut pinned = 0usize;
    let mut next_eligible_window: Option<Value> = None;
    let mut operator_flags = Vec::new();

    for memory in active_memories.iter().chain(invalidated_memories.iter()) {
        let lifecycle = derive_memory_lifecycle(memory, now, &policy);
        match lifecycle.lifecycle_state {
            MemoryLifecycleState::Superseded => superseded_count += 1,
            MemoryLifecycleState::Archived => archived_count += 1,
            MemoryLifecycleState::Rejected => rejected_count += 1,
            _ => {}
        }
        if lifecycle.purge_eligible {
            purge_eligible += 1;
        }
        if lifecycle.pinned {
            pinned += 1;
        }
        if next_eligible_window.is_none() {
            if let Some(eligible_after) = lifecycle.eligible_after {
                next_eligible_window = Some(json!(crate::types::Datetime::from(eligible_after)));
            }
        }
        if memory.superseded_by.is_some() && memory.valid_until.is_none() {
            operator_flags.push(json!({
                "memory_id": memory_id(memory),
                "signal": "replacement_linked_without_invalidation",
            }));
        }
        if memory.content.chars().count() > 8_000 {
            operator_flags.push(json!({
                "memory_id": memory_id(memory),
                "signal": "long_memory_truncation_risk",
            }));
        }
        if memory.content_hash.is_none() {
            operator_flags.push(json!({
                "memory_id": memory_id(memory),
                "signal": "missing_content_fingerprint",
            }));
        }
    }

    let mut observation_sample = active_memories.clone();
    if params.include_invalidated.unwrap_or(false) {
        observation_sample.extend(invalidated_memories.clone());
    }

    Ok(success_json(json!({
        "lifecycle_counts": {
            "active": active_count,
            "invalidated": invalidated_count,
            "superseded": superseded_count,
            "archived_learning": archived_count,
            "rejected_learning": rejected_count,
        },
        "purge_readiness": {
            "eligible_count_sampled": purge_eligible,
            "pinned_count_sampled": pinned,
            "invalidated_backlog": invalidated_count,
            "next_eligible_window": next_eligible_window,
        },
        "observation_counts": observation_counts(&observation_sample),
        "operator_signals": {
            "requires_attention": !operator_flags.is_empty(),
            "signals": operator_flags,
            "sample_limit": limit,
        },
        "contract": audit_contract_json(),
        "summary": summary_collection_response(
            "memory_audit",
            active_memories.len() + invalidated_memories.len(),
            Some(active_count + invalidated_count),
            (active_count + invalidated_count) > active_memories.len() + invalidated_memories.len(),
            None
        )
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::ContentHasher;
    use crate::server::params::{MemoryAuditParams, MemoryBootstrapParams};
    use crate::test_utils::TestContext;
    use crate::types::{EmbeddingState, Memory, MemoryType};

    fn text_result(result: &CallToolResult) -> Value {
        match &result.content[0].raw {
            rmcp::model::RawContent::Text(text) => serde_json::from_str(&text.text).unwrap(),
            other => panic!("unexpected content: {other:?}"),
        }
    }

    async fn seed(ctx: &TestContext, content: &str, memory_type: MemoryType) -> String {
        let mut memory = Memory::new(content.to_string()).with_type(memory_type);
        memory.namespace = Some("boot".to_string());
        memory.embedding = Some(vec![0.1; 768]);
        memory.embedding_state = EmbeddingState::Ready;
        memory.content_hash = Some(ContentHasher::hash(content));
        let id = ctx.state.storage.create_memory(memory).await.unwrap();
        let stored = ctx.state.storage.get_memory(&id).await.unwrap().unwrap();
        ctx.state.memory_search.upsert_memory(stored).await;
        id
    }

    #[tokio::test]
    async fn bootstrap_returns_grouped_memory_context() {
        let ctx = TestContext::new().await;
        seed(
            &ctx,
            "TASK: WP01\nStatus: in_progress\nCurrent: T002",
            MemoryType::Episodic,
        )
        .await;
        seed(
            &ctx,
            "DECISION: Use explicit bootstrap contract",
            MemoryType::Semantic,
        )
        .await;
        seed(
            &ctx,
            "USER: prefers scoped server changes",
            MemoryType::Semantic,
        )
        .await;
        seed(
            &ctx,
            "RESEARCH: agentmemory suggests bootstrap",
            MemoryType::Semantic,
        )
        .await;

        let result = memory_bootstrap(
            &ctx.state,
            MemoryBootstrapParams {
                prompt: Some("continue WP01".to_string()),
                compact_summary: None,
                namespace: Some("boot".to_string()),
                user_id: None,
                agent_id: None,
                run_id: None,
                project_id: None,
                limit: Some(5),
                token_budget: Some(2_000),
            },
        )
        .await
        .unwrap();

        let json = text_result(&result);
        assert_eq!(json["active_tasks"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["stable_context"]["decision"].as_array().unwrap().len(),
            1
        );
        assert_eq!(json["stable_context"]["user"].as_array().unwrap().len(), 1);
        assert_eq!(
            json["stable_context"]["research"].as_array().unwrap().len(),
            1
        );
        assert!(json.get("contract").is_some());
        assert!(json.get("summary").is_some());
    }

    #[tokio::test]
    async fn bootstrap_filters_compact_summary_recovery_duplicates() {
        let ctx = TestContext::new().await;
        seed(
            &ctx,
            "TASK: WP02\nStatus: in_progress\nCurrent: T010\nPath: src/server/logic/bootstrap.rs",
            MemoryType::Episodic,
        )
        .await;

        let result = memory_bootstrap(
            &ctx.state,
            MemoryBootstrapParams {
                prompt: Some("resume bootstrap work".to_string()),
                compact_summary: Some(
                    "TASK WP02 Status in_progress Current T010 Path src server logic bootstrap"
                        .to_string(),
                ),
                namespace: Some("boot".to_string()),
                user_id: None,
                agent_id: None,
                run_id: None,
                project_id: None,
                limit: Some(5),
                token_budget: Some(2_000),
            },
        )
        .await
        .unwrap();
        let json = text_result(&result);
        assert_eq!(json["recovery"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn bootstrap_reports_limit_and_token_truncation() {
        let ctx = TestContext::new().await;
        for index in 0..4 {
            seed(
                &ctx,
                &format!(
                    "DECISION: bootstrap truncation candidate {index} {}",
                    "word ".repeat(200)
                ),
                MemoryType::Semantic,
            )
            .await;
        }

        let result = memory_bootstrap(
            &ctx.state,
            MemoryBootstrapParams {
                prompt: None,
                compact_summary: None,
                namespace: Some("boot".to_string()),
                user_id: None,
                agent_id: None,
                run_id: None,
                project_id: None,
                limit: Some(1),
                token_budget: Some(256),
            },
        )
        .await
        .unwrap();
        let json = text_result(&result);
        assert!(
            json["selection_summary"]["truncated_by_limit"]
                .as_u64()
                .unwrap()
                > 0
                || json["selection_summary"]["truncated_by_token_budget"]
                    .as_u64()
                    .unwrap()
                    > 0
        );
        assert_eq!(
            json["summary"]["partial"]["reason_code"].as_str(),
            Some("partial")
        );
    }

    #[tokio::test]
    async fn audit_reports_lifecycle_and_observations() {
        let ctx = TestContext::new().await;
        let active_id = seed(&ctx, "CONTEXT: active observation", MemoryType::Episodic).await;
        let mut active = ctx
            .state
            .storage
            .get_memory(&active_id)
            .await
            .unwrap()
            .unwrap();
        active.metadata = Some(json!({
            "observation": {
                "schema_version": 1,
                "source": "plugin",
                "event_type": "session_start",
                "confidence": 0.9,
                "redaction_state": "none",
                "created_from": "memory_observation_create"
            }
        }));
        ctx.state
            .storage
            .update_memory(
                &active_id,
                crate::types::MemoryUpdate {
                    metadata: active.metadata.clone(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let invalidated_id = seed(&ctx, "CONTEXT: superseded old", MemoryType::Semantic).await;
        ctx.state
            .storage
            .invalidate(&invalidated_id, Some("superseded"), Some("replacement"))
            .await
            .unwrap();

        let result = memory_audit(
            &ctx.state,
            MemoryAuditParams {
                namespace: Some("boot".to_string()),
                memory_type: None,
                include_invalidated: Some(true),
                limit: Some(20),
            },
        )
        .await
        .unwrap();
        let json = text_result(&result);
        assert!(json["lifecycle_counts"]["active"].as_u64().unwrap() >= 1);
        assert!(json["lifecycle_counts"]["invalidated"].as_u64().unwrap() >= 1);
        assert_eq!(
            json["observation_counts"]["by_source_event_type"]["plugin"]["session_start"].as_u64(),
            Some(1)
        );
        assert!(json.get("contract").is_some());
        assert!(json.get("summary").is_some());
    }
}
