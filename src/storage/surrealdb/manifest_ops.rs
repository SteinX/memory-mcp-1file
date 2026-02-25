//! File manifest operations for SurrealDB.
//!
//! Tracks all files belonging to a project so that the indexer can detect
//! files that have been deleted between indexing runs.

use surrealdb::engine::local::Db;
use surrealdb::Surreal;

use crate::types::ManifestEntry;
use crate::Result;

// ─────────────────────────────────────────────────────────────────────────────
// Upsert
// ─────────────────────────────────────────────────────────────────────────────

/// Insert or update a single manifest entry (mark file as seen now).
pub(super) async fn upsert_manifest_entry(
    db: &Surreal<Db>,
    project_id: &str,
    file_path: &str,
) -> Result<()> {
    let sql = "
        UPSERT file_manifest
            SET project_id   = $project_id,
                file_path    = $file_path,
                last_seen_at = time::now()
            WHERE project_id = $project_id AND file_path = $file_path;
    ";
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("file_path", file_path.to_string()))
        .await?;
    Ok(())
}

/// Batch upsert multiple file paths (one query per file — SurrealDB 3.x UPSERT
/// does not support multi-row VALUES syntax, so we chain statements).
pub(super) async fn upsert_manifest_entries(
    db: &Surreal<Db>,
    project_id: &str,
    file_paths: &[String],
) -> Result<()> {
    if file_paths.is_empty() {
        return Ok(());
    }

    // Build a single multi-statement query for efficiency.
    // Each UPSERT is separated by a semicolon.
    let stmt = "UPSERT file_manifest \
                    SET project_id = $project_id, file_path = $file_path, last_seen_at = time::now() \
                    WHERE project_id = $project_id AND file_path = $file_path;";

    // For large batches, chunk to avoid hitting query-size limits.
    const CHUNK: usize = 500;
    for chunk in file_paths.chunks(CHUNK) {
        let mut query_str = String::with_capacity(stmt.len() * chunk.len());
        for _ in chunk {
            query_str.push_str(stmt);
        }

        let mut q = db.query(&query_str);
        for fp in chunk {
            q = q
                .bind(("project_id", project_id.to_string()))
                .bind(("file_path", fp.clone()));
        }
        q.await?;
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Read
// ─────────────────────────────────────────────────────────────────────────────

/// Return all manifest entries for a project.
pub(super) async fn get_manifest_entries(
    db: &Surreal<Db>,
    project_id: &str,
) -> Result<Vec<ManifestEntry>> {
    let sql = "SELECT project_id, file_path, last_seen_at \
               FROM file_manifest WHERE project_id = $project_id";

    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;

    let entries: Vec<ManifestEntry> = response.take(0).unwrap_or_default();
    Ok(entries)
}

// ─────────────────────────────────────────────────────────────────────────────
// Delete
// ─────────────────────────────────────────────────────────────────────────────

/// Remove all manifest entries for a project (full re-index).
pub(super) async fn delete_manifest_entries(db: &Surreal<Db>, project_id: &str) -> Result<()> {
    let sql = "DELETE file_manifest WHERE project_id = $project_id";
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    Ok(())
}

/// Remove a single manifest entry (file deleted from project).
pub(super) async fn delete_manifest_entry(
    db: &Surreal<Db>,
    project_id: &str,
    file_path: &str,
) -> Result<()> {
    let sql = "DELETE file_manifest WHERE project_id = $project_id AND file_path = $file_path";
    db.query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("file_path", file_path.to_string()))
        .await?;
    Ok(())
}
