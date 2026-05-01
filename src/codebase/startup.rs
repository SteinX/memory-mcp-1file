use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::codebase::resolver::{
    resolve_startup_project_root, StartupProjectRootSource, StartupProjectRootStatus,
};
use crate::codebase::{CodebaseManager, IndexWorker};
use crate::config::AppState;
use crate::types::CodeIntelligenceDiagnostic;

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

            let manager = match CodebaseManager::new(state.clone(), path.clone(), index_tx.clone()) {
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
                    format!("Compatibility project root is available: {}", path.display())
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
    use crate::test_utils::TestContext;

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
                source,
                project_id,
                ..
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
    async fn startup_lifecycle_missing_configured_root_does_not_register_or_start_fallback_project() {
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
                assert_eq!(diagnostic.reason_code, crate::types::CodeIntelligenceDiagnosticCode::Selected);
            }
            other => panic!("expected started outcome for fallback project root, got {other:?}"),
        }

        let pending = context.state.index_pending.read().await;
        assert!(pending.contains_key("project"));

        context.state.shutdown_tx.send(true).unwrap();
    }

}
