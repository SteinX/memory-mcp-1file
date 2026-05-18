//! Shared logic for creating symbol relations.

use std::collections::HashMap;

use crate::codebase::symbol_index::{ResolutionContext, SymbolIndex};
use crate::storage::StorageBackend;
use crate::types::safe_thing;
use crate::types::symbol::{
    CodeReference, CodeRelationType, CodeSymbol, SymbolRef, SymbolRelation,
};
use crate::types::{ConfidenceClass, RelationClass, RelationProvenance, StalenessState};

/// Statistics from relation creation.
#[derive(Debug, Default)]
pub struct RelationStats {
    pub created: u32,
    pub failed: u32,
    pub unresolved: u32,
}

/// Create symbol relations from references using the symbol index for resolution.
///
/// Uses a two-pass approach to avoid N individual DB round-trips:
///   Pass 1: resolve all references in-memory via SymbolIndex (O(1) per lookup).
///           Collect names that failed in-memory resolution for a single batch DB query.
///   Pass 2: resolve remaining names from the batch DB result.
///   Final:  write all resolved relations in a single batch RELATE query.
pub async fn create_symbol_relations(
    storage: &impl StorageBackend,
    project_id: &str,
    references: &[CodeReference],
    symbol_index: &SymbolIndex,
) -> RelationStats {
    let mut stats = RelationStats::default();

    if references.is_empty() {
        return stats;
    }

    // ── Pass 1: in-memory resolution ────────────────────────────────────────
    // For each reference, try the SymbolIndex first (hash-map lookup, no I/O).
    // Collect names that need a DB fallback — deduplicated to minimise the
    // number of rows the batch query must scan.

    struct Pending<'a> {
        reference: &'a CodeReference,
        from_thing: surrealdb::types::RecordId,
        // None means we need a DB fallback for to_symbol
        to_thing: Option<surrealdb::types::RecordId>,
        in_memory: bool, // true → use reference's own class/provenance
    }

    let mut pending: Vec<Pending> = Vec::with_capacity(references.len());
    let mut fallback_names: Vec<String> = Vec::new();
    let mut fallback_name_set: std::collections::HashSet<String> = std::collections::HashSet::new();

    for reference in references {
        let from_thing = safe_thing::symbol_thing(
            project_id,
            &reference.file_path,
            &reference.from_symbol,
            reference.from_symbol_line,
        );
        let ctx = ResolutionContext::new(reference.file_path.clone());

        if let Some(resolved) = symbol_index.resolve(&reference.to_symbol, &ctx) {
            pending.push(Pending {
                reference,
                from_thing,
                to_thing: Some(resolved.to_thing(project_id)),
                in_memory: true,
            });
        } else {
            // Need DB fallback — deduplicate names to avoid redundant queries
            if fallback_name_set.insert(reference.to_symbol.clone()) {
                fallback_names.push(reference.to_symbol.clone());
            }
            pending.push(Pending {
                reference,
                from_thing,
                to_thing: None,
                in_memory: false,
            });
        }
    }

    // ── Pass 2: single batched DB query for all unresolved names ────────────
    // One round-trip replaces up to N individual find_symbol_by_name_with_context calls.
    let db_fallback_count = fallback_names.len();
    let mut db_symbol_map: HashMap<String, SymbolRef> = HashMap::new();

    if !fallback_names.is_empty() {
        tracing::debug!(
            count = fallback_names.len(),
            "Batch DB fallback for unresolved symbol names"
        );
        match storage
            .find_symbols_by_names(project_id, &fallback_names)
            .await
        {
            Ok(symbols) => {
                for sym in symbols {
                    // Keep the first match per name (same priority as the old single-lookup path)
                    db_symbol_map
                        .entry(sym.name.clone())
                        .or_insert_with(|| SymbolRef::from_symbol(&sym));
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Batch symbol name lookup failed; relations for unresolved names will be skipped");
            }
        }
    }

    // ── Build relation batch ─────────────────────────────────────────────────
    let mut batch: Vec<SymbolRelation> = Vec::with_capacity(pending.len());

    for p in pending {
        let to_thing = if let Some(t) = p.to_thing {
            t
        } else {
            // Resolve from batch DB result
            match db_symbol_map.get(&p.reference.to_symbol) {
                Some(sym_ref) => sym_ref.to_thing(project_id),
                None => {
                    stats.unresolved += 1;
                    tracing::debug!(
                        from = %p.reference.from_symbol,
                        to = %p.reference.to_symbol,
                        file = %p.reference.file_path,
                        "Skipping external symbol (not in project)"
                    );
                    continue;
                }
            }
        };

        let (relation_class, provenance, confidence_class) = if p.in_memory {
            (
                p.reference.relation_class,
                p.reference.provenance,
                p.reference.confidence_class,
            )
        } else {
            (
                RelationClass::Inferred,
                RelationProvenance::HeuristicResolver,
                ConfidenceClass::Ambiguous,
            )
        };

        batch.push(SymbolRelation::new(
            p.from_thing,
            to_thing,
            p.reference.relation_type,
            relation_class,
            provenance,
            confidence_class,
            p.reference.freshness_generation,
            p.reference.staleness_state,
            p.reference.file_path.clone(),
            p.reference.line,
            project_id.to_string(),
        ));
    }

    tracing::info!(
        total_references = references.len(),
        db_fallback_names = db_fallback_count,
        db_symbols_found = db_symbol_map.len(),
        batch_size = batch.len(),
        unresolved = stats.unresolved,
        "Symbol relation resolution complete"
    );

    // 4. Flush all relations in a single batch query
    if !batch.is_empty() {
        match storage.create_symbol_relations_batch(batch).await {
            Ok(n) => stats.created = n,
            Err(e) => {
                stats.failed = 1; // report as a single batch failure
                tracing::warn!(
                    error = %e,
                    "Batch symbol relation creation failed"
                );
            }
        }
    }

    if stats.created > 0 || stats.failed > 0 || stats.unresolved > 0 {
        tracing::info!(
            created = stats.created,
            failed = stats.failed,
            unresolved = stats.unresolved,
            "Relation creation complete"
        );
    }

    stats
}

pub async fn create_symbol_relations_for_generation(
    storage: &impl StorageBackend,
    project_id: &str,
    references: &[CodeReference],
    symbol_index: &SymbolIndex,
    generation: u64,
) -> RelationStats {
    let references: Vec<CodeReference> = references
        .iter()
        .cloned()
        .map(|mut reference| {
            reference.freshness_generation = generation;
            reference
        })
        .collect();
    create_symbol_relations(storage, project_id, &references, symbol_index).await
}

/// Detect containment (parent→child) relationships between symbols in the same file.
///
/// Two symbols have a containment relationship when one's line range fully
/// encloses the other's. For example, an `impl` block (lines 10-50) contains
/// a `fn` (lines 15-25). Only the **tightest** (most specific) parent is
/// linked to avoid redundant transitive edges.
///
/// Returns `CodeReference` entries with `relation_type: Contains` that can be
/// fed into the standard `create_symbol_relations` pipeline.
pub fn detect_containment_references(symbols: &[CodeSymbol]) -> Vec<CodeReference> {
    if symbols.len() < 2 {
        return vec![];
    }

    // Group by file_path (containment is intra-file only)
    let mut by_file: HashMap<&str, Vec<&CodeSymbol>> = HashMap::new();
    for sym in symbols {
        by_file.entry(&sym.file_path).or_default().push(sym);
    }

    let mut refs = Vec::new();

    for (file_path, mut file_syms) in by_file {
        if file_syms.len() < 2 {
            continue;
        }

        // Sort by (start_line ASC, end_line DESC) so parents appear before
        // children. When two symbols start at the same line, the wider one
        // (larger end_line) comes first — that's the parent.
        file_syms.sort_by(|a, b| {
            a.start_line
                .cmp(&b.start_line)
                .then(b.end_line.cmp(&a.end_line))
        });

        // Stack of potential parents (index into file_syms).
        // Invariant: stack entries have non-increasing end_line (nesting order).
        let mut stack: Vec<usize> = Vec::new();

        for (i, sym) in file_syms.iter().enumerate() {
            // Pop stack entries that don't contain the current symbol
            while let Some(&top_idx) = stack.last() {
                let parent = file_syms[top_idx];
                if parent.end_line >= sym.end_line {
                    break; // parent still encloses current symbol
                }
                stack.pop();
            }

            // If there's a parent on the stack, emit a Contains edge
            if let Some(&parent_idx) = stack.last() {
                let parent = file_syms[parent_idx];
                // Skip self-containment (same line range = same symbol)
                if parent.start_line != sym.start_line || parent.end_line != sym.end_line {
                    refs.push(CodeReference {
                        name: format!("{}→{}", parent.name, sym.name),
                        from_symbol: parent.name.clone(),
                        from_symbol_line: parent.start_line,
                        to_symbol: sym.name.clone(),
                        relation_type: CodeRelationType::Contains,
                        relation_class: RelationClass::Observed,
                        provenance: RelationProvenance::ContainmentDerived,
                        confidence_class: ConfidenceClass::Extracted,
                        freshness_generation: 0,
                        staleness_state: StalenessState::Current,
                        file_path: file_path.to_string(),
                        line: sym.start_line,
                        column: 0,
                    });
                }
            }

            stack.push(i);
        }
    }

    refs
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::codebase::symbol_index::SymbolIndex;
    use crate::storage::traits::CapacityMemoryCandidate;
    use crate::types::symbol::{CodeReference, CodeRelationType};
    use crate::types::{ConfidenceClass, RelationClass, RelationProvenance, StalenessState};

    // ── Minimal mock that counts fallback calls ───────────────────────────────

    struct FallbackCountingStorage {
        /// Number of times `find_symbols_by_names` (batch) was called.
        batch_calls: Arc<AtomicU32>,
        /// Number of times `find_symbol_by_name_with_context` (per-symbol) was called.
        per_symbol_calls: Arc<AtomicU32>,
        /// Symbols returned by the batch lookup.
        symbols: Vec<crate::types::CodeSymbol>,
    }

    impl FallbackCountingStorage {
        fn new(symbols: Vec<crate::types::CodeSymbol>) -> Self {
            Self {
                batch_calls: Arc::new(AtomicU32::new(0)),
                per_symbol_calls: Arc::new(AtomicU32::new(0)),
                symbols,
            }
        }
    }

    // Implement the full StorageBackend trait; only the three symbol-lookup
    // methods and `create_symbol_relations_batch` are reachable in this test.
    #[allow(unused_variables)]
    impl crate::storage::StorageBackend for FallbackCountingStorage {
        // ── Symbol lookup (the hot-path under test) ───────────────────────────

        async fn find_symbols_by_names(
            &self,
            _project_id: &str,
            _names: &[String],
        ) -> crate::Result<Vec<crate::types::CodeSymbol>> {
            self.batch_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.symbols.clone())
        }

        async fn find_symbol_by_name(
            &self,
            _project_id: &str,
            _name: &str,
        ) -> crate::Result<Option<crate::types::CodeSymbol>> {
            self.per_symbol_calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }

        async fn find_symbol_by_name_with_context(
            &self,
            _project_id: &str,
            _name: &str,
            _prefer_file: Option<&str>,
        ) -> crate::Result<Option<crate::types::CodeSymbol>> {
            self.per_symbol_calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }

        async fn get_symbol_project_id(&self, _: &str) -> crate::Result<Option<String>> {
            Ok(None)
        }

        // ── Relation write (needed to complete the happy path) ────────────────

        async fn create_symbol_relations_batch(
            &self,
            _relations: Vec<crate::types::SymbolRelation>,
        ) -> crate::Result<u32> {
            Ok(_relations.len() as u32)
        }

        async fn create_symbol_relation(
            &self,
            _relation: crate::types::SymbolRelation,
        ) -> crate::Result<String> {
            Ok("test_id".to_string())
        }

        // ── Everything else is unreachable in this test ───────────────────────

        async fn create_memory(&self, _: crate::types::Memory) -> crate::Result<String> {
            unreachable!()
        }
        async fn get_memory(&self, _: &str) -> crate::Result<Option<crate::types::Memory>> {
            unreachable!()
        }
        async fn update_memory(
            &self,
            _: &str,
            _: crate::types::MemoryUpdate,
        ) -> crate::Result<crate::types::Memory> {
            unreachable!()
        }
        fn record_memory_access(
            &self,
            _: &str,
            _: chrono::DateTime<chrono::Utc>,
        ) -> impl Future<Output = crate::Result<()>> + Send {
            async { unreachable!() }
        }
        async fn delete_memory(&self, _: &str) -> crate::Result<bool> {
            unreachable!()
        }
        async fn list_memories(
            &self,
            _: &crate::types::MemoryQuery,
            _: usize,
            _: usize,
        ) -> crate::Result<Vec<crate::types::Memory>> {
            unreachable!()
        }
        async fn list_memory_ids(
            &self,
            _: &crate::types::MemoryQuery,
        ) -> crate::Result<Vec<String>> {
            unreachable!()
        }
        async fn count_memories(&self) -> crate::Result<usize> {
            unreachable!()
        }
        async fn count_memories_filtered(
            &self,
            _: &crate::types::MemoryQuery,
        ) -> crate::Result<usize> {
            unreachable!()
        }
        fn count_valid_memories(&self) -> impl Future<Output = crate::Result<usize>> + Send {
            async { unreachable!() }
        }
        fn list_capacity_candidates(
            &self,
        ) -> impl Future<Output = crate::Result<Vec<CapacityMemoryCandidate>>> + Send {
            async { unreachable!() }
        }
        fn get_memory_last_accessed_at(
            &self,
            _: &str,
        ) -> impl Future<Output = crate::Result<Option<chrono::DateTime<chrono::Utc>>>> + Send
        {
            async { unreachable!() }
        }
        async fn find_memories_by_content_hash(
            &self,
            _: &crate::types::MemoryQuery,
            _: &str,
        ) -> crate::Result<Vec<crate::types::Memory>> {
            unreachable!()
        }
        async fn vector_search(
            &self,
            _: &[f32],
            _: &crate::types::MemoryQuery,
            _: usize,
        ) -> crate::Result<Vec<crate::types::SearchResult>> {
            unreachable!()
        }
        async fn vector_search_code(
            &self,
            _: &[f32],
            _: Option<&str>,
            _: Option<u64>,
            _: usize,
        ) -> crate::Result<Vec<crate::types::ScoredCodeChunk>> {
            unreachable!()
        }
        async fn vector_search_symbols(
            &self,
            _: &[f32],
            _: Option<&str>,
            _: Option<u64>,
            _: usize,
        ) -> crate::Result<Vec<crate::types::CodeSymbol>> {
            unreachable!()
        }
        async fn bm25_search(
            &self,
            _: &str,
            _: &crate::types::MemoryQuery,
            _: usize,
        ) -> crate::Result<Vec<crate::types::SearchResult>> {
            unreachable!()
        }
        async fn bm25_search_code(
            &self,
            _: &str,
            _: Option<&str>,
            _: usize,
        ) -> crate::Result<Vec<crate::types::ScoredCodeChunk>> {
            unreachable!()
        }
        async fn clear_project_embeddings(&self, _: &str) -> crate::Result<u64> {
            unreachable!()
        }
        async fn create_entity(&self, _: crate::types::Entity) -> crate::Result<String> {
            unreachable!()
        }
        async fn get_entity(&self, _: &str) -> crate::Result<Option<crate::types::Entity>> {
            unreachable!()
        }
        async fn search_entities(
            &self,
            _: &str,
            _: usize,
        ) -> crate::Result<Vec<crate::types::Entity>> {
            unreachable!()
        }
        async fn create_relation(&self, _: crate::types::Relation) -> crate::Result<String> {
            unreachable!()
        }
        async fn get_related(
            &self,
            _: &str,
            _: usize,
            _: crate::types::Direction,
        ) -> crate::Result<(Vec<crate::types::Entity>, Vec<crate::types::Relation>)> {
            unreachable!()
        }
        async fn get_subgraph(
            &self,
            _: &[String],
        ) -> crate::Result<(Vec<crate::types::Entity>, Vec<crate::types::Relation>)> {
            unreachable!()
        }
        async fn get_node_degrees(&self, _: &[String]) -> crate::Result<HashMap<String, usize>> {
            unreachable!()
        }
        async fn get_all_entities(&self) -> crate::Result<Vec<crate::types::Entity>> {
            unreachable!()
        }
        async fn get_all_relations(&self) -> crate::Result<Vec<crate::types::Relation>> {
            unreachable!()
        }
        async fn get_valid(
            &self,
            _: &crate::types::MemoryQuery,
            _: usize,
        ) -> crate::Result<Vec<crate::types::Memory>> {
            unreachable!()
        }
        async fn get_valid_at(
            &self,
            _: &crate::types::MemoryQuery,
            _: usize,
        ) -> crate::Result<Vec<crate::types::Memory>> {
            unreachable!()
        }
        fn invalidate(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
        ) -> impl Future<Output = crate::Result<bool>> + Send {
            async { unreachable!() }
        }
        async fn create_code_chunk(&self, _: crate::types::CodeChunk) -> crate::Result<String> {
            unreachable!()
        }
        async fn create_code_chunks_batch(
            &self,
            _: Vec<crate::types::CodeChunk>,
        ) -> crate::Result<Vec<(String, crate::types::CodeChunk)>> {
            unreachable!()
        }
        async fn delete_project_chunks(&self, _: &str) -> crate::Result<usize> {
            unreachable!()
        }
        async fn delete_chunks_by_path(&self, _: &str, _: &str) -> crate::Result<usize> {
            unreachable!()
        }
        async fn get_chunks_by_path(
            &self,
            _: &str,
            _: &str,
            _: Option<u64>,
        ) -> crate::Result<Vec<crate::types::CodeChunk>> {
            unreachable!()
        }
        async fn get_all_chunks_for_project(
            &self,
            _: &str,
            _: Option<u64>,
        ) -> crate::Result<Vec<crate::types::CodeChunk>> {
            unreachable!()
        }
        async fn get_chunks_paginated(
            &self,
            _: &str,
            _: Option<u64>,
            _: usize,
            _: usize,
        ) -> crate::Result<Vec<crate::types::CodeChunk>> {
            unreachable!()
        }
        async fn get_chunks_by_ids(
            &self,
            _: &[String],
            _: Option<u64>,
        ) -> crate::Result<Vec<crate::types::CodeChunk>> {
            unreachable!()
        }
        async fn get_index_status(
            &self,
            _: &str,
        ) -> crate::Result<Option<crate::types::IndexStatus>> {
            unreachable!()
        }
        async fn update_index_status(&self, _: crate::types::IndexStatus) -> crate::Result<()> {
            unreachable!()
        }
        async fn delete_index_status(&self, _: &str) -> crate::Result<()> {
            unreachable!()
        }
        async fn list_projects(&self) -> crate::Result<Vec<String>> {
            unreachable!()
        }
        async fn create_or_update_index_job(
            &self,
            _: &crate::types::IndexJobRecord,
        ) -> crate::Result<()> {
            unreachable!()
        }
        async fn get_index_job(
            &self,
            _: &str,
            _: &str,
        ) -> crate::Result<Option<crate::types::IndexJobRecord>> {
            unreachable!()
        }
        async fn list_index_jobs_for_project(
            &self,
            _: &str,
        ) -> crate::Result<Vec<crate::types::IndexJobRecord>> {
            unreachable!()
        }
        async fn delete_index_job(&self, _: &str, _: &str) -> crate::Result<()> {
            unreachable!()
        }
        async fn upsert_file_checkpoint(
            &self,
            _: &crate::types::IndexFileCheckpoint,
        ) -> crate::Result<()> {
            unreachable!()
        }
        async fn get_file_checkpoint(
            &self,
            _: &str,
            _: u64,
            _: &str,
        ) -> crate::Result<Option<crate::types::IndexFileCheckpoint>> {
            unreachable!()
        }
        async fn list_file_checkpoints_for_job(
            &self,
            _: &str,
            _: u64,
        ) -> crate::Result<Vec<crate::types::IndexFileCheckpoint>> {
            unreachable!()
        }
        async fn get_active_generation(&self, _: &str) -> crate::Result<Option<u64>> {
            unreachable!()
        }
        async fn set_active_generation(&self, _: &str, _: u64) -> crate::Result<()> {
            unreachable!()
        }
        async fn get_serving_generation(
            &self,
            _: &str,
            _: crate::types::CapabilityKind,
        ) -> crate::Result<Option<u64>> {
            unreachable!()
        }
        async fn set_serving_generation(
            &self,
            _: &str,
            _: crate::types::CapabilityKind,
            _: u64,
        ) -> crate::Result<()> {
            unreachable!()
        }
        async fn get_indexing_generation(&self, _: &str) -> crate::Result<Option<u64>> {
            unreachable!()
        }
        async fn set_indexing_generation(&self, _: &str, _: Option<u64>) -> crate::Result<()> {
            unreachable!()
        }
        async fn get_serving_metadata(
            &self,
            _: &str,
        ) -> crate::Result<crate::types::ServingGenerationMetadata> {
            unreachable!()
        }
        async fn list_abandoned_generations(&self, _: &str) -> crate::Result<Vec<u64>> {
            unreachable!()
        }
        async fn delete_project_generation(&self, _: &str, _: u64) -> crate::Result<()> {
            unreachable!()
        }
        async fn get_file_hash(&self, _: &str, _: &str) -> crate::Result<Option<String>> {
            unreachable!()
        }
        async fn set_file_hash(&self, _: &str, _: &str, _: &str) -> crate::Result<()> {
            unreachable!()
        }
        async fn set_file_hashes_batch(
            &self,
            _: &str,
            _: &[(String, String)],
        ) -> crate::Result<()> {
            unreachable!()
        }
        async fn delete_file_hashes(&self, _: &str) -> crate::Result<()> {
            unreachable!()
        }
        async fn delete_file_hash(&self, _: &str, _: &str) -> crate::Result<()> {
            unreachable!()
        }
        async fn create_code_symbol(&self, _: crate::types::CodeSymbol) -> crate::Result<String> {
            unreachable!()
        }
        async fn create_code_symbols_batch(
            &self,
            _: Vec<crate::types::CodeSymbol>,
        ) -> crate::Result<Vec<String>> {
            unreachable!()
        }
        async fn update_symbol_embedding(&self, _: &str, _: Vec<f32>) -> crate::Result<()> {
            unreachable!()
        }
        async fn update_chunk_embedding(&self, _: &str, _: Vec<f32>) -> crate::Result<()> {
            unreachable!()
        }
        async fn batch_update_symbol_embeddings(
            &self,
            _: &[(String, Vec<f32>)],
        ) -> crate::Result<()> {
            unreachable!()
        }
        async fn batch_update_chunk_embeddings(
            &self,
            _: &[(String, Vec<f32>)],
        ) -> crate::Result<()> {
            unreachable!()
        }
        async fn delete_project_symbols(&self, _: &str) -> crate::Result<usize> {
            unreachable!()
        }
        async fn delete_symbols_by_path(&self, _: &str, _: &str) -> crate::Result<usize> {
            unreachable!()
        }
        async fn get_project_symbols(
            &self,
            _: &str,
            _: Option<u64>,
        ) -> crate::Result<Vec<crate::types::CodeSymbol>> {
            unreachable!()
        }
        async fn get_symbol_callers(
            &self,
            _: &str,
            _: Option<u64>,
        ) -> crate::Result<Vec<crate::types::CodeSymbol>> {
            unreachable!()
        }
        async fn get_symbol_callees(
            &self,
            _: &str,
            _: Option<u64>,
        ) -> crate::Result<Vec<crate::types::CodeSymbol>> {
            unreachable!()
        }
        async fn get_related_symbols(
            &self,
            _: &str,
            _: usize,
            _: crate::types::Direction,
            _: Option<u64>,
        ) -> crate::Result<(
            Vec<crate::types::CodeSymbol>,
            Vec<crate::types::SymbolRelation>,
        )> {
            unreachable!()
        }
        async fn get_code_subgraph(
            &self,
            _: &[String],
            _: Option<u64>,
        ) -> crate::Result<(
            Vec<crate::types::CodeSymbol>,
            Vec<crate::types::SymbolRelation>,
        )> {
            unreachable!()
        }
        async fn search_symbols(
            &self,
            _: &str,
            _: Option<&str>,
            _: usize,
            _: usize,
            _: Option<&str>,
            _: Option<&str>,
            _: Option<u64>,
        ) -> crate::Result<(Vec<crate::types::CodeSymbol>, u32)> {
            unreachable!()
        }
        async fn replace_symbol_chunk_map(
            &self,
            _: &str,
            _: &[(String, String, f32)],
        ) -> crate::Result<u32> {
            unreachable!()
        }
        async fn get_mapped_chunks_for_symbols(
            &self,
            _: &str,
            _: &[String],
            _: Option<u64>,
            _: usize,
        ) -> crate::Result<Vec<(String, f32)>> {
            unreachable!()
        }
        async fn count_symbols(&self, _: &str, _: Option<u64>) -> crate::Result<u32> {
            unreachable!()
        }
        async fn count_chunks(&self, _: &str, _: Option<u64>) -> crate::Result<u32> {
            unreachable!()
        }
        async fn count_embedded_symbols(&self, _: &str, _: Option<u64>) -> crate::Result<u32> {
            unreachable!()
        }
        async fn count_embedded_chunks(&self, _: &str, _: Option<u64>) -> crate::Result<u32> {
            unreachable!()
        }
        async fn get_all_project_stats(
            &self,
        ) -> crate::Result<std::collections::HashMap<String, crate::storage::traits::ProjectStats>>
        {
            unreachable!()
        }
        async fn get_unembedded_chunks(&self, _: &str) -> crate::Result<Vec<(String, String)>> {
            unreachable!()
        }
        async fn get_unembedded_symbols(&self, _: &str) -> crate::Result<Vec<(String, String)>> {
            unreachable!()
        }
        async fn count_symbol_relations(&self, _: &str) -> crate::Result<u32> {
            unreachable!()
        }
        async fn health_check(&self) -> crate::Result<bool> {
            unreachable!()
        }
        async fn reset_db(&self) -> crate::Result<()> {
            unreachable!()
        }
        async fn shutdown(&self) -> crate::Result<()> {
            unreachable!()
        }
        async fn upsert_manifest_entry(&self, _: &str, _: &str) -> crate::Result<()> {
            unreachable!()
        }
        async fn upsert_manifest_entries(&self, _: &str, _: &[String]) -> crate::Result<()> {
            unreachable!()
        }
        async fn get_manifest_entries(
            &self,
            _: &str,
        ) -> crate::Result<Vec<crate::types::ManifestEntry>> {
            unreachable!()
        }
        async fn delete_manifest_entries(&self, _: &str) -> crate::Result<()> {
            unreachable!()
        }
        async fn delete_manifest_entry(&self, _: &str, _: &str) -> crate::Result<()> {
            unreachable!()
        }
        async fn count_manifest_entries(&self, _: &str) -> crate::Result<usize> {
            unreachable!()
        }
    }

    fn make_ref(from: &str, to: &str, file: &str) -> CodeReference {
        CodeReference {
            name: format!("{from}→{to}"),
            from_symbol: from.to_string(),
            from_symbol_line: 1,
            to_symbol: to.to_string(),
            relation_type: CodeRelationType::Calls,
            relation_class: RelationClass::Observed,
            provenance: RelationProvenance::ParserExtracted,
            confidence_class: ConfidenceClass::Extracted,
            freshness_generation: 0,
            staleness_state: StalenessState::Current,
            file_path: file.to_string(),
            line: 1,
            column: 0,
        }
    }

    /// Regression test: N unresolved references must trigger exactly ONE batched
    /// `find_symbols_by_names` call, never N individual `find_symbol_by_name_with_context`
    /// calls.  If the code regresses to the N+1 pattern this test will panic
    /// because `per_symbol_calls` will be > 0.
    #[tokio::test]
    async fn test_no_n_plus_one_db_fallback() {
        // Three references whose targets are NOT in the SymbolIndex → all need
        // DB fallback.  The fix must issue a single batch query, not three.
        let references = vec![
            make_ref("caller_a", "unknown_x", "src/a.rs"),
            make_ref("caller_b", "unknown_y", "src/b.rs"),
            make_ref("caller_c", "unknown_z", "src/c.rs"),
        ];

        let storage = FallbackCountingStorage::new(vec![]); // DB returns nothing
        let batch_counter = Arc::clone(&storage.batch_calls);
        let per_sym_counter = Arc::clone(&storage.per_symbol_calls);

        let index = SymbolIndex::new(); // empty — forces all refs to DB fallback
        let stats = create_symbol_relations(&storage, "proj", &references, &index).await;

        // All three should be unresolved (DB returned nothing)
        assert_eq!(stats.unresolved, 3, "expected 3 unresolved symbols");

        // The critical invariant: exactly one batch call, zero per-symbol calls.
        let batch_calls = batch_counter.load(Ordering::SeqCst);
        assert_eq!(
            batch_calls, 1,
            "expected exactly 1 batched find_symbols_by_names call, got {batch_calls}"
        );
        assert_eq!(
            per_sym_counter.load(Ordering::SeqCst),
            0,
            "expected 0 per-symbol find_symbol_by_name_with_context calls (N+1 regression)"
        );
    }

    /// Verify that references already resolved in-memory (via SymbolIndex) do
    /// NOT trigger any DB fallback at all.
    #[tokio::test]
    async fn test_in_memory_resolution_skips_db() {
        use crate::types::symbol::{CodeSymbol, SymbolType};

        let sym = CodeSymbol::new(
            "known_fn".to_string(),
            SymbolType::Function,
            "src/lib.rs".to_string(),
            10,
            20,
            "proj".to_string(),
        );

        let mut index = SymbolIndex::new();
        index.add(&sym);

        let references = vec![make_ref("caller", "known_fn", "src/main.rs")];

        let storage = FallbackCountingStorage::new(vec![]);
        let batch_counter = Arc::clone(&storage.batch_calls);
        let per_sym_counter = Arc::clone(&storage.per_symbol_calls);

        let _stats = create_symbol_relations(&storage, "proj", &references, &index).await;

        assert_eq!(
            batch_counter.load(Ordering::SeqCst),
            0,
            "in-memory hit must not trigger any DB call"
        );
        assert_eq!(
            per_sym_counter.load(Ordering::SeqCst),
            0,
            "in-memory hit must not trigger any DB call"
        );
    }
}
