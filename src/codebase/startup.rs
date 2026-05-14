use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::codebase::resolver::{
    resolve_startup_project_root, StartupProjectRootSource, StartupProjectRootStatus,
};
use crate::codebase::{resume_embeddings_for_project, CodebaseManager, IndexWorker};
use crate::config::AppState;
use crate::storage::StorageBackend;
use crate::types::{CodeIntelligenceDiagnostic, IndexJobReasonCode, IndexJobState, IndexState};

#[derive(Debug, Clone)]
pub enum CodeIntelligenceStartupStatus {
    Started {
        project_id: String,
        project_path: PathBuf,
        source: StartupProjectRootSource,
        diagnostic: CodeIntelligenceDiagnostic,
    },
    MissingRoot {
        configured_path: PathBuf,
        fallback_path: PathBuf,
        diagnostic: CodeIntelligenceDiagnostic,
    },
    Disabled {
        fallback_path: PathBuf,
        diagnostic: CodeIntelligenceDiagnostic,
    },
    StartupFailed {
        project_path: PathBuf,
        diagnostic: CodeIntelligenceDiagnostic,
        fatal: bool,
    },
}

#[derive(Debug, Clone)]
pub struct CodeIntelligenceStartupOutcome {
    pub status: CodeIntelligenceStartupStatus,
}

/// On server startup, scan all projects for jobs that were `Running` when the
/// server last shut down. Transition them to:
/// - `Resumable` if at least one completed checkpoint exists for the job's target generation
/// - `Failed` (with `reason_code=checkpoint_generation_missing`) if no checkpoints exist
///
/// This must be called once, early in startup, before any new indexing requests are accepted.
pub async fn perform_startup_job_recovery(state: &Arc<AppState>) {
    let project_ids = match state.storage.list_projects().await {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(error = %error, "startup job recovery: failed to list projects; skipping");
            return;
        }
    };

    let mut recovered = 0u32;
    let mut failed = 0u32;

    for project_id in &project_ids {
        let jobs = match state.storage.list_index_jobs_for_project(project_id).await {
            Ok(jobs) => jobs,
            Err(error) => {
                tracing::warn!(
                    project_id = %project_id,
                    error = %error,
                    "startup job recovery: failed to list jobs for project; skipping"
                );
                continue;
            }
        };

        for mut job in jobs {
            if job.state != IndexJobState::Running {
                continue;
            }

            let has_checkpoints = match state
                .storage
                .list_file_checkpoints_for_job(project_id, job.target_generation)
                .await
            {
                Ok(checkpoints) => checkpoints.iter().any(|c| c.completed),
                Err(error) => {
                    tracing::warn!(
                        project_id = %project_id,
                        job_id = %job.job_id,
                        error = %error,
                        "startup job recovery: failed to list checkpoints; treating as no checkpoints"
                    );
                    false
                }
            };

            if has_checkpoints {
                job.state = IndexJobState::Resumable;
                job.reason_code = Some(IndexJobReasonCode::ResumableInterruptedJob);
                tracing::info!(
                    project_id = %project_id,
                    job_id = %job.job_id,
                    target_generation = job.target_generation,
                    "startup job recovery: running job has checkpoints → marked resumable"
                );
                recovered += 1;
            } else {
                job.state = IndexJobState::Failed;
                job.reason_code = Some(IndexJobReasonCode::CheckpointGenerationMissing);
                tracing::info!(
                    project_id = %project_id,
                    job_id = %job.job_id,
                    target_generation = job.target_generation,
                    "startup job recovery: running job has no checkpoints → marked failed"
                );
                failed += 1;
            }

            job.updated_at = crate::types::Datetime::default();

            if let Err(error) = state.storage.create_or_update_index_job(&job).await {
                tracing::error!(
                    project_id = %project_id,
                    job_id = %job.job_id,
                    error = %error,
                    "startup job recovery: failed to persist recovered job state"
                );
            }
        }
    }

    if recovered + failed > 0 {
        tracing::info!(recovered, failed, "startup job recovery: completed");
    }
}

/// Re-enqueue incomplete code embeddings for every persisted project. This is
/// separate from the code intelligence lifecycle because HTTP/SSE deployments
/// can run without a startup `--project-path` while still serving previously
/// registered projects.
pub async fn resume_pending_embeddings_on_startup(state: &Arc<AppState>) {
    let project_ids = match state.storage.list_projects().await {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "startup embedding resume: failed to list projects; skipping"
            );
            return;
        }
    };

    let mut projects_resumed = 0u32;
    let mut items_enqueued = 0usize;

    for project_id in project_ids {
        let status = match state.storage.get_index_status(&project_id).await {
            Ok(Some(status)) => status,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    project_id = %project_id,
                    error = %error,
                    "startup embedding resume: failed to read project status; skipping"
                );
                continue;
            }
        };

        if status.status != IndexState::EmbeddingPending {
            continue;
        }

        match resume_embeddings_for_project(state, &project_id).await {
            Ok(count) if count > 0 => {
                projects_resumed += 1;
                items_enqueued += count;
                tracing::info!(
                    project_id = %project_id,
                    count,
                    "startup embedding resume: queued unembedded code items"
                );
            }
            Ok(_) => {
                tracing::debug!(
                    project_id = %project_id,
                    "startup embedding resume: no unembedded code items found"
                );
            }
            Err(error) => {
                tracing::error!(
                    project_id = %project_id,
                    error = %error,
                    "startup embedding resume: failed to queue unembedded code items"
                );
            }
        }
    }

    if projects_resumed > 0 {
        tracing::info!(
            projects_resumed,
            items_enqueued,
            "startup embedding resume: completed"
        );
    }
}

/// Clear stale indexing-generation markers left by older builds after a
/// project has already completed and semantic serving has caught up.
pub async fn clear_completed_indexing_generations_on_startup(state: &Arc<AppState>) {
    let project_ids = match state.storage.list_projects().await {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "startup indexing generation cleanup: failed to list projects; skipping"
            );
            return;
        }
    };

    let mut cleared = 0u32;
    for project_id in project_ids {
        let status = match state.storage.get_index_status(&project_id).await {
            Ok(Some(status)) => status,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    project_id = %project_id,
                    error = %error,
                    "startup indexing generation cleanup: failed to read project status; skipping"
                );
                continue;
            }
        };

        if status.status != IndexState::Completed
            || status.semantic_generation != status.structural_generation
        {
            continue;
        }

        let indexing_generation = match state.storage.get_indexing_generation(&project_id).await {
            Ok(generation) => generation,
            Err(error) => {
                tracing::warn!(
                    project_id = %project_id,
                    error = %error,
                    "startup indexing generation cleanup: failed to read indexing generation"
                );
                continue;
            }
        };

        if indexing_generation.is_none() {
            continue;
        }

        if let Err(error) = state
            .storage
            .set_indexing_generation(&project_id, None)
            .await
        {
            tracing::warn!(
                project_id = %project_id,
                error = %error,
                "startup indexing generation cleanup: failed to clear indexing generation"
            );
            continue;
        }

        cleared += 1;
        tracing::info!(
            project_id = %project_id,
            "startup indexing generation cleanup: cleared completed project marker"
        );
    }

    if cleared > 0 {
        tracing::info!(cleared, "startup indexing generation cleanup: completed");
    }
}

pub async fn start_code_intelligence_lifecycle(
    state: Arc<AppState>,
    configured_root: Option<&Path>,
    fallback_root: &Path,
) -> CodeIntelligenceStartupOutcome {
    let startup_root = resolve_startup_project_root(configured_root, fallback_root);

    match startup_root.status {
        StartupProjectRootStatus::Selected { path, source } => {
            let project_id = "project".to_string();

            if let Err(error) = state
                .project_registry
                .ensure_project(project_id.clone(), &path)
                .await
            {
                return CodeIntelligenceStartupOutcome {
                    status: CodeIntelligenceStartupStatus::StartupFailed {
                        project_path: path.clone(),
                        diagnostic: CodeIntelligenceDiagnostic::degraded(format!(
                            "Code intelligence startup failed: could not register project {} at {}: {error}",
                            project_id,
                            path.display()
                        )),
                        fatal: true,
                    },
                };
            }

            let (index_worker, index_tx) = IndexWorker::new(state.clone(), project_id.clone());
            state
                .index_pending
                .write()
                .await
                .insert(project_id.clone(), index_tx.pending_arc());
            index_worker.start(state.shutdown_rx());

            let manager = match CodebaseManager::new(state.clone(), path.clone(), index_tx.clone())
            {
                Ok(manager) => manager,
                Err(error) => {
                    return CodeIntelligenceStartupOutcome {
                        status: CodeIntelligenceStartupStatus::StartupFailed {
                            project_path: path.clone(),
                            diagnostic: CodeIntelligenceDiagnostic::degraded(format!(
                                "Code intelligence startup failed: could not create manager for {}: {error}",
                                path.display()
                            )),
                            fatal: true,
                        },
                    };
                }
            };

            let start_result = manager.start().await;
            let manager = Arc::new(manager);
            spawn_periodic_manifest_diff(
                state.shutdown_rx(),
                state.config.manifest_diff_interval_mins,
                project_id.clone(),
                manager,
            );

            if let Err(error) = start_result {
                return CodeIntelligenceStartupOutcome {
                    status: CodeIntelligenceStartupStatus::StartupFailed {
                        project_path: path.clone(),
                        diagnostic: CodeIntelligenceDiagnostic::degraded(format!(
                            "Code intelligence startup degraded for {}: manager start failed: {error}",
                            path.display()
                        )),
                        fatal: false,
                    },
                };
            }

            let selected_message = match source {
                StartupProjectRootSource::Configured => {
                    format!("Configured project root is available: {}", path.display())
                }
                StartupProjectRootSource::Fallback => {
                    format!(
                        "Compatibility project root is available: {}",
                        path.display()
                    )
                }
            };

            CodeIntelligenceStartupOutcome {
                status: CodeIntelligenceStartupStatus::Started {
                    project_id,
                    project_path: path,
                    source,
                    diagnostic: CodeIntelligenceDiagnostic::selected(selected_message),
                },
            }
        }
        StartupProjectRootStatus::MissingConfiguredRoot {
            configured_path,
            fallback_path,
            diagnostic,
        } => CodeIntelligenceStartupOutcome {
            status: CodeIntelligenceStartupStatus::MissingRoot {
                configured_path,
                fallback_path,
                diagnostic,
            },
        },
        StartupProjectRootStatus::Disabled {
            fallback_path,
            diagnostic,
        } => CodeIntelligenceStartupOutcome {
            status: CodeIntelligenceStartupStatus::Disabled {
                fallback_path,
                diagnostic,
            },
        },
    }
}

fn spawn_periodic_manifest_diff(
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    manifest_diff_interval_mins: u64,
    project_id: String,
    manager: Arc<CodebaseManager>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(
            manifest_diff_interval_mins.saturating_mul(60),
        ));
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    tracing::debug!(project_id = %project_id, "Periodic manifest diff starting");
                    if let Err(error) = manager.validate_index_full().await {
                        tracing::warn!(project_id = %project_id, error = %error, "Periodic manifest diff failed");
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::debug!(project_id = %project_id, "Manifest diff task stopping");
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::test_utils::TestContext;
    use crate::types::{ChunkType, CodeChunk, CodeSymbol, IndexStatus, Language, SymbolType};

    #[tokio::test]
    async fn startup_lifecycle_missing_configured_root_returns_missing_root_diagnostic() {
        let context = TestContext::new().await;
        let fallback_dir = tempfile::tempdir().unwrap();
        let missing_configured = fallback_dir.path().join("missing-configured");

        let outcome = start_code_intelligence_lifecycle(
            context.state.clone(),
            Some(missing_configured.as_path()),
            fallback_dir.path(),
        )
        .await;

        match outcome.status {
            CodeIntelligenceStartupStatus::MissingRoot {
                configured_path,
                fallback_path,
                diagnostic,
            } => {
                assert_eq!(configured_path, missing_configured);
                assert_eq!(fallback_path, fallback_dir.path());
                assert_eq!(
                    diagnostic.reason_code,
                    crate::types::CodeIntelligenceDiagnosticCode::MissingRoot
                );
            }
            other => panic!("expected missing-root outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn startup_lifecycle_configured_single_root_preserves_project_id_and_starts() {
        let context = TestContext::new().await;
        let configured_parent = tempfile::tempdir().unwrap();
        let configured_dir = configured_parent.path().join("my-app");
        std::fs::create_dir_all(&configured_dir).unwrap();
        let fallback_dir = tempfile::tempdir().unwrap();

        let outcome = start_code_intelligence_lifecycle(
            context.state.clone(),
            Some(configured_dir.as_path()),
            fallback_dir.path(),
        )
        .await;

        let project_id = match outcome.status {
            CodeIntelligenceStartupStatus::Started {
                source, project_id, ..
            } => {
                assert_eq!(source, StartupProjectRootSource::Configured);
                assert_eq!(project_id, "project");
                project_id
            }
            other => panic!("expected started outcome, got {other:?}"),
        };

        let pending = context.state.index_pending.read().await;
        assert!(pending.contains_key(&project_id));

        let lifecycle = context
            .state
            .project_registry
            .get(&project_id)
            .await
            .expect("startup should register selected project in registry");
        assert_eq!(lifecycle.project_id, project_id);
        assert_eq!(
            lifecycle.canonical_root,
            configured_dir.canonicalize().unwrap()
        );

        context.state.shutdown_tx.send(true).unwrap();
    }

    #[tokio::test]
    async fn startup_lifecycle_missing_configured_root_does_not_register_or_start_fallback_project()
    {
        let context = TestContext::new().await;
        let fallback_dir = tempfile::tempdir().unwrap();
        let fallback_project = fallback_dir.path().join("project");
        std::fs::create_dir_all(&fallback_project).unwrap();
        let missing_configured = fallback_dir.path().join("missing-configured");

        let outcome = start_code_intelligence_lifecycle(
            context.state.clone(),
            Some(missing_configured.as_path()),
            fallback_project.as_path(),
        )
        .await;

        match outcome.status {
            CodeIntelligenceStartupStatus::MissingRoot { diagnostic, .. } => {
                assert_eq!(
                    diagnostic.reason_code,
                    crate::types::CodeIntelligenceDiagnosticCode::MissingRoot
                );
            }
            other => panic!("expected missing-root outcome, got {other:?}"),
        }

        assert!(context.state.project_registry.is_empty().await);
        let pending = context.state.index_pending.read().await;
        assert!(pending.is_empty());
        assert!(!pending.contains_key("project"));
    }

    #[tokio::test]
    async fn startup_lifecycle_unconfigured_compatibility_project_root_uses_fallback_project_id() {
        let context = TestContext::new().await;
        let fallback_dir = tempfile::tempdir().unwrap();
        let compatibility_project = fallback_dir.path().join("project");
        std::fs::create_dir_all(&compatibility_project).unwrap();

        let outcome = start_code_intelligence_lifecycle(
            context.state.clone(),
            None,
            compatibility_project.as_path(),
        )
        .await;

        match outcome.status {
            CodeIntelligenceStartupStatus::Started {
                project_id,
                project_path,
                source,
                diagnostic,
            } => {
                assert_eq!(project_id, "project");
                assert_eq!(project_path, compatibility_project);
                assert_eq!(source, StartupProjectRootSource::Fallback);
                assert_eq!(
                    diagnostic.reason_code,
                    crate::types::CodeIntelligenceDiagnosticCode::Selected
                );
            }
            other => panic!("expected started outcome for fallback project root, got {other:?}"),
        }

        let pending = context.state.index_pending.read().await;
        assert!(pending.contains_key("project"));

        context.state.shutdown_tx.send(true).unwrap();
    }

    #[tokio::test]
    async fn startup_resume_embeddings_covers_registered_project_without_lifecycle_manager() {
        let context = TestContext::new().await;
        let project_id = "persisted-project-without-manager";

        let chunk = CodeChunk {
            id: None,
            file_path: "/workspace/persisted/src/lib.rs".to_string(),
            content: "pub fn persisted_embedding_resume_marker() { let value = 42; }".to_string(),
            language: Language::Rust,
            start_line: 1,
            end_line: 1,
            chunk_type: ChunkType::Function,
            name: Some("persisted_embedding_resume_marker".to_string()),
            context_path: None,
            embedding: None,
            content_hash: "persisted-embedding-resume-marker".to_string(),
            project_id: Some(project_id.to_string()),
            generation: Some(8),
            indexed_at: crate::types::Datetime::default(),
        };
        context
            .state
            .storage
            .create_code_chunks_batch(vec![chunk])
            .await
            .unwrap();

        let symbol = CodeSymbol::new(
            "PersistedEmbeddingResume".to_string(),
            SymbolType::Function,
            "/workspace/persisted/src/lib.rs".to_string(),
            1,
            1,
            project_id.to_string(),
        )
        .with_signature("fn PersistedEmbeddingResume()".to_string());
        context
            .state
            .storage
            .create_code_symbols_batch(vec![symbol])
            .await
            .unwrap();
        let stats = context.state.storage.get_all_project_stats().await.unwrap();
        assert_eq!(
            stats.get(project_id).map(|stats| stats.symbols),
            Some(1),
            "test setup should persist one symbol row for startup recovery"
        );
        let unembedded_symbols = context
            .state
            .storage
            .get_unembedded_symbols(project_id)
            .await
            .unwrap();
        assert_eq!(
            unembedded_symbols.len(),
            1,
            "test setup should persist one unembedded symbol for startup recovery"
        );

        let mut status = IndexStatus::new(project_id.to_string());
        status.status = IndexState::EmbeddingPending;
        status.structural_generation = 8;
        status.total_chunks = 1;
        status.total_symbols = 1;
        context
            .state
            .storage
            .update_index_status(status)
            .await
            .unwrap();

        resume_pending_embeddings_on_startup(&context.state).await;

        assert_eq!(
            context.state.embedding_queue.metrics().get_queue_depth(),
            2,
            "startup resume should enqueue chunk and symbol embeddings for persisted projects"
        );
    }

    #[tokio::test]
    async fn startup_clears_completed_project_indexing_generation_marker() {
        let context = TestContext::new().await;
        let project_id = "completed-project-with-stale-indexing-generation";

        let mut status = IndexStatus::new(project_id.to_string());
        status.status = IndexState::Completed;
        status.structural_generation = 8;
        status.semantic_generation = 8;
        context
            .state
            .storage
            .update_index_status(status)
            .await
            .unwrap();
        context
            .state
            .storage
            .set_indexing_generation(project_id, Some(7))
            .await
            .unwrap();

        clear_completed_indexing_generations_on_startup(&context.state).await;

        assert_eq!(
            context
                .state
                .storage
                .get_indexing_generation(project_id)
                .await
                .unwrap(),
            None,
            "completed projects should not keep a stale indexing generation marker"
        );
    }
}
