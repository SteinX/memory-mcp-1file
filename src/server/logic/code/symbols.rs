use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::server::params::{normalize_project_id, SearchSymbolsParams, SymbolGraphParams};
use crate::storage::StorageBackend;
use crate::types::ExportIdentity;

use super::super::{error_response, strip_symbol_embeddings, success_json};
use super::super::contracts::{
    export_contract_meta, exported_symbol_edges, exported_symbol_nodes,
    summary_collection_response, summary_symbol_graph_response, with_frontier_contract,
    with_surface_guidance, with_traversal_defaults,
};

fn symbol_contract_json(symbol_id: &str) -> serde_json::Value {
    let contract = with_traversal_defaults(
        with_surface_guidance(
            export_contract_meta(
                ExportIdentity {
                    stable_symbol_id: Some(symbol_id.to_string()),
                    stable_node_ids: true,
                    node_ids_are_project_scoped: true,
                    stable_edge_ids: false,
                    edge_ids_are_local_only: true,
                    node_id_semantics: Some("stable_project_scoped_symbol_id".to_string()),
                    edge_id_semantics: Some("local_only_edge_reference".to_string()),
                    ..Default::default()
                },
                None,
            ),
            &["nodes", "edges", "contract"],
            &["symbols", "relations", "results"],
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
        "unexpanded_symbol_boundary_for_manual_follow_up",
        "stable_project_scoped_symbol_id",
        true,
        true,
    );
    serde_json::to_value(contract).unwrap_or_else(|_| json!({}))
}

pub async fn search_symbols(
    state: &Arc<AppState>,
    params: SearchSymbolsParams,
) -> anyhow::Result<CallToolResult> {
    let SearchSymbolsParams {
        query,
        project_id,
        limit,
        offset,
        symbol_type,
        path_prefix,
    } = params;
    let project_id = normalize_project_id(project_id);
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let offset = offset.unwrap_or(0);

    match state
        .storage
        .search_symbols(
            &query,
            project_id.as_deref(),
            limit,
            offset,
            symbol_type.as_deref(),
            path_prefix.as_deref(),
        )
        .await
    {
        Ok((mut symbols, total)) => {
            let count = symbols.len();
            strip_symbol_embeddings(&mut symbols);

            let has_more = offset + count < total as usize;

            let mut response = json!({
                "results": symbols,
                "count": count,
                "total": total,
                "offset": offset,
                "limit": limit,
                "has_more": has_more,
                "summary": summary_collection_response("collection", count, Some(total as usize), false, None),
                "contract": symbol_contract_json("search_symbols"),
                "query": query,
                "filters": {
                    "project_id": project_id,
                    "symbol_type": symbol_type,
                    "path_prefix": path_prefix
                }
            });

            if let Some(degradation) = super::get_degradation_info(state).await {
                response["_indexing"] = degradation;
            }

            Ok(success_json(response))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn symbol_graph(
    state: &Arc<AppState>,
    params: SymbolGraphParams,
) -> anyhow::Result<CallToolResult> {
    match params.action.as_str() {
        "callers" => match state.storage.get_symbol_callers(&params.symbol_id).await {
            Ok(mut callers) => {
                strip_symbol_embeddings(&mut callers);
                let mut response = json!({
                    "results": callers,
                    "count": callers.len(),
                    "summary": summary_collection_response("collection", callers.len(), Some(callers.len()), false, None),
                    "contract": symbol_contract_json(&params.symbol_id),
                    "symbol_id": params.symbol_id
                });
                if let Some(degradation) = super::get_degradation_info(state).await {
                    response["_indexing"] = degradation;
                }
                Ok(success_json(response))
            }
            Err(e) => Ok(error_response(e)),
        },
        "callees" => match state.storage.get_symbol_callees(&params.symbol_id).await {
            Ok(mut callees) => {
                strip_symbol_embeddings(&mut callees);
                let mut response = json!({
                    "results": callees,
                    "count": callees.len(),
                    "summary": summary_collection_response("collection", callees.len(), Some(callees.len()), false, None),
                    "contract": symbol_contract_json(&params.symbol_id),
                    "symbol_id": params.symbol_id
                });
                if let Some(degradation) = super::get_degradation_info(state).await {
                    response["_indexing"] = degradation;
                }
                Ok(success_json(response))
            }
            Err(e) => Ok(error_response(e)),
        },
        "related" => {
            use crate::graph::{SymbolGraphTraverser, TraversalConfig};
            use crate::types::Direction;

            let depth = params.depth.unwrap_or(1).min(5);
            let direction: Direction = params
                .direction
                .as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default();

            let config = TraversalConfig {
                max_depth: 5,
                max_entities_per_level: 50,
                max_total_entities: 200,
            };

            let traverser = SymbolGraphTraverser::with_config(state.storage.as_ref(), config);

            match traverser
                .traverse(&params.symbol_id, depth, direction)
                .await
            {
                Ok(result) => {
                    let mut symbols = result.symbols;
                    strip_symbol_embeddings(&mut symbols);
                    let nodes = exported_symbol_nodes(&symbols);
                    let edges = exported_symbol_edges(&result.relations);
                    let mut response = json!({
                        "summary": summary_symbol_graph_response(
                            nodes.len(),
                            edges.len(),
                            result.depth_reached,
                            result.truncated,
                            result.deferred_count,
                            &result.frontier,
                        ),
                        "nodes": nodes,
                        "edges": edges,
                        "symbols": symbols,
                        "relations": result.relations,
                        "contract": symbol_contract_json(&params.symbol_id),
                        "symbol_count": symbols.len(),
                        "relation_count": result.relations.len(),
                        "depth_reached": result.depth_reached,
                        "truncated": result.truncated,
                        "deferred_count": result.deferred_count,
                        "frontier": result.frontier
                    });
                    if let Some(degradation) = super::get_degradation_info(state).await {
                        response["_indexing"] = degradation;
                    }
                    Ok(success_json(response))
                }
                Err(e) => Ok(error_response(e)),
            }
        }
        other => Ok(error_response(anyhow::anyhow!(
            "Invalid action '{}'. Use: callers, callees, related",
            other
        ))),
    }
}
