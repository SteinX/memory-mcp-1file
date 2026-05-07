use serde::{Deserialize, Serialize};

use super::{CodeSymbol, Datetime, Entity, IndexStatus, MemoryType, Relation, SymbolRelation};

pub const MEMORY_MIGRATION_SCHEMA_VERSION: u32 = 1;
pub const MEMORY_MIGRATION_RECORD_TYPE: &str = "memory";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityPolicy {
    pub mode: String,
    pub clients_must_ignore_unknown_fields: bool,
    pub clients_must_ignore_unknown_enum_values: bool,
    pub db_shape_is_not_public_contract: bool,
}

impl Default for CompatibilityPolicy {
    fn default() -> Self {
        Self {
            mode: "additive_first".to_string(),
            clients_must_ignore_unknown_fields: true,
            clients_must_ignore_unknown_enum_values: true,
            db_shape_is_not_public_contract: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceGuidance {
    pub preferred_response_fields: Vec<String>,
    pub legacy_compatibility_fields: Vec<String>,
    pub forbidden_to_depend_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationRecordType {
    Memory,
}

impl Default for MigrationRecordType {
    fn default() -> Self {
        Self::Memory
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportConflictStrategy {
    Remap,
    Skip,
    Fail,
}

impl Default for ImportConflictStrategy {
    fn default() -> Self {
        Self::Remap
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportErrorCode {
    UnsupportedSchemaVersion,
    InvalidJsonl,
    InvalidRecordType,
    MissingRequiredField,
    InvalidMemoryType,
    StorageError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationMemoryRecord {
    pub schema_version: u32,
    pub record_type: MigrationRecordType,
    pub id: String,
    pub content: String,
    pub memory_type: MemoryType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub importance_score: f32,
    pub created_at: Datetime,
    pub updated_at: Datetime,
    pub valid_from: Datetime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<Datetime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    pub invalidated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalidation_reason: Option<String>,
}

impl MigrationMemoryRecord {
    pub fn unsupported_schema_version_error(
        &self,
        line_number: Option<usize>,
    ) -> Option<ImportError> {
        if self.schema_version == MEMORY_MIGRATION_SCHEMA_VERSION {
            return None;
        }

        Some(ImportError {
            code: ImportErrorCode::UnsupportedSchemaVersion,
            message: format!(
                "Unsupported memory migration schema_version {}. Supported schema_version is {}.",
                self.schema_version, MEMORY_MIGRATION_SCHEMA_VERSION
            ),
            line_number,
            source_id: Some(self.id.clone()),
            field: Some("schema_version".to_string()),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MigrationSummary {
    pub schema_version: u32,
    pub record_type: MigrationRecordType,
    pub total_records: usize,
    pub memory_records: usize,
    pub exported_records: usize,
    pub imported_records: usize,
    pub skipped_records: usize,
    pub failed_records: usize,
    pub valid_records: usize,
    pub invalidated_records: usize,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportMemoryResponse {
    pub schema_version: u32,
    pub record_type: MigrationRecordType,
    pub jsonl: String,
    pub exported_count: usize,
    pub truncated: bool,
    pub summary: MigrationSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportIdMapping {
    pub old_id: String,
    pub new_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportError {
    pub code: ImportErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_number: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportMemoryResponse {
    pub schema_version: u32,
    pub record_type: MigrationRecordType,
    pub conflict_strategy: ImportConflictStrategy,
    pub dry_run: bool,
    pub imported_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub summary: MigrationSummary,
    pub id_mappings: Vec<ImportIdMapping>,
    pub errors: Vec<ImportError>,
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    fn migration_record() -> MigrationMemoryRecord {
        let now = Datetime::default();
        MigrationMemoryRecord {
            schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
            record_type: MigrationRecordType::Memory,
            id: "memory-1".to_string(),
            content: "portable memory content".to_string(),
            memory_type: MemoryType::Semantic,
            user_id: Some("user-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            run_id: Some("run-1".to_string()),
            namespace: Some("namespace-1".to_string()),
            project_id: Some("project-1".to_string()),
            metadata: Some(serde_json::json!({ "source": "unit-test" })),
            importance_score: 2.5,
            created_at: now,
            updated_at: now,
            valid_from: now,
            valid_until: None,
            superseded_by: Some("memory-2".to_string()),
            invalidated: true,
            invalidation_reason: Some("superseded".to_string()),
        }
    }

    #[test]
    fn export_memory_schema_serializes_jsonl_line() {
        let record = migration_record();
        let line = serde_json::to_string(&record).unwrap();
        assert!(!line.contains('\n'));

        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["schema_version"], MEMORY_MIGRATION_SCHEMA_VERSION);
        assert_eq!(value["record_type"], MEMORY_MIGRATION_RECORD_TYPE);
        assert_eq!(value["id"], "memory-1");
        assert_eq!(value["content"], "portable memory content");
        assert_eq!(value["memory_type"], "semantic");
        assert_eq!(value["user_id"], "user-1");
        assert_eq!(value["agent_id"], "agent-1");
        assert_eq!(value["run_id"], "run-1");
        assert_eq!(value["namespace"], "namespace-1");
        assert_eq!(value["project_id"], "project-1");
        assert_eq!(value["metadata"]["source"], "unit-test");
        assert_eq!(value["importance_score"], 2.5);
        assert!(value.get("created_at").is_some());
        assert!(value.get("updated_at").is_some());
        assert!(value.get("valid_from").is_some());
        assert!(value.get("valid_until").is_none());
        assert_eq!(value["superseded_by"], "memory-2");
        assert_eq!(value["invalidated"], true);
        assert_eq!(value["invalidation_reason"], "superseded");
        assert!(value.get("embedding").is_none());
        assert!(value.get("embeddings").is_none());
        assert!(value.get("vector").is_none());
        assert!(value.get("vectors").is_none());
        assert!(value.get("embedding_state").is_none());
    }

    #[test]
    fn import_memory_rejects_unsupported_schema_version() {
        let mut record = migration_record();
        record.schema_version = 2;

        let error = record.unsupported_schema_version_error(Some(7)).unwrap();
        assert_eq!(error.code, ImportErrorCode::UnsupportedSchemaVersion);
        assert_eq!(error.line_number, Some(7));
        assert_eq!(error.source_id.as_deref(), Some("memory-1"));
        assert_eq!(error.field.as_deref(), Some("schema_version"));
        assert!(error
            .message
            .contains("Unsupported memory migration schema_version 2"));
        assert!(error.message.contains("Supported schema_version is 1"));

        let response = ImportMemoryResponse {
            schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
            record_type: MigrationRecordType::Memory,
            conflict_strategy: ImportConflictStrategy::Remap,
            dry_run: true,
            imported_count: 0,
            skipped_count: 0,
            failed_count: 1,
            summary: MigrationSummary {
                schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
                record_type: MigrationRecordType::Memory,
                total_records: 1,
                memory_records: 1,
                exported_records: 0,
                imported_records: 0,
                skipped_records: 0,
                failed_records: 1,
                valid_records: 0,
                invalidated_records: 0,
                dry_run: true,
            },
            id_mappings: Vec::new(),
            errors: vec![error],
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["errors"][0]["code"], "unsupported_schema_version");
        assert_eq!(value["conflict_strategy"], "remap");
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["imported_count"], 0);
        assert_eq!(value["skipped_count"], 0);
        assert_eq!(value["failed_count"], 1);
        assert!(value["id_mappings"].as_array().unwrap().is_empty());
        assert_eq!(value["summary"]["failed_records"], 1);
    }

    #[test]
    fn export_memory_response_serializes_top_level_report_fields() {
        let record_line = serde_json::to_string(&migration_record()).unwrap();
        let response = ExportMemoryResponse {
            schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
            record_type: MigrationRecordType::Memory,
            jsonl: record_line,
            exported_count: 1,
            truncated: false,
            summary: MigrationSummary {
                schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
                record_type: MigrationRecordType::Memory,
                total_records: 1,
                memory_records: 1,
                exported_records: 1,
                imported_records: 0,
                skipped_records: 0,
                failed_records: 0,
                valid_records: 0,
                invalidated_records: 1,
                dry_run: false,
            },
        };

        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["schema_version"], MEMORY_MIGRATION_SCHEMA_VERSION);
        assert_eq!(value["record_type"], MEMORY_MIGRATION_RECORD_TYPE);
        assert_eq!(value["exported_count"], 1);
        assert_eq!(value["truncated"], false);
        assert!(value["jsonl"]
            .as_str()
            .unwrap()
            .contains("portable memory content"));
        assert_eq!(value["summary"]["exported_records"], 1);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraversalDefaults {
    pub default_relation_scope: String,
    pub relation_metadata_exposed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontier_semantics: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontier_items_identity_basis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontier_items_are_stable_node_ids: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontier_items_are_project_scoped: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontier_is_cursor: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionContractState {
    Missing,
    Stale,
    Current,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractReasonCode {
    Missing,
    Stale,
    Partial,
    Degraded,
    InvalidLocator,
    GenerationMismatch,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionLocatorLookupState {
    Created,
    Resolved,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralLifecycleView {
    pub state: String,
    pub is_ready: bool,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticLifecycleView {
    pub state: String,
    pub is_ready: bool,
    pub generation: u64,
    pub is_caught_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionLifecycleView {
    pub state: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleView {
    pub structural: StructuralLifecycleView,
    pub semantic: SemanticLifecycleView,
    pub projection: ProjectionLifecycleView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationBasis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<LifecycleView>,
    pub structural_generation: u64,
    pub semantic_generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionEnvelope {
    pub state: ProjectionContractState,
    pub basis: String,
    pub generation: u64,
    pub materialization: ProjectionMaterializationEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionMaterializationEnvelope {
    pub strategy: String,
    pub identity_basis: String,
    pub refresh_basis: String,
    pub persistence: String,
    pub persistence_semantics: String,
    pub shape_version: u32,
    pub shape_version_semantics: String,
    pub is_addressable: bool,
    pub addressability_semantics: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    pub locator_semantics: String,
    pub locator_stability: String,
    pub locator_scope: String,
    pub locator_is_opaque: bool,
    pub locator_can_be_persisted_by_clients: bool,
    pub locator_survives_generation_change: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_materialized_generation: Option<u64>,
    pub current_generation: u64,
    pub consistent_with_projection_state: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionLocatorLifecycle {
    pub scope: String,
    pub same_process_only: bool,
    pub survives_process_restart: bool,
    pub survives_generation_change: bool,
    pub client_persistable: bool,
    pub generation_binding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionLocatorLookup {
    pub state: ProjectionLocatorLookupState,
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<ContractReasonCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionLocatorRecord {
    pub locator: String,
    pub locator_kind: String,
    pub project_id: String,
    pub generation: u64,
    pub request: ProjectProjectionRequest,
    pub lifecycle: ProjectionLocatorLifecycle,
    pub lookup: ProjectionLocatorLookup,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CountSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_files: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbols: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrontierSummary {
    pub count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraversalSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_reached: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferred_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontier: Option<FrontierSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionShapingSummary {
    pub relation_scope_applied: String,
    pub sort_mode_applied: String,
    pub node_selection_basis: String,
    pub edge_selection_basis: String,
    pub output_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PartialSummary {
    pub is_partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<ContractReasonCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResponseSummary {
    pub result_kind: String,
    pub counts: CountSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traversal: Option<TraversalSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial: Option<PartialSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExportIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stable_symbol_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stable_memory_id: Option<String>,
    pub stable_node_ids: bool,
    pub node_ids_are_project_scoped: bool,
    pub stable_edge_ids: bool,
    pub edge_ids_are_local_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id_semantics: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_id_semantics: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportContractMeta {
    pub schema_version: u32,
    pub compatibility: CompatibilityPolicy,
    pub identity: ExportIdentity,
    pub generated_at: Datetime,
    pub generation_basis: GenerationBasis,
    pub projection_state: ProjectionContractState,
    pub projection: ProjectionEnvelope,
    pub surface_guidance: SurfaceGuidance,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traversal_defaults: Option<TraversalDefaults>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedGraphNode {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub entity_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ExportedGraphNode {
    pub fn from_entity(entity: &Entity, id: String) -> Self {
        Self {
            id,
            kind: entity.entity_type.clone(),
            name: entity.name.clone(),
            entity_type: entity.entity_type.clone(),
            description: entity.description.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedGraphEdge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub from_id: String,
    pub to_id: String,
    pub relation_type: String,
    pub relation_class: String,
    pub provenance: String,
    pub confidence_class: String,
    pub freshness_generation: u64,
    pub staleness_state: String,
    pub weight: f32,
}

impl ExportedGraphEdge {
    pub fn from_relation(
        relation: &Relation,
        id: Option<String>,
        from_id: String,
        to_id: String,
    ) -> Self {
        Self {
            id,
            from_id,
            to_id,
            relation_type: relation.relation_type.clone(),
            relation_class: relation.relation_class.to_string(),
            provenance: relation.provenance.to_string(),
            confidence_class: relation.confidence_class.to_string(),
            freshness_generation: relation.freshness_generation,
            staleness_state: relation.staleness_state.to_string(),
            weight: relation.weight,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSymbolNode {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub symbol_type: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl ExportedSymbolNode {
    pub fn from_symbol(symbol: &CodeSymbol, id: String) -> Self {
        Self {
            id,
            kind: symbol.symbol_type.to_string(),
            name: symbol.name.clone(),
            symbol_type: symbol.symbol_type.to_string(),
            file_path: symbol.file_path.clone(),
            start_line: symbol.start_line,
            end_line: symbol.end_line,
            project_id: symbol.project_id.clone(),
            signature: symbol.signature.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSymbolEdge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub from_id: String,
    pub to_id: String,
    pub relation_type: String,
    pub relation_class: String,
    pub provenance: String,
    pub confidence_class: String,
    pub freshness_generation: u64,
    pub staleness_state: String,
    pub file_path: String,
    pub line_number: u32,
    pub project_id: String,
}

impl ExportedSymbolEdge {
    pub fn from_relation(
        relation: &SymbolRelation,
        id: Option<String>,
        from_id: String,
        to_id: String,
    ) -> Self {
        Self {
            id,
            from_id,
            to_id,
            relation_type: relation.relation_type.to_string(),
            relation_class: relation.relation_class.to_string(),
            provenance: relation.provenance.to_string(),
            confidence_class: relation.confidence_class.to_string(),
            freshness_generation: relation.freshness_generation,
            staleness_state: relation.staleness_state.to_string(),
            file_path: relation.file_path.clone(),
            line_number: relation.line_number,
            project_id: relation.project_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedProjectProjection {
    pub project_id: String,
    pub request: ProjectProjectionRequest,
    pub contract: ExportContractMeta,
    pub summary: ExportResponseSummary,
    pub shaping: ProjectionShapingSummary,
    pub lifecycle: LifecycleView,
    pub counts: CountSummary,
    pub nodes: Vec<ExportedSymbolNode>,
    pub edges: Vec<ExportedSymbolEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectProjectionRequest {
    pub relation_scope: String,
    pub sort_mode: String,
}

#[derive(Debug, Clone)]
pub struct ProjectProjectionInputs {
    pub status: IndexStatus,
    pub total_files: u32,
    pub indexed_files: u32,
    pub total_chunks: u32,
    pub total_symbols: u32,
    pub request: ProjectProjectionRequest,
    pub symbols: Vec<CodeSymbol>,
    pub relations: Vec<SymbolRelation>,
}
