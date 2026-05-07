use surrealdb::engine::local::Db;
use surrealdb::Surreal;

use std::collections::{HashMap, HashSet};

use crate::storage::traits::{CapacityMemoryCandidate, MemoryExportOptions, MemoryImportOptions};
use crate::types::SurrealValue;
use crate::types::{
    record_key_to_string, AppError, ExportMemoryResponse, ImportConflictStrategy, ImportError,
    ImportErrorCode, ImportIdMapping, ImportMemoryResponse, Memory, MemoryQuery, MemoryUpdate,
    MigrationMemoryRecord, MigrationRecordType, MigrationSummary, SearchResult,
    MEMORY_MIGRATION_SCHEMA_VERSION,
};
use crate::Result;

use super::helpers::{generate_id, parse_thing};

const MEMORY_SELECT: &str = "*, access_count OR 0 AS access_count";

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct SearchRow {
    id: String,
    content: String,
    #[serde(default)]
    content_hash: Option<String>,
    #[serde(default)]
    memory_type: crate::types::MemoryType,
    score: f32,
    #[serde(default = "default_importance_score")]
    importance_score: f32,
    #[serde(default)]
    event_time: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    ingestion_time: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    access_count: u32,
    #[serde(default)]
    last_accessed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    #[serde(default)]
    superseded_by: Option<String>,
    #[serde(default)]
    valid_until: Option<crate::types::Datetime>,
    #[serde(default)]
    invalidation_reason: Option<String>,
    #[serde(default)]
    consolidation_trace: Option<serde_json::Value>,
    #[serde(default)]
    replacement_lineage: Option<serde_json::Value>,
    #[serde(default)]
    attention_summary: Option<serde_json::Value>,
    #[serde(default)]
    operator_summary: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct CapacityCandidateRow {
    id: String,
    memory_type: crate::types::MemoryType,
    #[serde(default)]
    event_time: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    ingestion_time: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    access_count: u32,
    #[serde(default)]
    last_accessed_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default = "default_importance_score")]
    importance_score: f32,
}

impl From<CapacityCandidateRow> for CapacityMemoryCandidate {
    fn from(row: CapacityCandidateRow) -> Self {
        Self {
            id: row.id,
            memory_type: row.memory_type,
            event_time: row.event_time,
            ingestion_time: row.ingestion_time,
            access_count: row.access_count,
            last_accessed_at: row.last_accessed_at,
            importance_score: row.importance_score,
        }
    }
}

fn default_importance_score() -> f32 {
    1.0
}

impl From<SearchRow> for SearchResult {
    fn from(row: SearchRow) -> Self {
        Self {
            id: row.id,
            content: row.content,
            content_hash: row.content_hash,
            memory_type: row.memory_type,
            score: row.score,
            importance_score: row.importance_score,
            event_time: row.event_time,
            ingestion_time: row.ingestion_time,
            access_count: row.access_count,
            last_accessed_at: row.last_accessed_at,
            user_id: row.user_id,
            agent_id: row.agent_id,
            run_id: row.run_id,
            namespace: row.namespace,
            metadata: row.metadata,
            superseded_by: row.superseded_by,
            valid_until: row.valid_until,
            invalidation_reason: row.invalidation_reason,
            consolidation_trace: row.consolidation_trace,
            replacement_lineage: row.replacement_lineage,
            attention_summary: row.attention_summary,
            operator_summary: row.operator_summary,
        }
    }
}

pub(super) async fn create_memory(db: &Surreal<Db>, mut memory: Memory) -> Result<String> {
    let id = generate_id();
    memory.id = Some(crate::types::RecordId::new("memories", id.as_str()));
    let _: Option<Memory> = db.create(("memories", id.as_str())).content(memory).await?;
    Ok(id)
}

pub(super) async fn get_memory(db: &Surreal<Db>, id: &str) -> Result<Option<Memory>> {
    let sql = format!("SELECT {MEMORY_SELECT} FROM memories WHERE id = type::record($id) LIMIT 1");
    let mut response = db
        .query(&sql)
        .bind(("id", format!("memories:{id}")))
        .await?;
    let mut memories: Vec<Memory> = response.take(0)?;
    Ok(memories.pop())
}

pub(super) async fn update_memory(
    db: &Surreal<Db>,
    id: &str,
    update: MemoryUpdate,
) -> Result<Memory> {
    let mut memory = get_memory(db, id)
        .await?
        .ok_or_else(|| crate::types::AppError::NotFound(id.to_string()))?;

    if let Some(content) = update.content {
        memory.content = content;
    }
    if let Some(memory_type) = update.memory_type {
        memory.memory_type = memory_type;
    }
    if let Some(user_id) = update.user_id {
        memory.user_id = Some(user_id);
    }
    if let Some(agent_id) = update.agent_id {
        memory.agent_id = Some(agent_id);
    }
    if let Some(run_id) = update.run_id {
        memory.run_id = Some(run_id);
    }
    if let Some(namespace) = update.namespace {
        memory.namespace = Some(namespace);
    }
    if let Some(importance_score) = update.importance_score {
        memory.importance_score = importance_score;
    }
    if let Some(metadata) = update.metadata {
        memory.metadata = Some(metadata);
    }
    if let Some(embedding) = update.embedding {
        memory.embedding = Some(embedding);
    }
    if let Some(content_hash) = update.content_hash {
        memory.content_hash = Some(content_hash);
    }
    if let Some(embedding_state) = update.embedding_state {
        memory.embedding_state = embedding_state;
    }

    let updated: Option<Memory> = db.update(("memories", id)).content(memory).await?;
    updated.ok_or_else(|| crate::types::AppError::NotFound(id.to_string()))
}

pub(super) async fn record_memory_access(
    db: &Surreal<Db>,
    id: &str,
    accessed_at: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    let sql = r#"
        UPDATE memories
        SET access_count = (access_count OR 0) + 1,
            last_accessed_at = $accessed_at
        WHERE id = (type::record($id))
    "#;

    let response = db
        .query(sql)
        .bind(("id", format!("memories:{id}")))
        .bind(("accessed_at", accessed_at))
        .await?;
    response.check()?;
    Ok(())
}

pub(super) async fn delete_memory(db: &Surreal<Db>, id: &str) -> Result<bool> {
    let deleted: Option<Memory> = db.delete(("memories", id)).await?;
    Ok(deleted.is_some())
}

pub(super) async fn list_memories(
    db: &Surreal<Db>,
    filters: &MemoryQuery,
    limit: usize,
    offset: usize,
) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {MEMORY_SELECT} FROM memories WHERE {} ORDER BY ingestion_time DESC LIMIT $limit START $offset",
        base_filter_clause("time::now()")
    );
    let mut response = bind_memory_query(db.query(&sql), filters)
        .bind(("limit", limit))
        .bind(("offset", offset))
        .await?;
    let mut memories: Vec<Memory> = response.take(0)?;
    memories.retain(|m| metadata_matches(m.metadata.as_ref(), filters.metadata_filter.as_ref()));
    Ok(memories)
}

pub(super) async fn count_memories(db: &Surreal<Db>) -> Result<usize> {
    let mut response = db.query("SELECT count() FROM memories GROUP ALL").await?;
    let result: Option<serde_json::Value> = response.take(0)?;
    let count = result
        .and_then(|v| v.get("count").and_then(|c| c.as_u64()))
        .unwrap_or(0) as usize;
    Ok(count)
}

pub(super) async fn count_memories_filtered(
    db: &Surreal<Db>,
    filters: &MemoryQuery,
) -> Result<usize> {
    let sql = format!(
        "SELECT {MEMORY_SELECT} FROM memories WHERE {}",
        base_filter_clause("time::now()")
    );
    let mut response = bind_memory_query(db.query(&sql), filters).await?;
    let mut memories: Vec<Memory> = response.take(0)?;
    memories.retain(|m| metadata_matches(m.metadata.as_ref(), filters.metadata_filter.as_ref()));
    Ok(memories.len())
}

pub(super) async fn count_valid_memories(db: &Surreal<Db>) -> Result<usize> {
    let sql = r#"
        SELECT count() AS count
        FROM memories
        WHERE valid_until IS NONE OR valid_until > time::now()
        GROUP ALL
    "#;
    let mut response = db.query(sql).await?;
    let result: Option<serde_json::Value> = response.take(0)?;
    let count = result
        .and_then(|v| v.get("count").and_then(|c| c.as_u64()))
        .unwrap_or(0) as usize;
    Ok(count)
}

pub(super) async fn list_capacity_candidates(
    db: &Surreal<Db>,
) -> Result<Vec<CapacityMemoryCandidate>> {
    let sql = r#"
        SELECT meta::id(id) AS id,
               memory_type,
               event_time,
               ingestion_time,
               access_count OR 0 AS access_count,
               last_accessed_at,
               importance_score
        FROM memories
        WHERE valid_until IS NONE OR valid_until > time::now()
    "#;
    let mut response = db.query(sql).await?;
    let rows: Vec<CapacityCandidateRow> = response.take(0)?;
    Ok(rows
        .into_iter()
        .map(CapacityMemoryCandidate::from)
        .collect())
}

pub(super) async fn get_memory_last_accessed_at(
    db: &Surreal<Db>,
    id: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct LastAccessRow {
        #[serde(default)]
        last_accessed_at: Option<chrono::DateTime<chrono::Utc>>,
    }

    let sql = r#"
        SELECT last_accessed_at
        FROM memories
        WHERE id = type::record($id)
        LIMIT 1
    "#;
    let mut response = db.query(sql).bind(("id", format!("memories:{id}"))).await?;
    let row: Option<LastAccessRow> = response.take(0)?;
    Ok(row.and_then(|row| row.last_accessed_at))
}

pub(super) async fn find_memories_by_content_hash(
    db: &Surreal<Db>,
    filters: &MemoryQuery,
    content_hash: &str,
) -> Result<Vec<Memory>> {
    let sql = format!(
        "SELECT {MEMORY_SELECT} FROM memories WHERE content_hash = $content_hash AND {} ORDER BY ingestion_time DESC",
        base_filter_clause("time::now()")
    );
    let mut response = bind_memory_query(db.query(&sql), filters)
        .bind(("content_hash", content_hash.to_string()))
        .await?;
    let mut memories: Vec<Memory> = response.take(0)?;
    memories.retain(|m| metadata_matches(m.metadata.as_ref(), filters.metadata_filter.as_ref()));
    Ok(memories)
}

pub(super) async fn bm25_search(
    db: &Surreal<Db>,
    query: &str,
    filters: &MemoryQuery,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    // SurrealDB v3.0.0: search::score() is broken (bug #6852/#6946).
    // We split query into words and require ALL words present (AND logic),
    // then score in Rust using term-frequency counting as a proxy for BM25.
    let fetch_limit = (limit * 3).max(limit);

    // Split query into individual words for multi-word matching
    let words: Vec<&str> = query.split_whitespace().filter(|w| w.len() >= 2).collect();
    if words.is_empty() {
        return Ok(vec![]);
    }

    // Build WHERE clause: each word must be present (AND)
    let conditions: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, _)| format!("string::lowercase(content) CONTAINS string::lowercase($w{i})"))
        .collect();
    let where_clause = conditions.join(" AND ");

    let sql = format!(
        r#"
        SELECT meta::id(id) AS id, content, content_hash, memory_type, 1.0f AS score,
               importance_score,
               event_time, ingestion_time, access_count OR 0 AS access_count, last_accessed_at,
               user_id, agent_id, run_id, namespace, metadata, superseded_by,
               valid_until, invalidation_reason
        FROM memories
        WHERE {where_clause}
          AND {}
        LIMIT $limit
    "#,
        base_filter_clause("time::now()")
    );

    let mut response = bind_memory_query(db.query(&sql), filters).bind(("limit", fetch_limit));
    for (i, word) in words.iter().enumerate() {
        response = response.bind((format!("w{i}"), word.to_string()));
    }
    let mut response = response.await?;
    let mut results: Vec<SearchResult> = response
        .take::<Vec<SearchRow>>(0)?
        .into_iter()
        .map(SearchResult::from)
        .collect();
    results.retain(|r| metadata_matches(r.metadata.as_ref(), filters.metadata_filter.as_ref()));

    // Compute relevance score in Rust: normalized term frequency.
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();
    for r in &mut results {
        let content_lower = r.content.to_lowercase();
        let mut total_tf: f32 = 0.0;
        for qw in &query_words {
            let count = content_lower.matches(qw).count() as f32;
            total_tf += count;
        }
        // TF-like score: more word hits in shorter content = higher relevance.
        let tf = total_tf / (content_lower.len() as f32 + 1.0) * 1000.0;
        r.score = tf.clamp(0.01, 1.0);
    }

    // Sort by score descending, then truncate to requested limit.
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    Ok(results)
}

pub(super) async fn vector_search(
    db: &Surreal<Db>,
    embedding: &[f32],
    filters: &MemoryQuery,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    // Use HNSW index via <|K,EF|> KNN operator for fast candidate selection,
    // then compute exact cosine similarity for scoring.
    // Over-fetch because the valid_until post-filter may discard some KNN hits.
    let knn_k = (limit * 4).min(200);
    let ef = knn_k.max(150);

    let query = format!(
        r#"
        SELECT meta::id(id) AS id, content, content_hash, memory_type,
            vector::similarity::cosine(embedding, $vec) AS score,
            importance_score,
            event_time, ingestion_time, access_count OR 0 AS access_count, last_accessed_at,
            user_id, agent_id, run_id, namespace, metadata, superseded_by,
            valid_until, invalidation_reason
        FROM memories 
        WHERE embedding <|{knn_k},{ef}|> $vec
          AND embedding IS NOT NONE
          AND {}
        ORDER BY score DESC 
        LIMIT $limit
    "#,
        base_filter_clause("time::now()")
    );
    let mut response = bind_memory_query(db.query(&query), filters)
        .bind(("vec", embedding.to_vec()))
        .bind(("limit", limit))
        .await?;
    let mut results: Vec<SearchResult> = response
        .take::<Vec<SearchRow>>(0)?
        .into_iter()
        .map(SearchResult::from)
        .collect();
    results.retain(|r| metadata_matches(r.metadata.as_ref(), filters.metadata_filter.as_ref()));
    Ok(results)
}

pub(super) async fn get_valid(
    db: &Surreal<Db>,
    filters: &MemoryQuery,
    limit: usize,
) -> Result<Vec<Memory>> {
    let sql = format!(
        r#"
        SELECT {MEMORY_SELECT} FROM memories 
        WHERE {}
        ORDER BY ingestion_time DESC
        LIMIT $limit
    "#,
        base_filter_clause("time::now()")
    );
    let mut response = bind_memory_query(db.query(&sql), filters)
        .bind(("limit", limit))
        .await?;
    let mut memories: Vec<Memory> = response.take(0)?;
    memories.retain(|m| metadata_matches(m.metadata.as_ref(), filters.metadata_filter.as_ref()));
    Ok(memories)
}

pub(super) async fn get_valid_at(
    db: &Surreal<Db>,
    filters: &MemoryQuery,
    limit: usize,
) -> Result<Vec<Memory>> {
    let timestamp = filters.valid_at.clone().ok_or_else(|| {
        crate::types::AppError::Internal(anyhow::anyhow!("valid_at timestamp required").into())
    })?;

    let sql = format!(
        r#"
        SELECT {MEMORY_SELECT} FROM memories 
        WHERE {}
        ORDER BY ingestion_time DESC
        LIMIT $limit
    "#,
        base_filter_clause("$timestamp")
    );
    let mut response = bind_memory_query(db.query(&sql), filters)
        .bind(("timestamp", timestamp))
        .bind(("limit", limit))
        .await?;
    let mut memories: Vec<Memory> = response.take(0)?;
    memories.retain(|m| metadata_matches(m.metadata.as_ref(), filters.metadata_filter.as_ref()));
    Ok(memories)
}

fn base_filter_clause(reference_time_expr: &str) -> String {
    format!(
        "(($valid_at IS NONE AND (valid_until IS NONE OR valid_until > {reference_time_expr})) OR \
          ($valid_at IS NOT NONE AND valid_from <= $valid_at AND (valid_until IS NONE OR valid_until > $valid_at))) \
          AND ($user_id IS NONE OR user_id = $user_id) \
          AND ($agent_id IS NONE OR agent_id = $agent_id) \
          AND ($run_id IS NONE OR run_id = $run_id) \
          AND ($namespace IS NONE OR namespace = $namespace) \
          AND ($memory_type IS NONE OR memory_type = $memory_type) \
          AND ($event_after IS NONE OR event_time >= $event_after) \
          AND ($event_before IS NONE OR event_time <= $event_before) \
          AND ($ingestion_after IS NONE OR ingestion_time >= $ingestion_after) \
          AND ($ingestion_before IS NONE OR ingestion_time <= $ingestion_before)"
    )
}

fn bind_memory_query<'a>(
    mut query: surrealdb::method::Query<'a, Db>,
    filters: &MemoryQuery,
) -> surrealdb::method::Query<'a, Db> {
    query = query.bind(("valid_at", filters.valid_at.clone()));
    query = query.bind(("user_id", filters.user_id.clone()));
    query = query.bind(("agent_id", filters.agent_id.clone()));
    query = query.bind(("run_id", filters.run_id.clone()));
    query = query.bind(("namespace", filters.namespace.clone()));
    query = query.bind((
        "memory_type",
        filters.memory_type.as_ref().map(|m| match m {
            crate::types::MemoryType::Episodic => "episodic".to_string(),
            crate::types::MemoryType::Semantic => "semantic".to_string(),
            crate::types::MemoryType::Procedural => "procedural".to_string(),
        }),
    ));
    query = query.bind(("event_after", filters.event_after.clone()));
    query = query.bind(("event_before", filters.event_before.clone()));
    query = query.bind(("ingestion_after", filters.ingestion_after.clone()));
    query.bind(("ingestion_before", filters.ingestion_before.clone()))
}

fn metadata_matches(
    candidate: Option<&serde_json::Value>,
    filter: Option<&serde_json::Value>,
) -> bool {
    match filter {
        None => true,
        Some(filter) => match candidate {
            Some(candidate) => json_contains(candidate, filter),
            None => false,
        },
    }
}

fn json_contains(candidate: &serde_json::Value, filter: &serde_json::Value) -> bool {
    match (candidate, filter) {
        (serde_json::Value::Object(candidate_map), serde_json::Value::Object(filter_map)) => {
            filter_map.iter().all(|(key, filter_value)| {
                candidate_map
                    .get(key)
                    .map(|candidate_value| json_contains(candidate_value, filter_value))
                    .unwrap_or(false)
            })
        }
        (serde_json::Value::Array(candidate_items), serde_json::Value::Array(filter_items)) => {
            filter_items.iter().all(|filter_item| {
                candidate_items
                    .iter()
                    .any(|candidate_item| json_contains(candidate_item, filter_item))
            })
        }
        _ => candidate == filter,
    }
}

pub(super) async fn invalidate(
    db: &Surreal<Db>,
    id: &str,
    reason: Option<&str>,
    superseded_by: Option<&str>,
) -> Result<bool> {
    let thing = crate::types::RecordId::new("memories", id);
    let sql = r#"
        UPDATE $thing SET 
            valid_until = time::now(),
            invalidation_reason = $reason,
            superseded_by = $superseded_by
    "#;
    let mut response = db
        .query(sql)
        .bind(("thing", thing))
        .bind(("reason", reason.map(String::from)))
        .bind(("superseded_by", superseded_by.map(String::from)))
        .await?;
    let updated: Option<Memory> = response.take(0).ok().flatten();
    Ok(updated.is_some())
}

/// Directly update embedding fields for a memory (used by stale re-embed process).
pub(super) async fn raw_update_embedding(
    db: &Surreal<Db>,
    id: &str,
    embedding: Vec<f32>,
    content_hash: String,
    embedding_state: &str,
) -> Result<()> {
    let thing = parse_thing(&format!("memories:{}", id))?;
    db.query("UPDATE $thing SET embedding = $emb, content_hash = $hash, embedding_state = $state")
        .bind(("thing", thing))
        .bind(("emb", embedding))
        .bind(("hash", content_hash))
        .bind(("state", embedding_state.to_string()))
        .await?;
    Ok(())
}

pub(super) async fn export_memories(
    db: &Surreal<Db>,
    options: &MemoryExportOptions,
) -> Result<ExportMemoryResponse> {
    validate_project_scope(&options.project_id)?;

    let mut filters = options.filters.clone();
    filters.namespace = Some(options.project_id.clone());
    let fetch_limit = options
        .limit
        .map(|limit| limit.saturating_add(1))
        .unwrap_or(1_000_000usize);
    let validity_clause = if options.include_invalidated {
        "TRUE"
    } else if options.valid_only {
        "valid_until IS NONE OR valid_until > time::now()"
    } else {
        "TRUE"
    };
    let sql = format!(
        r#"
        SELECT {MEMORY_SELECT}
        FROM memories
        WHERE namespace = $project_id
          AND {validity_clause}
          AND ($user_id IS NONE OR user_id = $user_id)
          AND ($agent_id IS NONE OR agent_id = $agent_id)
          AND ($run_id IS NONE OR run_id = $run_id)
          AND ($memory_type IS NONE OR memory_type = $memory_type)
          AND ($event_after IS NONE OR event_time >= $event_after)
          AND ($event_before IS NONE OR event_time <= $event_before)
          AND ($ingestion_after IS NONE OR ingestion_time >= $ingestion_after)
          AND ($ingestion_before IS NONE OR ingestion_time <= $ingestion_before)
        ORDER BY ingestion_time DESC
        LIMIT $limit
    "#
    );
    let mut response = bind_memory_query(db.query(&sql), &filters)
        .bind(("project_id", options.project_id.clone()))
        .bind(("limit", fetch_limit))
        .await?;
    let mut memories: Vec<Memory> = response.take(0)?;
    memories.retain(|m| metadata_matches(m.metadata.as_ref(), filters.metadata_filter.as_ref()));

    let truncated = options.limit.is_some_and(|limit| memories.len() > limit);
    if let Some(limit) = options.limit {
        memories.truncate(limit);
    }

    let records: Vec<MigrationMemoryRecord> = memories
        .iter()
        .map(|memory| memory_to_migration_record(memory, &options.project_id))
        .collect::<Result<Vec<_>>>()?;
    let jsonl = records
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error).into()))?
        .join("\n");
    let invalidated_records = records.iter().filter(|record| record.invalidated).count();

    Ok(ExportMemoryResponse {
        schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
        record_type: MigrationRecordType::Memory,
        jsonl,
        exported_count: records.len(),
        truncated,
        summary: MigrationSummary {
            schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
            record_type: MigrationRecordType::Memory,
            total_records: records.len(),
            memory_records: records.len(),
            exported_records: records.len(),
            imported_records: 0,
            skipped_records: 0,
            failed_records: 0,
            valid_records: records.len().saturating_sub(invalidated_records),
            invalidated_records,
            dry_run: false,
        },
    })
}

pub(super) async fn import_memories(
    db: &Surreal<Db>,
    records: Vec<MigrationMemoryRecord>,
    options: &MemoryImportOptions,
) -> Result<ImportMemoryResponse> {
    validate_project_scope(&options.project_id)?;

    let payload_ids: HashSet<String> = records.iter().map(|record| record.id.clone()).collect();
    let mut seen_ids = HashSet::new();
    let mut errors = Vec::new();
    let mut skipped_count = 0usize;
    let mut failed_count = 0usize;
    let mut valid_records = 0usize;
    let mut invalidated_records = 0usize;
    let mut importable = Vec::new();

    for (index, record) in records.into_iter().enumerate() {
        let line_number = Some(index + 1);
        if let Some(error) = record.unsupported_schema_version_error(line_number) {
            failed_count += 1;
            errors.push(error);
            continue;
        }
        if record.record_type != MigrationRecordType::Memory {
            failed_count += 1;
            errors.push(ImportError {
                code: ImportErrorCode::InvalidRecordType,
                message: "Only memory migration records can be imported by memory storage"
                    .to_string(),
                line_number,
                source_id: Some(record.id.clone()),
                field: Some("record_type".to_string()),
            });
            continue;
        }
        if record.id.trim().is_empty() {
            failed_count += 1;
            errors.push(ImportError {
                code: ImportErrorCode::MissingRequiredField,
                message: "Memory migration record id is required".to_string(),
                line_number,
                source_id: None,
                field: Some("id".to_string()),
            });
            continue;
        }
        if record.content.is_empty() {
            failed_count += 1;
            errors.push(ImportError {
                code: ImportErrorCode::MissingRequiredField,
                message: "Memory migration record content is required".to_string(),
                line_number,
                source_id: Some(record.id.clone()),
                field: Some("content".to_string()),
            });
            continue;
        }
        if !seen_ids.insert(record.id.clone()) {
            failed_count += 1;
            errors.push(ImportError {
                code: ImportErrorCode::StorageError,
                message: format!("Duplicate memory id {} in import payload", record.id),
                line_number,
                source_id: Some(record.id.clone()),
                field: Some("id".to_string()),
            });
            continue;
        }
        if record.invalidated || record.valid_until.is_some() {
            invalidated_records += 1;
            if !options.allow_invalidated {
                skipped_count += 1;
                errors.push(ImportError {
                    code: ImportErrorCode::StorageError,
                    message: "Invalidated memory records require allow_invalidated import option"
                        .to_string(),
                    line_number,
                    source_id: Some(record.id.clone()),
                    field: Some("invalidated".to_string()),
                });
                continue;
            }
        } else {
            valid_records += 1;
        }
        importable.push(record);
    }

    let importable_ids: Vec<String> = importable.iter().map(|record| record.id.clone()).collect();
    let conflicts = existing_memory_ids(db, &importable_ids).await?;
    let mut id_map = HashMap::new();
    let mut id_mappings = Vec::new();
    for record in &importable {
        let old_id = record.id.clone();
        let new_id = if conflicts.contains(&old_id) {
            match options.conflict_strategy {
                ImportConflictStrategy::Remap => {
                    deterministic_remap_id(&old_id, &options.project_id)
                }
                ImportConflictStrategy::Skip => {
                    skipped_count += 1;
                    continue;
                }
                ImportConflictStrategy::Fail => {
                    failed_count += 1;
                    errors.push(ImportError {
                        code: ImportErrorCode::StorageError,
                        message: format!("Memory id {old_id} already exists"),
                        line_number: None,
                        source_id: Some(old_id),
                        field: Some("id".to_string()),
                    });
                    continue;
                }
            }
        } else {
            old_id.clone()
        };
        if new_id != old_id {
            id_mappings.push(ImportIdMapping {
                old_id: old_id.clone(),
                new_id: new_id.clone(),
            });
        }
        id_map.insert(old_id, new_id);
    }

    let planned_records: Vec<(MigrationMemoryRecord, String)> = importable
        .into_iter()
        .filter_map(|record| {
            id_map
                .get(&record.id)
                .cloned()
                .map(|new_id| (record, new_id))
        })
        .collect();

    let has_errors = failed_count > 0 || !errors.is_empty();
    let committed_count = if options.dry_run || has_errors {
        0
    } else {
        for (record, new_id) in &planned_records {
            let memory =
                migration_record_to_memory(record, &options.project_id, &id_map, &payload_ids);
            let _: Option<Memory> = db
                .create(("memories", new_id.as_str()))
                .content(memory)
                .await?;
            if let Some(target) = record.superseded_by.as_ref() {
                let mapped_target = if payload_ids.contains(target) {
                    id_map
                        .get(target)
                        .cloned()
                        .unwrap_or_else(|| target.clone())
                } else {
                    target.clone()
                };
                db.query("UPDATE $thing SET superseded_by = $superseded_by")
                    .bind((
                        "thing",
                        crate::types::RecordId::new("memories", new_id.as_str()),
                    ))
                    .bind(("superseded_by", mapped_target))
                    .await?
                    .check()?;
            }
        }
        planned_records.len()
    };

    let planned_count = planned_records.len();
    Ok(ImportMemoryResponse {
        schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
        record_type: MigrationRecordType::Memory,
        conflict_strategy: options.conflict_strategy.clone(),
        dry_run: options.dry_run,
        imported_count: committed_count,
        skipped_count,
        failed_count,
        summary: MigrationSummary {
            schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
            record_type: MigrationRecordType::Memory,
            total_records: valid_records + invalidated_records + skipped_count + failed_count,
            memory_records: valid_records + invalidated_records + skipped_count + failed_count,
            exported_records: 0,
            imported_records: committed_count,
            skipped_records: skipped_count,
            failed_records: failed_count,
            valid_records,
            invalidated_records,
            dry_run: options.dry_run,
        },
        id_mappings,
        errors,
    })
}

fn validate_project_scope(project_id: &str) -> Result<()> {
    if project_id.trim().is_empty() {
        return Err(AppError::InvalidPath(
            "project_id is required for memory migration storage operations".to_string(),
        ));
    }
    Ok(())
}

async fn existing_memory_ids(db: &Surreal<Db>, ids: &[String]) -> Result<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let record_ids: Vec<crate::types::RecordId> = ids
        .iter()
        .map(|id| crate::types::RecordId::new("memories", id.as_str()))
        .collect();
    let mut response = db
        .query("SELECT meta::id(id) AS id FROM memories WHERE id IN $ids")
        .bind(("ids", record_ids))
        .await?;
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct IdRow {
        id: String,
    }
    let rows: Vec<IdRow> = response.take(0)?;
    Ok(rows.into_iter().map(|row| row.id).collect())
}

fn deterministic_remap_id(old_id: &str, project_id: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    project_id.hash(&mut hasher);
    old_id.hash(&mut hasher);
    format!("{old_id}-import-{:016x}", hasher.finish())
}

fn memory_to_migration_record(memory: &Memory, project_id: &str) -> Result<MigrationMemoryRecord> {
    let id = memory
        .id
        .as_ref()
        .map(|id| record_key_to_string(&id.key))
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("memory record missing id").into()))?;
    let invalidated = memory.valid_until.is_some();
    Ok(MigrationMemoryRecord {
        schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
        record_type: MigrationRecordType::Memory,
        id,
        content: memory.content.clone(),
        memory_type: memory.memory_type.clone(),
        user_id: memory.user_id.clone(),
        agent_id: memory.agent_id.clone(),
        run_id: memory.run_id.clone(),
        namespace: memory.namespace.clone(),
        project_id: Some(project_id.to_string()),
        metadata: memory.metadata.clone(),
        importance_score: memory.importance_score,
        created_at: memory.ingestion_time,
        updated_at: memory.ingestion_time,
        valid_from: memory.valid_from,
        valid_until: memory.valid_until,
        superseded_by: memory.superseded_by.clone(),
        invalidated,
        invalidation_reason: memory.invalidation_reason.clone(),
    })
}

fn migration_record_to_memory(
    record: &MigrationMemoryRecord,
    project_id: &str,
    id_map: &HashMap<String, String>,
    payload_ids: &HashSet<String>,
) -> Memory {
    let superseded_by = record.superseded_by.as_ref().map(|id| {
        if payload_ids.contains(id) {
            id_map.get(id).cloned().unwrap_or_else(|| id.clone())
        } else {
            id.clone()
        }
    });

    Memory {
        id: None,
        content: record.content.clone(),
        embedding: None,
        memory_type: record.memory_type.clone(),
        user_id: record.user_id.clone(),
        agent_id: record.agent_id.clone(),
        run_id: record.run_id.clone(),
        namespace: Some(project_id.to_string()),
        metadata: record.metadata.clone(),
        event_time: record.created_at,
        ingestion_time: record.created_at,
        valid_from: record.valid_from,
        valid_until: record.valid_until,
        importance_score: record.importance_score,
        access_count: 0,
        last_accessed_at: None,
        invalidation_reason: record.invalidation_reason.clone(),
        superseded_by,
        content_hash: None,
        embedding_state: Default::default(),
    }
}
