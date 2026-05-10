//! Learning-aware filter parsing on top of `MemoryQuery`.
//!
//! Provides `LearningFilter` — a typed representation of the filter fields
//! accepted by `learning_memory_search` and `learning_memory_list` — and
//! helpers that translate those filters into `MemoryQuery` metadata conditions.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::types::{search::MemoryQuery, learning::LearningStatus};

// ─── Error ────────────────────────────────────────────────────────────────────

/// Errors produced by filter validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterError {
    /// A destructive operation was requested without an `id` or a scoped filter.
    BroadDestructiveOperation {
        hint: String,
    },
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterError::BroadDestructiveOperation { hint } => {
                write!(f, "Unsafe broad destructive operation: {hint}")
            }
        }
    }
}

impl std::error::Error for FilterError {}

// ─── Fallback controls ────────────────────────────────────────────────────────

/// Controls whether a project/workspace-scoped query may fall back to global
/// records when no project-specific results are found.
///
/// Defaults to `include_global = false` — queries are **never** broadened to
/// global scope unless the caller explicitly opts in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackOptions {
    /// When `true`, allow global-scope learning records to be included in
    /// results even when a project/workspace scope is active.
    #[serde(default)]
    pub include_global: bool,
}

impl Default for FallbackOptions {
    fn default() -> Self {
        Self { include_global: false }
    }
}

// ─── LearningFilter ───────────────────────────────────────────────────────────

/// Typed representation of the filter fields accepted by learning memory tools.
///
/// All fields are optional; callers should use `default_search_filter()` or
/// `default_list_filter()` to obtain sensible defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningFilter {
    /// Statuses to include. `None` means "no status restriction".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_status: Option<Vec<LearningStatus>>,

    /// Statuses to exclude. Applied after `include_status`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_status: Option<Vec<LearningStatus>>,

    /// When `true`, include records that have been logically invalidated
    /// (i.e. `valid_until` is in the past). Defaults to `false`.
    #[serde(default)]
    pub include_invalidated: bool,

    /// Audit mode: when `true`, include all requested statuses including
    /// invalidated records (overrides `include_invalidated`).
    #[serde(default)]
    pub audit: bool,

    /// Fallback scope controls.
    #[serde(default)]
    pub fallback: FallbackOptions,
}

impl Default for LearningFilter {
    fn default() -> Self {
        Self {
            include_status: None,
            exclude_status: None,
            include_invalidated: false,
            audit: false,
            fallback: FallbackOptions::default(),
        }
    }
}

// ─── Default filters ──────────────────────────────────────────────────────────

/// Default filter for `learning_memory_search`.
///
/// Includes `confirmed` and `rule`; excludes `candidate`, `rejected`,
/// `superseded`, and `archived`.
pub fn default_search_filter() -> LearningFilter {
    LearningFilter {
        include_status: Some(vec![LearningStatus::Confirmed, LearningStatus::Rule]),
        exclude_status: Some(vec![
            LearningStatus::Candidate,
            LearningStatus::Rejected,
            LearningStatus::Superseded,
            LearningStatus::Archived,
        ]),
        include_invalidated: false,
        audit: false,
        fallback: FallbackOptions::default(),
    }
}

/// Default filter for `learning_memory_list`.
///
/// Includes `candidate`, `confirmed`, and `rule`; excludes `rejected`,
/// `superseded`, and `archived`.
pub fn default_list_filter() -> LearningFilter {
    LearningFilter {
        include_status: Some(vec![
            LearningStatus::Candidate,
            LearningStatus::Confirmed,
            LearningStatus::Rule,
        ]),
        exclude_status: Some(vec![
            LearningStatus::Rejected,
            LearningStatus::Superseded,
            LearningStatus::Archived,
        ]),
        include_invalidated: false,
        audit: false,
        fallback: FallbackOptions::default(),
    }
}

// ─── Filter application ───────────────────────────────────────────────────────

/// Serialise a `LearningStatus` to its snake_case string representation.
fn status_str(s: &LearningStatus) -> &'static str {
    match s {
        LearningStatus::Candidate  => "candidate",
        LearningStatus::Confirmed  => "confirmed",
        LearningStatus::Rule       => "rule",
        LearningStatus::Rejected   => "rejected",
        LearningStatus::Superseded => "superseded",
        LearningStatus::Archived   => "archived",
    }
}

/// Apply a `LearningFilter` to a `MemoryQuery`.
///
/// The filter is translated into a `metadata_filter` JSON condition that the
/// storage layer evaluates against the `metadata.learning.status` field.
///
/// # Behaviour
/// - `include_status` → `metadata.learning.status IN [...]`
/// - `exclude_status` → `metadata.learning.status NOT IN [...]`
/// - When both are present, `include_status` takes precedence and
///   `exclude_status` is applied as an additional exclusion.
/// - `audit = true` overrides `include_invalidated` and sets it to `true`.
/// - `fallback.include_global` is stored in the filter for callers to inspect;
///   this function does **not** modify scope fields on `MemoryQuery` — scope
///   broadening is the caller's responsibility.
///
/// Any pre-existing `metadata_filter` on the query is merged (AND-combined)
/// with the new conditions.
pub fn apply_learning_filter(query: &mut MemoryQuery, filter: &LearningFilter) {
    let effective_include_invalidated = filter.audit || filter.include_invalidated;

    // Build the learning-status sub-filter.
    let mut conditions: Vec<serde_json::Value> = Vec::new();

    if let Some(include) = &filter.include_status {
        if !include.is_empty() {
            let statuses: Vec<&str> = include.iter().map(status_str).collect();
            conditions.push(json!({
                "field": "metadata.learning.status",
                "op": "in",
                "value": statuses
            }));
        }
    }

    if let Some(exclude) = &filter.exclude_status {
        if !exclude.is_empty() {
            let statuses: Vec<&str> = exclude.iter().map(status_str).collect();
            conditions.push(json!({
                "field": "metadata.learning.status",
                "op": "not_in",
                "value": statuses
            }));
        }
    }

    if !effective_include_invalidated {
        // Restrict to currently valid records only.
        conditions.push(json!({
            "field": "metadata.learning.invalidated",
            "op": "ne",
            "value": true
        }));
    }

    if conditions.is_empty() {
        return;
    }

    let new_filter = if conditions.len() == 1 {
        conditions.remove(0)
    } else {
        json!({ "and": conditions })
    };

    // Merge with any pre-existing metadata_filter.
    query.metadata_filter = Some(match query.metadata_filter.take() {
        None => new_filter,
        Some(existing) => json!({ "and": [existing, new_filter] }),
    });
}

// ─── Destructive filter validation ───────────────────────────────────────────

/// Validate that a destructive operation (reject, archive, supersede, migration
/// apply) is scoped to a specific record or an explicit filter.
///
/// A destructive operation is considered **safe** when at least one of the
/// following is true:
/// - `id` is `Some` (targets a specific record).
/// - The `MemoryQuery` has at least one non-metadata scope field set
///   (`user_id`, `agent_id`, `run_id`, or `namespace`).
/// - The `LearningFilter` has a non-empty `include_status` list (explicit
///   status scope).
///
/// Returns `Err(FilterError::BroadDestructiveOperation)` when none of the
/// above conditions hold.
pub fn validate_destructive_filter(
    id: Option<&str>,
    query: &MemoryQuery,
    filter: &LearningFilter,
) -> Result<(), FilterError> {
    let has_id = id.map(|s| !s.trim().is_empty()).unwrap_or(false);
    let has_scope = query.user_id.is_some()
        || query.agent_id.is_some()
        || query.run_id.is_some()
        || query.namespace.is_some();
    let has_status_filter = filter
        .include_status
        .as_ref()
        .map(|v| !v.is_empty())
        .unwrap_or(false);

    if has_id || has_scope || has_status_filter {
        Ok(())
    } else {
        Err(FilterError::BroadDestructiveOperation {
            hint: "Destructive operations require an `id`, a scope field \
                   (user_id / agent_id / run_id / namespace), or an explicit \
                   `include_status` filter. Provide at least one to proceed."
                .to_string(),
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::search::MemoryQuery;

    // ── default_search_filter ────────────────────────────────────────────────

    #[test]
    fn default_search_filter_includes_confirmed_and_rule() {
        let f = default_search_filter();
        let inc = f.include_status.as_ref().unwrap();
        assert!(inc.contains(&LearningStatus::Confirmed));
        assert!(inc.contains(&LearningStatus::Rule));
        assert!(!inc.contains(&LearningStatus::Candidate));
    }

    #[test]
    fn default_search_filter_excludes_candidate_rejected_superseded_archived() {
        let f = default_search_filter();
        let exc = f.exclude_status.as_ref().unwrap();
        assert!(exc.contains(&LearningStatus::Candidate));
        assert!(exc.contains(&LearningStatus::Rejected));
        assert!(exc.contains(&LearningStatus::Superseded));
        assert!(exc.contains(&LearningStatus::Archived));
    }

    #[test]
    fn default_search_filter_does_not_include_invalidated() {
        let f = default_search_filter();
        assert!(!f.include_invalidated);
        assert!(!f.audit);
    }

    #[test]
    fn default_search_filter_fallback_include_global_is_false() {
        let f = default_search_filter();
        assert!(!f.fallback.include_global);
    }

    // ── default_list_filter ──────────────────────────────────────────────────

    #[test]
    fn default_list_filter_includes_candidate_confirmed_rule() {
        let f = default_list_filter();
        let inc = f.include_status.as_ref().unwrap();
        assert!(inc.contains(&LearningStatus::Candidate));
        assert!(inc.contains(&LearningStatus::Confirmed));
        assert!(inc.contains(&LearningStatus::Rule));
    }

    #[test]
    fn default_list_filter_excludes_rejected_superseded_archived() {
        let f = default_list_filter();
        let exc = f.exclude_status.as_ref().unwrap();
        assert!(exc.contains(&LearningStatus::Rejected));
        assert!(exc.contains(&LearningStatus::Superseded));
        assert!(exc.contains(&LearningStatus::Archived));
        assert!(!exc.contains(&LearningStatus::Candidate));
    }

    // ── apply_learning_filter ────────────────────────────────────────────────

    #[test]
    fn apply_sets_metadata_filter_with_include_status() {
        let mut q = MemoryQuery::default();
        let f = LearningFilter {
            include_status: Some(vec![LearningStatus::Confirmed]),
            exclude_status: None,
            include_invalidated: true, // skip invalidated condition for simplicity
            audit: false,
            fallback: FallbackOptions::default(),
        };
        apply_learning_filter(&mut q, &f);
        let mf = q.metadata_filter.unwrap();
        // Should contain an "in" condition for confirmed
        let s = mf.to_string();
        assert!(s.contains("confirmed"), "expected 'confirmed' in filter: {s}");
        assert!(s.contains("\"op\":\"in\""), "expected op=in in filter: {s}");
    }

    #[test]
    fn apply_excludes_invalidated_by_default() {
        let mut q = MemoryQuery::default();
        let f = LearningFilter {
            include_status: None,
            exclude_status: None,
            include_invalidated: false,
            audit: false,
            fallback: FallbackOptions::default(),
        };
        apply_learning_filter(&mut q, &f);
        let mf = q.metadata_filter.unwrap();
        let s = mf.to_string();
        assert!(s.contains("invalidated"), "expected invalidated guard: {s}");
    }

    #[test]
    fn apply_audit_mode_skips_invalidated_guard() {
        let mut q = MemoryQuery::default();
        let f = LearningFilter {
            include_status: Some(vec![LearningStatus::Archived]),
            exclude_status: None,
            include_invalidated: false,
            audit: true,
            fallback: FallbackOptions::default(),
        };
        apply_learning_filter(&mut q, &f);
        let mf = q.metadata_filter.unwrap();
        let s = mf.to_string();
        // audit=true → no invalidated guard
        assert!(!s.contains("invalidated"), "audit mode should skip invalidated guard: {s}");
    }

    #[test]
    fn apply_merges_with_existing_metadata_filter() {
        let mut q = MemoryQuery::default();
        q.metadata_filter = Some(json!({ "field": "project_id", "op": "eq", "value": "proj-1" }));
        let f = default_search_filter();
        apply_learning_filter(&mut q, &f);
        let mf = q.metadata_filter.unwrap();
        let s = mf.to_string();
        assert!(s.contains("proj-1"), "existing filter should be preserved: {s}");
        assert!(s.contains("confirmed"), "new filter should be merged: {s}");
    }

    #[test]
    fn apply_no_op_when_filter_is_empty_and_include_invalidated_true() {
        let mut q = MemoryQuery::default();
        let f = LearningFilter {
            include_status: None,
            exclude_status: None,
            include_invalidated: true,
            audit: false,
            fallback: FallbackOptions::default(),
        };
        apply_learning_filter(&mut q, &f);
        // No conditions → metadata_filter stays None
        assert!(q.metadata_filter.is_none());
    }

    // ── scoped search excludes unrelated records ──────────────────────────────

    #[test]
    fn scoped_search_does_not_broaden_to_global_by_default() {
        let f = default_search_filter();
        // fallback.include_global must be false — callers must not silently
        // broaden project queries to global scope.
        assert!(!f.fallback.include_global);
    }

    #[test]
    fn explicit_include_global_is_respected() {
        let f = LearningFilter {
            fallback: FallbackOptions { include_global: true },
            ..Default::default()
        };
        assert!(f.fallback.include_global);
    }

    // ── validate_destructive_filter ──────────────────────────────────────────

    #[test]
    fn destructive_with_id_is_allowed() {
        let q = MemoryQuery::default();
        let f = LearningFilter::default();
        assert!(validate_destructive_filter(Some("mem:abc123"), &q, &f).is_ok());
    }

    #[test]
    fn destructive_with_namespace_scope_is_allowed() {
        let mut q = MemoryQuery::default();
        q.namespace = Some("project:my-proj".to_string());
        let f = LearningFilter::default();
        assert!(validate_destructive_filter(None, &q, &f).is_ok());
    }

    #[test]
    fn destructive_with_user_id_scope_is_allowed() {
        let mut q = MemoryQuery::default();
        q.user_id = Some("user-1".to_string());
        let f = LearningFilter::default();
        assert!(validate_destructive_filter(None, &q, &f).is_ok());
    }

    #[test]
    fn destructive_with_status_filter_is_allowed() {
        let q = MemoryQuery::default();
        let f = LearningFilter {
            include_status: Some(vec![LearningStatus::Candidate]),
            ..Default::default()
        };
        assert!(validate_destructive_filter(None, &q, &f).is_ok());
    }

    #[test]
    fn destructive_with_no_scope_is_rejected() {
        let q = MemoryQuery::default();
        let f = LearningFilter::default();
        let result = validate_destructive_filter(None, &q, &f);
        assert!(result.is_err());
        match result.unwrap_err() {
            FilterError::BroadDestructiveOperation { .. } => {}
        }
    }

    #[test]
    fn destructive_with_empty_id_string_is_rejected() {
        let q = MemoryQuery::default();
        let f = LearningFilter::default();
        // Whitespace-only id should be treated as absent.
        let result = validate_destructive_filter(Some("   "), &q, &f);
        assert!(result.is_err());
    }

    #[test]
    fn destructive_with_empty_include_status_is_rejected() {
        let q = MemoryQuery::default();
        let f = LearningFilter {
            include_status: Some(vec![]), // empty — not a real scope
            ..Default::default()
        };
        let result = validate_destructive_filter(None, &q, &f);
        assert!(result.is_err());
    }

    // ── FilterError display ──────────────────────────────────────────────────

    #[test]
    fn filter_error_display_is_human_readable() {
        let e = FilterError::BroadDestructiveOperation {
            hint: "test hint".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains("Unsafe broad destructive operation"));
        assert!(s.contains("test hint"));
    }
}
