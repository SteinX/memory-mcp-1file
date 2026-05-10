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
        match storage.find_symbols_by_names(project_id, &fallback_names).await {
            Ok(symbols) => {
                for sym in symbols {
                    // Keep the first match per name (same priority as the old single-lookup path)
                    db_symbol_map.entry(sym.name.clone()).or_insert_with(|| SymbolRef::from_symbol(&sym));
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
            (p.reference.relation_class, p.reference.provenance, p.reference.confidence_class)
        } else {
            (RelationClass::Inferred, RelationProvenance::HeuristicResolver, ConfidenceClass::Ambiguous)
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
