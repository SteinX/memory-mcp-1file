use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::codebase::ProjectLifecycleStatus;
use crate::config::{AppState, IndexMonitor};
use crate::server::params::{
    DeleteProjectParams, GetIndexStatusParams, GetProjectProjectionParams, GetProjectStatsParams,
    GetProjectionByLocatorParams, IndexProjectParams, ListProjectsParams,
};
use crate::storage::StorageBackend;
use crate::types::{
    derive_project_id, CodeIntelligenceDiagnostic, ContractReasonCode, ExportIdentity, IndexState,
    IndexJobPhase, IndexJobReasonCode, IndexJobRecord, IndexJobState, IndexStatus,
    ProjectionLocatorLifecycle, ProjectionLocatorLookup, ProjectionLocatorLookupState,
    ProjectionLocatorRecord,
};
use crate::types::code::{IndexJobError, IndexJobProgress, IndexJobResumeState};

use super::super::contracts::{
    assemble_project_projection, collect_project_projection_inputs, export_contract_meta,
    project_info_capability_block, shape_project_projection_graph, summary_collection_response,
    summary_index_status_response, summary_index_status_response_with_reason, with_surface_guidance,
};
use super::super::{error_response, success_json};

fn root_diagnostic(
    configured_root: Option<&std::path::Path>,
    fallback_root: &std::path::Path,
) -> CodeIntelligenceDiagnostic {
    if let Some(configured_root) = configured_root {
        if configured_root.exists() {
            return CodeIntelligenceDiagnostic::selected(format!(
                "Configured project root is available: {}",
                configured_root.display()
            ));
        }

        return CodeIntelligenceDiagnostic::missing_root(format!(
                "Configured project root is missing: {}. Set PROJECT_PATH to an existing server-visible path.",
                configured_root.display()
            ));
    }

    if fallback_root.exists() {
        CodeIntelligenceDiagnostic::selected(format!(
            "Compatibility project root is available: {}",
            fallback_root.display()
        ))
    } else {
        CodeIntelligenceDiagnostic::disabled(format!(
                "Code intelligence startup root is unavailable. Mount {} or set PROJECT_PATH to a server-visible directory.",
                fallback_root.display()
            ))
    }
}

fn runtime_root_diagnostic() -> CodeIntelligenceDiagnostic {
    let configured_root = std::env::var_os("PROJECT_PATH").map(std::path::PathBuf::from);
    root_diagnostic(configured_root.as_deref(), std::path::Path::new("/project"))
}

fn project_state_diagnostic(
    status: IndexState,
    status_metadata_missing: bool,
) -> CodeIntelligenceDiagnostic {
    if status_metadata_missing {
        return CodeIntelligenceDiagnostic::degraded(
            "Index status metadata is missing while code intelligence rows exist.",
        );
    }

    match status {
        IndexState::Completed => {
            CodeIntelligenceDiagnostic::ready("Code intelligence is ready for this project.")
        }
        IndexState::Indexing | IndexState::EmbeddingPending => {
            CodeIntelligenceDiagnostic::indexing(
                "Code intelligence indexing is in progress for this project.",
            )
        }
        IndexState::Failed => {
            CodeIntelligenceDiagnostic::degraded("Code intelligence is degraded for this project.")
        }
    }
}

fn lifecycle_json(status: &crate::types::IndexStatus) -> serde_json::Value {
    json!({
        "structural": {
            "state": status.structural_state.to_string(),
            "is_ready": status.structural_state == crate::types::StructuralState::Ready,
            "generation": status.structural_generation
        },
        "semantic": {
            "state": status.semantic_state.to_string(),
            "is_ready": status.semantic_state == crate::types::SemanticState::Ready,
            "generation": status.semantic_generation,
            "is_caught_up": status.semantic_state == crate::types::SemanticState::Ready
                && status.semantic_generation == status.structural_generation
        },
        "projection": {
            "state": status.projection_state.to_string(),
            "is_current": status.projection_state == crate::types::ProjectionState::Current
        }
    })
}

fn registry_lifecycle_json(status: &ProjectLifecycleStatus) -> serde_json::Value {
    json!({
        "project_id": status.project_id,
        "root_path": status.root_path.to_string_lossy(),
        "state": format!("{:?}", status.state).to_lowercase(),
        "diagnostic": status.diagnostic.as_json(),
        "pending_jobs": status.pending_jobs,
        "handles": {
            "manager": status.has_manager_handle,
            "worker": status.has_worker_handle,
            "worker_sender": status.has_worker_sender
        },
        "options": {
            "registry_starts_workers": status.options.registry_starts_workers,
            "registry_starts_watchers": status.options.registry_starts_watchers
        },
        "last_error": status.last_error
    })
}

fn read_monitor_string(lock: &std::sync::RwLock<String>) -> String {
    lock.read().map(|value| value.clone()).unwrap_or_default()
}

fn read_monitor_optional_string(lock: &std::sync::RwLock<Option<String>>) -> Option<String> {
    lock.read().ok().and_then(|value| value.clone())
}

fn set_monitor_string(lock: &std::sync::RwLock<String>, value: impl Into<String>) {
    if let Ok(mut guard) = lock.write() {
        *guard = value.into();
    }
}

fn set_monitor_optional_string(lock: &std::sync::RwLock<Option<String>>, value: Option<String>) {
    if let Ok(mut guard) = lock.write() {
        *guard = value;
    }
}

fn one_shot_task_message() -> &'static str {
    "The registry lifecycle reports only registry-managed watchers/workers. Manual index requests run as one-shot background tasks. Track them with project_info(action=\"status\") or the client alias project_status(action=\"status\")."
}

fn new_index_operation_id(project_id: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("idx-{project_id}-{millis}-{sequence}")
}

fn new_index_job_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("idx_{nanos}_{sequence}")
}

fn index_job_phase_str(phase: &IndexJobPhase) -> &'static str {
    match phase {
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
    }
}

fn checkpoint_resume_token(phase: &IndexJobPhase, files_done: u64) -> String {
    format!("ckpt_v1_phase_{}_file_{}", index_job_phase_str(phase), files_done)
}

fn durable_index_job_record(
    project_id: &str,
    workspace_path: &str,
    structural_generation: u64,
) -> IndexJobRecord {
    let now = crate::types::Datetime::default();
    IndexJobRecord {
        id: None,
        job_id: new_index_job_id(),
        project_id: project_id.to_string(),
        target_generation: structural_generation,
        workspace_path: workspace_path.to_string(),
        target_fingerprint: None,
        structural_generation,
        state: IndexJobState::Running,
        stored_phase: Some(IndexJobPhase::Discover),
        phase: IndexJobPhase::Discover,
        resume_token: checkpoint_resume_token(&IndexJobPhase::Discover, 0),
        created_at: now.clone(),
        started_at: Some(now.clone()),
        updated_at: now,
        completed_at: None,
        error: None,
        resume: Some(IndexJobResumeState {
            supported: false,
            token: None,
            checkpoint_generation: None,
            reason_if_not_supported: Some(IndexJobReasonCode::CheckpointGenerationMissing),
        }),
        completed_files_count: 0,
        total_files_count: None,
        reason_code: None,
        progress: IndexJobProgress::default(),
    }
}

fn index_job_state_str(state: &IndexJobState) -> &'static str {
    match state {
        IndexJobState::Queued => "queued",
        IndexJobState::Running => "running",
        IndexJobState::Paused => "paused",
        IndexJobState::Interrupted => "interrupted",
        IndexJobState::Resumable => "resumable",
        IndexJobState::Completed => "completed",
        IndexJobState::Failed => "failed",
        IndexJobState::CancelRequested => "cancel_requested",
        IndexJobState::Cancelled => "cancelled",
        IndexJobState::Abandoned => "abandoned",
    }
}

fn index_job_reason_code_str(reason: &IndexJobReasonCode) -> &'static str {
    match reason {
        IndexJobReasonCode::CancelledByUser => "cancelled_by_user",
        IndexJobReasonCode::InterruptedByShutdown => "interrupted_by_shutdown",
        IndexJobReasonCode::LostSameProcessTask => "lost_same_process_task",
        IndexJobReasonCode::StorageError => "storage_error",
        IndexJobReasonCode::ParseError => "parse_error",
        IndexJobReasonCode::EmbeddingError => "embedding_error",
        IndexJobReasonCode::Bm25Error => "bm25_error",
        IndexJobReasonCode::Unknown => "unknown",
        IndexJobReasonCode::ActiveIndexRunning => "active_index_running",
        IndexJobReasonCode::ResumableInterruptedJob => "resumable_interrupted_job",
        IndexJobReasonCode::LostOneShotIndexingTaskAfterRestart => {
            "lost_one_shot_indexing_task_after_restart"
        }
        IndexJobReasonCode::CheckpointGenerationMissing => "checkpoint_generation_missing",
        IndexJobReasonCode::WorkspaceChangedSinceCheckpoint => "workspace_changed_since_checkpoint",
        IndexJobReasonCode::StaleGeneration => "stale_generation",
        IndexJobReasonCode::IndexStorageCorrupt => "index_storage_corrupt",
        IndexJobReasonCode::IllegalStateTransition => "illegal_state_transition",
        IndexJobReasonCode::ResumeTokenRequired => "resume_token_required",
        IndexJobReasonCode::ForceRestartConfirmationRequired => "force_restart_confirmation_required",
        IndexJobReasonCode::CancellationRequested => "cancellation_requested",
        IndexJobReasonCode::CleanupRequested => "cleanup_requested",
    }
}

async fn resumable_job_fields(
    state: &Arc<AppState>,
    job: &IndexJobRecord,
) -> (bool, Option<String>, Option<u64>, Option<u64>, Option<IndexJobPhase>) {
    match state
        .storage
        .list_file_checkpoints_for_job(&job.project_id, job.target_generation)
        .await
    {
        Ok(checkpoints) => {
            let files_done = checkpoints.iter().filter(|checkpoint| checkpoint.completed).count() as u64;
            if files_done == 0 {
                return (false, None, Some(0), job.total_files_count, None);
            }
            let phase = checkpoints
                .iter()
                .rev()
                .find(|checkpoint| checkpoint.completed)
                .map(|checkpoint| checkpoint.phase.clone())
                .unwrap_or(IndexJobPhase::Parse);
            (
                true,
                Some(checkpoint_resume_token(&phase, files_done)),
                Some(files_done),
                job.total_files_count,
                Some(phase),
            )
        }
        Err(_) => (false, None, None, job.total_files_count, None),
    }
}

async fn index_job_json(state: &Arc<AppState>, job: &IndexJobRecord) -> serde_json::Value {
    let reason_code = job
        .reason_code
        .as_ref()
        .or_else(|| job.error.as_ref().map(|error| &error.code));
    let (checkpoint_can_resume, checkpoint_token, files_done, files_total, checkpoint_phase) =
        resumable_job_fields(state, job).await;
    let can_resume = matches!(job.state, IndexJobState::Resumable | IndexJobState::Interrupted | IndexJobState::Failed)
        && checkpoint_can_resume;
    let resume_token = if can_resume { checkpoint_token } else { None };
    let effective_reason_code = if can_resume {
        Some(IndexJobReasonCode::ResumableInterruptedJob)
    } else {
        reason_code.cloned()
    };
    let mut progress = serde_json::to_value(&job.progress).unwrap_or_else(|_| json!({}));
    if let Some(object) = progress.as_object_mut() {
        if let Some(files_done) = files_done {
            object.insert("files_done".to_string(), json!(files_done));
        }
        if let Some(files_total) = files_total {
            object.insert("files_total".to_string(), json!(files_total));
        }
    }

    json!({
        "job_id": job.job_id,
        "state": if can_resume { "resumable" } else { index_job_state_str(&job.state) },
        "operation_id": null,
        "can_resume": can_resume,
        "reason_code": effective_reason_code.as_ref().map(index_job_reason_code_str),
        "requires_force": matches!(job.state, IndexJobState::Failed) && !can_resume,
        "requires_confirmation": matches!(job.state, IndexJobState::Failed) && !can_resume,
        "restart_fallback": if matches!(job.state, IndexJobState::Failed) && !can_resume {
            Some(json!({ "force": true, "confirm_failed_restart": true }))
        } else {
            None
        },
        "resume_token": resume_token,
        "target_generation": job.target_generation,
        "structural_generation": job.structural_generation,
        "phase": index_job_phase_str(checkpoint_phase.as_ref().unwrap_or(&job.phase)),
        "created_at": job.created_at,
        "started_at": job.started_at,
        "updated_at": job.updated_at,
        "completed_at": job.completed_at,
        "progress": progress,
        "identity_semantics": {
            "job_id": "durable indexing job id persisted in storage; use for cross-restart tracking and future resume/cancel/cleanup flows",
            "operation_id": "same-process one-shot task id; null after restart and never valid for resume"
        }
    })
}

async fn index_job_json_with_operation(
    state: &Arc<AppState>,
    job: &IndexJobRecord,
    operation_id: Option<String>,
) -> serde_json::Value {
    let mut value = index_job_json(state, job).await;
    if let Some(object) = value.as_object_mut() {
        object.insert("operation_id".to_string(), json!(operation_id));
    }
    value
}

fn lost_one_shot_index_job_json() -> serde_json::Value {
    json!({
        "job_id": null,
        "state": "failed",
        "operation_id": null,
        "can_resume": false,
        "reason_code": "lost_one_shot_indexing_task_after_restart",
        "requires_force": true,
        "requires_confirmation": true,
        "restart_fallback": { "force": true, "confirm_failed_restart": true },
        "resume_token": null,
        "identity_semantics": {
            "job_id": "durable indexing job id persisted in storage; unavailable for legacy one-shot lost-task status before durable job adoption",
            "operation_id": "same-process one-shot task id; null after restart and never valid for resume"
        }
    })
}

async fn latest_index_job_json(
    state: &Arc<AppState>,
    project_id: &str,
) -> Option<serde_json::Value> {
    let job = state
        .storage
        .list_index_jobs_for_project(project_id)
        .await
        .ok()
        .and_then(|jobs| jobs.into_iter().next())?;
    Some(index_job_json(state, &job).await)
}

fn index_status_summary(
    status: &IndexStatus,
    indexed_files: u32,
    total_chunks: u32,
    total_symbols: u32,
    overall_progress_percent: f32,
    message: Option<String>,
) -> crate::types::ExportResponseSummary {
    match status.status {
        IndexState::Completed => summary_index_status_response(
            status.total_files,
            indexed_files,
            total_chunks,
            total_symbols,
            overall_progress_percent,
            false,
            message,
        ),
        IndexState::Failed => summary_index_status_response_with_reason(
            status.total_files,
            indexed_files,
            total_chunks,
            total_symbols,
            overall_progress_percent,
            true,
            Some(ContractReasonCode::Degraded),
            Some("indexing_failed".to_string()),
            message.or_else(|| {
                Some(
                    "Indexing failed. Project stats are incomplete until a force rebuild succeeds."
                        .to_string(),
                )
            }),
        ),
        IndexState::Indexing | IndexState::EmbeddingPending => summary_index_status_response(
            status.total_files,
            indexed_files,
            total_chunks,
            total_symbols,
            overall_progress_percent,
            true,
            message,
        ),
    }
}

fn lost_one_shot_recovery_json(project_id: &str) -> serde_json::Value {
    json!({
        "reason": "lost_one_shot_indexing_task_after_restart",
        "guidance": "The project is persisted as indexing, but the local one-shot task does not survive process restart. Verify resources and retry with force=true and confirm_failed_restart=true.",
        "recommended_action": "retry_with_force_and_confirmation",
        "example": {
            "tool": "index_project",
            "arguments": {
                "project_id": project_id,
                "force": true,
                "confirm_failed_restart": true
            }
        }
    })
}

fn lost_one_shot_indexing_summary(
    status: &IndexStatus,
    indexed_files: u32,
    total_chunks: u32,
    total_symbols: u32,
    overall_progress_percent: f32,
) -> crate::types::ExportResponseSummary {
    summary_index_status_response_with_reason(
        status.total_files,
        indexed_files,
        total_chunks,
        total_symbols,
        overall_progress_percent,
        true,
        Some(ContractReasonCode::Degraded),
        Some("lost_one_shot_indexing_task_after_restart".to_string()),
        Some(
            "Indexing cannot be observed in this process because the local one-shot task was lost after restart. Retry with force=true and confirm_failed_restart=true."
                .to_string(),
        ),
    )
}

fn lost_one_shot_contract_json(status: &IndexStatus) -> serde_json::Value {
    let lifecycle = lifecycle_json(status);
    let mut contract =
        phase1_contract_json(Some(&status.project_id), Some(&lifecycle), Some(status));
    if let Some(object) = contract.as_object_mut() {
        object.insert("generated_at".to_string(), json!("unknown_after_restart"));
    }
    contract
}

async fn is_lost_one_shot_indexing_task_after_restart(
    state: &Arc<AppState>,
    project_id: &str,
    status: &IndexStatus,
    background_task: &serde_json::Value,
    sync_queue_size: usize,
) -> bool {
    if status.status != IndexState::Indexing || sync_queue_size > 0 {
        return false;
    }

    if background_task
        .get("state")
        .and_then(|value| value.as_str())
        != Some("unknown_after_restart")
        || background_task
            .get("operation_id")
            .is_some_and(|value| !value.is_null())
        || background_task
            .get("runner")
            .and_then(|value| value.as_str())
            != Some("local_tokio_task")
    {
        return false;
    }

    if state.progress.get(project_id).await.is_some() {
        return false;
    }

    state
        .indexing_projects
        .lock()
        .map(|projects| !projects.contains(project_id))
        .unwrap_or(false)
}

fn monitor_one_shot_index_task_json(monitor: &IndexMonitor) -> serde_json::Value {
    let task_state = read_monitor_string(&monitor.task_state);
    let phase = match task_state.as_str() {
        "queued" => "queued",
        "running" => "task_spawned",
        "idle" => "accepted",
        other => other,
    };

    json!({
        "operation_id": read_monitor_optional_string(&monitor.operation_id),
        "state": task_state,
        "phase": phase,
        "runner": "local_tokio_task",
        "registry_lifecycle_scope": "manual_project_registration_only",
        "current_file": read_monitor_string(&monitor.current_file),
        "total_files": monitor.total_files.load(std::sync::atomic::Ordering::Relaxed),
        "indexed_files": monitor.indexed_files.load(std::sync::atomic::Ordering::Relaxed),
        "last_error": read_monitor_optional_string(&monitor.last_error),
        "survives_process_restart": false,
        "restart_fallback": "If the process restarts while status remains indexing, the in-memory one-shot operation is gone; retry with force=true and confirm_failed_restart=true after checking resources.",
        "message": one_shot_task_message()
    })
}

async fn one_shot_index_task_json(
    state: &Arc<AppState>,
    project_id: &str,
    task_state: &str,
) -> serde_json::Value {
    if let Some(monitor) = state.progress.get(project_id).await {
        if read_monitor_string(&monitor.task_state) == "idle" {
            set_monitor_string(&monitor.task_state, task_state);
        }
        return monitor_one_shot_index_task_json(&monitor);
    }

    json!({
        "operation_id": null,
        "state": task_state,
        "phase": if task_state == "already_running" { "task_spawned" } else { "accepted" },
        "runner": "local_tokio_task",
        "registry_lifecycle_scope": "manual_project_registration_only",
        "survives_process_restart": false,
        "message": one_shot_task_message()
    })
}

async fn status_background_task_json(
    state: &Arc<AppState>,
    project_id: &str,
    status: &IndexStatus,
) -> serde_json::Value {
    if let Some(monitor) = state.progress.get(project_id).await {
        return monitor_one_shot_index_task_json(&monitor);
    }

    if status.status == IndexState::Indexing {
        let phase = if status.total_files == 0 && status.total_chunks == 0 {
            "before_file_enumeration"
        } else {
            "lost_task_after_restart"
        };

        return json!({
            "operation_id": null,
            "state": "unknown_after_restart",
            "phase": phase,
            "status": "failed",
            "retryable": true,
            "reason_code": "lost_one_shot_indexing_task_after_restart",
            "runner": "local_tokio_task",
            "registry_lifecycle_scope": "manual_project_registration_only",
            "survives_process_restart": false,
            "recovery": lost_one_shot_recovery_json(&status.project_id),
            "message": "Index status is persisted as indexing, but no same-process one-shot task is registered. The server likely restarted or reconnected after queuing; verify resources, then retry with force=true and confirm_failed_restart=true if progress remains unchanged."
        });
    }

    json!({
        "operation_id": null,
        "state": status.status.to_string(),
        "runner": "none",
        "registry_lifecycle_scope": "manual_project_registration_only",
        "survives_process_restart": false,
        "message": one_shot_task_message()
    })
}

fn phase1_contract_json(
    project_id: Option<&str>,
    lifecycle: Option<&serde_json::Value>,
    status: Option<&crate::types::IndexStatus>,
) -> serde_json::Value {
    let structural_generation = lifecycle
        .and_then(|value| value.get("structural"))
        .and_then(|value| value.get("generation"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let semantic_generation = lifecycle
        .and_then(|value| value.get("semantic"))
        .and_then(|value| value.get("generation"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0);

    let mut contract = with_surface_guidance(
        export_contract_meta(
            ExportIdentity {
                project_id: project_id.map(|id| id.to_string()),
                stable_node_ids: true,
                node_ids_are_project_scoped: true,
                stable_edge_ids: false,
                edge_ids_are_local_only: true,
                node_id_semantics: Some("stable_project_scoped_project_id".to_string()),
                edge_id_semantics: Some("no_public_edge_ids".to_string()),
                ..Default::default()
            },
            status,
        ),
        &["lifecycle", "contract"],
        &[
            "status",
            "total_files",
            "indexed_files",
            "total_chunks",
            "total_symbols",
        ],
        &[],
    );
    contract.generation_basis.structural_generation = structural_generation;
    contract.generation_basis.semantic_generation = semantic_generation;
    contract.projection.generation = semantic_generation;
    contract.projection.materialization.current_generation = semantic_generation;
    serde_json::to_value(contract).unwrap_or_else(|_| json!({}))
}

fn projection_locator_lifecycle() -> ProjectionLocatorLifecycle {
    ProjectionLocatorLifecycle {
        scope: "same_process_ephemeral_projection_registry".to_string(),
        same_process_only: true,
        survives_process_restart: false,
        survives_generation_change: false,
        client_persistable: false,
        generation_binding:
            "locator is bound to the semantic generation captured at projection creation time"
                .to_string(),
    }
}

fn projection_locator_record(
    locator: String,
    project_id: String,
    generation: u64,
    request: crate::types::ProjectProjectionRequest,
    lookup: ProjectionLocatorLookup,
) -> ProjectionLocatorRecord {
    ProjectionLocatorRecord {
        locator,
        locator_kind: "ephemeral_projection_handle".to_string(),
        project_id,
        generation,
        request,
        lifecycle: projection_locator_lifecycle(),
        lookup,
    }
}

pub async fn index_project(
    state: &Arc<AppState>,
    params: IndexProjectParams,
) -> anyhow::Result<CallToolResult> {
    let Some(request_path) = params.path.as_deref() else {
        return Ok(error_response(
            "path is required to start a new index job; resume-by-project/job is not implemented yet"
                .to_string(),
        ));
    };
    let path = std::path::Path::new(request_path);

    if !path.exists() {
        return Ok(error_response(format!("Path does not exist: {}", request_path)));
    }

    let project_id = match derive_project_id(path) {
        Ok(project_id) => project_id,
        Err(error) => {
            return Ok(error_response(format!("Invalid project path: {}", error)));
        }
    };
    let canonical_root_path = path
        .canonicalize()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());

    let resume_requested = params.resume.unwrap_or(false);
    let requested_job_id = params.job_id.clone();
    let requested_resume_token = params.resume_token.clone();

    let registry_lifecycle = match state
        .project_registry
        .ensure_project(project_id.clone(), path)
        .await
    {
        Ok(lifecycle) => lifecycle.status(),
        Err(error) => {
            return Ok(success_json(json!({
                "error": format!(
                    "Project registry rejected project {} at {}: {}",
                    project_id,
                    path.display(),
                    error
                ),
                "reason_code": error.reason_code(),
                "project_id": project_id,
                "path": path.display().to_string()
            })));
        }
    };
    let registry_lifecycle = registry_lifecycle_json(&registry_lifecycle);

    let force = params.force.unwrap_or(false);
    let confirm_failed_restart = params.confirm_failed_restart.unwrap_or(false);

    tracing::info!(
        path = %request_path,
        project_id = %project_id,
        force,
        confirm_failed_restart,
        "index_project request received"
    );

    if !(force && confirm_failed_restart) && !resume_requested {
        if let Ok(jobs) = state.storage.list_index_jobs_for_project(&project_id).await {
            if let Some(running_job) = jobs.iter().find(|j| j.state == IndexJobState::Running) {
                let index_job = index_job_json(state, running_job).await;
                return Ok(success_json(json!({
                    "project_id": project_id,
                    "root_path": canonical_root_path,
                    "status": "indexing",
                    "reason_code": "active_index_running",
                    "job_id": running_job.job_id,
                    "lifecycle": registry_lifecycle,
                    "index_job": index_job,
                    "message": "An indexing job is already running for this project. Wait for it to finish or use force=true and confirm_failed_restart=true to override."
                })));
            }
        }
    }

    // Check current status from DB (for informational purposes / force flag)
    if let Ok(Some(status)) = state.storage.get_index_status(&project_id).await {
        match status.status {
            crate::types::IndexState::Completed | crate::types::IndexState::EmbeddingPending => {
                if !force {
                    return Ok(success_json(json!({
                        "project_id": project_id,
                        "status": status.status.to_string(),
                        "total_files": status.total_files,
                        "indexed_files": status.indexed_files,
                        "total_chunks": status.total_chunks,
                        "lifecycle": registry_lifecycle,
                        "message": "Project already indexed. File changes are tracked incrementally. Use force=true to re-index from scratch."
                    })));
                }
                tracing::info!(project_id = %project_id, "Force re-indexing project");
            }
            crate::types::IndexState::Failed => {
                if !force || !confirm_failed_restart {
                    let root_path = status
                        .root_path
                        .clone()
                        .unwrap_or_else(|| canonical_root_path.clone());
                    return Ok(success_json(json!({
                        "project_id": project_id,
                        "root_path": root_path,
                        "status": status.status.to_string(),
                        "state": "blocked",
                        "can_retry": true,
                        "requires_force": true,
                        "requires_confirmation": true,
                        "recommended_action": "retry_with_force_and_confirmation",
                        "total_files": status.total_files,
                        "indexed_files": status.indexed_files,
                        "total_chunks": status.total_chunks,
                        "lifecycle": registry_lifecycle,
                        "message": "Previous indexing failed. Refusing to restart full indexing unless force=true and confirm_failed_restart=true are both provided.",
                        "error_message": status.error_message,
                        "failed_files": status.failed_files,
                        "recovery": {
                            "reason": "failed_index_restart_requires_explicit_confirmation",
                            "next_step": "Only retry after confirming the previous failure cause is understood and resources are sufficient.",
                            "example": {
                                "tool": "index_project",
                                "arguments": {
                                    "path": request_path,
                                    "force": true,
                                    "confirm_failed_restart": true
                                }
                            }
                        }
                    })));
                }

                tracing::info!(
                    project_id = %project_id,
                    error = ?status.error_message,
                    "Previous indexing failed, confirmed force re-indexing project"
                );
            }
            crate::types::IndexState::Indexing => {
                let background_task =
                    status_background_task_json(state, &project_id, &status).await;
                if is_lost_one_shot_indexing_task_after_restart(
                    state,
                    &project_id,
                    &status,
                    &background_task,
                    0,
                )
                .await
                    && (!force || !confirm_failed_restart)
                {
                    let root_path = status
                        .root_path
                        .clone()
                        .unwrap_or_else(|| canonical_root_path.clone());
                    return Ok(success_json(json!({
                        "project_id": project_id,
                        "root_path": root_path,
                        "status": "failed",
                        "state": "blocked",
                        "can_retry": true,
                        "retryable": true,
                        "requires_force": true,
                        "requires_confirmation": true,
                        "recommended_action": "retry_with_force_and_confirmation",
                        "reason_code": "lost_one_shot_indexing_task_after_restart",
                        "total_files": status.total_files,
                        "indexed_files": status.indexed_files,
                        "total_chunks": status.total_chunks,
                        "lifecycle": registry_lifecycle,
                        "background_task": background_task,
                        "recovery": lost_one_shot_recovery_json(&project_id),
                        "message": "Previous indexing was interrupted after process restart. Refusing to restart full indexing unless force=true and confirm_failed_restart=true are both provided.",
                        "error_message": status.error_message,
                        "failed_files": status.failed_files
                    })));
                }
            }
        }
    }

    // Atomic TOCTOU guard: insert returns false if project_id already present.
    // This is the authoritative gate — the DB status check above is only informational.
    // We must NOT hold the MutexGuard across any `.await` point (std::sync::Mutex is !Send).
    // So: lock → check+insert → drop guard → then await if needed.
    let already_indexing = {
        let mut guard = state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned");
        !guard.insert(project_id.clone())
    }; // guard dropped here — no awaits held

    if already_indexing {
        let background_task = one_shot_index_task_json(state, &project_id, "already_running").await;
        let index_job = latest_index_job_json(state, &project_id).await;
        // Already indexing — return current progress from DB
        let (total_files, indexed_files, total_chunks) = state
            .storage
            .get_index_status(&project_id)
            .await
            .ok()
            .flatten()
            .map(|s| (s.total_files, s.indexed_files, s.total_chunks))
            .unwrap_or((0, 0, 0));
        return Ok(success_json(json!({
            "project_id": project_id,
            "root_path": canonical_root_path,
            "status": "indexing",
            "reason_code": "active_index_running",
            "total_files": total_files,
            "indexed_files": indexed_files,
            "total_chunks": total_chunks,
            "lifecycle": registry_lifecycle,
            "index_job": index_job,
            "background_task": background_task,
            "message": "Indexing already in progress. Registry lifecycle only describes registry-managed workers; this manual index is tracked through index status/progress."
        })));
    }

    let monitor = state.progress.get_or_create(&project_id).await;
    let operation_id = new_index_operation_id(&project_id);
    tracing::info!(
        project_id = %project_id,
        root_path = %canonical_root_path,
        operation_id = %operation_id,
        force,
        confirm_failed_restart,
        "Queued one-shot code index task"
    );
    set_monitor_optional_string(&monitor.operation_id, Some(operation_id));
    set_monitor_string(&monitor.task_state, "queued");
    set_monitor_optional_string(&monitor.last_error, None);
    monitor
        .total_files
        .store(0, std::sync::atomic::Ordering::Relaxed);
    monitor
        .indexed_files
        .store(0, std::sync::atomic::Ordering::Relaxed);
    set_monitor_string(&monitor.current_file, "");
    let previous_status = state.storage.get_index_status(&project_id).await.ok().flatten();
    let resume_job = if resume_requested {
        let job_id = requested_job_id.as_deref().ok_or_else(|| anyhow::anyhow!("job_id is required when resume=true"))?;
        match state.storage.get_index_job(&project_id, job_id).await? {
            Some(job) => Some(job),
            None => {
                if params.allow_full_restart_fallback.unwrap_or(false) {
                    None
                } else {
                    if let Ok(mut guard) = state.indexing_projects.lock() {
                        guard.remove(&project_id);
                    }
                    return Ok(success_json(json!({
                        "project_id": project_id,
                        "job_id": job_id,
                        "state": "failed",
                        "can_resume": false,
                        "reason_code": "checkpoint_generation_missing",
                        "requires_force": true,
                        "requires_confirmation": true,
                    })));
                }
            }
        }
    } else {
        None
    };

    let structural_generation = resume_job
        .as_ref()
        .map(|job| job.target_generation)
        .unwrap_or_else(|| previous_status
        .as_ref()
        .map(|status| status.structural_generation.saturating_add(1))
        .unwrap_or(1));
    let mut durable_job = resume_job.unwrap_or_else(|| durable_index_job_record(&project_id, &canonical_root_path, structural_generation));
    durable_job.state = IndexJobState::Running;
    durable_job.started_at = durable_job.started_at.or_else(|| Some(crate::types::Datetime::default()));
    durable_job.updated_at = crate::types::Datetime::default();
    durable_job.reason_code = None;
    durable_job.error = None;
    // `operation_id` is same-process-only progress metadata. `job_id` is durable
    // storage identity and intentionally exists before file enumeration starts.
    if let Err(error) = state.storage.create_or_update_index_job(&durable_job).await {
        if let Ok(mut guard) = state.indexing_projects.lock() {
            guard.remove(&project_id);
        }
        set_monitor_string(&monitor.task_state, "failed");
        set_monitor_optional_string(&monitor.last_error, Some(error.to_string()));
        return Ok(error_response(format!(
            "failed to create durable index job before file enumeration: {error}"
        )));
    }
    let background_task = monitor_one_shot_index_task_json(&monitor);
    let index_job = index_job_json_with_operation(
        state,
        &durable_job,
        read_monitor_optional_string(&monitor.operation_id),
    ).await;

    // Spawn indexing in background
    let state_clone = state.clone();
    let path_clone = request_path.to_string();
    let project_id_for_cleanup = project_id.clone();
    let durable_job_for_task = durable_job.clone();
    let filter_config_opt = if params.include_patterns.is_some() || params.exclude_patterns.is_some() {
        let include = params.include_patterns.unwrap_or_else(|| state.config.code_index.include_patterns.clone());
        let exclude = params.exclude_patterns.unwrap_or_else(|| state.config.code_index.exclude_patterns.clone());
        Some(crate::codebase::scanner::IndexFilterConfig {
            include_patterns: include,
            exclude_patterns: exclude,
        })
    } else {
        None
    };

    tokio::spawn(async move {
        if let Some(monitor) = state_clone.progress.get(&project_id_for_cleanup).await {
            set_monitor_string(&monitor.task_state, "running");
            tracing::info!(
                project_id = %project_id_for_cleanup,
                operation_id = read_monitor_optional_string(&monitor.operation_id)
                    .as_deref()
                    .unwrap_or(""),
                "One-shot code index task running"
            );
        }
        let path = std::path::Path::new(&path_clone);
        let resume_options = crate::codebase::indexer::IndexResumeOptions {
            resume: resume_requested,
            job_id: requested_job_id.clone(),
            resume_token: requested_resume_token.clone(),
        };
        match if let Some(filter_config) = filter_config_opt {
            crate::codebase::indexer::index_project_after_admission_with_resume_and_filter(
                state_clone.clone(),
                path,
                resume_options,
                filter_config,
            )
            .await
        } else {
            crate::codebase::indexer::index_project_after_admission_with_resume(
                state_clone.clone(),
                path,
                resume_options,
            )
            .await
        }
        {
            Ok(status) => {
                let operation_id = state_clone
                    .progress
                    .get(&project_id_for_cleanup)
                    .await
                    .and_then(|monitor| read_monitor_optional_string(&monitor.operation_id));
                if let Some(monitor) = state_clone.progress.get(&project_id_for_cleanup).await {
                    set_monitor_string(&monitor.task_state, "completed");
                    set_monitor_optional_string(&monitor.last_error, None);
                }
                let mut completed_job = durable_job_for_task.clone();
                completed_job.state = IndexJobState::Completed;
                completed_job.phase = IndexJobPhase::Finalize;
                completed_job.stored_phase = Some(IndexJobPhase::Finalize);
                completed_job.updated_at = crate::types::Datetime::default();
                completed_job.completed_at = Some(crate::types::Datetime::default());
                completed_job.completed_files_count = u64::from(status.indexed_files);
                completed_job.total_files_count = Some(u64::from(status.total_files));
                completed_job.progress.total_files = Some(status.total_files);
                completed_job.progress.discovered_files = Some(status.total_files);
                completed_job.progress.indexed_files = Some(status.indexed_files);
                completed_job.reason_code = None;
                completed_job.error = None;
                if let Err(error) = state_clone
                    .storage
                    .create_or_update_index_job(&completed_job)
                    .await
                {
                    tracing::error!(
                        project_id = %status.project_id,
                        job_id = %completed_job.job_id,
                        error = %error,
                        "Failed to mark durable index job completed"
                    );
                }
                tracing::info!(
                    project_id = %status.project_id,
                    operation_id = operation_id.as_deref().unwrap_or(""),
                    files = status.indexed_files,
                    chunks = status.total_chunks,
                    symbols = status.total_symbols,
                    "Indexing completed"
                );
            }
            Err(e) => {
                let operation_id = state_clone
                    .progress
                    .get(&project_id_for_cleanup)
                    .await
                    .and_then(|monitor| read_monitor_optional_string(&monitor.operation_id));
                if let Some(monitor) = state_clone.progress.get(&project_id_for_cleanup).await {
                    set_monitor_string(&monitor.task_state, "failed");
                    set_monitor_optional_string(&monitor.last_error, Some(e.to_string()));
                }
                let mut failed_job = durable_job_for_task.clone();
                let checkpoints = state_clone
                    .storage
                    .list_file_checkpoints_for_job(&project_id_for_cleanup, failed_job.target_generation)
                    .await
                    .unwrap_or_default();
                let completed_files = checkpoints.iter().filter(|checkpoint| checkpoint.completed).count() as u64;
                if completed_files > 0 {
                    failed_job.state = IndexJobState::Resumable;
                    failed_job.reason_code = Some(IndexJobReasonCode::ResumableInterruptedJob);
                    failed_job.resume_token = checkpoint_resume_token(&IndexJobPhase::Parse, completed_files);
                    failed_job.resume = Some(IndexJobResumeState {
                        supported: true,
                        token: Some(failed_job.resume_token.clone()),
                        checkpoint_generation: Some(failed_job.target_generation),
                        reason_if_not_supported: None,
                    });
                } else {
                    failed_job.state = IndexJobState::Failed;
                    failed_job.reason_code = Some(IndexJobReasonCode::Unknown);
                }
                failed_job.updated_at = crate::types::Datetime::default();
                failed_job.completed_at = Some(crate::types::Datetime::default());
                failed_job.completed_files_count = completed_files;
                failed_job.error = Some(IndexJobError {
                    code: failed_job.reason_code.clone().unwrap_or(IndexJobReasonCode::Unknown),
                    message: e.to_string(),
                    retryable: true,
                });
                if let Err(error) = state_clone
                    .storage
                    .create_or_update_index_job(&failed_job)
                    .await
                {
                    tracing::error!(
                        project_id = %project_id_for_cleanup,
                        job_id = %failed_job.job_id,
                        error = %error,
                        "Failed to mark durable index job failed"
                    );
                }
                tracing::error!(
                    project_id = %project_id_for_cleanup,
                    operation_id = operation_id.as_deref().unwrap_or(""),
                    error = %e,
                    "Indexing failed"
                );
            }
        }
        // Release the atomic lock regardless of outcome
        if let Ok(mut guard) = state_clone.indexing_projects.lock() {
            guard.remove(&project_id_for_cleanup);
        }
    });

    // Return immediately
    Ok(success_json(json!({
        "project_id": project_id,
        "root_path": canonical_root_path,
        "status": "indexing",
        "job_id": durable_job.job_id,
        "operation_id": background_task.get("operation_id").cloned().unwrap_or(serde_json::Value::Null),
        "can_resume": false,
        "reason_code": null,
        "lifecycle": registry_lifecycle,
        "index_job": index_job,
        "background_task": background_task,
        "message": "Indexing queued in a one-shot background task. Use project_info(action=\"status\") or client alias project_status(action=\"status\") to track operation_id/state/progress; registry lifecycle only describes registry-managed workers."
    })))
}

pub async fn get_index_status(
    state: &Arc<AppState>,
    params: GetIndexStatusParams,
) -> anyhow::Result<CallToolResult> {
    let project_id = params.project_id.trim().to_string();
    if project_id.is_empty() {
        return Ok(error_response("project_id required for status action"));
    }

    match state.storage.get_index_status(&project_id).await {
        Ok(Some(mut status)) => {
            status.refresh_lifecycle_states();
            let background_task = status_background_task_json(state, &project_id, &status).await;
            let mut current_file: Option<String> = None;

            // Always try to fetch current_file from monitor if available, even if failed or stuck
            if let Some(monitor) = state.progress.get(&project_id).await {
                if let Ok(cf) = monitor.current_file.read() {
                    if !cf.is_empty() {
                        current_file = Some(cf.clone());
                    }
                }

                // If we are actively indexing, update the progress counters
                if status.status == crate::types::IndexState::Indexing {
                    let indexed = monitor
                        .indexed_files
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let total = monitor
                        .total_files
                        .load(std::sync::atomic::Ordering::Relaxed);

                    if indexed > 0 {
                        status.indexed_files = std::cmp::max(status.indexed_files, indexed);
                    }
                    if total > 0 {
                        status.total_files = std::cmp::max(status.total_files, total);
                    }
                }
            }

            // Use manifest entry count as the authoritative "indexed files" number.
            let indexed_files = state
                .storage
                .count_manifest_entries(&project_id)
                .await
                .unwrap_or(0) as u32;

            // Sync queue status from shared AtomicUsize counter.
            let sync_queue_size = {
                let map = state.index_pending.read().await;
                map.get(&project_id)
                    .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
                    .unwrap_or(0)
            };
            let is_syncing = sync_queue_size > 0;
            let active_generation = state
                .storage
                .get_active_generation(&project_id)
                .await
                .ok()
                .flatten();

            let total_symbols = state
                .storage
                .count_symbols(&project_id, active_generation)
                .await
                .unwrap_or(0);
            let total_chunks = state
                .storage
                .count_chunks(&project_id, active_generation)
                .await
                .unwrap_or(0);
            let embedded_symbols = state
                .storage
                .count_embedded_symbols(&project_id, active_generation)
                .await
                .unwrap_or(0);
            let embedded_chunks = state
                .storage
                .count_embedded_chunks(&project_id, active_generation)
                .await
                .unwrap_or(0);

            let vector_progress = if total_chunks > 0 {
                (embedded_chunks as f32 / total_chunks as f32) * 100.0
            } else {
                0.0
            };
            let graph_progress = if total_symbols > 0 {
                (embedded_symbols as f32 / total_symbols as f32) * 100.0
            } else {
                0.0
            };

            // Two-phase progress: parsing (70%) + embedding (30%).
            //
            // During parsing, total_chunks/total_symbols grow dynamically as
            // the parser produces output.  The embedding worker typically keeps
            // pace, so  embedded/total ≈ 1.0  —  which made the OLD formula
            // report ~98% when only a fraction of files had been parsed.
            //
            // Fix: anchor progress to the file-parsing ratio (known up-front)
            // and let embedding fill a smaller tail portion.
            //
            //   overall = parse_ratio × (PARSE_W + embed_ratio × EMBED_W) × 100
            //
            // Properties:
            //   • Monotonically increasing (parse_ratio only grows; embed_ratio ≈ 1)
            //   • Continuous at the phase boundary (parse_ratio = 1 ⇒ formula
            //     becomes  (0.70 + embed_ratio × 0.30) × 100 )
            //   • Accurate: 50% files parsed + embedder keeping up  →  ~50%
            const PARSE_WEIGHT: f32 = 0.70;
            const EMBED_WEIGHT: f32 = 0.30;

            let parse_ratio = if status.total_files > 0 {
                (indexed_files as f32 / status.total_files as f32).min(1.0)
            } else {
                0.0
            };

            let embed_ratio = if (total_chunks + total_symbols) > 0 {
                (embedded_chunks + embedded_symbols) as f32 / (total_chunks + total_symbols) as f32
            } else {
                // Nothing to embed (yet or ever) — treat as fully caught up
                1.0
            };

            let overall_progress =
                parse_ratio * (PARSE_WEIGHT + embed_ratio * EMBED_WEIGHT) * 100.0;
            let lost_one_shot_after_restart = is_lost_one_shot_indexing_task_after_restart(
                state,
                &project_id,
                &status,
                &background_task,
                sync_queue_size,
            )
            .await;
            let durable_index_job = latest_index_job_json(state, &project_id).await;
            let index_job = if lost_one_shot_after_restart && durable_index_job.is_none() {
                lost_one_shot_index_job_json()
            } else {
                durable_index_job.unwrap_or(serde_json::Value::Null)
            };

            let parsing_done = status.total_files > 0 && indexed_files >= status.total_files;
            let indexing_message = match status.status {
                crate::types::IndexState::Completed => None,
                crate::types::IndexState::Failed => status.error_message.clone().or_else(|| {
                    Some("Indexing failed. Status and counts are incomplete.".to_string())
                }),
                crate::types::IndexState::Indexing | crate::types::IndexState::EmbeddingPending => {
                    Some("Indexing in progress. Status and counts may still change.".to_string())
                }
            };

            let serving_meta = state
                .storage
                .get_serving_metadata(&project_id)
                .await
                .unwrap_or_default();
    let explicit_indexing_gen = state
        .storage
        .get_indexing_generation(&project_id)
        .await
        .ok()
        .flatten();
    let abandoned_max = state
        .storage
        .list_abandoned_generations(&project_id)
        .await
        .ok()
        .and_then(|gens| gens.into_iter().filter(|gen| Some(*gen) != serving_meta.structural).max());
    let indexing_gen = explicit_indexing_gen.or(abandoned_max).or(serving_meta.structural);
            let is_indexing = status.status == crate::types::IndexState::Indexing;
            let is_interrupted = match (abandoned_max, serving_meta.structural) {
                (Some(a), Some(s)) => a > s,
                _ => false,
            };
            let capability_block = project_info_capability_block(&serving_meta, indexing_gen, is_indexing, is_interrupted);

            Ok(success_json(json!({
                "project_id": status.project_id,
                "root_path": status.root_path,
                "status": if lost_one_shot_after_restart { "failed".to_string() } else { status.status.to_string() },
                "retryable": if lost_one_shot_after_restart { Some(true) } else { None },
                "reason_code": if lost_one_shot_after_restart { Some("lost_one_shot_indexing_task_after_restart") } else { None },
                "recovery": if lost_one_shot_after_restart { Some(lost_one_shot_recovery_json(&project_id)) } else { None },
                "code_intelligence": if lost_one_shot_after_restart {
                    CodeIntelligenceDiagnostic::degraded(
                        "Code intelligence indexing was interrupted after restart; retry with force=true and confirm_failed_restart=true."
                    ).as_json()
                } else {
                    project_state_diagnostic(status.status.clone(), false).as_json()
                },
                "contract": if lost_one_shot_after_restart {
                    lost_one_shot_contract_json(&status)
                } else {
                    phase1_contract_json(
                        Some(&status.project_id),
                        Some(&lifecycle_json(&status)),
                        Some(&status),
                    )
                },
                "summary": if lost_one_shot_after_restart {
                    lost_one_shot_indexing_summary(
                        &status,
                        indexed_files,
                        total_chunks,
                        total_symbols,
                        overall_progress,
                    )
                } else {
                    index_status_summary(
                        &status,
                        indexed_files,
                        total_chunks,
                        total_symbols,
                        overall_progress,
                        indexing_message,
                    )
                },
                "is_syncing": is_syncing,
                "sync_queue_size": sync_queue_size,
                "total_files": status.total_files,
                "indexed_files": indexed_files,
                "started_at": status.started_at,
                "completed_at": status.completed_at,
                "lifecycle": lifecycle_json(&status),
                "index_job": index_job,
                "background_task": background_task,
                "files": {
                    "total": status.total_files,
                    "indexed": indexed_files,
                    "parse_percent": format!("{:.1}", parse_ratio * 100.0)
                },
                "chunks": {
                    "total": total_chunks,
                    "embedded": embedded_chunks,
                    "progress_percent": format!("{:.1}", vector_progress)
                },
                "symbols": {
                    "total": total_symbols,
                    "embedded": embedded_symbols,
                    "progress_percent": format!("{:.1}", graph_progress)
                },

                "parsing": {
                    "status": if lost_one_shot_after_restart { "failed" } else if indexed_files >= status.total_files { "completed" } else { "in_progress" },
                    "progress": format!("{}/{}", indexed_files, status.total_files),
                    "current_file": current_file
                },

                "vector_embeddings": {
                    "status": if lost_one_shot_after_restart { "pending" } else if status.status == crate::types::IndexState::Completed
                        || (parsing_done && embedded_chunks >= total_chunks && total_chunks > 0)
                        { "completed" } else { "in_progress" },
                    "total": total_chunks,
                    "completed": embedded_chunks,
                    "percent": format!("{:.1}", vector_progress)
                },

                "graph_embeddings": {
                    "status": if lost_one_shot_after_restart { "pending" } else if status.status == crate::types::IndexState::Completed
                        || (parsing_done && embedded_symbols >= total_symbols && total_symbols > 0)
                        { "completed" } else { "in_progress" },
                    "total": total_symbols,
                    "completed": embedded_symbols,
                    "percent": format!("{:.1}", graph_progress)
                },

                "overall_progress": {
                    "percent": format!("{:.1}", overall_progress),
                    "is_complete": !lost_one_shot_after_restart
                        && (status.status == crate::types::IndexState::Completed
                        || (parsing_done
                            && embedded_chunks >= total_chunks
                            && embedded_symbols >= total_symbols
                            && total_chunks > 0))
                },
                "error_message": status.error_message,
                "failed_files": status.failed_files,
                "serving": capability_block["serving"].clone(),
                "indexing_generation": capability_block["indexing_generation"].clone(),
                "capabilities": capability_block["capabilities"].clone()
            })))
        }
        Ok(None) => {
            let total_symbols = state.storage.count_symbols(&project_id, None).await.unwrap_or(0);
            let total_chunks = state.storage.count_chunks(&project_id, None).await.unwrap_or(0);
            let indexed_files = state
                .storage
                .count_manifest_entries(&project_id)
                .await
                .unwrap_or(0) as u32;

            if total_chunks == 0 && total_symbols == 0 && indexed_files == 0 {
                if let Some(monitor) = state.progress.get(&project_id).await {
                    let background_task = monitor_one_shot_index_task_json(&monitor);
                    let index_job = latest_index_job_json(state, &project_id)
                        .await
                        .unwrap_or(serde_json::Value::Null);
                    return Ok(success_json(json!({
                        "project_id": project_id,
                        "root_path": null,
                        "status": "indexing",
                        "code_intelligence": CodeIntelligenceDiagnostic::indexing(
                            "Manual indexing has been queued but persistent index status has not been written yet."
                        ).as_json(),
                        "summary": summary_index_status_response(
                            0,
                            0,
                            0,
                            0,
                            0.0,
                            true,
                            Some("Indexing queued. Persistent status will appear after the one-shot task starts.".to_string()),
                        ),
                        "index_job": index_job,
                        "background_task": background_task,
                        "parsing": {
                            "status": "queued",
                            "progress": "0/0",
                            "current_file": null
                        },
                        "vector_embeddings": {
                            "status": "pending",
                            "total": 0,
                            "completed": 0,
                            "percent": "0.0"
                        },
                        "graph_embeddings": {
                            "status": "pending",
                            "total": 0,
                            "completed": 0,
                            "percent": "0.0"
                        },
                        "overall_progress": {
                            "percent": "0.0",
                            "is_complete": false
                        }
                    })));
                }

                return Ok(success_json(json!({
                    "error": format!("Project not found: {}", project_id),
                    "code_intelligence": runtime_root_diagnostic().as_json()
                })));
            }

            let mut status = IndexStatus::new(project_id.clone());
            status.status = IndexState::Failed;
            status.total_files = indexed_files;
            status.indexed_files = indexed_files;
            status.total_chunks = total_chunks as u32;
            status.total_symbols = total_symbols as u32;
            status.error_message = Some(
                "Index status metadata is missing while code intelligence rows exist; re-run index_project with force=true and confirm_failed_restart=true to rebuild metadata."
                    .to_string(),
            );
            status.refresh_lifecycle_states();
            let background_task = status_background_task_json(state, &project_id, &status).await;

            let embedded_symbols = state
                .storage
                .count_embedded_symbols(&project_id, None)
                .await
                .unwrap_or(0);
            let embedded_chunks = state
                .storage
                .count_embedded_chunks(&project_id, None)
                .await
                .unwrap_or(0);
            let sync_queue_size = {
                let map = state.index_pending.read().await;
                map.get(&project_id)
                    .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
                    .unwrap_or(0)
            };
            let vector_progress = if total_chunks > 0 {
                (embedded_chunks as f32 / total_chunks as f32) * 100.0
            } else {
                0.0
            };
            let graph_progress = if total_symbols > 0 {
                (embedded_symbols as f32 / total_symbols as f32) * 100.0
            } else {
                0.0
            };
            let indexing_message = status.error_message.clone();

            Ok(success_json(json!({
                "project_id": project_id,
                "root_path": status.root_path,
                "status": status.status.to_string(),
                "code_intelligence": project_state_diagnostic(status.status.clone(), true).as_json(),
                "contract": phase1_contract_json(
                    Some(&status.project_id),
                    Some(&lifecycle_json(&status)),
                    Some(&status),
                ),
                "summary": summary_index_status_response(
                    status.total_files,
                    indexed_files,
                    total_chunks,
                    total_symbols,
                    0.0,
                    true,
                    indexing_message,
                ),
                "diagnostics": {
                    "status_metadata_missing": true,
                    "reason_code": "degraded",
                    "message": status.error_message.clone()
                },
                "is_syncing": sync_queue_size > 0,
                "sync_queue_size": sync_queue_size,
                "total_files": status.total_files,
                "indexed_files": indexed_files,
                "started_at": status.started_at,
                "completed_at": status.completed_at,
                "lifecycle": lifecycle_json(&status),
                "background_task": background_task,
                "parsing": {
                    "status": "degraded",
                    "progress": format!("{}/{}", indexed_files, status.total_files),
                    "current_file": null
                },
                "vector_embeddings": {
                    "status": if total_chunks > 0 && embedded_chunks >= total_chunks { "completed" } else { "pending" },
                    "total": total_chunks,
                    "completed": embedded_chunks,
                    "percent": format!("{:.1}", vector_progress)
                },
                "graph_embeddings": {
                    "status": if total_symbols > 0 && embedded_symbols >= total_symbols { "completed" } else { "pending" },
                    "total": total_symbols,
                    "completed": embedded_symbols,
                    "percent": format!("{:.1}", graph_progress)
                },
                "overall_progress": {
                    "percent": "0.0",
                    "is_complete": false
                },
                "error_message": status.error_message.clone(),
                "failed_files": status.failed_files
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn list_projects(
    state: &Arc<AppState>,
    _params: ListProjectsParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.list_projects().await {
        Ok(projects) => {
            let project_stats = state.storage.get_all_project_stats().await.unwrap_or_default();
            let mut enriched = Vec::with_capacity(projects.len());
            let mut has_ready = false;
            let mut has_indexing = false;
            let mut has_degraded = false;

            for project_id in &projects {
                let mut status = state
                    .storage
                    .get_index_status(project_id)
                    .await
                    .ok()
                    .flatten();
                if let Some(status) = status.as_mut() {
                    status.refresh_lifecycle_states();
                }
                let stats = project_stats.get(project_id).cloned().unwrap_or_default();
                let chunks = stats.chunks;
                let symbols = stats.symbols;
                let embedded_chunks = stats.embedded_chunks;
                let embedded_symbols = stats.embedded_symbols;

                let status_str = status
                    .as_ref()
                    .map(|s| s.status.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                if let Some(status) = status.as_ref() {
                    match status.status {
                        IndexState::Completed => has_ready = true,
                        IndexState::Indexing | IndexState::EmbeddingPending => has_indexing = true,
                        IndexState::Failed => has_degraded = true,
                    }
                } else if chunks > 0 || symbols > 0 {
                    has_degraded = true;
                }
                let lifecycle = status
                    .as_ref()
                    .map(lifecycle_json)
                    .unwrap_or_else(|| json!({
                        "structural": { "state": "pending", "is_ready": false, "generation": 0 },
                        "semantic": { "state": "pending", "is_ready": false, "generation": 0, "is_caught_up": true },
                        "projection": { "state": "stale", "is_current": false }
                    }));
                let status_is_complete = status
                    .as_ref()
                    .map(|s| s.status == crate::types::IndexState::Completed)
                    .unwrap_or(false);
                let status_summary_message = match status.as_ref().map(|s| &s.status) {
                    Some(IndexState::Completed) => None,
                    Some(IndexState::Failed) => Some(
                        "Project indexing failed. Results are incomplete until a force rebuild succeeds."
                            .to_string(),
                    ),
                    Some(IndexState::Indexing) | Some(IndexState::EmbeddingPending) => {
                        Some("Project indexing is still in progress.".to_string())
                    }
                    None if chunks > 0 || symbols > 0 => Some(
                        "Index status metadata is missing while code intelligence rows exist."
                            .to_string(),
                    ),
                    None => None,
                };

                enriched.push(json!({
                    "id": project_id,
                    "project_id": project_id,
                    "root_path": status.as_ref().and_then(|status| status.root_path.clone()),
                    "status": status_str,
                    "lifecycle": lifecycle.clone(),
                    "contract": phase1_contract_json(Some(project_id), Some(&lifecycle), status.as_ref()),
                    "summary": summary_collection_response(
                        "project",
                        chunks as usize,
                        Some((chunks + symbols) as usize),
                        !status_is_complete,
                        status_summary_message,
                    ),
                    "chunks": chunks,
                    "symbols": symbols,
                    "embedded_chunks": embedded_chunks,
                    "embedded_symbols": embedded_symbols,
                    "diagnostics": {
                        "status_metadata_missing": status.is_none() && (chunks > 0 || symbols > 0),
                        "reason_code": if status.is_none() && (chunks > 0 || symbols > 0) { "degraded" } else { "ok" },
                        "message": if status.is_none() && (chunks > 0 || symbols > 0) {
                            Some("Index status metadata is missing while code intelligence rows exist; force rebuild writes a staged generation before promoting it active.".to_string())
                        } else {
                            None
                        }
                    }
                }));
            }

            let aggregate = if has_indexing {
                CodeIntelligenceDiagnostic::indexing("At least one project is still indexing.")
            } else if has_degraded {
                CodeIntelligenceDiagnostic::degraded("At least one project is degraded.")
            } else if has_ready {
                CodeIntelligenceDiagnostic::ready("Code intelligence projects are ready.")
            } else {
                runtime_root_diagnostic()
            };

            Ok(success_json(json!({
                "projects": enriched,
                "count": projects.len(),
                "code_intelligence": aggregate.as_json()
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn delete_project(
    state: &Arc<AppState>,
    params: DeleteProjectParams,
) -> anyhow::Result<CallToolResult> {
    let project_id = params.project_id;

    let _ = state.storage.delete_project_symbols(&project_id).await;

    let _ = state.storage.delete_index_status(&project_id).await;
    let _ = state.storage.delete_file_hashes(&project_id).await;
    let _ = state.storage.delete_manifest_entries(&project_id).await;

    // Remove from in-memory BM25 index
    state.code_search.remove_project(&project_id).await;
    state.project_registry.remove(&project_id).await;
    state.progress.remove(&project_id).await;
    if let Ok(mut guard) = state.indexing_projects.lock() {
        guard.remove(&project_id);
    }

    match state.storage.delete_project_chunks(&project_id).await {
        Ok(deleted) => Ok(success_json(json!({
            "deleted_chunks": deleted,
            "project_id": project_id
        }))),
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn get_project_stats(
    state: &Arc<AppState>,
    params: GetProjectStatsParams,
) -> anyhow::Result<CallToolResult> {
    let project_id = params.project_id.trim().to_string();
    if project_id.is_empty() {
        return Ok(error_response("project_id required for stats action"));
    }

    let total_symbols = state.storage.count_symbols(&project_id, None).await.unwrap_or(0);
    let total_chunks = state.storage.count_chunks(&project_id, None).await.unwrap_or(0);
    let embedded_symbols = state
        .storage
        .count_embedded_symbols(&project_id, None)
        .await
        .unwrap_or(0);
    let embedded_chunks = state
        .storage
        .count_embedded_chunks(&project_id, None)
        .await
        .unwrap_or(0);

    // Use manifest entry count as the authoritative "indexed files" number.
    let indexed_files = state
        .storage
        .count_manifest_entries(&project_id)
        .await
        .unwrap_or(0) as u32;

    let status = state.storage.get_index_status(&project_id).await?;
    let status_metadata_missing = status.is_none();

    let mut status = match status {
        Some(status) => status,
        None => {
            if total_chunks == 0 && total_symbols == 0 && indexed_files == 0 {
                return Ok(success_json(json!({
                    "error": format!("Project not found: {}", project_id),
                    "code_intelligence": runtime_root_diagnostic().as_json()
                })));
            }

            let early_serving = state.storage.get_serving_metadata(&project_id).await.unwrap_or_default();

            let mut status = IndexStatus::new(project_id.clone());
            status.status = if early_serving.structural.is_some() {
                IndexState::Completed
            } else {
                IndexState::Failed
            };
            status.total_files = indexed_files;
            status.indexed_files = indexed_files;
            status.total_chunks = total_chunks as u32;
            status.total_symbols = total_symbols as u32;
            if early_serving.structural.is_none() {
                status.error_message = Some(
                    "Index status metadata is missing while code intelligence rows exist; re-run index_project with force=true and confirm_failed_restart=true to rebuild metadata."
                        .to_string(),
                );
            }
            status
        }
    };
    status.refresh_lifecycle_states();
    let background_task = status_background_task_json(state, &project_id, &status).await;

    // Sync queue status from shared AtomicUsize counter.
    let sync_queue_size = {
        let map = state.index_pending.read().await;
        map.get(&project_id)
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    };
    let is_syncing = sync_queue_size > 0;

    let vector_progress = if total_chunks > 0 {
        (embedded_chunks as f32 / total_chunks as f32) * 100.0
    } else {
        0.0
    };
    let graph_progress = if total_symbols > 0 {
        (embedded_symbols as f32 / total_symbols as f32) * 100.0
    } else {
        0.0
    };

    // Two-phase overall progress (same formula as get_index_status)
    const PARSE_WEIGHT: f32 = 0.70;
    const EMBED_WEIGHT: f32 = 0.30;

    let parse_ratio = if status.total_files > 0 {
        (indexed_files as f32 / status.total_files as f32).min(1.0)
    } else {
        0.0
    };
    let embed_ratio = if (total_chunks + total_symbols) > 0 {
        (embedded_chunks + embedded_symbols) as f32 / (total_chunks + total_symbols) as f32
    } else {
        1.0
    };
    let overall_progress = parse_ratio * (PARSE_WEIGHT + embed_ratio * EMBED_WEIGHT) * 100.0;
    let lost_one_shot_after_restart = is_lost_one_shot_indexing_task_after_restart(
        state,
        &project_id,
        &status,
        &background_task,
        sync_queue_size,
    )
    .await;
    let durable_index_job = latest_index_job_json(state, &project_id).await;
    let index_job = if lost_one_shot_after_restart && durable_index_job.is_none() {
        lost_one_shot_index_job_json()
    } else {
        durable_index_job.unwrap_or(serde_json::Value::Null)
    };
    let indexing_message = if status_metadata_missing {
        status.error_message.clone()
    } else {
        match status.status {
            IndexState::Completed => None,
            IndexState::Failed => status
                .error_message
                .clone()
                .or_else(|| Some("Indexing failed. Project stats are incomplete.".to_string())),
            IndexState::Indexing | IndexState::EmbeddingPending => {
                Some("Indexing in progress. Project stats may still change.".to_string())
            }
        }
    };

    let serving_meta = state
        .storage
        .get_serving_metadata(&project_id)
        .await
        .unwrap_or_default();
    let explicit_indexing_gen = state
        .storage
        .get_indexing_generation(&project_id)
        .await
        .ok()
        .flatten();
    let abandoned_max = state
        .storage
        .list_abandoned_generations(&project_id)
        .await
        .ok()
        .and_then(|gens| gens.into_iter().filter(|gen| Some(*gen) != serving_meta.structural).max());
    let indexing_gen = explicit_indexing_gen.or(abandoned_max).or(serving_meta.structural);
    let is_indexing_stats = status.status == IndexState::Indexing;
    let is_interrupted_stats = match (abandoned_max, serving_meta.structural) {
        (Some(a), Some(s)) => a > s,
        _ => false,
    };
    let capability_block = project_info_capability_block(&serving_meta, indexing_gen, is_indexing_stats, is_interrupted_stats);

    Ok(success_json(json!({
        "project_id": project_id,
        "root_path": status.root_path,
        "status": if lost_one_shot_after_restart { "failed".to_string() } else { status.status.to_string() },
        "retryable": if lost_one_shot_after_restart { Some(true) } else { None },
        "reason_code": if lost_one_shot_after_restart { Some("lost_one_shot_indexing_task_after_restart") } else { None },
        "recovery": if lost_one_shot_after_restart { Some(lost_one_shot_recovery_json(&project_id)) } else { None },
        "code_intelligence": if lost_one_shot_after_restart {
            CodeIntelligenceDiagnostic::degraded(
                "Code intelligence indexing was interrupted after restart; retry with force=true and confirm_failed_restart=true."
            ).as_json()
        } else {
            project_state_diagnostic(status.status.clone(), status_metadata_missing).as_json()
        },
        "contract": if lost_one_shot_after_restart {
            lost_one_shot_contract_json(&status)
        } else {
            phase1_contract_json(
                Some(&status.project_id),
                Some(&lifecycle_json(&status)),
                Some(&status),
            )
        },
        "summary": if lost_one_shot_after_restart {
            lost_one_shot_indexing_summary(
                &status,
                indexed_files,
                total_chunks,
                total_symbols,
                overall_progress,
            )
        } else {
            index_status_summary(
                &status,
                indexed_files,
                total_chunks,
                total_symbols,
                overall_progress,
                indexing_message,
            )
        },
        "diagnostics": {
            "status_metadata_missing": status_metadata_missing,
            "reason_code": if lost_one_shot_after_restart { "lost_one_shot_indexing_task_after_restart" } else if status_metadata_missing { "degraded" } else { "ok" },
            "message": if lost_one_shot_after_restart {
                Some("Persisted indexing lost its same-process one-shot task after restart.".to_string())
            } else if status_metadata_missing { status.error_message.clone() } else { None }
        },
        "is_syncing": is_syncing,
        "sync_queue_size": sync_queue_size,
        "lifecycle": lifecycle_json(&status),
        "index_job": index_job,
        "background_task": background_task,
        "files": {
            "total": status.total_files,
            "indexed": indexed_files,
            "parse_percent": format!("{:.1}", parse_ratio * 100.0)
        },
        "chunks": {
            "total": total_chunks,
            "embedded": embedded_chunks,
            "progress_percent": format!("{:.1}", vector_progress)
        },
        "symbols": {
            "total": total_symbols,
            "embedded": embedded_symbols,
            "progress_percent": format!("{:.1}", graph_progress)
        },
        "overall_progress_percent": format!("{:.1}", overall_progress),
        "overall_progress": {
            "percent": format!("{:.1}", overall_progress),
            "is_complete": !lost_one_shot_after_restart && status.status == crate::types::IndexState::Completed
        },
        "started_at": status.started_at,
        "completed_at": status.completed_at,
        "failed_files": status.failed_files,
        "serving": capability_block["serving"].clone(),
        "serving_generation": serving_meta.structural.or(serving_meta.bm25).or(serving_meta.vector),
        "indexing_generation": capability_block["indexing_generation"].clone(),
        "capabilities": capability_block["capabilities"].clone(),
        "capability_readiness": {
            "serving_generation": serving_meta.structural.or(serving_meta.bm25).or(serving_meta.vector),
            "indexing_generation": capability_block["indexing_generation"].clone(),
            "capabilities": capability_block["capabilities"].clone(),
        }
    })))
}

pub async fn get_project_projection(
    state: &Arc<AppState>,
    params: GetProjectProjectionParams,
) -> anyhow::Result<CallToolResult> {
    let status = state.storage.get_index_status(&params.project_id).await?;

    if status.is_none() {
        return Ok(error_response(format!(
            "Project not found: {}",
            params.project_id
        )));
    }

    let mut status = status.unwrap();
    status.refresh_lifecycle_states();

    let total_files = status.total_files;
    let indexed_files = state
        .storage
        .count_manifest_entries(&params.project_id)
        .await
        .unwrap_or(0) as u32;
    let total_chunks = state
        .storage
        .count_chunks(&params.project_id, None)
        .await
        .unwrap_or(0);
    let total_symbols = state
        .storage
        .count_symbols(&params.project_id, None)
        .await
        .unwrap_or(0);
    let symbols = state
        .storage
        .get_project_symbols(&params.project_id, None)
        .await
        .unwrap_or_default();

    let symbol_ids: Vec<String> = symbols
        .iter()
        .filter_map(|symbol| {
            symbol
                .id
                .as_ref()
                .map(|id| crate::types::record_key_to_string(&id.key))
        })
        .collect();

    let relations = if symbol_ids.is_empty() {
        Vec::new()
    } else {
        state
            .storage
            .get_code_subgraph(&symbol_ids, None)
            .await
            .map(|(_, relations)| relations)
            .unwrap_or_default()
    };

    let inputs = collect_project_projection_inputs(
        status,
        total_files,
        indexed_files,
        total_chunks,
        total_symbols,
        crate::types::ProjectProjectionRequest {
            relation_scope: params.relation_scope.unwrap_or_else(|| "all".to_string()),
            sort_mode: params.sort_mode.unwrap_or_else(|| "canonical".to_string()),
        },
        symbols,
        relations,
    );
    let shaped = shape_project_projection_graph(inputs);
    let projection = assemble_project_projection(shaped);
    let locator = format!(
        "projection:{}:{}:{}:{}",
        params.project_id,
        projection.request.relation_scope,
        projection.request.sort_mode,
        projection.contract.projection.generation
    );

    state
        .projection_registry
        .write()
        .await
        .insert(locator.clone(), projection.clone());

    let locator_record = projection_locator_record(
        locator,
        projection.project_id.clone(),
        projection.contract.projection.generation,
        projection.request.clone(),
        ProjectionLocatorLookup {
            state: ProjectionLocatorLookupState::Created,
            found: true,
            reason_code: None,
            message: Some(
                "Locator created for same-process ephemeral readback only; clients must not persist it or treat it as generation-stable."
                    .to_string(),
            ),
        },
    );

    Ok(success_json(json!({
        "project_id": params.project_id,
        "locator": locator_record,
        "projection": projection,
    })))
}

pub async fn get_project_projection_by_locator(
    state: &Arc<AppState>,
    params: GetProjectionByLocatorParams,
) -> anyhow::Result<CallToolResult> {
    let registry = state.projection_registry.read().await;
    let projection = match registry.get(&params.locator) {
        Some(projection) => projection.clone(),
        None => {
            let locator_record = projection_locator_record(
                params.locator.clone(),
                "unknown".to_string(),
                0,
                crate::types::ProjectProjectionRequest {
                    relation_scope: "unknown".to_string(),
                    sort_mode: "unknown".to_string(),
                },
                ProjectionLocatorLookup {
                    state: ProjectionLocatorLookupState::Missing,
                    found: false,
                    reason_code: Some(ContractReasonCode::InvalidLocator),
                    message: Some(
                        "Projection locator was not found in this process. Ephemeral locators are same-process only, non-persistable, and not generation-stable."
                            .to_string(),
                    ),
                },
            );

            return Ok(success_json(json!({
                "error": format!(
                    "Projection locator not found in this process: {}",
                    params.locator
                ),
                "reason_code": ContractReasonCode::InvalidLocator,
                "locator": locator_record,
            })));
        }
    };

    let locator_record = projection_locator_record(
        params.locator,
        projection.project_id.clone(),
        projection.contract.projection.generation,
        projection.request.clone(),
        ProjectionLocatorLookup {
            state: ProjectionLocatorLookupState::Resolved,
            found: true,
            reason_code: None,
            message: Some(
                "Locator resolved from the same-process ephemeral projection registry for the captured semantic generation."
                    .to_string(),
            ),
        },
    );

    Ok(success_json(json!({
        "locator": locator_record,
        "projection": projection,
    })))
}

/// Returns indexing degradation info for any active project, or None if all complete.
/// Used by recall_code, search_symbols, symbol_graph to inform users about degraded features.
pub async fn get_degradation_info(state: &Arc<AppState>) -> Option<serde_json::Value> {
    let project_ids = state.storage.list_projects().await.ok()?;

    for project_id in &project_ids {
        let status = match state.storage.get_index_status(project_id).await {
            Ok(Some(s)) => s,
            _ => continue,
        };

        if status.status == crate::types::IndexState::Completed
            || status.status == crate::types::IndexState::Failed
        {
            continue;
        }

        let total_chunks = state.storage.count_chunks(project_id, None).await.unwrap_or(0);
        let embedded_chunks = state
            .storage
            .count_embedded_chunks(project_id, None)
            .await
            .unwrap_or(0);
        let total_symbols = state.storage.count_symbols(project_id, None).await.unwrap_or(0);
        let embedded_symbols = state
            .storage
            .count_embedded_symbols(project_id, None)
            .await
            .unwrap_or(0);

        let chunk_pct = if total_chunks > 0 {
            (embedded_chunks as f64 / total_chunks as f64) * 100.0
        } else {
            0.0
        };
        let overall_progress = if (total_chunks + total_symbols) > 0 {
            ((embedded_chunks + embedded_symbols) as f64 / (total_chunks + total_symbols) as f64)
                * 100.0
        } else {
            0.0
        };

        return Some(json!({
            "status": status.status.to_string(),
            "lifecycle": lifecycle_json(&status),
            "contract": phase1_contract_json(
                Some(project_id),
                Some(&lifecycle_json(&status)),
                Some(&status),
            ),
            "summary": summary_index_status_response(
                status.total_files,
                status.indexed_files,
                total_chunks,
                total_symbols,
                overall_progress as f32,
                true,
                Some("Indexing in progress. Semantic (vector) search unavailable until complete.".to_string()),
            ),
            "progress": format!("{}/{} chunks ({:.1}%), {}/{} symbols",
                embedded_chunks, total_chunks, chunk_pct, embedded_symbols, total_symbols),
            "degraded": ["vector_search"],
            "available": ["bm25_search", "symbol_search", "ppr_graph"],
            "message": "Indexing in progress. Semantic (vector) search unavailable until complete."
        }));
    }

    None
}

pub async fn cancel_index(
    state: &Arc<AppState>,
    project_id: String,
    job_id: String,
) -> anyhow::Result<CallToolResult> {
    let job = match state.storage.get_index_job(&project_id, &job_id).await? {
        Some(j) => j,
        None => {
            return Ok(success_json(json!({
                "state": "not_found",
                "job_id": job_id,
                "project_id": project_id,
                "reason_code": "job_not_found",
            })));
        }
    };

    if matches!(
        job.state,
        IndexJobState::Completed | IndexJobState::Cancelled | IndexJobState::Abandoned | IndexJobState::Failed
    ) {
        return Ok(success_json(json!({
            "state": job.state,
            "job_id": job_id,
            "project_id": project_id,
            "reason_code": "job_already_terminal",
        })));
    }

    let mut updated = job;
    updated.state = IndexJobState::CancelRequested;
    updated.reason_code = Some(IndexJobReasonCode::CancellationRequested);
    updated.updated_at = crate::types::Datetime::default();
    state.storage.create_or_update_index_job(&updated).await?;

    Ok(success_json(json!({
        "state": "cancel_requested",
        "job_id": job_id,
        "project_id": project_id,
        "reason_code": "cancellation_requested",
    })))
}

pub async fn cleanup_abandoned_index_jobs(
    state: &Arc<AppState>,
    project_id: String,
) -> anyhow::Result<CallToolResult> {
    let jobs = state
        .storage
        .list_index_jobs_for_project(&project_id)
        .await?;

    let cleanable: Vec<_> = jobs
        .into_iter()
        .filter(|j| {
            matches!(
                j.state,
                IndexJobState::Abandoned | IndexJobState::Failed | IndexJobState::Cancelled
            )
        })
        .collect();

    let mut cleaned_up_count = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for job in &cleanable {
        if let Err(e) = state
            .storage
            .delete_project_generation(&project_id, job.target_generation)
            .await
        {
            errors.push(format!(
                "delete_generation {} for job {}: {}",
                job.target_generation, job.job_id, e
            ));
        }
        match state
            .storage
            .delete_index_job(&project_id, &job.job_id)
            .await
        {
            Ok(()) => cleaned_up_count += 1,
            Err(e) => errors.push(format!("delete_job {}: {}", job.job_id, e)),
        }
    }

    Ok(success_json(json!({
        "cleaned_up_count": cleaned_up_count,
        "project_id": project_id,
        "reason_code": "cleanup_requested",
        "errors": errors,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;
    use crate::types::{CodeIntelligenceDiagnostic, IndexState, IndexStatus};

    fn call_result_json(result: &CallToolResult) -> serde_json::Value {
        let value = serde_json::to_value(result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    fn assert_stale_lost_one_shot_contract(json: &serde_json::Value) {
        assert_eq!(json["status"], "failed");
        assert_eq!(json["retryable"], true);
        assert_eq!(
            json["reason_code"],
            "lost_one_shot_indexing_task_after_restart"
        );
        assert_eq!(json["background_task"]["state"], "unknown_after_restart");
        assert_eq!(json["background_task"]["phase"], "before_file_enumeration");
        assert_eq!(
            json["background_task"]["operation_id"],
            serde_json::Value::Null
        );
        assert_eq!(json["background_task"]["runner"], "local_tokio_task");
        assert_eq!(json["index_job"]["job_id"], serde_json::Value::Null);
        assert_eq!(json["index_job"]["state"], "failed");
        assert_eq!(json["index_job"]["can_resume"], false);
        assert_eq!(
            json["index_job"]["reason_code"],
            "lost_one_shot_indexing_task_after_restart"
        );
        assert_eq!(json["index_job"]["requires_force"], true);
        assert_eq!(json["index_job"]["requires_confirmation"], true);
        assert_eq!(json["index_job"]["restart_fallback"]["force"], true);
        assert_eq!(
            json["index_job"]["restart_fallback"]["confirm_failed_restart"],
            true
        );
        assert_eq!(json["files"]["total"], 0);
        assert_eq!(json["chunks"]["total"], 0);
        assert_eq!(json["overall_progress"]["is_complete"], false);
        assert_eq!(json["summary"]["partial"]["is_partial"], true);
        assert_eq!(json["summary"]["partial"]["reason_code"], "degraded");
        assert_eq!(
            json["summary"]["partial"]["reason"],
            "lost_one_shot_indexing_task_after_restart"
        );

        let guidance = serde_json::to_string(&json["recovery"]).unwrap();
        assert!(guidance.contains("force=true"));
        assert!(guidance.contains("confirm_failed_restart=true"));
        assert_eq!(json["recovery"]["example"]["arguments"]["force"], true);
        assert_eq!(
            json["recovery"]["example"]["arguments"]["confirm_failed_restart"],
            true
        );
    }

    async fn persist_lost_one_shot_indexing_status(ctx: &TestContext, project_id: &str) {
        let mut stale = IndexStatus::new(project_id.to_string());
        stale.status = IndexState::Indexing;
        stale.total_files = 0;
        stale.indexed_files = 0;
        stale.total_chunks = 0;
        stale.total_symbols = 0;
        stale.refresh_lifecycle_states();
        ctx.state.storage.update_index_status(stale).await.unwrap();

        assert!(ctx.state.progress.get(project_id).await.is_none());
        assert!(!ctx
            .state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned")
            .contains(project_id));
    }

    fn assert_no_lost_one_shot_reason(json: &serde_json::Value) {
        assert_ne!(
            json.get("reason_code").and_then(|value| value.as_str()),
            Some("lost_one_shot_indexing_task_after_restart")
        );
        assert_ne!(
            json["summary"]["partial"]["reason"].as_str(),
            Some("lost_one_shot_indexing_task_after_restart")
        );
    }

    async fn persist_active_indexing_status(ctx: &TestContext, project_id: &str) {
        let mut active = IndexStatus::new(project_id.to_string());
        active.status = IndexState::Indexing;
        active.total_files = 0;
        active.indexed_files = 0;
        active.total_chunks = 0;
        active.total_symbols = 0;
        active.refresh_lifecycle_states();
        ctx.state.storage.update_index_status(active).await.unwrap();

        let monitor = ctx.state.progress.get_or_create(project_id).await;
        set_monitor_optional_string(
            &monitor.operation_id,
            Some(format!("idx-{project_id}-active-test")),
        );
        set_monitor_string(&monitor.task_state, "running");
        monitor
            .total_files
            .store(0, std::sync::atomic::Ordering::Relaxed);
        monitor
            .indexed_files
            .store(0, std::sync::atomic::Ordering::Relaxed);

        ctx.state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned")
            .insert(project_id.to_string());
    }

    async fn persist_running_durable_job(ctx: &TestContext, project_id: &str) -> IndexJobRecord {
        let job = durable_index_job_record(project_id, "/tmp/durable-workspace", 3);
        ctx.state
            .storage
            .create_or_update_index_job(&job)
            .await
            .unwrap();
        job
    }

    #[tokio::test]
    async fn project_status_includes_durable_job_metadata() {
        let ctx = TestContext::new().await;
        let project_id = "durable-status-project";
        let mut status = IndexStatus::new(project_id.to_string());
        status.status = IndexState::Indexing;
        status.structural_generation = 3;
        status.refresh_lifecycle_states();
        ctx.state.storage.update_index_status(status).await.unwrap();
        let job = persist_running_durable_job(&ctx, project_id).await;

        let result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();

        let json = call_result_json(&result);
        assert_eq!(json["index_job"]["job_id"], job.job_id);
        assert_eq!(json["index_job"]["state"], "running");
        assert_eq!(json["index_job"]["can_resume"], false);
        assert_eq!(json["index_job"]["reason_code"], serde_json::Value::Null);
        assert_eq!(json["index_job"]["requires_force"], false);
        assert_eq!(json["index_job"]["requires_confirmation"], false);
        assert_eq!(json["index_job"]["restart_fallback"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn status_reports_lost_one_shot_with_restart_fallback() {
        let ctx = TestContext::new().await;
        persist_lost_one_shot_indexing_status(&ctx, "lost-one-shot-job-project").await;

        let result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: "lost-one-shot-job-project".to_string(),
            },
        )
        .await
        .unwrap();

        let json = call_result_json(&result);
        assert_eq!(
            json["index_job"]["reason_code"],
            "lost_one_shot_indexing_task_after_restart"
        );
        assert_eq!(json["index_job"]["can_resume"], false);
        assert_eq!(json["index_job"]["requires_force"], true);
        assert_eq!(json["index_job"]["requires_confirmation"], true);
        assert_eq!(json["index_job"]["restart_fallback"]["force"], true);
        assert_eq!(
            json["index_job"]["restart_fallback"]["confirm_failed_restart"],
            true
        );
    }

    #[tokio::test]
    async fn index_status_distinguishes_job_id_from_operation_id() {
        let ctx = TestContext::new().await;
        let project_id = "job-vs-operation-project";
        persist_active_indexing_status(&ctx, project_id).await;
        let job = persist_running_durable_job(&ctx, project_id).await;

        let result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();

        let json = call_result_json(&result);
        assert_eq!(json["index_job"]["job_id"], job.job_id);
        assert_eq!(json["background_task"]["operation_id"], format!("idx-{project_id}-active-test"));
        assert_eq!(json["index_job"]["operation_id"], serde_json::Value::Null);
        assert_ne!(json["index_job"]["job_id"], json["background_task"]["operation_id"]);
        assert!(json["index_job"]["identity_semantics"]["job_id"]
            .as_str()
            .unwrap()
            .contains("durable"));
        assert!(json["index_job"]["identity_semantics"]["operation_id"]
            .as_str()
            .unwrap()
            .contains("same-process"));
    }

    #[tokio::test]
    async fn root_diagnostic_reports_disabled_when_no_roots_are_available() {
        let temp = tempfile::tempdir().unwrap();
        let missing_fallback = temp.path().join("missing-fallback-root");

        let diagnostic = root_diagnostic(None, &missing_fallback);

        assert_eq!(
            diagnostic.status,
            crate::types::CodeIntelligenceDiagnosticCode::Disabled
        );
        assert_eq!(
            diagnostic.reason_code,
            crate::types::CodeIntelligenceDiagnosticCode::Disabled
        );
        assert!(diagnostic
            .message
            .contains("Code intelligence startup root is unavailable"));
    }

    #[tokio::test]
    async fn root_diagnostic_reports_missing_root_for_missing_configured_path() {
        let temp = tempfile::tempdir().unwrap();
        let missing_configured = temp.path().join("missing-configured-root");

        let diagnostic = root_diagnostic(Some(&missing_configured), temp.path());

        assert_eq!(
            diagnostic.status,
            crate::types::CodeIntelligenceDiagnosticCode::MissingRoot
        );
        assert_eq!(
            diagnostic.reason_code,
            crate::types::CodeIntelligenceDiagnosticCode::MissingRoot
        );
        assert!(diagnostic
            .message
            .contains("Configured project root is missing"));
    }

    #[test]
    fn code_intelligence_root_diagnostic_serializes_selected_when_root_exists() {
        let diagnostic =
            CodeIntelligenceDiagnostic::selected("Configured project root is available: /project");

        let json = diagnostic.as_json();
        assert_eq!(json["status"], "selected");
        assert_eq!(json["reason_code"], "selected");
        assert_eq!(
            json["message"],
            "Configured project root is available: /project"
        );
    }

    #[tokio::test]
    async fn get_project_stats_keeps_legacy_fields_and_adds_code_intelligence_status() {
        let ctx = TestContext::new().await;

        let mut status = IndexStatus::new("compat-status-project".to_string());
        status.status = IndexState::Completed;
        status.mark_structural_generation_advanced();
        status.mark_semantic_generation_caught_up();
        ctx.state.storage.update_index_status(status).await.unwrap();

        let result = get_project_stats(
            &ctx.state,
            GetProjectStatsParams {
                project_id: "compat-status-project".to_string(),
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["status"], "completed");
        assert!(json["files"].is_object());
        assert!(json["chunks"].is_object());
        assert!(json["symbols"].is_object());
        assert!(json["summary"].is_object());
        assert!(json["contract"].is_object());
        assert_eq!(json["code_intelligence"]["status"], "ready");
        assert_eq!(json["code_intelligence"]["reason_code"], "ready");
        assert!(json["code_intelligence"]["message"].is_string());
    }

    #[tokio::test]
    async fn index_project_does_not_auto_restart_failed_index_without_force() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("failed-project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let mut failed = IndexStatus::new("failed-project".to_string());
        failed.status = IndexState::Failed;
        failed.total_files = 3798;
        failed.indexed_files = 3710;
        failed.total_chunks = 12000;
        failed.error_message = Some("Indexing stalled at 3710/3798 files for >1800s".to_string());
        ctx.state.storage.update_index_status(failed).await.unwrap();

        let result = index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_dir.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
            include_patterns: None,
            exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["status"], "failed");
        assert_eq!(json["state"], "blocked");
        assert_eq!(json["can_retry"], true);
        assert_eq!(json["requires_force"], true);
        assert_eq!(json["requires_confirmation"], true);
        assert_eq!(json["lifecycle"]["project_id"], "failed-project");
        assert_eq!(json["lifecycle"]["state"], "registered");
        assert_eq!(json["lifecycle"]["diagnostic"]["status"], "selected");
        assert_eq!(
            json["recommended_action"],
            "retry_with_force_and_confirmation"
        );
        assert_eq!(json["indexed_files"], 3710);
        assert!(json["message"]
            .as_str()
            .unwrap()
            .contains("confirm_failed_restart=true"));
        assert_eq!(
            json["recovery"]["reason"],
            "failed_index_restart_requires_explicit_confirmation"
        );
        assert_eq!(json["recovery"]["example"]["tool"], "index_project");
        assert_eq!(json["recovery"]["example"]["arguments"]["force"], true);
        assert_eq!(
            json["recovery"]["example"]["arguments"]["confirm_failed_restart"],
            true
        );

        let stored = ctx
            .state
            .storage
            .get_index_status("failed-project")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, IndexState::Failed);
        assert_eq!(ctx.state.project_registry.len().await, 1);
        let lifecycle = ctx
            .state
            .project_registry
            .status("failed-project")
            .await
            .unwrap();
        assert_eq!(lifecycle.root_path, project_dir.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn lost_one_shot_retry_requires_failed_restart_confirmation() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("stale-indexing-project");
        std::fs::create_dir_all(&project_dir).unwrap();

        persist_lost_one_shot_indexing_status(&ctx, "stale-indexing-project").await;

        let before = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: "stale-indexing-project".to_string(),
            },
        )
        .await
        .unwrap();
        let before_value = serde_json::to_value(&before).unwrap();
        let before_text = before_value["content"][0]["text"].as_str().unwrap();
        let before_json: serde_json::Value = serde_json::from_str(before_text).unwrap();
        assert_stale_lost_one_shot_contract(&before_json);

        let result = index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_dir.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: Some(true),
                confirm_failed_restart: None,
            include_patterns: None,
            exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["project_id"], "stale-indexing-project");
        assert_eq!(json["status"], "failed");
        assert_eq!(json["state"], "blocked");
        assert_eq!(json["can_retry"], true);
        assert_eq!(json["requires_force"], true);
        assert_eq!(json["requires_confirmation"], true);
        assert_eq!(
            json["recommended_action"],
            "retry_with_force_and_confirmation"
        );
        assert_eq!(json["background_task"]["state"], "unknown_after_restart");
        assert_eq!(
            json["background_task"]["operation_id"],
            serde_json::Value::Null
        );
        assert_eq!(json["background_task"]["runner"], "local_tokio_task");
        assert!(json["message"]
            .as_str()
            .unwrap_or_default()
            .contains("confirm_failed_restart=true"));
        assert_eq!(json["lifecycle"]["project_id"], "stale-indexing-project");
        assert_eq!(json["lifecycle"]["state"], "registered");
        assert_eq!(ctx.state.project_registry.len().await, 1);

        let marked_active = ctx
            .state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned")
            .contains("stale-indexing-project");
        assert!(!marked_active);

        let stored = ctx
            .state
            .storage
            .get_index_status("stale-indexing-project")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, IndexState::Indexing);

        let after = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: "stale-indexing-project".to_string(),
            },
        )
        .await
        .unwrap();
        let after_value = serde_json::to_value(&after).unwrap();
        let after_text = after_value["content"][0]["text"].as_str().unwrap();
        let after_json: serde_json::Value = serde_json::from_str(after_text).unwrap();
        assert_stale_lost_one_shot_contract(&after_json);
    }

    #[tokio::test]
    async fn explicit_retry_after_lost_one_shot_starts_new_operation() {
        let ctx = TestContext::new().await;
        let project_dir = ctx
            ._temp_dir
            .path()
            .join("stale-indexing-project-confirmed");
        std::fs::create_dir_all(&project_dir).unwrap();

        persist_lost_one_shot_indexing_status(&ctx, "stale-indexing-project-confirmed").await;

        let result = index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_dir.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: Some(true),
                confirm_failed_restart: Some(true),
                include_patterns: None,
                exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["project_id"], "stale-indexing-project-confirmed");
        assert_eq!(json["status"], "indexing");
        assert_eq!(json["background_task"]["runner"], "local_tokio_task");
        assert_ne!(json["background_task"]["state"], "unknown_after_restart");
        assert!(matches!(
            json["background_task"]["state"].as_str(),
            Some("queued") | Some("running") | Some("indexing")
        ));
        assert!(json["background_task"]["operation_id"]
            .as_str()
            .map(|value| !value.is_empty())
            .unwrap_or(false));
        assert_no_lost_one_shot_reason(&json);

        let active = ctx
            .state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned")
            .contains("stale-indexing-project-confirmed");
        assert!(active);

        let status = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: "stale-indexing-project-confirmed".to_string(),
            },
        )
        .await
        .unwrap();
        let status_value = serde_json::to_value(&status).unwrap();
        let status_text = status_value["content"][0]["text"].as_str().unwrap();
        let status_json: serde_json::Value = serde_json::from_str(status_text).unwrap();
        assert_eq!(status_json["status"], "indexing");
        assert_eq!(status_json["background_task"]["runner"], "local_tokio_task");
        assert_ne!(
            status_json["background_task"]["state"],
            "unknown_after_restart"
        );
        assert!(status_json["background_task"]["operation_id"]
            .as_str()
            .map(|value| !value.is_empty())
            .unwrap_or(false));
        assert_no_lost_one_shot_reason(&status_json);
    }

    #[tokio::test]
    async fn index_project_registers_and_reuses_manual_lifecycle_for_completed_project() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("manual-register");
        std::fs::create_dir_all(&project_dir).unwrap();

        let mut completed = IndexStatus::new("manual-register".to_string());
        completed.status = IndexState::Completed;
        completed.total_files = 2;
        completed.indexed_files = 2;
        completed.total_chunks = 3;
        ctx.state
            .storage
            .update_index_status(completed)
            .await
            .unwrap();

        let first = index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_dir.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
            include_patterns: None,
            exclude_patterns: None,
            },
        )
        .await
        .unwrap();
        let first_value = serde_json::to_value(&first).unwrap();
        let first_text = first_value["content"][0]["text"].as_str().unwrap();
        let first_json: serde_json::Value = serde_json::from_str(first_text).unwrap();

        let alias_path = project_dir.join(".");
        let second = index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(alias_path.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
            include_patterns: None,
            exclude_patterns: None,
            },
        )
        .await
        .unwrap();
        let second_value = serde_json::to_value(&second).unwrap();
        let second_text = second_value["content"][0]["text"].as_str().unwrap();
        let second_json: serde_json::Value = serde_json::from_str(second_text).unwrap();

        assert_eq!(first_json["project_id"], "manual-register");
        assert_eq!(first_json["status"], "completed");
        assert_eq!(first_json["total_files"], 2);
        assert_eq!(first_json["indexed_files"], 2);
        assert_eq!(first_json["total_chunks"], 3);
        assert_eq!(first_json["lifecycle"]["project_id"], "manual-register");
        assert_eq!(first_json["lifecycle"]["state"], "registered");
        assert_eq!(first_json["lifecycle"]["diagnostic"]["status"], "selected");
        assert_eq!(
            first_json["lifecycle"]["root_path"],
            project_dir
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        );

        assert_eq!(second_json["project_id"], "manual-register");
        assert_eq!(second_json["status"], "completed");
        assert_eq!(
            second_json["lifecycle"]["root_path"],
            first_json["lifecycle"]["root_path"]
        );
        assert_eq!(ctx.state.project_registry.len().await, 1);

        let lifecycle = ctx
            .state
            .project_registry
            .status("manual-register")
            .await
            .unwrap();
        assert_eq!(lifecycle.root_path, project_dir.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn index_project_force_without_confirmation_still_blocks_failed_restart() {
        let ctx = TestContext::new().await;
        let project_dir = ctx._temp_dir.path().join("failed-project-force");
        std::fs::create_dir_all(&project_dir).unwrap();

        let mut failed = IndexStatus::new("failed-project-force".to_string());
        failed.status = IndexState::Failed;
        failed.error_message = Some("Indexing stalled previously".to_string());
        ctx.state.storage.update_index_status(failed).await.unwrap();

        let result = index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(project_dir.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: Some(true),
                confirm_failed_restart: None,
            include_patterns: None,
            exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["status"], "failed");
        assert_eq!(json["state"], "blocked");
        assert_eq!(json["requires_force"], true);
        assert_eq!(json["requires_confirmation"], true);
        assert_eq!(json["lifecycle"]["project_id"], "failed-project-force");
        assert_eq!(ctx.state.project_registry.len().await, 1);
    }

    #[tokio::test]
    async fn index_project_reports_path_not_allowed_reason_code_when_allowlist_rejects_path() {
        let temp = tempfile::tempdir().unwrap();
        let allowed_parent = temp.path().join("allowed-parent");
        std::fs::create_dir_all(&allowed_parent).unwrap();
        let ctx = TestContext::new_with_registry_policy(crate::codebase::ProjectRegistryPolicy {
            allowed_roots: Some(vec![allowed_parent.canonicalize().unwrap()]),
            max_projects: 5,
        })
        .await;
        let disallowed_project = tempfile::tempdir().unwrap();

        let result = index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(disallowed_project.path().to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
            include_patterns: None,
            exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        let error = json["error"].as_str().unwrap();
        assert!(error.contains("path_not_allowed"));
        assert_eq!(json["reason_code"], "path_not_allowed");
        assert_eq!(ctx.state.project_registry.len().await, 0);
    }

    #[tokio::test]
    async fn index_project_reports_max_project_limit_reason_code_when_registry_is_full() {
        let ctx = TestContext::new_with_registry_policy(crate::codebase::ProjectRegistryPolicy {
            allowed_roots: None,
            max_projects: 1,
        })
        .await;
        let first = ctx._temp_dir.path().join("first");
        let second = ctx._temp_dir.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();

        ctx.state
            .project_registry
            .ensure_project("first", &first)
            .await
            .unwrap();

        let result = index_project(
            &ctx.state,
            IndexProjectParams {
                path: Some(second.to_string_lossy().to_string()),
                project_id: None,
                resume: None,
                job_id: None,
                resume_token: None,
                allow_full_restart_fallback: None,
                force: None,
                confirm_failed_restart: None,
            include_patterns: None,
            exclude_patterns: None,
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        let error = json["error"].as_str().unwrap();
        assert!(error.contains("max_project_limit"));
        assert_eq!(json["reason_code"], "max_project_limit");
    }

    #[tokio::test]
    async fn get_index_status_exposes_lifecycle_contract() {
        let ctx = TestContext::new().await;

        let mut status = IndexStatus::new("lifecycle-project".to_string());
        status.status = IndexState::EmbeddingPending;
        status.mark_structural_generation_advanced();
        ctx.state.storage.update_index_status(status).await.unwrap();

        let result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: "lifecycle-project".to_string(),
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["lifecycle"]["structural"]["state"], "ready");
        assert_eq!(json["lifecycle"]["structural"]["is_ready"], true);
        assert_eq!(json["lifecycle"]["structural"]["generation"], 1);
        assert_eq!(json["lifecycle"]["semantic"]["state"], "pending");
        assert_eq!(json["lifecycle"]["semantic"]["is_ready"], false);
        assert_eq!(json["lifecycle"]["semantic"]["generation"], 0);
        assert_eq!(json["lifecycle"]["semantic"]["is_caught_up"], false);
        assert_eq!(json["lifecycle"]["projection"]["state"], "stale");
        assert_eq!(json["lifecycle"]["projection"]["is_current"], false);
        assert_eq!(json["contract"]["schema_version"], 1);
        assert_eq!(
            json["contract"]["compatibility"]["clients_must_ignore_unknown_fields"],
            true
        );
        assert_eq!(
            json["contract"]["generation_basis"]["structural_generation"],
            1
        );
        assert_eq!(
            json["contract"]["generation_basis"]["semantic_generation"],
            0
        );
        assert_eq!(json["contract"]["projection"]["state"], "stale");
        assert_eq!(
            json["contract"]["projection"]["basis"],
            "semantic_generation"
        );
        assert_eq!(json["contract"]["projection"]["generation"], 0);
        assert_eq!(
            json["contract"]["projection"]["materialization"]["strategy"],
            "not_materialized"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["refresh_basis"],
            "semantic_generation"
        );
        assert_eq!(json["contract"]["projection"]["materialization"]["persistence_semantics"], "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(
            json["contract"]["projection"]["materialization"]["shape_version_semantics"],
            "materialized_projection_payload_shape_version"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["addressability_semantics"],
            "no_stable_external_read_target_is_promised_until_materialization_strategy_changes"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_kind"],
            serde_json::Value::Null
        );
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_semantics"], "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_stability"],
            "not_stable_until_materialization_strategy_changes"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_scope"],
            "none_when_not_materialized"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_is_opaque"],
            true
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]
                ["locator_can_be_persisted_by_clients"],
            false
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_survives_generation_change"],
            false
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["current_generation"],
            0
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["is_addressable"],
            false
        );
        assert_eq!(json["summary"]["result_kind"], "status");
        assert_eq!(json["summary"]["counts"]["files"], 0);
        assert_eq!(json["summary"]["counts"]["indexed_files"], 0);
        assert_eq!(json["summary"]["partial"]["is_partial"], true);
    }

    #[tokio::test]
    async fn stale_indexing_status_marks_lost_one_shot_task_failed_and_retryable() {
        let ctx = TestContext::new().await;
        persist_lost_one_shot_indexing_status(&ctx, "stale-indexing-status-project").await;

        let result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: "stale-indexing-status-project".to_string(),
            },
        )
        .await
        .unwrap();

        let json = call_result_json(&result);
        assert_stale_lost_one_shot_contract(&json);
        assert_eq!(json["parsing"]["status"], "failed");
        assert_eq!(json["vector_embeddings"]["status"], "pending");
        assert_eq!(json["graph_embeddings"]["status"], "pending");
    }

    #[tokio::test]
    async fn stale_indexing_stats_marks_lost_one_shot_task_failed_and_retryable() {
        let ctx = TestContext::new().await;
        persist_lost_one_shot_indexing_status(&ctx, "stale-indexing-stats-project").await;

        let result = get_project_stats(
            &ctx.state,
            GetProjectStatsParams {
                project_id: "stale-indexing-stats-project".to_string(),
            },
        )
        .await
        .unwrap();

        let json = call_result_json(&result);
        assert_stale_lost_one_shot_contract(&json);
        assert_eq!(json["files"]["indexed"], 0);
        assert_eq!(json["chunks"]["embedded"], 0);
        assert_eq!(json["symbols"]["total"], 0);
        assert_eq!(json["overall_progress_percent"], "0.0");
    }

    #[tokio::test]
    async fn stale_indexing_false_positive_completed_empty_project_not_failed() {
        let ctx = TestContext::new().await;
        let project_id = "completed-empty-project";
        let mut completed = IndexStatus::new(project_id.to_string());
        completed.status = IndexState::Completed;
        completed.total_files = 0;
        completed.indexed_files = 0;
        completed.total_chunks = 0;
        completed.total_symbols = 0;
        completed.refresh_lifecycle_states();
        ctx.state
            .storage
            .update_index_status(completed)
            .await
            .unwrap();

        let status_result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();
        let status_json = call_result_json(&status_result);

        assert_eq!(status_json["status"], "completed");
        assert_eq!(status_json["code_intelligence"]["status"], "ready");
        assert_eq!(status_json["background_task"]["runner"], "none");
        assert_eq!(status_json["total_files"], 0);
        assert_eq!(status_json["summary"]["partial"]["is_partial"], false);
        assert_no_lost_one_shot_reason(&status_json);

        let stats_result = get_project_stats(
            &ctx.state,
            GetProjectStatsParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();
        let stats_json = call_result_json(&stats_result);

        assert_eq!(stats_json["status"], "completed");
        assert_eq!(stats_json["code_intelligence"]["status"], "ready");
        assert_eq!(stats_json["background_task"]["runner"], "none");
        assert_eq!(stats_json["files"]["total"], 0);
        assert_eq!(stats_json["chunks"]["total"], 0);
        assert_no_lost_one_shot_reason(&stats_json);
    }

    #[tokio::test]
    async fn active_indexing_status_not_marked_failed() {
        let ctx = TestContext::new().await;
        let project_id = "active-indexing-project";
        persist_active_indexing_status(&ctx, project_id).await;

        let result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();

        let json = call_result_json(&result);
        assert_eq!(json["status"], "indexing");
        assert_eq!(json["background_task"]["state"], "running");
        assert_eq!(
            json["background_task"]["operation_id"],
            format!("idx-{project_id}-active-test")
        );
        assert_eq!(json["background_task"]["runner"], "local_tokio_task");
        assert_eq!(json["summary"]["partial"]["is_partial"], true);
        assert_no_lost_one_shot_reason(&json);
        assert!(ctx
            .state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned")
            .contains(project_id));
    }

    #[tokio::test]
    async fn stale_indexing_status_idempotent() {
        let ctx = TestContext::new().await;
        let project_id = "stale-indexing-idempotent-project";
        persist_lost_one_shot_indexing_status(&ctx, project_id).await;

        let first_status = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();
        let first_status_json = call_result_json(&first_status);

        let first_stats = get_project_stats(
            &ctx.state,
            GetProjectStatsParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();
        let first_stats_json = call_result_json(&first_stats);

        let second_status = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();
        let second_status_json = call_result_json(&second_status);

        let second_stats = get_project_stats(
            &ctx.state,
            GetProjectStatsParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();
        let second_stats_json = call_result_json(&second_stats);

        assert_stale_lost_one_shot_contract(&first_status_json);
        assert_stale_lost_one_shot_contract(&first_stats_json);
        assert_eq!(first_status_json, second_status_json);
        assert_eq!(first_stats_json, second_stats_json);
        assert_eq!(
            second_status_json["background_task"]["operation_id"],
            serde_json::Value::Null
        );
        assert_eq!(
            second_stats_json["background_task"]["operation_id"],
            serde_json::Value::Null
        );
        assert!(ctx.state.progress.get(project_id).await.is_none());
        assert!(!ctx
            .state
            .indexing_projects
            .lock()
            .expect("indexing_projects mutex poisoned")
            .contains(project_id));
    }

    #[tokio::test]
    async fn get_project_stats_exposes_completed_lifecycle_contract() {
        let ctx = TestContext::new().await;

        let mut status = IndexStatus::new("completed-lifecycle-project".to_string());
        status.status = IndexState::Completed;
        status.mark_structural_generation_advanced();
        status.mark_semantic_generation_caught_up();
        ctx.state.storage.update_index_status(status).await.unwrap();

        let result = get_project_stats(
            &ctx.state,
            GetProjectStatsParams {
                project_id: "completed-lifecycle-project".to_string(),
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["lifecycle"]["structural"]["state"], "ready");
        assert_eq!(json["lifecycle"]["structural"]["generation"], 1);
        assert_eq!(json["lifecycle"]["semantic"]["state"], "ready");
        assert_eq!(json["lifecycle"]["semantic"]["generation"], 1);
        assert_eq!(json["lifecycle"]["semantic"]["is_caught_up"], true);
        assert_eq!(json["lifecycle"]["projection"]["state"], "stale");
        assert_eq!(json["contract"]["schema_version"], 1);
        assert_eq!(
            json["contract"]["identity"]["project_id"],
            "completed-lifecycle-project"
        );
        assert_eq!(json["contract"]["projection"]["state"], "stale");
        assert_eq!(json["contract"]["projection"]["generation"], 1);
        assert_eq!(
            json["contract"]["projection"]["materialization"]["strategy"],
            "not_materialized"
        );
        assert_eq!(json["contract"]["projection"]["materialization"]["persistence_semantics"], "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(
            json["contract"]["projection"]["materialization"]["shape_version_semantics"],
            "materialized_projection_payload_shape_version"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["addressability_semantics"],
            "no_stable_external_read_target_is_promised_until_materialization_strategy_changes"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_kind"],
            serde_json::Value::Null
        );
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_semantics"], "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_stability"],
            "not_stable_until_materialization_strategy_changes"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_scope"],
            "none_when_not_materialized"
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_is_opaque"],
            true
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]
                ["locator_can_be_persisted_by_clients"],
            false
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["locator_survives_generation_change"],
            false
        );
        assert_eq!(
            json["contract"]["projection"]["materialization"]["current_generation"],
            1
        );
        assert_eq!(json["summary"]["result_kind"], "status");
        assert_eq!(json["summary"]["counts"]["files"], 0);
        assert_eq!(json["summary"]["counts"]["indexed_files"], 0);
        assert_eq!(json["summary"]["partial"]["is_partial"], false);
    }

    #[tokio::test]
    async fn get_project_stats_reports_degraded_orphaned_rows() {
        let ctx = TestContext::new().await;

        let chunk = crate::types::CodeChunk {
            id: None,
            file_path: "src/orphaned.rs".to_string(),
            content: "fn orphaned() {}".to_string(),
            language: crate::types::Language::Rust,
            start_line: 1,
            end_line: 1,
            chunk_type: crate::types::ChunkType::Function,
            name: Some("orphaned".to_string()),
            context_path: None,
            embedding: None,
            content_hash: "hash-orphaned".to_string(),
            project_id: Some("orphaned-project".to_string()),
            generation: None,
            indexed_at: crate::types::Datetime::default(),
        };
        ctx.state.storage.create_code_chunk(chunk).await.unwrap();

        let result = get_project_stats(
            &ctx.state,
            GetProjectStatsParams {
                project_id: "orphaned-project".to_string(),
            },
        )
        .await
        .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        assert_eq!(json["project_id"], "orphaned-project");
        assert_eq!(json["status"], "failed");
        assert_eq!(json["diagnostics"]["status_metadata_missing"], true);
        assert_eq!(json["diagnostics"]["reason_code"], "degraded");
        assert_eq!(json["chunks"]["total"], 1);
        assert_eq!(json["symbols"]["total"], 0);
        assert!(json["diagnostics"]["message"]
            .as_str()
            .unwrap()
            .contains("Index status metadata is missing"));
    }

    #[tokio::test]
    async fn delete_project_removes_manifest_discovery_rows() {
        let ctx = TestContext::new().await;

        ctx.state
            .storage
            .upsert_manifest_entry("manifest-only-project", "src/stale.rs")
            .await
            .unwrap();

        let before_delete = ctx.state.storage.list_projects().await.unwrap();
        assert!(before_delete.contains(&"manifest-only-project".to_string()));

        let result = delete_project(
            &ctx.state,
            DeleteProjectParams {
                project_id: "manifest-only-project".to_string(),
            },
        )
        .await
        .unwrap();
        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(json["project_id"], "manifest-only-project");

        let after_delete = ctx.state.storage.list_projects().await.unwrap();
        assert!(!after_delete.contains(&"manifest-only-project".to_string()));

        let stats = get_project_stats(
            &ctx.state,
            GetProjectStatsParams {
                project_id: "manifest-only-project".to_string(),
            },
        )
        .await
        .unwrap();
        let value = serde_json::to_value(&stats).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(json["error"]
            .as_str()
            .unwrap()
            .contains("Project not found"));
    }

    #[tokio::test]
    async fn list_projects_exposes_lifecycle_contract() {
        let ctx = TestContext::new().await;

        let chunk = crate::types::CodeChunk {
            id: None,
            file_path: "src/lib.rs".to_string(),
            content: "fn demo() {}".to_string(),
            language: crate::types::Language::Rust,
            start_line: 1,
            end_line: 1,
            chunk_type: crate::types::ChunkType::Function,
            name: Some("demo".to_string()),
            context_path: None,
            embedding: None,
            content_hash: "hash-demo".to_string(),
            project_id: Some("list-lifecycle-project".to_string()),
            generation: None,
            indexed_at: crate::types::Datetime::default(),
        };
        ctx.state.storage.create_code_chunk(chunk).await.unwrap();

        let mut status = IndexStatus::new("list-lifecycle-project".to_string());
        status.status = IndexState::Completed;
        status.mark_structural_generation_advanced();
        status.mark_semantic_generation_caught_up();
        status.mark_projection_current();
        ctx.state.storage.update_index_status(status).await.unwrap();

        let result = list_projects(&ctx.state, ListProjectsParams { _placeholder: true })
            .await
            .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let project = json["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|project| project["id"] == "list-lifecycle-project")
            .cloned()
            .expect("project should be listed");

        assert_eq!(project["lifecycle"]["structural"]["state"], "ready");
        assert_eq!(project["lifecycle"]["structural"]["generation"], 1);
        assert_eq!(project["lifecycle"]["semantic"]["state"], "ready");
        assert_eq!(project["lifecycle"]["semantic"]["generation"], 1);
        assert_eq!(project["lifecycle"]["semantic"]["is_caught_up"], true);
        assert_eq!(project["lifecycle"]["projection"]["state"], "current");
        assert_eq!(project["lifecycle"]["projection"]["is_current"], true);
        assert_eq!(project["contract"]["schema_version"], 1);
        assert_eq!(
            project["contract"]["identity"]["project_id"],
            "list-lifecycle-project"
        );
        assert_eq!(project["contract"]["projection"]["state"], "current");
        assert_eq!(
            project["contract"]["projection"]["basis"],
            "semantic_generation"
        );
        assert_eq!(project["contract"]["projection"]["generation"], 1);
        assert_eq!(
            project["contract"]["projection"]["materialization"]["strategy"],
            "not_materialized"
        );
        assert_eq!(project["contract"]["projection"]["materialization"]["persistence_semantics"], "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(
            project["contract"]["projection"]["materialization"]["shape_version_semantics"],
            "materialized_projection_payload_shape_version"
        );
        assert_eq!(
            project["contract"]["projection"]["materialization"]["addressability_semantics"],
            "no_stable_external_read_target_is_promised_until_materialization_strategy_changes"
        );
        assert_eq!(
            project["contract"]["projection"]["materialization"]["locator_kind"],
            serde_json::Value::Null
        );
        assert_eq!(project["contract"]["projection"]["materialization"]["locator_semantics"], "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(
            project["contract"]["projection"]["materialization"]["locator_stability"],
            "not_stable_until_materialization_strategy_changes"
        );
        assert_eq!(
            project["contract"]["projection"]["materialization"]["locator_scope"],
            "none_when_not_materialized"
        );
        assert_eq!(
            project["contract"]["projection"]["materialization"]["locator_is_opaque"],
            true
        );
        assert_eq!(
            project["contract"]["projection"]["materialization"]
                ["locator_can_be_persisted_by_clients"],
            false
        );
        assert_eq!(
            project["contract"]["projection"]["materialization"]
                ["locator_survives_generation_change"],
            false
        );
        assert_eq!(
            project["contract"]["projection"]["materialization"]["current_generation"],
            1
        );
        assert_eq!(project["summary"]["result_kind"], "project");
        assert_eq!(project["summary"]["counts"]["results"], 1);
        assert_eq!(project["summary"]["counts"]["total"], 1);
        assert_eq!(project["summary"]["partial"]["is_partial"], false);
    }

    fn assert_list_projects_shape(project: &serde_json::Value) {
        for field in [
            "id",
            "project_id",
            "root_path",
            "status",
            "lifecycle",
            "contract",
            "summary",
            "chunks",
            "symbols",
            "embedded_chunks",
            "embedded_symbols",
            "diagnostics",
        ] {
            assert!(project.get(field).is_some(), "missing field {field}");
        }

        assert!(project["id"].is_string());
        assert!(project["project_id"].is_string());
        assert!(project["root_path"].is_string());
        assert!(project["status"].is_string());
        assert!(project["lifecycle"].is_object());
        assert!(project["contract"].is_object());
        assert!(project["summary"].is_object());
        assert!(project["diagnostics"].is_object());

        for field in ["chunks", "symbols", "embedded_chunks", "embedded_symbols"] {
            assert!(project[field].is_u64() || project[field].is_i64(), "{field} must be numeric");
        }
    }

    #[tokio::test]
    async fn list_projects_snapshot_includes_full_shape_and_zero_counts() {
        let ctx = TestContext::new().await;

        let data_project_id = "list-projects-data";
        let empty_project_id = "list-projects-empty";

        ctx.state
            .storage
            .create_code_chunk(crate::types::CodeChunk {
                id: None,
                file_path: "src/data.rs".to_string(),
                content: "fn data_project() {}".to_string(),
                language: crate::types::Language::Rust,
                start_line: 1,
                end_line: 1,
                chunk_type: crate::types::ChunkType::Function,
                name: Some("data_project".to_string()),
                context_path: None,
                embedding: None,
                content_hash: "hash-data-project".to_string(),
                project_id: Some(data_project_id.to_string()),
                generation: None,
                indexed_at: crate::types::Datetime::default(),
            })
            .await
            .unwrap();

        ctx.state
            .storage
            .create_code_symbol(crate::types::CodeSymbol::new(
                "data_project_symbol".to_string(),
                crate::types::SymbolType::Function,
                "src/data.rs".to_string(),
                1,
                3,
                data_project_id.to_string(),
            ))
            .await
            .unwrap();

        let mut data_status = IndexStatus::new(data_project_id.to_string());
        data_status.status = IndexState::Completed;
        data_status.root_path = Some("/workspace/list-projects-data".to_string());
        data_status.mark_structural_generation_advanced();
        data_status.mark_semantic_generation_caught_up();
        data_status.mark_projection_current();
        ctx.state.storage.update_index_status(data_status).await.unwrap();

        let mut empty_status = IndexStatus::new(empty_project_id.to_string());
        empty_status.root_path = Some("/workspace/list-projects-empty".to_string());
        ctx.state.storage.update_index_status(empty_status).await.unwrap();

        let result = list_projects(&ctx.state, ListProjectsParams { _placeholder: true })
            .await
            .unwrap();

        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        let projects = json["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 2);

        let data_project = projects
            .iter()
            .find(|project| project["project_id"] == data_project_id)
            .cloned()
            .expect("data project should be listed");
        let empty_project = projects
            .iter()
            .find(|project| project["project_id"] == empty_project_id)
            .cloned()
            .expect("empty project should be listed");

        assert_list_projects_shape(&data_project);
        assert_list_projects_shape(&empty_project);

        for field in ["chunks", "symbols", "embedded_chunks", "embedded_symbols"] {
            assert_eq!(
                empty_project[field].as_u64().or_else(|| empty_project[field].as_i64().map(|n| n as u64)),
                Some(0),
                "{field} must be 0 for empty project"
            );
        }
    }

    #[test]
    fn completed_status_preserves_projection_current_on_refresh() {
        let mut status = IndexStatus::new("projection-current".to_string());
        status.status = IndexState::Completed;
        status.mark_projection_current();

        status.refresh_lifecycle_states();

        assert_eq!(
            status.structural_state,
            crate::types::StructuralState::Ready
        );
        assert_eq!(status.semantic_state, crate::types::SemanticState::Ready);
        assert_eq!(
            status.projection_state,
            crate::types::ProjectionState::Current
        );
    }

    #[test]
    fn structural_generation_advances_before_semantic_completion() {
        let mut status = IndexStatus::new("generation-progress".to_string());

        status.mark_structural_generation_advanced();
        status.status = IndexState::EmbeddingPending;
        status.refresh_lifecycle_states();

        assert_eq!(status.structural_generation, 1);
        assert_eq!(status.semantic_generation, 0);
        assert_eq!(
            status.structural_state,
            crate::types::StructuralState::Ready
        );
        assert_eq!(status.semantic_state, crate::types::SemanticState::Pending);
        assert_eq!(
            status.projection_state,
            crate::types::ProjectionState::Stale
        );
    }

    #[test]
    fn semantic_generation_catches_up_on_completion() {
        let mut status = IndexStatus::new("generation-complete".to_string());

        status.mark_structural_generation_advanced();
        status.status = IndexState::EmbeddingPending;
        status.refresh_lifecycle_states();
        status.status = IndexState::Completed;
        status.mark_semantic_generation_caught_up();
        status.refresh_lifecycle_states();

        assert_eq!(status.structural_generation, 1);
        assert_eq!(status.semantic_generation, 1);
        assert_eq!(status.semantic_state, crate::types::SemanticState::Ready);
    }

    #[tokio::test]
    async fn project_info_capability_status_contract() {
        let ctx = TestContext::new().await;
        let project_id = "capability-status-project";

        ctx.state
            .storage
            .set_serving_generation(project_id, crate::types::code::CapabilityKind::Bm25, 5)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation(project_id, crate::types::code::CapabilityKind::Vector, 5)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_indexing_generation(project_id, Some(6))
            .await
            .unwrap();

        let mut status = IndexStatus::new(project_id.to_string());
        status.status = IndexState::Indexing;
        status.structural_generation = 5;
        status.refresh_lifecycle_states();
        ctx.state.storage.update_index_status(status).await.unwrap();

        ctx.state
            .indexing_projects
            .lock()
            .unwrap()
            .insert(project_id.to_string());

        let result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();

        let json = call_result_json(&result);
        assert_eq!(json["status"], "indexing");
        assert!(json["capabilities"].is_array(), "capabilities must be an array");
        assert!(json["serving"].is_object(), "serving must be an object");
        assert_eq!(json["indexing_generation"], 6);
        assert_eq!(json["serving"]["indexing"], 6);

        let caps = json["capabilities"].as_array().unwrap();
        let bm25_cap = caps.iter().find(|c| c["capability"] == "bm25").unwrap();
        assert_eq!(bm25_cap["freshness"], "stale");
        assert_eq!(bm25_cap["serving_generation"], 5);
        assert_eq!(bm25_cap["reason_code"], "indexing_in_progress");

        let project_info_cap = caps.iter().find(|c| c["capability"] == "project_info").unwrap();
        assert_eq!(project_info_cap["freshness"], "missing");
        assert_eq!(project_info_cap["reason_code"], "no_serving_generation");
    }

    #[tokio::test]
    async fn project_info_interrupted_generation_contract() {
        let ctx = TestContext::new().await;
        let project_id = "interrupted-gen-project";

        ctx.state
            .storage
            .set_serving_generation(project_id, crate::types::code::CapabilityKind::Bm25, 3)
            .await
            .unwrap();
        ctx.state
            .storage
            .set_serving_generation(project_id, crate::types::code::CapabilityKind::Vector, 3)
            .await
            .unwrap();

        let mut status = IndexStatus::new(project_id.to_string());
        status.status = IndexState::Failed;
        status.structural_generation = 3;
        status.refresh_lifecycle_states();
        ctx.state.storage.update_index_status(status).await.unwrap();

        let result = get_index_status(
            &ctx.state,
            GetIndexStatusParams {
                project_id: project_id.to_string(),
            },
        )
        .await
        .unwrap();

        let json = call_result_json(&result);
        assert_eq!(json["status"], "failed");
        assert_eq!(json["indexing_generation"], serde_json::Value::Null);
        assert_eq!(json["serving"]["bm25"], 3);
        assert_eq!(json["serving"]["vector"], 3);
        assert_eq!(json["serving"]["indexing"], serde_json::Value::Null);

        let caps = json["capabilities"].as_array().unwrap();
        let bm25_cap = caps.iter().find(|c| c["capability"] == "bm25").unwrap();
        assert_eq!(bm25_cap["freshness"], "fresh");
        assert_eq!(bm25_cap["serving_generation"], 3);
        assert_eq!(bm25_cap["reason_code"], serde_json::Value::Null);
    }
}
