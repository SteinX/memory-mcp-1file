//! Unified memory lifecycle and retention policy helpers.
//!
//! This module derives GC-facing lifecycle state from the existing Memory
//! fields. It intentionally does not introduce a second persisted lifecycle
//! source of truth.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::forgetting::capacity::CAPACITY_CONTROLLER_INVALIDATION_REASON;
use crate::server::logic::learning_lifecycle::{
    derive_lifecycle_state as derive_learning_lifecycle_state, LearningLifecycleState,
};
use crate::types::{Memory, MemoryType};

pub const RETENTION_POLICY_VERSION: &str = "memory_gc_v1";
pub const HIGH_IMPORTANCE_PIN_THRESHOLD: f32 = 4.0;

const REASON_SUPERSEDED: &str = "superseded";
const REASON_LEARNING_REJECTED: &str = "learning_rejected";
const REASON_LEARNING_ARCHIVED: &str = "learning_archived";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLifecycleState {
    Active,
    Candidate,
    Rejected,
    Superseded,
    Archived,
    Invalidated,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub capacity_archive_days: i64,
    pub learning_rejected_days: i64,
    pub superseded_days: i64,
    pub learning_archived_days: i64,
    pub unknown_invalidated_days: i64,
    pub high_importance_pin_threshold: f32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            capacity_archive_days: 30,
            learning_rejected_days: 30,
            superseded_days: 90,
            learning_archived_days: 90,
            unknown_invalidated_days: 90,
            high_importance_pin_threshold: HIGH_IMPORTANCE_PIN_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MemoryLifecycleView {
    pub lifecycle_state: MemoryLifecycleState,
    pub default_search_visible: bool,
    pub purge_eligible: bool,
    pub retention_reason: String,
    pub eligible_after: Option<DateTime<Utc>>,
    pub pinned: bool,
}

impl MemoryLifecycleView {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "lifecycle_state": self.lifecycle_state,
            "default_search_visible": self.default_search_visible,
            "purge_eligible": self.purge_eligible,
            "retention_reason": self.retention_reason,
            "eligible_after": self.eligible_after.map(crate::types::Datetime::from),
            "pinned": self.pinned,
        })
    }
}

pub fn derive_memory_lifecycle(
    memory: &Memory,
    now: DateTime<Utc>,
    policy: &RetentionPolicy,
) -> MemoryLifecycleView {
    let lifecycle_state = derive_state(memory);
    let default_search_visible =
        matches!(lifecycle_state, MemoryLifecycleState::Active) && memory.valid_until.is_none();

    if default_search_visible {
        return MemoryLifecycleView {
            lifecycle_state,
            default_search_visible,
            purge_eligible: false,
            retention_reason: "active_memory_not_purgeable".to_string(),
            eligible_after: None,
            pinned: false,
        };
    }

    if is_pinned(memory, policy) {
        return MemoryLifecycleView {
            lifecycle_state,
            default_search_visible,
            purge_eligible: false,
            retention_reason: pin_reason(memory, policy),
            eligible_after: None,
            pinned: true,
        };
    }

    let Some(invalidated_at) = memory.valid_until.as_ref().map(datetime_to_utc) else {
        return MemoryLifecycleView {
            lifecycle_state,
            default_search_visible,
            purge_eligible: false,
            retention_reason: "missing_valid_until".to_string(),
            eligible_after: None,
            pinned: false,
        };
    };

    let retention_days = retention_days(memory, policy);
    let eligible_after = invalidated_at + Duration::days(retention_days);
    let purge_eligible = now >= eligible_after;

    MemoryLifecycleView {
        lifecycle_state,
        default_search_visible,
        purge_eligible,
        retention_reason: format!("retention_{}_days", retention_days),
        eligible_after: Some(eligible_after),
        pinned: false,
    }
}

pub fn retention_days(memory: &Memory, policy: &RetentionPolicy) -> i64 {
    match memory.invalidation_reason.as_deref() {
        Some(CAPACITY_CONTROLLER_INVALIDATION_REASON) => policy.capacity_archive_days,
        Some(REASON_LEARNING_REJECTED) => policy.learning_rejected_days,
        Some(REASON_SUPERSEDED) => policy.superseded_days,
        Some(REASON_LEARNING_ARCHIVED) => policy.learning_archived_days,
        Some(_) | None => policy.unknown_invalidated_days,
    }
}

pub fn protected_prefix(content: &str) -> Option<&'static str> {
    let trimmed = content.trim_start();
    ["DECISION:", "PROJECT:", "USER:"]
        .into_iter()
        .find(|prefix| trimmed.starts_with(prefix))
}

pub fn is_pinned(memory: &Memory, policy: &RetentionPolicy) -> bool {
    protected_prefix(&memory.content).is_some()
        || memory.importance_score >= policy.high_importance_pin_threshold
}

fn pin_reason(memory: &Memory, policy: &RetentionPolicy) -> String {
    if let Some(prefix) = protected_prefix(&memory.content) {
        return format!(
            "pinned_prefix_{}",
            prefix.trim_end_matches(':').to_lowercase()
        );
    }
    if memory.importance_score >= policy.high_importance_pin_threshold {
        return "pinned_high_importance".to_string();
    }
    "pinned".to_string()
}

fn derive_state(memory: &Memory) -> MemoryLifecycleState {
    if memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("learning"))
        .is_some()
    {
        return match derive_learning_lifecycle_state(memory) {
            LearningLifecycleState::Active => MemoryLifecycleState::Active,
            LearningLifecycleState::Candidate => MemoryLifecycleState::Candidate,
            LearningLifecycleState::Rejected => MemoryLifecycleState::Rejected,
            LearningLifecycleState::Superseded => MemoryLifecycleState::Superseded,
            LearningLifecycleState::Archived => MemoryLifecycleState::Archived,
            LearningLifecycleState::Invalidated => MemoryLifecycleState::Invalidated,
            LearningLifecycleState::Unknown => MemoryLifecycleState::Unknown,
        };
    }

    if memory.valid_until.is_some() {
        return match (
            memory.invalidation_reason.as_deref(),
            memory.superseded_by.as_ref(),
        ) {
            (Some(REASON_SUPERSEDED), _) | (_, Some(_)) => MemoryLifecycleState::Superseded,
            (Some(REASON_LEARNING_REJECTED), _) => MemoryLifecycleState::Rejected,
            (Some(REASON_LEARNING_ARCHIVED), _) => MemoryLifecycleState::Archived,
            _ => MemoryLifecycleState::Invalidated,
        };
    }

    match memory.memory_type {
        MemoryType::Episodic | MemoryType::Semantic | MemoryType::Procedural => {
            MemoryLifecycleState::Active
        }
    }
}

pub fn datetime_to_utc(datetime: &crate::types::Datetime) -> DateTime<Utc> {
    let system_time: std::time::SystemTime = (*datetime.clone()).into();
    DateTime::<Utc>::from(system_time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::types::Datetime;

    fn invalidated_memory(reason: &str, invalidated_at: DateTime<Utc>) -> Memory {
        Memory {
            content: "old memory".to_string(),
            valid_until: Some(Datetime::from(invalidated_at)),
            invalidation_reason: Some(reason.to_string()),
            ..Memory::new("old memory".to_string())
        }
    }

    #[test]
    fn active_memory_is_visible_and_not_purgeable() {
        let memory = Memory::new("active".to_string());
        let view = derive_memory_lifecycle(&memory, Utc::now(), &RetentionPolicy::default());
        assert_eq!(view.lifecycle_state, MemoryLifecycleState::Active);
        assert!(view.default_search_visible);
        assert!(!view.purge_eligible);
    }

    #[test]
    fn learning_candidate_is_not_default_search_visible() {
        let mut memory = Memory::new("candidate".to_string());
        memory.metadata = Some(json!({
            "learning": {
                "schema_version": 1,
                "kind": "project_lesson",
                "status": "candidate"
            }
        }));
        let view = derive_memory_lifecycle(&memory, Utc::now(), &RetentionPolicy::default());
        assert_eq!(view.lifecycle_state, MemoryLifecycleState::Candidate);
        assert!(!view.default_search_visible);
        assert!(!view.purge_eligible);
    }

    #[test]
    fn capacity_archive_is_eligible_after_30_days() {
        let now = Utc::now();
        let memory = invalidated_memory(
            CAPACITY_CONTROLLER_INVALIDATION_REASON,
            now - Duration::days(31),
        );
        let view = derive_memory_lifecycle(&memory, now, &RetentionPolicy::default());
        assert!(view.purge_eligible);
        assert_eq!(view.retention_reason, "retention_30_days");
    }

    #[test]
    fn superseded_is_not_eligible_before_90_days() {
        let now = Utc::now();
        let mut memory = invalidated_memory(REASON_SUPERSEDED, now - Duration::days(89));
        memory.superseded_by = Some("new".to_string());
        let view = derive_memory_lifecycle(&memory, now, &RetentionPolicy::default());
        assert_eq!(view.lifecycle_state, MemoryLifecycleState::Superseded);
        assert!(!view.purge_eligible);
    }

    #[test]
    fn protected_prefix_is_pinned() {
        let now = Utc::now();
        let mut memory = invalidated_memory(
            CAPACITY_CONTROLLER_INVALIDATION_REASON,
            now - Duration::days(365),
        );
        memory.content = "DECISION: keep this".to_string();
        let view = derive_memory_lifecycle(&memory, now, &RetentionPolicy::default());
        assert!(view.pinned);
        assert!(!view.purge_eligible);
        assert_eq!(view.retention_reason, "pinned_prefix_decision");
    }

    #[test]
    fn high_importance_is_pinned() {
        let now = Utc::now();
        let mut memory = invalidated_memory(
            CAPACITY_CONTROLLER_INVALIDATION_REASON,
            now - Duration::days(365),
        );
        memory.importance_score = 4.0;
        let view = derive_memory_lifecycle(&memory, now, &RetentionPolicy::default());
        assert!(view.pinned);
        assert!(!view.purge_eligible);
        assert_eq!(view.retention_reason, "pinned_high_importance");
    }
}
