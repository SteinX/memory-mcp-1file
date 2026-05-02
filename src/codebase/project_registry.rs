//! In-memory project lifecycle registry contract.
//!
//! The registry is a Wave 1 design skeleton for later code-intelligence
//! lifecycle ownership. It deliberately does not start [`super::IndexWorker`],
//! [`super::CodebaseManager`], file watchers, manifest diffing, or embedding
//! jobs. Its current responsibility is to define the concurrency-safe API
//! contract that later tasks can wire into startup and `index_project`:
//!
//! - `ensure_project(project_id, root)` is idempotent for the same stable ID and
//!   canonical root.
//! - the same stable ID with a different canonical root is rejected explicitly
//!   and reports both paths.
//! - the lifecycle entry carries placeholders for pending jobs, manager/worker
//!   handles, lifecycle diagnostics, lifecycle options, and the last error
//!   without claiming runtime ownership yet.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::RwLock;

use crate::types::{CodeIntelligenceDiagnostic, ProjectIdError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectLifecycleState {
    Registered,
    Starting,
    Ready,
    Degraded,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct ProjectLifecycle {
    pub project_id: String,
    pub canonical_root: PathBuf,
    pub state: ProjectLifecycleState,
    pub diagnostic: CodeIntelligenceDiagnostic,
    pub pending_count: Arc<AtomicUsize>,
    pub manager_handle: Option<ProjectManagerHandle>,
    pub worker_handle: Option<ProjectWorkerHandle>,
    pub worker_sender: Option<ProjectWorkerSenderHandle>,
    pub last_error: Option<String>,
    pub options: ProjectLifecycleOptions,
}

impl ProjectLifecycle {
    pub fn pending_jobs(&self) -> usize {
        self.pending_count.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> ProjectLifecycleStatus {
        ProjectLifecycleStatus {
            project_id: self.project_id.clone(),
            root_path: self.canonical_root.clone(),
            state: self.state,
            diagnostic: self.diagnostic.clone(),
            pending_jobs: self.pending_jobs(),
            has_manager_handle: self.manager_handle.is_some(),
            has_worker_handle: self.worker_handle.is_some(),
            has_worker_sender: self.worker_sender.is_some(),
            last_error: self.last_error.clone(),
            options: self.options.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLifecycleOptions {
    pub registry_starts_workers: bool,
    pub registry_starts_watchers: bool,
}

impl Default for ProjectLifecycleOptions {
    fn default() -> Self {
        Self {
            registry_starts_workers: false,
            registry_starts_watchers: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLifecycleStatus {
    pub project_id: String,
    pub root_path: PathBuf,
    pub state: ProjectLifecycleState,
    pub diagnostic: CodeIntelligenceDiagnostic,
    pub pending_jobs: usize,
    pub has_manager_handle: bool,
    pub has_worker_handle: bool,
    pub has_worker_sender: bool,
    pub last_error: Option<String>,
    pub options: ProjectLifecycleOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectManagerHandle {
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkerHandle {
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkerSenderHandle {
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRootConflict {
    pub project_id: String,
    pub existing_root: PathBuf,
    pub requested_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRegistryPolicy {
    pub allowed_roots: Option<Vec<PathBuf>>,
    pub max_projects: usize,
}

impl Default for ProjectRegistryPolicy {
    fn default() -> Self {
        Self {
            allowed_roots: None,
            max_projects: 5,
        }
    }
}

#[derive(Debug, Error)]
pub enum ProjectRegistryError {
    #[error("project_id {project_id} already registered for {existing_root}; requested {requested_root}")]
    RootConflict {
        project_id: String,
        existing_root: String,
        requested_root: String,
    },

    #[error(transparent)]
    InvalidRoot(#[from] ProjectIdError),

    #[error(
        "path_not_allowed: project_id {project_id} root {requested_root} is outside configured allowed roots {allowed_roots:?}"
    )]
    PathNotAllowed {
        project_id: String,
        requested_root: String,
        allowed_roots: Vec<String>,
    },

    #[error(
        "max_project_limit: project_id {project_id} denied because registry already has {current_projects} projects (max {max_projects})"
    )]
    MaxProjectLimit {
        project_id: String,
        max_projects: usize,
        current_projects: usize,
    },
}

impl ProjectRegistryError {
    pub fn root_conflict(&self) -> Option<ProjectRootConflict> {
        match self {
            Self::RootConflict {
                project_id,
                existing_root,
                requested_root,
            } => Some(ProjectRootConflict {
                project_id: project_id.clone(),
                existing_root: PathBuf::from(existing_root),
                requested_root: PathBuf::from(requested_root),
            }),
            Self::InvalidRoot(_) => None,
            Self::PathNotAllowed { .. } | Self::MaxProjectLimit { .. } => None,
        }
    }

    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidRoot(_) => "invalid_root",
            Self::RootConflict { .. } => "root_conflict",
            Self::PathNotAllowed { .. } => "path_not_allowed",
            Self::MaxProjectLimit { .. } => "max_project_limit",
        }
    }
}

#[derive(Debug)]
pub struct ProjectRegistry {
    policy: ProjectRegistryPolicy,
    lifecycles: RwLock<HashMap<String, Arc<ProjectLifecycle>>>,
}

impl ProjectRegistry {
    pub fn new() -> Self {
        Self {
            policy: ProjectRegistryPolicy::default(),
            lifecycles: RwLock::new(HashMap::new()),
        }
    }

    pub fn with_policy(policy: ProjectRegistryPolicy) -> Self {
        Self {
            policy,
            lifecycles: RwLock::new(HashMap::new()),
        }
    }

    pub async fn ensure_project(
        &self,
        project_id: impl Into<String>,
        root: impl AsRef<Path>,
    ) -> Result<Arc<ProjectLifecycle>, ProjectRegistryError> {
        let project_id = project_id.into();
        let canonical_root = canonicalize_registry_root(root.as_ref())?;
        let mut lifecycles = self.lifecycles.write().await;

        if let Some(allowed_roots) = &self.policy.allowed_roots {
            let inside_allowlist = allowed_roots
                .iter()
                .any(|allowed_root| canonical_root.starts_with(allowed_root));
            if !inside_allowlist {
                return Err(ProjectRegistryError::PathNotAllowed {
                    project_id,
                    requested_root: canonical_root.to_string_lossy().into_owned(),
                    allowed_roots: allowed_roots
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                });
            }
        }

        if let Some(existing) = lifecycles.get(&project_id) {
            if existing.canonical_root == canonical_root {
                return Ok(existing.clone());
            }

            return Err(ProjectRegistryError::RootConflict {
                project_id,
                existing_root: existing.canonical_root.to_string_lossy().into_owned(),
                requested_root: canonical_root.to_string_lossy().into_owned(),
            });
        }

        let current_projects = lifecycles.len();
        if current_projects >= self.policy.max_projects {
            return Err(ProjectRegistryError::MaxProjectLimit {
                project_id,
                max_projects: self.policy.max_projects,
                current_projects,
            });
        }

        let lifecycle = Arc::new(ProjectLifecycle {
            project_id: project_id.clone(),
            canonical_root,
            state: ProjectLifecycleState::Registered,
            diagnostic: CodeIntelligenceDiagnostic::selected(
                "project registered; manager and worker not started by registry skeleton",
            ),
            pending_count: Arc::new(AtomicUsize::new(0)),
            manager_handle: None,
            worker_handle: None,
            worker_sender: None,
            last_error: None,
            options: ProjectLifecycleOptions::default(),
        });

        lifecycles.insert(project_id, lifecycle.clone());
        Ok(lifecycle)
    }

    pub async fn get(&self, project_id: &str) -> Option<Arc<ProjectLifecycle>> {
        self.lifecycles.read().await.get(project_id).cloned()
    }

    pub async fn status(&self, project_id: &str) -> Option<ProjectLifecycleStatus> {
        self.lifecycles
            .read()
            .await
            .get(project_id)
            .map(|lifecycle| lifecycle.status())
            .map(mark_status_degraded_if_root_missing)
    }

    pub async fn statuses(&self) -> Vec<ProjectLifecycleStatus> {
        let lifecycles = self.lifecycles.read().await;
        let mut statuses: Vec<_> = lifecycles
            .values()
            .map(|lifecycle| lifecycle.status())
            .map(mark_status_degraded_if_root_missing)
            .collect();
        statuses.sort_by(|left, right| left.project_id.cmp(&right.project_id));
        statuses
    }

    pub async fn len(&self) -> usize {
        self.lifecycles.read().await.len()
    }

    pub async fn remove(&self, project_id: &str) -> bool {
        self.lifecycles.write().await.remove(project_id).is_some()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

impl Default for ProjectRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn mark_status_degraded_if_root_missing(mut status: ProjectLifecycleStatus) -> ProjectLifecycleStatus {
    if status.root_path.exists() {
        return status;
    }

    status.state = ProjectLifecycleState::Degraded;
    status.diagnostic = CodeIntelligenceDiagnostic::degraded(format!(
        "registered root is missing on disk: {}",
        status.root_path.display()
    ));
    status.last_error = Some(format!(
        "registered_root_missing:{}",
        status.root_path.display()
    ));
    status
}

fn canonicalize_registry_root(path: &Path) -> Result<PathBuf, ProjectIdError> {
    if path == Path::new("/") {
        return Err(ProjectIdError::UnsafeRoot {
            path: "/".to_string(),
        });
    }

    let canonical = path
        .canonicalize()
        .map_err(|source| ProjectIdError::Canonicalize {
            path: path.to_string_lossy().into_owned(),
            source,
        })?;

    if canonical == Path::new("/") {
        return Err(ProjectIdError::UnsafeRoot {
            path: canonical.to_string_lossy().into_owned(),
        });
    }

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::types::{derive_project_id, derive_project_id_with_existing};
    use std::collections::HashSet;
    use tempfile::tempdir;

    #[tokio::test]
    async fn project_registry_ensure_project_is_idempotent_for_same_project_root() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let project_id = derive_project_id(&root).unwrap();
        let registry = ProjectRegistry::new();

        let first = registry.ensure_project(project_id.clone(), &root).await.unwrap();
        let second = registry.ensure_project(project_id.clone(), &root).await.unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.project_id, project_id);
        assert_eq!(first.canonical_root, root.canonicalize().unwrap());
        assert_eq!(first.state, ProjectLifecycleState::Registered);
        assert_eq!(
            first.diagnostic.status,
            crate::types::CodeIntelligenceDiagnosticCode::Selected
        );
        assert_eq!(first.pending_jobs(), 0);
        assert!(first.manager_handle.is_none());
        assert!(first.worker_handle.is_none());
        assert!(first.worker_sender.is_none());
        assert!(first.last_error.is_none());
        assert_eq!(first.options, ProjectLifecycleOptions::default());
        assert_eq!(registry.len().await, 1);
    }

    #[tokio::test]
    async fn project_registry_status_includes_project_root_and_lifecycle_state() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let project_id = derive_project_id(&root).unwrap();
        let registry = ProjectRegistry::new();

        registry.ensure_project(project_id.clone(), &root).await.unwrap();
        let status = registry.status(&project_id).await.unwrap();

        assert_eq!(status.project_id, project_id);
        assert_eq!(status.root_path, root.canonicalize().unwrap());
        assert_eq!(status.state, ProjectLifecycleState::Registered);
        assert_eq!(
            status.diagnostic.status,
            crate::types::CodeIntelligenceDiagnosticCode::Selected
        );
        assert_eq!(status.pending_jobs, 0);
        assert!(!status.has_manager_handle);
        assert!(!status.has_worker_handle);
        assert!(!status.has_worker_sender);
        assert!(status.last_error.is_none());
        assert_eq!(status.options, ProjectLifecycleOptions::default());
    }

    #[tokio::test]
    async fn project_registry_statuses_are_inspectable_and_stable() {
        let first_dir = tempdir().unwrap();
        let second_dir = tempdir().unwrap();
        let first_root = first_dir.path().join("alpha");
        let second_root = second_dir.path().join("beta");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        let registry = ProjectRegistry::new();

        registry.ensure_project("zeta", &second_root).await.unwrap();
        registry.ensure_project("alpha", &first_root).await.unwrap();
        let statuses = registry.statuses().await;

        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].project_id, "alpha");
        assert_eq!(statuses[0].root_path, first_root.canonicalize().unwrap());
        assert_eq!(statuses[0].state, ProjectLifecycleState::Registered);
        assert_eq!(statuses[1].project_id, "zeta");
        assert_eq!(statuses[1].root_path, second_root.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn project_registry_ensure_project_uses_canonical_root_for_duplicate_paths() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let alias = root.join(".");
        let project_id = derive_project_id(&root).unwrap();
        let registry = ProjectRegistry::new();

        let first = registry.ensure_project(project_id.clone(), &root).await.unwrap();
        let second = registry.ensure_project(project_id, &alias).await.unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(registry.len().await, 1);
    }

    #[tokio::test]
    async fn project_registry_ensure_project_rejects_conflicting_root_for_same_project_id() {
        let first_dir = tempdir().unwrap();
        let second_dir = tempdir().unwrap();
        let first_root = first_dir.path().join("project");
        let second_root = second_dir.path().join("project");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();
        let registry = ProjectRegistry::new();

        registry
            .ensure_project("explicit-project", &first_root)
            .await
            .unwrap();
        let error = registry
            .ensure_project("explicit-project", &second_root)
            .await
            .unwrap_err();

        let conflict = error.root_conflict().expect("expected root conflict");
        assert_eq!(conflict.project_id, "explicit-project");
        assert_eq!(conflict.existing_root, first_root.canonicalize().unwrap());
        assert_eq!(conflict.requested_root, second_root.canonicalize().unwrap());
        assert_eq!(registry.len().await, 1);
        let stored = registry.get("explicit-project").await.unwrap();
        assert_eq!(stored.canonical_root, first_root.canonicalize().unwrap());
    }

    #[tokio::test]
    async fn project_registry_same_leaf_collision_requires_explicit_suffixed_project_id() {
        let first_dir = tempdir().unwrap();
        let second_dir = tempdir().unwrap();
        let first_root = first_dir.path().join("project");
        let second_root = second_dir.path().join("project");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();

        let first_id = derive_project_id(&first_root).unwrap();
        let mut existing_ids = HashSet::new();
        existing_ids.insert(first_id.clone());
        let second_id = derive_project_id_with_existing(&second_root, &existing_ids).unwrap();
        let registry = ProjectRegistry::new();

        registry.ensure_project(first_id.clone(), &first_root).await.unwrap();
        let conflict = registry
            .ensure_project(first_id.clone(), &second_root)
            .await
            .unwrap_err();
        let conflict = conflict.root_conflict().expect("expected root conflict");

        assert_eq!(first_id, "project");
        assert!(second_id.starts_with("project-"));
        assert_ne!(first_id, second_id);
        assert_eq!(conflict.project_id, first_id);
        assert_eq!(conflict.existing_root, first_root.canonicalize().unwrap());
        assert_eq!(conflict.requested_root, second_root.canonicalize().unwrap());
        assert_eq!(registry.len().await, 1);

        registry.ensure_project(second_id.clone(), &second_root).await.unwrap();
        assert_eq!(registry.len().await, 2);
        assert_eq!(
            registry.status(&first_id).await.unwrap().root_path,
            first_root.canonicalize().unwrap()
        );
        assert_eq!(
            registry.status(&second_id).await.unwrap().root_path,
            second_root.canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn project_registry_ensure_project_concurrent_duplicates_share_lifecycle() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        let project_id = derive_project_id(&root).unwrap();
        let registry = Arc::new(ProjectRegistry::new());

        let mut tasks = Vec::new();
        for _ in 0..16 {
            let registry = registry.clone();
            let project_id = project_id.clone();
            let root = root.clone();
            tasks.push(tokio::spawn(async move {
                registry.ensure_project(project_id, root).await.unwrap()
            }));
        }

        let mut lifecycles = Vec::new();
        for task in tasks {
            lifecycles.push(task.await.unwrap());
        }

        let first = lifecycles.first().unwrap().clone();
        assert!(lifecycles.iter().all(|lifecycle| Arc::ptr_eq(&first, lifecycle)));
        assert_eq!(registry.len().await, 1);
    }

    #[tokio::test]
    async fn project_registry_ensure_project_rejects_path_outside_allowlist() {
        let allowed_dir = tempdir().unwrap();
        let requested_dir = tempdir().unwrap();
        let allowed_root = allowed_dir.path().join("allowed");
        let requested_root = requested_dir.path().join("outside");
        std::fs::create_dir_all(&allowed_root).unwrap();
        std::fs::create_dir_all(&requested_root).unwrap();

        let policy = ProjectRegistryPolicy {
            allowed_roots: Some(vec![allowed_root.canonicalize().unwrap()]),
            max_projects: 5,
        };
        let registry = ProjectRegistry::with_policy(policy);
        let project_id = derive_project_id(&requested_root).unwrap();

        let error = registry
            .ensure_project(project_id, &requested_root)
            .await
            .unwrap_err();

        assert_eq!(error.reason_code(), "path_not_allowed");
        assert!(matches!(error, ProjectRegistryError::PathNotAllowed { .. }));
        assert_eq!(registry.len().await, 0);
    }

    #[tokio::test]
    async fn project_registry_ensure_project_enforces_max_project_limit() {
        let first_dir = tempdir().unwrap();
        let second_dir = tempdir().unwrap();
        let first_root = first_dir.path().join("first");
        let second_root = second_dir.path().join("second");
        std::fs::create_dir_all(&first_root).unwrap();
        std::fs::create_dir_all(&second_root).unwrap();

        let registry = ProjectRegistry::with_policy(ProjectRegistryPolicy {
            allowed_roots: None,
            max_projects: 1,
        });

        registry.ensure_project("first", &first_root).await.unwrap();
        let error = registry
            .ensure_project("second", &second_root)
            .await
            .unwrap_err();

        assert_eq!(error.reason_code(), "max_project_limit");
        assert!(matches!(error, ProjectRegistryError::MaxProjectLimit { .. }));
        assert_eq!(registry.len().await, 1);
    }

    #[tokio::test]
    async fn project_registry_status_marks_missing_root_as_degraded() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir_all(&root).unwrap();

        let registry = ProjectRegistry::new();
        registry.ensure_project("project", &root).await.unwrap();
        std::fs::remove_dir_all(&root).unwrap();

        let status = registry.status("project").await.unwrap();
        assert_eq!(status.state, ProjectLifecycleState::Degraded);
        assert_eq!(status.diagnostic.status, crate::types::CodeIntelligenceDiagnosticCode::Degraded);
        assert_eq!(status.diagnostic.reason_code, crate::types::CodeIntelligenceDiagnosticCode::Degraded);
        assert!(status
            .last_error
            .unwrap_or_default()
            .contains("registered_root_missing"));
    }
}
