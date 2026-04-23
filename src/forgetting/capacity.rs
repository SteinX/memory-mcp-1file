use std::cmp::Ordering;
use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tokio::sync::{watch, Mutex};
use tokio::time::MissedTickBehavior;

use crate::forgetting::config::{capacity_controller_enabled, ForgettingConfig};
use crate::forgetting::decay::compute_decay;
use crate::storage::MemoryStorage;
use crate::storage::traits::CapacityMemoryCandidate;
use crate::types::SearchResult;
use crate::Result;

pub const CAPACITY_CONTROLLER_INVALIDATION_REASON: &str = "capacity_controller_archive";
const RECENT_ACCESS_WINDOW_MINUTES: i64 = 5;

/// Background controller that archives low-value memories once capacity exceeds the soft limit.
pub struct CapacityController {
    config: ForgettingConfig,
    db: Arc<dyn MemoryStorage + Send + Sync>,
    shutdown: watch::Receiver<bool>,
    run_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CapacityRunStats {
    before_count: usize,
    target_count: usize,
    requested_archives: usize,
    archived: usize,
    skipped_recent: usize,
}

#[derive(Debug, Clone)]
struct ScoredCandidate {
    candidate: CapacityMemoryCandidate,
    effective_score: f32,
}

impl CapacityController {
    /// Create a controller with its own run lock.
    pub fn new(
        config: ForgettingConfig,
        db: Arc<dyn MemoryStorage + Send + Sync>,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self::with_lock(config, db, shutdown, Arc::new(Mutex::new(())))
    }

    fn with_lock(
        config: ForgettingConfig,
        db: Arc<dyn MemoryStorage + Send + Sync>,
        shutdown: watch::Receiver<bool>,
        run_lock: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            config,
            db,
            shutdown,
            run_lock,
        }
    }

    /// Run the periodic capacity controller until shutdown.
    pub async fn run(mut self) {
        if !capacity_controller_enabled() {
            tracing::info!("Capacity controller disabled by MEMORY_CAPACITY_CONTROLLER_ENABLED");
            return;
        }

        let mut interval = tokio::time::interval(self.config.check_interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        tracing::info!(
            check_interval_secs = self.config.check_interval.as_secs(),
            soft_limit = self.config.soft_limit,
            cleanup_target_ratio = self.config.cleanup_target_ratio,
            "Capacity controller started"
        );

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(error) = self.run_cycle().await {
                        tracing::warn!(error = %error, "Capacity controller cycle failed");
                    }
                }
                changed = self.shutdown.changed() => {
                    if changed.is_err() || *self.shutdown.borrow() {
                        tracing::info!("Capacity controller shutdown complete");
                        break;
                    }
                }
            }
        }
    }

    /// Execute a single cleanup cycle and return the observed stats.
    async fn run_cycle(&self) -> Result<CapacityRunStats> {
        let _run_guard = match self.run_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                tracing::debug!("Capacity controller run skipped; previous run still active");
                return Ok(CapacityRunStats::default());
            }
        };

        let before_count = self.db.count_valid_memories().await?;
        if before_count <= self.config.soft_limit {
            tracing::info!(count = before_count, "Capacity OK: {before_count} memories");
            return Ok(CapacityRunStats {
                before_count,
                ..CapacityRunStats::default()
            });
        }

        let target_count = ((self.config.soft_limit as f64)
            * f64::from(self.config.cleanup_target_ratio)) as usize;
        let requested_archives = before_count.saturating_sub(target_count);
        let candidates = self.db.list_capacity_candidates().await?;
        let mut scored = self.rank_candidates(candidates);
        scored.sort_by(Self::compare_candidates);

        let mut archived = 0usize;
        let mut skipped_recent = 0usize;
        for scored_candidate in scored.into_iter().take(requested_archives) {
            if self
                .was_accessed_recently(&scored_candidate.candidate.id)
                .await?
            {
                skipped_recent += 1;
                continue;
            }

            if self
                .db
                .invalidate_memory(
                    scored_candidate.candidate.id,
                    Some(CAPACITY_CONTROLLER_INVALIDATION_REASON.to_string()),
                )
                .await?
            {
                archived += 1;
            }
        }

        let after_count = before_count.saturating_sub(archived);
        tracing::info!(
            archived,
            skipped_recent,
            before_count,
            after_count,
            target_count,
            requested_archives,
            "Capacity controller archived {archived} memories (count: {before_count} → {after_count})"
        );

        Ok(CapacityRunStats {
            before_count,
            target_count,
            requested_archives,
            archived,
            skipped_recent,
        })
    }

    /// Convert raw candidates into scored entries for sorting.
    fn rank_candidates(&self, candidates: Vec<CapacityMemoryCandidate>) -> Vec<ScoredCandidate> {
        candidates
            .into_iter()
            .map(|candidate| ScoredCandidate {
                effective_score: self.compute_effective_score(&candidate),
                candidate,
            })
            .collect()
    }

    /// Compute the effective score used to decide which memories are archived first.
    fn compute_effective_score(&self, candidate: &CapacityMemoryCandidate) -> f32 {
        let search_result = SearchResult {
            id: candidate.id.clone(),
            content: String::new(),
            content_hash: None,
            memory_type: candidate.memory_type.clone(),
            score: candidate.importance_score,
            importance_score: candidate.importance_score,
            event_time: candidate.event_time,
            ingestion_time: candidate.ingestion_time,
            access_count: candidate.access_count,
            last_accessed_at: candidate.last_accessed_at,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            metadata: None,
            superseded_by: None,
            valid_until: None,
            invalidation_reason: None,
            consolidation_trace: None,
            replacement_lineage: None,
            attention_summary: None,
            operator_summary: None,
        };
        let (_, _, final_score) = compute_decay(&search_result, candidate.importance_score);
        final_score
    }

    /// Sort lower-scoring candidates first, then prefer older anchors and stable IDs.
    fn compare_candidates(left: &ScoredCandidate, right: &ScoredCandidate) -> Ordering {
        left.effective_score
            .partial_cmp(&right.effective_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| Self::candidate_anchor(left).cmp(&Self::candidate_anchor(right)))
            .then_with(|| left.candidate.id.cmp(&right.candidate.id))
    }

    /// Pick the timestamp used as the deterministic tie-breaker anchor.
    fn candidate_anchor(candidate: &ScoredCandidate) -> DateTime<Utc> {
        candidate
            .candidate
            .event_time
            .or(candidate.candidate.ingestion_time)
            .unwrap_or_else(Utc::now)
    }

    /// Check whether a candidate was accessed inside the recent-access grace window.
    async fn was_accessed_recently(&self, id: &str) -> Result<bool> {
        let cutoff = Utc::now() - ChronoDuration::minutes(RECENT_ACCESS_WINDOW_MINUTES);
        let last_accessed_at = self.db.get_memory_last_accessed_at(id.to_string()).await?;
        Ok(last_accessed_at.map(|timestamp| timestamp > cutoff).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use crate::storage::{StorageBackend, SurrealStorage};
    use crate::types::{Memory, MemoryQuery, MemoryType};
    use tempfile::tempdir;
    use tokio::sync::{watch, Notify};

    use super::*;

    #[derive(Default)]
    struct TestMemoryStorageState {
        count_valid_memories: usize,
        candidates: Vec<CapacityMemoryCandidate>,
        last_accessed_by_id: HashMap<String, Option<DateTime<Utc>>>,
        invalidated: Vec<(String, Option<String>)>,
    }

    struct TestMemoryStorage {
        state: StdMutex<TestMemoryStorageState>,
        count_delay: Option<Duration>,
        block_count: Option<Arc<Notify>>,
    }

    impl TestMemoryStorage {
        fn new(state: TestMemoryStorageState) -> Self {
            Self {
                state: StdMutex::new(state),
                count_delay: None,
                block_count: None,
            }
        }

        fn with_count_delay(mut self, delay: Duration, notify: Arc<Notify>) -> Self {
            self.count_delay = Some(delay);
            self.block_count = Some(notify);
            self
        }

        fn invalidated_ids(&self) -> Vec<String> {
            self.state
                .lock()
                .unwrap()
                .invalidated
                .iter()
                .map(|(id, _)| id.clone())
                .collect()
        }
    }

    impl MemoryStorage for TestMemoryStorage {
        fn record_memory_access(
            &self,
            _id: String,
            _accessed_at: DateTime<Utc>,
        ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }

        fn count_valid_memories(&self) -> Pin<Box<dyn Future<Output = Result<usize>> + Send + '_>> {
            Box::pin(async move {
                if let Some(notify) = &self.block_count {
                    notify.notify_waiters();
                }
                if let Some(delay) = self.count_delay {
                    tokio::time::sleep(delay).await;
                }
                Ok(self.state.lock().unwrap().count_valid_memories)
            })
        }

        fn list_capacity_candidates(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<CapacityMemoryCandidate>>> + Send + '_>> {
            Box::pin(async move { Ok(self.state.lock().unwrap().candidates.clone()) })
        }

        fn get_memory_last_accessed_at(
            &self,
            id: String,
        ) -> Pin<Box<dyn Future<Output = Result<Option<DateTime<Utc>>>> + Send + '_>> {
            Box::pin(async move {
                Ok(self
                    .state
                    .lock()
                    .unwrap()
                    .last_accessed_by_id
                    .get(&id)
                    .cloned()
                    .flatten())
            })
        }

        fn invalidate_memory(
            &self,
            id: String,
            reason: Option<String>,
        ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + '_>> {
            Box::pin(async move {
                self.state.lock().unwrap().invalidated.push((id, reason));
                Ok(true)
            })
        }
    }

    fn candidate(
        id: &str,
        memory_type: MemoryType,
        age_days: i64,
        access_count: u32,
        importance_score: f32,
        last_accessed_at: Option<DateTime<Utc>>,
    ) -> CapacityMemoryCandidate {
        CapacityMemoryCandidate {
            id: id.to_string(),
            memory_type,
            event_time: Some(Utc::now() - ChronoDuration::days(age_days)),
            ingestion_time: None,
            access_count,
            last_accessed_at,
            importance_score,
        }
    }

    #[tokio::test]
    async fn run_returns_immediately_when_feature_flag_disabled() {
        let previous = std::env::var("MEMORY_CAPACITY_CONTROLLER_ENABLED").ok();
        std::env::set_var("MEMORY_CAPACITY_CONTROLLER_ENABLED", "false");

        let storage = Arc::new(TestMemoryStorage::new(TestMemoryStorageState {
            count_valid_memories: 10_001,
            candidates: vec![candidate("m1", MemoryType::Semantic, 200, 0, 1.0, None)],
            ..TestMemoryStorageState::default()
        }));
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let controller = CapacityController::new(
            ForgettingConfig::default(),
            storage.clone(),
            shutdown_rx,
        );

        controller.run().await;

        assert!(storage.invalidated_ids().is_empty());

        match previous {
            Some(value) => std::env::set_var("MEMORY_CAPACITY_CONTROLLER_ENABLED", value),
            None => std::env::remove_var("MEMORY_CAPACITY_CONTROLLER_ENABLED"),
        }
    }

    #[tokio::test]
    async fn archives_lowest_scoring_memories_and_preserves_temporal_queries() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(SurrealStorage::new(dir.path(), 8).await.unwrap());

        let stale_id = create_memory_at(
            &storage,
            "stale memory",
            MemoryType::Episodic,
            Utc::now() - ChronoDuration::days(180),
            1.0,
            0,
            None,
        )
        .await;
        let _stable_id = create_memory_at(
            &storage,
            "reinforced memory",
            MemoryType::Semantic,
            Utc::now() - ChronoDuration::days(1),
            3.0,
            10,
            Some(Utc::now() - ChronoDuration::minutes(30)),
        )
        .await;
        let _recent_id = create_memory_at(
            &storage,
            "recently accessed",
            MemoryType::Episodic,
            Utc::now() - ChronoDuration::days(365),
            1.0,
            0,
            Some(Utc::now() - ChronoDuration::minutes(1)),
        )
        .await;

        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let controller = CapacityController::new(
            ForgettingConfig {
                soft_limit: 2,
                cleanup_target_ratio: 0.5,
                check_interval: Duration::from_secs(60),
                ..ForgettingConfig::default()
            },
            storage.clone(),
            shutdown_rx,
        );

        let before_archive = crate::types::Datetime::from(Utc::now() - ChronoDuration::seconds(1));
        let stats = controller.run_cycle().await.unwrap();

        assert_eq!(stats.before_count, 3);
        assert_eq!(stats.target_count, 1);
        assert_eq!(stats.requested_archives, 2);
        assert_eq!(stats.archived, 1);
        assert_eq!(stats.skipped_recent, 1);

        let current_valid = storage.get_valid(&MemoryQuery::default(), 10).await.unwrap();
        let current_ids: Vec<String> = current_valid
            .into_iter()
            .filter_map(|memory| memory.id.map(|id| crate::types::record_key_to_string(&id.key)))
            .collect();
        assert!(!current_ids.contains(&stale_id));

        let historical = storage
            .get_valid_at(
                &MemoryQuery {
                    valid_at: Some(before_archive),
                    ..MemoryQuery::default()
                },
                10,
            )
            .await
            .unwrap();
        let historical_ids: Vec<String> = historical
            .into_iter()
            .filter_map(|memory| memory.id.map(|id| crate::types::record_key_to_string(&id.key)))
            .collect();
        assert!(historical_ids.contains(&stale_id));

        let stale_memory = storage.get_memory(&stale_id).await.unwrap().unwrap();
        assert_eq!(
            stale_memory.invalidation_reason.as_deref(),
            Some(CAPACITY_CONTROLLER_INVALIDATION_REASON)
        );
        assert!(stale_memory.valid_until.is_some());
    }

    #[tokio::test]
    async fn concurrent_runs_are_skipped_when_cleanup_is_already_active() {
        let notify = Arc::new(Notify::new());
        let storage = Arc::new(
            TestMemoryStorage::new(TestMemoryStorageState {
                count_valid_memories: 10_001,
                candidates: vec![candidate("m1", MemoryType::Semantic, 200, 0, 1.0, None)],
                ..TestMemoryStorageState::default()
            })
            .with_count_delay(Duration::from_millis(150), notify.clone()),
        );
        let shared_lock = Arc::new(Mutex::new(()));
        let (_shutdown_tx_a, shutdown_rx_a) = watch::channel(false);
        let (_shutdown_tx_b, shutdown_rx_b) = watch::channel(false);
        let controller_a = CapacityController::with_lock(
            ForgettingConfig::default(),
            storage.clone(),
            shutdown_rx_a,
            shared_lock.clone(),
        );
        let controller_b = CapacityController::with_lock(
            ForgettingConfig::default(),
            storage.clone(),
            shutdown_rx_b,
            shared_lock,
        );

        let first_run = controller_a.run_cycle();
        tokio::pin!(first_run);
        notify.notified().await;
        let second_run = controller_b.run_cycle();
        let (first_stats, second_stats) = tokio::join!(first_run, second_run);
        let first_stats = first_stats.unwrap();
        let second_stats = second_stats.unwrap();

        assert_eq!(second_stats, CapacityRunStats::default());
        assert_eq!(first_stats.archived, 1);
        assert_eq!(storage.invalidated_ids(), vec!["m1".to_string()]);
    }

    async fn create_memory_at(
        storage: &Arc<SurrealStorage>,
        content: &str,
        memory_type: MemoryType,
        event_time: DateTime<Utc>,
        importance_score: f32,
        access_count: u32,
        last_accessed_at: Option<DateTime<Utc>>,
    ) -> String {
        let memory = Memory {
            content: content.to_string(),
            memory_type,
            event_time: crate::types::Datetime::from(event_time),
            ingestion_time: crate::types::Datetime::from(event_time),
            valid_from: crate::types::Datetime::from(event_time),
            importance_score,
            access_count,
            last_accessed_at: last_accessed_at.map(crate::types::Datetime::from),
            ..Memory::default()
        };

        storage.create_memory(memory).await.unwrap()
    }
}
