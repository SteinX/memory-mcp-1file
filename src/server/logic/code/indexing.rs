use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::server::params::{
    DeleteProjectParams, GetIndexStatusParams, GetProjectionByLocatorParams,
    GetProjectProjectionParams, GetProjectStatsParams, IndexProjectParams,
    ListProjectsParams,
};
use crate::storage::StorageBackend;
use crate::types::{
    ContractReasonCode, ExportIdentity, ProjectionLocatorLifecycle, ProjectionLocatorLookup,
    ProjectionLocatorLookupState, ProjectionLocatorRecord,
};

use super::super::{error_response, success_json};
use super::super::contracts::{
    assemble_project_projection, collect_project_projection_inputs, export_contract_meta,
    shape_project_projection_graph, summary_collection_response, summary_index_status_response,
    with_surface_guidance,
};

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
        &["status", "total_files", "indexed_files", "total_chunks", "total_symbols"],
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
        generation_binding: "locator is bound to the semantic generation captured at projection creation time"
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
    let path = std::path::Path::new(&params.path);

    if !path.exists() {
        return Ok(error_response(format!(
            "Path does not exist: {}",
            params.path
        )));
    }

    let project_id = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let force = params.force.unwrap_or(false);
    let confirm_failed_restart = params.confirm_failed_restart.unwrap_or(false);

    tracing::info!(
        path = %params.path,
        project_id = %project_id,
        force,
        confirm_failed_restart,
        "index_project request received"
    );

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
                        "message": "Project already indexed. File changes are tracked incrementally. Use force=true to re-index from scratch."
                    })));
                }
                tracing::info!(project_id = %project_id, "Force re-indexing project");
            }
            crate::types::IndexState::Failed => {
                if !force || !confirm_failed_restart {
                    return Ok(success_json(json!({
                        "project_id": project_id,
                        "status": status.status.to_string(),
                        "state": "blocked",
                        "can_retry": true,
                        "requires_force": true,
                        "requires_confirmation": true,
                        "recommended_action": "retry_with_force_and_confirmation",
                        "total_files": status.total_files,
                        "indexed_files": status.indexed_files,
                        "total_chunks": status.total_chunks,
                        "message": "Previous indexing failed. Refusing to restart full indexing unless force=true and confirm_failed_restart=true are both provided.",
                        "error_message": status.error_message,
                        "failed_files": status.failed_files,
                        "recovery": {
                            "reason": "failed_index_restart_requires_explicit_confirmation",
                            "next_step": "Only retry after confirming the previous failure cause is understood and resources are sufficient.",
                            "example": {
                                "tool": "index_project",
                                "arguments": {
                                    "path": params.path,
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
                // Will be caught by in-memory atomic lock below; DB state may lag
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
            "status": "indexing",
            "total_files": total_files,
            "indexed_files": indexed_files,
            "total_chunks": total_chunks,
            "message": "Indexing already in progress"
        })));
    }

    // Spawn indexing in background
    let state_clone = state.clone();
    let path_clone = params.path.clone();
    let project_id_for_cleanup = project_id.clone();

    tokio::spawn(async move {
        let path = std::path::Path::new(&path_clone);
        match crate::codebase::index_project(state_clone.clone(), path).await {
            Ok(status) => {
                tracing::info!(
                    project_id = %status.project_id,
                    files = status.indexed_files,
                    chunks = status.total_chunks,
                    "Indexing completed"
                );
            }
            Err(e) => {
                tracing::error!("Indexing failed: {}", e);
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
        "status": "indexing",
        "message": "Indexing started in background. Use get_index_status to check progress."
    })))
}

pub async fn get_index_status(
    state: &Arc<AppState>,
    params: GetIndexStatusParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.get_index_status(&params.project_id).await {
        Ok(Some(mut status)) => {
            status.refresh_lifecycle_states();
            let mut current_file: Option<String> = None;

            // Always try to fetch current_file from monitor if available, even if failed or stuck
            if let Some(monitor) = state.progress.get(&params.project_id).await {
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
                .count_manifest_entries(&params.project_id)
                .await
                .unwrap_or(0) as u32;

            // Sync queue status from shared AtomicUsize counter.
            let sync_queue_size = {
                let map = state.index_pending.read().await;
                map.get(&params.project_id)
                    .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
                    .unwrap_or(0)
            };
            let is_syncing = sync_queue_size > 0;

            let total_symbols = state
                .storage
                .count_symbols(&params.project_id)
                .await
                .unwrap_or(0);
            let total_chunks = state
                .storage
                .count_chunks(&params.project_id)
                .await
                .unwrap_or(0);
            let embedded_symbols = state
                .storage
                .count_embedded_symbols(&params.project_id)
                .await
                .unwrap_or(0);
            let embedded_chunks = state
                .storage
                .count_embedded_chunks(&params.project_id)
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

            let parsing_done = status.total_files > 0 && indexed_files >= status.total_files;
            let indexing_message = if status.status == crate::types::IndexState::Completed {
                None
            } else {
                Some("Indexing in progress. Status and counts may still change.".to_string())
            };

            Ok(success_json(json!({
                "project_id": status.project_id,
                "status": status.status.to_string(),
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
                    overall_progress,
                    status.status != crate::types::IndexState::Completed,
                    indexing_message,
                ),
                "is_syncing": is_syncing,
                "sync_queue_size": sync_queue_size,
                "total_files": status.total_files,
                "indexed_files": indexed_files,
                "started_at": status.started_at,
                "completed_at": status.completed_at,
                "lifecycle": lifecycle_json(&status),

                "parsing": {
                    "status": if indexed_files >= status.total_files { "completed" } else { "in_progress" },
                    "progress": format!("{}/{}", indexed_files, status.total_files),
                    "current_file": current_file
                },

                "vector_embeddings": {
                    "status": if status.status == crate::types::IndexState::Completed
                        || (parsing_done && embedded_chunks >= total_chunks && total_chunks > 0)
                        { "completed" } else { "in_progress" },
                    "total": total_chunks,
                    "completed": embedded_chunks,
                    "percent": format!("{:.1}", vector_progress)
                },

                "graph_embeddings": {
                    "status": if status.status == crate::types::IndexState::Completed
                        || (parsing_done && embedded_symbols >= total_symbols && total_symbols > 0)
                        { "completed" } else { "in_progress" },
                    "total": total_symbols,
                    "completed": embedded_symbols,
                    "percent": format!("{:.1}", graph_progress)
                },

                "overall_progress": {
                    "percent": format!("{:.1}", overall_progress),
                    "is_complete": status.status == crate::types::IndexState::Completed
                        || (parsing_done
                            && embedded_chunks >= total_chunks
                            && embedded_symbols >= total_symbols
                            && total_chunks > 0)
                },
                "error_message": status.error_message,
                "failed_files": status.failed_files
            })))
        }
        Ok(None) => Ok(error_response(format!(
            "Project not found: {}",
            params.project_id
        ))),
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn list_projects(
    state: &Arc<AppState>,
    _params: ListProjectsParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.list_projects().await {
        Ok(projects) => {
            let mut enriched = Vec::with_capacity(projects.len());

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
                let chunks = state.storage.count_chunks(project_id).await.unwrap_or(0);
                let symbols = state.storage.count_symbols(project_id).await.unwrap_or(0);
                let embedded_chunks = state
                    .storage
                    .count_embedded_chunks(project_id)
                    .await
                    .unwrap_or(0);
                let embedded_symbols = state
                    .storage
                    .count_embedded_symbols(project_id)
                    .await
                    .unwrap_or(0);

                let status_str = status
                    .as_ref()
                    .map(|s| s.status.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
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

                enriched.push(json!({
                    "id": project_id,
                    "status": status_str,
                    "lifecycle": lifecycle.clone(),
                    "contract": phase1_contract_json(Some(project_id), Some(&lifecycle), status.as_ref()),
                    "summary": summary_collection_response(
                        "project",
                        chunks as usize,
                        Some((chunks + symbols) as usize),
                        !status_is_complete,
                        if status_is_complete {
                            None
                        } else {
                            Some("Project indexing is still in progress.".to_string())
                        },
                    ),
                    "chunks": chunks,
                    "symbols": symbols,
                    "embedded_chunks": embedded_chunks,
                    "embedded_symbols": embedded_symbols
                }));
            }

            Ok(success_json(json!({
                "projects": enriched,
                "count": projects.len()
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn delete_project(
    state: &Arc<AppState>,
    params: DeleteProjectParams,
) -> anyhow::Result<CallToolResult> {
    let _ = state
        .storage
        .delete_project_symbols(&params.project_id)
        .await;

    let _ = state.storage.delete_index_status(&params.project_id).await;
    let _ = state.storage.delete_file_hashes(&params.project_id).await;

    // Remove from in-memory BM25 index
    state.code_search.remove_project(&params.project_id).await;

    match state
        .storage
        .delete_project_chunks(&params.project_id)
        .await
    {
        Ok(deleted) => Ok(success_json(json!({
            "deleted_chunks": deleted,
            "project_id": params.project_id
        }))),
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn get_project_stats(
    state: &Arc<AppState>,
    params: GetProjectStatsParams,
) -> anyhow::Result<CallToolResult> {
    let status = state.storage.get_index_status(&params.project_id).await?;

    if status.is_none() {
        return Ok(error_response(format!(
            "Project not found: {}",
            params.project_id
        )));
    }

    let status = status.unwrap();
    let mut status = status;
    status.refresh_lifecycle_states();

    let total_symbols = state
        .storage
        .count_symbols(&params.project_id)
        .await
        .unwrap_or(0);
    let total_chunks = state
        .storage
        .count_chunks(&params.project_id)
        .await
        .unwrap_or(0);
    let embedded_symbols = state
        .storage
        .count_embedded_symbols(&params.project_id)
        .await
        .unwrap_or(0);
    let embedded_chunks = state
        .storage
        .count_embedded_chunks(&params.project_id)
        .await
        .unwrap_or(0);

    // Use manifest entry count as the authoritative "indexed files" number.
    let indexed_files = state
        .storage
        .count_manifest_entries(&params.project_id)
        .await
        .unwrap_or(0) as u32;

    // Sync queue status from shared AtomicUsize counter.
    let sync_queue_size = {
        let map = state.index_pending.read().await;
        map.get(&params.project_id)
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
    let indexing_message = if status.status == crate::types::IndexState::Completed {
        None
    } else {
        Some("Indexing in progress. Project stats may still change.".to_string())
    };

    Ok(success_json(json!({
        "project_id": params.project_id,
        "status": status.status.to_string(),
        "contract": phase1_contract_json(
            Some(&params.project_id),
            Some(&lifecycle_json(&status)),
            Some(&status),
        ),
        "summary": summary_index_status_response(
            status.total_files,
            indexed_files,
            total_chunks,
            total_symbols,
            overall_progress,
            status.status != crate::types::IndexState::Completed,
            indexing_message,
        ),
        "is_syncing": is_syncing,
        "sync_queue_size": sync_queue_size,
        "lifecycle": lifecycle_json(&status),
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
        "started_at": status.started_at,
        "completed_at": status.completed_at,
        "failed_files": status.failed_files
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
        .count_chunks(&params.project_id)
        .await
        .unwrap_or(0);
    let total_symbols = state
        .storage
        .count_symbols(&params.project_id)
        .await
        .unwrap_or(0);
    let symbols = state
        .storage
        .get_project_symbols(&params.project_id)
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
            .get_code_subgraph(&symbol_ids)
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
            })))
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

        let total_chunks = state.storage.count_chunks(project_id).await.unwrap_or(0);
        let embedded_chunks = state
            .storage
            .count_embedded_chunks(project_id)
            .await
            .unwrap_or(0);
        let total_symbols = state.storage.count_symbols(project_id).await.unwrap_or(0);
        let embedded_symbols = state
            .storage
            .count_embedded_symbols(project_id)
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

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::test_utils::TestContext;
        use crate::types::{IndexState, IndexStatus};

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
                path: project_dir.to_string_lossy().to_string(),
                force: None,
                confirm_failed_restart: None,
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
                path: project_dir.to_string_lossy().to_string(),
                force: Some(true),
                confirm_failed_restart: None,
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
        assert_eq!(json["contract"]["compatibility"]["clients_must_ignore_unknown_fields"], true);
        assert_eq!(json["contract"]["generation_basis"]["structural_generation"], 1);
        assert_eq!(json["contract"]["generation_basis"]["semantic_generation"], 0);
        assert_eq!(json["contract"]["projection"]["state"], "stale");
        assert_eq!(json["contract"]["projection"]["basis"], "semantic_generation");
        assert_eq!(json["contract"]["projection"]["generation"], 0);
        assert_eq!(json["contract"]["projection"]["materialization"]["strategy"], "not_materialized");
        assert_eq!(json["contract"]["projection"]["materialization"]["refresh_basis"], "semantic_generation");
        assert_eq!(json["contract"]["projection"]["materialization"]["persistence_semantics"], "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(json["contract"]["projection"]["materialization"]["shape_version_semantics"], "materialized_projection_payload_shape_version");
        assert_eq!(json["contract"]["projection"]["materialization"]["addressability_semantics"], "no_stable_external_read_target_is_promised_until_materialization_strategy_changes");
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_kind"], serde_json::Value::Null);
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_semantics"], "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_stability"], "not_stable_until_materialization_strategy_changes");
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_scope"], "none_when_not_materialized");
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_is_opaque"], true);
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_can_be_persisted_by_clients"], false);
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_survives_generation_change"], false);
        assert_eq!(json["contract"]["projection"]["materialization"]["current_generation"], 0);
        assert_eq!(json["contract"]["projection"]["materialization"]["is_addressable"], false);
        assert_eq!(json["summary"]["result_kind"], "status");
        assert_eq!(json["summary"]["counts"]["files"], 0);
        assert_eq!(json["summary"]["counts"]["indexed_files"], 0);
        assert_eq!(json["summary"]["partial"]["is_partial"], true);
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
        assert_eq!(json["contract"]["identity"]["project_id"], "completed-lifecycle-project");
        assert_eq!(json["contract"]["projection"]["state"], "stale");
        assert_eq!(json["contract"]["projection"]["generation"], 1);
        assert_eq!(json["contract"]["projection"]["materialization"]["strategy"], "not_materialized");
        assert_eq!(json["contract"]["projection"]["materialization"]["persistence_semantics"], "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(json["contract"]["projection"]["materialization"]["shape_version_semantics"], "materialized_projection_payload_shape_version");
        assert_eq!(json["contract"]["projection"]["materialization"]["addressability_semantics"], "no_stable_external_read_target_is_promised_until_materialization_strategy_changes");
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_kind"], serde_json::Value::Null);
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_semantics"], "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_stability"], "not_stable_until_materialization_strategy_changes");
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_scope"], "none_when_not_materialized");
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_is_opaque"], true);
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_can_be_persisted_by_clients"], false);
        assert_eq!(json["contract"]["projection"]["materialization"]["locator_survives_generation_change"], false);
        assert_eq!(json["contract"]["projection"]["materialization"]["current_generation"], 1);
        assert_eq!(json["summary"]["result_kind"], "status");
        assert_eq!(json["summary"]["counts"]["files"], 0);
        assert_eq!(json["summary"]["counts"]["indexed_files"], 0);
        assert_eq!(json["summary"]["partial"]["is_partial"], false);
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
        assert_eq!(project["contract"]["identity"]["project_id"], "list-lifecycle-project");
        assert_eq!(project["contract"]["projection"]["state"], "current");
        assert_eq!(project["contract"]["projection"]["basis"], "semantic_generation");
        assert_eq!(project["contract"]["projection"]["generation"], 1);
        assert_eq!(project["contract"]["projection"]["materialization"]["strategy"], "not_materialized");
        assert_eq!(project["contract"]["projection"]["materialization"]["persistence_semantics"], "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(project["contract"]["projection"]["materialization"]["shape_version_semantics"], "materialized_projection_payload_shape_version");
        assert_eq!(project["contract"]["projection"]["materialization"]["addressability_semantics"], "no_stable_external_read_target_is_promised_until_materialization_strategy_changes");
        assert_eq!(project["contract"]["projection"]["materialization"]["locator_kind"], serde_json::Value::Null);
        assert_eq!(project["contract"]["projection"]["materialization"]["locator_semantics"], "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(project["contract"]["projection"]["materialization"]["locator_stability"], "not_stable_until_materialization_strategy_changes");
        assert_eq!(project["contract"]["projection"]["materialization"]["locator_scope"], "none_when_not_materialized");
        assert_eq!(project["contract"]["projection"]["materialization"]["locator_is_opaque"], true);
        assert_eq!(project["contract"]["projection"]["materialization"]["locator_can_be_persisted_by_clients"], false);
        assert_eq!(project["contract"]["projection"]["materialization"]["locator_survives_generation_change"], false);
        assert_eq!(project["contract"]["projection"]["materialization"]["current_generation"], 1);
        assert_eq!(project["summary"]["result_kind"], "project");
        assert_eq!(project["summary"]["counts"]["results"], 1);
        assert_eq!(project["summary"]["counts"]["total"], 1);
        assert_eq!(project["summary"]["partial"]["is_partial"], false);
    }

    #[test]
    fn completed_status_preserves_projection_current_on_refresh() {
        let mut status = IndexStatus::new("projection-current".to_string());
        status.status = IndexState::Completed;
        status.mark_projection_current();

        status.refresh_lifecycle_states();

        assert_eq!(status.structural_state, crate::types::StructuralState::Ready);
        assert_eq!(status.semantic_state, crate::types::SemanticState::Ready);
        assert_eq!(status.projection_state, crate::types::ProjectionState::Current);
    }

    #[test]
    fn structural_generation_advances_before_semantic_completion() {
        let mut status = IndexStatus::new("generation-progress".to_string());

        status.mark_structural_generation_advanced();
        status.status = IndexState::EmbeddingPending;
        status.refresh_lifecycle_states();

        assert_eq!(status.structural_generation, 1);
        assert_eq!(status.semantic_generation, 0);
        assert_eq!(status.structural_state, crate::types::StructuralState::Ready);
        assert_eq!(status.semantic_state, crate::types::SemanticState::Pending);
        assert_eq!(status.projection_state, crate::types::ProjectionState::Stale);
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
}
