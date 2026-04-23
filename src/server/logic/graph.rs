use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::embedding::ContentHasher;
use crate::graph::detect_communities as detect_communities_algo;
use crate::server::params::{
    CreateEntityParams, CreateRelationParams, DetectCommunitiesParams, GetRelatedParams,
};
use crate::storage::StorageBackend;
use crate::types::{
    ConfidenceClass, Datetime, Direction, Entity, ExportIdentity, RecordId, Relation, RelationClass,
    RelationProvenance, StalenessState, ThingId,
};

use super::{error_response, strip_entity_embeddings, success_json};
use super::contracts::{
    export_contract_meta, exported_graph_edges, exported_graph_nodes, summary_graph_response,
    with_frontier_contract, with_surface_guidance, with_traversal_defaults,
};

fn graph_contract_json(entity_id: Option<&str>) -> serde_json::Value {
    let contract = with_traversal_defaults(
        with_surface_guidance(
            export_contract_meta(
                ExportIdentity {
                    entity_id: entity_id.map(|id| id.to_string()),
                    stable_node_ids: true,
                    node_ids_are_project_scoped: false,
                    stable_edge_ids: false,
                    edge_ids_are_local_only: true,
                    node_id_semantics: Some("stable_public_node_id".to_string()),
                    edge_id_semantics: Some("local_only_edge_reference".to_string()),
                    ..Default::default()
                },
                None,
            ),
            &["nodes", "edges", "contract"],
            &["entities", "relations"],
            &["relations[].id", "edges[].id"],
        ),
        "mixed",
        &[
            "relation_class",
            "provenance",
            "confidence_class",
            "freshness_generation",
            "staleness_state",
        ],
    );

    let contract = with_frontier_contract(
        contract,
        "unexpanded_boundary_for_manual_follow_up",
        "stable_public_node_id",
        true,
        false,
    );

    serde_json::to_value(contract)
    .unwrap_or_else(|_| json!({}))
}

pub async fn create_entity(
    state: &Arc<AppState>,
    params: CreateEntityParams,
) -> anyhow::Result<CallToolResult> {
    let embed_text = format!(
        "{}: {}",
        params.name,
        params.description.as_deref().unwrap_or("")
    );
    let embedding = state.embedding.embed(&embed_text).await.ok();
    let content_hash = Some(ContentHasher::hash(&embed_text));

    let entity = Entity {
        name: params.name,
        entity_type: params.entity_type.unwrap_or_else(|| "unknown".to_string()),
        description: params.description,
        embedding,
        content_hash,
        user_id: params.user_id,
        ..Default::default()
    };

    match state.storage.create_entity(entity).await {
        Ok(id) => Ok(success_json(json!({ "id": id }))),
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn create_relation(
    state: &Arc<AppState>,
    params: CreateRelationParams,
) -> anyhow::Result<CallToolResult> {
    // Validate entity IDs to prevent SQL injection and Thing::from panics
    let from_id = match ThingId::new("entities", &params.from_entity) {
        Ok(id) => id,
        Err(e) => {
            return Ok(error_response(anyhow::anyhow!(
                "Invalid from_entity: {}",
                e
            )))
        }
    };
    let to_id = match ThingId::new("entities", &params.to_entity) {
        Ok(id) => id,
        Err(e) => return Ok(error_response(anyhow::anyhow!("Invalid to_entity: {}", e))),
    };

    let relation = Relation {
        id: None,
        from_entity: RecordId::new("entities", from_id.id().to_string()),
        to_entity: RecordId::new("entities", to_id.id().to_string()),
        relation_type: params.relation_type,
        relation_class: RelationClass::Observed,
        provenance: RelationProvenance::ImportedManual,
        confidence_class: ConfidenceClass::Extracted,
        freshness_generation: 0,
        staleness_state: StalenessState::Current,
        weight: params.weight.unwrap_or(1.0).clamp(0.0, 1.0),
        valid_from: Datetime::default(),
        valid_until: None,
    };

    match state.storage.create_relation(relation).await {
        Ok(id) => Ok(success_json(json!({ "id": id }))),
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn get_related(
    state: &Arc<AppState>,
    params: GetRelatedParams,
) -> anyhow::Result<CallToolResult> {
    let depth = params.depth.unwrap_or(1).min(3);
    let direction: Direction = params
        .direction
        .as_ref()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();

    match state
        .storage
        .get_related(&params.entity_id, depth, direction)
        .await
    {
        Ok((mut entities, relations)) => {
            strip_entity_embeddings(&mut entities);
            let nodes = exported_graph_nodes(&entities);
            let edges = exported_graph_edges(&relations);
            Ok(success_json(json!({
                "contract": graph_contract_json(Some(&params.entity_id)),
                "summary": summary_graph_response(nodes.len(), edges.len()),
                "nodes": nodes,
                "edges": edges,
                "entities": entities,
                "relations": relations,
                "entity_count": entities.len(),
                "relation_count": relations.len()
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn detect_communities(
    state: &Arc<AppState>,
    _params: DetectCommunitiesParams,
) -> anyhow::Result<CallToolResult> {
    use petgraph::graph::DiGraph;
    use std::collections::HashMap;

    let entities = match state.storage.get_all_entities().await {
        Ok(e) => e,
        Err(e) => return Ok(error_response(e)),
    };

    let relations = match state.storage.get_all_relations().await {
        Ok(r) => r,
        Err(e) => return Ok(error_response(e)),
    };

    let mut graph: DiGraph<String, f32> = DiGraph::new();
    let mut node_map = HashMap::new();

    for entity in &entities {
        if let Some(ref id) = entity.id {
            let id_str = crate::types::record_key_to_string(&id.key);
            let idx = graph.add_node(id_str.clone());
            node_map.insert(id_str, idx);
        }
    }

    for relation in &relations {
        let from_str = crate::types::record_key_to_string(&relation.from_entity.key);
        let to_str = crate::types::record_key_to_string(&relation.to_entity.key);
        if let (Some(&from_idx), Some(&to_idx)) = (node_map.get(&from_str), node_map.get(&to_str)) {
            graph.add_edge(from_idx, to_idx, relation.weight);
        }
    }

    let communities = detect_communities_algo(&graph);

    let reverse_map: HashMap<petgraph::graph::NodeIndex, String> =
        node_map.into_iter().map(|(id, idx)| (idx, id)).collect();

    let result_communities: Vec<Vec<String>> = communities
        .into_iter()
        .map(|comm| {
            comm.into_iter()
                .filter_map(|idx| reverse_map.get(&idx).cloned())
                .collect()
        })
        .collect();

    Ok(success_json(json!({
        "communities": result_communities,
        "community_count": result_communities.len(),
        "entity_count": entities.len()
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;

    #[tokio::test]
    async fn test_graph_logic() {
        let ctx = TestContext::new().await;

        // 1. Create Entities
        let e1_params = CreateEntityParams {
            name: "Alice".to_string(),
            entity_type: Some("person".to_string()),
            description: None,
            user_id: None,
        };
        let res1 = create_entity(&ctx.state, e1_params).await.unwrap();
        let val1 = serde_json::to_value(&res1).unwrap();
        let text1 = val1["content"][0]["text"].as_str().unwrap();
        let json1: serde_json::Value = serde_json::from_str(text1).unwrap();
        let id1 = json1["id"].as_str().unwrap().to_string();

        let e2_params = CreateEntityParams {
            name: "Bob".to_string(),
            entity_type: Some("person".to_string()),
            description: None,
            user_id: None,
        };
        let res2 = create_entity(&ctx.state, e2_params).await.unwrap();
        let val2 = serde_json::to_value(&res2).unwrap();
        let text2 = val2["content"][0]["text"].as_str().unwrap();
        let json2: serde_json::Value = serde_json::from_str(text2).unwrap();
        let id2 = json2["id"].as_str().unwrap().to_string();

        // 2. Create Relation
        let rel_params = CreateRelationParams {
            from_entity: id1.clone(),
            to_entity: id2.clone(),
            relation_type: "knows".to_string(),
            weight: None,
        };
        create_relation(&ctx.state, rel_params).await.unwrap();

        // 3. Get Related
        let related_params = GetRelatedParams {
            entity_id: id1.clone(),
            depth: Some(1),
            direction: Some("outgoing".to_string()),
        };
        let res_related = get_related(&ctx.state, related_params).await.unwrap();
        let val_related = serde_json::to_value(&res_related).unwrap();
        let text_related = val_related["content"][0]["text"].as_str().unwrap();
        let json_related: serde_json::Value = serde_json::from_str(text_related).unwrap();

        assert_eq!(json_related["contract"]["schema_version"], 1);
        assert_eq!(json_related["contract"]["identity"]["entity_id"], id1);
        assert_eq!(json_related["contract"]["identity"]["stable_node_ids"], true);
        assert_eq!(json_related["contract"]["identity"]["node_ids_are_project_scoped"], false);
        assert_eq!(json_related["contract"]["identity"]["edge_ids_are_local_only"], true);
        assert_eq!(json_related["contract"]["identity"]["node_id_semantics"], "stable_public_node_id");
        assert_eq!(json_related["contract"]["identity"]["edge_id_semantics"], "local_only_edge_reference");
        assert_eq!(json_related["contract"]["projection_state"], "missing");
        assert_eq!(json_related["contract"]["surface_guidance"]["preferred_response_fields"][0], "nodes");
        assert_eq!(json_related["contract"]["surface_guidance"]["legacy_compatibility_fields"][0], "entities");
        assert_eq!(json_related["contract"]["surface_guidance"]["forbidden_to_depend_fields"][0], "relations[].id");
        assert_eq!(json_related["contract"]["traversal_defaults"]["frontier_semantics"], "unexpanded_boundary_for_manual_follow_up");
        assert_eq!(json_related["contract"]["traversal_defaults"]["frontier_items_identity_basis"], "stable_public_node_id");
        assert_eq!(json_related["contract"]["traversal_defaults"]["frontier_items_are_stable_node_ids"], true);
        assert_eq!(json_related["contract"]["traversal_defaults"]["frontier_items_are_project_scoped"], false);
        assert_eq!(json_related["contract"]["traversal_defaults"]["frontier_is_cursor"], false);
        assert_eq!(json_related["entity_count"].as_u64().unwrap(), 1);
        assert_eq!(json_related["nodes"][0]["id"], id2);
        assert_eq!(json_related["edges"][0]["relation_type"], "knows");
        assert_eq!(json_related["entities"][0]["name"], "Bob");

        // 4. Detect Communities
        let comm_params = DetectCommunitiesParams {
            _placeholder: false,
        };
        let res_comm = detect_communities(&ctx.state, comm_params).await.unwrap();
        let val_comm = serde_json::to_value(&res_comm).unwrap();
        let text_comm = val_comm["content"][0]["text"].as_str().unwrap();
        let json_comm: serde_json::Value = serde_json::from_str(text_comm).unwrap();

        // Alice and Bob should be in the same community (connected)
        let communities = json_comm["communities"].as_array().unwrap();
        assert!(!communities.is_empty());
    }
}
