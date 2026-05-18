//! Learning lifecycle mapping helpers.
//!
//! Maps `metadata.learning.status` to Memory lifecycle fields and derives
//! the canonical `LearningLifecycleState` from a `Memory` record.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime;

use crate::types::{learning::LearningStatus, Memory};

// ─── Stable invalidation reason constants ────────────────────────────────────

const REASON_REJECTED: &str = "learning_rejected";
const REASON_ARCHIVED: &str = "learning_archived";
const REASON_SUPERSEDED: &str = "superseded";

/// Canonical lifecycle state derived from Memory lifecycle fields + learning metadata.
///
/// Serialised as snake_case to match the JSON API surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningLifecycleState {
    /// Active, confirmed learning (status = confirmed or rule).
    Active,
    /// Candidate — not yet confirmed, excluded from default search.
    Candidate,
    /// Explicitly rejected.
    Rejected,
    /// Replaced by a newer entry (`superseded_by` is set).
    Superseded,
    /// Archived for historical reference.
    Archived,
    /// Invalidated for an unknown or unrecognised reason.
    Invalidated,
    /// Inconsistent state (e.g. status = superseded but no `superseded_by`).
    Unknown,
}

// ─── Derive ──────────────────────────────────────────────────────────────────

/// Derive the canonical `LearningLifecycleState` from a `Memory` record.
///
/// Memory lifecycle fields are the **canonical truth**; `metadata.learning.status`
/// is used as a semantic hint only when the record is still active.
///
/// Decision table:
/// 1. `valid_until` is set → record is invalidated; inspect `invalidation_reason`
///    and `superseded_by` to pick the specific variant.
/// 2. Record is active → read `metadata.learning.status` to distinguish
///    `Active` (confirmed / rule) from `Candidate`.
/// 3. If status is `superseded` / `rejected` / `archived` but the record is
///    still active → `Unknown` (inconsistent state).
pub fn derive_lifecycle_state(memory: &Memory) -> LearningLifecycleState {
    if memory.valid_until.is_some() {
        // Record has been invalidated — use reason + superseded_by to classify.
        return match memory.invalidation_reason.as_deref() {
            Some(REASON_REJECTED) => LearningLifecycleState::Rejected,
            Some(REASON_ARCHIVED) => LearningLifecycleState::Archived,
            Some(REASON_SUPERSEDED) => LearningLifecycleState::Superseded,
            _ => {
                // Unknown reason, but superseded_by present → treat as superseded.
                if memory.superseded_by.is_some() {
                    LearningLifecycleState::Superseded
                } else {
                    LearningLifecycleState::Invalidated
                }
            }
        };
    }

    // Record is active — derive from metadata.learning.status.
    let status = extract_learning_status(memory);

    match status {
        Some(LearningStatus::Confirmed) | Some(LearningStatus::Rule) => {
            LearningLifecycleState::Active
        }
        Some(LearningStatus::Candidate) | None => LearningLifecycleState::Candidate,
        // Active record with a terminal status → inconsistent.
        Some(LearningStatus::Rejected)
        | Some(LearningStatus::Archived)
        | Some(LearningStatus::Superseded) => LearningLifecycleState::Unknown,
    }
}

// ─── Apply ───────────────────────────────────────────────────────────────────

/// Apply a `LearningStatus` to the Memory lifecycle fields.
///
/// For terminal statuses (`rejected`, `archived`, `superseded`) this sets
/// `valid_until = now` and the appropriate `invalidation_reason`.
/// For active statuses (`candidate`, `confirmed`, `rule`) the lifecycle fields
/// are cleared (record remains active).
///
/// # Errors
/// Returns `Err` if `status = Superseded` and `replacement_id` is `None`.
pub fn apply_status_to_lifecycle(
    memory: &mut Memory,
    status: LearningStatus,
    replacement_id: Option<String>,
) -> Result<(), String> {
    match status {
        LearningStatus::Rejected => {
            let now = Datetime::from(Utc::now());
            memory.valid_until = Some(now);
            memory.invalidation_reason = Some(REASON_REJECTED.to_string());
            memory.superseded_by = None;
        }
        LearningStatus::Archived => {
            let now = Datetime::from(Utc::now());
            memory.valid_until = Some(now);
            memory.invalidation_reason = Some(REASON_ARCHIVED.to_string());
            memory.superseded_by = None;
        }
        LearningStatus::Superseded => {
            let id = replacement_id.ok_or_else(|| {
                "apply_status_to_lifecycle: replacement_id is required for status=superseded"
                    .to_string()
            })?;
            let now = Datetime::from(Utc::now());
            memory.valid_until = Some(now);
            memory.invalidation_reason = Some(REASON_SUPERSEDED.to_string());
            memory.superseded_by = Some(id);
        }
        LearningStatus::Candidate | LearningStatus::Confirmed | LearningStatus::Rule => {
            // Keep record active.
            memory.valid_until = None;
            memory.invalidation_reason = None;
            memory.superseded_by = None;
        }
    }
    Ok(())
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Extract `LearningStatus` from `memory.metadata.learning.status`, if present.
fn extract_learning_status(memory: &Memory) -> Option<LearningStatus> {
    let meta = memory.metadata.as_ref()?;
    let status_val = meta.get("learning")?.get("status")?;
    serde_json::from_value(status_val.clone()).ok()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_memory() -> Memory {
        Memory {
            valid_until: None,
            invalidation_reason: None,
            superseded_by: None,
            metadata: None,
            ..Memory::default()
        }
    }

    fn memory_with_status(status: &str) -> Memory {
        Memory {
            metadata: Some(json!({ "learning": { "status": status } })),
            ..base_memory()
        }
    }

    // ── derive_lifecycle_state ────────────────────────────────────────────────

    #[test]
    fn derive_confirmed_is_active() {
        let m = memory_with_status("confirmed");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Active);
    }

    #[test]
    fn derive_rule_is_active() {
        let m = memory_with_status("rule");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Active);
    }

    #[test]
    fn derive_candidate_is_candidate() {
        let m = memory_with_status("candidate");
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Candidate
        );
    }

    #[test]
    fn derive_no_metadata_is_candidate() {
        let m = base_memory();
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Candidate
        );
    }

    #[test]
    fn derive_invalidated_rejected_reason() {
        let mut m = base_memory();
        m.valid_until = Some(Datetime::from(Utc::now()));
        m.invalidation_reason = Some("learning_rejected".to_string());
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Rejected);
    }

    #[test]
    fn derive_invalidated_archived_reason() {
        let mut m = base_memory();
        m.valid_until = Some(Datetime::from(Utc::now()));
        m.invalidation_reason = Some("learning_archived".to_string());
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Archived);
    }

    #[test]
    fn derive_invalidated_superseded_reason() {
        let mut m = base_memory();
        m.valid_until = Some(Datetime::from(Utc::now()));
        m.invalidation_reason = Some("superseded".to_string());
        m.superseded_by = Some("mem:other".to_string());
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Superseded
        );
    }

    #[test]
    fn derive_unknown_reason_with_superseded_by_is_superseded() {
        let mut m = base_memory();
        m.valid_until = Some(Datetime::from(Utc::now()));
        m.invalidation_reason = Some("some_unknown_reason".to_string());
        m.superseded_by = Some("mem:other".to_string());
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Superseded
        );
    }

    #[test]
    fn derive_unknown_reason_without_superseded_by_is_invalidated() {
        let mut m = base_memory();
        m.valid_until = Some(Datetime::from(Utc::now()));
        m.invalidation_reason = Some("some_unknown_reason".to_string());
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Invalidated
        );
    }

    #[test]
    fn derive_active_record_with_superseded_status_no_superseded_by_is_unknown() {
        // Active record (valid_until=None) but status=superseded → Unknown
        let m = memory_with_status("superseded");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Unknown);
    }

    #[test]
    fn derive_active_record_with_rejected_status_is_unknown() {
        let m = memory_with_status("rejected");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Unknown);
    }

    #[test]
    fn derive_active_record_with_archived_status_is_unknown() {
        let m = memory_with_status("archived");
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Unknown);
    }

    // ── apply_status_to_lifecycle ─────────────────────────────────────────────

    #[test]
    fn apply_rejected_sets_lifecycle_fields() {
        let mut m = base_memory();
        apply_status_to_lifecycle(&mut m, LearningStatus::Rejected, None).unwrap();
        assert!(m.valid_until.is_some());
        assert_eq!(m.invalidation_reason.as_deref(), Some("learning_rejected"));
        assert!(m.superseded_by.is_none());
    }

    #[test]
    fn apply_archived_sets_lifecycle_fields() {
        let mut m = base_memory();
        apply_status_to_lifecycle(&mut m, LearningStatus::Archived, None).unwrap();
        assert!(m.valid_until.is_some());
        assert_eq!(m.invalidation_reason.as_deref(), Some("learning_archived"));
        assert!(m.superseded_by.is_none());
    }

    #[test]
    fn apply_superseded_sets_lifecycle_fields() {
        let mut m = base_memory();
        apply_status_to_lifecycle(
            &mut m,
            LearningStatus::Superseded,
            Some("mem:new".to_string()),
        )
        .unwrap();
        assert!(m.valid_until.is_some());
        assert_eq!(m.invalidation_reason.as_deref(), Some("superseded"));
        assert_eq!(m.superseded_by.as_deref(), Some("mem:new"));
    }

    #[test]
    fn apply_superseded_without_replacement_id_errors() {
        let mut m = base_memory();
        let result = apply_status_to_lifecycle(&mut m, LearningStatus::Superseded, None);
        assert!(result.is_err());
    }

    #[test]
    fn apply_candidate_clears_lifecycle_fields() {
        let mut m = base_memory();
        m.valid_until = Some(Datetime::from(Utc::now()));
        m.invalidation_reason = Some("old".to_string());
        apply_status_to_lifecycle(&mut m, LearningStatus::Candidate, None).unwrap();
        assert!(m.valid_until.is_none());
        assert!(m.invalidation_reason.is_none());
        assert!(m.superseded_by.is_none());
    }

    #[test]
    fn apply_confirmed_clears_lifecycle_fields() {
        let mut m = base_memory();
        m.valid_until = Some(Datetime::from(Utc::now()));
        apply_status_to_lifecycle(&mut m, LearningStatus::Confirmed, None).unwrap();
        assert!(m.valid_until.is_none());
    }

    #[test]
    fn apply_rule_clears_lifecycle_fields() {
        let mut m = base_memory();
        m.valid_until = Some(Datetime::from(Utc::now()));
        apply_status_to_lifecycle(&mut m, LearningStatus::Rule, None).unwrap();
        assert!(m.valid_until.is_none());
    }

    // ── round-trip: apply then derive ─────────────────────────────────────────

    #[test]
    fn roundtrip_rejected() {
        let mut m = memory_with_status("rejected");
        apply_status_to_lifecycle(&mut m, LearningStatus::Rejected, None).unwrap();
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Rejected);
    }

    #[test]
    fn roundtrip_archived() {
        let mut m = memory_with_status("archived");
        apply_status_to_lifecycle(&mut m, LearningStatus::Archived, None).unwrap();
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Archived);
    }

    #[test]
    fn roundtrip_superseded() {
        let mut m = memory_with_status("superseded");
        apply_status_to_lifecycle(
            &mut m,
            LearningStatus::Superseded,
            Some("mem:replacement".to_string()),
        )
        .unwrap();
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Superseded
        );
    }

    #[test]
    fn roundtrip_confirmed_is_active() {
        let mut m = memory_with_status("confirmed");
        apply_status_to_lifecycle(&mut m, LearningStatus::Confirmed, None).unwrap();
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Active);
    }

    #[test]
    fn roundtrip_rule_is_active() {
        let mut m = memory_with_status("rule");
        apply_status_to_lifecycle(&mut m, LearningStatus::Rule, None).unwrap();
        assert_eq!(derive_lifecycle_state(&m), LearningLifecycleState::Active);
    }

    #[test]
    fn roundtrip_candidate() {
        let mut m = memory_with_status("candidate");
        apply_status_to_lifecycle(&mut m, LearningStatus::Candidate, None).unwrap();
        assert_eq!(
            derive_lifecycle_state(&m),
            LearningLifecycleState::Candidate
        );
    }
}
