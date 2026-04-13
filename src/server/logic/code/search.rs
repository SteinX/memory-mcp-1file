use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::graph::{
    rrf_merge, run_ppr, DEFAULT_CODE_BM25_WEIGHT, DEFAULT_CODE_PPR_WEIGHT,
    DEFAULT_CODE_VECTOR_WEIGHT,
};
use crate::server::params::{normalize_project_id, RecallCodeParams, SearchCodeParams};
use crate::storage::StorageBackend;

use super::super::{normalize_limit, success_json};

fn split_identifier_tokens(input: &str) -> Vec<String> {
    fn class(c: char) -> u8 {
        if c.is_ascii_lowercase() {
            1
        } else if c.is_ascii_uppercase() {
            2
        } else if c.is_ascii_digit() {
            3
        } else {
            0
        }
    }

    let mut out = Vec::new();
    for word in input.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let lower = word.to_lowercase();
        out.push(lower.clone());

        let chars: Vec<char> = word.chars().collect();
        if chars.len() <= 1 {
            continue;
        }

        let mut start = 0usize;
        for i in 1..chars.len() {
            let prev = class(chars[i - 1]);
            let cur = class(chars[i]);
            let next = if i + 1 < chars.len() {
                class(chars[i + 1])
            } else {
                0
            };
            let split =
                matches!((prev, cur), (1, 2) | (3, 1) | (3, 2)) || (prev, cur, next) == (2, 2, 1);
            if split {
                let seg: String = chars[start..i].iter().collect();
                let seg = seg.to_lowercase();
                if !seg.is_empty() {
                    out.push(seg);
                }
                start = i;
            }
        }
        let seg: String = chars[start..].iter().collect();
        let seg = seg.to_lowercase();
        if !seg.is_empty() {
            out.push(seg);
        }
    }

    out.sort();
    out.dedup();
    out
}

fn is_codeish_query(query: &str, terms: &[String]) -> bool {
    query.contains('_')
        || query.contains("::")
        || query.contains('/')
        || query.contains('.')
        || query
            .chars()
            .zip(query.chars().skip(1))
            .any(|(a, b)| a.is_ascii_lowercase() && b.is_ascii_uppercase())
        || terms.iter().any(|t| {
            matches!(
                t.as_str(),
                "fn" | "impl" | "struct" | "trait" | "class" | "method"
            )
        })
}

fn lexical_rerank_score(
    chunk: &crate::types::ScoredCodeChunk,
    query_lower: &str,
    terms: &[String],
    codeish: bool,
) -> f32 {
    let path = chunk.file_path.to_lowercase();
    let name = chunk.name.clone().unwrap_or_default().to_lowercase();
    let ctx = chunk
        .context_path
        .clone()
        .unwrap_or_default()
        .to_lowercase();
    let content = chunk.content.to_lowercase();

    let mut raw = 0.0f32;
    let mut matched_terms = 0usize;
    let mut strong_hit = false;

    if query_lower.len() >= 4
        && (path.contains(query_lower) || name.contains(query_lower) || ctx.contains(query_lower))
    {
        raw += 1.8;
        strong_hit = true;
    }

    for term in terms {
        if term.len() < 2 {
            continue;
        }
        let in_name = !name.is_empty() && name.contains(term);
        let in_path = path.contains(term);
        let in_ctx = !ctx.is_empty() && ctx.contains(term);
        let in_content = content.contains(term);

        if in_name {
            raw += 0.7;
            matched_terms += 1;
            strong_hit = true;
        } else if in_path {
            raw += 0.55;
            matched_terms += 1;
            strong_hit = true;
        } else if in_ctx {
            raw += 0.45;
            matched_terms += 1;
        } else if in_content {
            raw += 0.2;
            matched_terms += 1;
        }
    }

    if !terms.is_empty() {
        raw += 0.8 * (matched_terms as f32 / terms.len() as f32);
    }

    if codeish && strong_hit {
        raw += 0.8;
    }

    // Penalize very short generic chunks unless they have strong lexical evidence.
    if chunk.content.len() < 96 && !strong_hit {
        raw -= 0.6;
    }

    raw.clamp(0.0, 4.0) / 4.0
}

fn symbol_exactness_score(
    sym: &crate::types::CodeSymbol,
    query_lower: &str,
    terms: &[String],
) -> f32 {
    let name_lower = sym.name.to_lowercase();
    let sig_lower = sym.signature.clone().unwrap_or_default().to_lowercase();
    let name_tokens = split_identifier_tokens(&sym.name);
    let mut matched_terms = 0usize;
    let mut raw = 0.0f32;

    if name_lower == query_lower {
        raw += 2.2;
    } else if query_lower.len() >= 4 && name_lower.contains(query_lower) {
        raw += 1.6;
    }
    if !sig_lower.is_empty() && query_lower.len() >= 4 && sig_lower.contains(query_lower) {
        raw += 0.8;
    }

    for t in terms {
        if t.len() < 2 {
            continue;
        }
        if name_tokens.iter().any(|nt| nt == t) {
            matched_terms += 1;
            raw += 0.55;
        } else if name_lower.contains(t) || sig_lower.contains(t) {
            matched_terms += 1;
            raw += 0.35;
        }
    }
    if !terms.is_empty() {
        raw += 0.8 * (matched_terms as f32 / terms.len() as f32);
    }

    raw.clamp(0.0, 4.0) / 4.0
}

fn symbol_chunk_overlap_bonus(
    chunk: &crate::types::ScoredCodeChunk,
    sym: &crate::types::CodeSymbol,
) -> f32 {
    if chunk.file_path != sym.file_path {
        return 0.0;
    }

    if chunk.start_line <= sym.end_line && sym.start_line <= chunk.end_line {
        // Exact line-range overlap in the same file
        return 1.0;
    }

    // Fallback: near-line proximity in the same file
    let d1 = chunk.start_line.abs_diff(sym.start_line);
    let d2 = chunk.end_line.abs_diff(sym.start_line);
    let dist = d1.min(d2);
    if dist <= 6 {
        0.65
    } else if dist <= 16 {
        0.35
    } else {
        0.0
    }
}

fn build_symbol_probes(query: &str, terms: &[String]) -> Vec<String> {
    let mut probes = Vec::new();
    let q = query.trim();
    if !q.is_empty() {
        probes.push(q.to_string());
    }

    let mut ranked_terms: Vec<String> = terms.iter().filter(|t| t.len() >= 3).cloned().collect();
    ranked_terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
    ranked_terms.truncate(4);
    probes.extend(ranked_terms);

    probes.sort();
    probes.dedup();
    probes
}

pub async fn search_code(
    state: &Arc<AppState>,
    params: SearchCodeParams,
) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);

    let SearchCodeParams {
        query,
        project_id,
        limit,
    } = params;
    let project_id = normalize_project_id(project_id);

    let mut is_partial = false;
    let mut indexing_message = None;

    if let Some(ref project_id) = project_id {
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

    let query_embedding = state.embedding.embed(&query).await?;

    let limit = normalize_limit(limit);

    // Run vector search and BM25 in parallel for robust results.
    // Previously BM25 was only a fallback — degenerate vectors masked BM25 entirely.
    let project_id = project_id.as_deref();
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
                .search(&query, project_id, limit, state.storage.as_ref())
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
        "query": query,
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
    use std::collections::{HashMap, HashSet};

    crate::ensure_embedding_ready!(state);

    let RecallCodeParams {
        query,
        project_id,
        limit,
        mode: _,
        vector_weight,
        bm25_weight,
        ppr_weight,
        path_prefix,
        language,
        chunk_type,
    } = params;
    let project_id = normalize_project_id(project_id);

    let mut is_partial = false;
    let mut indexing_message = None;

    if let Some(ref project_id) = project_id {
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

    let query_embedding = state.embedding.embed(&query).await?;

    let limit = normalize_limit(limit);

    let vector_weight = vector_weight.unwrap_or(DEFAULT_CODE_VECTOR_WEIGHT);
    let bm25_weight = bm25_weight.unwrap_or(DEFAULT_CODE_BM25_WEIGHT);
    let ppr_weight = ppr_weight.unwrap_or(DEFAULT_CODE_PPR_WEIGHT);

    let project_id = project_id.as_deref();
    let query_lower = query.to_lowercase();
    let query_terms = split_identifier_tokens(&query);
    let codeish_query = is_codeish_query(&query, &query_terms);

    // ── Pre-filter configuration ───────────────────────────────────────────
    // Each channel is filtered independently BEFORE RRF merge so that
    // irrelevant results don't occupy rank slots and dilute precision.
    let path_prefix = path_prefix.as_deref();
    let language_filter = language.as_deref();
    let chunk_type_filter = chunk_type.as_deref();
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

    // Over-fetch and rerank locally to improve exact identifier/path quality.
    // Keep it bounded to avoid pathological memory/time growth.
    let fetch_limit = if has_filters {
        (limit * 10).min(300)
    } else {
        (limit * 8).min(250)
    };

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
    let mut bm25_results: Vec<_> = state
        .code_search
        .search(&query, project_id, fetch_limit, state.storage.as_ref())
        .await
        .into_iter()
        .filter(|r| matches_filters(r))
        .collect();

    // 2.5 Symbol lexical candidates (shared by exact channel and PPR seeding).
    // `search_symbols` is substring-based for a single query string, so we probe
    // both full query and top identifier tokens to avoid missing exact names.
    let symbol_probes = build_symbol_probes(&query, &query_terms);
    let mut seed_symbols_lex = Vec::new();
    let mut seen_symbol_ids = HashSet::new();
    for probe in &symbol_probes {
        if let Ok((symbols, _)) = state
            .storage
            .search_symbols(probe, project_id, 20, 0, None, path_prefix)
            .await
        {
            for s in symbols {
                let key =
                    s.id.as_ref()
                        .map(|id| {
                            format!(
                                "{}:{}",
                                id.table.as_str(),
                                crate::types::record_key_to_string(&id.key)
                            )
                        })
                        .unwrap_or_else(|| {
                            format!(
                                "{}:{}:{}",
                                s.file_path.to_lowercase(),
                                s.name.to_lowercase(),
                                s.start_line
                            )
                        });
                if seen_symbol_ids.insert(key) {
                    seed_symbols_lex.push(s);
                }
            }
        }
    }

    // 2.6 Exact symbol -> chunk channel.
    // For identifier-like queries, map exact symbol name matches to concrete chunk IDs.
    let mut exact_tuples: Vec<(String, f32)> = Vec::new();
    let mut exact_by_chunk: HashMap<String, f32> = HashMap::new();

    if codeish_query && project_id.is_some() {
        let mut exact_symbols: Vec<(f32, crate::types::CodeSymbol)> = seed_symbols_lex
            .iter()
            .cloned()
            .map(|s| (symbol_exactness_score(&s, &query_lower, &query_terms), s))
            .filter(|(score, _)| *score >= 0.45)
            .collect();
        exact_symbols.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        exact_symbols.truncate(20);

        let mut chunk_pool: HashMap<String, crate::types::ScoredCodeChunk> = HashMap::new();
        for c in vector_results.iter().chain(bm25_results.iter()) {
            chunk_pool.entry(c.id.clone()).or_insert_with(|| c.clone());
        }

        // Primary exact channel: symbol->chunk map built during indexing.
        if let Some(pid) = project_id {
            let symbol_keys: Vec<String> = exact_symbols
                .iter()
                .filter_map(|(_, s)| {
                    s.id.as_ref()
                        .map(|id| crate::types::record_key_to_string(&id.key))
                })
                .collect();
            if !symbol_keys.is_empty() {
                if let Ok(mapped) = state
                    .storage
                    .get_mapped_chunks_for_symbols(pid, &symbol_keys, 80)
                    .await
                {
                    let missing_ids: Vec<String> = mapped
                        .iter()
                        .map(|(cid, _)| cid.clone())
                        .filter(|cid| !chunk_pool.contains_key(cid))
                        .collect();
                    if !missing_ids.is_empty() {
                        if let Ok(chunks) = state.storage.get_chunks_by_ids(&missing_ids).await {
                            for chunk in chunks {
                                let Some(id) = chunk
                                    .id
                                    .as_ref()
                                    .map(|t| crate::types::record_key_to_string(&t.key))
                                else {
                                    continue;
                                };
                                let scored = crate::types::ScoredCodeChunk {
                                    id: id.clone(),
                                    file_path: chunk.file_path,
                                    content: chunk.content,
                                    language: chunk.language,
                                    start_line: chunk.start_line,
                                    end_line: chunk.end_line,
                                    chunk_type: chunk.chunk_type,
                                    name: chunk.name,
                                    context_path: chunk.context_path,
                                    score: 0.0,
                                };
                                if matches_filters(&scored) {
                                    chunk_pool.entry(id).or_insert(scored);
                                }
                            }
                        }
                    }

                    for (chunk_id, map_score) in mapped {
                        exact_by_chunk
                            .entry(chunk_id)
                            .and_modify(|s| {
                                if map_score > *s {
                                    *s = map_score;
                                }
                            })
                            .or_insert(map_score);
                    }
                }
            }
        }

        // If we don't have overlapping chunks for exact symbols yet,
        // fetch chunks from symbol files directly and compute overlap locally.
        if let Some(pid) = project_id {
            let mut files_to_fetch: Vec<String> = exact_symbols
                .iter()
                .map(|(_, s)| s.file_path.clone())
                .collect();
            files_to_fetch.sort();
            files_to_fetch.dedup();
            files_to_fetch.truncate(8);

            for file_path in files_to_fetch {
                if let Ok(chunks) = state.storage.get_chunks_by_path(pid, &file_path).await {
                    for chunk in chunks {
                        let Some(id) = chunk
                            .id
                            .as_ref()
                            .map(|t| crate::types::record_key_to_string(&t.key))
                        else {
                            continue;
                        };
                        let scored = crate::types::ScoredCodeChunk {
                            id: id.clone(),
                            file_path: chunk.file_path,
                            content: chunk.content,
                            language: chunk.language,
                            start_line: chunk.start_line,
                            end_line: chunk.end_line,
                            chunk_type: chunk.chunk_type,
                            name: chunk.name,
                            context_path: chunk.context_path,
                            score: 0.0,
                        };
                        if matches_filters(&scored) {
                            chunk_pool.entry(id).or_insert(scored);
                        }
                    }
                }
            }
        }

        for (sym_score, sym) in exact_symbols {
            for chunk in chunk_pool.values() {
                let overlap = symbol_chunk_overlap_bonus(chunk, &sym);
                if overlap <= 0.0 {
                    continue;
                }
                let score = (sym_score * overlap).clamp(0.0, 1.0);
                exact_by_chunk
                    .entry(chunk.id.clone())
                    .and_modify(|s| {
                        if score > *s {
                            *s = score;
                        }
                    })
                    .or_insert(score);
            }
        }

        // Extend retrieval channels with exact-channel chunks if they were not present.
        // This allows exact hits to enter final top-K even when vector/BM25 missed them.
        let known_ids: HashSet<String> = vector_results
            .iter()
            .chain(bm25_results.iter())
            .map(|c| c.id.clone())
            .collect();
        let mut added = 0usize;
        for id in exact_by_chunk.keys() {
            if known_ids.contains(id) || added >= 40 {
                continue;
            }
            if let Some(chunk) = chunk_pool.get(id) {
                bm25_results.push(chunk.clone());
                added += 1;
            }
        }

        exact_tuples = exact_by_chunk
            .iter()
            .map(|(id, s)| (id.clone(), *s))
            .collect();
        exact_tuples.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    }

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
        let seed_symbols_vec = state
            .storage
            .vector_search_symbols(&query_embedding, project_id, 20)
            .await
            .unwrap_or_default();

        let mut symbol_ids: Vec<String> = seed_symbols_vec
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
        symbol_ids.extend(seed_symbols_lex.iter().filter_map(|s| {
            s.id.as_ref().map(|id| {
                format!(
                    "{}:{}",
                    id.table.as_str(),
                    crate::types::record_key_to_string(&id.key)
                )
            })
        }));
        let mut seen = HashSet::new();
        symbol_ids.retain(|id| seen.insert(id.clone()));

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
                        let mut best_by_chunk: HashMap<String, f32> = HashMap::new();
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
                                best_by_chunk
                                    .entry(chunk.id.clone())
                                    .and_modify(|s| {
                                        if best_score > *s {
                                            *s = best_score;
                                        }
                                    })
                                    .or_insert(best_score);
                            }
                        }
                        let mut tuples: Vec<(String, f32)> = best_by_chunk.into_iter().collect();
                        tuples.sort_by(|a, b| {
                            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
                        });
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
    let mut merged = rrf_merge(
        &vector_tuples,
        &bm25_tuples,
        &ppr_tuples,
        vector_weight,
        bm25_weight,
        ppr_weight,
        fetch_limit,
    );
    // Ensure exact-channel-only chunks are considered in final rerank.
    let existing_ids: HashSet<String> = merged.iter().map(|(id, _)| id.clone()).collect();
    for (id, _) in &exact_tuples {
        if !existing_ids.contains(id) {
            merged.push((id.clone(), crate::graph::RrfScores::default()));
        }
    }

    // 5. Build response with score breakdown
    let mut content_map: HashMap<String, crate::types::ScoredCodeChunk> = HashMap::new();
    for r in &vector_results {
        content_map.insert(r.id.clone(), r.clone());
    }
    for r in &bm25_results {
        content_map.entry(r.id.clone()).or_insert_with(|| r.clone());
    }
    let exact_map: HashMap<String, f32> = exact_tuples.into_iter().collect();

    let mut ranked: Vec<(f32, serde_json::Value)> = merged
        .into_iter()
        .filter_map(|(id, scores)| {
            content_map.get(&id).map(|chunk| {
                let lexical_score =
                    lexical_rerank_score(chunk, &query_lower, &query_terms, codeish_query);
                let exact_score = *exact_map.get(&id).unwrap_or(&0.0);
                let lexical_weight = if codeish_query { 0.02 } else { 0.008 };
                let exact_weight = if codeish_query { 0.035 } else { 0.0 };
                let final_score = scores.combined_score
                    + lexical_score * lexical_weight
                    + exact_score * exact_weight;
                (
                    final_score,
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
                        "score": final_score,
                        "rrf_score": scores.combined_score,
                        "lexical_score": lexical_score,
                        "exact_score": exact_score,
                        "vector_score": scores.vector_score,
                        "bm25_score": scores.bm25_score,
                        "ppr_score": scores.ppr_score,
                    }),
                )
            })
        })
        .collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(limit);
    let results: Vec<serde_json::Value> = ranked.into_iter().map(|(_, v)| v).collect();

    let mut response = json!({
        "results": results,
        "count": results.len(),
        "query": query,
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
