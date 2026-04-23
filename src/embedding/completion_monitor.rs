use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::config::AppState;
use crate::storage::StorageBackend;
use crate::types::IndexState;

const POLL_INTERVAL_SECS: u64 = 10;

/// Runs the completion monitor loop until `shutdown_rx` receives `true`.
pub async fn run_completion_monitor(state: Arc<AppState>, mut shutdown_rx: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut progress_map: HashMap<String, (u32, u32, u8)> = HashMap::new();

    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    tracing::debug!("Completion monitor received shutdown signal");
                    return;
                }
            }
        }

        let projects = match state.storage.list_projects().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Completion monitor: failed to list projects: {}", e);
                continue;
            }
        };

        for project_id in projects {
            if let Err(e) = check_and_complete_project(&state, &project_id, &mut progress_map).await
            {
                tracing::debug!(
                    project_id = %project_id,
                    error = %e,
                    "Completion check failed"
                );
            }
        }
    }
}

async fn check_and_complete_project(
    state: &Arc<AppState>,
    project_id: &str,
    progress_map: &mut HashMap<String, (u32, u32, u8)>,
) -> crate::Result<()> {
    let status = match state.storage.get_index_status(project_id).await? {
        Some(s) => s,
        None => return Ok(()),
    };

    // Detect stale Indexing: if no file progress for 300s, mark Failed
    if status.status == IndexState::Indexing {
        let key = format!("idx:{}", project_id);
        let entry = progress_map
            .entry(key.clone())
            .or_insert((status.indexed_files, 0, 0));
        if entry.0 == status.indexed_files {
            entry.2 += 1;
            if entry.2 >= 180 {
                // 180 ticks × 10s = 1800s (30 min) with no progress
                // Qwen3 on CPU: ~20s/batch of 8, queue throttle can block indexer
                tracing::warn!(
                    project_id = %project_id,
                    indexed = status.indexed_files,
                    total = status.total_files,
                    stall_ticks = entry.2,
                    "Indexing stuck for 1800s, marking as failed"
                );
                progress_map.remove(&key);
                let mut updated_status = status.clone();
                updated_status.status = IndexState::Failed;
                updated_status.error_message = Some(format!(
                    "Indexing stalled at {}/{} files for >1800s",
                    status.indexed_files, status.total_files
                ));
                state.storage.update_index_status(updated_status).await?;
            }
        } else {
            entry.0 = status.indexed_files;
            entry.2 = 0;
        }
        return Ok(());
    }

    if status.status != IndexState::EmbeddingPending {
        progress_map.remove(project_id);
        return Ok(());
    }

    let total_chunks = state.storage.count_chunks(project_id).await?;
    let total_symbols = state.storage.count_symbols(project_id).await?;
    let embedded_chunks = state.storage.count_embedded_chunks(project_id).await?;
    let embedded_symbols = state.storage.count_embedded_symbols(project_id).await?;
    let failed = state.embedding_queue.metrics().get_failed_total() as u32;

    let chunks_complete = (embedded_chunks + failed) >= total_chunks;
    // Safety threshold: if ≥99.5% symbols embedded, consider complete
    // (handles edge case where some symbols lack signature text for embedding)
    let symbols_complete = (embedded_symbols + failed) >= total_symbols
        || (total_symbols > 0
            && embedded_symbols > 0
            && (embedded_symbols as f64 / total_symbols as f64) >= 0.995);
    let has_content = total_chunks > 0 || total_symbols > 0;

    let mut is_stuck = false;
    if !chunks_complete || !symbols_complete {
        let entry = progress_map.entry(project_id.to_string()).or_insert((
            embedded_chunks,
            embedded_symbols,
            0,
        ));
        if entry.0 == embedded_chunks && entry.1 == embedded_symbols {
            entry.2 += 1;
            // Don't force-complete if zero embeddings — engine likely still loading
            // Use 600s (60 ticks) timeout: Gemma on CPU can take 2-5 min per batch
            // Previous 60s timeout caused premature completion with only ~11% embedded
            if entry.2 >= 60 && (embedded_chunks > 0 || embedded_symbols > 0) {
                // 600 seconds stuck WITH some progress = genuinely stuck
                is_stuck = true;
                tracing::warn!(
                    project_id = %project_id,
                    embedded_chunks,
                    total_chunks,
                    embedded_symbols,
                    total_symbols,
                    "Embedding progress stuck for 600s with partial progress, forcing completion"
                );
            } else if entry.2 >= 60 && embedded_chunks == 0 && embedded_symbols == 0 {
                // 600 seconds with zero embeddings = engine not ready yet, reset and wait
                if entry.2.is_multiple_of(60) {
                    tracing::info!(
                        project_id = %project_id,
                        stuck_ticks = entry.2,
                        "Embedding engine not ready yet (0 embeddings), waiting..."
                    );
                }
            } else if entry.2 > 0 && entry.2.is_multiple_of(6) {
                // Log every 60s while waiting.
                // Cast to wider integer to avoid u8 overflow in stall_secs.
                let chunk_pct = if total_chunks > 0 {
                    (embedded_chunks as f64 / total_chunks as f64 * 100.0) as u32
                } else {
                    0
                };
                let symbol_pct = if total_symbols > 0 {
                    (embedded_symbols as f64 / total_symbols as f64 * 100.0) as u32
                } else {
                    0
                };
                tracing::info!(
                    project_id = %project_id,
                    embedded_chunks,
                    total_chunks,
                    chunk_pct,
                    embedded_symbols,
                    total_symbols,
                    symbol_pct,
                    stall_secs = u32::from(entry.2) * 10,
                    "Embedding in progress, waiting for next batch..."
                );
            }
        } else {
            entry.0 = embedded_chunks;
            entry.1 = embedded_symbols;
            entry.2 = 0;
        }
    }

    if (chunks_complete && symbols_complete && has_content) || is_stuck {
        progress_map.remove(project_id);

        let mut updated_status = status.clone();
        updated_status.status = IndexState::Completed;
        updated_status.mark_semantic_generation_caught_up();
        updated_status.total_chunks = total_chunks;
        updated_status.total_symbols = total_symbols;
        updated_status.failed_embeddings = failed;

        state.storage.update_index_status(updated_status).await?;

        // Rebuild HNSW vector indices so semantic search works immediately
        let dim = state.embedding.dimensions();
        if let Err(e) = state.storage.rebuild_vector_indices(dim).await {
            tracing::error!(
                project_id = %project_id,
                error = %e,
                "Failed to rebuild HNSW indices after completion"
            );
        } else {
            tracing::info!(
                project_id = %project_id,
                dim = dim,
                "HNSW vector indices rebuilt successfully"
            );
        }

        tracing::info!(
            project_id = %project_id,
            chunks = total_chunks,
            symbols = total_symbols,
            failed = failed,
            "Project indexing completed"
        );
    }

    Ok(())
}
