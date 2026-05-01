use std::collections::HashSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectIdError {
    #[error("unsafe project root is not allowed: {path}")]
    UnsafeRoot { path: String },

    #[error("project root is not valid UTF-8: {path}")]
    NonUtf8Root { path: String },

    #[error("failed to canonicalize project root {path}: {source}")]
    Canonicalize {
        path: String,
        source: std::io::Error,
    },

    #[error("project root has no leaf directory: {path}")]
    MissingLeaf { path: String },
}

pub fn derive_project_id(path: &Path) -> Result<String, ProjectIdError> {
    derive_project_id_with_existing(path, &HashSet::new())
}

pub fn derive_project_id_with_existing(
    path: &Path,
    existing_ids: &HashSet<String>,
) -> Result<String, ProjectIdError> {
    let canonical = canonicalize_project_root(path)?;
    let canonical_utf8 = canonical
        .to_str()
        .ok_or_else(|| ProjectIdError::NonUtf8Root {
            path: canonical.to_string_lossy().into_owned(),
        })?;

    let leaf = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ProjectIdError::MissingLeaf {
            path: canonical_utf8.to_string(),
        })?
        .to_string();

    if !existing_ids.contains(&leaf) {
        return Ok(leaf);
    }

    Ok(format!("{}-{}", leaf, short_hash(canonical_utf8)))
}

fn canonicalize_project_root(path: &Path) -> Result<PathBuf, ProjectIdError> {
    if path == Path::new("/") {
        return Err(ProjectIdError::UnsafeRoot {
            path: "/".to_string(),
        });
    }

    let canonical = path.canonicalize().map_err(|source| ProjectIdError::Canonicalize {
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

fn short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    #[test]
    fn derives_project_for_compatibility_root() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        let project_id = derive_project_id(&project).unwrap();

        assert_eq!(project_id, "project");
    }

    #[test]
    fn rejects_root_path() {
        let error = derive_project_id(Path::new("/")).unwrap_err();

        match error {
            ProjectIdError::UnsafeRoot { path } => assert_eq!(path, "/"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn handles_trailing_slash_and_canonical_path() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("workspace");
        std::fs::create_dir_all(&project).unwrap();

        let with_slash = PathBuf::from(format!("{}/", project.display()));
        let project_id = derive_project_id(&with_slash).unwrap();

        assert_eq!(project_id, "workspace");
    }

    #[test]
    fn uses_canonical_path_for_symlink_collisions() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target-root");
        let link = dir.path().join("linked-root");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let mut existing = HashSet::new();
        existing.insert("target-root".to_string());

        let project_id = derive_project_id_with_existing(&link, &existing).unwrap();
        let canonical = target.canonicalize().unwrap();
        let expected_hash = short_hash(canonical.to_str().unwrap());

        assert_eq!(project_id, format!("target-root-{}", expected_hash));
    }

    #[test]
    fn preserves_unicode_leaf_and_hashes_canonical_path() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("项目");
        std::fs::create_dir_all(&project).unwrap();

        let project_id = derive_project_id(&project).unwrap();

        assert_eq!(project_id, "项目");

        let mut existing = HashSet::new();
        existing.insert("项目".to_string());

        let suffixed = derive_project_id_with_existing(&project, &existing).unwrap();
        let canonical = project.canonicalize().unwrap();
        let expected_hash = short_hash(canonical.to_str().unwrap());

        assert_eq!(suffixed, format!("项目-{}", expected_hash));
    }

    #[test]
    fn suffixes_same_leaf_from_different_roots() {
        let first_dir = tempdir().unwrap();
        let second_dir = tempdir().unwrap();
        let first = first_dir.path().join("project");
        let second = second_dir.path().join("project");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();

        let first_id = derive_project_id(&first).unwrap();
        let mut existing = HashSet::new();
        existing.insert(first_id.clone());

        let second_id = derive_project_id_with_existing(&second, &existing).unwrap();
        let expected_hash = short_hash(second.canonicalize().unwrap().to_str().unwrap());

        assert_eq!(first_id, "project");
        assert_eq!(second_id, format!("project-{}", expected_hash));
    }
}
