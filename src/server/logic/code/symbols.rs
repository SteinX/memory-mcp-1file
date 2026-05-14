use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::{json, Value};

use crate::config::AppState;
use crate::server::params::{normalize_project_id, SearchSymbolsParams, SymbolGraphParams};
use crate::storage::StorageBackend;
use crate::types::{
    CapabilityKind, CodeSymbol, ContractReasonCode, ExportIdentity, ServingGenerationMetadata,
};

use super::super::contracts::{
    export_contract_meta, exported_symbol_edges, exported_symbol_nodes,
    summary_collection_response, summary_symbol_graph_response, with_frontier_contract,
    with_surface_guidance, with_traversal_defaults,
};
use super::super::{error_response, strip_symbol_embeddings, success_json};
use super::{
    apply_project_resolution, effective_indexing_generation_for_project,
    resolve_project_for_code_tool, CodeToolContext,
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

fn symbol_id(symbol: &CodeSymbol) -> Option<String> {
    symbol.id.as_ref().map(|id| {
        format!(
            "{}:{}",
            id.table.as_str(),
            crate::types::record_key_to_string(&id.key)
        )
    })
}

fn freshness_for_file(
    file_path: &str,
    item_generation: Option<u64>,
    serving_generation: Option<u64>,
    fresh_files: &std::collections::HashSet<String>,
    indexing_generation: Option<u64>,
) -> Value {
    let file_has_serving_checkpoint = fresh_files.contains(file_path);

    let state = match (
        item_generation,
        serving_generation,
        file_has_serving_checkpoint,
    ) {
        (Some(item), Some(serving), true) if item == serving => "fresh",
        (Some(item), Some(serving), false) if item == serving => match indexing_generation {
            Some(indexing) if indexing > serving => "stale",
            _ => "fresh",
        },
        (None, Some(0), true) => "fresh",
        (_, Some(_), true) => "stale",
        (_, Some(_), false) => "unknown",
        _ => "missing",
    };

    json!({
        "state": state,
        "generation": item_generation,
        "serving_generation": serving_generation,
        "file_path": file_path,
        "evidence": if file_has_serving_checkpoint { "file_checkpoint" } else { "none" },
    })
}

async fn serving_file_set(
    state: &Arc<AppState>,
    project_id: Option<&str>,
    serving_generation: Option<u64>,
) -> std::collections::HashSet<String> {
    match (project_id, serving_generation) {
        (Some(project_id), Some(generation)) => state
            .storage
            .list_file_checkpoints_for_job(project_id, generation)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|checkpoint| checkpoint.relative_file_path)
            .collect(),
        _ => std::collections::HashSet::new(),
    }
}

async fn attach_symbol_freshness(
    state: &Arc<AppState>,
    symbols: &[CodeSymbol],
    serving_generation: Option<u64>,
    indexing_generation: Option<u64>,
) -> Vec<Value> {
    let project_id = symbols.first().map(|symbol| symbol.project_id.as_str());
    let fresh_files = serving_file_set(state, project_id, serving_generation).await;
    symbols
        .iter()
        .map(|symbol| {
            let mut value = serde_json::to_value(symbol).unwrap_or_else(|_| json!({}));
            value["freshness"] = freshness_for_file(
                &symbol.file_path,
                symbol.generation,
                serving_generation,
                &fresh_files,
                indexing_generation,
            );
            value
        })
        .collect()
}

async fn attach_node_freshness(
    state: &Arc<AppState>,
    symbols: &[CodeSymbol],
    nodes: Vec<Value>,
    serving_generation: Option<u64>,
    indexing_generation: Option<u64>,
) -> Vec<Value> {
    let project_id = symbols.first().map(|symbol| symbol.project_id.as_str());
    let fresh_files = serving_file_set(state, project_id, serving_generation).await;
    nodes
        .into_iter()
        .map(|mut node| {
            if let Some(symbol) = symbols.iter().find(|symbol| {
                symbol_id(symbol)
                    .as_deref()
                    .and_then(|id| id.split_once(':').map(|(_, k)| k))
                    == node["id"].as_str()
            }) {
                node["freshness"] = freshness_for_file(
                    &symbol.file_path,
                    symbol.generation,
                    serving_generation,
                    &fresh_files,
                    indexing_generation,
                );
            }
            node
        })
        .collect()
}

fn attach_edge_freshness(edges: Vec<Value>, serving_generation: Option<u64>) -> Vec<Value> {
    edges
        .into_iter()
        .map(|mut edge| {
            let generation = edge["freshness_generation"].as_u64();
            let state = match (generation, serving_generation) {
                (Some(edge_generation), Some(serving)) if edge_generation == serving => "fresh",
                (Some(_), Some(_)) => "stale",
                (None, Some(_)) => "unknown",
                _ => "missing",
            };
            edge["freshness"] = json!({
                "state": state,
                "generation": generation,
                "serving_generation": serving_generation,
                "evidence": "relation_generation",
            });
            edge
        })
        .collect()
}

fn graph_summary_with_partial(
    nodes: usize,
    edges: usize,
    depth_reached: usize,
    truncated: bool,
    deferred_count: usize,
    frontier: &[String],
    is_partial: bool,
    reason_code: Option<ContractReasonCode>,
    reason: Option<&str>,
    message: Option<&str>,
) -> Value {
    let mut summary = serde_json::to_value(summary_symbol_graph_response(
        nodes,
        edges,
        depth_reached,
        truncated,
        deferred_count,
        frontier,
    ))
    .unwrap_or_else(|_| json!({}));
    summary["partial"] = json!({
        "is_partial": is_partial,
        "reason_code": reason_code
            .map(|code| serde_json::to_value(code).unwrap_or(Value::Null))
            .unwrap_or(Value::Null),
        "reason": reason.map(|reason| json!(reason)).unwrap_or(Value::Null),
        "message": message.map(|message| json!(message)).unwrap_or(Value::Null),
    });
    summary
}

async fn attach_graph_generation_metadata(
    response: &mut Value,
    serving: &ServingGenerationMetadata,
    state: &Arc<AppState>,
    project_id: Option<&str>,
) {
    let serving_generation = json!({
        "structural": serving.structural,
        "bm25": serving.bm25,
        "symbols": serving.symbols,
        "graph": serving.graph,
        "vector": serving.vector,
        "semantic": serving.semantic,
    });
    let effective_graph_gen = serving.graph.or(serving.structural);
    let effective_symbols_gen = serving.symbols.or(serving.structural);
    let cap_graph_gen = serving.graph;
    let (indexing_generation, is_interrupted) = match project_id {
        Some(pid) => {
            effective_indexing_generation_for_project(
                state,
                pid,
                serving.structural,
                serving.structural,
                None,
            )
            .await
        }
        None => (None, false),
    };
    let is_stale = match (indexing_generation, serving.structural) {
        (Some(i), Some(s)) => i > s,
        _ => false,
    };
    let graph_reason_code = if is_stale {
        json!("stale")
    } else if cap_graph_gen.is_none() {
        json!("missing")
    } else {
        Value::Null
    };
    let symbols_reason_code = if is_stale {
        json!("stale")
    } else if effective_symbols_gen.is_none() {
        json!("missing")
    } else {
        Value::Null
    };
    let mut capabilities = vec![
        json!({
            "capability": "graph",
            "freshness": if cap_graph_gen.is_some() { if is_stale { "stale" } else { "fresh" } } else { "missing" },
            "serving_generation": cap_graph_gen,
            "reason_code": graph_reason_code,
            "reason": if cap_graph_gen.is_some() { Value::Null } else { json!("missing_graph") },
        }),
        json!({
            "capability": "symbols",
            "freshness": if effective_symbols_gen.is_some() { if is_stale { "stale" } else { "fresh" } } else { "missing" },
            "serving_generation": effective_symbols_gen,
            "reason_code": symbols_reason_code,
            "reason": if effective_symbols_gen.is_some() { Value::Null } else { json!("no_serving_generation") },
        }),
    ];
    if is_stale {
        capabilities.push(json!({
            "capability": "graph",
            "freshness": "partial",
            "serving_generation": cap_graph_gen,
            "reason_code": "partial",
            "reason": Value::Null,
        }));
    }
    if is_interrupted && is_stale {
        capabilities.push(json!({
            "capability": "graph",
            "freshness": "degraded",
            "serving_generation": cap_graph_gen,
            "reason_code": "degraded",
            "reason": Value::Null,
        }));
    }
    response["serving_generation"] = json!(effective_graph_gen);
    response["symbol_serving_generation"] = json!(serving.symbols.or(serving.structural));
    response["indexing_generation"] = json!(indexing_generation);
    response["capability_readiness"] = json!({
        "serving_generation": cap_graph_gen,
        "indexing_generation": indexing_generation,
        "capabilities": capabilities,
        "serving_generation_by_capability": serving_generation,
    });
    if let Some(summary) = response
        .get_mut("summary")
        .and_then(|value| value.as_object_mut())
    {
        summary.insert("serving_generation".to_string(), serving_generation.clone());
        summary.insert(
            "indexing_generation".to_string(),
            json!(indexing_generation),
        );
        if is_stale {
            if let Some(partial) = summary.get_mut("partial").and_then(|v| v.as_object_mut()) {
                partial.insert("is_partial".to_string(), json!(true));
                partial.insert("reason_code".to_string(), json!("stale"));
            }
        }
    }
    if let Some(contract) = response
        .get_mut("contract")
        .and_then(|value| value.as_object_mut())
    {
        contract.insert(
            "symbol_graph".to_string(),
            json!({
                "serving_generation": serving_generation,
                "indexing_generation": indexing_generation,
                "generation_binding": "single_serving_graph_generation",
            }),
        );
    }
}

async fn serving_metadata_for_symbol(
    state: &Arc<AppState>,
    target_symbol_id: &str,
) -> anyhow::Result<ServingGenerationMetadata> {
    let project_id = state
        .storage
        .get_symbol_project_id(target_symbol_id)
        .await?;
    let Some(project_id) = project_id else {
        return Ok(ServingGenerationMetadata::default());
    };
    Ok(state.storage.get_serving_metadata(&project_id).await?)
}

fn effective_graph_serving_metadata(
    serving: &ServingGenerationMetadata,
) -> ServingGenerationMetadata {
    if serving.graph.or(serving.symbols).is_some() {
        serving.clone()
    } else {
        ServingGenerationMetadata {
            structural: serving.structural,
            bm25: serving.bm25,
            symbols: serving.structural,
            graph: serving.structural,
            vector: serving.vector,
            semantic: serving.semantic,
            indexing: serving.indexing,
        }
    }
}

async fn missing_graph_frontier_response(
    state: &Arc<AppState>,
    params: &SymbolGraphParams,
    serving: &ServingGenerationMetadata,
    indexing_generation: Option<u64>,
) -> anyhow::Result<CallToolResult> {
    let project_id = state
        .storage
        .get_symbol_project_id(&params.symbol_id)
        .await?;
    let mut symbols = match project_id.as_deref() {
        Some(project_id) => state
            .storage
            .get_project_symbols(project_id, serving.symbols)
            .await?
            .into_iter()
            .filter(|symbol| symbol_id(symbol).as_deref() == Some(params.symbol_id.as_str()))
            .take(1)
            .collect::<Vec<_>>(),
        None => vec![],
    };
    if symbols.is_empty() {
        if let Some(project_id) = project_id.as_deref() {
            symbols = state
                .storage
                .get_project_symbols(project_id, serving.symbols)
                .await?
                .into_iter()
                .take(10)
                .collect();
        }
    }
    strip_symbol_embeddings(&mut symbols);
    let frontier: Vec<String> = symbols.iter().filter_map(symbol_id).collect();
    let symbol_values =
        attach_symbol_freshness(state, &symbols, serving.symbols, indexing_generation).await;
    let nodes = attach_node_freshness(
        state,
        &symbols,
        exported_symbol_nodes(&symbols)
            .into_iter()
            .map(|node| serde_json::to_value(node).unwrap_or_else(|_| json!({})))
            .collect(),
        serving.symbols,
        indexing_generation,
    )
    .await;

    let mut response = json!({
        "summary": graph_summary_with_partial(
            nodes.len(),
            0,
            0,
            false,
            frontier.len(),
            &frontier,
            true,
            Some(ContractReasonCode::Partial),
            Some("missing_graph"),
            Some("Graph capability has no serving generation; returning symbol frontier only."),
        ),
        "nodes": nodes,
        "edges": [],
        "symbols": symbol_values,
        "relations": [],
        "contract": symbol_contract_json(&params.symbol_id),
        "symbol_count": symbols.len(),
        "relation_count": 0,
        "count": symbols.len(),
        "depth_reached": 0,
        "truncated": false,
        "deferred_count": frontier.len(),
        "frontier": frontier,
        "fallback_path": "symbol_frontier",
    });
    attach_graph_generation_metadata(&mut response, serving, state, project_id.as_deref()).await;
    if let Some(degradation) = super::get_degradation_info(state).await {
        response["_indexing"] = degradation;
    }
    Ok(success_json(response))
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
    let serving_gen = match project_id.as_deref() {
        Some(pid) => {
            let symbols_gen = state
                .storage
                .get_serving_generation(pid, CapabilityKind::Symbols)
                .await
                .ok()
                .flatten();
            if symbols_gen.is_some() {
                symbols_gen
            } else {
                state
                    .storage
                    .get_active_generation(pid)
                    .await
                    .ok()
                    .flatten()
            }
        }
        None => None,
    };
    let (indexing_generation, is_interrupted_generation) = match project_id.as_deref() {
        Some(project_id) => {
            effective_indexing_generation_for_project(
                state,
                project_id,
                serving_gen,
                serving_gen,
                None,
            )
            .await
        }
        None => (None, false),
    };

    if serving_gen.is_none() && project_id.is_some() {
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
                Some("no_serving_generation".to_string()),
            ),
            "contract": symbol_contract_json("search_symbols"),
            "filters": {
                "project_id": project_id,
                "symbol_type": symbol_type,
                "path_prefix": path_prefix
            },
            "serving_generation": Value::Null,
            "indexing_generation": indexing_generation,
            "capability_readiness": {
                "serving_generation": Value::Null,
                "indexing_generation": indexing_generation,
                "capabilities": [
                    {
                        "capability": "symbols",
                        "freshness": "missing",
                        "serving_generation": Value::Null,
                        "reason_code": "missing",
                        "reason": "no_serving_generation",
                    }
                ],
            },
        });
        if let Some(summary) = response.get_mut("summary").and_then(|v| v.as_object_mut()) {
            summary.insert(
                "partial".to_string(),
                json!({
                    "is_partial": true,
                    "reason_code": "missing",
                    "reason": "no_serving_generation",
                }),
            );
        }
        apply_project_resolution(&mut response, &project_resolution);
        return Ok(success_json(response));
    }

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
            serving_gen,
        )
        .await
    {
        Ok((mut symbols, total)) => {
            let count = symbols.len();
            strip_symbol_embeddings(&mut symbols);
            let symbol_values =
                attach_symbol_freshness(state, &symbols, serving_gen, indexing_generation).await;

            let has_more = offset + count < total as usize;

            let is_stale = match (indexing_generation, serving_gen) {
                (Some(i), Some(s)) => i > s,
                _ => false,
            };
            let is_partial = is_stale;
            let symbols_reason_code = if is_stale {
                Value::String("stale".to_string())
            } else {
                Value::Null
            };
            let mut capabilities = vec![json!({
                "capability": "symbols",
                "freshness": if serving_gen.is_some() { if is_stale { "stale" } else { "fresh" } } else { "missing" },
                "serving_generation": serving_gen,
                "reason_code": symbols_reason_code,
                "reason": Value::Null,
            })];
            if is_stale {
                capabilities.push(json!({
                    "capability": "symbols",
                    "freshness": "partial",
                    "serving_generation": serving_gen,
                    "reason_code": "partial",
                    "reason": Value::Null,
                }));
            }
            if is_interrupted_generation && is_stale {
                capabilities.push(json!({
                    "capability": "symbols",
                    "freshness": "degraded",
                    "serving_generation": serving_gen,
                    "reason_code": "degraded",
                    "reason": Value::Null,
                }));
            }

            let mut response = json!({
                "results": symbol_values,
                "count": count,
                "total": total,
                "offset": offset,
                "limit": limit,
                "has_more": has_more,
                "summary": summary_collection_response("collection", count, Some(total as usize), is_partial, None),
                "contract": symbol_contract_json("search_symbols"),
                "query": query,
                "filters": {
                    "project_id": project_id,
                    "symbol_type": symbol_type,
                    "path_prefix": path_prefix
                },
                "serving_generation": serving_gen,
                "indexing_generation": indexing_generation,
                "capability_readiness": {
                    "serving_generation": serving_gen,
                    "indexing_generation": indexing_generation,
                    "capabilities": capabilities,
                },
            });
            if is_stale {
                if let Some(partial) = response["summary"]["partial"].as_object_mut() {
                    partial.insert(
                        "reason_code".to_string(),
                        Value::String("stale".to_string()),
                    );
                }
            }

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
        Err(e) => {
            eprintln!("[search_symbols_err] error={}", e);
            Ok(error_response(e))
        }
    }
}

pub async fn symbol_graph(
    state: &Arc<AppState>,
    params: SymbolGraphParams,
) -> anyhow::Result<CallToolResult> {
    let raw_serving = serving_metadata_for_symbol(state, &params.symbol_id).await?;
    let symbol_project_id = state
        .storage
        .get_symbol_project_id(&params.symbol_id)
        .await?;

    let effective_for_guard = effective_graph_serving_metadata(&raw_serving);
    if effective_for_guard.graph.is_none() && effective_for_guard.symbols.is_none() {
        let mut response = json!({
            "summary": graph_summary_with_partial(
                0, 0, 0, false, 0, &[],
                true,
                Some(ContractReasonCode::Missing),
                Some("no_serving_generation"),
                Some("No serving generation available for graph or symbol capability."),
            ),
            "nodes": [],
            "edges": [],
            "symbols": [],
            "relations": [],
            "contract": symbol_contract_json(&params.symbol_id),
            "symbol_count": 0,
            "relation_count": 0,
            "count": 0,
            "depth_reached": 0,
            "truncated": false,
            "deferred_count": 0,
            "frontier": [],
        });
        let effective_for_attach = effective_graph_serving_metadata(&raw_serving);
        attach_graph_generation_metadata(
            &mut response,
            &effective_for_attach,
            state,
            symbol_project_id.as_deref(),
        )
        .await;
        return Ok(success_json(response));
    }

    let serving = effective_graph_serving_metadata(&raw_serving);
    let graph_generation = serving.graph;

    let compute_indexing_generation =
        |pid: Option<&str>, serving_generation: Option<u64>, fallback: Option<u64>| {
            let pid = pid.map(str::to_string);
            let state = state.clone();
            async move {
                match pid.as_deref() {
                    Some(pid) => {
                        effective_indexing_generation_for_project(
                            &state,
                            pid,
                            serving_generation,
                            fallback,
                            None,
                        )
                        .await
                        .0
                    }
                    None => None,
                }
            }
        };

    if graph_generation.is_none() {
        let indexing_generation = compute_indexing_generation(
            symbol_project_id.as_deref(),
            serving.symbols,
            serving.symbols,
        )
        .await;
        return missing_graph_frontier_response(state, &params, &serving, indexing_generation)
            .await;
    }

    let indexing_generation = compute_indexing_generation(
        symbol_project_id.as_deref(),
        graph_generation,
        graph_generation,
    )
    .await;
    let is_stale = match (indexing_generation, graph_generation) {
        (Some(i), Some(s)) => i > s,
        _ => false,
    };
    let stale_reason_code = if is_stale {
        Some(ContractReasonCode::Stale)
    } else {
        None
    };

    match params.action.as_str() {
        "callers" => match state
            .storage
            .get_symbol_callers(&params.symbol_id, graph_generation)
            .await
        {
            Ok(mut callers) => {
                strip_symbol_embeddings(&mut callers);
                let result_values =
                    attach_symbol_freshness(state, &callers, graph_generation, indexing_generation)
                        .await;
                let mut response = json!({
                    "results": result_values,
                    "count": callers.len(),
                    "summary": graph_summary_with_partial(
                        0, 0, 0, false, 0, &[],
                        is_stale,
                        stale_reason_code.clone(),
                        if is_stale { Some("stale") } else { None },
                        None,
                    ),
                    "contract": symbol_contract_json(&params.symbol_id),
                    "symbol_id": params.symbol_id
                });
                attach_graph_generation_metadata(
                    &mut response,
                    &serving,
                    state,
                    symbol_project_id.as_deref(),
                )
                .await;
                if let Some(degradation) = super::get_degradation_info(state).await {
                    response["_indexing"] = degradation;
                }
                Ok(success_json(response))
            }
            Err(e) => Ok(error_response(e)),
        },
        "callees" => match state
            .storage
            .get_symbol_callees(&params.symbol_id, graph_generation)
            .await
        {
            Ok(mut callees) => {
                strip_symbol_embeddings(&mut callees);
                let result_values =
                    attach_symbol_freshness(state, &callees, graph_generation, indexing_generation)
                        .await;
                let mut response = json!({
                    "results": result_values,
                    "count": callees.len(),
                    "summary": graph_summary_with_partial(
                        0, 0, 0, false, 0, &[],
                        is_stale,
                        stale_reason_code.clone(),
                        if is_stale { Some("stale") } else { None },
                        None,
                    ),
                    "contract": symbol_contract_json(&params.symbol_id),
                    "symbol_id": params.symbol_id
                });
                attach_graph_generation_metadata(
                    &mut response,
                    &serving,
                    state,
                    symbol_project_id.as_deref(),
                )
                .await;
                if let Some(degradation) = super::get_degradation_info(state).await {
                    response["_indexing"] = degradation;
                }
                Ok(success_json(response))
            }
            Err(e) => Ok(error_response(e)),
        },
        "related" => {
            use crate::types::Direction;

            let depth = params.depth.unwrap_or(1).min(5);
            let direction: Direction = params
                .direction
                .as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default();

            match state
                .storage
                .get_related_symbols(&params.symbol_id, depth, direction, graph_generation)
                .await
            {
                Ok(result) => {
                    let (mut symbols, relations) = result;
                    strip_symbol_embeddings(&mut symbols);
                    if symbols.is_empty() && is_stale {
                        if let Some(project_id) = symbol_project_id.as_deref() {
                            symbols = state
                                .storage
                                .get_project_symbols(project_id, graph_generation)
                                .await
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|symbol| {
                                    symbol_id(symbol).as_deref() == Some(params.symbol_id.as_str())
                                })
                                .collect();
                        }
                    }
                    let frontier: Vec<String> = symbols.iter().filter_map(symbol_id).collect();
                    let nodes = attach_node_freshness(
                        state,
                        &symbols,
                        exported_symbol_nodes(&symbols)
                            .into_iter()
                            .map(|node| serde_json::to_value(node).unwrap_or_else(|_| json!({})))
                            .collect(),
                        graph_generation,
                        indexing_generation,
                    )
                    .await;
                    let edges = attach_edge_freshness(
                        exported_symbol_edges(&relations)
                            .into_iter()
                            .map(|edge| serde_json::to_value(edge).unwrap_or_else(|_| json!({})))
                            .collect(),
                        graph_generation,
                    );
                    let symbol_values = attach_symbol_freshness(
                        state,
                        &symbols,
                        graph_generation,
                        indexing_generation,
                    )
                    .await;
                    let relation_values: Vec<Value> = relations
                        .iter()
                        .map(|relation| {
                            let mut value = serde_json::to_value(relation).unwrap_or_else(|_| json!({}));
                            value["freshness"] = json!({
                                "state": if Some(relation.freshness_generation) == graph_generation { "fresh" } else { "stale" },
                                "generation": relation.freshness_generation,
                                "serving_generation": graph_generation,
                                "evidence": "relation_generation",
                            });
                            value
                        })
                        .collect();
                    let mut response = json!({
                        "summary": graph_summary_with_partial(
                            nodes.len(),
                            edges.len(),
                            depth,
                            false,
                            frontier.len(),
                            &frontier,
                            is_stale,
                            stale_reason_code.clone(),
                            if is_stale { Some("stale") } else { None },
                            None,
                        ),
                        "nodes": nodes,
                        "edges": edges,
                        "results": symbol_values.clone(),
                        "symbols": symbol_values,
                        "relations": relation_values,
                        "contract": symbol_contract_json(&params.symbol_id),
                        "symbol_count": symbols.len(),
                        "relation_count": relations.len(),
                        "depth_reached": depth,
                        "truncated": false,
                        "deferred_count": frontier.len(),
                        "frontier": frontier
                    });
                    attach_graph_generation_metadata(
                        &mut response,
                        &serving,
                        state,
                        symbol_project_id.as_deref(),
                    )
                    .await;
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
