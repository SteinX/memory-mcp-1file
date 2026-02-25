use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

/// Detect communities using the Louvain algorithm for modularity maximization.
///
/// This implementation performs local modularity optimization with iterative refinement.
/// For most knowledge graphs, this provides good community structure detection.
///
/// # Parameters
/// - `graph`: Directed graph with String node weights and f32 edge weights
///
/// # Returns
/// Vector of communities, where each community is a vector of node indices
pub fn detect_communities(graph: &DiGraph<String, f32>) -> Vec<Vec<NodeIndex>> {
    detect_communities_with_resolution(graph, 1.0)
}

/// Detect communities with configurable resolution parameter.
///
/// # Parameters  
/// - `resolution`: Tune granularity. <1 = more communities, >1 = fewer.
pub fn detect_communities_with_resolution(
    graph: &DiGraph<String, f32>,
    resolution: f32,
) -> Vec<Vec<NodeIndex>> {
    let n = graph.node_count();
    if n == 0 {
        return vec![];
    }

    let mut neighbors: Vec<Vec<(usize, f32)>> = vec![vec![]; n];
    let mut total_weight: f32 = 0.0;
    let mut node_weights: Vec<f32> = vec![0.0; n];

    for edge in graph.edge_references() {
        let u = edge.source().index();
        let v = edge.target().index();
        let w = *edge.weight();

        neighbors[u].push((v, w));
        neighbors[v].push((u, w));
        node_weights[u] += w;
        node_weights[v] += w;
        total_weight += w;
    }

    if total_weight == 0.0 {
        return graph.node_indices().map(|idx| vec![idx]).collect();
    }

    let mut community_assignment: Vec<usize> = (0..n).collect();
    let mut community_weights: Vec<f32> = node_weights.clone();

    let m2 = 2.0 * total_weight;

    let mut changed = true;
    let mut iterations = 0;
    const MAX_ITER: usize = 20;

    while changed && iterations < MAX_ITER {
        changed = false;
        iterations += 1;

        for i in 0..n {
            let current_comm = community_assignment[i];
            let ki = node_weights[i];

            let mut gain_map: HashMap<usize, f32> = HashMap::new();
            for &(neighbor, weight) in &neighbors[i] {
                let neighbor_comm = community_assignment[neighbor];
                *gain_map.entry(neighbor_comm).or_insert(0.0) += weight;
            }

            let ki_in_current = *gain_map.get(&current_comm).unwrap_or(&0.0);
            let sum_tot_current = community_weights[current_comm] - ki;

            let mut best_comm = current_comm;
            let mut max_delta: f32 = 0.0;

            for (&comm, &ki_in) in &gain_map {
                if comm == current_comm {
                    continue;
                }

                let sum_tot = community_weights[comm];

                let gain_new = (ki_in / m2) - resolution * (sum_tot * ki) / (m2 * m2);
                let gain_current =
                    (ki_in_current / m2) - resolution * (sum_tot_current * ki) / (m2 * m2);

                let delta = gain_new - gain_current;

                if delta > max_delta {
                    max_delta = delta;
                    best_comm = comm;
                }
            }

            if best_comm != current_comm && max_delta > 1e-10 {
                community_assignment[i] = best_comm;
                community_weights[current_comm] -= ki;
                community_weights[best_comm] += ki;
                changed = true;
            }
        }
    }

    let mut communities_map: HashMap<usize, Vec<NodeIndex>> = HashMap::new();
    for (node_idx, &comm_idx) in community_assignment.iter().enumerate() {
        communities_map
            .entry(comm_idx)
            .or_default()
            .push(NodeIndex::new(node_idx));
    }

    communities_map.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use petgraph::graph::DiGraph;

    fn community_count(graph: &DiGraph<String, f32>) -> usize {
        detect_communities(graph).len()
    }

    fn nodes_in_same_community(graph: &DiGraph<String, f32>, a: NodeIndex, b: NodeIndex) -> bool {
        let communities = detect_communities(graph);
        communities.iter().any(|c| c.contains(&a) && c.contains(&b))
    }

    #[test]
    fn test_empty_graph_returns_no_communities() {
        let graph: DiGraph<String, f32> = DiGraph::new();
        let communities = detect_communities(&graph);
        assert!(
            communities.is_empty(),
            "Empty graph should have no communities"
        );
    }

    #[test]
    fn test_single_node_no_edges_forms_its_own_community() {
        let mut graph: DiGraph<String, f32> = DiGraph::new();
        graph.add_node("A".to_string());
        // Single node, zero weight → early-return path (one community per node)
        let communities = detect_communities(&graph);
        assert_eq!(communities.len(), 1);
        assert_eq!(communities[0].len(), 1);
    }

    #[test]
    fn test_two_disconnected_cliques_form_two_communities() {
        // Clique 1: nodes 0-1-2 fully connected
        // Clique 2: nodes 3-4-5 fully connected
        // No edges between the cliques → algorithm should separate them.
        let mut graph: DiGraph<String, f32> = DiGraph::new();
        let n: Vec<NodeIndex> = (0..6).map(|i| graph.add_node(format!("node{i}"))).collect();

        // Clique 1
        for &(a, b) in &[(0, 1), (1, 2), (0, 2)] {
            graph.add_edge(n[a], n[b], 1.0);
            graph.add_edge(n[b], n[a], 1.0);
        }
        // Clique 2
        for &(a, b) in &[(3, 4), (4, 5), (3, 5)] {
            graph.add_edge(n[a], n[b], 1.0);
            graph.add_edge(n[b], n[a], 1.0);
        }

        let num_communities = community_count(&graph);
        assert_eq!(
            num_communities, 2,
            "Two disconnected cliques should produce exactly 2 communities, got {num_communities}"
        );

        // Nodes within the same clique must be co-located
        assert!(nodes_in_same_community(&graph, n[0], n[1]));
        assert!(nodes_in_same_community(&graph, n[0], n[2]));
        assert!(nodes_in_same_community(&graph, n[3], n[4]));
        assert!(nodes_in_same_community(&graph, n[3], n[5]));

        // Nodes from different cliques must NOT be co-located
        assert!(!nodes_in_same_community(&graph, n[0], n[3]));
        assert!(!nodes_in_same_community(&graph, n[2], n[5]));
    }

    #[test]
    fn test_three_disconnected_cliques_form_three_communities() {
        let mut graph: DiGraph<String, f32> = DiGraph::new();
        let n: Vec<NodeIndex> = (0..9).map(|i| graph.add_node(format!("n{i}"))).collect();

        for clique_start in [0usize, 3, 6] {
            let (a, b, c) = (clique_start, clique_start + 1, clique_start + 2);
            for &(x, y) in &[(a, b), (b, c), (a, c)] {
                graph.add_edge(n[x], n[y], 1.0);
                graph.add_edge(n[y], n[x], 1.0);
            }
        }

        let num_communities = community_count(&graph);
        assert_eq!(
            num_communities, 3,
            "Three disconnected cliques should produce 3 communities, got {num_communities}"
        );
    }

    #[test]
    fn test_fully_connected_graph_is_one_community() {
        let mut graph: DiGraph<String, f32> = DiGraph::new();
        let n: Vec<NodeIndex> = (0..4).map(|i| graph.add_node(format!("n{i}"))).collect();

        for i in 0..4 {
            for j in 0..4 {
                if i != j {
                    graph.add_edge(n[i], n[j], 1.0);
                }
            }
        }

        let communities = detect_communities(&graph);
        assert_eq!(
            communities.len(),
            1,
            "Fully connected graph should form a single community"
        );
        assert_eq!(communities[0].len(), 4);
    }

    #[test]
    fn test_resolution_higher_creates_more_communities() {
        // With very high resolution even weak connections split into separate communities.
        let mut graph: DiGraph<String, f32> = DiGraph::new();
        let n: Vec<NodeIndex> = (0..4).map(|i| graph.add_node(format!("n{i}"))).collect();

        // Two pairs with a very weak bridge
        graph.add_edge(n[0], n[1], 10.0);
        graph.add_edge(n[1], n[0], 10.0);
        graph.add_edge(n[2], n[3], 10.0);
        graph.add_edge(n[3], n[2], 10.0);
        // Weak inter-cluster link
        graph.add_edge(n[1], n[2], 0.001);
        graph.add_edge(n[2], n[1], 0.001);

        let low_res = detect_communities_with_resolution(&graph, 0.01).len();
        let high_res = detect_communities_with_resolution(&graph, 100.0).len();

        assert!(
            high_res >= low_res,
            "Higher resolution should produce >= communities than lower resolution ({high_res} vs {low_res})"
        );
    }
}
