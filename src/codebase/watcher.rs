use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use super::scanner::{is_code_file, is_ignored_file};
use crate::Result;

pub struct FileWatcher {
    paths: Vec<PathBuf>,
    watcher: Option<RecommendedWatcher>,
    debounce_duration: Duration,
    cancel_tx: Option<mpsc::Sender<()>>,
}

impl FileWatcher {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            watcher: None,
            debounce_duration: Duration::from_secs(2),
            cancel_tx: None,
        }
    }

    pub fn start<F>(&mut self, callback: F, mut shutdown_rx: watch::Receiver<bool>) -> Result<()>
    where
        F: Fn(Vec<PathBuf>) + Send + Sync + 'static,
    {
        let (tx, mut rx) = mpsc::channel(100);
        let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);
        let debounce_duration = self.debounce_duration;

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() {
                    for path in event.paths {
                        if !is_ignored_file(&path) && is_code_file(&path) {
                            let _ = tx.blocking_send(path);
                        }
                    }
                }
            }
        })?;

        for path in &self.paths {
            if path.exists() {
                watcher.watch(path, RecursiveMode::Recursive)?;
                info!("Watching path: {:?}", path);
            } else {
                warn!("Path does not exist: {:?}", path);
            }
        }

        self.watcher = Some(watcher);
        self.cancel_tx = Some(cancel_tx);

        // Debounce logic in a background task
        tokio::spawn(async move {
            let mut last_event: Option<tokio::time::Instant> = None;
            let mut pending_paths = HashSet::new();

            loop {
                let sleep_duration = if last_event.is_some() {
                    Duration::from_millis(500)
                } else {
                    Duration::from_secs(3600) // Sleep long if nothing is happening
                };

                tokio::select! {
                    received = rx.recv() => {
                        if let Some(path) = received {
                            pending_paths.insert(path);
                            last_event = Some(tokio::time::Instant::now());
                        } else {
                            break;
                        }
                    }
                    _ = cancel_rx.recv() => {
                        info!("Watcher debounce cancelled, discarding {} pending paths", pending_paths.len());
                        break;
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("Watcher received shutdown signal, discarding {} pending paths", pending_paths.len());
                            break;
                        }
                    }
                    _ = tokio::time::sleep(sleep_duration) => {
                        if let Some(last) = last_event {
                            if last.elapsed() >= debounce_duration {
                                let paths: Vec<PathBuf> = pending_paths.drain().collect();
                                if !paths.is_empty() {
                                    callback(paths);
                                }
                                last_event = None;
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    pub fn stop(&mut self) {
        self.watcher = None;
        self.cancel_tx = None;
        info!("Stopped file watcher");
    }
}
