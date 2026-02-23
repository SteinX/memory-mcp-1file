use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::graph::{
    apply_hub_dampening, personalized_page_rank, rrf_merge, DEFAULT_CODE_BM25_WEIGHT,
    DEFAULT_CODE_PPR_WEIGHT, DEFAULT_CODE_VECTOR_WEIGHT, PPR_DAMPING, PPR_MAX_ITER, PPR_TOLERANCE,
};
use crate::server::params::{
    DeleteProjectParams, GetCalleesParams, GetCallersParams, GetIndexStatusParams,
    GetProjectStatsParams, IndexProjectParams, ListProjectsParams, RecallCodeParams,
    SearchCodeParams, SearchSymbolsParams,
};
use crate::storage::StorageBackend;

use super::{error_response, normalize_limit, strip_symbol_embeddings, success_json};

pub async fn index_project(
    state: &Arc<AppState>,
    params: IndexProjectParams,
) -> anyhow::Result<CallToolResult> {
    let path = std::path::Path::new(&params.path);

    if !path.exists() {
        return Ok(error_response(format!(
            "Path does not exist: {}",
            params.path
        )));
    }

    let project_id = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let force = params.force.unwrap_or(false);

    // Check current status
    if let Ok(Some(status)) = state.storage.get_index_status(&project_id).await {
        match status.status {
            crate::types::IndexState::Indexing => {
                // Already indexing - return current progress
                return Ok(success_json(json!({
                    "project_id": project_id,
                    "status": "indexing",
                    "total_files": status.total_files,
                    "indexed_files": status.indexed_files,
                    "total_chunks": status.total_chunks,
                    "message": "Indexing already in progress"
                })));
            }
            crate::types::IndexState::Completed | crate::types::IndexState::EmbeddingPending => {
                if !force {
                    return Ok(success_json(json!({
                        "project_id": project_id,
                        "status": status.status.to_string(),
                        "total_files": status.total_files,
                        "indexed_files": status.indexed_files,
                        "total_chunks": status.total_chunks,
                        "message": "Project already indexed. File changes are tracked incrementally. Use force=true to re-index from scratch."
                    })));
                }
                tracing::info!(project_id = %project_id, "Force re-indexing project");
            }
            crate::types::IndexState::Failed => {
                tracing::info!(
                    project_id = %project_id,
                    error = ?status.error_message,
                    "Previous indexing failed, re-indexing"
                );
            }
        }
    }

    // Spawn indexing in background
    let state_clone = state.clone();
    let path_clone = params.path.clone();

    tokio::spawn(async move {
        let path = std::path::Path::new(&path_clone);
        match crate::codebase::index_project(state_clone, path).await {
            Ok(status) => {
                tracing::info!(
                    project_id = %status.project_id,
                    files = status.indexed_files,
                    chunks = status.total_chunks,
                    "Indexing completed"
                );
            }
            Err(e) => {
                tracing::error!("Indexing failed: {}", e);
            }
        }
    });

    // Return immediately
    Ok(success_json(json!({
        "project_id": project_id,
        "status": "indexing",
        "message": "Indexing started in background. Use get_index_status to check progress."
    })))
}

pub async fn search_code(
    state: &Arc<AppState>,
    params: SearchCodeParams,
) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);

    let mut is_partial = false;
    let mut indexing_message = None;

    if let Some(ref project_id) = params.project_id {
        if let Ok(Some(status)) = state.storage.get_index_status(project_id).await {
            if status.status == crate::types::IndexState::Indexing
                || status.status == crate::types::IndexState::EmbeddingPending
            {
                is_partial = true;
                indexing_message = Some(format!(
                    "Indexing in progress ({}/{} files). Results may be incomplete.",
                    status.indexed_files, status.total_files
                ));
            }
        }
    }

    let query_embedding = state.embedding.embed(&params.query).await?;

    let limit = normalize_limit(params.limit);
    let results = state
        .storage
        .vector_search_code(&query_embedding, params.project_id.as_deref(), limit)
        .await
        .unwrap_or_default();

    if !results.is_empty() {
        return Ok(success_json(json!({
            "results": results,
            "count": results.len(),
            "query": params.query,
            "is_partial": is_partial,
            "message": indexing_message
        })));
    }

    match state
        .storage
        .bm25_search_code(&params.query, params.project_id.as_deref(), limit)
        .await
    {
        Ok(fallback) => Ok(success_json(json!({
            "results": fallback,
            "count": fallback.len(),
            "query": params.query,
            "note": "fallback to text search",
            "is_partial": is_partial,
            "message": indexing_message
        }))),
        Err(e) => Ok(error_response(e)),
    }
}

/// Hybrid code search: Vector + BM25 + Symbol Graph PageRank → RRF merge
pub async fn recall_code(
    state: &Arc<AppState>,
    params: RecallCodeParams,
) -> anyhow::Result<CallToolResult> {
    use petgraph::graph::{DiGraph, NodeIndex};
    use std::collections::HashMap;

    crate::ensure_embedding_ready!(state);

    let mut is_partial = false;
    let mut indexing_message = None;

    if let Some(ref project_id) = params.project_id {
        if let Ok(Some(status)) = state.storage.get_index_status(project_id).await {
            if status.status == crate::types::IndexState::Indexing
                || status.status == crate::types::IndexState::EmbeddingPending
            {
                is_partial = true;
                indexing_message = Some(format!(
                    "Indexing in progress ({}/{} files). Results may be incomplete.",
                    status.indexed_files, status.total_files
                ));
            }
        }
    }

    let query_embedding = state.embedding.embed(&params.query).await?;

    let limit = normalize_limit(params.limit);
    let fetch_limit = limit * 3;

    let vector_weight = params.vector_weight.unwrap_or(DEFAULT_CODE_VECTOR_WEIGHT);
    let bm25_weight = params.bm25_weight.unwrap_or(DEFAULT_CODE_BM25_WEIGHT);
    let ppr_weight = params.ppr_weight.unwrap_or(DEFAULT_CODE_PPR_WEIGHT);

    let project_id = params.project_id.as_deref();

    // 1. Vector search on code_chunks
    let vector_results = state
        .storage
        .vector_search_code(&query_embedding, project_id, fetch_limit)
        .await
        .unwrap_or_default();

    // 2. BM25 (CONTAINS fallback) search on code_chunks
    let bm25_results = state
        .storage
        .bm25_search_code(&params.query, project_id, fetch_limit)
        .await
        .unwrap_or_default();

    let vector_tuples: Vec<_> = vector_results
        .iter()
        .map(|r| (r.id.clone(), r.score))
        .collect();
    let bm25_tuples: Vec<_> = bm25_results
        .iter()
        .map(|r| (r.id.clone(), r.score))
        .collect();

    // 3. Graph component: find related symbols → PPR
    let _all_chunk_ids: Vec<String> = vector_results
        .iter()
        .chain(bm25_results.iter())
        .map(|r| r.id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let ppr_tuples: Vec<(String, f32)> = if ppr_weight > 0.0 {
        // Find semantically similar symbols via vector search
        let seed_symbols = state
            .storage
            .vector_search_symbols(&query_embedding, project_id, 20)
            .await
            .unwrap_or_default();

        let symbol_ids: Vec<String> = seed_symbols
            .iter()
            .filter_map(|s| {
                s.id.as_ref().map(|id| {
                    format!(
                        "{}:{}",
                        id.table.as_str(),
                        crate::types::record_key_to_string(&id.key)
                    )
                })
            })
            .collect();

        if !symbol_ids.is_empty() {
            match state.storage.get_code_subgraph(&symbol_ids).await {
                Ok((symbols, relations)) if !symbols.is_empty() => {
                    let mut graph: DiGraph<String, f32> = DiGraph::new();
                    let mut node_map: HashMap<String, NodeIndex> = HashMap::new();
                    // Map: symbol file_path → symbol node ID (for chunk→symbol mapping)
                    let mut file_to_symbols: HashMap<String, Vec<String>> = HashMap::new();

                    for sym in &symbols {
                        if let Some(ref id) = sym.id {
                            let id_str = format!(
                                "{}:{}",
                                id.table.as_str(),
                                crate::types::record_key_to_string(&id.key)
                            );
                            let idx = graph.add_node(id_str.clone());
                            node_map.insert(id_str.clone(), idx);
                            file_to_symbols
                                .entry(sym.file_path.clone())
                                .or_default()
                                .push(id_str);
                        }
                    }

                    for rel in &relations {
                        let from_str = format!(
                            "{}:{}",
                            rel.from_symbol.table.as_str(),
                            crate::types::record_key_to_string(&rel.from_symbol.key)
                        );
                        let to_str = format!(
                            "{}:{}",
                            rel.to_symbol.table.as_str(),
                            crate::types::record_key_to_string(&rel.to_symbol.key)
                        );
                        if let (Some(&from_idx), Some(&to_idx)) =
                            (node_map.get(&from_str), node_map.get(&to_str))
                        {
                            graph.add_edge(from_idx, to_idx, 1.0);
                        }
                    }

                    // Seed PPR with the vector-matched symbols
                    let seed_nodes: Vec<NodeIndex> = symbol_ids
                        .iter()
                        .filter_map(|id| node_map.get(id).copied())
                        .collect();

                    if !seed_nodes.is_empty() && graph.node_count() > 0 {
                        let mut ppr_scores = personalized_page_rank(
                            &graph,
                            &seed_nodes,
                            PPR_DAMPING,
                            PPR_TOLERANCE,
                            PPR_MAX_ITER,
                        );

                        let degrees: HashMap<NodeIndex, usize> = graph
                            .node_indices()
                            .map(|idx| (idx, graph.edges(idx).count()))
                            .collect();
                        apply_hub_dampening(&mut ppr_scores, &degrees);

                        // Map symbol PPR scores → chunk IDs by file_path
                        let reverse_map: HashMap<NodeIndex, String> = node_map
                            .iter()
                            .map(|(id, idx)| (*idx, id.clone()))
                            .collect();

                        // Build file_path → max PPR score
                        let mut file_scores: HashMap<String, f32> = HashMap::new();
                        for (idx, score) in &ppr_scores {
                            if let Some(sym_id) = reverse_map.get(idx) {
                                if let Some(sym) = symbols.iter().find(|s| {
                                    s.id.as_ref().map(|id| {
                                        format!(
                                            "{}:{}",
                                            id.table.as_str(),
                                            crate::types::record_key_to_string(&id.key)
                                        )
                                    }) == Some(sym_id.clone())
                                }) {
                                    let entry =
                                        file_scores.entry(sym.file_path.clone()).or_insert(0.0);
                                    if *score > *entry {
                                        *entry = *score;
                                    }
                                }
                            }
                        }

                        // Map file PPR scores to chunk IDs
                        let mut tuples: Vec<(String, f32)> = Vec::new();
                        for chunk in vector_results.iter().chain(bm25_results.iter()) {
                            if let Some(&score) = file_scores.get(&chunk.file_path) {
                                tuples.push((chunk.id.clone(), score));
                            }
                        }
                        tuples.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
                        tuples.dedup_by(|a, b| a.0 == b.0);
                        tuples
                    } else {
                        vec![]
                    }
                }
                _ => vec![],
            }
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    // 4. RRF merge
    let merged = rrf_merge(
        &vector_tuples,
        &bm25_tuples,
        &ppr_tuples,
        vector_weight,
        bm25_weight,
        ppr_weight,
        limit,
    );

    // 5. Build response with score breakdown
    let mut content_map: HashMap<String, &crate::types::ScoredCodeChunk> = HashMap::new();
    for r in &vector_results {
        content_map.insert(r.id.clone(), r);
    }
    for r in &bm25_results {
        content_map.entry(r.id.clone()).or_insert(r);
    }

    let results: Vec<serde_json::Value> = merged
        .into_iter()
        .filter_map(|(id, scores)| {
            content_map.get(&id).map(|chunk| {
                json!({
                    "id": id,
                    "file_path": chunk.file_path,
                    "content": chunk.content,
                    "language": chunk.language,
                    "start_line": chunk.start_line,
                    "end_line": chunk.end_line,
                    "chunk_type": chunk.chunk_type,
                    "name": chunk.name,
                    "score": scores.combined_score,
                    "vector_score": scores.vector_score,
                    "bm25_score": scores.bm25_score,
                    "ppr_score": scores.ppr_score,
                })
            })
        })
        .collect();

    Ok(success_json(json!({
        "results": results,
        "count": results.len(),
        "query": params.query,
        "weights": {
            "vector": vector_weight,
            "bm25": bm25_weight,
            "ppr": ppr_weight
        },
        "is_partial": is_partial,
        "message": indexing_message
    })))
}

pub async fn get_index_status(
    state: &Arc<AppState>,
    params: GetIndexStatusParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.get_index_status(&params.project_id).await {
        Ok(Some(mut status)) => {
            let mut current_file: Option<String> = None;
            if status.status == crate::types::IndexState::Indexing {
                if let Some(monitor) = state.progress.get(&params.project_id).await {
                    let indexed = monitor
                        .indexed_files
                        .load(std::sync::atomic::Ordering::Relaxed);
                    let total = monitor
                        .total_files
                        .load(std::sync::atomic::Ordering::Relaxed);

                    if indexed > 0 {
                        status.indexed_files = std::cmp::max(status.indexed_files, indexed);
                    }
                    if total > 0 {
                        status.total_files = std::cmp::max(status.total_files, total);
                    }

                    if let Ok(cf) = monitor.current_file.read() {
                        if !cf.is_empty() {
                            current_file = Some(cf.clone());
                        }
                    }
                }
            }

            let total_symbols = state
                .storage
                .count_symbols(&params.project_id)
                .await
                .unwrap_or(0);
            let total_chunks = state
                .storage
                .count_chunks(&params.project_id)
                .await
                .unwrap_or(0);
            let embedded_symbols = state
                .storage
                .count_embedded_symbols(&params.project_id)
                .await
                .unwrap_or(0);
            let embedded_chunks = state
                .storage
                .count_embedded_chunks(&params.project_id)
                .await
                .unwrap_or(0);

            let vector_progress = if total_chunks > 0 {
                (embedded_chunks as f32 / total_chunks as f32) * 100.0
            } else {
                0.0
            };
            let graph_progress = if total_symbols > 0 {
                (embedded_symbols as f32 / total_symbols as f32) * 100.0
            } else {
                0.0
            };
            let overall_progress = if (total_chunks + total_symbols) > 0 {
                ((embedded_chunks + embedded_symbols) as f32
                    / (total_chunks + total_symbols) as f32)
                    * 100.0
            } else {
                0.0
            };

            Ok(success_json(json!({
                "project_id": status.project_id,
                "status": status.status.to_string(),
                "total_files": status.total_files,
                "indexed_files": status.indexed_files,
                "started_at": status.started_at,
                "completed_at": status.completed_at,

                "parsing": {
                    "status": if status.indexed_files >= status.total_files { "completed" } else { "in_progress" },
                    "progress": format!("{}/{}", status.indexed_files, status.total_files),
                    "current_file": current_file
                },

                "vector_embeddings": {
                    "status": if status.status == crate::types::IndexState::Completed || (embedded_chunks >= total_chunks && total_chunks > 0) { "completed" } else { "in_progress" },
                    "total": total_chunks,
                    "completed": embedded_chunks,
                    "percent": format!("{:.1}", vector_progress)
                },

                "graph_embeddings": {
                    "status": if status.status == crate::types::IndexState::Completed || (embedded_symbols >= total_symbols && total_symbols > 0) { "completed" } else { "in_progress" },
                    "total": total_symbols,
                    "completed": embedded_symbols,
                    "percent": format!("{:.1}", graph_progress)
                },

                "overall_progress": {
                    "percent": format!("{:.1}", overall_progress),
                    "is_complete": status.status == crate::types::IndexState::Completed || (embedded_chunks >= total_chunks && embedded_symbols >= total_symbols && total_chunks > 0)
                }
            })))
        }
        Ok(None) => Ok(error_response(format!(
            "Project not found: {}",
            params.project_id
        ))),
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn list_projects(
    state: &Arc<AppState>,
    _params: ListProjectsParams,
) -> anyhow::Result<CallToolResult> {
    match state.storage.list_projects().await {
        Ok(projects) => {
            let mut enriched = Vec::with_capacity(projects.len());

            for project_id in &projects {
                let status = state
                    .storage
                    .get_index_status(project_id)
                    .await
                    .ok()
                    .flatten();
                let chunks = state.storage.count_chunks(project_id).await.unwrap_or(0);
                let symbols = state.storage.count_symbols(project_id).await.unwrap_or(0);
                let embedded_chunks = state
                    .storage
                    .count_embedded_chunks(project_id)
                    .await
                    .unwrap_or(0);
                let embedded_symbols = state
                    .storage
                    .count_embedded_symbols(project_id)
                    .await
                    .unwrap_or(0);

                let status_str = status
                    .as_ref()
                    .map(|s| s.status.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                enriched.push(json!({
                    "id": project_id,
                    "status": status_str,
                    "chunks": chunks,
                    "symbols": symbols,
                    "embedded_chunks": embedded_chunks,
                    "embedded_symbols": embedded_symbols
                }));
            }

            Ok(success_json(json!({
                "projects": enriched,
                "count": projects.len()
            })))
        }
        Err(e) => Ok(error_response(e)),
    }
}

pub async fn delete_project(
    state: &Arc<AppState>,
    params: DeleteProjectParams,
) -> anyhow::Result<CallToolResult> {
    let _ = state
        .storage
        .delete_project_symbols(&params.project_id)
        .await;

    let _ = state.storage.delete_index_status(&params.project_id).await;
    let _ = state.storage.delete_file_hashes(&params.project_id).await;

    match state
        .storage
        .delete_project_chunks(&params.project_id)
        .await
    {
        Ok(deleted) => Ok(success_json(json!({
            "deleted_chunks": deleted,
            "project_id": params.project_id
        }))),
        Err(e) => Ok(error_response(e)),
    }
}

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

pub async fn get_project_stats(
    state: &Arc<AppState>,
    params: GetProjectStatsParams,
) -> anyhow::Result<CallToolResult> {
    let status = state.storage.get_index_status(&params.project_id).await?;

    if status.is_none() {
        return Ok(error_response(format!(
            "Project not found: {}",
            params.project_id
        )));
    }

    let status = status.unwrap();

    let total_symbols = state
        .storage
        .count_symbols(&params.project_id)
        .await
        .unwrap_or(0);
    let total_chunks = state
        .storage
        .count_chunks(&params.project_id)
        .await
        .unwrap_or(0);
    let embedded_symbols = state
        .storage
        .count_embedded_symbols(&params.project_id)
        .await
        .unwrap_or(0);
    let embedded_chunks = state
        .storage
        .count_embedded_chunks(&params.project_id)
        .await
        .unwrap_or(0);

    let vector_progress = if total_chunks > 0 {
        (embedded_chunks as f32 / total_chunks as f32) * 100.0
    } else {
        0.0
    };
    let graph_progress = if total_symbols > 0 {
        (embedded_symbols as f32 / total_symbols as f32) * 100.0
    } else {
        0.0
    };

    Ok(success_json(json!({
        "project_id": params.project_id,
        "status": status.status.to_string(),
        "files": {
            "total": status.total_files,
            "indexed": status.indexed_files
        },
        "chunks": {
            "total": total_chunks,
            "embedded": embedded_chunks,
            "progress_percent": format!("{:.1}", vector_progress)
        },
        "symbols": {
            "total": total_symbols,
            "embedded": embedded_symbols,
            "progress_percent": format!("{:.1}", graph_progress)
        },
        "started_at": status.started_at,
        "completed_at": status.completed_at,
        "failed_files": status.failed_files
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;
    use std::fs;

    #[tokio::test]
    async fn test_code_logic_flow() {
        let ctx = TestContext::new().await;
        let unique_id = format!("test_project_{}", uuid::Uuid::new_v4().simple());
        let project_path = ctx._temp_dir.path().join(&unique_id);
        fs::create_dir_all(&project_path).unwrap();
        fs::write(
            project_path.join("main.rs"),
            "fn main() { println!(\"Hello\"); }",
        )
        .unwrap();

        let index_params = IndexProjectParams {
            path: project_path.to_string_lossy().to_string(),
            force: None,
        };

        // 1. Trigger Indexing
        let result = index_project(&ctx.state, index_params).await.unwrap();
        // Should return "indexing" status immediately
        if let rmcp::model::RawContent::Text(t) = &result.content[0].raw {
            assert!(t.text.contains("indexing"));
        } else {
            panic!("Expected text content");
        }

        // 2. Wait for indexing to complete
        // Since it's a background task, we poll get_index_status
        let status_params = GetIndexStatusParams {
            project_id: unique_id.clone(),
        };

        let mut retries = 0;
        let mut last_status = String::new();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let res = get_index_status(&ctx.state, status_params.clone())
                .await
                .unwrap();
            if let rmcp::model::RawContent::Text(t) = &res.content[0].raw {
                last_status = t.text.clone();
                let wait_for_full_completion = t.text.contains("\"status\":\"completed\"");
                if wait_for_full_completion {
                    break;
                }
            }
            retries += 1;
            if retries > 100 {
                panic!("Indexing timed out. Last status: {}", last_status);
            }
        }

        // 3. Search Code
        let search_params = SearchCodeParams {
            query: "Hello".to_string(),
            project_id: Some(unique_id.clone()),
            limit: Some(5),
        };
        let search_res = search_code(&ctx.state, search_params).await.unwrap();

        if let rmcp::model::RawContent::Text(t) = &search_res.content[0].raw {
            assert!(
                t.text.contains("main.rs"),
                "Expected 'main.rs' in search results. Got: {}",
                &t.text[..std::cmp::min(500, t.text.len())]
            );
            assert!(t.text.contains("Hello"));
        } else {
            panic!("Expected text content");
        }
    }
}
