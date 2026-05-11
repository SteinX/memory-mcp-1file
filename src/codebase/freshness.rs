use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::storage::StorageBackend;
use crate::types::{CapabilityFreshness, CodeSymbol, IndexFileCheckpoint};
use crate::Result;

fn normalize_file_path(file_path: &str) -> String {
    file_path.replace('\\', "/")
}

fn normalize_changed_paths(changed_paths: &[String]) -> HashSet<String> {
    changed_paths
        .iter()
        .map(|path| normalize_file_path(path))
        .collect()
}

fn checkpoint_path(checkpoint: &IndexFileCheckpoint) -> String {
    if checkpoint.relative_file_path.is_empty() {
        normalize_file_path(&checkpoint.file_path)
    } else {
        normalize_file_path(&checkpoint.relative_file_path)
    }
}

fn checkpoint_is_pending(checkpoint: &IndexFileCheckpoint) -> bool {
    !checkpoint.completed
}

fn checkpoints_match(serving: &IndexFileCheckpoint, indexing: &IndexFileCheckpoint) -> bool {
    !serving.content_hash.is_empty()
        && serving.content_hash == indexing.content_hash
        && checkpoint_path(serving) == checkpoint_path(indexing)
}

pub async fn classify_file_freshness(
    project_id: &str,
    file_path: &str,
    serving_gen: u64,
    indexing_gen: Option<u64>,
    storage: &impl StorageBackend,
) -> Result<CapabilityFreshness> {
    let normalized_path = normalize_file_path(file_path);
    let Some(serving_checkpoint) = storage
        .get_file_checkpoint(project_id, serving_gen, &normalized_path)
        .await?
    else {
        return Ok(CapabilityFreshness::Unavailable);
    };

    let Some(indexing_gen) = indexing_gen else {
        return Ok(CapabilityFreshness::Fresh);
    };

    if indexing_gen == serving_gen {
        return Ok(CapabilityFreshness::Fresh);
    }

    let indexing_checkpoint = storage
        .get_file_checkpoint(project_id, indexing_gen, &normalized_path)
        .await?;

    match indexing_checkpoint {
        Some(indexing_checkpoint) if checkpoint_is_pending(&indexing_checkpoint) => {
            Ok(CapabilityFreshness::Partial)
        }
        Some(indexing_checkpoint) if checkpoints_match(&serving_checkpoint, &indexing_checkpoint) => {
            Ok(CapabilityFreshness::Fresh)
        }
        Some(_) => Ok(CapabilityFreshness::Stale),
        None => Ok(CapabilityFreshness::Fresh),
    }
}

pub async fn build_freshness_map(
    project_id: &str,
    serving_gen: Option<u64>,
    indexing_gen: Option<u64>,
    changed_paths: &[String],
    storage: &impl StorageBackend,
) -> Result<HashMap<String, CapabilityFreshness>> {
    let mut freshness_by_path = HashMap::new();
    let Some(serving_gen) = serving_gen else {
        for changed_path in changed_paths {
            freshness_by_path.insert(normalize_file_path(changed_path), CapabilityFreshness::Unavailable);
        }
        return Ok(freshness_by_path);
    };

    let changed = normalize_changed_paths(changed_paths);
    for changed_path in &changed {
        freshness_by_path.insert(changed_path.clone(), CapabilityFreshness::Stale);
    }

    let serving_checkpoints = storage
        .list_file_checkpoints_for_job(project_id, serving_gen)
        .await?;
    let indexing_checkpoints = match indexing_gen.filter(|generation| *generation != serving_gen) {
        Some(indexing_gen) => storage
            .list_file_checkpoints_for_job(project_id, indexing_gen)
            .await?,
        None => Vec::new(),
    };
    let indexing_by_path: HashMap<String, IndexFileCheckpoint> = indexing_checkpoints
        .into_iter()
        .map(|checkpoint| (checkpoint_path(&checkpoint), checkpoint))
        .collect();

    for serving_checkpoint in serving_checkpoints {
        let path = checkpoint_path(&serving_checkpoint);
        let freshness = match indexing_by_path.get(&path) {
            Some(indexing_checkpoint) if checkpoint_is_pending(indexing_checkpoint) => {
                CapabilityFreshness::Partial
            }
            Some(indexing_checkpoint) if checkpoints_match(&serving_checkpoint, indexing_checkpoint) => {
                CapabilityFreshness::Fresh
            }
            Some(_) => CapabilityFreshness::Stale,
            None if changed.contains(&path) => CapabilityFreshness::Stale,
            None => CapabilityFreshness::Fresh,
        };
        freshness_by_path.insert(path, freshness);
    }

    Ok(freshness_by_path)
}

pub fn classify_symbol_freshness(
    symbol: &CodeSymbol,
    file_freshness_map: &HashMap<String, CapabilityFreshness>,
) -> CapabilityFreshness {
    file_freshness_map
        .get(&normalize_file_path(&symbol.file_path))
        .cloned()
        .unwrap_or(CapabilityFreshness::Unavailable)
}

pub fn classify_path_from_map(
    file_path: &str,
    file_freshness_map: &HashMap<String, CapabilityFreshness>,
) -> Option<CapabilityFreshness> {
    file_freshness_map
        .get(&normalize_file_path(file_path))
        .cloned()
}

pub fn relative_changed_paths(project_root: Option<&Path>, changed_paths: &[String]) -> Vec<String> {
    changed_paths
        .iter()
        .map(|path| {
            let path_ref = Path::new(path);
            project_root
                .and_then(|root| path_ref.strip_prefix(root).ok())
                .map(|relative| normalize_file_path(&relative.to_string_lossy()))
                .unwrap_or_else(|| normalize_file_path(path))
        })
        .collect()
}
