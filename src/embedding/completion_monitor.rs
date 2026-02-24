use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::config::AppState;
use crate::storage::StorageBackend;
use crate::types::IndexState;

const POLL_INTERVAL_SECS: u64 = 10;

pub async fn run_completion_monitor(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut progress_map: HashMap<String, (u32, u32, u8)> = HashMap::new();

    loop {
        interval.tick().await;

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
            if entry.2 >= 30 {
                // 30 ticks × 10s = 300s with no progress
                tracing::warn!(
                    project_id = %project_id,
                    indexed = status.indexed_files,
                    total = status.total_files,
                    "Indexing stuck for 300s, marking as failed"
                );
                progress_map.remove(&key);
                let mut updated_status = status.clone();
                updated_status.status = IndexState::Failed;
                updated_status.error_message = Some(format!(
                    "Indexing stalled at {}/{} files for >300s",
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

    let chunks_complete = embedded_chunks >= total_chunks;
    // Safety threshold: if ≥99.5% symbols embedded, consider complete
    // (handles edge case where some symbols lack signature text for embedding)
    let symbols_complete = embedded_symbols >= total_symbols
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
            if entry.2 >= 6 {
                // 60 seconds stuck
                is_stuck = true;
                tracing::warn!(project_id = %project_id, "Embedding progress stuck for 60s, forcing completion");
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
        updated_status.total_chunks = total_chunks;
        updated_status.total_symbols = total_symbols;

        state.storage.update_index_status(updated_status).await?;

        tracing::info!(
            project_id = %project_id,
            chunks = total_chunks,
            symbols = total_symbols,
            "Project indexing completed"
        );
    }

    Ok(())
}
