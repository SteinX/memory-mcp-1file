use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

pub const PPR_DAMPING: f32 = 0.5;
pub const PPR_TOLERANCE: f32 = 1e-6;
pub const PPR_MAX_ITER: usize = 15;

/// Run PPR + hub dampening on a pre-built graph, returning scored nodes sorted descending.
///
/// This is the shared kernel used by both the memory-graph recall path and the
/// code-symbol graph recall path.  Callers are responsible for building the
/// `DiGraph` and `seed_nodes` slice; this function handles the pure graph math.
///
/// Returns `Vec<(NodeIndex, f32)>` sorted by score descending.
pub fn run_ppr(graph: &DiGraph<String, f32>, seeds: &[NodeIndex]) -> Vec<(NodeIndex, f32)> {
    if graph.node_count() == 0 || seeds.is_empty() {
        return vec![];
    }
    let mut scores = personalized_page_rank(graph, seeds, PPR_DAMPING, PPR_TOLERANCE, PPR_MAX_ITER);
    let degrees: HashMap<NodeIndex, usize> = graph
        .node_indices()
        .map(|idx| (idx, graph.edges(idx).count()))
        .collect();
    apply_hub_dampening(&mut scores, &degrees);
    let mut result: Vec<(NodeIndex, f32)> = scores.into_iter().collect();
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    result
}

pub fn personalized_page_rank(
    graph: &DiGraph<String, f32>,
    seed_nodes: &[NodeIndex],
    damping: f32,
    tolerance: f32,
    max_iter: usize,
) -> HashMap<NodeIndex, f32> {
    let n = graph.node_count();
    if n == 0 {
        return HashMap::new();
    }

    let mut personalization = vec![0.0; n];
    if !seed_nodes.is_empty() {
        let seed_weight = 1.0 / seed_nodes.len() as f32;
        for &node in seed_nodes {
            if node.index() < n {
                personalization[node.index()] = seed_weight;
            }
        }
    } else {
        let uniform = 1.0 / n as f32;
        personalization.fill(uniform);
    }

    let mut scores = personalization.clone();

    let out_weight_sums: Vec<f32> = graph
        .node_indices()
        .map(|node| graph.edges(node).map(|e| *e.weight()).sum::<f32>())
        .collect();

    let dangling_nodes: Vec<NodeIndex> = graph
        .node_indices()
        .filter(|&node| out_weight_sums[node.index()] == 0.0)
        .collect();

    for _ in 0..max_iter {
        let mut new_scores = vec![0.0; n];

        let dangling_sum: f32 = dangling_nodes
            .iter()
            .map(|&node| scores[node.index()])
            .sum();

        for node in graph.node_indices() {
            let total_out_weight = out_weight_sums[node.index()];
            if total_out_weight > 0.0 {
                for edge in graph.edges(node) {
                    let weight = *edge.weight();
                    let transition_prob = weight / total_out_weight;
                    let contribution = scores[node.index()] * transition_prob * damping;
                    new_scores[edge.target().index()] += contribution;
                }
            }
        }

        for i in 0..n {
            new_scores[i] += damping * dangling_sum * personalization[i];
            new_scores[i] += (1.0 - damping) * personalization[i];
        }

        let diff: f32 = scores
            .iter()
            .zip(new_scores.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();

        scores = new_scores;

        if diff < tolerance {
            break;
        }
    }

    graph
        .node_indices()
        .map(|idx| (idx, scores[idx.index()]))
        .collect()
}

pub fn apply_hub_dampening(
    scores: &mut HashMap<NodeIndex, f32>,
    degrees: &HashMap<NodeIndex, usize>,
) {
    for (node, score) in scores.iter_mut() {
        if let Some(&degree) = degrees.get(node) {
            if degree > 0 {
                *score /= (degree as f32).sqrt();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ppr_empty_graph() {
        let graph: DiGraph<String, f32> = DiGraph::new();
        let result = personalized_page_rank(&graph, &[], 0.5, 1e-6, 15);
        assert!(result.is_empty());
    }

    #[test]
    fn test_ppr_single_node() {
        let mut graph: DiGraph<String, f32> = DiGraph::new();
        let n1 = graph.add_node("A".to_string());

        let result = personalized_page_rank(&graph, &[n1], 0.5, 1e-6, 15);
        assert_eq!(result.len(), 1);
        assert!((result[&n1] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_ppr_chain() {
        let mut graph: DiGraph<String, f32> = DiGraph::new();
        let n1 = graph.add_node("A".to_string());
        let n2 = graph.add_node("B".to_string());
        let n3 = graph.add_node("C".to_string());
        graph.add_edge(n1, n2, 1.0);
        graph.add_edge(n2, n3, 1.0);

        let result = personalized_page_rank(&graph, &[n1], 0.5, 1e-6, 15);
        assert!(result[&n1] > result[&n2]);
        assert!(result[&n2] > result[&n3]);
    }

    #[test]
    fn test_hub_dampening() {
        let n1 = NodeIndex::new(0);
        let n2 = NodeIndex::new(1);

        let mut scores: HashMap<NodeIndex, f32> = HashMap::new();
        scores.insert(n1, 1.0);
        scores.insert(n2, 1.0);

        let mut degrees: HashMap<NodeIndex, usize> = HashMap::new();
        degrees.insert(n1, 4);
        degrees.insert(n2, 1);

        apply_hub_dampening(&mut scores, &degrees);
        assert!((scores[&n1] - 0.5).abs() < 0.01);
        assert!((scores[&n2] - 1.0).abs() < 0.01);
    }
}
