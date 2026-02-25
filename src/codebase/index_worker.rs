//! Background worker that processes [`IndexJob`] messages from a channel and
//! runs incremental indexing + BM25 rebuild with batching and debouncing.
//!
//! # Design
//!
//! * The watcher and `validate_index_full` both push jobs through an
//!   [`tokio::sync::mpsc::UnboundedSender<IndexJob>`] rather than calling
//!   `incremental_index` directly, decoupling I/O detection from indexing work.
//!
//! * The worker reads jobs in batches of up to [`BATCH_SIZE`] or waits up to
//!   [`DEBOUNCE_MS`] milliseconds for the batch to fill, whichever comes first.
//!   This collapses rapid bursts of file-save events into a single index pass.
//!
//! * After each successful incremental index pass the worker updates the
//!   `file_manifest` table so the next `validate_index_full` diff is accurate.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tracing::{debug, error, info, warn};

use crate::config::AppState;
use crate::storage::StorageBackend;
use crate::Result;

use super::indexer::incremental_index;

// ── Constants ────────────────────────────────────────────────────────────────
// Default values are defined in `AppConfig` (config.rs).
// `index_batch_size` and `index_debounce_ms` fields are read at runtime.

// ── Public types ─────────────────────────────────────────────────────────────

/// A job sent to the [`IndexWorker`].
#[derive(Debug)]
pub enum IndexJob {
    /// File was created or modified — (re-)index its content.
    Upsert(PathBuf),
    /// File was deleted — remove its chunks/symbols from the index.
    Delete(PathBuf),
}

/// Channel sender used to push jobs into the worker.
pub type IndexJobSender = UnboundedSender<IndexJob>;

// ── Worker ───────────────────────────────────────────────────────────────────

/// Background worker that processes [`IndexJob`] messages.
///
/// Create one per project with [`IndexWorker::new`] then call
/// [`IndexWorker::start`] to drive it in a background Tokio task.
pub struct IndexWorker {
    state: Arc<AppState>,
    project_id: String,
    rx: UnboundedReceiver<IndexJob>,
}

impl IndexWorker {
    /// Create an `IndexWorker` for `project_id`.
    ///
    /// Returns `(worker, sender)` — keep the sender alive and clone it for
    /// every component that needs to push jobs.
    pub fn new(state: Arc<AppState>, project_id: impl Into<String>) -> (Self, IndexJobSender) {
        let (tx, rx) = mpsc::unbounded_channel();
        let worker = Self {
            state,
            project_id: project_id.into(),
            rx,
        };
        (worker, tx)
    }

    /// Spawn the worker's event loop in a background Tokio task.
    ///
    /// The task exits when the sender side of the channel is dropped (i.e.
    /// when the application shuts down) or when the `shutdown` watch fires.
    pub fn start(self, mut shutdown_rx: tokio::sync::watch::Receiver<bool>) {
        let project_id = self.project_id.clone();
        tokio::spawn(async move {
            info!(project_id = %project_id, "IndexWorker started");
            if let Err(e) = self.run(&mut shutdown_rx).await {
                error!(project_id = %project_id, error = %e, "IndexWorker terminated with error");
            } else {
                info!(project_id = %project_id, "IndexWorker stopped");
            }
        });
    }

    // ── Internal event loop ───────────────────────────────────────────────

    async fn run(mut self, shutdown_rx: &mut tokio::sync::watch::Receiver<bool>) -> Result<()> {
        loop {
            // ── 1. Wait for the first job (or shutdown) ───────────────────
            let first = tokio::select! {
                job = self.rx.recv() => match job {
                    Some(j) => j,
                    None => {
                        debug!(project_id = %self.project_id, "IndexWorker channel closed, exiting");
                        return Ok(());
                    }
                },
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        return Ok(());
                    }
                    continue;
                }
            };

            // ── 2. Accumulate a batch (debounce window) ───────────────────
            let mut upserts: HashMap<PathBuf, ()> = HashMap::new();
            let mut deletes: HashSet<PathBuf> = HashSet::new();

            Self::classify(first, &mut upserts, &mut deletes);

            let debounce_ms = self.state.config.index_debounce_ms;
            let batch_size = self.state.config.index_batch_size;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(debounce_ms);

            loop {
                if upserts.len() + deletes.len() >= batch_size {
                    break; // batch full — flush immediately
                }

                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break; // debounce window elapsed
                }

                tokio::select! {
                    job = self.rx.recv() => match job {
                        Some(j) => Self::classify(j, &mut upserts, &mut deletes),
                        None => {
                            // Channel closed — flush what we have, then exit
                            self.flush(upserts, deletes).await;
                            return Ok(());
                        }
                    },
                    _ = tokio::time::sleep(remaining) => break,
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            // Flush pending work before honouring shutdown
                            self.flush(upserts, deletes).await;
                            return Ok(());
                        }
                    }
                }
            }

            self.flush(upserts, deletes).await;
        }
    }

    /// Route a single job into the appropriate accumulation set.
    ///
    /// A `Delete` for a path that was previously queued for `Upsert` cancels
    /// the upsert; an `Upsert` for a previously queued `Delete` resurrects it.
    fn classify(job: IndexJob, upserts: &mut HashMap<PathBuf, ()>, deletes: &mut HashSet<PathBuf>) {
        match job {
            IndexJob::Upsert(p) => {
                deletes.remove(&p);
                upserts.insert(p, ());
            }
            IndexJob::Delete(p) => {
                upserts.remove(&p);
                deletes.insert(p);
            }
        }
    }

    // ── Flush ─────────────────────────────────────────────────────────────

    async fn flush(&self, upserts: HashMap<PathBuf, ()>, deletes: HashSet<PathBuf>) {
        let project_id = &self.project_id;

        // ── Handle explicit deletes ───────────────────────────────────────
        for path in &deletes {
            let path_str = path.to_string_lossy();
            let _ = self
                .state
                .storage
                .delete_chunks_by_path(project_id, &path_str)
                .await;
            let _ = self
                .state
                .storage
                .delete_symbols_by_path(project_id, &path_str)
                .await;
            let _ = self
                .state
                .storage
                .delete_file_hash(project_id, &path_str)
                .await;
            let _ = self
                .state
                .storage
                .delete_manifest_entry(project_id, &path_str)
                .await;
            debug!(project_id = %project_id, path = %path_str, "IndexWorker: deleted file");
        }

        // ── Run incremental index for upserts ─────────────────────────────
        let upsert_paths: Vec<PathBuf> = upserts.into_keys().collect();
        let changed = !upsert_paths.is_empty();
        let deleted = !deletes.is_empty();

        if !upsert_paths.is_empty() {
            info!(
                project_id = %project_id,
                files = upsert_paths.len(),
                "IndexWorker: running incremental index"
            );
            match incremental_index(self.state.clone(), project_id, upsert_paths.clone()).await {
                Ok(result) => {
                    info!(
                        project_id = %project_id,
                        updated = result.updated_files,
                        deleted_from_incr = result.deleted_files.len(),
                        "IndexWorker: incremental index done"
                    );

                    // Trigger BM25 rebuild if anything actually changed.
                    if result.updated_files > 0 || !result.new_chunks.is_empty() {
                        self.state
                            .code_search
                            .rebuild_from_storage(self.state.storage.as_ref(), project_id)
                            .await;
                    }
                }
                Err(e) => {
                    error!(project_id = %project_id, error = %e, "IndexWorker: incremental index failed");
                }
            }
        } else if deleted {
            // Only deletes happened — rebuild BM25 to reflect removed chunks.
            self.state
                .code_search
                .rebuild_from_storage(self.state.storage.as_ref(), project_id)
                .await;
        }

        // ── Update file_manifest ──────────────────────────────────────────
        if changed {
            let path_strings: Vec<String> = upsert_paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            if let Err(e) = self
                .state
                .storage
                .upsert_manifest_entries(project_id, &path_strings)
                .await
            {
                warn!(
                    project_id = %project_id,
                    error = %e,
                    "IndexWorker: failed to update file_manifest after upsert"
                );
            }
        }

        if deleted {
            // Entries already removed individually above; nothing extra to do.
            debug!(project_id = %project_id, count = deletes.len(), "IndexWorker: manifest entries removed for deleted files");
        }
    }
}
