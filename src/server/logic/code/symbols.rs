use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::server::params::{GetCalleesParams, GetCallersParams, SearchSymbolsParams};
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

            Ok(success_json(json!({
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
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn get_callers(
    state: &Arc<AppState>,
    params: GetCallersParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.get_symbol_callers(&params.symbol_id).await {
        Ok(mut callers) => {
            strip_symbol_embeddings(&mut callers);
            Ok(success_json(json!({
                "results": callers,
                "count": callers.len(),
                "symbol_id": params.symbol_id
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn get_callees(
    state: &Arc<AppState>,
    params: GetCalleesParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.get_symbol_callees(&params.symbol_id).await {
        Ok(mut callees) => {
            strip_symbol_embeddings(&mut callees);
            Ok(success_json(json!({
                "results": callees,
                "count": callees.len(),
                "symbol_id": params.symbol_id
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn get_related_symbols(
    state: &Arc<AppState>,
    params: crate::server::params::GetRelatedSymbolsParams,
) -> anyhow::Result<CallToolResult> {
    use crate::types::Direction;

    let depth = params.depth.unwrap_or(1).min(3);
    let direction: Direction = params
        .direction
        .as_ref()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();

    match state
        .storage
        .get_related_symbols(&params.symbol_id, depth, direction)
        .await
    {
        Ok((mut symbols, relations)) => {
            strip_symbol_embeddings(&mut symbols);
            Ok(success_json(json!({
                "symbols": symbols,
                "relations": relations,
                "symbol_count": symbols.len(),
                "relation_count": relations.len()
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}
