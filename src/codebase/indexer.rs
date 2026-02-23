use std::path::Path;
use std::sync::Arc;

use tokio::fs;

use crate::config::AppState;
use crate::storage::StorageBackend;
use crate::types::{IndexState, IndexStatus};
use crate::Result;

use super::chunker::chunk_file;
use super::parser::CodeParser;
use super::relations::{create_symbol_relations, RelationStats};
use super::scanner::scan_directory;
use super::symbol_index::SymbolIndex;

use crate::embedding::{EmbeddingRequest, EmbeddingTarget};
use crate::types::symbol::CodeReference;

pub async fn index_project(state: Arc<AppState>, project_path: &Path) -> Result<IndexStatus> {
    let project_id = project_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let state_clone = state.clone();
    let project_path_clone = project_path.to_path_buf();
    let project_id_clone = project_id.clone();

    // Spawn as a task so we can catch panics natively
    let handle = tokio::spawn(async move {
        do_index_project(state_clone, &project_path_clone, &project_id_clone).await
    });

    let result = match handle.await {
        Ok(res) => res,
        Err(join_err) => {
            let msg = if join_err.is_panic() {
                "Indexing panicked (crashed)".to_string()
            } else if join_err.is_cancelled() {
                "Indexing was cancelled".to_string()
            } else {
                "Indexing failed to complete".to_string()
            };
            Err(crate::AppError::Internal(msg))
        }
    };

    match result {
        Ok(status) => Ok(status),
        Err(e) => {
            tracing::error!(project_id = %project_id, error = %e, "Indexing failed");
            let mut status = IndexStatus::new(project_id.clone());
            if let Ok(Some(existing)) = state.storage.get_index_status(&project_id).await {
                status = existing;
            }

            // Extract last known file if possible
            if let Some(monitor) = state.progress.get(&project_id).await {
                if let Ok(cf) = monitor.current_file.read() {
                    if !cf.is_empty() {
                        status.failed_files.push(cf.clone());
                        status.error_message = Some(format!("{}: Failed at file {}", e, cf));
                    } else {
                        status.error_message = Some(e.to_string());
                    }
                } else {
                    status.error_message = Some(e.to_string());
                }
            } else {
                status.error_message = Some(e.to_string());
            }

            status.status = IndexState::Failed;
            status.completed_at = Some(crate::types::Datetime::default());
            let _ = state.storage.update_index_status(status.clone()).await;
            Err(e)
        }
    }
}

async fn do_index_project(
    state: Arc<AppState>,
    project_path: &Path,
    project_id: &str,
) -> Result<IndexStatus> {
    let mut status = IndexStatus::new(project_id.to_string());
    let monitor = state.progress.get_or_create(project_id).await;

    state.storage.delete_project_chunks(project_id).await?;
    state.storage.delete_project_symbols(project_id).await?;
    state.storage.delete_file_hashes(project_id).await?;

    let files = scan_directory(project_path)?;
    status.total_files = files.len() as u32;
    tracing::info!(
        project = %project_id,
        total_files = status.total_files,
        "Indexing started"
    );
    monitor
        .total_files
        .store(status.total_files, std::sync::atomic::Ordering::Relaxed);
    monitor
        .indexed_files
        .store(0, std::sync::atomic::Ordering::Relaxed);

    state.storage.update_index_status(status.clone()).await?;

    let batch_size = 20;
    let mut chunk_buffer = Vec::with_capacity(batch_size);
    let mut symbol_buffer = Vec::with_capacity(batch_size);
    let mut symbol_index = SymbolIndex::new();
    let mut relation_buffer: Vec<CodeReference> = Vec::new();
    let mut total_relation_stats = RelationStats::default();

    const MAX_CHUNKS_PER_FILE: usize = 50;

    for file_path in &files {
        // Update current file in monitor for status reporting
        if let Ok(mut cf) = monitor.current_file.write() {
            *cf = file_path.to_string_lossy().to_string();
        }

        tracing::info!("Indexing file: {:?}", file_path);

        // Skip auto-generated files (no useful semantic content)
        if crate::codebase::scanner::is_ignored_file(file_path) {
            tracing::debug!(path = ?file_path, "Skipping generated file");
            status.indexed_files += 1;
            continue;
        }

        // Warn on large files but still process them (with chunk cap)
        if let Ok(meta) = fs::metadata(file_path).await {
            if meta.len() > 1_048_576 {
                tracing::warn!(
                    path = ?file_path,
                    size_kb = meta.len() / 1024,
                    "Large file detected (>1MB), will cap at {} chunks",
                    MAX_CHUNKS_PER_FILE
                );
            }
        }

        let content = match fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to read file {:?}: {}", file_path, e);
                status
                    .failed_files
                    .push(file_path.to_string_lossy().to_string());
                continue;
            }
        };

        // Skip massive files to prevent OOM/TreeSitter crashes (e.g. giant bundled JS or Dart files)
        if content.len() > 1_000_000 {
            // > 1MB
            tracing::warn!("Skipping large file (>1MB): {:?}", file_path);
            status
                .failed_files
                .push(file_path.to_string_lossy().to_string());
            continue;
        }

        // Store file-level hash for incremental indexing
        let file_path_str = file_path.to_string_lossy().to_string();
        let file_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let _ = state
            .storage
            .set_file_hash(project_id, &file_path_str, &file_hash)
            .await;

        // 1. Chunking (Vector Search) — cap chunks per file to bound memory
        let mut chunks = chunk_file(file_path, &content, project_id);
        if chunks.len() > MAX_CHUNKS_PER_FILE {
            tracing::info!(
                path = ?file_path,
                total = chunks.len(),
                kept = MAX_CHUNKS_PER_FILE,
                "Capping chunks for large file"
            );
            chunks.truncate(MAX_CHUNKS_PER_FILE);
        }
        for chunk in chunks {
            chunk_buffer.push(chunk);
            status.total_chunks += 1;

            if chunk_buffer.len() >= batch_size {
                let batch = std::mem::take(&mut chunk_buffer);
                let _permit = state.db_semaphore.acquire().await;
                if let Ok(results) = state.storage.create_code_chunks_batch(batch).await {
                    for (id, chunk) in results {
                        let _ = state
                            .embedding_queue
                            .send(EmbeddingRequest {
                                text: chunk.content,
                                responder: None,
                                target: Some(EmbeddingTarget::Chunk(id)),
                                retry_count: 0,
                            })
                            .await;
                    }
                }
            }
        }

        // 2. Parsing (Code Graph)
        let (symbols, references) = CodeParser::parse_file(file_path, &content, project_id);

        if !symbols.is_empty() {
            tracing::debug!("File {:?}: found {} symbols", file_path, symbols.len());
        }

        // Add symbols to in-memory index FIRST (for relation resolution)
        for symbol in &symbols {
            symbol_index.add(symbol);
        }

        for symbol in symbols {
            symbol_buffer.push(symbol);
            status.total_symbols += 1;

            if symbol_buffer.len() >= batch_size {
                let batch = std::mem::take(&mut symbol_buffer);
                let _permit = state.db_semaphore.acquire().await;
                // 1. Insert batch to get IDs
                match state.storage.create_code_symbols_batch(batch.clone()).await {
                    Ok(ids) => {
                        // 2. Queue for async embedding
                        for (id, sym) in ids.iter().zip(batch.iter()) {
                            if let Some(sig) = &sym.signature {
                                let _ = state
                                    .embedding_queue
                                    .send(EmbeddingRequest {
                                        text: sig.clone(),
                                        responder: None,
                                        target: Some(EmbeddingTarget::Symbol(id.clone())),
                                        retry_count: 0,
                                    })
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            count = batch.len(),
                            error = %e,
                            "Failed to store symbol batch"
                        );
                    }
                }

                // Relations are deferred to final flush after ALL symbols are indexed
                // (removing mid-loop flush fixes cross-file forward reference loss)
            }
        }

        // Buffer references for deferred processing (after symbols are in DB)
        relation_buffer.extend(references);

        status.indexed_files += 1;
        monitor
            .indexed_files
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        if status.indexed_files.is_multiple_of(10) {
            let percent = (status.indexed_files as f32 / status.total_files as f32 * 100.0) as u32;
            tracing::info!(
                indexed = status.indexed_files,
                total = status.total_files,
                percent,
                chunks = status.total_chunks,
                symbols = status.total_symbols,
                failed = status.failed_files.len(),
                "Indexing progress"
            );
            if let Err(e) = state.storage.update_index_status(status.clone()).await {
                tracing::warn!("Failed to update intermediate status: {}", e);
            }
        }
    }

    if !chunk_buffer.is_empty() {
        let _permit = state.db_semaphore.acquire().await;
        if let Ok(results) = state.storage.create_code_chunks_batch(chunk_buffer).await {
            for (id, chunk) in results {
                let _ = state
                    .embedding_queue
                    .send(EmbeddingRequest {
                        text: chunk.content,
                        responder: None,
                        target: Some(EmbeddingTarget::Chunk(id)),
                        retry_count: 0,
                    })
                    .await;
            }
        }
    }

    if !symbol_buffer.is_empty() {
        let batch = symbol_buffer;
        let _permit = state.db_semaphore.acquire().await;
        let ids = state
            .storage
            .create_code_symbols_batch(batch.clone())
            .await?;

        for (id, sym) in ids.iter().zip(batch.iter()) {
            if let Some(sig) = &sym.signature {
                let _ = state
                    .embedding_queue
                    .send(EmbeddingRequest {
                        text: sig.clone(),
                        responder: None,
                        target: Some(EmbeddingTarget::Symbol(id.clone())),
                        retry_count: 0,
                    })
                    .await;
            }
        }
    }

    // Final flush of remaining relations
    if !relation_buffer.is_empty() {
        let stats = create_symbol_relations(
            state.storage.as_ref(),
            project_id,
            &relation_buffer,
            &symbol_index,
        )
        .await;
        total_relation_stats.created += stats.created;
        total_relation_stats.failed += stats.failed;
        total_relation_stats.unresolved += stats.unresolved;
    }

    // Log relation stats
    if total_relation_stats.created > 0 || total_relation_stats.failed > 0 {
        tracing::info!(
            created = total_relation_stats.created,
            failed = total_relation_stats.failed,
            unresolved = total_relation_stats.unresolved,
            "Symbol relations indexed"
        );
    }

    status.status = IndexState::EmbeddingPending;
    status.completed_at = Some(crate::types::Datetime::default());

    state.storage.update_index_status(status.clone()).await?;

    Ok(status)
}

/// Incremental re-index for changed files only
pub async fn incremental_index(
    state: Arc<AppState>,
    project_id: &str,
    changed_paths: Vec<std::path::PathBuf>,
) -> Result<usize> {
    let mut updated = 0;

    for path in changed_paths {
        let path_str = path.to_string_lossy().to_string();

        if !path.exists() {
            match state
                .storage
                .delete_chunks_by_path(project_id, &path_str)
                .await
            {
                Ok(deleted) => {
                    if deleted > 0 {
                        tracing::debug!(path = %path_str, deleted, "Removed chunks for deleted file");
                        updated += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(path = %path_str, error = %e, "Failed to delete chunks");
                }
            }
            // Also delete symbols and file hash
            let _ = state
                .storage
                .delete_symbols_by_path(project_id, &path_str)
                .await;
            let _ = state.storage.delete_file_hash(project_id, &path_str).await;
            continue;
        }

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path_str, error = %e, "Failed to read file");
                continue;
            }
        };

        let new_hash = blake3::hash(content.as_bytes()).to_hex().to_string();

        // Compare file-level hash from dedicated file_hashes table
        if let Ok(Some(existing_hash)) = state.storage.get_file_hash(project_id, &path_str).await {
            if existing_hash == new_hash {
                continue; // File unchanged, skip re-indexing
            }
        }

        let _ = state
            .storage
            .delete_chunks_by_path(project_id, &path_str)
            .await;
        let _ = state
            .storage
            .delete_symbols_by_path(project_id, &path_str)
            .await;

        // 1. Chunks - async via queue (consistent with index_project)
        let chunks = super::chunker::chunk_file(&path, &content, project_id);

        let _permit = state.db_semaphore.acquire().await;
        if let Ok(results) = state.storage.create_code_chunks_batch(chunks).await {
            for (id, chunk) in results {
                let _ = state
                    .embedding_queue
                    .send(EmbeddingRequest {
                        text: chunk.content,
                        responder: None,
                        target: Some(EmbeddingTarget::Chunk(id)),
                        retry_count: 0,
                    })
                    .await;
            }
        }

        // 2. Symbols
        let (symbols, references) = CodeParser::parse_file(&path, &content, project_id);
        if !symbols.is_empty() {
            let _permit = state.db_semaphore.acquire().await;
            let created_ids = match state
                .storage
                .create_code_symbols_batch(symbols.clone())
                .await
            {
                Ok(ids) => ids,
                Err(e) => {
                    tracing::warn!(path = %path_str, error = %e, "Failed to create symbols");
                    vec![]
                }
            };

            for (id, sym) in created_ids.iter().zip(symbols.iter()) {
                if let Some(sig) = &sym.signature {
                    let _ = state
                        .embedding_queue
                        .send(EmbeddingRequest {
                            text: sig.clone(),
                            responder: None,
                            target: Some(EmbeddingTarget::Symbol(id.clone())),
                            retry_count: 0,
                        })
                        .await;
                }
            }
        }

        // Create relations using project-wide symbol index for cross-file resolution
        if !references.is_empty() {
            let mut symbol_index = SymbolIndex::new();
            // Load ALL project symbols from DB for cross-file resolution
            if let Ok(all_symbols) = state.storage.get_project_symbols(project_id).await {
                symbol_index.add_batch(&all_symbols);
            }
            // Also add current file's new symbols (may not be in DB yet)
            symbol_index.add_batch(&symbols);
            let _stats = create_symbol_relations(
                state.storage.as_ref(),
                project_id,
                &references,
                &symbol_index,
            )
            .await;
        }

        // Store updated file hash
        let _ = state
            .storage
            .set_file_hash(project_id, &path_str, &new_hash)
            .await;
        updated += 1;
    }

    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;
    use std::fs;

    #[tokio::test]
    async fn test_indexer_batching() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("test_project");
        fs::create_dir_all(&project_dir).unwrap();

        for i in 0..150 {
            let file_path = project_dir.join(format!("file_{}.rs", i));
            fs::write(file_path, format!("fn test_{}() {{}}", i)).unwrap();
        }

        // Must run with a real queue/worker setup or mock state
        // For unit test, we can just use the ctx.state which has a dummy queue if we updated TestContext
        // But TestContext::new() needs to be updated to initialize embedding_queue.

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();

        assert_eq!(status.total_files, 150);
        assert_eq!(status.total_chunks, 150);

        let chunks = ctx
            .state
            .storage
            .bm25_search_code("fn test", None, 200)
            .await
            .unwrap();
        assert_eq!(chunks.len(), 150);
    }
}
