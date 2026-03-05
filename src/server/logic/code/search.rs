use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::graph::{
    rrf_merge, run_ppr, DEFAULT_CODE_BM25_WEIGHT, DEFAULT_CODE_PPR_WEIGHT,
    DEFAULT_CODE_VECTOR_WEIGHT,
};
use crate::server::params::{RecallCodeParams, SearchCodeParams};
use crate::storage::StorageBackend;

use super::super::{normalize_limit, success_json};

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

    // Run vector search and BM25 in parallel for robust results.
    // Previously BM25 was only a fallback — degenerate vectors masked BM25 entirely.
    let project_id = params.project_id.as_deref();
    let (vector_results, bm25_results) = tokio::join!(
        async {
            match state
                .storage
                .vector_search_code(&query_embedding, project_id, limit)
                .await
            {
                Ok(results) => {
                    tracing::debug!(hits = results.len(), "search_code: vector search completed");
                    results
                }
                Err(e) => {
                    tracing::warn!(error = %e, "search_code: vector search failed, falling back to empty");
                    Vec::new()
                }
            }
        },
        async {
            state
                .code_search
                .search(&params.query, project_id, limit, state.storage.as_ref())
                .await
        }
    );

    // Merge: vector results first, then BM25 results not already present.
    // This gives vector priority in ranking while ensuring BM25 fills gaps.
    use std::collections::HashSet;
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut merged = Vec::with_capacity(limit);

    for r in &vector_results {
        if seen_ids.insert(r.id.clone()) {
            merged.push(json!({
                "id": r.id,
                "file_path": r.file_path,
                "content": r.content,
                "language": r.language,
                "start_line": r.start_line,
                "end_line": r.end_line,
                "chunk_type": r.chunk_type,
                "name": r.name,
                "context_path": r.context_path,
                "score": r.score,
                "source": "vector"
            }));
        }
        if merged.len() >= limit {
            break;
        }
    }

    for r in &bm25_results {
        if merged.len() >= limit {
            break;
        }
        if seen_ids.insert(r.id.clone()) {
            merged.push(json!({
                "id": r.id,
                "file_path": r.file_path,
                "content": r.content,
                "language": r.language,
                "start_line": r.start_line,
                "end_line": r.end_line,
                "chunk_type": r.chunk_type,
                "name": r.name,
                "context_path": r.context_path,
                "score": r.score,
                "source": "bm25"
            }));
        }
    }

    Ok(success_json(json!({
        "results": merged,
        "count": merged.len(),
        "query": params.query,
        "vector_hits": vector_results.len(),
        "bm25_hits": bm25_results.len(),
        "is_partial": is_partial,
        "message": indexing_message
    })))
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

    let vector_weight = params.vector_weight.unwrap_or(DEFAULT_CODE_VECTOR_WEIGHT);
    let bm25_weight = params.bm25_weight.unwrap_or(DEFAULT_CODE_BM25_WEIGHT);
    let ppr_weight = params.ppr_weight.unwrap_or(DEFAULT_CODE_PPR_WEIGHT);

    let project_id = params.project_id.as_deref();

    // ── Pre-filter configuration ───────────────────────────────────────────
    // Each channel is filtered independently BEFORE RRF merge so that
    // irrelevant results don't occupy rank slots and dilute precision.
    let path_prefix = params.path_prefix.as_deref();
    let language_filter = params.language.as_deref();
    let chunk_type_filter = params.chunk_type.as_deref();
    let has_filters =
        path_prefix.is_some() || language_filter.is_some() || chunk_type_filter.is_some();

    let matches_filters = |chunk: &crate::types::ScoredCodeChunk| -> bool {
        if let Some(prefix) = path_prefix {
            if !chunk.file_path.starts_with(prefix) {
                return false;
            }
        }
        if let Some(lang) = language_filter {
            // Language enum uses serde rename_all = "lowercase"
            let chunk_lang = serde_json::to_string(&chunk.language)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            if !chunk_lang.eq_ignore_ascii_case(lang) {
                return false;
            }
        }
        if let Some(ct) = chunk_type_filter {
            let chunk_ct = serde_json::to_string(&chunk.chunk_type)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            if !chunk_ct.eq_ignore_ascii_case(ct) {
                return false;
            }
        }
        true
    };

    // When filters are active, over-fetch to compensate for post-filter attrition
    let fetch_limit = if has_filters { limit * 6 } else { limit * 3 };

    // 1. Vector search on code_chunks
    let vector_results: Vec<_> = match state
        .storage
        .vector_search_code(&query_embedding, project_id, fetch_limit)
        .await
    {
        Ok(results) => {
            tracing::debug!(hits = results.len(), "recall_code: vector search completed");
            results
        }
        Err(e) => {
            tracing::warn!(error = %e, "recall_code: vector search failed, falling back to empty");
            Vec::new()
        }
    }
    .into_iter()
    .filter(|r| matches_filters(r))
    .collect();

    // 2. BM25 search via in-memory engine (replaces DB-based CONTAINS fallback)
    let bm25_results: Vec<_> = state
        .code_search
        .search(
            &params.query,
            project_id,
            fetch_limit,
            state.storage.as_ref(),
        )
        .await
        .into_iter()
        .filter(|r| matches_filters(r))
        .collect();

    let vector_tuples: Vec<_> = vector_results
        .iter()
        .map(|r| (r.id.clone(), r.score))
        .collect();
    let bm25_tuples: Vec<_> = bm25_results
        .iter()
        .map(|r| (r.id.clone(), r.score))
        .collect();

    // 3. Graph component: find related symbols → PPR
    // (removed: _all_chunk_ids — HashSet+Vec was built but never read)

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

                    // Pre-build O(1) lookup: symbol ID string → &CodeSymbol
                    // Avoids the O(n²) `.find()` scan in the PPR mapping loop below.
                    let mut sym_by_id: HashMap<String, &crate::types::CodeSymbol> = HashMap::new();

                    for sym in &symbols {
                        if let Some(ref id) = sym.id {
                            let id_str = format!(
                                "{}:{}",
                                id.table.as_str(),
                                crate::types::record_key_to_string(&id.key)
                            );
                            let idx = graph.add_node(id_str.clone());
                            node_map.insert(id_str.clone(), idx);
                            sym_by_id.insert(id_str, sym);
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
                        // Run shared PPR + hub-dampening kernel
                        let ppr_scores = run_ppr(&graph, &seed_nodes);

                        // ── Chunk-level PPR mapping ────────────────────────────
                        // Instead of collapsing PPR scores to file_path (lossy),
                        // map each symbol's PPR score to chunks whose line range
                        // overlaps the symbol's line range within the same file.
                        let reverse_map: HashMap<NodeIndex, String> = node_map
                            .iter()
                            .map(|(id, idx)| (*idx, id.clone()))
                            .collect();

                        // Collect (file_path, start_line, end_line, ppr_score) for each symbol.
                        // O(1) lookup via pre-built HashMap instead of O(n) .find() scan.
                        struct SymbolPpr {
                            file_path: String,
                            start_line: u32,
                            end_line: u32,
                            score: f32,
                        }
                        let mut symbol_pprs: Vec<SymbolPpr> = Vec::new();
                        for (idx, score) in &ppr_scores {
                            if let Some(sym_id) = reverse_map.get(idx) {
                                if let Some(sym) = sym_by_id.get(sym_id) {
                                    symbol_pprs.push(SymbolPpr {
                                        file_path: sym.file_path.clone(),
                                        start_line: sym.start_line,
                                        end_line: sym.end_line,
                                        score: *score,
                                    });
                                }
                            }
                        }

                        // Map symbol PPR scores to chunks by line-range overlap.
                        // A chunk overlaps a symbol when they share the same file
                        // AND their line ranges intersect:
                        //   chunk.start_line <= sym.end_line && sym.start_line <= chunk.end_line
                        let mut tuples: Vec<(String, f32)> = Vec::new();
                        for chunk in vector_results.iter().chain(bm25_results.iter()) {
                            let mut best_score: f32 = 0.0;
                            for sp in &symbol_pprs {
                                if sp.file_path == chunk.file_path
                                    && chunk.start_line <= sp.end_line
                                    && sp.start_line <= chunk.end_line
                                    && sp.score > best_score
                                {
                                    best_score = sp.score;
                                }
                            }
                            if best_score > 0.0 {
                                tuples.push((chunk.id.clone(), best_score));
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
                    "context_path": chunk.context_path,
                    "score": scores.combined_score,
                    "vector_score": scores.vector_score,
                    "bm25_score": scores.bm25_score,
                    "ppr_score": scores.ppr_score,
                })
            })
        })
        .collect();

    let mut response = json!({
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
    });

    if let Some(degradation) = super::get_degradation_info(state).await {
        response["_indexing"] = degradation;
    }

    Ok(success_json(response))
}
