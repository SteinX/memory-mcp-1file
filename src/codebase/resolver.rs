use std::path::{Path, PathBuf};

use crate::types::CodeIntelligenceDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupProjectRootStatus {
    Selected {
        path: PathBuf,
        source: StartupProjectRootSource,
    },
    MissingConfiguredRoot {
        configured_path: PathBuf,
        fallback_path: PathBuf,
        diagnostic: CodeIntelligenceDiagnostic,
    },
    Disabled {
        fallback_path: PathBuf,
        diagnostic: CodeIntelligenceDiagnostic,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupProjectRootSource {
    Configured,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupProjectRootResolution {
    pub status: StartupProjectRootStatus,
}

pub fn resolve_startup_project_root(
    configured_root: Option<&Path>,
    fallback_root: &Path,
) -> StartupProjectRootResolution {
    let status = match configured_root {
        Some(configured_root) if configured_root.exists() => StartupProjectRootStatus::Selected {
            path: configured_root.to_path_buf(),
            source: StartupProjectRootSource::Configured,
        },
        Some(configured_root) => StartupProjectRootStatus::MissingConfiguredRoot {
            configured_path: configured_root.to_path_buf(),
            fallback_path: fallback_root.to_path_buf(),
            diagnostic: CodeIntelligenceDiagnostic::missing_root(format!(
                "Configured project root is missing: {}. Server startup will continue without code intelligence auto-start.",
                configured_root.display()
            )),
        },
        None if fallback_root.exists() => StartupProjectRootStatus::Selected {
            path: fallback_root.to_path_buf(),
            source: StartupProjectRootSource::Fallback,
        },
        None => StartupProjectRootStatus::Disabled {
            fallback_path: fallback_root.to_path_buf(),
            diagnostic: CodeIntelligenceDiagnostic::disabled(format!(
                "Code intelligence auto-start is disabled because no configured project root was provided and the fallback root is unavailable: {}",
                fallback_root.display()
            )),
        },
    };

    StartupProjectRootResolution { status }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn configured_root_exists() -> StartupProjectRootResolution {
        let configured_dir = tempdir().unwrap();
        let fallback_dir = tempdir().unwrap();

        resolve_startup_project_root(Some(configured_dir.path()), fallback_dir.path())
    }

    #[test]
    fn startup_project_root_configured_exists_is_selected() {
        let resolution = configured_root_exists();

        assert!(matches!(
            resolution.status,
            StartupProjectRootStatus::Selected {
                source: StartupProjectRootSource::Configured,
                ..
            }
        ));
    }

    #[test]
    fn startup_project_root_configured_missing_is_diagnostic() {
        let fallback_dir = tempdir().unwrap();
        let missing_configured = fallback_dir.path().join("missing-configured");

        let resolution = resolve_startup_project_root(Some(&missing_configured), fallback_dir.path());

        match resolution.status {
            StartupProjectRootStatus::MissingConfiguredRoot {
                configured_path,
                fallback_path,
                diagnostic,
            } => {
                assert_eq!(configured_path, missing_configured);
                assert_eq!(fallback_path, fallback_dir.path());
                assert_eq!(diagnostic.status, crate::types::CodeIntelligenceDiagnosticCode::MissingRoot);
                assert_eq!(diagnostic.reason_code, crate::types::CodeIntelligenceDiagnosticCode::MissingRoot);
                assert!(diagnostic.message.contains("missing"));
            }
            other => panic!("expected missing configured root diagnostic, got {other:?}"),
        }
    }

    #[test]
    fn startup_project_root_fallback_exists_is_selected_when_unconfigured() {
        let fallback_dir = tempdir().unwrap();

        let resolution = resolve_startup_project_root(None, fallback_dir.path());

        assert!(matches!(
            resolution.status,
            StartupProjectRootStatus::Selected {
                path,
                source: StartupProjectRootSource::Fallback,
            } if path == fallback_dir.path()
        ));
    }

    #[test]
    fn startup_project_root_compatibility_project_dir_is_selected_without_config() {
        let fallback_dir = tempdir().unwrap();
        let compatibility_project = fallback_dir.path().join("project");
        std::fs::create_dir_all(&compatibility_project).unwrap();

        let resolution = resolve_startup_project_root(None, &compatibility_project);

        assert!(matches!(
            resolution.status,
            StartupProjectRootStatus::Selected {
                path,
                source: StartupProjectRootSource::Fallback,
            } if path == compatibility_project
        ));
    }

    #[test]
    fn startup_project_root_fallback_missing_disables_with_diagnostic() {
        let fallback_dir = tempdir().unwrap();
        let missing_fallback = fallback_dir.path().join("missing-fallback");

        let resolution = resolve_startup_project_root(None, &missing_fallback);

        match resolution.status {
            StartupProjectRootStatus::Disabled {
                fallback_path,
                diagnostic,
            } => {
                assert_eq!(fallback_path, missing_fallback);
                assert_eq!(diagnostic.status, crate::types::CodeIntelligenceDiagnosticCode::Disabled);
                assert_eq!(diagnostic.reason_code, crate::types::CodeIntelligenceDiagnosticCode::Disabled);
                assert!(diagnostic.message.contains("disabled"));
            }
            other => panic!("expected disabled resolution, got {other:?}"),
        }
    }
}
