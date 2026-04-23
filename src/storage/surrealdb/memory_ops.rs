use surrealdb::engine::local::Db;
use surrealdb::Surreal;

use crate::storage::traits::CapacityMemoryCandidate;
use crate::types::{Memory, MemoryQuery, MemoryUpdate, SearchResult};
use crate::types::SurrealValue;
use crate::Result;

use super::helpers::{generate_id, parse_thing};

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
    let result: Option<Memory> = db.select(("memories", id)).await?;
    Ok(result)
}

pub(super) async fn update_memory(
    db: &Surreal<Db>,
    id: &str,
    update: MemoryUpdate,
) -> Result<Memory> {
    let existing: Option<Memory> = db.select(("memories", id)).await?;
    let mut memory = existing.ok_or_else(|| crate::types::AppError::NotFound(id.to_string()))?;

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
        SET access_count = math::max(0, access_count) + 1,
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
        "SELECT * FROM memories WHERE {} ORDER BY ingestion_time DESC LIMIT $limit START $offset",
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
        "SELECT * FROM memories WHERE {}",
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
               access_count,
               last_accessed_at,
               importance_score
        FROM memories
        WHERE valid_until IS NONE OR valid_until > time::now()
    "#;
    let mut response = db.query(sql).await?;
    let rows: Vec<CapacityCandidateRow> = response.take(0)?;
    Ok(rows.into_iter().map(CapacityMemoryCandidate::from).collect())
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
        "SELECT * FROM memories WHERE content_hash = $content_hash AND {} ORDER BY ingestion_time DESC",
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
               event_time, ingestion_time, access_count, last_accessed_at,
               user_id, agent_id, run_id, namespace, metadata, superseded_by,
               valid_until, invalidation_reason
        FROM memories
        WHERE {where_clause}
          AND {}
        LIMIT $limit
    "#
        , base_filter_clause("time::now()")
    );

    let mut response = bind_memory_query(db.query(&sql), filters)
        .bind(("limit", fetch_limit));
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
            event_time, ingestion_time, access_count, last_accessed_at,
            user_id, agent_id, run_id, namespace, metadata, superseded_by,
            valid_until, invalidation_reason
        FROM memories 
        WHERE embedding <|{knn_k},{ef}|> $vec
          AND embedding IS NOT NONE
          AND {}
        ORDER BY score DESC 
        LIMIT $limit
    "#
        , base_filter_clause("time::now()")
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
    let sql = format!(r#"
        SELECT * FROM memories 
        WHERE {}
        ORDER BY ingestion_time DESC
        LIMIT $limit
    "#, base_filter_clause("time::now()"));
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
    let timestamp = filters
        .valid_at
        .clone()
        .ok_or_else(|| crate::types::AppError::Internal(anyhow::anyhow!("valid_at timestamp required").into()))?;

    let sql = format!(r#"
        SELECT * FROM memories 
        WHERE {}
        ORDER BY ingestion_time DESC
        LIMIT $limit
    "#, base_filter_clause("$timestamp"));
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
    db.query(
        "UPDATE $thing SET embedding = $emb, content_hash = $hash, embedding_state = $state",
    )
    .bind(("thing", thing))
    .bind(("emb", embedding))
    .bind(("hash", content_hash))
    .bind(("state", embedding_state.to_string()))
    .await?;
    Ok(())
}
