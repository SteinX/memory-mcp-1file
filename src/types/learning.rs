//! `metadata.learning` schema v1 — typed structs, validation, and tests.
//!
//! A `Memory` entry carries learning metadata in its `metadata` field under the key `"learning"`.
//! This module defines the canonical Rust types and a validation function that parses and
//! validates a raw `serde_json::Value` into a typed `LearningMetadata`.

use serde::{Deserialize, Serialize};
use std::fmt;

// ─── Enums ───────────────────────────────────────────────────────────────────

/// The kind of learning captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningKind {
    UserPreference,
    ProjectLesson,
    ProjectPattern,
    ProjectPitfall,
    WorkflowRule,
}

/// Lifecycle status of a learning entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningStatus {
    Candidate,
    Confirmed,
    Rule,
    Rejected,
    Superseded,
    Archived,
}

/// Granularity level of the scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeLevel {
    Global,
    Project,
    Workspace,
    Mode,
    Agent,
    Session,
}

/// How the learning entry was originally created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreatedFrom {
    Manual,
    Plugin,
    Migration,
    Promotion,
    Supersede,
    Consolidation,
}

// ─── Supporting structs ───────────────────────────────────────────────────────

/// Scope of applicability for a learning entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningScope {
    pub level: ScopeLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// Provenance information for a learning entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningSource {
    pub created_from: CreatedFrom,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_memory_ids: Vec<String>,
}

// ─── Root struct ─────────────────────────────────────────────────────────────

/// Typed representation of the `metadata["learning"]` object (schema version 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningMetadata {
    /// Must be `1`.
    pub schema_version: u32,
    pub kind: LearningKind,
    pub status: LearningStatus,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: f64,
    pub scope: LearningScope,
    pub source: LearningSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applies_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_hints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
}

// ─── Validation error ────────────────────────────────────────────────────────

/// A structured validation error that names the offending field.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl ValidationError {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "validation error on field `{}`: {}", self.field, self.message)
    }
}

impl std::error::Error for ValidationError {}

// ─── Validation function ─────────────────────────────────────────────────────

/// Parse and validate a raw JSON value (the contents of `metadata["learning"]`)
/// into a typed [`LearningMetadata`].
///
/// Returns [`ValidationError`] when:
/// - A required field is missing.
/// - An enum field contains an unknown variant.
/// - `confidence` is outside `[0.0, 1.0]`.
/// - `schema_version` is not `1`.
pub fn validate_learning_metadata(value: &serde_json::Value) -> Result<LearningMetadata, ValidationError> {
    // Deserialize into the typed struct first; serde handles unknown enum variants.
    let meta: LearningMetadata = serde_json::from_value(value.clone()).map_err(|e| {
        // Attempt to give a field-specific error by inspecting the message.
        let msg = e.to_string();
        // serde_json error messages often contain the field path.
        let field = extract_field_from_serde_error(&msg);
        ValidationError::new(field, msg)
    })?;

    // Post-deserialization semantic checks.
    if meta.schema_version != 1 {
        return Err(ValidationError::new(
            "schema_version",
            format!("must be 1, got {}", meta.schema_version),
        ));
    }

    if !(0.0..=1.0).contains(&meta.confidence) {
        return Err(ValidationError::new(
            "confidence",
            format!("must be in [0.0, 1.0], got {}", meta.confidence),
        ));
    }

    Ok(meta)
}

/// Best-effort extraction of a field name from a serde_json error message.
fn extract_field_from_serde_error(msg: &str) -> String {
    // serde_json messages look like: "missing field `foo`" or "unknown variant `bar`"
    if let Some(start) = msg.find('`') {
        if let Some(end) = msg[start + 1..].find('`') {
            return msg[start + 1..start + 1 + end].to_string();
        }
    }
    "unknown".to_string()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod learning_schema {
    use super::*;
    use serde_json::json;

    fn valid_base() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "kind": "user_preference",
            "status": "candidate",
            "confidence": 0.8,
            "scope": {
                "level": "project",
                "project_id": "project"
            },
            "source": {
                "created_from": "plugin",
                "client": "opencode-plugin",
                "source_memory_ids": []
            }
        })
    }

    // ── Valid enum variants ──────────────────────────────────────────────────

    #[test]
    fn all_kind_variants() {
        let kinds = [
            ("user_preference", LearningKind::UserPreference),
            ("project_lesson", LearningKind::ProjectLesson),
            ("project_pattern", LearningKind::ProjectPattern),
            ("project_pitfall", LearningKind::ProjectPitfall),
            ("workflow_rule", LearningKind::WorkflowRule),
        ];
        for (raw, expected) in kinds {
            let mut v = valid_base();
            v["kind"] = json!(raw);
            let meta = validate_learning_metadata(&v).unwrap_or_else(|e| panic!("kind={raw}: {e}"));
            assert_eq!(meta.kind, expected, "kind={raw}");
        }
    }

    #[test]
    fn all_status_variants() {
        let statuses = [
            ("candidate", LearningStatus::Candidate),
            ("confirmed", LearningStatus::Confirmed),
            ("rule", LearningStatus::Rule),
            ("rejected", LearningStatus::Rejected),
            ("superseded", LearningStatus::Superseded),
            ("archived", LearningStatus::Archived),
        ];
        for (raw, expected) in statuses {
            let mut v = valid_base();
            v["status"] = json!(raw);
            let meta = validate_learning_metadata(&v).unwrap_or_else(|e| panic!("status={raw}: {e}"));
            assert_eq!(meta.status, expected, "status={raw}");
        }
    }

    #[test]
    fn all_scope_level_variants() {
        let levels = [
            ("global", ScopeLevel::Global),
            ("project", ScopeLevel::Project),
            ("workspace", ScopeLevel::Workspace),
            ("mode", ScopeLevel::Mode),
            ("agent", ScopeLevel::Agent),
            ("session", ScopeLevel::Session),
        ];
        for (raw, expected) in levels {
            let mut v = valid_base();
            v["scope"]["level"] = json!(raw);
            let meta = validate_learning_metadata(&v).unwrap_or_else(|e| panic!("level={raw}: {e}"));
            assert_eq!(meta.scope.level, expected, "level={raw}");
        }
    }

    #[test]
    fn all_created_from_variants() {
        let variants = [
            ("manual", CreatedFrom::Manual),
            ("plugin", CreatedFrom::Plugin),
            ("migration", CreatedFrom::Migration),
            ("promotion", CreatedFrom::Promotion),
            ("supersede", CreatedFrom::Supersede),
            ("consolidation", CreatedFrom::Consolidation),
        ];
        for (raw, expected) in variants {
            let mut v = valid_base();
            v["source"]["created_from"] = json!(raw);
            let meta = validate_learning_metadata(&v).unwrap_or_else(|e| panic!("created_from={raw}: {e}"));
            assert_eq!(meta.source.created_from, expected, "created_from={raw}");
        }
    }

    // ── Missing required fields ──────────────────────────────────────────────

    #[test]
    fn missing_schema_version() {
        let mut v = valid_base();
        v.as_object_mut().unwrap().remove("schema_version");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn missing_kind() {
        let mut v = valid_base();
        v.as_object_mut().unwrap().remove("kind");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn missing_status() {
        let mut v = valid_base();
        v.as_object_mut().unwrap().remove("status");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn missing_confidence() {
        let mut v = valid_base();
        v.as_object_mut().unwrap().remove("confidence");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn missing_scope() {
        let mut v = valid_base();
        v.as_object_mut().unwrap().remove("scope");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn missing_scope_level() {
        let mut v = valid_base();
        v["scope"].as_object_mut().unwrap().remove("level");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn missing_source() {
        let mut v = valid_base();
        v.as_object_mut().unwrap().remove("source");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn missing_source_created_from() {
        let mut v = valid_base();
        v["source"].as_object_mut().unwrap().remove("created_from");
        assert!(validate_learning_metadata(&v).is_err());
    }

    // ── Unknown enum values ──────────────────────────────────────────────────

    #[test]
    fn unknown_kind() {
        let mut v = valid_base();
        v["kind"] = json!("not_a_kind");
        let err = validate_learning_metadata(&v).unwrap_err();
        assert!(err.field.contains("kind") || err.message.contains("not_a_kind"), "{err}");
    }

    #[test]
    fn unknown_status() {
        let mut v = valid_base();
        v["status"] = json!("pending");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn unknown_scope_level() {
        let mut v = valid_base();
        v["scope"]["level"] = json!("team");
        assert!(validate_learning_metadata(&v).is_err());
    }

    #[test]
    fn unknown_created_from() {
        let mut v = valid_base();
        v["source"]["created_from"] = json!("auto");
        assert!(validate_learning_metadata(&v).is_err());
    }

    // ── Confidence range ─────────────────────────────────────────────────────

    #[test]
    fn confidence_zero_is_valid() {
        let mut v = valid_base();
        v["confidence"] = json!(0.0);
        assert!(validate_learning_metadata(&v).is_ok());
    }

    #[test]
    fn confidence_one_is_valid() {
        let mut v = valid_base();
        v["confidence"] = json!(1.0);
        assert!(validate_learning_metadata(&v).is_ok());
    }

    #[test]
    fn confidence_above_one_is_invalid() {
        let mut v = valid_base();
        v["confidence"] = json!(1.1);
        let err = validate_learning_metadata(&v).unwrap_err();
        assert_eq!(err.field, "confidence");
    }

    #[test]
    fn confidence_negative_is_invalid() {
        let mut v = valid_base();
        v["confidence"] = json!(-0.1);
        let err = validate_learning_metadata(&v).unwrap_err();
        assert_eq!(err.field, "confidence");
    }

    // ── Optional fields absent ───────────────────────────────────────────────

    #[test]
    fn optional_fields_absent_is_valid() {
        // evidence, applies_to, trigger_hints, supersedes, constraints all absent
        let v = valid_base();
        let meta = validate_learning_metadata(&v).expect("should be valid without optional fields");
        assert!(meta.evidence.is_empty());
        assert!(meta.applies_to.is_empty());
        assert!(meta.trigger_hints.is_empty());
        assert!(meta.supersedes.is_empty());
        assert!(meta.constraints.is_empty());
    }

    #[test]
    fn optional_fields_present_is_valid() {
        let mut v = valid_base();
        v["evidence"] = json!(["used serde_json for parsing"]);
        v["applies_to"] = json!(["rust", "serde"]);
        v["trigger_hints"] = json!(["when parsing JSON"]);
        v["supersedes"] = json!(["mem:old-id"]);
        v["constraints"] = json!(["must not break existing tests"]);
        let meta = validate_learning_metadata(&v).expect("should be valid with optional fields");
        assert_eq!(meta.evidence.len(), 1);
        assert_eq!(meta.applies_to.len(), 2);
        assert_eq!(meta.trigger_hints.len(), 1);
        assert_eq!(meta.supersedes.len(), 1);
        assert_eq!(meta.constraints.len(), 1);
    }

    // ── Schema version ───────────────────────────────────────────────────────

    #[test]
    fn schema_version_not_one_is_invalid() {
        let mut v = valid_base();
        v["schema_version"] = json!(2);
        let err = validate_learning_metadata(&v).unwrap_err();
        assert_eq!(err.field, "schema_version");
    }

    // ── Scope optional sub-fields ────────────────────────────────────────────

    #[test]
    fn scope_optional_subfields_absent() {
        // Only `level` is required in scope; all others optional
        let mut v = valid_base();
        v["scope"] = json!({ "level": "global" });
        let meta = validate_learning_metadata(&v).expect("scope with only level should be valid");
        assert_eq!(meta.scope.level, ScopeLevel::Global);
        assert!(meta.scope.project_id.is_none());
    }

    // ── Full canonical example ───────────────────────────────────────────────

    #[test]
    fn canonical_example_roundtrip() {
        let v = json!({
            "schema_version": 1,
            "kind": "user_preference",
            "status": "candidate",
            "confidence": 0.8,
            "scope": {
                "level": "project",
                "project_id": "project",
                "workspace": null,
                "mode": null,
                "user_id": null,
                "agent_id": null,
                "run_id": null,
                "namespace": null
            },
            "source": {
                "created_from": "plugin",
                "client": "opencode-plugin",
                "source_memory_ids": []
            },
            "evidence": [],
            "applies_to": [],
            "trigger_hints": [],
            "supersedes": [],
            "constraints": []
        });
        let meta = validate_learning_metadata(&v).expect("canonical example must be valid");
        assert_eq!(meta.schema_version, 1);
        assert_eq!(meta.kind, LearningKind::UserPreference);
        assert_eq!(meta.status, LearningStatus::Candidate);
        assert!((meta.confidence - 0.8).abs() < f64::EPSILON);
        assert_eq!(meta.scope.level, ScopeLevel::Project);
        assert_eq!(meta.source.created_from, CreatedFrom::Plugin);
    }
}
