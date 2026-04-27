//! Reciprocal Rank Fusion (RRF) for hybrid search
//!
//! Merges results from multiple ranking sources (vector, BM25, PPR)
//! into a single ranked list using RRF algorithm.

use std::collections::HashMap;

/// RRF constant (standard value from literature)
pub const RRF_K: f32 = 60.0;

/// Default weights for hybrid memory search
pub const DEFAULT_VECTOR_WEIGHT: f32 = 0.40;
pub const DEFAULT_BM25_WEIGHT: f32 = 0.15;
pub const DEFAULT_PPR_WEIGHT: f32 = 0.45;

/// Default weights for hybrid code search (recall_code)
pub const DEFAULT_CODE_VECTOR_WEIGHT: f32 = 0.50;
pub const DEFAULT_CODE_BM25_WEIGHT: f32 = 0.20;
pub const DEFAULT_CODE_PPR_WEIGHT: f32 = 0.30;

/// Individual score components for a merged result
#[derive(Debug, Clone, Default)]
pub struct RrfScores {
    pub vector_score: f32,
    pub bm25_score: f32,
    pub ppr_score: f32,
    pub combined_score: f32,
}

/// Merge multiple ranked lists using Reciprocal Rank Fusion
///
/// Each input is a Vec of (id, score) tuples, already sorted by score descending.
/// Returns merged results sorted by combined RRF score.
///
/// # Arguments
/// * `vector_results` - Results from vector similarity search
/// * `bm25_results` - Results from BM25 text search
/// * `ppr_results` - Results from Personalized PageRank (can be empty)
/// * `vector_weight` - Weight for vector component (default 0.40)
/// * `bm25_weight` - Weight for BM25 component (default 0.15)
/// * `ppr_weight` - Weight for PPR component (default 0.45)
/// * `limit` - Maximum results to return
pub fn rrf_merge(
    vector_results: &[(String, f32)],
    bm25_results: &[(String, f32)],
    ppr_results: &[(String, f32)],
    vector_weight: f32,
    bm25_weight: f32,
    ppr_weight: f32,
    limit: usize,
) -> Vec<(String, RrfScores)> {
    let mut scores: HashMap<String, RrfScores> = HashMap::new();

    for (rank, (id, original_score)) in vector_results.iter().enumerate() {
        let rrf_score = vector_weight / (RRF_K + rank as f32 + 1.0);
        let entry = scores.entry(id.clone()).or_default();
        entry.vector_score = *original_score;
        entry.combined_score += rrf_score;
    }

    for (rank, (id, original_score)) in bm25_results.iter().enumerate() {
        let rrf_score = bm25_weight / (RRF_K + rank as f32 + 1.0);
        let entry = scores.entry(id.clone()).or_default();
        entry.bm25_score = *original_score;
        entry.combined_score += rrf_score;
    }

    for (rank, (id, original_score)) in ppr_results.iter().enumerate() {
        let rrf_score = ppr_weight / (RRF_K + rank as f32 + 1.0);
        let entry = scores.entry(id.clone()).or_default();
        entry.ppr_score = *original_score;
        entry.combined_score += rrf_score;
    }

    let mut results: Vec<_> = scores.into_iter().collect();
    results.sort_by(|a, b| {
        b.1.combined_score
            .partial_cmp(&a.1.combined_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_merge_empty() {
        let results = rrf_merge(&[], &[], &[], 0.4, 0.15, 0.45, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rrf_merge_single_source() {
        let vector = vec![
            ("a".to_string(), 0.9),
            ("b".to_string(), 0.8),
            ("c".to_string(), 0.7),
        ];
        let results = rrf_merge(&vector, &[], &[], 0.4, 0.15, 0.45, 10);

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "a");
        assert!(results[0].1.vector_score > 0.0);
        assert_eq!(results[0].1.bm25_score, 0.0);
    }

    #[test]
    fn test_rrf_merge_multiple_sources() {
        let vector = vec![("a".to_string(), 0.9), ("b".to_string(), 0.8)];
        let bm25 = vec![("b".to_string(), 0.95), ("c".to_string(), 0.7)];
        let results = rrf_merge(&vector, &bm25, &[], 0.4, 0.15, 0.45, 10);
        assert_eq!(results.len(), 3);
        let b_result = results.iter().find(|(id, _)| id == "b").unwrap();
        assert!(b_result.1.vector_score > 0.0);
        assert!(b_result.1.bm25_score > 0.0);
    }

    #[test]
    fn test_rrf_merge_complex() {
        let vector = vec![
            ("1".to_string(), 0.9),
            ("2".to_string(), 0.8),
            ("3".to_string(), 0.7),
        ];
        let bm25 = vec![("3".to_string(), 0.9), ("1".to_string(), 0.8)];

        let results = rrf_merge(&vector, &bm25, &[], 0.5, 0.5, 0.0, 10);

        // Item 1 rank: vector=0, bm25=1. Score = 0.5/(60+0+1) + 0.5/(60+1+1)
        // Item 3 rank: vector=2, bm25=0. Score = 0.5/(60+2+1) + 0.5/(60+0+1)

        assert_eq!(results[0].0, "1"); // Item 1 should be first because rank 0 + rank 1 is better than rank 2 + rank 0
        assert_eq!(results[1].0, "3");
        assert_eq!(results[2].0, "2");
    }

    #[test]
    fn test_rrf_merge_vector_dominates_when_vector_weight_is_higher() {
        let vector = vec![("a".to_string(), 0.99), ("b".to_string(), 0.98)];
        let bm25 = vec![("b".to_string(), 0.99), ("a".to_string(), 0.98)];

        let results = rrf_merge(&vector, &bm25, &[], 0.9, 0.1, 0.0, 10);

        assert_eq!(results[0].0, "a");
        assert!(results[0].1.vector_score > results[0].1.bm25_score);
        assert!(results[0].1.combined_score > results[1].1.combined_score);
    }

    #[test]
    fn test_rrf_merge_bm25_dominates_when_bm25_weight_is_higher() {
        let vector = vec![("a".to_string(), 0.99), ("b".to_string(), 0.98)];
        let bm25 = vec![("b".to_string(), 0.99), ("a".to_string(), 0.98)];

        let results = rrf_merge(&vector, &bm25, &[], 0.1, 0.9, 0.0, 10);

        assert_eq!(results[0].0, "b");
        assert!(results[0].1.bm25_score > results[0].1.vector_score);
        assert!(results[0].1.combined_score > results[1].1.combined_score);
    }
}
