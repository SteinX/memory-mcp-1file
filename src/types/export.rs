use serde::{Deserialize, Serialize};

use super::{CodeSymbol, Datetime, Entity, IndexStatus, Relation, SymbolRelation};

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
    pub fn from_relation(relation: &Relation, id: Option<String>, from_id: String, to_id: String) -> Self {
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
