use surrealdb::engine::local::Db;
use surrealdb::Surreal;

use std::collections::BTreeSet;

use crate::types::{
    CodeChunk, IndexFileCheckpoint, IndexJobRecord, IndexStatus, RecordId, ScoredCodeChunk,
};
use crate::Result;

use super::helpers::generate_id;

const ACTIVE_GENERATION_FILTER: &str =
    "($active_generation IS NONE OR generation = $active_generation OR generation IS NONE)";

pub(super) async fn create_code_chunk(db: &Surreal<Db>, mut chunk: CodeChunk) -> Result<String> {
    let id = generate_id();
    chunk.id = Some(crate::types::RecordId::new("code_chunks", id.as_str()));
    let _: Option<CodeChunk> = db
        .create(("code_chunks", id.as_str()))
        .content(chunk)
        .await?;
    Ok(id)
}

pub(super) async fn create_code_chunks_batch(
    db: &Surreal<Db>,
    mut chunks: Vec<CodeChunk>,
) -> Result<Vec<(String, CodeChunk)>> {
    let count = chunks.len();
    if count == 0 {
        return Ok(vec![]);
    }

    for chunk in &mut chunks {
        if chunk.id.is_none() {
            let id = generate_id();
            chunk.id = Some(crate::types::RecordId::new("code_chunks", id.as_str()));
        }
    }

    let created: Vec<CodeChunk> = db.insert("code_chunks").content(chunks).await?;

    let pairs = created
        .into_iter()
        .filter_map(|c| {
            c.id.as_ref().map(|t| {
                (
                    format!(
                        "{}:{}",
                        t.table.as_str(),
                        crate::types::record_key_to_string(&t.key)
                    ),
                    c.clone(),
                )
            })
        })
        .collect();

    Ok(pairs)
}

pub(super) async fn delete_project_chunks(db: &Surreal<Db>, project_id: &str) -> Result<usize> {
    let count = count_chunks(db, project_id, None).await? as usize;
    let sql = "DELETE FROM code_chunks WHERE project_id = $project_id";
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    Ok(count)
}

pub(super) async fn delete_chunks_by_path(
    db: &Surreal<Db>,
    project_id: &str,
    file_path: &str,
) -> Result<usize> {
    let sql = "DELETE FROM code_chunks WHERE project_id = $project_id AND file_path = $file_path RETURN BEFORE";
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("file_path", file_path.to_string()))
        .await?;
    let deleted: Vec<CodeChunk> = response.take(0).unwrap_or_default();
    Ok(deleted.len())
}

pub(super) async fn get_chunks_by_path(
    db: &Surreal<Db>,
    project_id: &str,
    file_path: &str,
    active_generation: Option<u64>,
) -> Result<Vec<CodeChunk>> {
    let sql = format!(
        "SELECT * FROM code_chunks WHERE project_id = $project_id AND file_path = $file_path AND {ACTIVE_GENERATION_FILTER}"
    );
    let mut response = db
        .query(&sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("file_path", file_path.to_string()))
        .bind(("active_generation", active_generation.map(|g| g as i64)))
        .await?;
    let chunks: Vec<CodeChunk> = response.take(0).unwrap_or_default();
    Ok(chunks)
}

/// Returns all chunks for a project without embedding data.
/// Kept for backward compat with callers that still need the full set.
#[allow(dead_code)]
pub(super) async fn get_all_chunks_for_project(
    db: &Surreal<Db>,
    project_id: &str,
    active_generation: Option<u64>,
) -> Result<Vec<CodeChunk>> {
    // OMIT embedding: the 768-dim Vec<f32> (~3KB/chunk) is never used by callers
    // (BM25 warm-up discards it immediately). CodeChunk.embedding is Option<Vec<f32>>
    // so serde defaults it to None when the field is absent.
    let sql = format!(
        "SELECT * OMIT embedding FROM code_chunks WHERE project_id = $project_id AND {ACTIVE_GENERATION_FILTER}"
    );
    let mut response = db
        .query(&sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("active_generation", active_generation.map(|g| g as i64)))
        .await?;
    let chunks: Vec<CodeChunk> = response.take(0).unwrap_or_default();
    Ok(chunks)
}

/// Paginated variant of `get_all_chunks_for_project` to avoid materialising
/// the entire chunk set into a single `Vec`.  Callers iterate pages until an
/// empty page is returned.
///
/// `limit`  – page size (number of rows per page)
/// `offset` – zero-based row offset (i.e. `page * limit`)
pub(super) async fn get_chunks_paginated(
    db: &Surreal<Db>,
    project_id: &str,
    active_generation: Option<u64>,
    limit: usize,
    offset: usize,
) -> Result<Vec<CodeChunk>> {
    let sql = format!(
        "SELECT * OMIT embedding FROM code_chunks \
               WHERE project_id = $project_id AND {ACTIVE_GENERATION_FILTER} \
               LIMIT $limit START $offset"
    );
    let mut response = db
        .query(&sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("active_generation", active_generation.map(|g| g as i64)))
        .bind(("limit", limit))
        .bind(("offset", offset))
        .await?;
    let chunks: Vec<CodeChunk> = response.take(0).unwrap_or_default();
    Ok(chunks)
}

pub(super) async fn get_chunks_by_ids(
    db: &Surreal<Db>,
    ids: &[String],
    active_generation: Option<u64>,
) -> Result<Vec<CodeChunk>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    // Build a SELECT using record IDs: SELECT * FROM code_chunks:id1, code_chunks:id2, ...
    // SurrealDB supports fetching specific records by Thing notation.
    let record_list: Vec<String> = ids.iter().map(|id| format!("code_chunks:{}", id)).collect();
    let sql = format!(
        "SELECT * FROM {} WHERE {ACTIVE_GENERATION_FILTER}",
        record_list.join(", ")
    );
    let mut response = db
        .query(&sql)
        .bind(("active_generation", active_generation.map(|g| g as i64)))
        .await?;
    let chunks: Vec<CodeChunk> = response.take(0).unwrap_or_default();
    Ok(chunks)
}

pub(super) async fn get_index_status(
    db: &Surreal<Db>,
    project_id: &str,
) -> Result<Option<IndexStatus>> {
    let sql = "SELECT * FROM index_status WHERE project_id = $project_id LIMIT 1";
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    let result: Vec<IndexStatus> = response.take(0).unwrap_or_default();
    Ok(result.into_iter().next())
}

pub(super) async fn update_index_status(db: &Surreal<Db>, status: IndexStatus) -> Result<()> {
    let mut status = status;
    status.refresh_lifecycle_states();

    let record_id = status.project_id.clone();
    status.id = Some(RecordId::new("index_status", record_id.as_str()));
    let _: Option<IndexStatus> = db
        .upsert(("index_status", record_id.as_str()))
        .content(status)
        .await?;

    Ok(())
}

pub(super) async fn delete_index_status(db: &Surreal<Db>, project_id: &str) -> Result<()> {
    let sql = "DELETE FROM index_status WHERE project_id = $project_id";
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    Ok(())
}

pub(super) async fn list_projects(db: &Surreal<Db>) -> Result<Vec<String>> {
    let sql = r#"
        SELECT project_id FROM index_status;
        SELECT project_id FROM code_chunks WHERE project_id IS NOT NONE;
        SELECT project_id FROM code_symbols;
        SELECT project_id FROM file_manifest;
    "#;

    let mut response = db.query(sql).await?;
    let mut projects = BTreeSet::new();

    for result_index in 0..4 {
        let results: Vec<serde_json::Value> = response.take(result_index).unwrap_or_default();
        for value in results {
            if let Some(project_id) = value.get("project_id").and_then(|p| p.as_str()) {
                projects.insert(project_id.to_string());
            }
        }
    }

    Ok(projects.into_iter().collect())
}

fn index_job_record_id(project_id: &str, job_id: &str) -> String {
    format!("{}::{}", project_id, job_id)
}

fn index_file_checkpoint_record_id(
    project_id: &str,
    generation: u64,
    relative_file_path: &str,
) -> String {
    let path_hash = blake3::hash(relative_file_path.as_bytes()).to_hex();
    format!("{}::{}::{}", project_id, generation, path_hash)
}

pub(super) async fn create_or_update_index_job(
    db: &Surreal<Db>,
    job: &IndexJobRecord,
) -> Result<()> {
    let mut job = job.clone();
    if job.target_generation == 0 {
        job.target_generation = job.structural_generation;
    }
    if job.resume_token.is_empty() {
        if let Some(resume) = &job.resume {
            job.resume_token = resume.token.clone().unwrap_or_default();
        }
    }
    if job.completed_files_count == 0 {
        job.completed_files_count = u64::from(job.progress.indexed_files.unwrap_or(0));
    }
    if job.total_files_count.is_none() {
        job.total_files_count = job.progress.total_files.map(u64::from);
    }
    if job.reason_code.is_none() {
        job.reason_code = job.error.as_ref().map(|error| error.code.clone());
    }
    let record_id = index_job_record_id(&job.project_id, &job.job_id);
    job.id = Some(crate::types::RecordId::new(
        "index_jobs",
        record_id.as_str(),
    ));
    let _: Option<IndexJobRecord> = db
        .upsert(("index_jobs", record_id.as_str()))
        .content(job)
        .await?;
    Ok(())
}

pub(super) async fn create_index_job(db: &Surreal<Db>, job: IndexJobRecord) -> Result<()> {
    create_or_update_index_job(db, &job).await
}

pub(super) async fn update_index_job(db: &Surreal<Db>, job: IndexJobRecord) -> Result<()> {
    create_index_job(db, job).await
}

pub(super) async fn get_index_job(
    db: &Surreal<Db>,
    project_id: &str,
    job_id: &str,
) -> Result<Option<IndexJobRecord>> {
    let record_id = index_job_record_id(project_id, job_id);
    let job: Option<IndexJobRecord> = db.select(("index_jobs", record_id.as_str())).await?;
    Ok(job)
}

pub(super) async fn list_index_jobs_for_project(
    db: &Surreal<Db>,
    project_id: &str,
) -> Result<Vec<IndexJobRecord>> {
    let sql = "SELECT * FROM index_jobs WHERE project_id = $project_id ORDER BY updated_at DESC";
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    let jobs: Vec<IndexJobRecord> = response.take(0).unwrap_or_default();
    Ok(jobs)
}

pub(super) async fn delete_index_job(
    db: &Surreal<Db>,
    project_id: &str,
    job_id: &str,
) -> Result<()> {
    let record_id = index_job_record_id(project_id, job_id);
    let _: Option<IndexJobRecord> = db.delete(("index_jobs", record_id.as_str())).await?;
    Ok(())
}

pub(super) async fn upsert_file_checkpoint(
    db: &Surreal<Db>,
    checkpoint: &IndexFileCheckpoint,
) -> Result<()> {
    let mut checkpoint = checkpoint.clone();
    if checkpoint.relative_file_path.is_empty() {
        checkpoint.relative_file_path = checkpoint.file_path.clone();
    }
    if checkpoint.file_path.is_empty() {
        checkpoint.file_path = checkpoint.relative_file_path.clone();
    }
    if checkpoint.checkpoint_generation == 0 {
        checkpoint.checkpoint_generation = checkpoint.generation;
    }
    let record_id = index_file_checkpoint_record_id(
        &checkpoint.project_id,
        checkpoint.generation,
        &checkpoint.relative_file_path,
    );
    checkpoint.id = Some(crate::types::RecordId::new(
        "index_file_checkpoints",
        record_id.as_str(),
    ));
    let _: Option<IndexFileCheckpoint> = db
        .upsert(("index_file_checkpoints", record_id.as_str()))
        .content(checkpoint)
        .await?;
    Ok(())
}

pub(super) async fn list_file_checkpoints_for_job(
    db: &Surreal<Db>,
    project_id: &str,
    generation: u64,
) -> Result<Vec<IndexFileCheckpoint>> {
    let sql = r#"
        SELECT * FROM index_file_checkpoints
        WHERE project_id = $project_id
          AND generation = $generation
        ORDER BY relative_file_path ASC
    "#;
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("generation", generation as i64))
        .await?;
    let checkpoints: Vec<IndexFileCheckpoint> = response.take(0).unwrap_or_default();
    Ok(checkpoints)
}

pub(super) async fn get_file_checkpoint(
    db: &Surreal<Db>,
    project_id: &str,
    generation: u64,
    relative_file_path: &str,
) -> Result<Option<IndexFileCheckpoint>> {
    let sql = r#"
        SELECT * FROM index_file_checkpoints
        WHERE project_id = $project_id
          AND generation = $generation
          AND relative_file_path = $relative_file_path
        LIMIT 1
    "#;
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("generation", generation as i64))
        .bind(("relative_file_path", relative_file_path.to_string()))
        .await?;
    let checkpoints: Vec<IndexFileCheckpoint> = response.take(0).unwrap_or_default();
    Ok(checkpoints.into_iter().next())
}

pub(super) async fn get_active_generation(
    db: &Surreal<Db>,
    project_id: &str,
) -> Result<Option<u64>> {
    let sql =
        "SELECT generation FROM index_active_generations WHERE project_id = $project_id LIMIT 1";
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    let result: Vec<serde_json::Value> = response.take(0).unwrap_or_default();
    Ok(result
        .first()
        .and_then(|v| v.get("generation"))
        .and_then(|g| {
            g.as_u64()
                .or_else(|| g.as_i64().and_then(|i| u64::try_from(i).ok()))
        }))
}

pub(super) async fn set_active_generation(
    db: &Surreal<Db>,
    project_id: &str,
    generation: u64,
) -> Result<()> {
    let sql = r#"
        UPSERT index_active_generations SET
            project_id = $project_id,
            generation = $generation,
            updated_at = time::now()
        WHERE project_id = $project_id
    "#;
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("generation", generation as i64))
        .await?;
    Ok(())
}

pub(super) async fn list_abandoned_generations(
    db: &Surreal<Db>,
    project_id: &str,
) -> Result<Vec<u64>> {
    let active = get_active_generation(db, project_id).await?.unwrap_or(0);
    let sql = r#"
        SELECT generation FROM code_chunks WHERE project_id = $project_id AND generation IS NOT NONE;
        SELECT generation FROM code_symbols WHERE project_id = $project_id AND generation IS NOT NONE;
        SELECT generation FROM index_file_checkpoints WHERE project_id = $project_id;
        SELECT target_generation AS generation FROM index_jobs WHERE project_id = $project_id;
    "#;
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    let mut generations = BTreeSet::new();
    for result_index in 0..4 {
        let rows: Vec<serde_json::Value> = response.take(result_index).unwrap_or_default();
        for row in rows {
            if let Some(generation) = row.get("generation").and_then(|g| {
                g.as_u64()
                    .or_else(|| g.as_i64().and_then(|i| u64::try_from(i).ok()))
            }) {
                if generation != active {
                    generations.insert(generation);
                }
            }
        }
    }
    Ok(generations.into_iter().collect())
}

pub(super) async fn delete_project_generation(
    db: &Surreal<Db>,
    project_id: &str,
    generation: u64,
) -> Result<()> {
    let sql = r#"
        BEGIN TRANSACTION;
        DELETE symbol_relation
            WHERE project_id = $project_id
              AND freshness_generation = $generation;
        DELETE symbol_chunk_map
            WHERE project_id = $project_id
              AND freshness_generation = $generation;
        DELETE code_symbols
            WHERE project_id = $project_id
              AND generation = $generation;
        DELETE code_chunks
            WHERE project_id = $project_id
              AND generation = $generation;
        DELETE index_file_checkpoints
            WHERE project_id = $project_id
              AND generation = $generation;
        COMMIT TRANSACTION;
    "#;

    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("generation", generation as i64))
        .await?;
    Ok(())
}

pub(super) async fn get_file_hash(
    db: &Surreal<Db>,
    project_id: &str,
    file_path: &str,
) -> Result<Option<String>> {
    let sql = "SELECT content_hash FROM file_hashes WHERE project_id = $project_id AND file_path = $file_path LIMIT 1";
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("file_path", file_path.to_string()))
        .await?;
    let result: Vec<serde_json::Value> = response.take(0).unwrap_or_default();
    Ok(result.into_iter().next().and_then(|v| {
        v.get("content_hash")
            .and_then(|h| h.as_str())
            .map(String::from)
    }))
}

pub(super) async fn set_file_hash(
    db: &Surreal<Db>,
    project_id: &str,
    file_path: &str,
    hash: &str,
) -> Result<()> {
    let sql = r#"
        UPSERT file_hashes SET
            project_id = $project_id,
            file_path = $file_path,
            content_hash = $hash,
            indexed_at = time::now()
        WHERE project_id = $project_id AND file_path = $file_path
    "#;
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("file_path", file_path.to_string()))
        .bind(("hash", hash.to_string()))
        .await?;
    Ok(())
}

/// Batch UPSERT file hashes in a single DB round-trip.
/// `hashes` is a slice of `(file_path, content_hash)` pairs.
pub(super) async fn set_file_hashes_batch(
    db: &Surreal<Db>,
    project_id: &str,
    hashes: &[(String, String)],
) -> Result<()> {
    if hashes.is_empty() {
        return Ok(());
    }

    let sql = r#"
        FOR $h IN $hashes {
            UPSERT file_hashes SET
                project_id = $project_id,
                file_path = $h.file_path,
                content_hash = $h.content_hash,
                indexed_at = time::now()
            WHERE project_id = $project_id AND file_path = $h.file_path;
        };
    "#;

    let data: Vec<_> = hashes
        .iter()
        .map(|(path, hash)| {
            serde_json::json!({
                "file_path": path,
                "content_hash": hash,
            })
        })
        .collect();

    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("hashes", data))
        .await?;
    Ok(())
}

pub(super) async fn delete_file_hashes(db: &Surreal<Db>, project_id: &str) -> Result<()> {
    let sql = "DELETE FROM file_hashes WHERE project_id = $project_id";
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    Ok(())
}

pub(super) async fn delete_file_hash(
    db: &Surreal<Db>,
    project_id: &str,
    file_path: &str,
) -> Result<()> {
    let sql = "DELETE FROM file_hashes WHERE project_id = $project_id AND file_path = $file_path";
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("file_path", file_path.to_string()))
        .await?;
    Ok(())
}

pub(super) async fn bm25_search_code(
    db: &Surreal<Db>,
    query: &str,
    project_id: Option<&str>,
    limit: usize,
) -> Result<Vec<ScoredCodeChunk>> {
    let active_generation = match project_id {
        Some(pid) => get_active_generation(db, pid).await?,
        None => None,
    };
    // SurrealDB v3.0.0: search::score() is broken (bug #6852/#6946).
    // CONTAINS provides correct filtering; scoring is done in Rust.
    // The project_id IS NONE pattern works: SurrealDB Rust SDK maps
    // Rust's Option::None to SurrealDB NONE (not NULL).
    let fetch_limit = (limit * 3).max(limit);
    let sql = r#"
        SELECT
            meta::id(id) AS id,
            file_path,
            content,
            language,
            start_line,
            end_line,
            chunk_type,
            name,
            context_path,
            1.0f AS score
        FROM code_chunks
        WHERE string::lowercase(content) CONTAINS string::lowercase($query)
          AND ($project_id IS NONE OR project_id = $project_id)
          AND ($active_generation IS NONE OR generation = $active_generation OR generation IS NONE)
        LIMIT $limit
    "#;
    let mut response = db
        .query(sql)
        .bind(("query", query.to_string()))
        .bind(("project_id", project_id.map(String::from)))
        .bind(("active_generation", active_generation.map(|g| g as i64)))
        .bind(("limit", fetch_limit))
        .await?;
    let mut results: Vec<ScoredCodeChunk> = response.take(0)?;

    // Compute relevance score in Rust: normalized term frequency.
    let query_lower = query.to_lowercase();
    for r in &mut results {
        let content_lower = r.content.to_lowercase();
        let count = content_lower.matches(query_lower.as_str()).count() as f32;
        let tf = count / (content_lower.len() as f32 + 1.0) * 1000.0;
        r.score = tf.clamp(0.01, 1.0);
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    Ok(results)
}

pub(super) async fn update_chunk_embedding(
    db: &Surreal<Db>,
    id: &str,
    embedding: Vec<f32>,
) -> Result<()> {
    let sql = "UPDATE code_chunks SET embedding = $embedding WHERE id = (type::record($id))";
    let _ = db
        .query(sql)
        .bind(("embedding", embedding))
        .bind(("id", id.to_string()))
        .await?;
    Ok(())
}

pub(super) async fn batch_update_chunk_embeddings(
    db: &Surreal<Db>,
    updates: &[(String, Vec<f32>)],
) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }

    let sql = r#"
        FOR $u IN $updates {
            UPDATE (type::record($u.id)) SET embedding = $u.embedding;
        };
    "#;

    let data: Vec<_> = updates
        .iter()
        .map(|(id, emb)| serde_json::json!({"id": id, "embedding": emb}))
        .collect();

    let response = db.query(sql).bind(("updates", data)).await?;
    if let Err(e) = response.check() {
        tracing::error!(
            count = updates.len(),
            first_id = %updates[0].0,
            error = %e,
            "batch_update_chunk_embeddings: query-level error"
        );
        return Err(e.into());
    }
    tracing::debug!(count = updates.len(), "batch_update_chunk_embeddings: OK");
    Ok(())
}

pub(super) async fn count_chunks(
    db: &Surreal<Db>,
    project_id: &str,
    active_generation: Option<u64>,
) -> Result<u32> {
    use surrealdb_types::SurrealValue;

    let sql = format!(
        "SELECT count() FROM code_chunks WHERE project_id = $project_id AND {ACTIVE_GENERATION_FILTER} GROUP ALL"
    );
    let mut response = db
        .query(&sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("active_generation", active_generation.map(|g| g as i64)))
        .await?;

    #[derive(serde::Deserialize, SurrealValue)]
    struct CountResult {
        count: u32,
    }

    let result: Option<CountResult> = response.take(0)?;
    Ok(result.map(|r| r.count).unwrap_or(0))
}

pub(super) async fn count_embedded_chunks(
    db: &Surreal<Db>,
    project_id: &str,
    active_generation: Option<u64>,
) -> Result<u32> {
    use surrealdb_types::SurrealValue;

    let sql = format!(
        "SELECT count() FROM code_chunks WHERE project_id = $project_id AND (embedding IS NOT NONE OR string::len(content) < 50) AND {ACTIVE_GENERATION_FILTER} GROUP ALL"
    );
    let mut response = db
        .query(&sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("active_generation", active_generation.map(|g| g as i64)))
        .await?;

    #[derive(serde::Deserialize, SurrealValue)]
    struct CountResult {
        count: u32,
    }

    let result: Option<CountResult> = response.take(0)?;
    Ok(result.map(|r| r.count).unwrap_or(0))
}

pub(super) async fn get_unembedded_chunks(
    db: &Surreal<Db>,
    project_id: &str,
) -> Result<Vec<(String, String)>> {
    use surrealdb_types::SurrealValue;

    let sql = "SELECT id, content FROM code_chunks WHERE project_id = $project_id AND embedding IS NONE AND string::len(content) >= 50";
    let response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    let mut response = response.check()?;

    #[derive(serde::Deserialize, SurrealValue)]
    struct Row {
        id: crate::types::RecordId,
        content: String,
    }

    let rows: Vec<Row> = response.take(0)?;
    let results = rows
        .into_iter()
        .map(|row| {
            let id_str = format!(
                "{}:{}",
                row.id.table,
                crate::types::record_key_to_string(&row.id.key)
            );
            (id_str, row.content)
        })
        .collect();
    Ok(results)
}

/// Clear all embeddings for a project (set to NONE), forcing re-embedding
/// via the existing resume pipeline (get_unembedded_chunks → EmbeddingWorker).
pub(super) async fn clear_project_embeddings(db: &Surreal<Db>, project_id: &str) -> Result<u64> {
    let sql = "
        UPDATE code_chunks SET embedding = NONE WHERE project_id = $project_id;
        UPDATE code_symbols SET embedding = NONE WHERE project_id = $project_id;
    ";
    let response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    let response = response.check()?;
    // Each UPDATE returns affected rows; sum both statements
    let chunks_cleared = response.num_statements();
    Ok(chunks_cleared as u64)
}
