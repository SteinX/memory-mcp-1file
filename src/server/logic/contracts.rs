use crate::types::{
    record_key_to_string, CodeSymbol, ContractReasonCode, CountSummary, Entity,
    ExportContractMeta, ExportIdentity, ExportResponseSummary, ExportedGraphEdge,
    ExportedGraphNode, ExportedProjectProjection, ExportedSymbolEdge, ExportedSymbolNode,
    FrontierSummary, GenerationBasis, IndexStatus, LifecycleView, PartialSummary,
    ProjectionContractState, ProjectionEnvelope, ProjectProjectionInputs,
    ProjectProjectionRequest, ProjectionLifecycleView, ProjectionMaterializationEnvelope,
    Relation, SemanticLifecycleView, StructuralLifecycleView, SurfaceGuidance,
    SymbolRelation, TraversalDefaults, TraversalSummary,
};

fn projection_partial_reason_code(status: &IndexStatus) -> Option<ContractReasonCode> {
    match status.projection_state {
        crate::types::ProjectionState::Current => None,
        crate::types::ProjectionState::Stale => Some(ContractReasonCode::Stale),
    }
}

fn projection_partial_reason_slug(status: &IndexStatus) -> Option<String> {
    projection_partial_reason_code(status).map(|code| match code {
        ContractReasonCode::Stale => "projection_stale".to_string(),
        ContractReasonCode::Missing => "projection_missing".to_string(),
        ContractReasonCode::Partial => "projection_partial".to_string(),
        ContractReasonCode::Degraded => "projection_degraded".to_string(),
        ContractReasonCode::InvalidLocator => "projection_invalid_locator".to_string(),
        ContractReasonCode::GenerationMismatch => "projection_generation_mismatch".to_string(),
        ContractReasonCode::Unsupported => "projection_unsupported".to_string(),
    })
}

fn collection_partial_reason_code(is_partial: bool) -> Option<ContractReasonCode> {
    if is_partial {
        Some(ContractReasonCode::Partial)
    } else {
        None
    }
}

fn collection_partial_reason_slug(is_partial: bool) -> Option<String> {
    if is_partial {
        Some("indexing_in_progress".to_string())
    } else {
        None
    }
}

fn index_status_reason_code(is_partial: bool) -> Option<ContractReasonCode> {
    if is_partial {
        Some(ContractReasonCode::Partial)
    } else {
        None
    }
}

fn index_status_reason_slug(is_partial: bool, overall_progress_percent: f32) -> Option<String> {
    if is_partial {
        Some(format!("progress:{overall_progress_percent:.1}"))
    } else {
        None
    }
}

fn lifecycle_view(status: &IndexStatus) -> LifecycleView {
    LifecycleView {
        structural: StructuralLifecycleView {
            state: status.structural_state.to_string(),
            is_ready: status.structural_state == crate::types::StructuralState::Ready,
            generation: status.structural_generation,
        },
        semantic: SemanticLifecycleView {
            state: status.semantic_state.to_string(),
            is_ready: status.semantic_state == crate::types::SemanticState::Ready,
            generation: status.semantic_generation,
            is_caught_up: status.semantic_state == crate::types::SemanticState::Ready
                && status.semantic_generation == status.structural_generation,
        },
        projection: ProjectionLifecycleView {
            state: status.projection_state.to_string(),
            is_current: status.projection_state == crate::types::ProjectionState::Current,
        },
    }
}

fn projection_contract_state(status: Option<&IndexStatus>) -> ProjectionContractState {
    match status {
        None => ProjectionContractState::Missing,
        Some(status) => match status.projection_state {
            crate::types::ProjectionState::Current => ProjectionContractState::Current,
            crate::types::ProjectionState::Stale => ProjectionContractState::Stale,
        },
    }
}

fn projection_generation(status: Option<&IndexStatus>) -> u64 {
    status.map(|s| s.semantic_generation).unwrap_or(0)
}

fn projection_basis_kind() -> &'static str {
    "semantic_generation"
}

fn projection_materialization(status: Option<&IndexStatus>) -> ProjectionMaterializationEnvelope {
    let current_generation = projection_generation(status);
    let state = projection_contract_state(status);

    ProjectionMaterializationEnvelope {
        strategy: "not_materialized".to_string(),
        identity_basis: "project_id + semantic_generation".to_string(),
        refresh_basis: projection_basis_kind().to_string(),
        persistence: "ephemeral_contract_only".to_string(),
        persistence_semantics:
            "contract is exposed on status surfaces only; no persisted projection artifact is promised yet"
                .to_string(),
        shape_version: 1,
        shape_version_semantics: "materialized_projection_payload_shape_version".to_string(),
        is_addressable: false,
        addressability_semantics:
            "no_stable_external_read_target_is_promised_until_materialization_strategy_changes"
                .to_string(),
        locator_kind: None,
        locator: None,
        locator_semantics:
            "absent_when_not_materialized; when present it identifies the externally consumable projection instance"
                .to_string(),
        locator_stability: "not_stable_until_materialization_strategy_changes".to_string(),
        locator_scope: "none_when_not_materialized".to_string(),
        locator_is_opaque: true,
        locator_can_be_persisted_by_clients: false,
        locator_survives_generation_change: false,
        last_materialized_generation: None,
        current_generation,
        consistent_with_projection_state: matches!(
            state,
            ProjectionContractState::Missing
                | ProjectionContractState::Stale
                | ProjectionContractState::Current
        ),
    }
}

pub fn export_contract_meta(
    identity: ExportIdentity,
    status: Option<&IndexStatus>,
) -> ExportContractMeta {
    ExportContractMeta {
        schema_version: 1,
        compatibility: Default::default(),
        identity,
        generated_at: crate::types::Datetime::default(),
        generation_basis: GenerationBasis {
            project_status: status.map(|s| s.status.to_string()),
            lifecycle: status.map(lifecycle_view),
            structural_generation: status.map(|s| s.structural_generation).unwrap_or(0),
            semantic_generation: status.map(|s| s.semantic_generation).unwrap_or(0),
        },
        projection_state: projection_contract_state(status.clone()),
        projection: ProjectionEnvelope {
            state: projection_contract_state(status),
            basis: projection_basis_kind().to_string(),
            generation: projection_generation(status),
            materialization: projection_materialization(status),
        },
        surface_guidance: SurfaceGuidance {
            preferred_response_fields: Vec::new(),
            legacy_compatibility_fields: Vec::new(),
            forbidden_to_depend_fields: Vec::new(),
        },
        traversal_defaults: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContractReasonCode, IndexState, IndexStatus, ProjectionState};

    #[test]
    fn export_contract_meta_without_status_sets_missing_projection_and_zero_generations() {
        let contract = export_contract_meta(ExportIdentity::default(), None);

        assert_eq!(contract.projection_state, ProjectionContractState::Missing);
        assert_eq!(contract.projection.state, ProjectionContractState::Missing);
        assert_eq!(contract.projection.basis, "semantic_generation");
        assert_eq!(contract.projection.generation, 0);
        assert_eq!(contract.projection.materialization.strategy, "not_materialized");
        assert_eq!(contract.projection.materialization.identity_basis, "project_id + semantic_generation");
        assert_eq!(contract.projection.materialization.refresh_basis, "semantic_generation");
        assert_eq!(contract.projection.materialization.persistence_semantics, "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(contract.projection.materialization.is_addressable, false);
        assert_eq!(contract.projection.materialization.shape_version_semantics, "materialized_projection_payload_shape_version");
        assert_eq!(contract.projection.materialization.addressability_semantics, "no_stable_external_read_target_is_promised_until_materialization_strategy_changes");
        assert_eq!(contract.projection.materialization.locator_kind, None);
        assert_eq!(contract.projection.materialization.locator_semantics, "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(contract.projection.materialization.locator_stability, "not_stable_until_materialization_strategy_changes");
        assert_eq!(contract.projection.materialization.locator_scope, "none_when_not_materialized");
        assert_eq!(contract.projection.materialization.locator_is_opaque, true);
        assert_eq!(contract.projection.materialization.locator_can_be_persisted_by_clients, false);
        assert_eq!(contract.projection.materialization.locator_survives_generation_change, false);
        assert_eq!(contract.projection.materialization.current_generation, 0);
        assert_eq!(contract.projection.materialization.last_materialized_generation, None);
        assert_eq!(contract.projection.materialization.consistent_with_projection_state, true);
        assert_eq!(contract.generation_basis.structural_generation, 0);
        assert_eq!(contract.generation_basis.semantic_generation, 0);
        assert!(contract.generation_basis.lifecycle.is_none());
    }

    #[test]
    fn export_contract_meta_with_stale_status_copies_generation_basis_and_lifecycle() {
        let mut status = IndexStatus::new("proj-stale".to_string());
        status.status = IndexState::EmbeddingPending;
        status.mark_structural_generation_advanced();
        status.refresh_lifecycle_states();

        let contract = export_contract_meta(ExportIdentity::default(), Some(&status));

        assert_eq!(contract.projection_state, ProjectionContractState::Stale);
        assert_eq!(contract.projection.state, ProjectionContractState::Stale);
        assert_eq!(contract.projection.generation, status.semantic_generation);
        assert_eq!(contract.projection.materialization.strategy, "not_materialized");
        assert_eq!(contract.projection.materialization.refresh_basis, "semantic_generation");
        assert_eq!(contract.projection.materialization.persistence_semantics, "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(contract.projection.materialization.current_generation, 0);
        assert_eq!(contract.projection.materialization.is_addressable, false);
        assert_eq!(contract.projection.materialization.shape_version_semantics, "materialized_projection_payload_shape_version");
        assert_eq!(contract.projection.materialization.addressability_semantics, "no_stable_external_read_target_is_promised_until_materialization_strategy_changes");
        assert_eq!(contract.projection.materialization.locator_kind, None);
        assert_eq!(contract.projection.materialization.locator_semantics, "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(contract.projection.materialization.locator_stability, "not_stable_until_materialization_strategy_changes");
        assert_eq!(contract.projection.materialization.locator_scope, "none_when_not_materialized");
        assert_eq!(contract.projection.materialization.locator_is_opaque, true);
        assert_eq!(contract.projection.materialization.locator_can_be_persisted_by_clients, false);
        assert_eq!(contract.projection.materialization.locator_survives_generation_change, false);
        assert_eq!(contract.projection.materialization.last_materialized_generation, None);
        assert_eq!(contract.projection.materialization.consistent_with_projection_state, true);
        assert_eq!(contract.generation_basis.structural_generation, 1);
        assert_eq!(contract.generation_basis.semantic_generation, 0);
        assert_eq!(contract.generation_basis.lifecycle.as_ref().unwrap().structural.generation, 1);
        assert_eq!(contract.generation_basis.lifecycle.as_ref().unwrap().semantic.generation, 0);
    }

    #[test]
    fn export_contract_meta_with_current_projection_reports_current() {
        let mut status = IndexStatus::new("proj-current".to_string());
        status.status = IndexState::Completed;
        status.mark_structural_generation_advanced();
        status.mark_semantic_generation_caught_up();
        status.projection_state = ProjectionState::Current;
        status.refresh_lifecycle_states();

        let contract = export_contract_meta(ExportIdentity::default(), Some(&status));

        assert_eq!(contract.projection_state, ProjectionContractState::Current);
        assert_eq!(contract.projection.state, ProjectionContractState::Current);
        assert_eq!(contract.projection.basis, "semantic_generation");
        assert_eq!(contract.projection.generation, 1);
        assert_eq!(contract.projection.materialization.strategy, "not_materialized");
        assert_eq!(contract.projection.materialization.refresh_basis, "semantic_generation");
        assert_eq!(contract.projection.materialization.persistence_semantics, "contract is exposed on status surfaces only; no persisted projection artifact is promised yet");
        assert_eq!(contract.projection.materialization.current_generation, 1);
        assert_eq!(contract.projection.materialization.is_addressable, false);
        assert_eq!(contract.projection.materialization.shape_version_semantics, "materialized_projection_payload_shape_version");
        assert_eq!(contract.projection.materialization.addressability_semantics, "no_stable_external_read_target_is_promised_until_materialization_strategy_changes");
        assert_eq!(contract.projection.materialization.locator_kind, None);
        assert_eq!(contract.projection.materialization.locator_semantics, "absent_when_not_materialized; when present it identifies the externally consumable projection instance");
        assert_eq!(contract.projection.materialization.locator_stability, "not_stable_until_materialization_strategy_changes");
        assert_eq!(contract.projection.materialization.locator_scope, "none_when_not_materialized");
        assert_eq!(contract.projection.materialization.locator_is_opaque, true);
        assert_eq!(contract.projection.materialization.locator_can_be_persisted_by_clients, false);
        assert_eq!(contract.projection.materialization.locator_survives_generation_change, false);
        assert_eq!(contract.projection.materialization.last_materialized_generation, None);
        assert_eq!(contract.projection.materialization.consistent_with_projection_state, true);
        assert_eq!(contract.generation_basis.semantic_generation, 1);
    }

    #[test]
    fn build_project_projection_sets_projection_identity_and_counts() {
        let mut status = IndexStatus::new("proj-builder".to_string());
        status.status = IndexState::Completed;
        status.total_files = 3;
        status.mark_structural_generation_advanced();
        status.mark_semantic_generation_caught_up();
        status.mark_projection_current();
        status.refresh_lifecycle_states();

        let symbols = vec![
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "caller")),
                name: "caller".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/lib.rs".to_string(),
                signature: Some("fn caller()".to_string()),
                start_line: 2,
                end_line: 2,
                embedding: None,
                project_id: "proj-builder".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "target")),
                name: "target".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/lib.rs".to_string(),
                signature: Some("fn target()".to_string()),
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-builder".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
        ];

        let relations = vec![SymbolRelation {
            id: None,
            from_symbol: crate::types::Thing::new("code_symbols", "caller"),
            to_symbol: crate::types::Thing::new("code_symbols", "target"),
            relation_type: crate::types::CodeRelationType::Calls,
            relation_class: crate::types::RelationClass::Observed,
            provenance: crate::types::RelationProvenance::ParserExtracted,
            confidence_class: crate::types::ConfidenceClass::Extracted,
            freshness_generation: 1,
            staleness_state: crate::types::StalenessState::Current,
            file_path: "src/lib.rs".to_string(),
            line_number: 2,
            project_id: "proj-builder".to_string(),
            created_at: crate::types::Datetime::default(),
        }];

        let projection = build_project_projection(
            &status,
            3,
            3,
            5,
            2,
            ProjectProjectionRequest {
                relation_scope: "all".to_string(),
                sort_mode: "canonical".to_string(),
            },
            &symbols,
            &relations,
        );

        assert_eq!(projection.project_id, "proj-builder");
        assert_eq!(projection.contract.schema_version, 1);
        assert_eq!(projection.contract.identity.project_id, Some("proj-builder".to_string()));
        assert_eq!(projection.contract.identity.stable_node_ids, true);
        assert_eq!(projection.contract.identity.node_ids_are_project_scoped, true);
        assert_eq!(projection.contract.identity.node_id_semantics, Some("stable_project_scoped_project_id".to_string()));
        assert_eq!(projection.contract.identity.edge_id_semantics, Some("no_public_edge_ids".to_string()));
        assert_eq!(projection.request.relation_scope, "all");
        assert_eq!(projection.request.sort_mode, "canonical");
        assert_eq!(projection.summary.result_kind, "graph");
        assert_eq!(projection.counts.files, Some(3));
        assert_eq!(projection.counts.indexed_files, Some(3));
        assert_eq!(projection.counts.chunks, Some(5));
        assert_eq!(projection.counts.symbols, Some(2));
        assert_eq!(projection.counts.nodes, Some(2));
        assert_eq!(projection.counts.edges, Some(1));
        assert_eq!(projection.nodes.len(), 2);
        assert_eq!(projection.edges.len(), 1);
    }

    #[test]
    fn build_project_projection_preserves_current_projection_state() {
        let mut status = IndexStatus::new("proj-current-builder".to_string());
        status.status = IndexState::Completed;
        status.mark_structural_generation_advanced();
        status.mark_semantic_generation_caught_up();
        status.mark_projection_current();
        status.refresh_lifecycle_states();

        let projection = build_project_projection(
            &status,
            1,
            1,
            0,
            0,
            ProjectProjectionRequest {
                relation_scope: "all".to_string(),
                sort_mode: "canonical".to_string(),
            },
            &[],
            &[],
        );

        assert_eq!(projection.lifecycle.projection.state, "current");
        assert_eq!(projection.lifecycle.projection.is_current, true);
        assert_eq!(projection.contract.projection.state, ProjectionContractState::Current);
        assert_eq!(projection.contract.projection.generation, 1);
        assert_eq!(projection.contract.projection.materialization.current_generation, 1);
        assert_eq!(projection.contract.projection.materialization.consistent_with_projection_state, true);
        assert_eq!(projection.request.relation_scope, "all");
        assert_eq!(projection.request.sort_mode, "canonical");
        assert_eq!(projection.summary.partial.as_ref().unwrap().is_partial, false);
        assert_eq!(projection.summary.partial.as_ref().unwrap().reason_code, None);
        assert_eq!(projection.summary.partial.as_ref().unwrap().reason, None);
        assert_eq!(
            projection.summary.partial.as_ref().unwrap().message.as_deref(),
            Some("Projection is an on-demand export of the current semantic snapshot; no separately materialized artifact is promised.")
        );
    }

    #[test]
    fn build_project_projection_marks_stale_projection_as_partial() {
        let mut status = IndexStatus::new("proj-stale-builder".to_string());
        status.status = IndexState::EmbeddingPending;
        status.mark_structural_generation_advanced();
        status.refresh_lifecycle_states();

        let projection = build_project_projection(
            &status,
            1,
            1,
            0,
            0,
            ProjectProjectionRequest {
                relation_scope: "all".to_string(),
                sort_mode: "canonical".to_string(),
            },
            &[],
            &[],
        );

        assert_eq!(projection.lifecycle.projection.state, "stale");
        assert_eq!(projection.contract.projection.state, ProjectionContractState::Stale);
        assert_eq!(projection.summary.partial.as_ref().unwrap().is_partial, true);
        assert_eq!(
            projection.summary.partial.as_ref().unwrap().reason_code,
            Some(ContractReasonCode::Stale)
        );
        assert_eq!(
            projection.summary.partial.as_ref().unwrap().reason.as_deref(),
            Some("projection_stale")
        );
        assert_eq!(
            projection.summary.partial.as_ref().unwrap().message.as_deref(),
            Some("Projection is an on-demand export of the latest available semantic snapshot and may lag structural changes.")
        );
    }

    #[test]
    fn build_project_projection_with_imports_scope_filters_non_import_edges() {
        let status = IndexStatus::new("proj-imports".to_string());

        let symbols = vec![
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "caller")),
                name: "caller".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 2,
                end_line: 2,
                embedding: None,
                project_id: "proj-imports".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "module")),
                name: "module".to_string(),
                symbol_type: crate::types::SymbolType::Module,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-imports".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "dep")),
                name: "dep".to_string(),
                symbol_type: crate::types::SymbolType::Module,
                file_path: "src/dep.rs".to_string(),
                signature: None,
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-imports".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "orphan")),
                name: "orphan".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/orphan.rs".to_string(),
                signature: None,
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-imports".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
        ];

        let relations = vec![
            SymbolRelation {
                id: None,
                from_symbol: crate::types::Thing::new("code_symbols", "caller"),
                to_symbol: crate::types::Thing::new("code_symbols", "target"),
                relation_type: crate::types::CodeRelationType::Calls,
                relation_class: crate::types::RelationClass::Observed,
                provenance: crate::types::RelationProvenance::ParserExtracted,
                confidence_class: crate::types::ConfidenceClass::Extracted,
                freshness_generation: 0,
                staleness_state: crate::types::StalenessState::Current,
                file_path: "src/lib.rs".to_string(),
                line_number: 2,
                project_id: "proj-imports".to_string(),
                created_at: crate::types::Datetime::default(),
            },
            SymbolRelation {
                id: None,
                from_symbol: crate::types::Thing::new("code_symbols", "module"),
                to_symbol: crate::types::Thing::new("code_symbols", "dep"),
                relation_type: crate::types::CodeRelationType::Imports,
                relation_class: crate::types::RelationClass::Observed,
                provenance: crate::types::RelationProvenance::ParserExtracted,
                confidence_class: crate::types::ConfidenceClass::Extracted,
                freshness_generation: 0,
                staleness_state: crate::types::StalenessState::Current,
                file_path: "src/lib.rs".to_string(),
                line_number: 1,
                project_id: "proj-imports".to_string(),
                created_at: crate::types::Datetime::default(),
            },
        ];

        let projection = build_project_projection(
            &status,
            1,
            1,
            0,
            0,
            ProjectProjectionRequest {
                relation_scope: "imports".to_string(),
                sort_mode: "canonical".to_string(),
            },
            &symbols,
            &relations,
        );

        assert_eq!(projection.request.relation_scope, "imports");
        assert_eq!(projection.edges.len(), 1);
        assert_eq!(projection.edges[0].relation_type, "imports");
        assert_eq!(projection.nodes.len(), 2);
        assert_eq!(projection.counts.nodes, Some(2));
        let node_ids: Vec<_> = projection.nodes.iter().map(|node| node.id.as_str()).collect();
        assert!(node_ids.contains(&"module"));
        assert!(node_ids.contains(&"dep"));
        assert!(!node_ids.contains(&"orphan"));
        assert_eq!(projection.counts.edges, Some(1));
    }

    #[test]
    fn build_project_projection_with_type_links_scope_filters_to_extends_and_implements() {
        let status = IndexStatus::new("proj-type-links".to_string());

        let symbols = vec![
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "caller")),
                name: "caller".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 2,
                end_line: 2,
                embedding: None,
                project_id: "proj-type-links".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "child")),
                name: "child".to_string(),
                symbol_type: crate::types::SymbolType::Class,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 3,
                end_line: 3,
                embedding: None,
                project_id: "proj-type-links".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "parent")),
                name: "parent".to_string(),
                symbol_type: crate::types::SymbolType::Class,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 4,
                end_line: 4,
                embedding: None,
                project_id: "proj-type-links".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "impl")),
                name: "impl".to_string(),
                symbol_type: crate::types::SymbolType::Class,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 5,
                end_line: 5,
                embedding: None,
                project_id: "proj-type-links".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "trait")),
                name: "trait".to_string(),
                symbol_type: crate::types::SymbolType::Trait,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 6,
                end_line: 6,
                embedding: None,
                project_id: "proj-type-links".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "orphan")),
                name: "orphan".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/orphan.rs".to_string(),
                signature: None,
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-type-links".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
        ];

        let relations = vec![
            SymbolRelation {
                id: None,
                from_symbol: crate::types::Thing::new("code_symbols", "caller"),
                to_symbol: crate::types::Thing::new("code_symbols", "target"),
                relation_type: crate::types::CodeRelationType::Calls,
                relation_class: crate::types::RelationClass::Observed,
                provenance: crate::types::RelationProvenance::ParserExtracted,
                confidence_class: crate::types::ConfidenceClass::Extracted,
                freshness_generation: 0,
                staleness_state: crate::types::StalenessState::Current,
                file_path: "src/lib.rs".to_string(),
                line_number: 2,
                project_id: "proj-type-links".to_string(),
                created_at: crate::types::Datetime::default(),
            },
            SymbolRelation {
                id: None,
                from_symbol: crate::types::Thing::new("code_symbols", "child"),
                to_symbol: crate::types::Thing::new("code_symbols", "parent"),
                relation_type: crate::types::CodeRelationType::Extends,
                relation_class: crate::types::RelationClass::Observed,
                provenance: crate::types::RelationProvenance::ParserExtracted,
                confidence_class: crate::types::ConfidenceClass::Extracted,
                freshness_generation: 0,
                staleness_state: crate::types::StalenessState::Current,
                file_path: "src/lib.rs".to_string(),
                line_number: 3,
                project_id: "proj-type-links".to_string(),
                created_at: crate::types::Datetime::default(),
            },
            SymbolRelation {
                id: None,
                from_symbol: crate::types::Thing::new("code_symbols", "impl"),
                to_symbol: crate::types::Thing::new("code_symbols", "trait"),
                relation_type: crate::types::CodeRelationType::Implements,
                relation_class: crate::types::RelationClass::Observed,
                provenance: crate::types::RelationProvenance::ParserExtracted,
                confidence_class: crate::types::ConfidenceClass::Extracted,
                freshness_generation: 0,
                staleness_state: crate::types::StalenessState::Current,
                file_path: "src/lib.rs".to_string(),
                line_number: 4,
                project_id: "proj-type-links".to_string(),
                created_at: crate::types::Datetime::default(),
            },
        ];

        let projection = build_project_projection(
            &status,
            1,
            1,
            0,
            0,
            ProjectProjectionRequest {
                relation_scope: "type_links".to_string(),
                sort_mode: "canonical".to_string(),
            },
            &symbols,
            &relations,
        );

        assert_eq!(projection.request.relation_scope, "type_links");
        assert_eq!(projection.edges.len(), 2);
        assert!(projection
            .edges
            .iter()
            .all(|edge| edge.relation_type == "extends" || edge.relation_type == "implements"));
        assert_eq!(projection.nodes.len(), 4);
        assert_eq!(projection.counts.nodes, Some(4));
        let node_ids: Vec<_> = projection.nodes.iter().map(|node| node.id.as_str()).collect();
        assert!(node_ids.contains(&"child"));
        assert!(node_ids.contains(&"parent"));
        assert!(node_ids.contains(&"impl"));
        assert!(node_ids.contains(&"trait"));
        assert!(!node_ids.contains(&"orphan"));
        assert_eq!(projection.counts.edges, Some(2));
    }

    #[test]
    fn build_project_projection_with_calls_scope_prunes_unrelated_nodes() {
        let status = IndexStatus::new("proj-calls-prune".to_string());

        let symbols = vec![
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "caller")),
                name: "caller".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 2,
                end_line: 2,
                embedding: None,
                project_id: "proj-calls-prune".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "target")),
                name: "target".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-calls-prune".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "orphan")),
                name: "orphan".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/orphan.rs".to_string(),
                signature: None,
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-calls-prune".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
        ];

        let relations = vec![
            SymbolRelation {
                id: None,
                from_symbol: crate::types::Thing::new("code_symbols", "caller"),
                to_symbol: crate::types::Thing::new("code_symbols", "target"),
                relation_type: crate::types::CodeRelationType::Calls,
                relation_class: crate::types::RelationClass::Observed,
                provenance: crate::types::RelationProvenance::ParserExtracted,
                confidence_class: crate::types::ConfidenceClass::Extracted,
                freshness_generation: 0,
                staleness_state: crate::types::StalenessState::Current,
                file_path: "src/lib.rs".to_string(),
                line_number: 2,
                project_id: "proj-calls-prune".to_string(),
                created_at: crate::types::Datetime::default(),
            },
            SymbolRelation {
                id: None,
                from_symbol: crate::types::Thing::new("code_symbols", "caller"),
                to_symbol: crate::types::Thing::new("code_symbols", "target"),
                relation_type: crate::types::CodeRelationType::Imports,
                relation_class: crate::types::RelationClass::Observed,
                provenance: crate::types::RelationProvenance::ParserExtracted,
                confidence_class: crate::types::ConfidenceClass::Extracted,
                freshness_generation: 0,
                staleness_state: crate::types::StalenessState::Current,
                file_path: "src/lib.rs".to_string(),
                line_number: 1,
                project_id: "proj-calls-prune".to_string(),
                created_at: crate::types::Datetime::default(),
            },
        ];

        let projection = build_project_projection(
            &status,
            1,
            1,
            0,
            0,
            ProjectProjectionRequest {
                relation_scope: "calls".to_string(),
                sort_mode: "canonical".to_string(),
            },
            &symbols,
            &relations,
        );

        assert_eq!(projection.request.relation_scope, "calls");
        assert_eq!(projection.edges.len(), 1);
        assert_eq!(projection.nodes.len(), 2);
        assert_eq!(projection.counts.nodes, Some(2));
        let node_ids: Vec<_> = projection.nodes.iter().map(|node| node.id.as_str()).collect();
        assert!(node_ids.contains(&"caller"));
        assert!(node_ids.contains(&"target"));
        assert!(!node_ids.contains(&"orphan"));
    }

    #[test]
    fn build_project_projection_with_none_scope_returns_empty_graph() {
        let status = IndexStatus::new("proj-none-empty".to_string());

        let symbols = vec![CodeSymbol {
            id: Some(crate::types::Thing::new("code_symbols", "orphan")),
            name: "orphan".to_string(),
            symbol_type: crate::types::SymbolType::Function,
            file_path: "src/orphan.rs".to_string(),
            signature: None,
            start_line: 1,
            end_line: 1,
            embedding: None,
            project_id: "proj-none-empty".to_string(),
            indexed_at: crate::types::Datetime::default(),
        }];

        let projection = build_project_projection(
            &status,
            1,
            1,
            0,
            1,
            ProjectProjectionRequest {
                relation_scope: "none".to_string(),
                sort_mode: "canonical".to_string(),
            },
            &symbols,
            &[],
        );

        assert_eq!(projection.request.relation_scope, "none");
        assert_eq!(projection.nodes.len(), 0);
        assert_eq!(projection.edges.len(), 0);
        assert_eq!(projection.counts.nodes, Some(0));
        assert_eq!(projection.counts.edges, Some(0));
        assert_eq!(projection.shaping.relation_scope_applied, "none");
        assert_eq!(projection.shaping.sort_mode_applied, "canonical");
        assert_eq!(projection.shaping.node_selection_basis, "empty_graph_when_no_edges_retained");
        assert_eq!(projection.shaping.edge_selection_basis, "no_edges_retained");
        assert_eq!(projection.shaping.output_kind, "empty_graph");
    }

    #[test]
    fn build_project_projection_reports_induced_subgraph_shaping_semantics() {
        let status = IndexStatus::new("proj-shaping".to_string());

        let symbols = vec![
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "caller")),
                name: "caller".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 2,
                end_line: 2,
                embedding: None,
                project_id: "proj-shaping".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "target")),
                name: "target".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/lib.rs".to_string(),
                signature: None,
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-shaping".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
            CodeSymbol {
                id: Some(crate::types::Thing::new("code_symbols", "orphan")),
                name: "orphan".to_string(),
                symbol_type: crate::types::SymbolType::Function,
                file_path: "src/orphan.rs".to_string(),
                signature: None,
                start_line: 1,
                end_line: 1,
                embedding: None,
                project_id: "proj-shaping".to_string(),
                indexed_at: crate::types::Datetime::default(),
            },
        ];

        let relations = vec![SymbolRelation {
            id: None,
            from_symbol: crate::types::Thing::new("code_symbols", "caller"),
            to_symbol: crate::types::Thing::new("code_symbols", "target"),
            relation_type: crate::types::CodeRelationType::Calls,
            relation_class: crate::types::RelationClass::Observed,
            provenance: crate::types::RelationProvenance::ParserExtracted,
            confidence_class: crate::types::ConfidenceClass::Extracted,
            freshness_generation: 0,
            staleness_state: crate::types::StalenessState::Current,
            file_path: "src/lib.rs".to_string(),
            line_number: 2,
            project_id: "proj-shaping".to_string(),
            created_at: crate::types::Datetime::default(),
        }];

        let projection = build_project_projection(
            &status,
            1,
            1,
            0,
            3,
            ProjectProjectionRequest {
                relation_scope: "calls".to_string(),
                sort_mode: "canonical".to_string(),
            },
            &symbols,
            &relations,
        );

        assert_eq!(projection.shaping.relation_scope_applied, "calls");
        assert_eq!(projection.shaping.sort_mode_applied, "canonical");
        assert_eq!(projection.shaping.node_selection_basis, "relation_endpoint_induced_subgraph");
        assert_eq!(projection.shaping.edge_selection_basis, "only_call_edges");
        assert_eq!(projection.shaping.output_kind, "induced_symbol_graph");
    }

    #[test]
    fn build_project_projection_normalizes_unknown_request_values_to_defaults() {
        let status = IndexStatus::new("proj-defaults".to_string());

        let projection = build_project_projection(
            &status,
            1,
            1,
            0,
            0,
            ProjectProjectionRequest {
                relation_scope: "weird".to_string(),
                sort_mode: "odd".to_string(),
            },
            &[],
            &[],
        );

        assert_eq!(projection.request.relation_scope, "all");
        assert_eq!(projection.request.sort_mode, "canonical");
    }
}

pub fn with_surface_guidance(
    mut contract: ExportContractMeta,
    preferred_response_fields: &[&str],
    legacy_compatibility_fields: &[&str],
    forbidden_to_depend_fields: &[&str],
) -> ExportContractMeta {
    contract.surface_guidance = SurfaceGuidance {
        preferred_response_fields: preferred_response_fields
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        legacy_compatibility_fields: legacy_compatibility_fields
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        forbidden_to_depend_fields: forbidden_to_depend_fields
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
    };
    contract
}

pub fn with_traversal_defaults(
    mut contract: ExportContractMeta,
    default_relation_scope: &str,
    relation_metadata_exposed: &[&str],
) -> ExportContractMeta {
    contract.traversal_defaults = Some(TraversalDefaults {
        default_relation_scope: default_relation_scope.to_string(),
        relation_metadata_exposed: relation_metadata_exposed
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        frontier_semantics: None,
        frontier_items_identity_basis: None,
        frontier_items_are_stable_node_ids: None,
        frontier_items_are_project_scoped: None,
        frontier_is_cursor: None,
    });
    contract
}

pub fn with_frontier_contract(
    mut contract: ExportContractMeta,
    frontier_semantics: &str,
    identity_basis: &str,
    are_stable_node_ids: bool,
    are_project_scoped: bool,
) -> ExportContractMeta {
    let defaults = contract
        .traversal_defaults
        .get_or_insert_with(|| TraversalDefaults {
            default_relation_scope: "mixed".to_string(),
            relation_metadata_exposed: Vec::new(),
            frontier_semantics: None,
            frontier_items_identity_basis: None,
            frontier_items_are_stable_node_ids: None,
            frontier_items_are_project_scoped: None,
            frontier_is_cursor: None,
        });

    defaults.frontier_semantics = Some(frontier_semantics.to_string());
    defaults.frontier_items_identity_basis = Some(identity_basis.to_string());
    defaults.frontier_items_are_stable_node_ids = Some(are_stable_node_ids);
    defaults.frontier_items_are_project_scoped = Some(are_project_scoped);
    defaults.frontier_is_cursor = Some(false);
    contract
}

pub fn summary_graph_response(nodes: usize, edges: usize) -> ExportResponseSummary {
    ExportResponseSummary {
        result_kind: "graph".to_string(),
        counts: CountSummary {
            nodes: Some(nodes),
            edges: Some(edges),
            ..Default::default()
        },
        traversal: None,
        partial: None,
    }
}

pub fn summary_project_projection_response(
    status: &IndexStatus,
    nodes: usize,
    edges: usize,
) -> ExportResponseSummary {
    let is_partial = status.projection_state != crate::types::ProjectionState::Current;

    ExportResponseSummary {
        result_kind: "graph".to_string(),
        counts: CountSummary {
            nodes: Some(nodes),
            edges: Some(edges),
            ..Default::default()
        },
        traversal: None,
        partial: Some(PartialSummary {
            is_partial,
            reason_code: projection_partial_reason_code(status),
            reason: projection_partial_reason_slug(status),
            message: Some(if is_partial {
                "Projection is an on-demand export of the latest available semantic snapshot and may lag structural changes."
                    .to_string()
            } else {
                "Projection is an on-demand export of the current semantic snapshot; no separately materialized artifact is promised."
                    .to_string()
            }),
        }),
    }
}

pub fn summary_project_projection_shaping(
    request: &ProjectProjectionRequest,
    nodes: usize,
    edges: usize,
) -> crate::types::ProjectionShapingSummary {
    let relation_scope_applied = request.relation_scope.clone();
    let edge_selection_basis = match relation_scope_applied.as_str() {
        "calls" => "only_call_edges".to_string(),
        "imports" => "only_import_edges".to_string(),
        "type_links" => "only_extends_and_implements_edges".to_string(),
        "none" => "no_edges_retained".to_string(),
        _ => "all_relation_edges".to_string(),
    };

    let node_selection_basis = if relation_scope_applied == "none" {
        "empty_graph_when_no_edges_retained".to_string()
    } else {
        "relation_endpoint_induced_subgraph".to_string()
    };

    let output_kind = if nodes == 0 && edges == 0 {
        "empty_graph".to_string()
    } else {
        "induced_symbol_graph".to_string()
    };

    crate::types::ProjectionShapingSummary {
        relation_scope_applied,
        sort_mode_applied: request.sort_mode.clone(),
        node_selection_basis,
        edge_selection_basis,
        output_kind,
    }
}

pub fn summary_symbol_graph_response(
    nodes: usize,
    edges: usize,
    depth_reached: usize,
    truncated: bool,
    deferred_count: usize,
    frontier: &[String],
) -> ExportResponseSummary {
    ExportResponseSummary {
        result_kind: "graph".to_string(),
        counts: CountSummary {
            nodes: Some(nodes),
            edges: Some(edges),
            ..Default::default()
        },
        traversal: Some(TraversalSummary {
            depth_reached: Some(depth_reached.try_into().unwrap_or(u32::MAX)),
            truncated: Some(truncated),
            deferred_count: Some(deferred_count),
            frontier: Some(FrontierSummary {
                count: frontier.len(),
                items: frontier.to_vec(),
            }),
        }),
        partial: None,
    }
}

pub fn summary_collection_response(
    result_kind: &str,
    count: usize,
    total: Option<usize>,
    is_partial: bool,
    message: Option<String>,
) -> ExportResponseSummary {
    ExportResponseSummary {
        result_kind: result_kind.to_string(),
        counts: CountSummary {
            results: Some(count),
            total,
            ..Default::default()
        },
        traversal: None,
        partial: Some(PartialSummary {
            is_partial,
            reason_code: collection_partial_reason_code(is_partial),
            reason: collection_partial_reason_slug(is_partial),
            message,
        }),
    }
}

pub fn summary_index_status_response(
    total_files: u32,
    indexed_files: u32,
    total_chunks: u32,
    total_symbols: u32,
    overall_progress_percent: f32,
    is_partial: bool,
    message: Option<String>,
) -> ExportResponseSummary {
    ExportResponseSummary {
        result_kind: "status".to_string(),
        counts: CountSummary {
            files: Some(total_files),
            indexed_files: Some(indexed_files),
            chunks: Some(total_chunks),
            symbols: Some(total_symbols),
            ..Default::default()
        },
        traversal: None,
        partial: Some(PartialSummary {
            is_partial,
            reason_code: index_status_reason_code(is_partial),
            reason: index_status_reason_slug(is_partial, overall_progress_percent),
            message,
        }),
    }
}

pub fn exported_graph_nodes(entities: &[Entity]) -> Vec<ExportedGraphNode> {
    entities
        .iter()
        .filter_map(|entity| {
            entity
                .id
                .as_ref()
                .map(|id| ExportedGraphNode::from_entity(entity, record_key_to_string(&id.key)))
        })
        .collect()
}

pub fn exported_graph_edges(relations: &[Relation]) -> Vec<ExportedGraphEdge> {
    relations
        .iter()
        .map(|relation| {
            ExportedGraphEdge::from_relation(
                relation,
                relation.id.as_ref().map(|id| record_key_to_string(&id.key)),
                record_key_to_string(&relation.from_entity.key),
                record_key_to_string(&relation.to_entity.key),
            )
        })
        .collect()
}

pub fn exported_symbol_nodes(symbols: &[CodeSymbol]) -> Vec<ExportedSymbolNode> {
    let mut nodes: Vec<_> = symbols
        .iter()
        .filter_map(|symbol| {
            symbol
                .id
                .as_ref()
                .map(|id| ExportedSymbolNode::from_symbol(symbol, record_key_to_string(&id.key)))
        })
        .collect();

    nodes.sort_by(|a, b| {
        a.id
            .cmp(&b.id)
            .then_with(|| a.project_id.cmp(&b.project_id))
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.start_line.cmp(&b.start_line))
            .then_with(|| a.end_line.cmp(&b.end_line))
            .then_with(|| a.name.cmp(&b.name))
    });

    nodes
}

pub fn exported_symbol_edges(relations: &[SymbolRelation]) -> Vec<ExportedSymbolEdge> {
    let mut edges: Vec<_> = relations
        .iter()
        .map(|relation| {
            ExportedSymbolEdge::from_relation(
                relation,
                relation.id.as_ref().map(|id| record_key_to_string(&id.key)),
                record_key_to_string(&relation.from_symbol.key),
                record_key_to_string(&relation.to_symbol.key),
            )
        })
        .collect();

    edges.sort_by(|a, b| {
        a.from_id
            .cmp(&b.from_id)
            .then_with(|| a.to_id.cmp(&b.to_id))
            .then_with(|| a.relation_type.cmp(&b.relation_type))
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.line_number.cmp(&b.line_number))
            .then_with(|| a.project_id.cmp(&b.project_id))
    });

    edges
}

pub fn collect_project_projection_inputs(
    status: IndexStatus,
    total_files: u32,
    indexed_files: u32,
    total_chunks: u32,
    total_symbols: u32,
    request: ProjectProjectionRequest,
    symbols: Vec<CodeSymbol>,
    relations: Vec<SymbolRelation>,
) -> ProjectProjectionInputs {
    ProjectProjectionInputs {
        status,
        total_files,
        indexed_files,
        total_chunks,
        total_symbols,
        request,
        symbols,
        relations,
    }
}

fn normalize_relation_scope(scope: &str) -> String {
    match scope {
        "calls" => "calls".to_string(),
        "imports" => "imports".to_string(),
        "type_links" => "type_links".to_string(),
        "none" => "none".to_string(),
        _ => "all".to_string(),
    }
}

fn normalize_sort_mode(mode: &str) -> String {
    match mode {
        "canonical" => "canonical".to_string(),
        _ => "canonical".to_string(),
    }
}

pub fn shape_project_projection_graph(
    mut inputs: ProjectProjectionInputs,
) -> ProjectProjectionInputs {
    inputs.request.relation_scope = normalize_relation_scope(&inputs.request.relation_scope);
    inputs.request.sort_mode = normalize_sort_mode(&inputs.request.sort_mode);

    match inputs.request.relation_scope.as_str() {
        "calls" => {
            inputs
                .relations
                .retain(|relation| relation.relation_type.to_string() == "calls");
        }
        "imports" => {
            inputs.relations.retain(|relation| {
                relation.relation_type == crate::types::CodeRelationType::Imports
            });
        }
        "type_links" => {
            inputs.relations.retain(|relation| {
                matches!(
                    relation.relation_type,
                    crate::types::CodeRelationType::Extends
                        | crate::types::CodeRelationType::Implements
                )
            });
        }
        "none" => {
            inputs.relations.clear();
        }
        _ => {}
    }

    if inputs.request.relation_scope == "none" {
        inputs.symbols.clear();
    } else {
        let mut referenced_symbol_ids = std::collections::HashSet::new();
        for relation in &inputs.relations {
            referenced_symbol_ids.insert(record_key_to_string(&relation.from_symbol.key));
            referenced_symbol_ids.insert(record_key_to_string(&relation.to_symbol.key));
        }

        inputs.symbols.retain(|symbol| {
            symbol
                .id
                .as_ref()
                .map(|id| referenced_symbol_ids.contains(&record_key_to_string(&id.key)))
                .unwrap_or(false)
        });
    }

    inputs.symbols.sort_by(|a, b| {
        a.project_id
            .cmp(&b.project_id)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.start_line.cmp(&b.start_line))
            .then_with(|| a.end_line.cmp(&b.end_line))
            .then_with(|| a.name.cmp(&b.name))
    });

    inputs.relations.sort_by(|a, b| {
        a.project_id
            .cmp(&b.project_id)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.line_number.cmp(&b.line_number))
            .then_with(|| record_key_to_string(&a.from_symbol.key).cmp(&record_key_to_string(&b.from_symbol.key)))
            .then_with(|| record_key_to_string(&a.to_symbol.key).cmp(&record_key_to_string(&b.to_symbol.key)))
            .then_with(|| a.relation_type.to_string().cmp(&b.relation_type.to_string()))
    });

    inputs
}

pub fn assemble_project_projection(
    inputs: ProjectProjectionInputs,
) -> ExportedProjectProjection {
    let ProjectProjectionInputs {
        status,
        total_files,
        indexed_files,
        total_chunks,
        total_symbols,
        request,
        symbols,
        relations,
    } = inputs;

    let lifecycle = lifecycle_view(&status);
    let contract = with_surface_guidance(
        export_contract_meta(
            ExportIdentity {
                project_id: Some(status.project_id.clone()),
                stable_node_ids: true,
                node_ids_are_project_scoped: true,
                stable_edge_ids: false,
                edge_ids_are_local_only: true,
                node_id_semantics: Some("stable_project_scoped_project_id".to_string()),
                edge_id_semantics: Some("no_public_edge_ids".to_string()),
                ..Default::default()
            },
            Some(&status),
        ),
        &["projection", "contract"],
        &["status", "lifecycle"],
        &[],
    );

    let shaping = summary_project_projection_shaping(&request, symbols.len(), relations.len());

    ExportedProjectProjection {
        project_id: status.project_id.clone(),
        request,
        summary: summary_project_projection_response(&status, symbols.len(), relations.len()),
        shaping,
        counts: CountSummary {
            files: Some(total_files),
            indexed_files: Some(indexed_files),
            chunks: Some(total_chunks),
            symbols: Some(total_symbols),
            nodes: Some(symbols.len()),
            edges: Some(relations.len()),
            ..Default::default()
        },
        nodes: exported_symbol_nodes(&symbols),
        edges: exported_symbol_edges(&relations),
        lifecycle,
        contract,
    }
}

pub fn build_project_projection(
    status: &IndexStatus,
    total_files: u32,
    indexed_files: u32,
    total_chunks: u32,
    total_symbols: u32,
    request: ProjectProjectionRequest,
    symbols: &[CodeSymbol],
    relations: &[SymbolRelation],
) -> ExportedProjectProjection {
    let inputs = collect_project_projection_inputs(
        status.clone(),
        total_files,
        indexed_files,
        total_chunks,
        total_symbols,
        request,
        symbols.to_vec(),
        relations.to_vec(),
    );
    let shaped = shape_project_projection_graph(inputs);
    assemble_project_projection(shaped)
}
