use surrealdb::engine::local::Db;
use surrealdb::Surreal;

use crate::types::{Datetime, Memory, MemoryUpdate, SearchResult};
use crate::Result;

use super::helpers::generate_id;

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
    if let Some(metadata) = update.metadata {
        memory.metadata = Some(metadata);
    }

    let updated: Option<Memory> = db.update(("memories", id)).content(memory).await?;
    updated.ok_or_else(|| crate::types::AppError::NotFound(id.to_string()))
}

pub(super) async fn delete_memory(db: &Surreal<Db>, id: &str) -> Result<bool> {
    let deleted: Option<Memory> = db.delete(("memories", id)).await?;
    Ok(deleted.is_some())
}

pub(super) async fn list_memories(
    db: &Surreal<Db>,
    limit: usize,
    offset: usize,
) -> Result<Vec<Memory>> {
    let query = "SELECT * FROM memories ORDER BY ingestion_time DESC LIMIT $limit START $offset";
    let mut response = db
        .query(query)
        .bind(("limit", limit))
        .bind(("offset", offset))
        .await?;
    let memories: Vec<Memory> = response.take(0)?;
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

pub(super) async fn bm25_search(
    db: &Surreal<Db>,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    // SurrealDB v3.0.0: search::score() is broken (bug #6852/#6946).
    // We use CONTAINS for filtering (substring match), then score in Rust using
    // term-frequency counting as a proxy for BM25 relevance.
    // Fetch 3x limit to allow Rust-side reranking, then truncate.
    let fetch_limit = (limit * 3).max(limit);
    let sql = r#"
        SELECT meta::id(id) AS id, content, memory_type, 1.0f AS score, metadata
        FROM memories
        WHERE string::lowercase(content) CONTAINS string::lowercase($query)
          AND (valid_until IS NONE OR valid_until > time::now())
        LIMIT $limit
    "#;
    let mut response = db
        .query(sql)
        .bind(("query", query.to_string()))
        .bind(("limit", fetch_limit))
        .await?;
    let mut results: Vec<SearchResult> = response.take(0)?;

    // Compute relevance score in Rust: normalized term frequency.
    // score = occurrences / (content_len + 1) * 1000, capped at 1.0.
    let query_lower = query.to_lowercase();
    for r in &mut results {
        let content_lower = r.content.to_lowercase();
        let count = content_lower.matches(query_lower.as_str()).count() as f32;
        // TF-like score: more occurrences in shorter content = higher relevance.
        let tf = count / (content_lower.len() as f32 + 1.0) * 1000.0;
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
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let query = r#"
        SELECT meta::id(id) AS id, content, memory_type,
            vector::similarity::cosine(embedding, $vec) AS score, metadata 
        FROM memories 
        WHERE embedding IS NOT NONE 
          AND (valid_until IS NONE OR valid_until > time::now())
        ORDER BY score DESC 
        LIMIT $limit
    "#;
    let mut response = db
        .query(query)
        .bind(("vec", embedding.to_vec()))
        .bind(("limit", limit))
        .await?;
    let results: Vec<SearchResult> = response.take(0)?;
    Ok(results)
}

pub(super) async fn get_valid(
    db: &Surreal<Db>,
    user_id: Option<&str>,
    limit: usize,
) -> Result<Vec<Memory>> {
    let sql = r#"
        SELECT * FROM memories 
        WHERE (valid_until IS NONE OR valid_until > time::now())
          AND ($user_id IS NONE OR user_id = $user_id)
        ORDER BY ingestion_time DESC
        LIMIT $limit
    "#;
    let mut response = db
        .query(sql)
        .bind(("user_id", user_id.map(String::from)))
        .bind(("limit", limit))
        .await?;
    let memories: Vec<Memory> = response.take(0)?;
    Ok(memories)
}

pub(super) async fn get_valid_at(
    db: &Surreal<Db>,
    timestamp: Datetime,
    user_id: Option<&str>,
    limit: usize,
) -> Result<Vec<Memory>> {
    let sql = r#"
        SELECT * FROM memories 
        WHERE valid_from <= $timestamp 
          AND (valid_until IS NONE OR valid_until > $timestamp)
          AND ($user_id IS NONE OR user_id = $user_id)
        ORDER BY ingestion_time DESC
        LIMIT $limit
    "#;
    let mut response = db
        .query(sql)
        .bind(("timestamp", timestamp))
        .bind(("user_id", user_id.map(String::from)))
        .bind(("limit", limit))
        .await?;
    let memories: Vec<Memory> = response.take(0)?;
    Ok(memories)
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
