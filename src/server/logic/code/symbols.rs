use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::server::params::{normalize_project_id, SearchSymbolsParams, SymbolGraphParams};
use crate::storage::StorageBackend;
use crate::types::ExportIdentity;

use super::super::contracts::{
    export_contract_meta, exported_symbol_edges, exported_symbol_nodes,
    summary_collection_response, summary_symbol_graph_response, with_frontier_contract,
    with_surface_guidance, with_traversal_defaults,
};
use super::super::{error_response, strip_symbol_embeddings, success_json};
use super::{apply_project_resolution, resolve_project_for_code_tool, CodeToolContext};

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
    search_symbols_with_context(state, params, None).await
}

pub(crate) async fn search_symbols_with_context(
    state: &Arc<AppState>,
    params: SearchSymbolsParams,
    context: Option<CodeToolContext>,
) -> anyhow::Result<CallToolResult> {
    let SearchSymbolsParams {
        query,
        project_id,
        limit,
        offset,
        symbol_type,
        path_prefix,
    } = params;
    let project_resolution =
        resolve_project_for_code_tool(state, normalize_project_id(project_id), context.as_ref())
            .await;
    let project_id = project_resolution.project_id().map(str::to_string);
    let limit = limit.unwrap_or(20).clamp(1, 100);
    let offset = offset.unwrap_or(0);
    let active_generation = match project_id.as_deref() {
        Some(project_id) => state.storage.get_active_generation(project_id).await.ok().flatten(),
        None => None,
    };

    if project_resolution.is_stale_binding() {
        let mut response = json!({
            "results": [],
            "count": 0,
            "total": 0,
            "offset": offset,
            "limit": limit,
            "has_more": false,
            "summary": summary_collection_response(
                "collection",
                0,
                Some(0),
                true,
                Some("Session-bound project is stale; refusing cross-project fallback.".to_string()),
            ),
            "contract": symbol_contract_json("search_symbols"),
            "query": query,
            "filters": {
                "project_id": project_id,
                "symbol_type": symbol_type,
                "path_prefix": path_prefix
            }
        });
        apply_project_resolution(&mut response, &project_resolution);
        return Ok(success_json(response));
    }

    match state
        .storage
        .search_symbols(
            &query,
            project_id.as_deref(),
            limit,
            offset,
            symbol_type.as_deref(),
            path_prefix.as_deref(),
            active_generation,
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

            apply_project_resolution(&mut response, &project_resolution);

            if let Some(degradation) = super::get_degradation_info(state).await {
                response["_indexing"] = degradation;
            }

            let missing_project_diagnostic =
                super::missing_project_binding_diagnostic(state, project_id.as_deref()).await;
            if let Some(diagnostic) = missing_project_diagnostic.as_ref() {
                super::apply_missing_project_binding_diagnostic(&mut response, diagnostic);
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
        "callers" => match state
            .storage
            .get_symbol_callers(&params.symbol_id, None)
            .await
        {
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
        "callees" => match state
            .storage
            .get_symbol_callees(&params.symbol_id, None)
            .await
        {
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
