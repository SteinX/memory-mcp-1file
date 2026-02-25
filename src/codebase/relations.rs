//! Shared logic for creating symbol relations.

use crate::codebase::symbol_index::{ResolutionContext, SymbolIndex};
use crate::storage::StorageBackend;
use crate::types::safe_thing;
use crate::types::symbol::{CodeReference, SymbolRef, SymbolRelation};

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
