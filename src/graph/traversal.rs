use crate::types::symbol::{CodeSymbol, SymbolRelation};
use crate::types::{Direction, Entity, Relation};
use crate::Result;
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum TraversalStrategy {
    Direct,
    Bfs,
}

#[derive(Debug, Clone)]
pub struct TraversalConfig {
    pub max_depth: usize,
    pub max_entities_per_level: usize,
    pub max_total_entities: usize,
}

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_entities_per_level: 100,
            max_total_entities: 1000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TraversalResult {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
    pub strategy_used: TraversalStrategy,
    pub depth_reached: usize,
    pub truncated: bool,
    pub deferred_count: usize,
}

#[allow(async_fn_in_trait)]
pub trait GraphTraversalStorage: Send + Sync {
    async fn get_direct_relations(
        &self,
        entity_id: &str,
        direction: Direction,
    ) -> Result<(Vec<Entity>, Vec<Relation>)>;

    async fn get_direct_relations_batch(
        &self,
        entity_ids: &[String],
        direction: Direction,
    ) -> Result<(Vec<Entity>, Vec<Relation>)>;
}

pub struct GraphTraverser<'a, S: GraphTraversalStorage> {
    storage: &'a S,
    config: TraversalConfig,
}

impl<'a, S: GraphTraversalStorage> GraphTraverser<'a, S> {
    pub fn new(storage: &'a S) -> Self {
        Self {
            storage,
            config: TraversalConfig::default(),
        }
    }

    pub fn with_config(storage: &'a S, config: TraversalConfig) -> Self {
        Self { storage, config }
    }

    pub async fn traverse(
        &self,
        entity_id: &str,
        depth: usize,
        direction: Direction,
    ) -> Result<TraversalResult> {
        let depth = depth.clamp(1, self.config.max_depth);

        if depth == 1 {
            let (entities, relations) = self
                .storage
                .get_direct_relations(entity_id, direction)
                .await?;
            return Ok(TraversalResult {
                entities,
                relations,
                strategy_used: TraversalStrategy::Direct,
                depth_reached: 1,
                truncated: false,
                deferred_count: 0,
            });
        }

        self.traverse_bfs(entity_id, depth, direction).await
    }

    async fn traverse_bfs(
        &self,
        entity_id: &str,
        depth: usize,
        direction: Direction,
    ) -> Result<TraversalResult> {
        let mut visited_entities: HashSet<String> = HashSet::new();
        let mut visited_relations: HashSet<String> = HashSet::new();
        let mut all_entities: Vec<Entity> = Vec::new();
        let mut all_relations: Vec<Relation> = Vec::new();
        let mut frontier: VecDeque<String> = VecDeque::new();
        let mut deferred_count: usize = 0;
        let mut truncated = false;

        frontier.push_back(entity_id.to_string());
        visited_entities.insert(entity_id.to_string());

        let mut actual_depth = 0;

        for current_depth in 1..=depth {
            if frontier.is_empty() {
                break;
            }

            actual_depth = current_depth;
            let frontier_vec: Vec<String> = frontier.drain(..).collect();

            let batch_size = frontier_vec.len().min(self.config.max_entities_per_level);

            if frontier_vec.len() > batch_size {
                let deferred = frontier_vec.len() - batch_size;
                deferred_count += deferred;
                truncated = true;
            }

            let (entities, relations) = self
                .storage
                .get_direct_relations_batch(&frontier_vec[..batch_size], direction)
                .await?;

            for rel in relations {
                let rel_id = rel
                    .id
                    .as_ref()
                    .map(|t| crate::types::record_key_to_string(&t.key))
                    .unwrap_or_default();
                if visited_relations.insert(rel_id) {
                    all_relations.push(rel);
                }
            }

            for entity in entities {
                let eid = entity
                    .id
                    .as_ref()
                    .map(|t| crate::types::record_key_to_string(&t.key))
                    .unwrap_or_default();

                if visited_entities.insert(eid.clone()) {
                    all_entities.push(entity);
                    frontier.push_back(eid);

                    if all_entities.len() >= self.config.max_total_entities {
                        truncated = true;
                        deferred_count += frontier.len();
                        return Ok(TraversalResult {
                            entities: all_entities,
                            relations: all_relations,
                            strategy_used: TraversalStrategy::Bfs,
                            depth_reached: actual_depth,
                            truncated,
                            deferred_count,
                        });
                    }
                }
            }
        }

        Ok(TraversalResult {
            entities: all_entities,
            relations: all_relations,
            strategy_used: TraversalStrategy::Bfs,
            depth_reached: actual_depth,
            truncated,
            deferred_count,
        })
    }
}

// ─── Symbol Graph Traversal ──────────────────────────────────────────────────

/// Storage trait for symbol graph traversal (code symbols + symbol relations).
#[allow(async_fn_in_trait)]
pub trait SymbolGraphTraversalStorage: Send + Sync {
    /// Fetch direct symbol relations for a single symbol ID.
    async fn get_direct_symbol_relations(
        &self,
        symbol_id: &str,
        direction: Direction,
    ) -> Result<(Vec<CodeSymbol>, Vec<SymbolRelation>)>;

    /// Fetch direct symbol relations for a batch of symbol IDs (single round-trip).
    async fn get_direct_symbol_relations_batch(
        &self,
        symbol_ids: &[String],
        direction: Direction,
    ) -> Result<(Vec<CodeSymbol>, Vec<SymbolRelation>)>;
}

/// Result of a symbol graph traversal with frontier for agent-driven beam search.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SymbolTraversalResult {
    /// Symbols discovered during traversal.
    pub symbols: Vec<CodeSymbol>,
    /// Relations traversed.
    pub relations: Vec<SymbolRelation>,
    /// Strategy used (Direct for depth=1, Bfs for depth>1).
    pub strategy_used: TraversalStrategy,
    /// Actual depth reached (may be less than requested if graph is shallow).
    pub depth_reached: usize,
    /// Whether the traversal was truncated due to bounding limits.
    pub truncated: bool,
    /// Number of nodes deferred (not explored) due to per-level or total caps.
    pub deferred_count: usize,
    /// Frontier: symbol IDs at the boundary that were NOT yet expanded.
    /// The agent can use these to continue traversal (beam search pattern).
    pub frontier: Vec<String>,
}

/// Bounded BFS traverser for the code symbol graph.
///
/// Mirrors `GraphTraverser` but operates on `CodeSymbol` / `SymbolRelation`
/// and exposes a `frontier` in the result for agent-driven beam search.
pub struct SymbolGraphTraverser<'a, S: SymbolGraphTraversalStorage> {
    storage: &'a S,
    config: TraversalConfig,
}

impl<'a, S: SymbolGraphTraversalStorage> SymbolGraphTraverser<'a, S> {
    pub fn new(storage: &'a S) -> Self {
        Self {
            storage,
            config: TraversalConfig::default(),
        }
    }

    pub fn with_config(storage: &'a S, config: TraversalConfig) -> Self {
        Self { storage, config }
    }

    pub async fn traverse(
        &self,
        symbol_id: &str,
        depth: usize,
        direction: Direction,
    ) -> Result<SymbolTraversalResult> {
        let depth = depth.clamp(1, self.config.max_depth);

        if depth == 1 {
            let (symbols, relations) = self
                .storage
                .get_direct_symbol_relations(symbol_id, direction)
                .await?;

            // Frontier = all discovered symbol IDs (they haven't been expanded)
            let frontier: Vec<String> = symbols
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

            return Ok(SymbolTraversalResult {
                symbols,
                relations,
                strategy_used: TraversalStrategy::Direct,
                depth_reached: 1,
                truncated: false,
                deferred_count: 0,
                frontier,
            });
        }

        self.traverse_bfs(symbol_id, depth, direction).await
    }

    async fn traverse_bfs(
        &self,
        symbol_id: &str,
        depth: usize,
        direction: Direction,
    ) -> Result<SymbolTraversalResult> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut visited_relations: HashSet<String> = HashSet::new();
        let mut all_symbols: Vec<CodeSymbol> = Vec::new();
        let mut all_relations: Vec<SymbolRelation> = Vec::new();
        let mut frontier: VecDeque<String> = VecDeque::new();
        let mut deferred_count: usize = 0;
        let mut truncated = false;

        frontier.push_back(symbol_id.to_string());
        visited.insert(symbol_id.to_string());

        let mut actual_depth = 0;

        for current_depth in 1..=depth {
            if frontier.is_empty() {
                break;
            }

            actual_depth = current_depth;
            let frontier_vec: Vec<String> = frontier.drain(..).collect();

            let batch_size = frontier_vec.len().min(self.config.max_entities_per_level);

            if frontier_vec.len() > batch_size {
                let deferred = frontier_vec.len() - batch_size;
                deferred_count += deferred;
                truncated = true;
                // Put deferred IDs back into frontier for reporting
                for id in &frontier_vec[batch_size..] {
                    frontier.push_back(id.clone());
                }
            }

            let (symbols, relations) = self
                .storage
                .get_direct_symbol_relations_batch(&frontier_vec[..batch_size], direction)
                .await?;

            for rel in relations {
                let rel_id = rel
                    .id
                    .as_ref()
                    .map(|t| crate::types::record_key_to_string(&t.key))
                    .unwrap_or_default();
                if visited_relations.insert(rel_id) {
                    all_relations.push(rel);
                }
            }

            for sym in symbols {
                let sid = sym
                    .id
                    .as_ref()
                    .map(|id| {
                        format!(
                            "{}:{}",
                            id.table.as_str(),
                            crate::types::record_key_to_string(&id.key)
                        )
                    })
                    .unwrap_or_default();

                if visited.insert(sid.clone()) {
                    all_symbols.push(sym);
                    frontier.push_back(sid);

                    if all_symbols.len() >= self.config.max_total_entities {
                        truncated = true;
                        deferred_count += frontier.len();
                        let remaining_frontier: Vec<String> = frontier.drain(..).collect();
                        return Ok(SymbolTraversalResult {
                            symbols: all_symbols,
                            relations: all_relations,
                            strategy_used: TraversalStrategy::Bfs,
                            depth_reached: actual_depth,
                            truncated,
                            deferred_count,
                            frontier: remaining_frontier,
                        });
                    }
                }
            }
        }

        // Final frontier = unexpanded nodes at the deepest level
        let remaining_frontier: Vec<String> = frontier.drain(..).collect();

        Ok(SymbolTraversalResult {
            symbols: all_symbols,
            relations: all_relations,
            strategy_used: TraversalStrategy::Bfs,
            depth_reached: actual_depth,
            truncated,
            deferred_count,
            frontier: remaining_frontier,
        })
    }
}
