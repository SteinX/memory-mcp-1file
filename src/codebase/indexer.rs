use num_cpus;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use tokio::fs;

use crate::config::AppState;
use crate::storage::StorageBackend;
use crate::types::{derive_project_id, IndexState, IndexStatus};
use crate::Result;

use super::chunker::chunk_file;
use super::parser::CodeParser;
use super::relations::{create_symbol_relations, detect_containment_references, RelationStats};
use super::scanner::scan_directory;
use super::symbol_index::SymbolIndex;

use crate::embedding::{EmbeddingRequest, EmbeddingTarget};
use crate::types::code::CodeChunk;
use crate::types::symbol::{CodeReference, CodeSymbol};

const PARSE_TIMEOUT_SECS: u64 = 30;

fn emit_index_timing(
    project_id: &str,
    phase: &'static str,
    elapsed_ms: u128,
    file_count: u64,
    chunk_count: u64,
    symbol_count: u64,
    relation_count: u64,
    failed_count: u64,
) {
    tracing::info!(
        project_id = %project_id,
        phase,
        elapsed_ms,
        file_count,
        chunk_count,
        symbol_count,
        relation_count,
        failed_count,
        "Code indexing phase timing"
    );
}

pub async fn index_project(state: Arc<AppState>, project_path: &Path) -> Result<IndexStatus> {
    let project_id = derive_project_id(project_path)
        .map_err(|error| crate::AppError::InvalidPath(error.to_string()))?;

    let state_clone = state.clone();
    let project_path_clone = project_path.to_path_buf();
    let project_id_clone = project_id.clone();

    // Spawn as a task so we can catch panics natively
    let handle = tokio::spawn(async move {
        do_index_project(state_clone, &project_path_clone, &project_id_clone).await
    });

    tracing::info!(
        project_id = %project_id,
        root_path = %project_path.display(),
        "Awaiting one-shot code index task"
    );

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
            Err(crate::AppError::Internal(msg.into()))
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
    let total_started = Instant::now();
    let mut file_read_hash_elapsed_ms = 0u128;
    let mut parse_chunk_elapsed_ms = 0u128;
    let mut chunk_db_write_elapsed_ms = 0u128;
    let mut symbol_db_write_elapsed_ms = 0u128;
    let mut embedding_enqueue_elapsed_ms = 0u128;
    let mut relation_create_elapsed_ms = 0u128;
    let mut status_update_elapsed_ms = 0u128;
    let mut chunks_written = 0u64;
    let mut symbols_written = 0u64;
    let mut embeddings_enqueued = 0u64;
    let mut files_read = 0u64;

    tracing::info!(
        project_id = %project_id,
        root_path = %project_path.display(),
        "One-shot code index task started"
    );
    tracing::info!(
        project_id = %project_id,
        phase = "accepted",
        "One-shot code index task accepted"
    );
    let mut status = IndexStatus::new(project_id.to_string());
    status.root_path = Some(
        project_path
            .canonicalize()
            .unwrap_or_else(|_| project_path.to_path_buf())
            .to_string_lossy()
            .into_owned(),
    );
    let monitor = state.progress.get_or_create(project_id).await;
    monitor
        .total_files
        .store(0, std::sync::atomic::Ordering::Relaxed);
    monitor
        .indexed_files
        .store(0, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut current_file) = monitor.current_file.write() {
        current_file.clear();
    }

    // Persist the indexing state before destructive cleanup. If the server
    // restarts while rebuilding from scratch, status/stats can report
    // "indexing/unknown_after_restart" instead of "metadata missing".
    tracing::info!(
        project_id = %project_id,
        root_path = status.root_path.as_deref().unwrap_or_default(),
        "Persisting initial indexing metadata before cleanup"
    );
    tracing::info!(project_id = %project_id, phase = "task_spawned", "One-shot code index task spawned");
    let status_started = Instant::now();
    state.storage.update_index_status(status.clone()).await?;
    status_update_elapsed_ms += status_started.elapsed().as_millis();
    emit_index_timing(
        project_id,
        "status_update",
        status_started.elapsed().as_millis(),
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        0,
        status.failed_files.len() as u64,
    );

    tracing::info!(project_id = %project_id, "Clearing stale code intelligence rows");

    let cleanup_started = Instant::now();
    let phase_started = Instant::now();
    tracing::info!(project_id = %project_id, "Deleting stale code chunks");
    let deleted_chunks = state.storage.delete_project_chunks(project_id).await?;
    tracing::info!(
        project_id = %project_id,
        deleted = deleted_chunks,
        elapsed_ms = phase_started.elapsed().as_millis(),
        "Deleted stale code chunks"
    );

    let phase_started = Instant::now();
    tracing::info!(project_id = %project_id, "Deleting stale code symbols");
    let deleted_symbols = state.storage.delete_project_symbols(project_id).await?;
    tracing::info!(
        project_id = %project_id,
        deleted = deleted_symbols,
        elapsed_ms = phase_started.elapsed().as_millis(),
        "Deleted stale code symbols"
    );

    let phase_started = Instant::now();
    tracing::info!(project_id = %project_id, "Deleting stale file hashes");
    state.storage.delete_file_hashes(project_id).await?;
    tracing::info!(
        project_id = %project_id,
        elapsed_ms = phase_started.elapsed().as_millis(),
        "Deleted stale file hashes"
    );

    let phase_started = Instant::now();
    tracing::info!(project_id = %project_id, "Deleting stale manifest entries");
    state.storage.delete_manifest_entries(project_id).await?;
    tracing::info!(
        project_id = %project_id,
        elapsed_ms = phase_started.elapsed().as_millis(),
        total_elapsed_ms = cleanup_started.elapsed().as_millis(),
        "Deleted stale manifest entries"
    );
    emit_index_timing(
        project_id,
        "cleanup",
        cleanup_started.elapsed().as_millis(),
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        0,
        status.failed_files.len() as u64,
    );

    // `scan_directory` uses `ignore::WalkBuilder` — a synchronous blocking I/O walk.
    // Wrap in `spawn_blocking` to avoid starving the Tokio async thread pool.
    tracing::info!(
        project_id = %project_id,
        root_path = %project_path.display(),
        "Scanning project files"
    );
    tracing::info!(project_id = %project_id, phase = "file_enumeration_started", "Project file enumeration started");
    let project_path_for_scan = project_path.to_path_buf();
    let scan_started = Instant::now();
    let files = tokio::task::spawn_blocking(move || scan_directory(&project_path_for_scan))
        .await
        .map_err(|e| crate::AppError::Internal(format!("scan_directory panicked: {e}").into()))??;
    tracing::info!(project_id = %project_id, phase = "file_enumeration_completed", total_files = files.len(), "Project file enumeration completed");
    status.total_files = files.len() as u32;
    tracing::info!(
        project = %project_id,
        root_path = %project_path.display(),
        total_files = status.total_files,
        "Indexing started"
    );
    monitor
        .total_files
        .store(status.total_files, std::sync::atomic::Ordering::Relaxed);

    emit_index_timing(
        project_id,
        "scan",
        scan_started.elapsed().as_millis(),
        status.total_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        0,
        status.failed_files.len() as u64,
    );
    let status_started = Instant::now();
    state.storage.update_index_status(status.clone()).await?;
    status_update_elapsed_ms += status_started.elapsed().as_millis();
    emit_index_timing(
        project_id,
        "status_update",
        status_started.elapsed().as_millis(),
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        0,
        status.failed_files.len() as u64,
    );

    let batch_size = 100;
    let mut chunk_buffer = Vec::with_capacity(batch_size);
    let mut symbol_buffer = Vec::with_capacity(batch_size);
    let mut symbol_index = SymbolIndex::new();
    let mut relation_buffer: Vec<CodeReference> = Vec::new();
    let mut total_relation_stats = RelationStats::default();
    // Buffer file hashes for batched UPSERT (Bug 3 fix: avoids N sequential DB round-trips)
    let mut hash_buffer: Vec<(String, String)> = Vec::with_capacity(batch_size);
    const HASH_FLUSH_SIZE: usize = 50;

    tracing::info!(project_id = %project_id, phase = "parsing_chunking_started", total_files = status.total_files, "Parsing and chunking started");

    // Issue 4 fix: Parse files with bounded concurrency using JoinSet instead of
    // sequential spawn_blocking. Up to max_concurrent_parses files are parsed on
    // the blocking thread pool simultaneously.
    #[allow(clippy::type_complexity)]
    let max_concurrent_parses = std::cmp::max(4, num_cpus::get() / 2);
    #[allow(clippy::type_complexity)]
    let mut parse_set: tokio::task::JoinSet<(
        Vec<CodeChunk>,
        Vec<CodeSymbol>,
        Vec<CodeReference>,
        String,
        u128,
    )> = tokio::task::JoinSet::new();

    // Macro to process one completed parse result (used in drain-when-full and final drain).
    // Expands in place so it can mutate surrounding locals and use `.await`.
    macro_rules! drain_one_parse {
        ($join_result:expr) => {{
            let (chunks, symbols, references, fp_str, parse_elapsed_ms) = $join_result
                .map_err(|e| crate::AppError::Internal(
                    format!("parse/chunk panicked: {e}").into(),
                ))?;
            parse_chunk_elapsed_ms += parse_elapsed_ms;
            emit_index_timing(
                project_id,
                "parse_chunk",
                parse_elapsed_ms,
                1,
                chunks.len() as u64,
                symbols.len() as u64,
                references.len() as u64,
                status.failed_files.len() as u64,
            );

            for chunk in chunks {
                chunk_buffer.push(chunk);
                status.total_chunks += 1;

                if chunk_buffer.len() >= batch_size {
                    let batch = std::mem::take(&mut chunk_buffer);
                    let batch_len = batch.len() as u64;
                    let _permit = state.db_semaphore.acquire().await;
                    let chunk_write_started = Instant::now();
                    let results = state
                        .storage
                        .create_code_chunks_batch(batch)
                        .await
                        .map_err(|e| crate::AppError::Internal(
                            format!("failed to persist code chunks for {fp_str}: {e}").into(),
                        ))?;
                    let chunk_write_elapsed = chunk_write_started.elapsed().as_millis();
                    chunk_db_write_elapsed_ms += chunk_write_elapsed;
                    chunks_written += batch_len;
                    emit_index_timing(
                        project_id,
                        "chunk_db_write",
                        chunk_write_elapsed,
                        status.indexed_files as u64,
                        batch_len,
                        status.total_symbols as u64,
                        (total_relation_stats.created + total_relation_stats.failed) as u64,
                        status.failed_files.len() as u64,
                    );

                    for (id, chunk) in results {
                        let enqueue_started = Instant::now();
                        if let Err(e) = state
                            .embedding_queue
                            .send(EmbeddingRequest {
                                text: chunk.content,
                                responder: None,
                                target: Some(EmbeddingTarget::Chunk(id.clone())),
                                retry_count: 0,
                            })
                            .await
                        {
                            tracing::warn!(chunk_id = %id, error = %e, "Failed to enqueue chunk embedding");
                        }
                        embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
                        embeddings_enqueued += 1;
                    }
                }
            }

            if !symbols.is_empty() {
                tracing::debug!(file = %fp_str, count = symbols.len(), "Parsed symbols");
            }

            // Add symbols to in-memory index (for cross-file relation resolution at end)
            for symbol in &symbols {
                symbol_index.add(symbol);
            }

            // Detect containment (parent→child) relations from symbol line ranges
            // BEFORE consuming `symbols` into the buffer. This produces Contains
            // edges (e.g. impl→fn, class→method) that tree-sitter doesn't emit.
            let containment_refs = detect_containment_references(&symbols);

            for symbol in symbols {
                symbol_buffer.push(symbol);
                status.total_symbols += 1;

                if symbol_buffer.len() >= batch_size {
                    let batch = std::mem::take(&mut symbol_buffer);
                    let batch_len = batch.len() as u64;
                    let _permit = state.db_semaphore.acquire().await;
                    let symbol_write_started = Instant::now();
                    match state.storage.create_code_symbols_batch(batch.clone()).await {
                        Ok(ids) => {
                            let symbol_write_elapsed = symbol_write_started.elapsed().as_millis();
                            symbol_db_write_elapsed_ms += symbol_write_elapsed;
                            symbols_written += batch_len;
                            emit_index_timing(
                                project_id,
                                "symbol_db_write",
                                symbol_write_elapsed,
                                status.indexed_files as u64,
                                status.total_chunks as u64,
                                batch_len,
                                (total_relation_stats.created + total_relation_stats.failed) as u64,
                                status.failed_files.len() as u64,
                            );
                            for (id, sym) in ids.iter().zip(batch.iter()) {
                                let embed_text = sym
                                    .signature
                                    .clone()
                                    .unwrap_or_else(|| format!("{} {}", sym.symbol_type, sym.name));
                                let enqueue_started = Instant::now();
                                if let Err(e) = state
                                    .embedding_queue
                                    .send(EmbeddingRequest {
                                        text: embed_text,
                                        responder: None,
                                        target: Some(EmbeddingTarget::Symbol(id.clone())),
                                        retry_count: 0,
                                    })
                                    .await
                                {
                                    tracing::warn!(symbol_id = %id, error = %e, "Failed to enqueue symbol embedding");
                                }
                                embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
                                embeddings_enqueued += 1;
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
                }
            }

            // Buffer references for deferred processing (after ALL symbols are indexed)
            relation_buffer.extend(references);
            relation_buffer.extend(containment_refs);

            // Mid-flight flush: avoid unbounded growth of relation_buffer on
            // large codebases.  5000 relations ≈ ~2 MB (each CodeReference is
            // small), so flushing at this threshold keeps peak RSS low.
            if relation_buffer.len() >= 5000 {
                let batch = std::mem::take(&mut relation_buffer);
                let relation_started = Instant::now();
                let stats = create_symbol_relations(
                    state.storage.as_ref(),
                    project_id,
                    &batch,
                    &symbol_index,
                )
                .await;
                total_relation_stats.created += stats.created;
                total_relation_stats.failed += stats.failed;
                total_relation_stats.unresolved += stats.unresolved;
                relation_create_elapsed_ms += relation_started.elapsed().as_millis();
                emit_index_timing(
                    project_id,
                    "relation_create",
                    relation_started.elapsed().as_millis(),
                    status.indexed_files as u64,
                    status.total_chunks as u64,
                    status.total_symbols as u64,
                    (stats.created + stats.failed) as u64,
                    status.failed_files.len() as u64,
                );
            }

            status.indexed_files += 1;
            monitor
                .indexed_files
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            if status.indexed_files % 10 == 0 {
                let percent =
                    (status.indexed_files as f32 / status.total_files as f32 * 100.0) as u32;
                tracing::info!(
                    indexed = status.indexed_files,
                    total = status.total_files,
                    percent,
                    chunks = status.total_chunks,
                    symbols = status.total_symbols,
                    failed = status.failed_files.len(),
                    "Indexing progress"
                );
                let status_started = Instant::now();
                if let Err(e) = state.storage.update_index_status(status.clone()).await {
                    tracing::warn!("Failed to update intermediate status: {}", e);
                }
                status_update_elapsed_ms += status_started.elapsed().as_millis();
                emit_index_timing(
                    project_id,
                    "status_update",
                    status_started.elapsed().as_millis(),
                    status.indexed_files as u64,
                    status.total_chunks as u64,
                    status.total_symbols as u64,
                    (total_relation_stats.created + total_relation_stats.failed) as u64,
                    status.failed_files.len() as u64,
                );
            }
        }};
    }

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
                    "Large file detected (>1MB), might take a while to parse",

                );
            }
        }

        let read_hash_started = Instant::now();
        let content = match fs::read_to_string(file_path).await {
            Ok(c) => c,
            Err(e) => {
                let read_hash_elapsed = read_hash_started.elapsed().as_millis();
                file_read_hash_elapsed_ms += read_hash_elapsed;
                tracing::warn!("Failed to read file {:?}: {}", file_path, e);
                status
                    .failed_files
                    .push(file_path.to_string_lossy().to_string());
                emit_index_timing(
                    project_id,
                    "file_read_hash",
                    read_hash_elapsed,
                    files_read,
                    status.total_chunks as u64,
                    status.total_symbols as u64,
                    (total_relation_stats.created + total_relation_stats.failed) as u64,
                    status.failed_files.len() as u64,
                );
                continue;
            }
        };

        // Skip massive files to prevent OOM/TreeSitter crashes (e.g. giant bundled JS or Dart files)
        if content.len() > 1_000_000 {
            // > 1MB
            let read_hash_elapsed = read_hash_started.elapsed().as_millis();
            file_read_hash_elapsed_ms += read_hash_elapsed;
            tracing::warn!("Skipping large file (>1MB): {:?}", file_path);
            status
                .failed_files
                .push(file_path.to_string_lossy().to_string());
            emit_index_timing(
                project_id,
                "file_read_hash",
                read_hash_elapsed,
                files_read,
                status.total_chunks as u64,
                status.total_symbols as u64,
                (total_relation_stats.created + total_relation_stats.failed) as u64,
                status.failed_files.len() as u64,
            );
            continue;
        }

        // Compute file-level hash for incremental indexing and buffer for batch flush
        let file_path_str = file_path.to_string_lossy().to_string();
        let file_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        hash_buffer.push((file_path_str, file_hash));
        let read_hash_elapsed = read_hash_started.elapsed().as_millis();
        file_read_hash_elapsed_ms += read_hash_elapsed;
        files_read += 1;
        emit_index_timing(
            project_id,
            "file_read_hash",
            read_hash_elapsed,
            files_read,
            status.total_chunks as u64,
            status.total_symbols as u64,
            (total_relation_stats.created + total_relation_stats.failed) as u64,
            status.failed_files.len() as u64,
        );

        // Flush hash buffer periodically to avoid unbounded growth
        if hash_buffer.len() >= HASH_FLUSH_SIZE {
            let batch = std::mem::take(&mut hash_buffer);
            let _ = state
                .storage
                .set_file_hashes_batch(project_id, &batch)
                .await;
        }

        let mut skip_spawn = false;
        if parse_set.len() >= max_concurrent_parses {
            match tokio::time::timeout(
                std::time::Duration::from_secs(PARSE_TIMEOUT_SECS),
                parse_set.join_next(),
            )
            .await
            {
                Ok(Some(join_result)) => {
                    drain_one_parse!(join_result);
                }
                Ok(None) => {}
                Err(_timeout) => {
                    tracing::warn!(
                        timeout_secs = PARSE_TIMEOUT_SECS,
                        pending = parse_set.len(),
                        path = ?file_path,
                        "Parse task timed out, skipping file"
                    );
                    status
                        .failed_files
                        .push(file_path.to_string_lossy().to_string());
                    status.indexed_files += 1;
                    monitor
                        .indexed_files
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    skip_spawn = true;
                }
            }
        }

        if skip_spawn {
            continue;
        }

        // Spawn CPU-bound chunk+parse work onto the blocking thread pool.
        // `content` is moved (not cloned) — the hash was already computed above.
        let file_path_for_blocking = file_path.clone();
        let project_id_for_blocking = project_id.to_string();
        parse_set.spawn_blocking(move || {
            let parse_started = Instant::now();
            let fp_str = file_path_for_blocking.to_string_lossy().to_string();
            let chunks = chunk_file(&file_path_for_blocking, &content, &project_id_for_blocking);
            let (symbols, references) =
                CodeParser::parse_file(&file_path_for_blocking, &content, &project_id_for_blocking);
            (
                chunks,
                symbols,
                references,
                fp_str,
                parse_started.elapsed().as_millis(),
            )
        });
    }

    // Drain remaining in-flight parse tasks from the JoinSet
    let mut consecutive_timeouts = 0;
    loop {
        match tokio::time::timeout(
            std::time::Duration::from_secs(PARSE_TIMEOUT_SECS),
            parse_set.join_next(),
        )
        .await
        {
            Ok(Some(join_result)) => {
                consecutive_timeouts = 0;
                drain_one_parse!(join_result);
            }
            Ok(None) => break,
            Err(_timeout) => {
                tracing::warn!(
                    timeout_secs = PARSE_TIMEOUT_SECS,
                    pending = parse_set.len(),
                    "Final drain parse task timed out"
                );
                status
                    .failed_files
                    .push(format!("parse_timeout_{}s", PARSE_TIMEOUT_SECS));
                if let Some(join_result) = parse_set.try_join_next() {
                    consecutive_timeouts = 0;
                    let _ = drain_one_parse!(join_result);
                } else {
                    consecutive_timeouts += 1;
                    if consecutive_timeouts > parse_set.len() {
                        tracing::warn!(
                            pending = parse_set.len(),
                            "All remaining tasks timed out, aborting final drain"
                        );
                        break;
                    }
                }
            }
        }
    }

    // Flush any remaining buffered file hashes
    if !hash_buffer.is_empty() {
        let _ = state
            .storage
            .set_file_hashes_batch(project_id, &hash_buffer)
            .await;
    }

    tracing::info!(project_id = %project_id, phase = "embedding_started", total_chunks = status.total_chunks, total_symbols = status.total_symbols, "Embedding dispatch started");

    if !chunk_buffer.is_empty() {
        let batch = chunk_buffer;
        let batch_len = batch.len() as u64;
        let _permit = state.db_semaphore.acquire().await;
        let chunk_write_started = Instant::now();
        let results = state
            .storage
            .create_code_chunks_batch(batch)
            .await
            .map_err(|e| {
                crate::AppError::Internal(
                    format!(
                        "failed to persist final code chunk batch for project {project_id}: {e}"
                    )
                    .into(),
                )
            })?;
        let chunk_write_elapsed = chunk_write_started.elapsed().as_millis();
        chunk_db_write_elapsed_ms += chunk_write_elapsed;
        chunks_written += batch_len;
        emit_index_timing(
            project_id,
            "chunk_db_write",
            chunk_write_elapsed,
            status.indexed_files as u64,
            batch_len,
            status.total_symbols as u64,
            (total_relation_stats.created + total_relation_stats.failed) as u64,
            status.failed_files.len() as u64,
        );

        for (id, chunk) in results {
            let enqueue_started = Instant::now();
            let _ = state
                .embedding_queue
                .send(EmbeddingRequest {
                    text: chunk.content,
                    responder: None,
                    target: Some(EmbeddingTarget::Chunk(id)),
                    retry_count: 0,
                })
                .await;
            embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
            embeddings_enqueued += 1;
        }
    }

    if !symbol_buffer.is_empty() {
        let batch = symbol_buffer;
        let batch_len = batch.len() as u64;
        let _permit = state.db_semaphore.acquire().await;
        let symbol_write_started = Instant::now();
        let ids = state
            .storage
            .create_code_symbols_batch(batch.clone())
            .await?;
        let symbol_write_elapsed = symbol_write_started.elapsed().as_millis();
        symbol_db_write_elapsed_ms += symbol_write_elapsed;
        symbols_written += batch_len;
        emit_index_timing(
            project_id,
            "symbol_db_write",
            symbol_write_elapsed,
            status.indexed_files as u64,
            status.total_chunks as u64,
            batch_len,
            (total_relation_stats.created + total_relation_stats.failed) as u64,
            status.failed_files.len() as u64,
        );

        for (id, sym) in ids.iter().zip(batch.iter()) {
            let embed_text = sym
                .signature
                .clone()
                .unwrap_or_else(|| format!("{} {}", sym.symbol_type, sym.name));
            let enqueue_started = Instant::now();
            let _ = state
                .embedding_queue
                .send(EmbeddingRequest {
                    text: embed_text,
                    responder: None,
                    target: Some(EmbeddingTarget::Symbol(id.clone())),
                    retry_count: 0,
                })
                .await;
            embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
            embeddings_enqueued += 1;
        }
    }

    emit_index_timing(
        project_id,
        "embedding_enqueue",
        embedding_enqueue_elapsed_ms,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        embeddings_enqueued,
        status.failed_files.len() as u64,
    );

    // Final flush of remaining relations
    if !relation_buffer.is_empty() {
        let relation_started = Instant::now();
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
        relation_create_elapsed_ms += relation_started.elapsed().as_millis();
        emit_index_timing(
            project_id,
            "relation_create",
            relation_started.elapsed().as_millis(),
            status.indexed_files as u64,
            status.total_chunks as u64,
            status.total_symbols as u64,
            (stats.created + stats.failed) as u64,
            status.failed_files.len() as u64,
        );
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

    tracing::info!(project_id = %project_id, phase = "projection_refresh_started", "Projection/materialized read-model refresh started");

    status.mark_structural_generation_advanced();
    status.status = if status.total_files == 0 {
        status.mark_semantic_generation_caught_up();
        IndexState::Completed
    } else {
        IndexState::EmbeddingPending
    };
    status.completed_at = Some(crate::types::Datetime::default());
    status.refresh_lifecycle_states();

    let status_started = Instant::now();
    state.storage.update_index_status(status.clone()).await?;
    let final_status_elapsed = status_started.elapsed().as_millis();
    status_update_elapsed_ms += final_status_elapsed;
    emit_index_timing(
        project_id,
        "status_update",
        final_status_elapsed,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );

    // Rebuild in-memory BM25 index for this project from the freshly inserted chunks
    let bm25_started = Instant::now();
    state
        .code_search
        .rebuild_from_storage(state.storage.as_ref(), project_id)
        .await;
    emit_index_timing(
        project_id,
        "bm25_rebuild",
        bm25_started.elapsed().as_millis(),
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );

    emit_index_timing(
        project_id,
        "file_read_hash_total",
        file_read_hash_elapsed_ms,
        files_read,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "parse_chunk_total",
        parse_chunk_elapsed_ms,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "chunk_db_write_total",
        chunk_db_write_elapsed_ms,
        status.indexed_files as u64,
        chunks_written,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "symbol_db_write_total",
        symbol_db_write_elapsed_ms,
        status.indexed_files as u64,
        status.total_chunks as u64,
        symbols_written,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "relation_create_total",
        relation_create_elapsed_ms,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "status_update_total",
        status_update_elapsed_ms,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "total_index",
        total_started.elapsed().as_millis(),
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );

    tracing::info!(project_id = %project_id, phase = "final_persistence_completed", status = %status.status, "One-shot code index task completed");

    Ok(status)
}

/// Result of an incremental re-index operation.
#[derive(Debug, Default)]
pub struct IncrementalResult {
    /// New/updated chunks that were written to storage (without embedding yet).
    pub new_chunks: Vec<CodeChunk>,
    /// File paths that were removed from the index (file no longer exists on disk).
    pub deleted_files: Vec<String>,
    /// Number of files whose content changed and were re-indexed.
    pub updated_files: usize,
}

/// Incremental re-index for changed files only.
/// Returns an [`IncrementalResult`] describing what changed.
/// The caller is responsible for rebuilding in-memory indices (BM25 etc.) when needed.
pub async fn incremental_index(
    state: Arc<AppState>,
    project_id: &str,
    changed_paths: Vec<std::path::PathBuf>,
) -> Result<IncrementalResult> {
    let mut result = IncrementalResult::default();
    // Keep a local alias for the previous `updated` counter used inside the macro.
    macro_rules! inc_updated {
        () => {
            result.updated_files += 1;
        };
    }

    // Issue 4 fix: Bounded-concurrency parsing via JoinSet (same pattern as do_index_project).
    let max_concurrent_parses = std::cmp::max(4, num_cpus::get() / 2);
    // Return type: (chunks, symbols, references, path_str, new_hash)
    type IncrResult = (
        Vec<CodeChunk>,
        Vec<CodeSymbol>,
        Vec<CodeReference>,
        String,
        String,
    );
    let mut parse_set: tokio::task::JoinSet<IncrResult> = tokio::task::JoinSet::new();

    // Process one completed incremental parse result (DB writes + relations + hash store).
    macro_rules! drain_one_incr {
        ($join_result:expr) => {{
            let (chunks, symbols, references, path_str, new_hash) = $join_result.map_err(|e| {
                crate::AppError::Internal(
                    format!("incremental parse/chunk panicked: {e}").into(),
                )
            })?;

            let _permit = state.db_semaphore.acquire().await;
            let results = state
                .storage
                .create_code_chunks_batch(chunks)
                .await
                .map_err(|e| crate::AppError::Internal(
                    format!("failed to persist incremental code chunks for {path_str}: {e}").into(),
                ))?;

            for (id, chunk) in results {
                // Collect the written chunk so the caller can inspect / BM25-rebuild selectively.
                result.new_chunks.push(chunk.clone());
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
            // Also detect containment edges from symbol nesting within this file.
            let containment_refs = detect_containment_references(&symbols);
            let mut all_refs = references;
            all_refs.extend(containment_refs);

            if !all_refs.is_empty() {
                let mut symbol_index = SymbolIndex::new();
                if let Ok(all_symbols) = state.storage.get_project_symbols(project_id).await {
                    symbol_index.add_batch(&all_symbols);
                }
                symbol_index.add_batch(&symbols);
                let _stats = create_symbol_relations(
                    state.storage.as_ref(),
                    project_id,
                    &all_refs,
                    &symbol_index,
                )
                .await;
            }

            // Store updated file hash
            let _ = state
                .storage
                .set_file_hash(project_id, &path_str, &new_hash)
                .await;
            inc_updated!();
        }};
    }

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
                        result.deleted_files.push(path_str.clone());
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

        let mut skip_spawn = false;
        if parse_set.len() >= max_concurrent_parses {
            match tokio::time::timeout(
                std::time::Duration::from_secs(PARSE_TIMEOUT_SECS),
                parse_set.join_next(),
            )
            .await
            {
                Ok(Some(join_result)) => {
                    drain_one_incr!(join_result);
                }
                Ok(None) => {}
                Err(_timeout) => {
                    tracing::warn!(
                        timeout_secs = PARSE_TIMEOUT_SECS,
                        pending = parse_set.len(),
                        path = %path_str,
                        "Incremental parse task timed out, skipping file"
                    );
                    skip_spawn = true;
                }
            }
        }

        if skip_spawn {
            continue;
        }

        // Spawn CPU-bound chunk+parse onto the blocking thread pool.
        // `content` is moved (not cloned) — the hash was already computed above.
        // `path_str` and `new_hash` are moved through the result for post-processing.
        let path_for_blocking = path.clone();
        let project_id_for_blocking = project_id.to_string();
        parse_set.spawn_blocking(move || {
            let chunks =
                super::chunker::chunk_file(&path_for_blocking, &content, &project_id_for_blocking);
            let (symbols, references) =
                CodeParser::parse_file(&path_for_blocking, &content, &project_id_for_blocking);
            (chunks, symbols, references, path_str, new_hash)
        });
    }

    // Drain remaining in-flight parse tasks
    let mut consecutive_timeouts = 0;
    loop {
        match tokio::time::timeout(
            std::time::Duration::from_secs(PARSE_TIMEOUT_SECS),
            parse_set.join_next(),
        )
        .await
        {
            Ok(Some(join_result)) => {
                consecutive_timeouts = 0;
                drain_one_incr!(join_result);
            }
            Ok(None) => break,
            Err(_timeout) => {
                tracing::warn!(
                    timeout_secs = PARSE_TIMEOUT_SECS,
                    pending = parse_set.len(),
                    "Incremental final drain parse task timed out"
                );
                if let Some(join_result) = parse_set.try_join_next() {
                    consecutive_timeouts = 0;
                    let _ = drain_one_incr!(join_result);
                } else {
                    consecutive_timeouts += 1;
                    if consecutive_timeouts > parse_set.len() {
                        tracing::warn!(
                            pending = parse_set.len(),
                            "All remaining tasks timed out, aborting final drain"
                        );
                        break;
                    }
                }
            }
        }
    }

    // NOTE: rebuild_from_storage is intentionally NOT called here.
    // The caller (CodebaseManager) decides when to rebuild the in-memory BM25
    // index, allowing it to batch multiple incremental results or defer the
    // rebuild to a background worker.

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;
    use std::collections::HashSet;
    use std::fs;
    use std::sync::{Arc, Mutex, OnceLock};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::{Layer, Registry};

    static TIMING_PHASES: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();

    struct TimingPhaseLayer {
        phases: Arc<Mutex<Vec<String>>>,
    }

    impl<S> Layer<S> for TimingPhaseLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = PhaseVisitor::default();
            event.record(&mut visitor);
            if let Some(phase) = visitor.phase {
                self.phases.lock().unwrap().push(phase);
            }
        }
    }

    #[derive(Default)]
    struct PhaseVisitor {
        phase: Option<String>,
    }

    impl Visit for PhaseVisitor {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "phase" {
                self.phase = Some(value.to_string());
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "phase" {
                self.phase = Some(format!("{value:?}").trim_matches('"').to_string());
            }
        }
    }

    fn init_timing_phase_capture() -> Arc<Mutex<Vec<String>>> {
        let phases = TIMING_PHASES
            .get_or_init(|| {
                let phases = Arc::new(Mutex::new(Vec::new()));
                let subscriber = Registry::default().with(TimingPhaseLayer {
                    phases: phases.clone(),
                });
                let _ = tracing::subscriber::set_global_default(subscriber);
                phases
            })
            .clone();
        phases.lock().unwrap().clear();
        phases
    }

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

        // Use in-memory BM25 engine (rebuilt automatically after indexing)
        let chunks = ctx
            .state
            .code_search
            .search("fn test", None, 200, ctx.state.storage.as_ref())
            .await;
        assert_eq!(chunks.len(), 150);
    }

    #[tokio::test]
    async fn index_project_empty_directory_completes_with_zero_files() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("empty-project");
        fs::create_dir_all(&project_dir).unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();

        assert_eq!(status.project_id, "empty-project");
        assert_eq!(status.status, IndexState::Completed);
        assert_eq!(status.total_files, 0);
        assert_eq!(status.indexed_files, 0);
        assert_eq!(status.total_chunks, 0);
        assert_eq!(status.total_symbols, 0);
        assert_eq!(
            status.structural_state,
            crate::types::StructuralState::Ready
        );
        assert_eq!(status.semantic_state, crate::types::SemanticState::Ready);
        assert_eq!(status.structural_generation, 1);
        assert_eq!(status.semantic_generation, 1);

        let stored = ctx
            .state
            .storage
            .get_index_status("empty-project")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, IndexState::Completed);
        assert_eq!(
            ctx.state
                .storage
                .count_manifest_entries("empty-project")
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn index_project_mixed_mobile_fixture_is_stable_and_extracts_symbols() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("mobile-mixed-project");

        let mixed_files = [
            (
                "ios/Worker.h",
                "@interface Worker : NSObject\n- (void)start;\n@end\n",
            ),
            (
                "ios/Worker.m",
                "#import <Foundation/Foundation.h>\n@interface Worker : NSObject\n- (void)start;\n@end\n@implementation Worker\n- (void)start { NSLog(@\"start\"); }\n@end\n",
            ),
            (
                "ios/Bridge.mm",
                "#import <Foundation/Foundation.h>\n@interface Bridge : NSObject\n- (void)bridgeRun;\n@end\n@implementation Bridge\n- (void)bridgeRun { [self bridgeRun]; }\n@end\n",
            ),
            (
                "swift/App.swift",
                "import Foundation\nclass SwiftScreen {\n    func render() { swiftHelper() }\n}\nfunc swiftHelper() {}\n",
            ),
            (
                "kotlin/App.kt",
                "package com.example\nimport kotlin.collections.List\nclass KotlinRepo {\n    fun run() { println(\"ok\") }\n}\n",
            ),
            (
                "kotlin/buildLogic.kts",
                "fun configureBuild() { println(\"kts\") }\nconfigureBuild()\n",
            ),
            (
                "native/main.c",
                "#include <stdio.h>\nint c_entry(void) { printf(\"c\"); return 0; }\n",
            ),
            (
                "native/main.cpp",
                "#include <vector>\nint cpp_entry() { return 0; }\n",
            ),
        ];

        let ignored_files = [
            "Pods/SDK/Generated.m",
            ".gradle/cache/Cache.kt",
            ".android/generated/Build.kt",
            ".symlinks/plugins/Plugin.swift",
            "build/intermediates/Gen.cpp",
            "generated/schema/Auto.c",
            ".generated/code/Auto.mm",
        ];

        for (relative_path, content) in &mixed_files {
            let path = project_dir.join(relative_path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }

        for relative_path in &ignored_files {
            let path = project_dir.join(relative_path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "fn ignored() {}\n").unwrap();
        }

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();
        assert_eq!(status.status, IndexState::EmbeddingPending);
        assert_eq!(status.total_files, mixed_files.len() as u32);
        assert!(
            status.failed_files.is_empty(),
            "indexing should not fail for mixed mobile files: {:?}",
            status.failed_files
        );

        let scanned = crate::codebase::scanner::scan_directory(&project_dir).unwrap();
        let scanned_set: HashSet<String> = scanned
            .iter()
            .map(|path| {
                path.strip_prefix(&project_dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(scanned_set.len(), mixed_files.len());
        for forbidden in ["Pods/", ".gradle/", ".android/", ".symlinks/", "build/"] {
            assert!(
                scanned_set.iter().all(|path| !path.starts_with(forbidden)),
                "scanner unexpectedly included file under {forbidden}: {:?}",
                scanned_set
            );
        }
        assert!(
            scanned_set.iter().all(|path| !path.contains("/generated/")),
            "scanner unexpectedly included generated directory file: {:?}",
            scanned_set
        );
        assert!(
            scanned_set
                .iter()
                .all(|path| !path.starts_with(".generated/")),
            "scanner unexpectedly included .generated directory file: {:?}",
            scanned_set
        );

        let symbols = ctx
            .state
            .storage
            .get_project_symbols(&status.project_id)
            .await
            .unwrap();
        let symbol_names: HashSet<String> =
            symbols.iter().map(|symbol| symbol.name.clone()).collect();

        for expected in [
            "Worker",
            "start",
            "bridgeRun",
            "SwiftScreen",
            "render",
            "swiftHelper",
            "KotlinRepo",
            "run",
            "c_entry",
            "cpp_entry",
        ] {
            assert!(
                symbol_names.contains(expected),
                "expected symbol '{expected}' in indexed mixed mobile fixture, got {:?}",
                symbol_names
            );
        }
    }

    #[tokio::test]
    async fn index_project_emits_baseline_timing_events_for_fixture() {
        let captured_phases = init_timing_phase_capture();
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("phase-checkpoints-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(
            project_dir.join("lib.rs"),
            "pub fn alpha() -> i32 { beta() }\nfn beta() -> i32 { 1 }\n",
        )
        .unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();

        assert_eq!(status.status, IndexState::EmbeddingPending);
        assert_eq!(status.total_files, 1);
        assert!(status.total_chunks >= 1);
        assert!(status.total_symbols >= 2);

        let phases = captured_phases.lock().unwrap().clone();
        for required_phase in [
            "cleanup",
            "scan",
            "file_read_hash",
            "parse_chunk",
            "chunk_db_write",
            "symbol_db_write",
            "embedding_enqueue",
            "status_update",
            "bm25_rebuild",
            "total_index",
        ] {
            eprintln!("captured_timing_phase={required_phase}");
            assert!(
                phases.iter().any(|phase| phase == required_phase),
                "missing timing phase {required_phase}; captured phases: {phases:?}"
            );
        }
    }
}
