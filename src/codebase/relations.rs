//! Shared logic for creating symbol relations.

use std::collections::HashMap;

use crate::codebase::symbol_index::{ResolutionContext, SymbolIndex};
use crate::storage::StorageBackend;
use crate::types::safe_thing;
use crate::types::symbol::{
    CodeReference, CodeRelationType, CodeSymbol, SymbolRef, SymbolRelation,
};

/// Statistics from relation creation.
#[derive(Debug, Default)]
pub struct RelationStats {
    pub created: u32,
    pub failed: u32,
    pub unresolved: u32,
}

/// Create symbol relations from references using the symbol index for resolution.
///
/// Collects all resolvable relations into a Vec and writes them in a single
/// batch query instead of N individual RELATE round-trips.
pub async fn create_symbol_relations(
    storage: &impl StorageBackend,
    project_id: &str,
    references: &[CodeReference],
    symbol_index: &SymbolIndex,
) -> RelationStats {
    let mut stats = RelationStats::default();
    let mut batch: Vec<SymbolRelation> = Vec::with_capacity(references.len());

    for reference in references {
        // 1. Build from_symbol Thing using the stored definition line
        let from_thing = safe_thing::symbol_thing(
            project_id,
            &reference.file_path,
            &reference.from_symbol,
            reference.from_symbol_line,
        );

        // 2. Resolve to_symbol with priority (same file > same dir > any)
        let ctx = ResolutionContext::new(reference.file_path.clone());

        let to_thing = if let Some(resolved) = symbol_index.resolve(&reference.to_symbol, &ctx) {
            resolved.to_thing(project_id)
        } else {
            // Fallback: DB lookup with file context preference
            match storage
                .find_symbol_by_name_with_context(
                    project_id,
                    &reference.to_symbol,
                    Some(&reference.file_path),
                )
                .await
            {
                Ok(Some(sym)) => SymbolRef::from_symbol(&sym).to_thing(project_id),
                _ => {
                    stats.unresolved += 1;
                    tracing::debug!(
                        from = %reference.from_symbol,
                        to = %reference.to_symbol,
                        file = %reference.file_path,
                        "Skipping external symbol (not in project)"
                    );
                    continue;
                }
            }
        };

        // 3. Collect the relation for batch write
        batch.push(SymbolRelation::new(
            from_thing,
            to_thing,
            reference.relation_type,
            reference.file_path.clone(),
            reference.line,
            project_id.to_string(),
        ));
    }

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
