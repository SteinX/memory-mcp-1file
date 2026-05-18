use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::fs;

use crate::config::{AppState, CodeIndexPipelineMode};
use crate::storage::StorageBackend;
use crate::types::{derive_project_id, CapabilityKind, IndexState, IndexStatus};
use crate::Result;

use super::chunker::chunk_file_for_generation;
use super::parser::CodeParser;
use super::relations::{
    create_symbol_relations, create_symbol_relations_for_generation, detect_containment_references,
    RelationStats,
};
use super::scanner::{is_generated_source_content, scan_directory_with_filter, IndexFilterConfig};
use super::symbol_index::SymbolIndex;

use crate::embedding::{EmbeddingRequest, EmbeddingTarget};
use crate::types::code::{CodeChunk, IndexFileCheckpoint, IndexJobPhase, IndexJobState};
use crate::types::symbol::{CodeReference, CodeSymbol};

const PARSE_TIMEOUT_SECS: u64 = 30;
const EMBEDDING_REENQUEUE_YIELD_EVERY: usize = 256;

fn effective_parse_workers(config: &crate::config::CodeIndexConfig) -> usize {
    std::cmp::max(config.parse_workers, 2)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct IndexResumeOptions {
    pub resume: bool,
    pub job_id: Option<String>,
    pub resume_token: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct FileCheckpointContext {
    job_id: String,
    generation: u64,
    completed_relative_paths: HashSet<String>,
}

#[derive(Default)]
pub struct IndexMetrics {
    pub file_read_hash_elapsed_ms: u128,
    pub parse_chunk_elapsed_ms: u128,
    pub chunk_db_write_elapsed_ms: u128,
    pub symbol_db_write_elapsed_ms: u128,
    pub embedding_enqueue_elapsed_ms: u128,
    pub relation_create_elapsed_ms: u128,
    pub status_update_elapsed_ms: u128,
    pub chunks_written: u64,
    pub symbols_written: u64,
    pub embeddings_enqueued: u64,
    pub files_read: u64,
}

struct ParsedFile {
    seq: usize,
    path: std::path::PathBuf,
    path_str: String,
    file_hash: Option<String>,
    chunks: Vec<CodeChunk>,
    symbols: Vec<CodeSymbol>,
    references: Vec<CodeReference>,
    read_elapsed_ms: u128,
    parse_elapsed_ms: u128,
    error: Option<String>,
    skipped: bool,
}

fn code_chunk_embedding_request(id: String, content: String) -> EmbeddingRequest {
    EmbeddingRequest {
        text: content,
        responder: None,
        target: Some(EmbeddingTarget::Chunk(id)),
        retry_count: 0,
    }
}

async fn enqueue_embedding_for_backfill(
    state: &Arc<AppState>,
    project_id: &str,
    request: EmbeddingRequest,
    enqueued: &mut u64,
    failed: &mut u64,
) {
    let (target_kind, target_id) = match &request.target {
        Some(EmbeddingTarget::Chunk(id)) => ("chunk", id.clone()),
        Some(EmbeddingTarget::Symbol(id)) => ("symbol", id.clone()),
        None => ("adhoc", String::new()),
    };

    match state.embedding_queue.send(request).await {
        Ok(()) => {
            *enqueued += 1;
        }
        Err(error) => {
            *failed += 1;
            if *failed == 1 || failed.is_multiple_of(1000) {
                tracing::warn!(
                    project_id = %project_id,
                    target_kind,
                    target_id = %target_id,
                    failed = *failed,
                    error = %error,
                    "Failed to enqueue post-publish embedding backfill item"
                );
            }
        }
    }
}

async fn enqueue_generation_embeddings_after_publish(state: Arc<AppState>, project_id: String) {
    let started = Instant::now();
    let mut enqueued = 0u64;
    let mut failed = 0u64;

    let unembedded_chunks = match state.storage.get_unembedded_chunks(&project_id).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                project_id = %project_id,
                error = %error,
                "Failed to load unembedded chunks for post-publish backfill"
            );
            return;
        }
    };
    for (index, (id, content)) in unembedded_chunks.into_iter().enumerate() {
        enqueue_embedding_for_backfill(
            &state,
            &project_id,
            code_chunk_embedding_request(id, content),
            &mut enqueued,
            &mut failed,
        )
        .await;
        if index > 0 && index.is_multiple_of(EMBEDDING_REENQUEUE_YIELD_EVERY) {
            tokio::task::yield_now().await;
        }
    }

    let unembedded_symbols = match state.storage.get_unembedded_symbols(&project_id).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                project_id = %project_id,
                error = %error,
                "Failed to load unembedded symbols for post-publish backfill"
            );
            return;
        }
    };
    for (index, (id, text)) in unembedded_symbols.into_iter().enumerate() {
        enqueue_embedding_for_backfill(
            &state,
            &project_id,
            EmbeddingRequest {
                text,
                responder: None,
                target: Some(EmbeddingTarget::Symbol(id)),
                retry_count: 0,
            },
            &mut enqueued,
            &mut failed,
        )
        .await;
        if index > 0 && index.is_multiple_of(EMBEDDING_REENQUEUE_YIELD_EVERY) {
            tokio::task::yield_now().await;
        }
    }

    tracing::info!(
        project_id = %project_id,
        enqueued,
        failed,
        queue_capacity = state.embedding_queue.channel_capacity(),
        elapsed_ms = started.elapsed().as_millis(),
        "Post-publish embedding backfill queued"
    );
}

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

    let admission = IndexAdmissionGuard::try_acquire(state.clone(), project_id.clone())?;

    let result = index_project_after_admission(state, project_path).await;
    drop(admission);

    result
}

pub async fn index_project_with_filter(
    state: Arc<AppState>,
    project_path: &Path,
    filter_config: IndexFilterConfig,
) -> Result<IndexStatus> {
    let project_id = derive_project_id(project_path)
        .map_err(|error| crate::AppError::InvalidPath(error.to_string()))?;

    let admission = IndexAdmissionGuard::try_acquire(state.clone(), project_id.clone())?;

    let result = index_project_after_admission_with_resume_and_filter(
        state,
        project_path,
        IndexResumeOptions::default(),
        filter_config,
    )
    .await;
    drop(admission);

    result
}

pub(crate) async fn index_project_after_admission(
    state: Arc<AppState>,
    project_path: &Path,
) -> Result<IndexStatus> {
    index_project_after_admission_with_resume(state, project_path, IndexResumeOptions::default())
        .await
}

pub(crate) async fn index_project_after_admission_with_resume(
    state: Arc<AppState>,
    project_path: &Path,
    resume_options: IndexResumeOptions,
) -> Result<IndexStatus> {
    let filter_config = IndexFilterConfig {
        include_patterns: state.config.code_index.include_patterns.clone(),
        exclude_patterns: state.config.code_index.exclude_patterns.clone(),
    };
    index_project_after_admission_with_resume_and_filter(
        state,
        project_path,
        resume_options,
        filter_config,
    )
    .await
}

pub(crate) async fn index_project_after_admission_with_resume_and_filter(
    state: Arc<AppState>,
    project_path: &Path,
    resume_options: IndexResumeOptions,
    filter_config: IndexFilterConfig,
) -> Result<IndexStatus> {
    let (status, _metrics) = index_project_after_admission_with_resume_and_filter_inner(
        state,
        project_path,
        resume_options,
        filter_config,
    )
    .await?;
    Ok(status)
}

async fn index_project_after_admission_with_resume_and_filter_inner(
    state: Arc<AppState>,
    project_path: &Path,
    resume_options: IndexResumeOptions,
    filter_config: IndexFilterConfig,
) -> Result<(IndexStatus, IndexMetrics)> {
    let project_id = derive_project_id(project_path)
        .map_err(|error| crate::AppError::InvalidPath(error.to_string()))?;

    let state_clone = state.clone();
    let project_path_clone = project_path.to_path_buf();
    let project_id_clone = project_id.clone();

    let handle = tokio::spawn(async move {
        do_index_project(
            state_clone,
            &project_path_clone,
            &project_id_clone,
            resume_options,
            filter_config,
        )
        .await
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
        Ok((status, metrics)) => Ok((status, metrics)),
        Err(e) => {
            tracing::error!(project_id = %project_id, error = %e, "Indexing failed");
            if is_non_destructive_index_error(&e) {
                return Err(e);
            }
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

pub async fn index_project_with_metrics(
    state: Arc<AppState>,
    project_path: &Path,
) -> Result<(IndexStatus, IndexMetrics)> {
    let project_id = derive_project_id(project_path)
        .map_err(|error| crate::AppError::InvalidPath(error.to_string()))?;
    let admission = IndexAdmissionGuard::try_acquire(state.clone(), project_id)?;
    let filter_config = IndexFilterConfig {
        include_patterns: state.config.code_index.include_patterns.clone(),
        exclude_patterns: state.config.code_index.exclude_patterns.clone(),
    };
    let result = index_project_after_admission_with_resume_and_filter_inner(
        state,
        project_path,
        IndexResumeOptions::default(),
        filter_config,
    )
    .await;
    drop(admission);
    result
}

struct IndexAdmissionGuard {
    state: Arc<AppState>,
    project_id: String,
}

impl IndexAdmissionGuard {
    fn try_acquire(state: Arc<AppState>, project_id: String) -> Result<Self> {
        let inserted = {
            let mut guard = state
                .indexing_projects
                .lock()
                .expect("indexing_projects mutex poisoned");
            guard.insert(project_id.clone())
        };

        if !inserted {
            return Err(crate::AppError::Indexing(format!(
                "already_running: indexing already in progress for project_id {project_id}"
            )));
        }

        Ok(Self { state, project_id })
    }
}

impl Drop for IndexAdmissionGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.indexing_projects.lock() {
            guard.remove(&self.project_id);
        }
    }
}

fn is_non_destructive_index_error(error: &crate::AppError) -> bool {
    match error {
        crate::AppError::Indexing(message) => {
            message.starts_with("already_running")
                || message.starts_with("stale_generation")
                || message.starts_with("bm25_rebuild_failed")
        }
        _ => false,
    }
}

fn prepare_started_index_status(
    project_id: &str,
    project_path: &Path,
    previous: Option<IndexStatus>,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
) -> IndexStatus {
    let previous_structural_generation = previous
        .as_ref()
        .map(|status| status.structural_generation)
        .unwrap_or(0);
    let previous_semantic_generation = previous
        .as_ref()
        .map(|status| status.semantic_generation)
        .unwrap_or(0);

    let mut status = IndexStatus::new(project_id.to_string());
    status.root_path = Some(
        project_path
            .canonicalize()
            .unwrap_or_else(|_| project_path.to_path_buf())
            .to_string_lossy()
            .into_owned(),
    );
    // Reserve the generation at start. Finalization compares this value with
    // storage so a cancelled/stale generation cannot publish Completed or
    // EmbeddingPending after a newer generation has started.
    status.structural_generation = previous_structural_generation.saturating_add(1);
    status.semantic_generation = previous_semantic_generation.min(status.structural_generation);
    status.include_patterns = include_patterns;
    status.exclude_patterns = exclude_patterns;
    status.refresh_lifecycle_states();
    status
}

pub(crate) async fn finalize_index_status_if_current(
    state: &Arc<AppState>,
    mut status: IndexStatus,
    expected_structural_generation: u64,
) -> Result<IndexStatus> {
    let current = state
        .storage
        .get_index_status(&status.project_id)
        .await?
        .ok_or_else(|| {
            crate::AppError::Indexing(format!(
                "stale_generation: missing active status for project_id {} generation {}",
                status.project_id, expected_structural_generation
            ))
        })?;

    if current.status != IndexState::Indexing
        || current.structural_generation != expected_structural_generation
    {
        return Err(crate::AppError::Indexing(format!(
            "stale_generation: refusing to finalize project_id {} generation {}; current status={} structural_generation={}",
            status.project_id,
            expected_structural_generation,
            current.status,
            current.structural_generation
        )));
    }

    status.structural_generation = expected_structural_generation;
    if status.status == IndexState::Completed {
        status.mark_semantic_generation_caught_up();
    } else if status.semantic_generation > status.structural_generation {
        status.semantic_generation = status.structural_generation;
    }
    status.completed_at = Some(crate::types::Datetime::default());
    status.refresh_lifecycle_states();
    state.storage.update_index_status(status.clone()).await?;
    Ok(status)
}

async fn ensure_index_generation_current(
    state: &Arc<AppState>,
    project_id: &str,
    expected_structural_generation: u64,
) -> Result<()> {
    let current = state
        .storage
        .get_index_status(project_id)
        .await?
        .ok_or_else(|| {
            crate::AppError::Indexing(format!(
                "stale_generation: missing active status for project_id {project_id} generation {expected_structural_generation}"
            ))
        })?;

    if current.status != IndexState::Indexing
        || current.structural_generation != expected_structural_generation
    {
        return Err(crate::AppError::Indexing(format!(
            "stale_generation: refusing BM25 rebuild for project_id {project_id} generation {expected_structural_generation}; current status={} structural_generation={}",
            current.status, current.structural_generation
        )));
    }

    Ok(())
}

async fn record_bm25_rebuild_failure_if_current(
    state: &Arc<AppState>,
    project_id: &str,
    status: IndexStatus,
    active_structural_generation: u64,
    error: impl std::fmt::Display,
) -> crate::AppError {
    let message = format!(
        "bm25_rebuild_failed: failed to rebuild BM25 index for project {project_id}: {error}"
    );
    let mut failed_status = status;
    failed_status.status = IndexState::Failed;
    failed_status.error_message = Some(message.clone());
    let _ =
        finalize_index_status_if_current(state, failed_status, active_structural_generation).await;
    crate::AppError::Indexing(message)
}

async fn rebuild_bm25_and_finalize_index_status(
    state: &Arc<AppState>,
    project_id: &str,
    status: IndexStatus,
    active_structural_generation: u64,
) -> Result<IndexStatus> {
    ensure_index_generation_current(state, project_id, active_structural_generation).await?;

    match state
        .code_search
        .try_rebuild_from_storage(
            state.storage.as_ref(),
            project_id,
            Some(active_structural_generation),
        )
        .await
    {
        Ok(_) => {
            finalize_index_status_if_current(state, status, active_structural_generation).await
        }
        Err(error) => Err(record_bm25_rebuild_failure_if_current(
            state,
            project_id,
            status,
            active_structural_generation,
            error,
        )
        .await),
    }
}

async fn promote_index_generation_and_cleanup(
    state: &Arc<AppState>,
    project_id: &str,
    target_generation: u64,
) -> Result<()> {
    ensure_index_generation_current(state, project_id, target_generation).await?;

    let promotion_started = Instant::now();
    state
        .storage
        .set_active_generation(project_id, target_generation)
        .await?;
    state
        .storage
        .set_serving_generation(project_id, CapabilityKind::Bm25, target_generation)
        .await?;
    state
        .storage
        .set_serving_generation(project_id, CapabilityKind::Symbols, target_generation)
        .await?;
    state
        .storage
        .set_serving_generation(project_id, CapabilityKind::Graph, target_generation)
        .await?;
    state
        .storage
        .set_indexing_generation(project_id, None)
        .await?;
    emit_index_timing(
        project_id,
        "promote",
        promotion_started.elapsed().as_millis(),
        0,
        0,
        0,
        0,
        0,
    );

    let serving_metadata = state.storage.get_serving_metadata(project_id).await?;
    let protected_serving_generations = [
        serving_metadata.structural,
        serving_metadata.bm25,
        serving_metadata.symbols,
        serving_metadata.graph,
        serving_metadata.vector,
        serving_metadata.semantic,
    ];
    let abandoned_generations = state.storage.list_abandoned_generations(project_id).await?;
    if abandoned_generations.is_empty() {
        return Ok(());
    }

    let cleanup_started = Instant::now();
    for generation in abandoned_generations {
        if generation >= target_generation
            || protected_serving_generations
                .iter()
                .any(|serving_generation| *serving_generation == Some(generation))
        {
            continue;
        }
        let generation_cleanup_started = Instant::now();
        state
            .storage
            .delete_project_generation(project_id, generation)
            .await?;
        tracing::info!(
            project_id = %project_id,
            generation,
            elapsed_ms = generation_cleanup_started.elapsed().as_millis(),
            "Deleted old staged code index generation"
        );
    }
    emit_index_timing(
        project_id,
        "cleanup",
        cleanup_started.elapsed().as_millis(),
        0,
        0,
        0,
        0,
        0,
    );

    Ok(())
}

fn checkpoint_resume_token(phase: &IndexJobPhase, files_done: u64) -> String {
    let phase = match phase {
        IndexJobPhase::Discover => "discover",
        IndexJobPhase::Parse => "parse",
        IndexJobPhase::Chunk => "chunk",
        IndexJobPhase::Symbols => "symbols",
        IndexJobPhase::Relations => "relations",
        IndexJobPhase::Embed => "embed",
        IndexJobPhase::EmbedEnqueue => "embed_enqueue",
        IndexJobPhase::Bm25 => "bm25",
        IndexJobPhase::Finalize => "finalize",
        IndexJobPhase::Promote => "promote",
        IndexJobPhase::Cleanup => "cleanup",
    };
    format!("ckpt_v1_phase_{phase}_file_{files_done}")
}

async fn prepare_file_checkpoint_context(
    state: &Arc<AppState>,
    project_id: &str,
    generation: u64,
    resume_options: &IndexResumeOptions,
) -> Result<Option<FileCheckpointContext>> {
    let job_id = if resume_options.resume {
        resume_options.job_id.clone().ok_or_else(|| {
            crate::AppError::Indexing(
                "resume_token_required: job_id is required for checkpoint resume".into(),
            )
        })?
    } else {
        state
            .storage
            .list_index_jobs_for_project(project_id)
            .await?
            .into_iter()
            .find(|job| job.target_generation == generation)
            .map(|job| job.job_id)
            .unwrap_or_default()
    };

    if job_id.is_empty() {
        return Ok(None);
    }

    let checkpoints = state
        .storage
        .list_file_checkpoints_for_job(project_id, generation)
        .await?;
    let completed_relative_paths = if resume_options.resume {
        let token = resume_options.resume_token.as_deref().ok_or_else(|| {
            crate::AppError::Indexing(
                "resume_token_required: resume_token is required for checkpoint resume".into(),
            )
        })?;
        if checkpoints.is_empty() {
            return Err(crate::AppError::Indexing(
                "checkpoint_generation_missing: no file checkpoints exist for requested resume"
                    .into(),
            ));
        }
        let files_done = checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.completed)
            .count() as u64;
        let expected_parse = checkpoint_resume_token(&IndexJobPhase::Parse, files_done);
        let expected_embed = checkpoint_resume_token(&IndexJobPhase::Embed, files_done);
        if token != expected_parse && token != expected_embed {
            return Err(crate::AppError::Indexing(format!(
                "checkpoint_generation_missing: resume_token {token} does not match checkpoint state {expected_parse}"
            )));
        }
        checkpoints
            .into_iter()
            .filter(|checkpoint| checkpoint.completed)
            .map(|checkpoint| checkpoint.relative_file_path)
            .collect()
    } else {
        HashSet::new()
    };

    Ok(Some(FileCheckpointContext {
        job_id,
        generation,
        completed_relative_paths,
    }))
}

fn checkpoint_relative_path(file_path: &Path) -> String {
    file_path.to_string_lossy().replace('\\', "/")
}

async fn upsert_file_checkpoint_after_file(
    state: &Arc<AppState>,
    checkpoint_context: Option<&FileCheckpointContext>,
    project_id: &str,
    file_path: &Path,
    content_hash: Option<String>,
    phase: IndexJobPhase,
    chunks_written: u64,
    symbols_written: u64,
) -> Result<()> {
    let Some(context) = checkpoint_context else {
        return Ok(());
    };
    let relative_file_path = checkpoint_relative_path(file_path);
    let now = crate::types::Datetime::default();
    let checkpoint = IndexFileCheckpoint {
        id: None,
        job_id: context.job_id.clone(),
        project_id: project_id.to_string(),
        generation: context.generation,
        relative_file_path: relative_file_path.clone(),
        file_path: relative_file_path,
        content_hash: content_hash.unwrap_or_default(),
        checkpoint_generation: context.generation,
        phase,
        completed: true,
        completed_at: now.clone(),
        chunks_written,
        symbols_written,
        updated_at: now,
    };
    state.storage.upsert_file_checkpoint(&checkpoint).await
}

async fn do_index_project(
    state: Arc<AppState>,
    project_path: &Path,
    project_id: &str,
    resume_options: IndexResumeOptions,
    filter_config: IndexFilterConfig,
) -> Result<(IndexStatus, IndexMetrics)> {
    // Compile filter BEFORE any destructive cleanup — invalid patterns fail fast without data loss.
    let compiled_filter = filter_config
        .compile()
        .map_err(|e| crate::AppError::Indexing(format!("invalid_filter: {e}").into()))?;

    let total_started = Instant::now();
    let file_read_hash_elapsed_ms = 0u128;
    let parse_chunk_elapsed_ms = 0u128;
    let chunk_db_write_elapsed_ms = 0u128;
    let symbol_db_write_elapsed_ms = 0u128;
    let embedding_enqueue_elapsed_ms = 0u128;
    let relation_create_elapsed_ms = 0u128;
    let mut status_update_elapsed_ms = 0u128;
    let chunks_written = 0u64;
    let symbols_written = 0u64;
    let embeddings_enqueued = 0u64;
    let files_read = 0u64;

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
    let previous_status = state.storage.get_index_status(project_id).await?;
    let mut status = prepare_started_index_status(
        project_id,
        project_path,
        previous_status,
        filter_config.include_patterns.clone(),
        filter_config.exclude_patterns.clone(),
    );
    if resume_options.resume {
        if let Some(job_id) = resume_options.job_id.as_deref() {
            if let Some(job) = state.storage.get_index_job(project_id, job_id).await? {
                status.structural_generation = job.target_generation;
            }
        }
    }
    let active_structural_generation = status.structural_generation;
    let checkpoint_context = prepare_file_checkpoint_context(
        &state,
        project_id,
        active_structural_generation,
        &resume_options,
    )
    .await?;
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

    // Persist the indexing state before writing staged rows. If the server
    // restarts while rebuilding, status/stats can report
    // "indexing/unknown_after_restart" while the previous active generation
    // remains visible to readers until this target generation is promoted.
    tracing::info!(
        project_id = %project_id,
        root_path = status.root_path.as_deref().unwrap_or_default(),
        target_generation = active_structural_generation,
        "Persisting initial indexing metadata before staged writes"
    );
    tracing::info!(project_id = %project_id, phase = "task_spawned", "One-shot code index task spawned");
    let status_started = Instant::now();
    state.storage.update_index_status(status.clone()).await?;
    state
        .storage
        .set_indexing_generation(project_id, Some(active_structural_generation))
        .await?;
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
    let files = tokio::task::spawn_blocking(move || {
        scan_directory_with_filter(&project_path_for_scan, &compiled_filter)
    })
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

    match state.config.code_index.pipeline_mode {
        CodeIndexPipelineMode::Legacy => {
            let code_index_config = state.config.code_index.clone();
            run_legacy_index_pipeline(
                state,
                &code_index_config,
                project_path,
                project_id,
                status,
                active_structural_generation,
                total_started,
                file_read_hash_elapsed_ms,
                parse_chunk_elapsed_ms,
                chunk_db_write_elapsed_ms,
                symbol_db_write_elapsed_ms,
                embedding_enqueue_elapsed_ms,
                relation_create_elapsed_ms,
                status_update_elapsed_ms,
                chunks_written,
                symbols_written,
                embeddings_enqueued,
                files_read,
                files,
                checkpoint_context,
            )
            .await
        }
        CodeIndexPipelineMode::Staged => {
            run_staged_index_pipeline(
                state,
                project_path,
                project_id,
                status,
                active_structural_generation,
                total_started,
                files,
                checkpoint_context,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_legacy_index_pipeline(
    state: Arc<AppState>,
    config: &crate::config::CodeIndexConfig,
    _project_path: &Path,
    project_id: &str,
    mut status: IndexStatus,
    active_structural_generation: u64,
    total_started: Instant,
    mut file_read_hash_elapsed_ms: u128,
    mut parse_chunk_elapsed_ms: u128,
    mut chunk_db_write_elapsed_ms: u128,
    mut symbol_db_write_elapsed_ms: u128,
    mut embedding_enqueue_elapsed_ms: u128,
    mut relation_create_elapsed_ms: u128,
    mut status_update_elapsed_ms: u128,
    mut chunks_written: u64,
    mut symbols_written: u64,
    mut embeddings_enqueued: u64,
    mut files_read: u64,
    files: Vec<std::path::PathBuf>,
    checkpoint_context: Option<FileCheckpointContext>,
) -> Result<(IndexStatus, IndexMetrics)> {
    let monitor = state.progress.get_or_create(project_id).await;
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
    let max_concurrent_parses = effective_parse_workers(config);
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
            let file_chunk_count = chunks.len() as u64;
            let file_symbol_count = symbols.len() as u64;
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

                    let enqueue_started = Instant::now();
                    embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
                    embeddings_enqueued += results.len() as u64;
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
                            let enqueue_started = Instant::now();
                            embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
                            embeddings_enqueued += ids.len() as u64;
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

            status.indexed_files += 1;
            monitor
                .indexed_files
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            upsert_file_checkpoint_after_file(
                &state,
                checkpoint_context.as_ref(),
                project_id,
                Path::new(&fp_str),
                None,
                IndexJobPhase::Parse,
                file_chunk_count,
                file_symbol_count,
            )
            .await?;

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
        if checkpoint_context.as_ref().is_some_and(|context| {
            context
                .completed_relative_paths
                .contains(&checkpoint_relative_path(file_path))
        }) {
            status.indexed_files += 1;
            monitor
                .indexed_files
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            continue;
        }

        // Update current file in monitor for status reporting
        if let Ok(mut cf) = monitor.current_file.write() {
            *cf = file_path.to_string_lossy().to_string();
        }

        tracing::info!("Indexing file: {:?}", file_path);

        // Skip auto-generated files (no useful semantic content)
        if crate::codebase::scanner::is_ignored_file(file_path) {
            tracing::debug!(path = ?file_path, "Skipping generated file");
            status.indexed_files += 1;
            monitor
                .indexed_files
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            upsert_file_checkpoint_after_file(
                &state,
                checkpoint_context.as_ref(),
                project_id,
                file_path,
                None,
                IndexJobPhase::Parse,
                0,
                0,
            )
            .await?;
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
                status.indexed_files += 1;
                monitor
                    .indexed_files
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                upsert_file_checkpoint_after_file(
                    &state,
                    checkpoint_context.as_ref(),
                    project_id,
                    file_path,
                    None,
                    IndexJobPhase::Parse,
                    0,
                    0,
                )
                .await?;
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

        if is_generated_source_content(&content) {
            let read_hash_elapsed = read_hash_started.elapsed().as_millis();
            file_read_hash_elapsed_ms += read_hash_elapsed;
            tracing::debug!(path = ?file_path, "Skipping generated source file");
            status.indexed_files += 1;
            monitor
                .indexed_files
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            upsert_file_checkpoint_after_file(
                &state,
                checkpoint_context.as_ref(),
                project_id,
                file_path,
                None,
                IndexJobPhase::Parse,
                0,
                0,
            )
            .await?;
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

        // Skip massive files to prevent OOM/TreeSitter crashes (e.g. giant bundled JS or Dart files)
        if content.len() > 1_000_000 {
            // > 1MB
            let read_hash_elapsed = read_hash_started.elapsed().as_millis();
            file_read_hash_elapsed_ms += read_hash_elapsed;
            tracing::warn!("Skipping large file (>1MB): {:?}", file_path);
            status
                .failed_files
                .push(file_path.to_string_lossy().to_string());
            status.indexed_files += 1;
            monitor
                .indexed_files
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            upsert_file_checkpoint_after_file(
                &state,
                checkpoint_context.as_ref(),
                project_id,
                file_path,
                None,
                IndexJobPhase::Parse,
                0,
                0,
            )
            .await?;
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
                    upsert_file_checkpoint_after_file(
                        &state,
                        checkpoint_context.as_ref(),
                        project_id,
                        file_path,
                        None,
                        IndexJobPhase::Parse,
                        0,
                        0,
                    )
                    .await?;
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
        let generation_for_blocking = active_structural_generation;
        parse_set.spawn_blocking(move || {
            let parse_started = Instant::now();
            let fp_str = file_path_for_blocking.to_string_lossy().to_string();
            let chunks = chunk_file_for_generation(
                &file_path_for_blocking,
                &content,
                &project_id_for_blocking,
                Some(generation_for_blocking),
            );
            let (symbols, references) =
                CodeParser::parse_file(&file_path_for_blocking, &content, &project_id_for_blocking);
            let symbols = symbols
                .into_iter()
                .map(|mut symbol| {
                    symbol.generation = Some(generation_for_blocking);
                    symbol
                })
                .collect();
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

        let enqueue_started = Instant::now();
        embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
        embeddings_enqueued += results.len() as u64;
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

        let enqueue_started = Instant::now();
        embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
        embeddings_enqueued += ids.len() as u64;
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

    // Final flush of remaining relations after all symbol/chunk/hash commits.
    flush_relation_batches(
        state.as_ref(),
        project_id,
        &status,
        active_structural_generation,
        &symbol_index,
        &mut total_relation_stats,
        &mut relation_create_elapsed_ms,
        &relation_buffer,
        state.config.code_index.relation_batch_size,
    )
    .await;

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

    status.status = if status.total_files == 0 {
        IndexState::Completed
    } else {
        IndexState::EmbeddingPending
    };

    promote_index_generation_and_cleanup(&state, project_id, active_structural_generation).await?;
    let bm25_started = Instant::now();
    status = rebuild_bm25_and_finalize_index_status(
        &state,
        project_id,
        status,
        active_structural_generation,
    )
    .await?;
    let bm25_elapsed = bm25_started.elapsed().as_millis();
    emit_index_timing(
        project_id,
        "bm25_rebuild",
        bm25_elapsed,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    let final_status_elapsed = bm25_elapsed;
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

    if status.status == IndexState::EmbeddingPending {
        tokio::spawn(enqueue_generation_embeddings_after_publish(
            state.clone(),
            project_id.to_string(),
        ));
    }

    let metrics = IndexMetrics {
        file_read_hash_elapsed_ms,
        parse_chunk_elapsed_ms,
        chunk_db_write_elapsed_ms,
        symbol_db_write_elapsed_ms,
        embedding_enqueue_elapsed_ms,
        relation_create_elapsed_ms,
        status_update_elapsed_ms,
        chunks_written,
        symbols_written,
        embeddings_enqueued,
        files_read,
    };
    Ok((status, metrics))
}

async fn read_file_for_staged(
    seq: usize,
    path: std::path::PathBuf,
    project_id: String,
    active_structural_generation: u64,
    read_worker_permits: Arc<tokio::sync::Semaphore>,
    parse_worker_permits: Arc<tokio::sync::Semaphore>,
    file_permits: Arc<tokio::sync::Semaphore>,
    byte_permits: Arc<tokio::sync::Semaphore>,
    max_inflight_bytes: usize,
) -> ParsedFile {
    let _read_worker_permit = match read_worker_permits.acquire_owned().await {
        Ok(permit) => permit,
        Err(e) => {
            return ParsedFile {
                seq,
                path: path.clone(),
                path_str: path.to_string_lossy().to_string(),
                file_hash: None,
                chunks: vec![],
                symbols: vec![],
                references: vec![],
                read_elapsed_ms: 0,
                parse_elapsed_ms: 0,
                error: Some(format!("read worker permit unavailable: {e}")),
                skipped: false,
            };
        }
    };
    let file_permit = match file_permits.acquire_owned().await {
        Ok(permit) => permit,
        Err(e) => {
            return ParsedFile {
                seq,
                path: path.clone(),
                path_str: path.to_string_lossy().to_string(),
                file_hash: None,
                chunks: vec![],
                symbols: vec![],
                references: vec![],
                read_elapsed_ms: 0,
                parse_elapsed_ms: 0,
                error: Some(format!("read file permit unavailable: {e}")),
                skipped: false,
            };
        }
    };
    let _file_permit = file_permit;

    let read_hash_started = Instant::now();
    let path_str = path.to_string_lossy().to_string();
    let size_hint = fs::metadata(&path)
        .await
        .map(|meta| meta.len() as usize)
        .unwrap_or(1)
        .clamp(1, max_inflight_bytes.max(1).min(u32::MAX as usize));
    let byte_permits_for_file = match byte_permits
        .clone()
        .acquire_many_owned(size_hint as u32)
        .await
    {
        Ok(permit) => Some(permit),
        Err(e) => {
            return ParsedFile {
                seq,
                path,
                path_str,
                file_hash: None,
                chunks: vec![],
                symbols: vec![],
                references: vec![],
                read_elapsed_ms: read_hash_started.elapsed().as_millis(),
                parse_elapsed_ms: 0,
                error: Some(format!("read byte permits unavailable: {e}")),
                skipped: false,
            };
        }
    };

    match fs::read_to_string(&path).await {
        Ok(content) => {
            if is_generated_source_content(&content) {
                return ParsedFile {
                    seq,
                    path,
                    path_str,
                    file_hash: None,
                    chunks: vec![],
                    symbols: vec![],
                    references: vec![],
                    read_elapsed_ms: read_hash_started.elapsed().as_millis(),
                    parse_elapsed_ms: 0,
                    error: None,
                    skipped: true,
                };
            }
            if content.len() > 1_000_000 {
                return ParsedFile {
                    seq,
                    path,
                    path_str,
                    file_hash: None,
                    chunks: vec![],
                    symbols: vec![],
                    references: vec![],
                    read_elapsed_ms: read_hash_started.elapsed().as_millis(),
                    parse_elapsed_ms: 0,
                    error: None,
                    skipped: true,
                };
            }
            let file_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
            parse_file_for_staged(
                seq,
                path,
                path_str,
                project_id,
                active_structural_generation,
                content,
                file_hash,
                read_hash_started.elapsed().as_millis(),
                byte_permits_for_file,
                parse_worker_permits,
            )
            .await
        }
        Err(e) => ParsedFile {
            seq,
            path,
            path_str,
            file_hash: None,
            chunks: vec![],
            symbols: vec![],
            references: vec![],
            read_elapsed_ms: read_hash_started.elapsed().as_millis(),
            parse_elapsed_ms: 0,
            error: Some(e.to_string()),
            skipped: false,
        },
    }
}

async fn parse_file_for_staged(
    seq: usize,
    path: std::path::PathBuf,
    path_str: String,
    project_id: String,
    active_structural_generation: u64,
    content: String,
    file_hash: String,
    read_elapsed_ms: u128,
    byte_permits: Option<tokio::sync::OwnedSemaphorePermit>,
    parse_worker_permits: Arc<tokio::sync::Semaphore>,
) -> ParsedFile {
    let _parse_worker_permit = match parse_worker_permits.acquire_owned().await {
        Ok(permit) => permit,
        Err(e) => {
            return ParsedFile {
                seq,
                path,
                path_str,
                file_hash: None,
                chunks: vec![],
                symbols: vec![],
                references: vec![],
                read_elapsed_ms,
                parse_elapsed_ms: 0,
                error: Some(format!("parse worker permit unavailable: {e}")),
                skipped: false,
            };
        }
    };
    let parse_path = path.clone();
    let join = tokio::task::spawn_blocking(move || {
        let _byte_permits = byte_permits;
        let parse_started = Instant::now();
        let chunks = chunk_file_for_generation(
            &parse_path,
            &content,
            &project_id,
            Some(active_structural_generation),
        );
        let (symbols, references) = CodeParser::parse_file(&parse_path, &content, &project_id);
        let symbols = symbols
            .into_iter()
            .map(|mut symbol| {
                symbol.generation = Some(active_structural_generation);
                symbol
            })
            .collect();
        (
            chunks,
            symbols,
            references,
            parse_started.elapsed().as_millis(),
        )
    });

    match tokio::time::timeout(Duration::from_secs(PARSE_TIMEOUT_SECS), join).await {
        Ok(Ok((chunks, symbols, references, parse_elapsed_ms))) => ParsedFile {
            seq,
            path,
            path_str,
            file_hash: Some(file_hash),
            chunks,
            symbols,
            references,
            read_elapsed_ms,
            parse_elapsed_ms,
            error: None,
            skipped: false,
        },
        Ok(Err(e)) => ParsedFile {
            seq,
            path,
            path_str,
            file_hash: None,
            chunks: vec![],
            symbols: vec![],
            references: vec![],
            read_elapsed_ms,
            parse_elapsed_ms: 0,
            error: Some(format!("parse/chunk panicked: {e}")),
            skipped: false,
        },
        Err(_) => ParsedFile {
            seq,
            path,
            path_str,
            file_hash: None,
            chunks: vec![],
            symbols: vec![],
            references: vec![],
            read_elapsed_ms,
            parse_elapsed_ms: 0,
            error: Some(format!("parse_timeout_{}s", PARSE_TIMEOUT_SECS)),
            skipped: false,
        },
    }
}

async fn flush_staged_chunks(
    state: &Arc<AppState>,
    project_id: &str,
    status: &IndexStatus,
    total_relation_stats: &RelationStats,
    metrics: &mut IndexMetrics,
    batch: Vec<CodeChunk>,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let batch_len = batch.len() as u64;
    let _permit = state.db_semaphore.acquire().await;
    let chunk_write_started = Instant::now();
    let results = state
        .storage
        .create_code_chunks_batch(batch)
        .await
        .map_err(|e| {
            crate::AppError::Internal(
                format!("failed to persist staged code chunks for project {project_id}: {e}")
                    .into(),
            )
        })?;
    let chunk_write_elapsed = chunk_write_started.elapsed().as_millis();
    metrics.chunk_db_write_elapsed_ms += chunk_write_elapsed;
    metrics.chunks_written += batch_len;
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

    let enqueue_started = Instant::now();
    metrics.embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
    metrics.embeddings_enqueued += results.len() as u64;

    Ok(())
}

async fn flush_staged_symbols(
    state: &Arc<AppState>,
    project_id: &str,
    status: &IndexStatus,
    total_relation_stats: &RelationStats,
    metrics: &mut IndexMetrics,
    batch: Vec<CodeSymbol>,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let batch_len = batch.len() as u64;
    let _permit = state.db_semaphore.acquire().await;
    let symbol_write_started = Instant::now();
    let ids = state
        .storage
        .create_code_symbols_batch(batch.clone())
        .await?;
    let symbol_write_elapsed = symbol_write_started.elapsed().as_millis();
    metrics.symbol_db_write_elapsed_ms += symbol_write_elapsed;
    metrics.symbols_written += batch_len;
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

    let enqueue_started = Instant::now();
    metrics.embedding_enqueue_elapsed_ms += enqueue_started.elapsed().as_millis();
    metrics.embeddings_enqueued += ids.len() as u64;

    Ok(())
}

async fn flush_relation_batches(
    state: &AppState,
    project_id: &str,
    status: &IndexStatus,
    active_structural_generation: u64,
    symbol_index: &SymbolIndex,
    total_relation_stats: &mut RelationStats,
    relation_create_elapsed_ms: &mut u128,
    batch: &[CodeReference],
    relation_batch_size: usize,
) {
    let relation_batch_size = relation_batch_size.max(1);
    for relation_batch in batch.chunks(relation_batch_size) {
        let relation_started = Instant::now();
        let stats = create_symbol_relations_for_generation(
            state.storage.as_ref(),
            project_id,
            relation_batch,
            symbol_index,
            active_structural_generation,
        )
        .await;
        total_relation_stats.created += stats.created;
        total_relation_stats.failed += stats.failed;
        total_relation_stats.unresolved += stats.unresolved;
        *relation_create_elapsed_ms += relation_started.elapsed().as_millis();
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
}

async fn flush_staged_relations(
    state: &Arc<AppState>,
    project_id: &str,
    status: &IndexStatus,
    active_structural_generation: u64,
    symbol_index: &SymbolIndex,
    total_relation_stats: &mut RelationStats,
    metrics: &mut IndexMetrics,
    batch: &[CodeReference],
    relation_batch_size: usize,
) {
    flush_relation_batches(
        state.as_ref(),
        project_id,
        status,
        active_structural_generation,
        symbol_index,
        total_relation_stats,
        &mut metrics.relation_create_elapsed_ms,
        batch,
        relation_batch_size,
    )
    .await;
}

async fn finish_full_index(
    state: Arc<AppState>,
    project_id: &str,
    mut status: IndexStatus,
    active_structural_generation: u64,
    total_started: Instant,
    total_relation_stats: RelationStats,
    mut metrics: IndexMetrics,
) -> Result<(IndexStatus, IndexMetrics)> {
    emit_index_timing(
        project_id,
        "embedding_enqueue",
        metrics.embedding_enqueue_elapsed_ms,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        metrics.embeddings_enqueued,
        status.failed_files.len() as u64,
    );

    if total_relation_stats.created > 0 || total_relation_stats.failed > 0 {
        tracing::info!(
            created = total_relation_stats.created,
            failed = total_relation_stats.failed,
            unresolved = total_relation_stats.unresolved,
            "Symbol relations indexed"
        );
    }

    tracing::info!(project_id = %project_id, phase = "projection_refresh_started", "Projection/materialized read-model refresh started");

    status.status = if status.total_files == 0 {
        IndexState::Completed
    } else {
        IndexState::EmbeddingPending
    };

    promote_index_generation_and_cleanup(&state, project_id, active_structural_generation).await?;
    let bm25_started = Instant::now();
    status = rebuild_bm25_and_finalize_index_status(
        &state,
        project_id,
        status,
        active_structural_generation,
    )
    .await?;
    let bm25_elapsed = bm25_started.elapsed().as_millis();
    let final_status_elapsed = bm25_elapsed;
    let status_update_elapsed_ms = metrics.status_update_elapsed_ms + final_status_elapsed;
    emit_index_timing(
        project_id,
        "bm25_rebuild",
        bm25_elapsed,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
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

    emit_index_timing(
        project_id,
        "file_read_hash_total",
        metrics.file_read_hash_elapsed_ms,
        metrics.files_read,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "parse_chunk_total",
        metrics.parse_chunk_elapsed_ms,
        status.indexed_files as u64,
        status.total_chunks as u64,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "chunk_db_write_total",
        metrics.chunk_db_write_elapsed_ms,
        status.indexed_files as u64,
        metrics.chunks_written,
        status.total_symbols as u64,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "symbol_db_write_total",
        metrics.symbol_db_write_elapsed_ms,
        status.indexed_files as u64,
        status.total_chunks as u64,
        metrics.symbols_written,
        (total_relation_stats.created + total_relation_stats.failed) as u64,
        status.failed_files.len() as u64,
    );
    emit_index_timing(
        project_id,
        "relation_create_total",
        metrics.relation_create_elapsed_ms,
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

    if status.status == IndexState::EmbeddingPending {
        tokio::spawn(enqueue_generation_embeddings_after_publish(
            state.clone(),
            project_id.to_string(),
        ));
    }

    metrics.status_update_elapsed_ms = status_update_elapsed_ms;
    Ok((status, metrics))
}

async fn run_staged_index_pipeline(
    state: Arc<AppState>,
    _project_path: &Path,
    project_id: &str,
    mut status: IndexStatus,
    active_structural_generation: u64,
    total_started: Instant,
    files: Vec<std::path::PathBuf>,
    checkpoint_context: Option<FileCheckpointContext>,
) -> Result<(IndexStatus, IndexMetrics)> {
    let config = state.config.code_index.clone();
    let monitor = state.progress.get_or_create(project_id).await;
    let mut metrics = IndexMetrics::default();
    let mut symbol_index = SymbolIndex::new();
    let mut total_relation_stats = RelationStats::default();
    let mut pending: BTreeMap<usize, ParsedFile> = BTreeMap::new();
    let mut next_commit_seq = 0usize;
    let mut chunk_buffer = Vec::with_capacity(config.commit_batch_size);
    let mut symbol_buffer = Vec::with_capacity(config.commit_batch_size);
    let mut hash_buffer: Vec<(String, String)> = Vec::with_capacity(config.commit_batch_size);
    let mut relation_buffer: Vec<CodeReference> = Vec::with_capacity(config.relation_batch_size);
    let mut last_status_flush = Instant::now();

    tracing::info!(
        project_id = %project_id,
        phase = "staged_pipeline_started",
        read_workers = config.read_workers,
        parse_workers = config.parse_workers,
        commit_batch_size = config.commit_batch_size,
        max_inflight_files = config.max_inflight_files,
        max_inflight_bytes = config.max_inflight_bytes,
        relation_batch_size = config.relation_batch_size,
        "Staged code index pipeline started"
    );
    tracing::info!(project_id = %project_id, phase = "parsing_chunking_started", total_files = status.total_files, "Parsing and chunking started");

    let read_worker_permits = Arc::new(tokio::sync::Semaphore::new(config.read_workers));
    let parse_worker_permits = Arc::new(tokio::sync::Semaphore::new(config.parse_workers));
    let file_permits = Arc::new(tokio::sync::Semaphore::new(config.max_inflight_files));
    let byte_permits = Arc::new(tokio::sync::Semaphore::new(
        config.max_inflight_bytes.min(u32::MAX as usize),
    ));
    let mut parse_set: tokio::task::JoinSet<ParsedFile> = tokio::task::JoinSet::new();

    for (seq, file_path) in files.iter().cloned().enumerate() {
        if checkpoint_context.as_ref().is_some_and(|context| {
            context
                .completed_relative_paths
                .contains(&checkpoint_relative_path(&file_path))
        }) {
            pending.insert(
                seq,
                ParsedFile {
                    seq,
                    path: file_path.clone(),
                    path_str: file_path.to_string_lossy().to_string(),
                    file_hash: None,
                    chunks: vec![],
                    symbols: vec![],
                    references: vec![],
                    read_elapsed_ms: 0,
                    parse_elapsed_ms: 0,
                    error: None,
                    skipped: true,
                },
            );
            drain_ready_staged_results(
                &state,
                project_id,
                &monitor,
                &mut status,
                &mut metrics,
                &mut symbol_index,
                &mut total_relation_stats,
                &mut pending,
                &mut next_commit_seq,
                &mut chunk_buffer,
                &mut symbol_buffer,
                &mut hash_buffer,
                &mut relation_buffer,
                &config,
                &mut last_status_flush,
                checkpoint_context.as_ref(),
            )
            .await?;
            continue;
        }

        // Check for cancellation request at per-file boundary (every 10 files to reduce DB load)
        if seq % 10 == 0 {
            if let Ok(jobs) = state.storage.list_index_jobs_for_project(project_id).await {
                if jobs
                    .iter()
                    .any(|j| j.state == IndexJobState::CancelRequested)
                {
                    tracing::info!(project_id = %project_id, seq = seq, "Cancellation requested — stopping indexer");
                    return Err(crate::AppError::Indexing(
                        "indexing cancelled by request".into(),
                    ));
                }
            }
        }

        if let Ok(mut cf) = monitor.current_file.write() {
            *cf = file_path.to_string_lossy().to_string();
        }

        tracing::info!("Indexing file: {:?}", file_path);

        if crate::codebase::scanner::is_ignored_file(&file_path) {
            tracing::debug!(path = ?file_path, "Skipping generated file");
            pending.insert(
                seq,
                ParsedFile {
                    seq,
                    path: file_path.clone(),
                    path_str: file_path.to_string_lossy().to_string(),
                    file_hash: None,
                    chunks: vec![],
                    symbols: vec![],
                    references: vec![],
                    read_elapsed_ms: 0,
                    parse_elapsed_ms: 0,
                    error: None,
                    skipped: true,
                },
            );
            drain_ready_staged_results(
                &state,
                project_id,
                &monitor,
                &mut status,
                &mut metrics,
                &mut symbol_index,
                &mut total_relation_stats,
                &mut pending,
                &mut next_commit_seq,
                &mut chunk_buffer,
                &mut symbol_buffer,
                &mut hash_buffer,
                &mut relation_buffer,
                &config,
                &mut last_status_flush,
                checkpoint_context.as_ref(),
            )
            .await?;
            continue;
        }

        parse_set.spawn(read_file_for_staged(
            seq,
            file_path,
            project_id.to_string(),
            active_structural_generation,
            read_worker_permits.clone(),
            parse_worker_permits.clone(),
            file_permits.clone(),
            byte_permits.clone(),
            config.max_inflight_bytes,
        ));

        while parse_set.len() >= config.max_inflight_files {
            if let Some(join_result) = parse_set.join_next().await {
                let parsed = join_result.map_err(|e| {
                    crate::AppError::Internal(format!("staged parse task panicked: {e}").into())
                })?;
                pending.insert(parsed.seq, parsed);
                drain_ready_staged_results(
                    &state,
                    project_id,
                    &monitor,
                    &mut status,
                    &mut metrics,
                    &mut symbol_index,
                    &mut total_relation_stats,
                    &mut pending,
                    &mut next_commit_seq,
                    &mut chunk_buffer,
                    &mut symbol_buffer,
                    &mut hash_buffer,
                    &mut relation_buffer,
                    &config,
                    &mut last_status_flush,
                    checkpoint_context.as_ref(),
                )
                .await?;
            }
        }
    }

    while let Some(join_result) = parse_set.join_next().await {
        let parsed = join_result.map_err(|e| {
            crate::AppError::Internal(format!("staged parse task panicked: {e}").into())
        })?;
        pending.insert(parsed.seq, parsed);
        drain_ready_staged_results(
            &state,
            project_id,
            &monitor,
            &mut status,
            &mut metrics,
            &mut symbol_index,
            &mut total_relation_stats,
            &mut pending,
            &mut next_commit_seq,
            &mut chunk_buffer,
            &mut symbol_buffer,
            &mut hash_buffer,
            &mut relation_buffer,
            &config,
            &mut last_status_flush,
            checkpoint_context.as_ref(),
        )
        .await?;
    }

    flush_staged_chunks(
        &state,
        project_id,
        &status,
        &total_relation_stats,
        &mut metrics,
        std::mem::take(&mut chunk_buffer),
    )
    .await?;
    flush_staged_symbols(
        &state,
        project_id,
        &status,
        &total_relation_stats,
        &mut metrics,
        std::mem::take(&mut symbol_buffer),
    )
    .await?;
    if !hash_buffer.is_empty() {
        let _ = state
            .storage
            .set_file_hashes_batch(project_id, &hash_buffer)
            .await;
    }
    flush_staged_relations(
        &state,
        project_id,
        &status,
        active_structural_generation,
        &symbol_index,
        &mut total_relation_stats,
        &mut metrics,
        &relation_buffer,
        config.relation_batch_size,
    )
    .await;

    finish_full_index(
        state,
        project_id,
        status,
        active_structural_generation,
        total_started,
        total_relation_stats,
        metrics,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn drain_ready_staged_results(
    state: &Arc<AppState>,
    project_id: &str,
    monitor: &Arc<crate::config::IndexMonitor>,
    status: &mut IndexStatus,
    metrics: &mut IndexMetrics,
    symbol_index: &mut SymbolIndex,
    total_relation_stats: &mut RelationStats,
    pending: &mut BTreeMap<usize, ParsedFile>,
    next_commit_seq: &mut usize,
    chunk_buffer: &mut Vec<CodeChunk>,
    symbol_buffer: &mut Vec<CodeSymbol>,
    hash_buffer: &mut Vec<(String, String)>,
    relation_buffer: &mut Vec<CodeReference>,
    config: &crate::config::CodeIndexConfig,
    last_status_flush: &mut Instant,
    checkpoint_context: Option<&FileCheckpointContext>,
) -> Result<()> {
    while let Some(parsed) = pending.remove(next_commit_seq) {
        metrics.file_read_hash_elapsed_ms += parsed.read_elapsed_ms;
        emit_index_timing(
            project_id,
            "file_read_hash",
            parsed.read_elapsed_ms,
            metrics.files_read,
            status.total_chunks as u64,
            status.total_symbols as u64,
            (total_relation_stats.created + total_relation_stats.failed) as u64,
            status.failed_files.len() as u64,
        );

        if let Some(error) = parsed.error {
            tracing::warn!(path = ?parsed.path, error = %error, "Failed to index staged file");
            status.failed_files.push(parsed.path_str);
            status.indexed_files += 1;
            monitor
                .indexed_files
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            upsert_file_checkpoint_after_file(
                state,
                checkpoint_context,
                project_id,
                &parsed.path,
                None,
                IndexJobPhase::Parse,
                0,
                0,
            )
            .await?;
            *next_commit_seq += 1;
            continue;
        }

        if parsed.skipped {
            tracing::debug!(path = ?parsed.path, "Skipping staged file without failure");
            status.indexed_files += 1;
            monitor
                .indexed_files
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            upsert_file_checkpoint_after_file(
                state,
                checkpoint_context,
                project_id,
                &parsed.path,
                parsed.file_hash.clone(),
                IndexJobPhase::Parse,
                0,
                0,
            )
            .await?;
            *next_commit_seq += 1;
            continue;
        }

        metrics.files_read += 1;
        metrics.parse_chunk_elapsed_ms += parsed.parse_elapsed_ms;
        emit_index_timing(
            project_id,
            "parse_chunk",
            parsed.parse_elapsed_ms,
            1,
            parsed.chunks.len() as u64,
            parsed.symbols.len() as u64,
            parsed.references.len() as u64,
            status.failed_files.len() as u64,
        );

        let checkpoint_content_hash = parsed.file_hash.clone();
        let file_chunk_count = parsed.chunks.len() as u64;
        let file_symbol_count = parsed.symbols.len() as u64;

        if let Some(file_hash) = parsed.file_hash {
            hash_buffer.push((parsed.path_str.clone(), file_hash));
            if hash_buffer.len() >= config.commit_batch_size {
                let batch = std::mem::take(hash_buffer);
                let _ = state
                    .storage
                    .set_file_hashes_batch(project_id, &batch)
                    .await;
            }
        }

        for chunk in parsed.chunks {
            chunk_buffer.push(chunk);
            status.total_chunks += 1;
            if chunk_buffer.len() >= config.commit_batch_size {
                flush_staged_chunks(
                    state,
                    project_id,
                    status,
                    total_relation_stats,
                    metrics,
                    std::mem::take(chunk_buffer),
                )
                .await?;
            }
        }

        for symbol in &parsed.symbols {
            symbol_index.add(symbol);
        }
        let containment_refs = detect_containment_references(&parsed.symbols);

        for symbol in parsed.symbols {
            symbol_buffer.push(symbol);
            status.total_symbols += 1;
            if symbol_buffer.len() >= config.commit_batch_size {
                flush_staged_symbols(
                    state,
                    project_id,
                    status,
                    total_relation_stats,
                    metrics,
                    std::mem::take(symbol_buffer),
                )
                .await?;
            }
        }

        relation_buffer.extend(parsed.references);
        relation_buffer.extend(containment_refs);

        status.indexed_files += 1;
        monitor
            .indexed_files
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        upsert_file_checkpoint_after_file(
            state,
            checkpoint_context,
            project_id,
            &parsed.path,
            checkpoint_content_hash,
            IndexJobPhase::Parse,
            file_chunk_count,
            file_symbol_count,
        )
        .await?;

        if last_status_flush.elapsed() >= Duration::from_millis(config.status_flush_ms) {
            let status_started = Instant::now();
            if let Err(e) = state.storage.update_index_status(status.clone()).await {
                tracing::warn!("Failed to update intermediate status: {}", e);
            }
            metrics.status_update_elapsed_ms += status_started.elapsed().as_millis();
            *last_status_flush = Instant::now();
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

        *next_commit_seq += 1;
    }

    Ok(())
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
    let active_structural_generation = state.storage.get_active_generation(project_id).await?;
    // Keep a local alias for the previous `updated` counter used inside the macro.
    macro_rules! inc_updated {
        () => {
            result.updated_files += 1;
        };
    }

    // Issue 4 fix: Bounded-concurrency parsing via JoinSet (same pattern as do_index_project).
    let max_concurrent_parses = effective_parse_workers(&state.config.code_index);
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
                    .try_send(code_chunk_embedding_request(id, chunk.content));
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
                    let text = sym
                        .signature
                        .clone()
                        .unwrap_or_else(|| format!("{} {}", sym.symbol_type, sym.name));
                    let _ = state.embedding_queue.try_send(EmbeddingRequest {
                        text,
                        responder: None,
                        target: Some(EmbeddingTarget::Symbol(id.clone())),
                        retry_count: 0,
                    });
                }
            }

            // Create relations using project-wide symbol index for cross-file resolution
            // Also detect containment edges from symbol nesting within this file.
            let containment_refs = detect_containment_references(&symbols);
            let mut all_refs = references;
            all_refs.extend(containment_refs);

            if !all_refs.is_empty() {
                let mut symbol_index = SymbolIndex::new();
                if let Ok(all_symbols) = state
                    .storage
                    .get_project_symbols(project_id, active_structural_generation)
                    .await
                {
                    symbol_index.add_batch(&all_symbols);
                }
                symbol_index.add_batch(&symbols);
                let _stats = if let Some(generation) = active_structural_generation {
                    create_symbol_relations_for_generation(
                        state.storage.as_ref(),
                        project_id,
                        &all_refs,
                        &symbol_index,
                        generation,
                    )
                    .await
                } else {
                    create_symbol_relations(
                        state.storage.as_ref(),
                        project_id,
                        &all_refs,
                        &symbol_index,
                    )
                    .await
                };
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

        if is_generated_source_content(&content) {
            let _ = state
                .storage
                .delete_chunks_by_path(project_id, &path_str)
                .await;
            let _ = state
                .storage
                .delete_symbols_by_path(project_id, &path_str)
                .await;
            let _ = state.storage.delete_file_hash(project_id, &path_str).await;
            result.deleted_files.push(path_str);
            continue;
        }

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
        let generation_for_blocking = active_structural_generation;
        parse_set.spawn_blocking(move || {
            let chunks = super::chunker::chunk_file_for_generation(
                &path_for_blocking,
                &content,
                &project_id_for_blocking,
                generation_for_blocking,
            );
            let (symbols, references) =
                CodeParser::parse_file(&path_for_blocking, &content, &project_id_for_blocking);
            let symbols = symbols
                .into_iter()
                .map(|mut symbol| {
                    symbol.generation = generation_for_blocking;
                    symbol
                })
                .collect();
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
    use crate::config::{CodeIndexConfig, CodeIndexPipelineMode};
    use crate::test_utils::TestContext;
    use crate::types::{ChunkType, CodeChunk, CodeSymbol, Language, SymbolType};
    use std::collections::HashSet;
    use std::fs;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex, OnceLock};
    use tokio::time::{sleep, Duration};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;
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
    async fn index_project_rejects_same_project_duplicate_before_cleanup() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("duplicate-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("lib.rs"), "fn original() {}\n").unwrap();

        let mut previous = IndexStatus::new("duplicate-project".to_string());
        previous.status = IndexState::Completed;
        previous.total_files = 1;
        previous.indexed_files = 1;
        previous.total_chunks = 7;
        previous.total_symbols = 3;
        previous.mark_structural_generation_advanced();
        previous.mark_semantic_generation_caught_up();
        ctx.state
            .storage
            .update_index_status(previous)
            .await
            .unwrap();
        ctx.state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned")
            .insert("duplicate-project".to_string());

        let error = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("already_running"),
            "expected already_running duplicate guard, got {error}"
        );
        let stored = ctx
            .state
            .storage
            .get_index_status("duplicate-project")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, IndexState::Completed);
        assert_eq!(stored.total_chunks, 7);
        assert_eq!(stored.total_symbols, 3);
        assert_eq!(stored.structural_generation, 1);
    }

    #[tokio::test]
    async fn duplicate_same_project_request_keeps_existing_rows_intact() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("duplicate-row-preservation");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("lib.rs"), "fn incoming() {}\n").unwrap();
        let project_id = derive_project_id(&project_dir).unwrap();

        let seeded_chunk = CodeChunk {
            id: None,
            file_path: project_dir.join("seed.rs").to_string_lossy().to_string(),
            content: "fn seed_chunk() {}".to_string(),
            language: Language::Rust,
            start_line: 1,
            end_line: 1,
            chunk_type: ChunkType::Function,
            name: Some("seed_chunk".to_string()),
            context_path: None,
            embedding: None,
            content_hash: blake3::hash(b"seed_chunk").to_hex().to_string(),
            project_id: Some(project_id.clone()),
            generation: None,
            indexed_at: crate::types::Datetime::default(),
        };
        let seeded_symbol = CodeSymbol::new(
            "seed_symbol".to_string(),
            SymbolType::Function,
            project_dir.join("seed.rs").to_string_lossy().to_string(),
            1,
            1,
            project_id.clone(),
        );
        ctx.state
            .storage
            .create_code_chunks_batch(vec![seeded_chunk])
            .await
            .unwrap();
        ctx.state
            .storage
            .create_code_symbols_batch(vec![seeded_symbol])
            .await
            .unwrap();

        let mut previous = IndexStatus::new(project_id.clone());
        previous.status = IndexState::Completed;
        previous.total_files = 1;
        previous.indexed_files = 1;
        previous.total_chunks = 1;
        previous.total_symbols = 1;
        previous.mark_structural_generation_advanced();
        previous.mark_semantic_generation_caught_up();
        ctx.state
            .storage
            .update_index_status(previous)
            .await
            .unwrap();
        ctx.state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned")
            .insert(project_id.clone());

        let before_chunks = ctx
            .state
            .storage
            .count_chunks(&project_id, None)
            .await
            .unwrap();
        let before_symbols = ctx
            .state
            .storage
            .count_symbols(&project_id, None)
            .await
            .unwrap();

        let error = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already_running"));

        let after_chunks = ctx
            .state
            .storage
            .count_chunks(&project_id, None)
            .await
            .unwrap();
        let after_symbols = ctx
            .state
            .storage
            .count_symbols(&project_id, None)
            .await
            .unwrap();
        assert_eq!(before_chunks, after_chunks);
        assert_eq!(before_symbols, after_symbols);
    }

    #[tokio::test]
    async fn finalize_index_status_rejects_stale_generation() {
        let ctx = TestContext::new().await;
        let project_id = "stale-generation-project";

        let mut newer = IndexStatus::new(project_id.to_string());
        newer.structural_generation = 2;
        newer.status = IndexState::Indexing;
        newer.refresh_lifecycle_states();
        ctx.state.storage.update_index_status(newer).await.unwrap();

        let mut stale = IndexStatus::new(project_id.to_string());
        stale.structural_generation = 1;
        stale.status = IndexState::Completed;
        stale.total_files = 0;

        let error = finalize_index_status_if_current(&ctx.state, stale, 1)
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("stale_generation"),
            "expected stale_generation rejection, got {error}"
        );
        let stored = ctx
            .state
            .storage
            .get_index_status(project_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, IndexState::Indexing);
        assert_eq!(stored.structural_generation, 2);
        assert_eq!(stored.semantic_generation, 0);
    }

    #[tokio::test]
    async fn stale_generation_rejection_is_cancellation_safety_proxy() {
        let ctx = TestContext::new().await;
        let project_id = "cancellation-proxy-project";

        let mut newer = IndexStatus::new(project_id.to_string());
        newer.status = IndexState::Indexing;
        newer.structural_generation = 5;
        newer.refresh_lifecycle_states();
        ctx.state.storage.update_index_status(newer).await.unwrap();

        let stale_candidate = IndexStatus {
            project_id: project_id.to_string(),
            status: IndexState::Completed,
            structural_generation: 4,
            semantic_generation: 4,
            total_files: 2,
            indexed_files: 2,
            total_chunks: 2,
            total_symbols: 2,
            ..IndexStatus::new(project_id.to_string())
        };

        let error = finalize_index_status_if_current(&ctx.state, stale_candidate, 4)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("stale_generation"));

        let stored = ctx
            .state
            .storage
            .get_index_status(project_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, IndexState::Indexing);
        assert_eq!(stored.structural_generation, 5);
        assert_ne!(stored.status, IndexState::Completed);
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
            .get_project_symbols(&status.project_id, None)
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
            "promote",
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

    fn staged_config_for_test() -> CodeIndexConfig {
        CodeIndexConfig {
            pipeline_mode: CodeIndexPipelineMode::Staged,
            read_workers: 2,
            parse_workers: 2,
            commit_batch_size: 2,
            max_inflight_files: 2,
            max_inflight_bytes: 1024,
            status_flush_ms: 1,
            relation_batch_size: 2,
            bm25_mode: crate::config::CodeIndexBm25Mode::FinalRebuild,
            include_patterns: vec![],
            exclude_patterns: vec![],
        }
    }

    #[test]
    fn effective_parse_workers_uses_config_and_clamps_low_values() {
        let mut config = CodeIndexConfig::default();

        config.parse_workers = 6;
        assert_eq!(effective_parse_workers(&config), 6);

        config.parse_workers = 1;
        assert_eq!(effective_parse_workers(&config), 2);
    }

    fn sorted_chunk_fingerprint(chunks: Vec<CodeChunk>) -> Vec<(String, u32, u32, String)> {
        let mut rows: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                (
                    stable_fixture_path(&chunk.file_path),
                    chunk.start_line,
                    chunk.end_line,
                    chunk.content_hash,
                )
            })
            .collect();
        rows.sort();
        rows
    }

    fn sorted_symbol_fingerprint(symbols: Vec<CodeSymbol>) -> Vec<(String, u32, String, String)> {
        let mut rows: Vec<_> = symbols
            .into_iter()
            .map(|symbol| {
                (
                    stable_fixture_path(&symbol.file_path),
                    symbol.start_line,
                    symbol.name,
                    symbol.symbol_type.to_string(),
                )
            })
            .collect();
        rows.sort();
        rows
    }

    fn stable_fixture_path(path: &str) -> String {
        path.split("fixture-equivalence/")
            .nth(1)
            .unwrap_or(path)
            .to_string()
    }

    fn active_generation_for_compat(status: &IndexStatus) -> Option<u64> {
        if status.structural_generation == 0 {
            None
        } else {
            Some(status.structural_generation)
        }
    }

    fn write_equivalence_fixture(project_dir: &Path) {
        let files = [
            (
                "src/lib.rs",
                "pub mod utils;\npub fn alpha() -> i32 { utils::beta() }\n",
            ),
            (
                "src/utils.rs",
                "pub fn beta() -> i32 { 7 }\nfn hidden() {}\n",
            ),
            ("src/model.rs", "pub struct Model { pub value: i32 }\n"),
        ];

        for (relative_path, content) in files {
            let path = project_dir.join(relative_path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
    }

    #[tokio::test]
    async fn staged_pipeline_matches_legacy_fixture_outputs() {
        let legacy_ctx = TestContext::new().await;
        let staged_ctx = TestContext::new_with_code_index_config(staged_config_for_test()).await;
        let legacy_project_dir = legacy_ctx._temp_dir.path().join("fixture-equivalence");
        let staged_project_dir = staged_ctx._temp_dir.path().join("fixture-equivalence");
        write_equivalence_fixture(&legacy_project_dir);
        write_equivalence_fixture(&staged_project_dir);

        let legacy_status = index_project(legacy_ctx.state.clone(), &legacy_project_dir)
            .await
            .unwrap();
        let staged_status = index_project(staged_ctx.state.clone(), &staged_project_dir)
            .await
            .unwrap();

        assert_eq!(legacy_status.status, staged_status.status);
        assert_eq!(legacy_status.total_files, staged_status.total_files);
        assert_eq!(legacy_status.indexed_files, staged_status.indexed_files);
        assert_eq!(legacy_status.total_chunks, staged_status.total_chunks);
        assert_eq!(legacy_status.total_symbols, staged_status.total_symbols);
        assert_eq!(legacy_status.failed_files, staged_status.failed_files);

        let legacy_chunks = legacy_ctx
            .state
            .storage
            .get_all_chunks_for_project(
                &legacy_status.project_id,
                active_generation_for_compat(&legacy_status),
            )
            .await
            .unwrap();
        let staged_chunks = staged_ctx
            .state
            .storage
            .get_all_chunks_for_project(
                &staged_status.project_id,
                active_generation_for_compat(&staged_status),
            )
            .await
            .unwrap();
        assert_eq!(
            sorted_chunk_fingerprint(legacy_chunks),
            sorted_chunk_fingerprint(staged_chunks)
        );

        let legacy_symbols = legacy_ctx
            .state
            .storage
            .get_project_symbols(
                &legacy_status.project_id,
                active_generation_for_compat(&legacy_status),
            )
            .await
            .unwrap();
        let staged_symbols = staged_ctx
            .state
            .storage
            .get_project_symbols(
                &staged_status.project_id,
                active_generation_for_compat(&staged_status),
            )
            .await
            .unwrap();
        assert_eq!(
            sorted_symbol_fingerprint(legacy_symbols),
            sorted_symbol_fingerprint(staged_symbols)
        );

        assert_eq!(
            legacy_ctx
                .state
                .storage
                .count_symbol_relations(&legacy_status.project_id)
                .await
                .unwrap(),
            staged_ctx
                .state
                .storage
                .count_symbol_relations(&staged_status.project_id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn staged_pipeline_creates_cross_file_relations_after_all_symbol_commits() {
        let mut config = staged_config_for_test();
        config.commit_batch_size = 1;
        config.relation_batch_size = 1;
        config.max_inflight_files = 1;
        let ctx = TestContext::new_with_code_index_config(config).await;
        let project_dir = ctx._temp_dir.path().join("staged-cross-file-relations");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join("a.rs"),
            "pub fn alpha() -> i32 { beta() + gamma() }\n",
        )
        .unwrap();
        std::fs::write(project_dir.join("b.rs"), "pub fn beta() -> i32 { 7 }\n").unwrap();
        std::fs::write(project_dir.join("c.rs"), "pub fn gamma() -> i32 { 11 }\n").unwrap();

        let project_id = derive_project_id(&project_dir).unwrap();
        let mut status =
            prepare_started_index_status(&project_id, &project_dir, None, vec![], vec![]);
        status.total_files = 3;
        let active_structural_generation = status.structural_generation;
        ctx.state
            .storage
            .update_index_status(status.clone())
            .await
            .unwrap();

        let (status, _metrics) = run_staged_index_pipeline(
            ctx.state.clone(),
            &project_dir,
            &project_id,
            status,
            active_structural_generation,
            Instant::now(),
            vec![
                project_dir.join("a.rs"),
                project_dir.join("b.rs"),
                project_dir.join("c.rs"),
            ],
            None,
        )
        .await
        .unwrap();

        assert_eq!(status.status, IndexState::EmbeddingPending);
        assert_eq!(
            ctx.state
                .storage
                .count_symbol_relations(&status.project_id)
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn bm25_rebuild_failure_records_failed_status_for_active_generation() {
        let ctx = TestContext::new().await;
        let project_id = "bm25-failure-project";

        let mut active = IndexStatus::new(project_id.to_string());
        active.structural_generation = 1;
        active.status = IndexState::Indexing;
        active.refresh_lifecycle_states();
        ctx.state.storage.update_index_status(active).await.unwrap();

        let mut pending = IndexStatus::new(project_id.to_string());
        pending.structural_generation = 1;
        pending.status = IndexState::EmbeddingPending;
        pending.total_files = 1;
        pending.indexed_files = 1;
        pending.total_chunks = 1;

        let error = record_bm25_rebuild_failure_if_current(
            &ctx.state,
            project_id,
            pending,
            1,
            "forced rebuild error",
        )
        .await;

        assert!(
            error.to_string().contains("bm25_rebuild_failed"),
            "expected deterministic bm25_rebuild_failed error, got {error}"
        );
        let stored = ctx
            .state
            .storage
            .get_index_status(project_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, IndexState::Failed);
        assert_eq!(stored.structural_generation, 1);
        assert_eq!(stored.semantic_generation, 0);
        assert_eq!(
            stored.structural_state,
            crate::types::StructuralState::Failed
        );
        assert!(stored
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("bm25_rebuild_failed"));
    }

    #[tokio::test]
    async fn staged_pipeline_respects_backpressure_config_and_completes() {
        let mut config = staged_config_for_test();
        config.read_workers = 1;
        config.parse_workers = 1;
        config.commit_batch_size = 1;
        config.max_inflight_files = 1;
        config.max_inflight_bytes = 64;
        config.relation_batch_size = 1;
        let ctx = TestContext::new_with_code_index_config(config).await;
        let project_dir = ctx._temp_dir.path().join("staged-backpressure");
        std::fs::create_dir_all(&project_dir).unwrap();

        for i in 0..8 {
            std::fs::write(
                project_dir.join(format!("file_{i}.rs")),
                format!("pub fn item_{i}() -> i32 {{ {i} }}\n"),
            )
            .unwrap();
        }

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();

        assert_eq!(status.status, IndexState::EmbeddingPending);
        assert_eq!(status.total_files, 8);
        assert_eq!(status.indexed_files, 8);
        assert_eq!(status.total_chunks, 8);
        assert_eq!(status.total_symbols, 8);
        assert!(status.failed_files.is_empty());
    }

    #[tokio::test]
    async fn staged_pipeline_drains_normal_files_around_ignored_file() {
        let mut config = staged_config_for_test();
        config.read_workers = 1;
        config.parse_workers = 1;
        config.commit_batch_size = 1;
        config.max_inflight_files = 2;
        config.max_inflight_bytes = 1024;
        let ctx = TestContext::new_with_code_index_config(config).await;
        let project_dir = ctx._temp_dir.path().join("staged-ignored-ordering");
        std::fs::create_dir_all(&project_dir).unwrap();

        std::fs::write(project_dir.join("a.rs"), "pub fn first() -> i32 { 1 }\n").unwrap();
        std::fs::create_dir_all(project_dir.join("generated")).unwrap();
        std::fs::write(
            project_dir.join("generated/ignored.rs"),
            "pub fn ignored_generated() -> i32 { 0 }\n",
        )
        .unwrap();
        std::fs::write(project_dir.join("z.rs"), "pub fn second() -> i32 { 2 }\n").unwrap();

        let project_id = derive_project_id(&project_dir).unwrap();
        let mut status =
            prepare_started_index_status(&project_id, &project_dir, None, vec![], vec![]);
        status.total_files = 3;
        let active_structural_generation = status.structural_generation;
        ctx.state
            .storage
            .update_index_status(status.clone())
            .await
            .unwrap();

        let (status, _metrics) = run_staged_index_pipeline(
            ctx.state.clone(),
            &project_dir,
            &project_id,
            status,
            active_structural_generation,
            Instant::now(),
            vec![
                project_dir.join("a.rs"),
                project_dir.join("generated/ignored.rs"),
                project_dir.join("z.rs"),
            ],
            None,
        )
        .await
        .unwrap();

        assert_eq!(status.status, IndexState::EmbeddingPending);
        assert_eq!(status.total_files, 3);
        assert_eq!(status.indexed_files, 3);
        assert_eq!(status.total_chunks, 2);
        assert_eq!(status.total_symbols, 2);
        assert!(status.failed_files.is_empty());

        let symbols = ctx
            .state
            .storage
            .get_project_symbols(&status.project_id, None)
            .await
            .unwrap();
        let symbol_names: HashSet<String> = symbols.into_iter().map(|symbol| symbol.name).collect();
        assert!(symbol_names.contains("first"));
        assert!(symbol_names.contains("second"));
        assert!(!symbol_names.contains("ignored_generated"));
    }

    #[tokio::test]
    async fn staged_pipeline_skips_large_file_without_failure() {
        let mut config = staged_config_for_test();
        config.commit_batch_size = 1;
        config.max_inflight_files = 1;
        config.max_inflight_bytes = 1024;
        let ctx = TestContext::new_with_code_index_config(config).await;
        let project_dir = ctx._temp_dir.path().join("staged-large-file-skip");
        std::fs::create_dir_all(&project_dir).unwrap();
        let large_content = format!("pub fn huge() {{}}\n//{}\n", "x".repeat(1_000_001));
        std::fs::write(project_dir.join("huge.rs"), large_content).unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();

        assert_eq!(status.status, IndexState::EmbeddingPending);
        assert_eq!(status.total_files, 1);
        assert_eq!(status.indexed_files, 1);
        assert_eq!(status.total_chunks, 0);
        assert_eq!(status.total_symbols, 0);
        assert!(status.failed_files.is_empty());

        let chunks = ctx
            .state
            .storage
            .get_all_chunks_for_project(&status.project_id, None)
            .await
            .unwrap();
        let symbols = ctx
            .state
            .storage
            .get_project_symbols(&status.project_id, None)
            .await
            .unwrap();
        assert!(chunks.is_empty());
        assert!(symbols.is_empty());
    }

    #[tokio::test]
    async fn indexing_skips_protoc_generated_java_without_blocking_serving_generation() {
        let mut config = staged_config_for_test();
        config.commit_batch_size = 1;
        config.max_inflight_files = 1;
        config.max_inflight_bytes = 1024;
        let ctx = TestContext::new_with_code_index_config(config).await;
        let project_dir = ctx._temp_dir.path().join("protoc-generated-java-skip");
        let generated_dir = project_dir.join("android/lib_sdk_proto/src/main/java/event");
        fs::create_dir_all(project_dir.join("src")).unwrap();
        fs::create_dir_all(&generated_dir).unwrap();
        fs::write(
            project_dir.join("src/main.rs"),
            "pub fn searchable_symbol() -> i32 { 1 }\n",
        )
        .unwrap();
        fs::write(
            generated_dir.join("Event.java"),
            format!(
                "// Generated by the protocol buffer compiler.  DO NOT EDIT!\n// source: event.proto\npublic final class Event {{}}\n// {}\n",
                "x".repeat(1_100_000)
            ),
        )
        .unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();

        assert_eq!(status.status, IndexState::EmbeddingPending);
        assert_eq!(status.total_files, 1);
        assert_eq!(status.indexed_files, 1);
        assert_eq!(status.total_chunks, 1);
        assert_eq!(status.total_symbols, 1);
        assert!(status.failed_files.is_empty());

        let active_generation = ctx
            .state
            .storage
            .get_active_generation(&status.project_id)
            .await
            .unwrap();
        assert_eq!(active_generation, Some(status.structural_generation));
        let serving_bm25 = ctx
            .state
            .storage
            .get_serving_generation(&status.project_id, CapabilityKind::Bm25)
            .await
            .unwrap();
        let serving_symbols = ctx
            .state
            .storage
            .get_serving_generation(&status.project_id, CapabilityKind::Symbols)
            .await
            .unwrap();
        assert_eq!(serving_bm25, Some(status.structural_generation));
        assert_eq!(serving_symbols, Some(status.structural_generation));

        let symbols = ctx
            .state
            .storage
            .get_project_symbols(&status.project_id, active_generation)
            .await
            .unwrap();
        let symbol_names: HashSet<String> = symbols.into_iter().map(|symbol| symbol.name).collect();
        assert!(symbol_names.contains("searchable_symbol"));
        assert!(!symbol_names.contains("Event"));
    }

    #[tokio::test]
    async fn deterministic_repeated_indexing_has_stable_counts_and_locators() {
        let ctx = TestContext::new_with_code_index_config(staged_config_for_test()).await;
        let project_dir = ctx._temp_dir.path().join("deterministic-reindex");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(
            project_dir.join("lib.rs"),
            "pub fn alpha() -> i32 { beta() }\nfn beta() -> i32 { 7 }\n",
        )
        .unwrap();
        fs::write(
            project_dir.join("model.rs"),
            "pub struct Model { pub v: i32 }\n",
        )
        .unwrap();

        let status1 = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();
        let chunks1 = ctx
            .state
            .storage
            .get_all_chunks_for_project(&status1.project_id, active_generation_for_compat(&status1))
            .await
            .unwrap();
        let symbols1 = ctx
            .state
            .storage
            .get_project_symbols(&status1.project_id, active_generation_for_compat(&status1))
            .await
            .unwrap();

        let chunk_locators_1: Vec<(String, u32, u32, String)> = {
            let mut rows: Vec<_> = chunks1
                .iter()
                .map(|chunk| {
                    (
                        chunk.file_path.clone(),
                        chunk.start_line,
                        chunk.end_line,
                        chunk.content_hash.clone(),
                    )
                })
                .collect();
            rows.sort();
            rows
        };
        let symbol_locators_1: Vec<(String, u32, String, String)> = {
            let mut rows: Vec<_> = symbols1
                .iter()
                .map(|symbol| {
                    (
                        symbol.file_path.clone(),
                        symbol.start_line,
                        symbol.name.clone(),
                        symbol.symbol_type.to_string(),
                    )
                })
                .collect();
            rows.sort();
            rows
        };
        let symbol_ids_1: Vec<String> = {
            let mut ids: Vec<_> = symbols1
                .iter()
                .map(|symbol| {
                    let id = symbol.id.as_ref().expect("symbol id");
                    format!(
                        "{}:{}",
                        id.table.as_str(),
                        crate::types::record_key_to_string(&id.key)
                    )
                })
                .collect();
            ids.sort();
            ids
        };

        let status2 = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();
        let chunks2 = ctx
            .state
            .storage
            .get_all_chunks_for_project(&status2.project_id, active_generation_for_compat(&status2))
            .await
            .unwrap();
        let symbols2 = ctx
            .state
            .storage
            .get_project_symbols(&status2.project_id, active_generation_for_compat(&status2))
            .await
            .unwrap();

        let mut chunk_locators_2: Vec<_> = chunks2
            .iter()
            .map(|chunk| {
                (
                    chunk.file_path.clone(),
                    chunk.start_line,
                    chunk.end_line,
                    chunk.content_hash.clone(),
                )
            })
            .collect();
        chunk_locators_2.sort();
        let mut symbol_locators_2: Vec<_> = symbols2
            .iter()
            .map(|symbol| {
                (
                    symbol.file_path.clone(),
                    symbol.start_line,
                    symbol.name.clone(),
                    symbol.symbol_type.to_string(),
                )
            })
            .collect();
        symbol_locators_2.sort();
        let mut symbol_ids_2: Vec<String> = symbols2
            .iter()
            .map(|symbol| {
                let id = symbol.id.as_ref().expect("symbol id");
                format!(
                    "{}:{}",
                    id.table.as_str(),
                    crate::types::record_key_to_string(&id.key)
                )
            })
            .collect();
        symbol_ids_2.sort();

        assert_eq!(status1.total_files, status2.total_files);
        assert_eq!(status1.total_chunks, status2.total_chunks);
        assert_eq!(status1.total_symbols, status2.total_symbols);
        assert_eq!(chunk_locators_1, chunk_locators_2);
        assert_eq!(symbol_locators_1, symbol_locators_2);
        assert_eq!(symbol_ids_1, symbol_ids_2);

        assert_eq!(chunks2.len(), status2.total_chunks as usize);
        assert_eq!(symbols2.len(), status2.total_symbols as usize);
    }

    #[tokio::test]
    async fn different_projects_can_index_concurrently() {
        let ctx = TestContext::new_with_code_index_config(staged_config_for_test()).await;
        let project_a = ctx._temp_dir.path().join("concurrent-project-a");
        let project_b = ctx._temp_dir.path().join("concurrent-project-b");
        fs::create_dir_all(&project_a).unwrap();
        fs::create_dir_all(&project_b).unwrap();
        fs::write(project_a.join("a.rs"), "pub fn a() -> i32 { 1 }\n").unwrap();
        fs::write(project_b.join("b.rs"), "pub fn b() -> i32 { 2 }\n").unwrap();

        let (a, b) = tokio::join!(
            index_project(ctx.state.clone(), &project_a),
            index_project(ctx.state.clone(), &project_b)
        );

        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a.total_files, 1);
        assert_eq!(b.total_files, 1);
        assert_ne!(a.project_id, b.project_id);
    }

    #[tokio::test]
    async fn ignored_only_project_completes_without_indexed_rows() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("ignored-only-project");
        fs::create_dir_all(project_dir.join("generated")).unwrap();
        fs::write(project_dir.join("generated/auto.g.dart"), "// generated\n").unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();
        assert_eq!(status.status, IndexState::Completed);
        assert_eq!(status.total_files, 0);
        assert_eq!(status.total_chunks, 0);
        assert_eq!(status.total_symbols, 0);
    }

    #[tokio::test]
    async fn unsupported_only_project_completes_without_indexed_rows() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("unsupported-only-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("notes.txt"), "plain text\n").unwrap();
        fs::write(project_dir.join("data.json"), "{\"k\":1}\n").unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();
        assert_eq!(status.status, IndexState::Completed);
        assert_eq!(status.total_files, 0);
        assert_eq!(status.total_chunks, 0);
        assert_eq!(status.total_symbols, 0);
    }

    #[tokio::test]
    async fn parse_failure_in_one_file_records_failed_file_but_completes_other_files() {
        let ctx = TestContext::new_with_code_index_config(staged_config_for_test()).await;
        let project_dir = ctx._temp_dir.path().join("partial-parse-failure-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("ok.rs"), "pub fn ok() -> i32 { 1 }\n").unwrap();
        fs::write(project_dir.join("bad.rs"), vec![0xff, 0xfe, 0xfd]).unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();
        assert_eq!(status.total_files, 2);
        assert_eq!(status.indexed_files, 2);
        assert_eq!(status.total_symbols, 1);
        assert_eq!(status.total_chunks, 1);
        assert_eq!(status.failed_files.len(), 1);
        assert!(
            status.failed_files[0].contains("bad.rs"),
            "expected UTF-8 read failure for bad.rs, got {:?}",
            status.failed_files
        );
    }

    #[tokio::test]
    async fn incremental_index_handles_deleted_and_changed_files() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("incremental-delete-change");
        fs::create_dir_all(&project_dir).unwrap();
        let file_a = project_dir.join("a.rs");
        let file_b = project_dir.join("b.rs");
        fs::write(&file_a, "pub fn alpha() -> i32 { 1 }\n").unwrap();
        fs::write(&file_b, "pub fn beta() -> i32 { 2 }\n").unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();
        fs::write(&file_a, "pub fn alpha() -> i32 { 10 }\n").unwrap();
        fs::remove_file(&file_b).unwrap();

        let result = incremental_index(
            ctx.state.clone(),
            &status.project_id,
            vec![file_a.clone(), file_b.clone()],
        )
        .await
        .unwrap();

        assert_eq!(result.updated_files, 1);
        assert_eq!(result.deleted_files.len(), 1);
        assert!(result.deleted_files[0].contains("b.rs"));
        assert!(!result.new_chunks.is_empty());

        let b_chunks = ctx
            .state
            .storage
            .get_chunks_by_path(&status.project_id, &file_b.to_string_lossy(), None)
            .await
            .unwrap();
        let b_symbols: Vec<_> = ctx
            .state
            .storage
            .get_project_symbols(&status.project_id, None)
            .await
            .unwrap()
            .into_iter()
            .filter(|symbol| symbol.file_path == file_b.to_string_lossy())
            .collect();
        assert!(b_chunks.is_empty());
        assert!(b_symbols.is_empty());
    }

    #[tokio::test]
    async fn incremental_index_removes_file_that_becomes_generated_source() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("incremental-generated-source");
        fs::create_dir_all(&project_dir).unwrap();
        let file = project_dir.join("api.rs");
        fs::write(&file, "pub fn api() -> i32 { 1 }\n").unwrap();

        let status = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();
        let initial_symbols = ctx
            .state
            .storage
            .get_project_symbols(&status.project_id, None)
            .await
            .unwrap();
        assert!(initial_symbols.iter().any(|symbol| symbol.name == "api"));

        fs::write(
            &file,
            "// This file was generated by codegen. Do not edit.\npub fn api() -> i32 { 2 }\n",
        )
        .unwrap();

        let result = incremental_index(ctx.state.clone(), &status.project_id, vec![file.clone()])
            .await
            .unwrap();

        assert_eq!(result.updated_files, 0);
        assert_eq!(result.deleted_files.len(), 1);
        assert!(result.deleted_files[0].contains("api.rs"));

        let chunks = ctx
            .state
            .storage
            .get_chunks_by_path(&status.project_id, &file.to_string_lossy(), None)
            .await
            .unwrap();
        let symbols: Vec<_> = ctx
            .state
            .storage
            .get_project_symbols(&status.project_id, None)
            .await
            .unwrap()
            .into_iter()
            .filter(|symbol| symbol.file_path == file.to_string_lossy())
            .collect();
        assert!(chunks.is_empty());
        assert!(symbols.is_empty());
    }

    #[tokio::test]
    async fn staged_progress_is_monotonic_and_status_updates_are_throttled() {
        let mut config = staged_config_for_test();
        config.status_flush_ms = 60_000;
        let ctx = TestContext::new_with_code_index_config(config.clone()).await;
        let project_dir = ctx._temp_dir.path().join("progress-throttle-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("f.rs"), "pub fn f() -> i32 { 1 }\n").unwrap();

        let project_id = derive_project_id(&project_dir).unwrap();
        let monitor = ctx.state.progress.get_or_create(&project_id).await;
        let mut status = IndexStatus::new(project_id.clone());
        status.total_files = 1;
        let mut metrics = IndexMetrics::default();
        let mut symbol_index = SymbolIndex::new();
        let mut total_relation_stats = RelationStats::default();
        let mut pending = BTreeMap::new();
        let mut next_commit_seq = 0usize;
        let mut chunk_buffer = Vec::new();
        let mut symbol_buffer = Vec::new();
        let mut hash_buffer = Vec::new();
        let mut relation_buffer = Vec::new();

        pending.insert(
            0,
            ParsedFile {
                seq: 0,
                path: project_dir.join("f.rs"),
                path_str: project_dir.join("f.rs").to_string_lossy().to_string(),
                file_hash: None,
                chunks: vec![],
                symbols: vec![],
                references: vec![],
                read_elapsed_ms: 1,
                parse_elapsed_ms: 1,
                error: None,
                skipped: false,
            },
        );

        let mut last_status_flush = Instant::now();
        drain_ready_staged_results(
            &ctx.state,
            &project_id,
            &monitor,
            &mut status,
            &mut metrics,
            &mut symbol_index,
            &mut total_relation_stats,
            &mut pending,
            &mut next_commit_seq,
            &mut chunk_buffer,
            &mut symbol_buffer,
            &mut hash_buffer,
            &mut relation_buffer,
            &config,
            &mut last_status_flush,
            None,
        )
        .await
        .unwrap();

        assert_eq!(status.indexed_files, 1);
        assert_eq!(monitor.indexed_files.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.status_update_elapsed_ms, 0);

        pending.insert(
            1,
            ParsedFile {
                seq: 1,
                path: project_dir.join("f.rs"),
                path_str: project_dir.join("f.rs").to_string_lossy().to_string(),
                file_hash: None,
                chunks: vec![],
                symbols: vec![],
                references: vec![],
                read_elapsed_ms: 1,
                parse_elapsed_ms: 1,
                error: None,
                skipped: false,
            },
        );
        last_status_flush = Instant::now() - Duration::from_millis(config.status_flush_ms + 1);

        drain_ready_staged_results(
            &ctx.state,
            &project_id,
            &monitor,
            &mut status,
            &mut metrics,
            &mut symbol_index,
            &mut total_relation_stats,
            &mut pending,
            &mut next_commit_seq,
            &mut chunk_buffer,
            &mut symbol_buffer,
            &mut hash_buffer,
            &mut relation_buffer,
            &config,
            &mut last_status_flush,
            None,
        )
        .await
        .unwrap();

        assert_eq!(status.indexed_files, 2);
        assert_eq!(monitor.indexed_files.load(Ordering::Relaxed), 2);
        assert!(
            metrics.status_update_elapsed_ms > 0,
            "expected at least one throttled status update when interval elapsed"
        );
    }

    #[tokio::test]
    async fn invalid_filter_fails_before_cleanup() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("filter-invalid-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("main.rs"), "fn main() {}").unwrap();

        let filter_config = IndexFilterConfig {
            include_patterns: vec!["\\invalid\\pattern".to_string()],
            exclude_patterns: vec![],
        };

        let err = index_project_with_filter(ctx.state.clone(), &project_dir, filter_config)
            .await
            .unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("invalid_filter") || msg.contains("invalid glob"),
            "expected invalid_filter error, got: {msg}"
        );

        let stored = ctx
            .state
            .storage
            .get_index_status("filter-invalid-project")
            .await
            .unwrap();
        assert!(
            stored.is_none()
                || stored
                    .map(|s| s.status != IndexState::Indexing)
                    .unwrap_or(true),
            "index status should not be left in Indexing state after filter validation failure"
        );
    }

    #[tokio::test]
    async fn filtered_index_only_indexes_matching_files() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("filter-include-project");
        fs::create_dir_all(project_dir.join("src")).unwrap();
        fs::create_dir_all(project_dir.join("tests")).unwrap();

        fs::write(project_dir.join("src/lib.rs"), "pub fn lib_fn() {}").unwrap();
        fs::write(project_dir.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(project_dir.join("tests/test_a.rs"), "fn test_a() {}").unwrap();

        let filter_config = IndexFilterConfig {
            include_patterns: vec!["src/**".to_string()],
            exclude_patterns: vec![],
        };

        let status = index_project_with_filter(ctx.state.clone(), &project_dir, filter_config)
            .await
            .unwrap();

        assert_eq!(status.total_files, 2, "only src/ files should be indexed");
        assert!(
            status.status == IndexState::Completed || status.status == IndexState::EmbeddingPending,
            "expected Completed or EmbeddingPending, got: {:?}",
            status.status
        );
    }

    #[tokio::test]
    async fn no_filter_returns_same_count_as_default() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("filter-nofilter-project");
        fs::create_dir_all(&project_dir).unwrap();

        for i in 0..5 {
            fs::write(
                project_dir.join(format!("file_{i}.rs")),
                format!("fn f{i}() {{}}"),
            )
            .unwrap();
        }

        let status_default = index_project(ctx.state.clone(), &project_dir)
            .await
            .unwrap();

        let ctx2 = TestContext::new().await;
        let project_dir2 = ctx2._temp_dir.path().join("filter-nofilter-project2");
        fs::create_dir_all(&project_dir2).unwrap();
        for i in 0..5 {
            fs::write(
                project_dir2.join(format!("file_{i}.rs")),
                format!("fn f{i}() {{}}"),
            )
            .unwrap();
        }

        let filter_config = IndexFilterConfig {
            include_patterns: vec![],
            exclude_patterns: vec![],
        };
        let status_filtered =
            index_project_with_filter(ctx2.state.clone(), &project_dir2, filter_config)
                .await
                .unwrap();

        assert_eq!(
            status_default.total_files, status_filtered.total_files,
            "empty filter should index same files as default"
        );
    }

    #[tokio::test]
    async fn filter_snapshot_stored_in_index_status() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("filter-snapshot-project");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("main.rs"), "fn main() {}").unwrap();
        fs::write(project_dir.join("lib.rs"), "pub fn lib() {}").unwrap();

        let filter_config = IndexFilterConfig {
            include_patterns: vec!["*.rs".to_string()],
            exclude_patterns: vec!["**/generated/**".to_string()],
        };
        let status = index_project_with_filter(ctx.state.clone(), &project_dir, filter_config)
            .await
            .unwrap();

        assert_eq!(status.include_patterns, vec!["*.rs".to_string()]);
        assert_eq!(status.exclude_patterns, vec!["**/generated/**".to_string()]);
    }
}
