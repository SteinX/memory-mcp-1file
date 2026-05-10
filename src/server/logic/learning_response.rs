use serde::{Deserialize, Serialize};

use crate::server::logic::learning_lifecycle::LearningLifecycleState;
use crate::types::learning::{LearningKind, LearningScope, LearningStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningResponseSummary {
    pub schema_version: u32,
    pub kind: LearningKind,
    pub status: LearningStatus,
    pub scope: LearningScope,
    pub lifecycle_state: LearningLifecycleState,
    pub included_in_default_list: bool,
    pub included_in_default_search: bool,
    pub injectable_by_default: bool,
}

/// Full response envelope for every learning tool.
///
/// `contract` and `summary` are MANDATORY — never omit them.
/// `learning_summary` is additive and does not replace them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningMemoryResponse {
    pub record: serde_json::Value,
    pub learning_summary: LearningResponseSummary,
    pub contract: serde_json::Value,
    pub summary: serde_json::Value,
}

/// Compute `(included_in_default_list, included_in_default_search, injectable_by_default)`.
///
/// Non-active records are always excluded regardless of status.
/// Spec table:
/// | status     | list  | search | inject |
/// |------------|-------|--------|--------|
/// | candidate  | true  | false  | false  |
/// | confirmed  | true  | true   | true   |
/// | rule       | true  | true   | true   |
/// | rejected   | false | false  | false  |
/// | superseded | false | false  | false  |
/// | archived   | false | false  | false  |
pub fn compute_default_inclusion(
    status: &LearningStatus,
    lifecycle_state: &LearningLifecycleState,
) -> (bool, bool, bool) {
    if *lifecycle_state != LearningLifecycleState::Active {
        return (false, false, false);
    }
    match status {
        LearningStatus::Candidate => (true, false, false),
        LearningStatus::Confirmed => (true, true, true),
        LearningStatus::Rule => (true, true, true),
        LearningStatus::Rejected => (false, false, false),
        LearningStatus::Superseded => (false, false, false),
        LearningStatus::Archived => (false, false, false),
    }
}

/// Assemble a [`LearningMemoryResponse`].
///
/// `record` must already have the embedding field stripped.
/// `contract` and `summary` are the standard JSON values from `super::contracts`.
pub fn build_learning_response(
    record: serde_json::Value,
    kind: LearningKind,
    status: LearningStatus,
    scope: LearningScope,
    lifecycle_state: LearningLifecycleState,
    schema_version: u32,
    contract: serde_json::Value,
    summary: serde_json::Value,
) -> LearningMemoryResponse {
    let (included_in_default_list, included_in_default_search, injectable_by_default) =
        compute_default_inclusion(&status, &lifecycle_state);

    LearningMemoryResponse {
        record,
        learning_summary: LearningResponseSummary {
            schema_version,
            kind,
            status,
            scope,
            lifecycle_state,
            included_in_default_list,
            included_in_default_search,
            injectable_by_default,
        },
        contract,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::learning::{LearningKind, LearningScope, LearningStatus, ScopeLevel};
    use serde_json::json;

    fn dummy_scope() -> LearningScope {
        LearningScope {
            level: ScopeLevel::Global,
            project_id: None,
            workspace: None,
            mode: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
        }
    }

    fn dummy_contract() -> serde_json::Value {
        json!({ "schema_version": 1, "compatibility": { "mode": "additive_first" } })
    }

    fn dummy_summary() -> serde_json::Value {
        json!({ "result_kind": "learning_memory", "counts": { "returned": 1 } })
    }

    #[test]
    fn test_candidate_active() {
        let (list, search, inject) =
            compute_default_inclusion(&LearningStatus::Candidate, &LearningLifecycleState::Active);
        assert!(list);
        assert!(!search);
        assert!(!inject);
    }

    #[test]
    fn test_confirmed_active() {
        let (list, search, inject) =
            compute_default_inclusion(&LearningStatus::Confirmed, &LearningLifecycleState::Active);
        assert!(list);
        assert!(search);
        assert!(inject);
    }

    #[test]
    fn test_rule_active() {
        let (list, search, inject) =
            compute_default_inclusion(&LearningStatus::Rule, &LearningLifecycleState::Active);
        assert!(list);
        assert!(search);
        assert!(inject);
    }

    #[test]
    fn test_rejected_active() {
        let (list, search, inject) =
            compute_default_inclusion(&LearningStatus::Rejected, &LearningLifecycleState::Active);
        assert!(!list);
        assert!(!search);
        assert!(!inject);
    }

    #[test]
    fn test_superseded_active() {
        let (list, search, inject) =
            compute_default_inclusion(&LearningStatus::Superseded, &LearningLifecycleState::Active);
        assert!(!list);
        assert!(!search);
        assert!(!inject);
    }

    #[test]
    fn test_archived_active() {
        let (list, search, inject) =
            compute_default_inclusion(&LearningStatus::Archived, &LearningLifecycleState::Active);
        assert!(!list);
        assert!(!search);
        assert!(!inject);
    }

    #[test]
    fn test_confirmed_invalidated_all_false() {
        let (list, search, inject) = compute_default_inclusion(
            &LearningStatus::Confirmed,
            &LearningLifecycleState::Invalidated,
        );
        assert!(!list);
        assert!(!search);
        assert!(!inject);
    }

    #[test]
    fn test_rule_pending_review_all_false() {
        let (list, search, inject) = compute_default_inclusion(
            &LearningStatus::Rule,
            &LearningLifecycleState::Unknown,
        );
        assert!(!list);
        assert!(!search);
        assert!(!inject);
    }

    #[test]
    fn test_response_has_contract_and_summary() {
        let resp = build_learning_response(
            json!({ "id": "abc", "content": "test" }),
            LearningKind::UserPreference,
            LearningStatus::Confirmed,
            dummy_scope(),
            LearningLifecycleState::Active,
            1,
            dummy_contract(),
            dummy_summary(),
        );
        let v = serde_json::to_value(&resp).unwrap();
        assert!(v.get("contract").is_some());
        assert!(v.get("summary").is_some());
        assert!(v.get("learning_summary").is_some());
        assert!(v.get("record").is_some());
    }

    #[test]
    fn test_response_no_embedding_in_record() {
        let record = json!({ "id": "abc", "content": "test" });
        let resp = build_learning_response(
            record,
            LearningKind::ProjectLesson,
            LearningStatus::Confirmed,
            dummy_scope(),
            LearningLifecycleState::Active,
            1,
            dummy_contract(),
            dummy_summary(),
        );
        let v = serde_json::to_value(&resp).unwrap();
        assert!(v["record"].get("embedding").is_none());
    }

    #[test]
    fn test_inclusion_flags_propagated_to_learning_summary() {
        let resp = build_learning_response(
            json!({}),
            LearningKind::WorkflowRule,
            LearningStatus::Candidate,
            dummy_scope(),
            LearningLifecycleState::Active,
            1,
            dummy_contract(),
            dummy_summary(),
        );
        assert!(resp.learning_summary.included_in_default_list);
        assert!(!resp.learning_summary.included_in_default_search);
        assert!(!resp.learning_summary.injectable_by_default);
    }

    #[test]
    fn test_schema_version_preserved() {
        let resp = build_learning_response(
            json!({}),
            LearningKind::ProjectPattern,
            LearningStatus::Rule,
            dummy_scope(),
            LearningLifecycleState::Active,
            1,
            dummy_contract(),
            dummy_summary(),
        );
        assert_eq!(resp.learning_summary.schema_version, 1);
    }
}
