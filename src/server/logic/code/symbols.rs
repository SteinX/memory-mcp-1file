use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::server::params::{SearchSymbolsParams, SymbolGraphParams};
use crate::storage::StorageBackend;

use super::super::{error_response, strip_symbol_embeddings, success_json};

pub async fn search_symbols(
    state: &Arc<AppState>,
    params: SearchSymbolsParams,
) -> anyhow::Result<CallToolResult> {
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = params.offset.unwrap_or(0);

    match state
        .storage
        .search_symbols(
            &params.query,
            params.project_id.as_deref(),
            limit,
            offset,
            params.symbol_type.as_deref(),
            params.path_prefix.as_deref(),
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
                "query": params.query,
                "filters": {
                    "project_id": params.project_id,
                    "symbol_type": params.symbol_type,
                    "path_prefix": params.path_prefix
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
                    let mut response = json!({
                        "symbols": symbols,
                        "relations": result.relations,
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
