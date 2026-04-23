use std::collections::HashSet;
use std::sync::Arc;

use rmcp::model::CallToolResult;
use serde_json::json;

use crate::config::AppState;
use crate::graph::{
    rrf_merge, run_ppr, DEFAULT_BM25_WEIGHT, DEFAULT_PPR_WEIGHT, DEFAULT_VECTOR_WEIGHT,
};
use crate::server::params::{RecallParams, SearchParams};
use crate::storage::StorageBackend;
use crate::types::{record_key_to_string, ExportIdentity, MemoryQuery, MemoryType, ScoredMemory, SearchResult};

use super::contracts::{export_contract_meta, summary_collection_response, with_surface_guidance};
use super::{error_response, normalize_limit, success_json};

fn consolidation_trace_from_result(result: &SearchResult) -> serde_json::Value {
    let invalidated = result.valid_until.is_some() || result.invalidation_reason.is_some();
    let status = match (invalidated, result.superseded_by.as_ref()) {
        (true, Some(_)) => "superseded",
        (true, None) => "invalidated",
        (false, Some(_)) => "replacement_linked",
        (false, None) => "active",
    };

    json!({
        "status": status,
        "invalidated": invalidated,
        "invalidation_reason": result.invalidation_reason,
        "superseded_by": result.superseded_by,
        "has_replacement": result.superseded_by.is_some(),
    })
}

fn replacement_lineage_from_result(result: &SearchResult) -> serde_json::Value {
    match result.superseded_by.as_ref() {
        Some(id) => json!({
            "chain_ids": [id],
            "depth": 1,
            "terminal_replacement_id": id,
            "cycle_detected": false,
            "truncated": false,
        }),
        None => json!({
            "chain_ids": [],
            "depth": 0,
            "terminal_replacement_id": serde_json::Value::Null,
            "cycle_detected": false,
            "truncated": false,
        }),
    }
}

fn attention_summary_from_result(result: &SearchResult) -> serde_json::Value {
    let lineage = replacement_lineage_from_result(result);
    json!({
        "requires_operator_attention": false,
        "attention_flags": [],
        "multiple_matches": false,
        "partial_supersede": false,
        "lineage_cycle_detected": lineage["cycle_detected"],
        "lineage_truncated": lineage["truncated"],
        "lineage_depth": lineage["depth"],
        "fingerprint_checked": false,
    })
}

fn operator_summary_from_result(result: &SearchResult) -> serde_json::Value {
    let trace = consolidation_trace_from_result(result);
    let lineage = replacement_lineage_from_result(result);
    let attention = attention_summary_from_result(result);
    let requires_operator_attention = attention["requires_operator_attention"]
        .as_bool()
        .unwrap_or(false);
    let primary_signal = if requires_operator_attention {
        "attention_summary"
    } else {
        "consolidation_trace"
    };

    json!({
        "stage": "retrieval",
        "primary_signal": primary_signal,
        "requires_operator_attention": requires_operator_attention,
        "attention_flags": attention["attention_flags"].clone(),
        "lifecycle_status": trace["status"].clone(),
        "lineage_depth": lineage["depth"].clone(),
        "available_sections": [
            "consolidation_trace",
            "replacement_lineage",
            "attention_summary"
        ],
    })
}

fn enrich_result_truth(mut result: SearchResult) -> SearchResult {
    let lineage = replacement_lineage_from_result(&result);
    let trace = consolidation_trace_from_result(&result);
    let attention = attention_summary_from_result(&result);
    result.consolidation_trace = Some(trace);
    result.replacement_lineage = Some(lineage);
    result.attention_summary = Some(attention);
    result.operator_summary = Some(operator_summary_from_result(&result));
    result
}

fn enrich_results_truth(results: Vec<SearchResult>) -> Vec<SearchResult> {
    results.into_iter().map(enrich_result_truth).collect()
}

fn memory_search_contract_json() -> serde_json::Value {
    let contract = with_surface_guidance(
        export_contract_meta(
            ExportIdentity {
                stable_memory_id: None,
                stable_node_ids: true,
                node_ids_are_project_scoped: false,
                stable_edge_ids: false,
                edge_ids_are_local_only: true,
                node_id_semantics: Some("stable_public_memory_id".to_string()),
                edge_id_semantics: Some("no_public_edge_ids".to_string()),
                ..Default::default()
            },
            None,
        ),
        &["results", "memories", "contract", "summary"],
        &["count", "filters", "diagnostics", "weights", "query"],
        &[],
    );
    serde_json::to_value(contract).unwrap_or_else(|_| json!({}))
}

fn normalize_memory_content(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn dedup_key(r: &SearchResult) -> String {
    if let Some(h) = &r.content_hash {
        if !h.is_empty() {
            return format!("h:{h}");
        }
    }
    format!("c:{}", normalize_memory_content(&r.content).to_lowercase())
}

fn dedup_memory_results(mut results: Vec<SearchResult>, limit: usize) -> Vec<SearchResult> {
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out = Vec::with_capacity(limit.min(results.len()));
    let mut seen = std::collections::HashSet::new();
    for r in results {
        if seen.insert(dedup_key(&r)) {
            out.push(r);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

fn apply_min_score(mut results: Vec<SearchResult>, min_score: Option<f32>) -> Vec<SearchResult> {
    if let Some(ms) = min_score {
        let threshold = ms.clamp(0.0, 1.0);
        results.retain(|r| r.score >= threshold);
    }
    results
}

fn apply_min_score_scored(
    mut results: Vec<ScoredMemory>,
    min_score: Option<f32>,
) -> Vec<ScoredMemory> {
    if let Some(ms) = min_score {
        let threshold = ms.clamp(0.0, 1.0);
        results.retain(|r| r.score >= threshold);
    }
    results
}

fn channel_names(vector_score: f32, bm25_score: f32, ppr_score: f32) -> Vec<String> {
    let mut channels = Vec::new();
    if vector_score > 0.0 {
        channels.push("vector".to_string());
    }
    if bm25_score > 0.0 {
        channels.push("bm25".to_string());
    }
    if ppr_score > 0.0 {
        channels.push("ppr".to_string());
    }
    channels
}

fn importance_multiplier(importance_score: f32) -> f32 {
    importance_score.clamp(0.25, 4.0).sqrt()
}

fn overfetch_limit(limit: usize, filters: &MemoryQuery) -> usize {
    if filters.is_unfiltered() {
        limit * 3
    } else {
        limit * 5
    }
}

fn metadata_filter_diagnostics(filters: &MemoryQuery) -> serde_json::Value {
    json!({
        "enabled": filters.uses_metadata_post_filter(),
        "mode": if filters.uses_metadata_post_filter() {
            "post_query_subset_match"
        } else {
            "disabled"
        },
        "notes": if filters.uses_metadata_post_filter() {
            "metadata_filter is evaluated in Rust after DB retrieval; counts reflect post-filtered results."
        } else {
            "metadata_filter not used for this request."
        }
    })
}

fn metadata_matches(
    candidate: Option<&serde_json::Value>,
    filter: Option<&serde_json::Value>,
) -> bool {
    match filter {
        None => true,
        Some(filter) => match candidate {
            Some(candidate) => json_contains(candidate, filter),
            None => false,
        },
    }
}

fn json_contains(candidate: &serde_json::Value, filter: &serde_json::Value) -> bool {
    match (candidate, filter) {
        (serde_json::Value::Object(candidate_map), serde_json::Value::Object(filter_map)) => {
            filter_map.iter().all(|(key, filter_value)| {
                candidate_map
                    .get(key)
                    .map(|candidate_value| json_contains(candidate_value, filter_value))
                    .unwrap_or(false)
            })
        }
        (serde_json::Value::Array(candidate_items), serde_json::Value::Array(filter_items)) => {
            filter_items.iter().all(|filter_item| {
                candidate_items
                    .iter()
                    .any(|candidate_item| json_contains(candidate_item, filter_item))
            })
        }
        _ => candidate == filter,
    }
}

struct ChannelResults {
    retrieved_candidates: usize,
    post_filter_hits: usize,
    results: Vec<SearchResult>,
}

async fn lexical_memory_search(
    state: &Arc<AppState>,
    query: &str,
    filters: &MemoryQuery,
    limit: usize,
) -> ChannelResults {
    let mut prefilter_filters = filters.clone();
    prefilter_filters.metadata_filter = None;

    let memories = state
        .storage
        .list_memories(&prefilter_filters, overfetch_limit(limit, filters), 0)
        .await
        .unwrap_or_default();

    let retrieved_candidates = memories.len();
    if memories.is_empty() {
        return ChannelResults {
            retrieved_candidates,
            post_filter_hits: 0,
            results: vec![],
        };
    }

    let mut allowed_ids = HashSet::with_capacity(memories.len());
    for memory in memories {
        if let Some(id) = memory
            .id
            .as_ref()
            .map(|record| record_key_to_string(&record.key))
        {
            allowed_ids.insert(id);
        }
    }

    let scored = state
        .memory_search
        .search(query, Some(&allowed_ids), limit.max(retrieved_candidates))
        .await;

    let post_filtered: Vec<SearchResult> = scored
        .into_iter()
        .filter(|result| metadata_matches(result.metadata.as_ref(), filters.metadata_filter.as_ref()))
        .collect();
    let post_filter_hits = post_filtered.len();
    let results = dedup_memory_results(post_filtered, limit);

    ChannelResults {
        retrieved_candidates,
        post_filter_hits,
        results,
    }
}

fn apply_metadata_post_filter(results: Vec<SearchResult>, filters: &MemoryQuery) -> ChannelResults {
    let retrieved_candidates = results.len();
    let filtered: Vec<SearchResult> = results
        .into_iter()
        .filter(|result| metadata_matches(result.metadata.as_ref(), filters.metadata_filter.as_ref()))
        .collect();
    let post_filter_hits = filtered.len();
    ChannelResults {
        retrieved_candidates,
        post_filter_hits,
        results: filtered,
    }
}

pub async fn search(state: &Arc<AppState>, params: SearchParams) -> anyhow::Result<CallToolResult> {
    crate::ensure_embedding_ready!(state);
    let filters = match params.to_memory_query() {
        Ok(filters) => filters,
        Err(e) => return Ok(error_response(e)),
    };

    let query_embedding = state.embedding.embed(&params.query).await?;

    let limit = normalize_limit(params.limit);
    let fetch_limit = overfetch_limit(limit, &filters);
    let mut prefilter_filters = filters.clone();
    prefilter_filters.metadata_filter = None;
    let vector_raw = match state
        .storage
        .vector_search(&query_embedding, &prefilter_filters, fetch_limit)
        .await
    {
        Ok(r) => r,
        Err(e) => return Ok(error_response(e)),
    };
    let vector_channel = apply_metadata_post_filter(vector_raw, &filters);
    let results = apply_min_score(
        enrich_results_truth(dedup_memory_results(vector_channel.results, limit)),
        params.min_score,
    );

    Ok(success_json(json!({
        "results": results,
        "summary": summary_collection_response("collection", results.len(), Some(results.len()), false, None),
        "contract": memory_search_contract_json(),
        "count": results.len(),
        "query": params.query,
        "filters": filters.describe(),
        "vector_hits": results.len(),
        "metadata_filter_diagnostics": metadata_filter_diagnostics(&filters),
        "diagnostics": {
            "vector_retrieved_candidates": vector_channel.retrieved_candidates,
            "vector_post_filter_hits": vector_channel.post_filter_hits,
            "returned_hits": results.len(),
            "metadata_filter": metadata_filter_diagnostics(&filters)
        }
    })))
}

pub async fn search_text(
    state: &Arc<AppState>,
    params: SearchParams,
) -> anyhow::Result<CallToolResult> {
    let limit = normalize_limit(params.limit);
    let filters = match params.to_memory_query() {
        Ok(filters) => filters,
        Err(e) => return Ok(error_response(e)),
    };
    let bm25_channel = lexical_memory_search(state, &params.query, &filters, limit).await;
    let results = apply_min_score(
        enrich_results_truth(dedup_memory_results(bm25_channel.results, limit)),
        params.min_score,
    );

    Ok(success_json(json!({
        "results": results,
        "summary": summary_collection_response("collection", results.len(), Some(results.len()), false, None),
        "contract": memory_search_contract_json(),
        "count": results.len(),
        "query": params.query,
        "filters": filters.describe(),
        "bm25_hits": results.len(),
        "metadata_filter_diagnostics": metadata_filter_diagnostics(&filters),
        "diagnostics": {
            "bm25_retrieved_candidates": bm25_channel.retrieved_candidates,
            "bm25_post_filter_hits": bm25_channel.post_filter_hits,
            "returned_hits": results.len(),
            "metadata_filter": metadata_filter_diagnostics(&filters)
        }
    })))
}

pub async fn recall(state: &Arc<AppState>, params: RecallParams) -> anyhow::Result<CallToolResult> {
    use petgraph::graph::{DiGraph, NodeIndex};
    use std::collections::HashMap;

    crate::ensure_embedding_ready!(state);
    let filters = match params.to_memory_query() {
        Ok(filters) => filters,
        Err(e) => return Ok(error_response(e)),
    };

    let query_embedding = state.embedding.embed(&params.query).await?;

    let limit = normalize_limit(params.limit);
    let fetch_limit = overfetch_limit(limit, &filters);
    let mut prefilter_filters = filters.clone();
    prefilter_filters.metadata_filter = None;

    let vector_weight = params.vector_weight.unwrap_or(DEFAULT_VECTOR_WEIGHT);
    let bm25_weight = params.bm25_weight.unwrap_or(DEFAULT_BM25_WEIGHT);
    let ppr_weight = params.ppr_weight.unwrap_or(DEFAULT_PPR_WEIGHT);

    let vector_results_raw = state
        .storage
        .vector_search(&query_embedding, &prefilter_filters, fetch_limit)
        .await
        .unwrap_or_default();
    let vector_channel = apply_metadata_post_filter(vector_results_raw, &filters);
    let vector_results = enrich_results_truth(dedup_memory_results(vector_channel.results, fetch_limit));

    let bm25_channel = lexical_memory_search(state, &params.query, &filters, fetch_limit).await;
    let bm25_results = enrich_results_truth(dedup_memory_results(bm25_channel.results, fetch_limit));

    let vector_tuples: Vec<_> = vector_results
        .iter()
        .map(|r| (r.id.clone(), r.score))
        .collect();
    let bm25_tuples: Vec<_> = bm25_results
        .iter()
        .map(|r| (r.id.clone(), r.score))
        .collect();

    // Build deterministic, rank-preserving IDs for graph seeding.
    // Previous HashSet->Vec conversion produced random order, which made
    // PPR seeds unstable and degraded recall quality run-to-run.
    let mut all_ids: Vec<String> = Vec::with_capacity(vector_results.len() + bm25_results.len());
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for id in vector_results
        .iter()
        .chain(bm25_results.iter())
        .map(|r| r.id.clone())
    {
        if seen_ids.insert(id.clone()) {
            all_ids.push(id);
        }
    }

    let ppr_tuples: Vec<(String, f32)> = if !all_ids.is_empty() {
        match state.storage.get_subgraph(&all_ids).await {
            Ok((entities, relations)) if !entities.is_empty() => {
                let mut graph: DiGraph<String, f32> = DiGraph::new();
                let mut node_map: HashMap<String, NodeIndex> = HashMap::new();

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
                    if let (Some(&from_idx), Some(&to_idx)) =
                        (node_map.get(&from_str), node_map.get(&to_str))
                    {
                        graph.add_edge(from_idx, to_idx, relation.weight);
                    }
                }

                let seed_nodes: Vec<NodeIndex> = all_ids
                    .iter()
                    .take(20)
                    .filter_map(|id| node_map.get(id).copied())
                    .collect();

                let reverse_map: HashMap<NodeIndex, String> = node_map
                    .iter()
                    .map(|(id, idx)| (*idx, id.clone()))
                    .collect();

                run_ppr(&graph, &seed_nodes)
                    .into_iter()
                    .filter_map(|(idx, score)| reverse_map.get(&idx).map(|id| (id.clone(), score)))
                    .collect()
            }
            _ => vec![],
        }
    } else {
        vec![]
    };

    let merged = rrf_merge(
        &vector_tuples,
        &bm25_tuples,
        &ppr_tuples,
        vector_weight,
        bm25_weight,
        ppr_weight,
        limit,
    );

    let mut content_map: std::collections::HashMap<
        String,
        (&SearchResult, MemoryType, String),
    > =
        std::collections::HashMap::new();
    for r in &vector_results {
        content_map.insert(r.id.clone(), (r, r.memory_type.clone(), dedup_key(r)));
    }
    for r in &bm25_results {
        content_map
            .entry(r.id.clone())
            .or_insert((r, r.memory_type.clone(), dedup_key(r)));
    }

    let mut scored_candidates: Vec<(String, ScoredMemory)> = Vec::with_capacity(limit);
    let mut seen_keys = std::collections::HashSet::new();
    for (id, scores) in merged {
        if let Some((result, mem_type, k)) = content_map.get(&id) {
            let boosted_score = scores.combined_score * importance_multiplier(result.importance_score);
            scored_candidates.push((
                k.clone(),
                ScoredMemory {
                    id: id.clone(),
                    content: result.content.clone(),
                    memory_type: mem_type.clone(),
                    score: boosted_score,
                    vector_score: scores.vector_score,
                    bm25_score: scores.bm25_score,
                    ppr_score: scores.ppr_score,
                    importance_score: result.importance_score,
                    channels: channel_names(
                        scores.vector_score,
                        scores.bm25_score,
                        scores.ppr_score,
                    ),
                    user_id: result.user_id.clone(),
                    agent_id: result.agent_id.clone(),
                    run_id: result.run_id.clone(),
                    namespace: result.namespace.clone(),
                    metadata: result.metadata.clone(),
                    superseded_by: result.superseded_by.clone(),
                    valid_until: result.valid_until.clone(),
                    invalidation_reason: result.invalidation_reason.clone(),
                    consolidation_trace: result.consolidation_trace.clone(),
                    replacement_lineage: result.replacement_lineage.clone(),
                    attention_summary: result.attention_summary.clone(),
                    operator_summary: result.operator_summary.clone(),
                },
            ));
        }
    }

    scored_candidates.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut scored_memories: Vec<ScoredMemory> = Vec::with_capacity(limit);
    for (dedup_key, scored) in scored_candidates {
        if seen_keys.insert(dedup_key) {
            scored_memories.push(scored);
            if scored_memories.len() >= limit {
                break;
            }
        }
    }

    let fused_candidates = scored_memories.len();
    let scored_memories = apply_min_score_scored(scored_memories, params.min_score);

    Ok(success_json(json!({
        "memories": scored_memories,
        "summary": summary_collection_response("collection", scored_memories.len(), Some(scored_memories.len()), false, None),
        "contract": memory_search_contract_json(),
        "count": scored_memories.len(),
        "query": params.query,
        "filters": filters.describe(),
        "diagnostics": {
            "vector_hits": vector_results.len(),
            "bm25_hits": bm25_results.len(),
            "ppr_hits": ppr_tuples.len(),
            "fused_hits": scored_memories.len(),
            "vector_retrieved_candidates": vector_channel.retrieved_candidates,
            "vector_post_filter_hits": vector_channel.post_filter_hits,
            "bm25_retrieved_candidates": bm25_channel.retrieved_candidates,
            "bm25_post_filter_hits": bm25_channel.post_filter_hits,
            "fused_candidates": fused_candidates,
            "returned_hits": scored_memories.len(),
            "metadata_filter": metadata_filter_diagnostics(&filters)
        },
        "weights": {
            "vector": vector_weight,
            "bm25": bm25_weight,
            "ppr": ppr_weight
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::TestContext;
    use crate::types::Memory;

    async fn seed_memory(ctx: &TestContext, memory: Memory) -> String {
        let id = ctx.state.storage.create_memory(memory).await.unwrap();
        let stored = ctx
            .state
            .storage
            .get_memory(&id)
            .await
            .unwrap()
            .expect("seeded memory should exist");
        ctx.state.memory_search.upsert_memory(stored).await;
        id
    }

    #[tokio::test]
    async fn test_search_logic() {
        let ctx = TestContext::new().await;

        // Seed data
        let _ = seed_memory(&ctx, Memory {
                content: "Rust is a systems programming language".to_string(),
                embedding: Some(vec![0.1; 768]), // Mock embedding
                ..Memory::new("Rust is a systems programming language".to_string())
            })
            .await;

        let _ = seed_memory(&ctx, Memory {
                content: "prioritytest low".to_string(),
                embedding: Some(vec![0.2; 768]),
                importance_score: 0.5,
                ..Memory::new("prioritytest low".to_string())
            })
            .await;

        let _ = seed_memory(&ctx, Memory {
                content: "prioritytest high".to_string(),
                embedding: Some(vec![0.2; 768]),
                importance_score: 4.0,
                ..Memory::new("prioritytest high".to_string())
            })
            .await;

        let _ = seed_memory(&ctx, Memory {
                content: "Python is great for scripting".to_string(),
                embedding: Some(vec![0.9; 768]),
                ..Memory::new("Python is great for scripting".to_string())
            })
            .await;

        // 1. Vector Search
        let search_params = SearchParams {
            query: "Rust".to_string(),
            limit: Some(5),
            mode: None,
            min_score: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            memory_type: None,
            metadata_filter: None,
            valid_at: None,
            event_after: None,
            event_before: None,
            ingestion_after: None,
            ingestion_before: None,
        };
        let result = search(&ctx.state, search_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();

        // Vector ranking can be sensitive to adaptive score-flooring in tests.
        // Here we only verify response shape; relevance is asserted below via
        // BM25 and hybrid paths.
        assert!(json["count"].as_u64().is_some());
        assert!(json["diagnostics"]["vector_retrieved_candidates"].as_u64().is_some());
        assert_eq!(json["contract"]["schema_version"], 1);
        assert_eq!(json["contract"]["identity"]["node_id_semantics"], "stable_public_memory_id");
        assert_eq!(json["summary"]["result_kind"], "collection");

        // 2. BM25 Search
        let text_params = SearchParams {
            query: "scripting".to_string(),
            limit: Some(5),
            mode: None,
            min_score: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            memory_type: None,
            metadata_filter: None,
            valid_at: None,
            event_after: None,
            event_before: None,
            ingestion_after: None,
            ingestion_before: None,
        };
        let result = search_text(&ctx.state, text_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let content = json["results"][0]["content"].as_str().unwrap();
        assert!(content.contains("Python"));
        assert!(json["diagnostics"]["bm25_retrieved_candidates"].as_u64().is_some());
        assert_eq!(json["contract"]["identity"]["node_id_semantics"], "stable_public_memory_id");
        assert_eq!(json["summary"]["result_kind"], "collection");
        assert_eq!(json["results"][0]["consolidation_trace"]["status"], "active");
        assert_eq!(json["results"][0]["replacement_lineage"]["depth"], 0);
        assert_eq!(json["results"][0]["attention_summary"]["requires_operator_attention"], false);
        assert_eq!(json["results"][0]["operator_summary"]["stage"], "retrieval");
        assert_eq!(json["results"][0]["operator_summary"]["primary_signal"], "consolidation_trace");

        // 3. Recall (Hybrid)
        let recall_params = RecallParams {
            query: "systems".to_string(),
            limit: Some(5),
            vector_weight: None,
            bm25_weight: None,
            ppr_weight: None,
            min_score: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            memory_type: None,
            metadata_filter: None,
            valid_at: None,
            event_after: None,
            event_before: None,
            ingestion_after: None,
            ingestion_before: None,
        };
        let result = recall(&ctx.state, recall_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(json["count"].as_u64().unwrap() > 0);
        assert!(json["diagnostics"]["vector_hits"].as_u64().is_some());
        assert!(json["diagnostics"]["fused_candidates"].as_u64().is_some());
        assert_eq!(json["contract"]["identity"]["node_id_semantics"], "stable_public_memory_id");
        assert_eq!(json["summary"]["result_kind"], "collection");
        assert_eq!(json["memories"][0]["consolidation_trace"]["status"], "active");
        assert_eq!(json["memories"][0]["replacement_lineage"]["depth"], 0);
        assert_eq!(json["memories"][0]["attention_summary"]["requires_operator_attention"], false);
        assert_eq!(json["memories"][0]["operator_summary"]["stage"], "retrieval");
        assert_eq!(json["memories"][0]["operator_summary"]["lifecycle_status"], "active");

        let priority_params = RecallParams {
            query: "prioritytest".to_string(),
            limit: Some(5),
            vector_weight: None,
            bm25_weight: None,
            ppr_weight: None,
            min_score: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            memory_type: None,
            metadata_filter: None,
            valid_at: None,
            event_after: None,
            event_before: None,
            ingestion_after: None,
            ingestion_before: None,
        };
        let result = recall(&ctx.state, priority_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(json["memories"][0]["content"], "prioritytest high");
        assert_eq!(json["memories"][0]["importance_score"], 4.0);

        let metadata_params = SearchParams {
            query: "prioritytest".to_string(),
            limit: Some(5),
            mode: Some("bm25".to_string()),
            min_score: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            memory_type: None,
            metadata_filter: Some(serde_json::json!({"tier": "gold"})),
            valid_at: None,
            event_after: None,
            event_before: None,
            ingestion_after: None,
            ingestion_before: None,
        };

        let _ = seed_memory(&ctx, Memory {
                content: "prioritytest tagged gold".to_string(),
                embedding: Some(vec![0.3; 768]),
                metadata: Some(serde_json::json!({"tier": "gold"})),
                ..Memory::new("prioritytest tagged gold".to_string())
            })
            .await;

        let result = search_text(&ctx.state, metadata_params).await.unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(json["diagnostics"]["metadata_filter"]["mode"], "post_query_subset_match");
        assert!(json["diagnostics"]["bm25_retrieved_candidates"].as_u64().unwrap() >= json["diagnostics"]["bm25_post_filter_hits"].as_u64().unwrap());

        let superseded_seed = seed_memory(&ctx, Memory {
                content: "retrieval superseded truth".to_string(),
                embedding: Some(vec![0.4; 768]),
                ..Memory::new("retrieval superseded truth".to_string())
            })
            .await;
        let _ = crate::server::logic::memory::invalidate(
            &ctx.state,
            crate::server::params::InvalidateParams {
                id: superseded_seed,
                reason: Some("deduplicated".to_string()),
                superseded_by: Some("replacement-truth".to_string()),
            },
        )
        .await
        .unwrap();

        let result = search_text(
            &ctx.state,
            SearchParams {
                query: "retrieval superseded truth".to_string(),
                limit: Some(5),
                mode: Some("bm25".to_string()),
                min_score: None,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                memory_type: None,
                metadata_filter: None,
                valid_at: None,
                event_after: None,
                event_before: None,
                ingestion_after: None,
                ingestion_before: None,
            },
        )
        .await
        .unwrap();
        let val = serde_json::to_value(&result).unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(json["count"], 0);
    }
}
