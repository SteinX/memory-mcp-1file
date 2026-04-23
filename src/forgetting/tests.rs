use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tempfile::tempdir;
use tokio::sync::watch;

use crate::forgetting::access::AccessTracker;
use crate::forgetting::capacity::{CapacityController, CAPACITY_CONTROLLER_INVALIDATION_REASON};
use crate::forgetting::config::ForgettingConfig;
use crate::forgetting::decay::{
    apply_decay_scoring, compute_decay, decay_factor, effective_age_days, reinforcement_bonus,
};
use crate::storage::{StorageBackend, SurrealStorage};
use crate::types::{record_key_to_string, Datetime, Memory, MemoryQuery, MemoryType, ScoredMemory, SearchResult};

fn approx_eq(left: f32, right: f32, epsilon: f32) {
    assert!(
        (left - right).abs() <= epsilon,
        "left={left}, right={right}, epsilon={epsilon}"
    );
}

fn search_result(memory_type: MemoryType) -> SearchResult {
    SearchResult {
        id: "memory-1".to_string(),
        content: "content".to_string(),
        content_hash: None,
        memory_type,
        score: 1.0,
        importance_score: 1.0,
        event_time: None,
        ingestion_time: None,
        access_count: 0,
        last_accessed_at: None,
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
    }
}

fn decayed_score(memory_type: MemoryType, age_days: f64, access_count: u32) -> f32 {
    let effective_age = effective_age_days(age_days, access_count);
    let decay = decay_factor(effective_age, &memory_type);
    let bonus = reinforcement_bonus(access_count);
    apply_decay_scoring(1.0, decay, bonus)
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
        event_time: Datetime::from(event_time),
        ingestion_time: Datetime::from(event_time),
        valid_from: Datetime::from(event_time),
        importance_score,
        access_count,
        last_accessed_at: last_accessed_at.map(Datetime::from),
        ..Memory::default()
    };

    storage.create_memory(memory).await.unwrap()
}

#[test]
fn ac1_decay_correctness_half_life_halves_eposodic_score() {
    let fresh_score = decayed_score(MemoryType::Episodic, 0.0, 0);
    let month_old_score = decayed_score(MemoryType::Episodic, 30.0, 0);

    approx_eq(fresh_score, 1.0, 1e-6);
    approx_eq(month_old_score, 0.5, 1e-5);
    approx_eq(fresh_score, month_old_score * 2.0, 1e-5);
}

#[test]
fn ac2_reinforcement_boosts_same_age_memory_score() {
    let zero_access_score = decayed_score(MemoryType::Episodic, 60.0, 0);
    let reinforced_score = decayed_score(MemoryType::Episodic, 60.0, 10);

    assert!(reinforced_score > zero_access_score);
}

#[test]
fn ac3_type_differentiation_preserves_procedural_then_semantic_then_episodic() {
    let episodic_score = decayed_score(MemoryType::Episodic, 90.0, 0);
    let semantic_score = decayed_score(MemoryType::Semantic, 90.0, 0);
    let procedural_score = decayed_score(MemoryType::Procedural, 90.0, 0);

    assert!(procedural_score > semantic_score);
    assert!(semantic_score > episodic_score);
}

#[test]
fn ac4_and_ac7_zero_access_decay_is_finite_and_scored_memory_exposes_decay_factor() {
    let mut result = search_result(MemoryType::Semantic);
    result.event_time = Some(Utc::now() - chrono::Duration::days(30));
    result.access_count = 0;
    result.last_accessed_at = None;

    let (decay, bonus, final_score) = compute_decay(&result, 1.0);

    assert!(decay.is_finite());
    assert!(bonus.is_finite());
    assert!(final_score.is_finite());
    assert!((0.0..=1.0).contains(&decay));

    let scored = ScoredMemory {
        id: result.id.clone(),
        content: result.content.clone(),
        memory_type: result.memory_type.clone(),
        score: final_score,
        decay_factor: decay,
        vector_score: 1.0,
        bm25_score: 0.0,
        ppr_score: 0.0,
        importance_score: result.importance_score,
        channels: vec!["vector".to_string()],
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

    approx_eq(scored.decay_factor, decay, f32::EPSILON);
}

#[tokio::test]
async fn ac5_capacity_controller_invalidates_and_preserves_historical_validity() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(SurrealStorage::new(dir.path(), 8).await.unwrap());

    let stale_id = create_memory_at(
        &storage,
        "stale memory",
        MemoryType::Episodic,
        Utc::now() - chrono::Duration::days(180),
        1.0,
        0,
        None,
    )
    .await;
    let _durable_id = create_memory_at(
        &storage,
        "durable memory",
        MemoryType::Procedural,
        Utc::now() - chrono::Duration::days(7),
        2.0,
        6,
        Some(Utc::now() - chrono::Duration::minutes(30)),
    )
    .await;
    let _recent_id = create_memory_at(
        &storage,
        "recently accessed memory",
        MemoryType::Episodic,
        Utc::now() - chrono::Duration::days(365),
        1.0,
        0,
        Some(Utc::now() - chrono::Duration::minutes(1)),
    )
    .await;

    let before_archive = Datetime::from(Utc::now() - chrono::Duration::seconds(1));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let controller = CapacityController::new(
        ForgettingConfig {
            soft_limit: 2,
            cleanup_target_ratio: 0.5,
            check_interval: Duration::from_millis(10),
            ..ForgettingConfig::default()
        },
        storage.clone(),
        shutdown_rx,
    );

    let task = tokio::spawn(controller.run());
    tokio::time::sleep(Duration::from_millis(40)).await;
    let _ = shutdown_tx.send(true);
    task.await.unwrap();

    let current_valid = storage.get_valid(&MemoryQuery::default(), 10).await.unwrap();
    let current_ids: Vec<String> = current_valid
        .into_iter()
        .filter_map(|memory| memory.id.map(|id| record_key_to_string(&id.key)))
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
        .filter_map(|memory| memory.id.map(|id| record_key_to_string(&id.key)))
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
async fn ac6_access_tracker_emits_event_on_track() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let tracker = AccessTracker::new(sender);

    tracker.track("memory-42");

    let event = receiver.recv().await.expect("access event should be sent");
    assert_eq!(event.memory_id, "memory-42");
}
