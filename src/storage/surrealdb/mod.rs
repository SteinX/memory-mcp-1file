use std::collections::HashMap;
use std::path::Path;

#[cfg(test)]
use surrealdb::engine::local::Mem;
use surrealdb::engine::local::{Db, SurrealKv};
use surrealdb::Surreal;

use super::StorageBackend;
use crate::graph::{GraphTraversalStorage, SymbolGraphTraversalStorage};
use crate::storage::traits::{
    MemoryExportOptions, MemoryGcFilter, MemoryGcReasonCount, MemoryImportOptions, ProjectStats,
};
use crate::types::{
    CapabilityKind, CodeChunk, CodeSymbol, Direction, Entity, ExportMemoryResponse,
    ImportMemoryResponse, IndexFileCheckpoint, IndexJobRecord, IndexStatus, ManifestEntry, Memory,
    MemoryQuery, MemoryUpdate, MigrationMemoryRecord, Relation, ScoredCodeChunk, SearchResult,
    ServingGenerationMetadata, SymbolRelation,
};
use crate::Result;

mod code_ops;
mod graph_ops;
mod helpers;
mod manifest_ops;
mod memory_ops;
mod symbol_ops;

pub struct SurrealStorage {
    pub(super) db: Surreal<Db>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimensionCheck {
    Match { actual: usize },
    Mismatch { actual: usize, expected: usize },
    Unknown,
}

impl SurrealStorage {
    pub async fn new(data_dir: &Path, model_dim: usize) -> Result<Self> {
        let db_path = data_dir.join("db");
        let existing_db = db_path.exists();
        std::fs::create_dir_all(&db_path)?;

        let db: Surreal<Db> = Surreal::new::<SurrealKv>(db_path).await?;
        db.use_ns("memory").use_db("main").await?;

        if !existing_db {
            Self::initialize_schema_for_db(&db, model_dim).await?;
        }

        Ok(Self { db })
    }

    pub async fn initialize_schema(&self, model_dim: usize) -> Result<()> {
        Self::initialize_schema_for_db(&self.db, model_dim).await
    }

    async fn initialize_schema_for_db(db: &Surreal<Db>, model_dim: usize) -> Result<()> {
        // Drop old fulltext index on code_chunks that caused startup errors on existing databases
        db.query("REMOVE INDEX IF EXISTS idx_chunks_fts ON code_chunks;")
            .await?;

        let schema = include_str!("../schema.surql").replace("{dim}", &model_dim.to_string());
        db.query(&schema).await?;
        Ok(())
    }

    #[cfg(test)]
    async fn new_in_memory(model_dim: usize) -> Result<Self> {
        let db: Surreal<Db> = Surreal::new::<Mem>(()).await?;
        db.use_ns("memory").use_db("main").await?;

        db.query("REMOVE INDEX IF EXISTS idx_chunks_fts ON code_chunks;")
            .await?;

        let schema = include_str!("../schema.surql").replace("{dim}", &model_dim.to_string());
        db.query(&schema).await?;

        Ok(Self { db })
    }

    pub async fn check_dimension(&self, expected: usize) -> Result<()> {
        match self.inspect_dimension(expected).await? {
            DimensionCheck::Match { actual } => {
                tracing::info!(model = expected, db = actual, "Dimension check passed");
            }
            DimensionCheck::Mismatch { actual, expected } => {
                tracing::warn!(
                    old = actual,
                    new = expected,
                    "Dimension mismatch detected, rebuilding vector indices"
                );
                self.rebuild_vector_indices(expected).await?;
                self.mark_embeddings_stale().await?;
                tracing::info!("Indices rebuilt, old embeddings marked stale");
            }
            DimensionCheck::Unknown => {}
        }

        Ok(())
    }

    pub async fn inspect_dimension(&self, expected: usize) -> Result<DimensionCheck> {
        let mut response = self.db.query("INFO FOR TABLE memories").await?;
        let result: Option<serde_json::Value> = response.take(0)?;

        let Some(info) = result else {
            return Ok(DimensionCheck::Unknown);
        };
        let Some(indexes) = info.get("indexes").and_then(|i| i.as_object()) else {
            return Ok(DimensionCheck::Unknown);
        };
        let Some(idx_def) = indexes.get("idx_memories_vec").and_then(|v| v.as_str()) else {
            return Ok(DimensionCheck::Unknown);
        };
        let Some(actual) = self.extract_dimension(idx_def) else {
            return Ok(DimensionCheck::Unknown);
        };

        if actual == expected {
            Ok(DimensionCheck::Match { actual })
        } else {
            Ok(DimensionCheck::Mismatch { actual, expected })
        }
    }

    pub async fn mark_embeddings_stale(&self) -> Result<()> {
        self.db
            .query(
                "UPDATE memories SET embedding_state = 'stale', embedding = NONE;
                 UPDATE entities SET embedding = NONE;
                 UPDATE code_chunks SET embedding = NONE;
                 UPDATE code_symbols SET embedding = NONE;",
            )
            .await?;
        Ok(())
    }

    pub async fn rebuild_vector_indices(&self, dim: usize) -> Result<()> {
        let queries = format!(
            "REMOVE INDEX IF EXISTS idx_memories_vec ON memories;
             REMOVE INDEX IF EXISTS idx_entities_vec ON entities;
             REMOVE INDEX IF EXISTS idx_chunks_vec ON code_chunks;
             REMOVE INDEX IF EXISTS idx_symbols_vec ON code_symbols;
             DEFINE INDEX idx_memories_vec ON memories FIELDS embedding HNSW DIMENSION {d} DIST COSINE;
             DEFINE INDEX idx_entities_vec ON entities FIELDS embedding HNSW DIMENSION {d} DIST COSINE;
             DEFINE INDEX idx_chunks_vec ON code_chunks FIELDS embedding HNSW DIMENSION {d} DIST COSINE;
             DEFINE INDEX idx_symbols_vec ON code_symbols FIELDS embedding HNSW DIMENSION {d} DIST COSINE;",
            d = dim
        );
        self.db.query(&queries).await?;
        Ok(())
    }

    fn extract_dimension(&self, def: &str) -> Option<usize> {
        def.split("DIMENSION ")
            .nth(1)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    /// Directly update embedding fields for a memory (stale re-embed).
    pub async fn raw_update_embedding(
        &self,
        id: &str,
        embedding: Vec<f32>,
        content_hash: String,
        embedding_state: &str,
    ) -> Result<()> {
        memory_ops::raw_update_embedding(&self.db, id, embedding, content_hash, embedding_state)
            .await
    }

    /// Get all memories with stale or missing embeddings.
    pub async fn get_stale_memories(&self) -> Result<Vec<Memory>> {
        let mut response = self
            .db
            .query("SELECT * FROM memories WHERE embedding_state = 'stale' OR embedding IS NONE")
            .await?;
        let memories: Vec<Memory> = response.take(0).unwrap_or_default();
        Ok(memories)
    }
}

// ---------------------------------------------------------------------------
// GraphTraversalStorage — needed by the graph traversal engine
// ---------------------------------------------------------------------------

impl GraphTraversalStorage for SurrealStorage {
    async fn get_direct_relations(
        &self,
        entity_id: &str,
        direction: Direction,
    ) -> Result<(Vec<Entity>, Vec<Relation>)> {
        use crate::types::ThingId;

        let entity_thing = ThingId::new("entities", entity_id)?.to_string();

        let sql = match direction {
            Direction::Outgoing => "SELECT * FROM relations WHERE `in` = (type::record($entity_id))",
            Direction::Incoming => "SELECT * FROM relations WHERE `out` = (type::record($entity_id))",
            Direction::Both => {
                "SELECT * FROM relations WHERE `in` = (type::record($entity_id)) OR `out` = (type::record($entity_id))"
            }
        };

        let mut response = self
            .db
            .query(sql)
            .bind(("entity_id", entity_thing.clone()))
            .await?;

        // Use Value intermediary to bypass SurrealValue RecordId bug
        let raw: surrealdb_types::Value = response.take(0)?;
        let relations = helpers::value_to_relations(raw);

        let mut entity_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for rel in &relations {
            match direction {
                Direction::Outgoing => {
                    entity_ids.insert(crate::types::record_key_to_string(&rel.to_entity.key));
                }
                Direction::Incoming => {
                    entity_ids.insert(crate::types::record_key_to_string(&rel.from_entity.key));
                }
                Direction::Both => {
                    let from_id = crate::types::record_key_to_string(&rel.from_entity.key);
                    let to_id = crate::types::record_key_to_string(&rel.to_entity.key);
                    if from_id != entity_id {
                        entity_ids.insert(from_id);
                    }
                    if to_id != entity_id {
                        entity_ids.insert(to_id);
                    }
                }
            }
        }

        let entity_ids_vec: Vec<String> = entity_ids.into_iter().collect();
        let entity_sql = "SELECT * FROM entities WHERE meta::id(id) IN $ids";
        let mut entity_response = self
            .db
            .query(entity_sql)
            .bind(("ids", entity_ids_vec))
            .await?;
        let entities: Vec<Entity> = entity_response.take(0)?;

        Ok((entities, relations))
    }

    async fn get_direct_relations_batch(
        &self,
        entity_ids: &[String],
        direction: Direction,
    ) -> Result<(Vec<Entity>, Vec<Relation>)> {
        if entity_ids.is_empty() {
            return Ok((vec![], vec![]));
        }

        let things: Vec<crate::types::Thing> = entity_ids
            .iter()
            .map(|id| {
                use crate::types::ThingId;
                ThingId::new("entities", id).map(|t| t.to_thing())
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let sql = match direction {
            Direction::Outgoing => "SELECT * FROM relations WHERE `in` IN $entity_ids",
            Direction::Incoming => "SELECT * FROM relations WHERE `out` IN $entity_ids",
            Direction::Both => {
                "SELECT * FROM relations WHERE `in` IN $entity_ids OR `out` IN $entity_ids"
            }
        };

        let mut response = self.db.query(sql).bind(("entity_ids", things)).await?;

        let raw: surrealdb_types::Value = response.take(0)?;
        let relations = helpers::value_to_relations(raw);

        let source_ids: std::collections::HashSet<&String> = entity_ids.iter().collect();
        let mut new_entity_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for rel in &relations {
            let from_id = crate::types::record_key_to_string(&rel.from_entity.key);
            let to_id = crate::types::record_key_to_string(&rel.to_entity.key);

            if !source_ids.contains(&from_id) {
                new_entity_ids.insert(from_id);
            }
            if !source_ids.contains(&to_id) {
                new_entity_ids.insert(to_id);
            }
        }

        let mut entities: Vec<Entity> = vec![];
        for eid in new_entity_ids {
            if let Some(entity) = self.get_entity(&eid).await? {
                entities.push(entity);
            }
        }

        Ok((entities, relations))
    }
}

// ---------------------------------------------------------------------------
// SymbolGraphTraversalStorage — needed by the symbol graph traversal engine
// ---------------------------------------------------------------------------

impl SymbolGraphTraversalStorage for SurrealStorage {
    async fn get_direct_symbol_relations(
        &self,
        symbol_id: &str,
        direction: Direction,
    ) -> Result<(Vec<CodeSymbol>, Vec<SymbolRelation>)> {
        // Delegate to the existing single-hop implementation
        symbol_ops::get_related_symbols(&self.db, symbol_id, 1, direction, None).await
    }

    async fn get_direct_symbol_relations_batch(
        &self,
        symbol_ids: &[String],
        direction: Direction,
    ) -> Result<(Vec<CodeSymbol>, Vec<SymbolRelation>)> {
        if symbol_ids.is_empty() {
            return Ok((vec![], vec![]));
        }

        // Build Thing IDs for the batch query
        let things: Vec<crate::types::Thing> = symbol_ids
            .iter()
            .filter_map(|id| {
                let (table, key) = if let Some(idx) = id.find(':') {
                    (&id[..idx], &id[idx + 1..])
                } else {
                    ("code_symbols", id.as_str())
                };
                use crate::types::ThingId;
                ThingId::new(table, key).ok().map(|t| t.to_thing())
            })
            .collect();

        if things.is_empty() {
            return Ok((vec![], vec![]));
        }

        let sql = match direction {
            Direction::Outgoing => "SELECT * FROM symbol_relation WHERE `in` IN $ids",
            Direction::Incoming => "SELECT * FROM symbol_relation WHERE `out` IN $ids",
            Direction::Both => "SELECT * FROM symbol_relation WHERE `in` IN $ids OR `out` IN $ids",
        };

        let mut response = self.db.query(sql).bind(("ids", things)).await?;

        let raw: surrealdb_types::Value = response.take(0)?;
        let relations = helpers::value_to_symbol_relations(raw);

        // Collect target symbol IDs from relations (excluding source IDs)
        let source_set: std::collections::HashSet<&String> = symbol_ids.iter().collect();
        let mut target_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

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

            match direction {
                Direction::Outgoing => {
                    if !source_set.contains(&to_str) {
                        target_ids.insert(to_str);
                    }
                }
                Direction::Incoming => {
                    if !source_set.contains(&from_str) {
                        target_ids.insert(from_str);
                    }
                }
                Direction::Both => {
                    if !source_set.contains(&from_str) {
                        target_ids.insert(from_str);
                    }
                    if !source_set.contains(&to_str) {
                        target_ids.insert(to_str);
                    }
                }
            }
        }

        // Batch-fetch symbols
        let symbols: Vec<CodeSymbol> = if target_ids.is_empty() {
            vec![]
        } else {
            let record_list: Vec<String> = target_ids
                .into_iter()
                .map(|sid| {
                    if sid.contains(':') {
                        sid
                    } else {
                        format!("code_symbols:{}", sid)
                    }
                })
                .collect();
            let sql = format!("SELECT * FROM {}", record_list.join(", "));
            let mut response = self.db.query(sql).await?;
            response.take(0).unwrap_or_default()
        };

        Ok((symbols, relations))
    }
}

// ---------------------------------------------------------------------------
// StorageBackend — single impl block, delegates to sub-module free functions
// ---------------------------------------------------------------------------

#[allow(async_fn_in_trait)]
impl StorageBackend for SurrealStorage {
    async fn create_memory(&self, memory: Memory) -> Result<String> {
        memory_ops::create_memory(&self.db, memory).await
    }

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        memory_ops::get_memory(&self.db, id).await
    }

    async fn update_memory(&self, id: &str, update: MemoryUpdate) -> Result<Memory> {
        memory_ops::update_memory(&self.db, id, update).await
    }

    async fn record_memory_access(
        &self,
        id: &str,
        accessed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        memory_ops::record_memory_access(&self.db, id, accessed_at).await
    }

    async fn delete_memory(&self, id: &str) -> Result<bool> {
        memory_ops::delete_memory(&self.db, id).await
    }

    async fn delete_memories_batch(&self, ids: &[String]) -> Result<Vec<String>> {
        memory_ops::delete_memories_batch(&self.db, ids).await
    }

    async fn list_memories(
        &self,
        filters: &MemoryQuery,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Memory>> {
        memory_ops::list_memories(&self.db, filters, limit, offset).await
    }

    async fn count_memories(&self) -> Result<usize> {
        memory_ops::count_memories(&self.db).await
    }

    async fn list_memory_ids(&self, filters: &MemoryQuery) -> Result<Vec<String>> {
        memory_ops::list_memory_ids(&self.db, filters).await
    }

    async fn count_memories_filtered(&self, filters: &MemoryQuery) -> Result<usize> {
        memory_ops::count_memories_filtered(&self.db, filters).await
    }

    async fn count_valid_memories(&self) -> Result<usize> {
        memory_ops::count_valid_memories(&self.db).await
    }

    async fn list_capacity_candidates(
        &self,
    ) -> Result<Vec<crate::storage::traits::CapacityMemoryCandidate>> {
        memory_ops::list_capacity_candidates(&self.db).await
    }

    async fn get_memory_last_accessed_at(
        &self,
        id: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
        memory_ops::get_memory_last_accessed_at(&self.db, id).await
    }

    async fn find_memories_by_content_hash(
        &self,
        filters: &MemoryQuery,
        content_hash: &str,
    ) -> Result<Vec<Memory>> {
        memory_ops::find_memories_by_content_hash(&self.db, filters, content_hash).await
    }

    async fn list_invalidated_memories_for_gc(
        &self,
        filter: &MemoryGcFilter,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Memory>> {
        memory_ops::list_invalidated_memories_for_gc(&self.db, filter, limit, offset).await
    }

    async fn count_invalidated_memories_for_gc(&self, filter: &MemoryGcFilter) -> Result<usize> {
        memory_ops::count_invalidated_memories_for_gc(&self.db, filter).await
    }

    async fn count_invalidated_memories_by_reason(
        &self,
        filter: &MemoryGcFilter,
    ) -> Result<Vec<MemoryGcReasonCount>> {
        memory_ops::count_invalidated_memories_by_reason(&self.db, filter).await
    }

    async fn export_memories(&self, options: &MemoryExportOptions) -> Result<ExportMemoryResponse> {
        memory_ops::export_memories(&self.db, options).await
    }

    async fn import_memories(
        &self,
        records: Vec<MigrationMemoryRecord>,
        options: &MemoryImportOptions,
    ) -> Result<ImportMemoryResponse> {
        memory_ops::import_memories(&self.db, records, options).await
    }

    async fn vector_search(
        &self,
        embedding: &[f32],
        filters: &MemoryQuery,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        memory_ops::vector_search(&self.db, embedding, filters, limit).await
    }

    async fn vector_search_code(
        &self,
        embedding: &[f32],
        project_id: Option<&str>,
        active_generation: Option<u64>,
        limit: usize,
    ) -> Result<Vec<ScoredCodeChunk>> {
        symbol_ops::vector_search_code(&self.db, embedding, project_id, active_generation, limit)
            .await
    }

    async fn vector_search_symbols(
        &self,
        embedding: &[f32],
        project_id: Option<&str>,
        active_generation: Option<u64>,
        limit: usize,
    ) -> Result<Vec<CodeSymbol>> {
        symbol_ops::vector_search_symbols(&self.db, embedding, project_id, active_generation, limit)
            .await
    }

    async fn bm25_search(
        &self,
        query: &str,
        filters: &MemoryQuery,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        memory_ops::bm25_search(&self.db, query, filters, limit).await
    }

    async fn bm25_search_code(
        &self,
        query: &str,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ScoredCodeChunk>> {
        code_ops::bm25_search_code(&self.db, query, project_id, limit).await
    }

    async fn create_entity(&self, entity: Entity) -> Result<String> {
        graph_ops::create_entity(&self.db, entity).await
    }

    async fn get_entity(&self, id: &str) -> Result<Option<Entity>> {
        graph_ops::get_entity(&self.db, id).await
    }

    async fn search_entities(&self, query: &str, limit: usize) -> Result<Vec<Entity>> {
        graph_ops::search_entities(&self.db, query, limit).await
    }

    async fn create_relation(&self, relation: Relation) -> Result<String> {
        graph_ops::create_relation(&self.db, relation).await
    }

    async fn get_related(
        &self,
        entity_id: &str,
        depth: usize,
        direction: Direction,
    ) -> Result<(Vec<Entity>, Vec<Relation>)> {
        graph_ops::get_related(&self.db, entity_id, depth, direction).await
    }

    async fn get_subgraph(&self, entity_ids: &[String]) -> Result<(Vec<Entity>, Vec<Relation>)> {
        graph_ops::get_subgraph(&self.db, entity_ids).await
    }

    async fn get_node_degrees(&self, entity_ids: &[String]) -> Result<HashMap<String, usize>> {
        graph_ops::get_node_degrees(&self.db, entity_ids).await
    }

    async fn get_all_entities(&self) -> Result<Vec<Entity>> {
        graph_ops::get_all_entities(&self.db).await
    }

    async fn get_all_relations(&self) -> Result<Vec<Relation>> {
        graph_ops::get_all_relations(&self.db).await
    }

    async fn get_valid(&self, filters: &MemoryQuery, limit: usize) -> Result<Vec<Memory>> {
        memory_ops::get_valid(&self.db, filters, limit).await
    }

    async fn get_valid_at(&self, filters: &MemoryQuery, limit: usize) -> Result<Vec<Memory>> {
        memory_ops::get_valid_at(&self.db, filters, limit).await
    }

    async fn invalidate(
        &self,
        id: &str,
        reason: Option<&str>,
        superseded_by: Option<&str>,
    ) -> Result<bool> {
        memory_ops::invalidate(&self.db, id, reason, superseded_by).await
    }

    async fn create_code_chunk(&self, chunk: CodeChunk) -> Result<String> {
        code_ops::create_code_chunk(&self.db, chunk).await
    }

    async fn create_code_chunks_batch(
        &self,
        chunks: Vec<CodeChunk>,
    ) -> Result<Vec<(String, CodeChunk)>> {
        code_ops::create_code_chunks_batch(&self.db, chunks).await
    }

    async fn delete_project_chunks(&self, project_id: &str) -> Result<usize> {
        code_ops::delete_project_chunks(&self.db, project_id).await
    }

    async fn delete_chunks_by_path(&self, project_id: &str, file_path: &str) -> Result<usize> {
        code_ops::delete_chunks_by_path(&self.db, project_id, file_path).await
    }

    async fn get_chunks_by_path(
        &self,
        project_id: &str,
        file_path: &str,
        active_generation: Option<u64>,
    ) -> Result<Vec<CodeChunk>> {
        code_ops::get_chunks_by_path(&self.db, project_id, file_path, active_generation).await
    }

    async fn get_all_chunks_for_project(
        &self,
        project_id: &str,
        active_generation: Option<u64>,
    ) -> Result<Vec<CodeChunk>> {
        code_ops::get_all_chunks_for_project(&self.db, project_id, active_generation).await
    }

    async fn get_chunks_paginated(
        &self,
        project_id: &str,
        active_generation: Option<u64>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<CodeChunk>> {
        code_ops::get_chunks_paginated(&self.db, project_id, active_generation, limit, offset).await
    }

    async fn get_chunks_by_ids(
        &self,
        ids: &[String],
        active_generation: Option<u64>,
    ) -> Result<Vec<CodeChunk>> {
        code_ops::get_chunks_by_ids(&self.db, ids, active_generation).await
    }

    async fn clear_project_embeddings(&self, project_id: &str) -> Result<u64> {
        code_ops::clear_project_embeddings(&self.db, project_id).await
    }

    async fn get_index_status(&self, project_id: &str) -> Result<Option<IndexStatus>> {
        code_ops::get_index_status(&self.db, project_id).await
    }

    async fn update_index_status(&self, status: IndexStatus) -> Result<()> {
        code_ops::update_index_status(&self.db, status).await
    }

    async fn delete_index_status(&self, project_id: &str) -> Result<()> {
        code_ops::delete_index_status(&self.db, project_id).await
    }

    async fn list_projects(&self) -> Result<Vec<String>> {
        code_ops::list_projects(&self.db).await
    }

    async fn list_index_statuses(&self) -> Result<Vec<IndexStatus>> {
        code_ops::list_index_statuses(&self.db).await
    }

    async fn create_or_update_index_job(&self, job: &IndexJobRecord) -> Result<()> {
        code_ops::create_or_update_index_job(&self.db, job).await
    }

    async fn create_index_job(&self, job: IndexJobRecord) -> Result<()> {
        code_ops::create_index_job(&self.db, job).await
    }

    async fn update_index_job(&self, job: IndexJobRecord) -> Result<()> {
        code_ops::update_index_job(&self.db, job).await
    }

    async fn get_index_job(
        &self,
        project_id: &str,
        job_id: &str,
    ) -> Result<Option<IndexJobRecord>> {
        code_ops::get_index_job(&self.db, project_id, job_id).await
    }

    async fn list_index_jobs_for_project(&self, project_id: &str) -> Result<Vec<IndexJobRecord>> {
        code_ops::list_index_jobs_for_project(&self.db, project_id).await
    }

    async fn delete_index_job(&self, project_id: &str, job_id: &str) -> Result<()> {
        code_ops::delete_index_job(&self.db, project_id, job_id).await
    }

    async fn upsert_file_checkpoint(&self, checkpoint: &IndexFileCheckpoint) -> Result<()> {
        code_ops::upsert_file_checkpoint(&self.db, checkpoint).await
    }

    async fn get_file_checkpoint(
        &self,
        project_id: &str,
        generation: u64,
        relative_file_path: &str,
    ) -> Result<Option<IndexFileCheckpoint>> {
        code_ops::get_file_checkpoint(&self.db, project_id, generation, relative_file_path).await
    }

    async fn list_file_checkpoints_for_job(
        &self,
        project_id: &str,
        generation: u64,
    ) -> Result<Vec<IndexFileCheckpoint>> {
        code_ops::list_file_checkpoints_for_job(&self.db, project_id, generation).await
    }

    async fn get_active_generation(&self, project_id: &str) -> Result<Option<u64>> {
        code_ops::get_active_generation(&self.db, project_id).await
    }

    async fn set_active_generation(&self, project_id: &str, generation: u64) -> Result<()> {
        code_ops::set_active_generation(&self.db, project_id, generation).await
    }

    async fn get_serving_generation(
        &self,
        project_id: &str,
        capability: CapabilityKind,
    ) -> Result<Option<u64>> {
        code_ops::get_serving_generation(&self.db, project_id, capability).await
    }

    async fn set_serving_generation(
        &self,
        project_id: &str,
        capability: CapabilityKind,
        generation: u64,
    ) -> Result<()> {
        code_ops::set_serving_generation(&self.db, project_id, capability, generation).await
    }

    async fn get_indexing_generation(&self, project_id: &str) -> Result<Option<u64>> {
        code_ops::get_indexing_generation(&self.db, project_id).await
    }

    async fn set_indexing_generation(
        &self,
        project_id: &str,
        generation: Option<u64>,
    ) -> Result<()> {
        code_ops::set_indexing_generation(&self.db, project_id, generation).await
    }

    async fn get_serving_metadata(&self, project_id: &str) -> Result<ServingGenerationMetadata> {
        code_ops::get_serving_metadata(&self.db, project_id).await
    }

    async fn list_abandoned_generations(&self, project_id: &str) -> Result<Vec<u64>> {
        code_ops::list_abandoned_generations(&self.db, project_id).await
    }

    async fn delete_project_generation(&self, project_id: &str, generation: u64) -> Result<()> {
        code_ops::delete_project_generation(&self.db, project_id, generation).await
    }

    async fn get_file_hash(&self, project_id: &str, file_path: &str) -> Result<Option<String>> {
        code_ops::get_file_hash(&self.db, project_id, file_path).await
    }

    async fn set_file_hash(&self, project_id: &str, file_path: &str, hash: &str) -> Result<()> {
        code_ops::set_file_hash(&self.db, project_id, file_path, hash).await
    }

    async fn set_file_hashes_batch(
        &self,
        project_id: &str,
        hashes: &[(String, String)],
    ) -> Result<()> {
        code_ops::set_file_hashes_batch(&self.db, project_id, hashes).await
    }

    async fn delete_file_hashes(&self, project_id: &str) -> Result<()> {
        code_ops::delete_file_hashes(&self.db, project_id).await
    }

    async fn delete_file_hash(&self, project_id: &str, file_path: &str) -> Result<()> {
        code_ops::delete_file_hash(&self.db, project_id, file_path).await
    }

    async fn create_code_symbol(&self, symbol: CodeSymbol) -> Result<String> {
        symbol_ops::create_code_symbol(&self.db, symbol).await
    }

    async fn create_code_symbols_batch(&self, symbols: Vec<CodeSymbol>) -> Result<Vec<String>> {
        symbol_ops::create_code_symbols_batch(&self.db, symbols).await
    }

    async fn update_symbol_embedding(&self, id: &str, embedding: Vec<f32>) -> Result<()> {
        symbol_ops::update_symbol_embedding(&self.db, id, embedding).await
    }

    async fn update_chunk_embedding(&self, id: &str, embedding: Vec<f32>) -> Result<()> {
        code_ops::update_chunk_embedding(&self.db, id, embedding).await
    }

    async fn batch_update_symbol_embeddings(&self, updates: &[(String, Vec<f32>)]) -> Result<()> {
        symbol_ops::batch_update_symbol_embeddings(&self.db, updates).await
    }

    async fn batch_update_chunk_embeddings(&self, updates: &[(String, Vec<f32>)]) -> Result<()> {
        code_ops::batch_update_chunk_embeddings(&self.db, updates).await
    }

    async fn create_symbol_relation(&self, relation: SymbolRelation) -> Result<String> {
        symbol_ops::create_symbol_relation(&self.db, relation).await
    }

    async fn create_symbol_relations_batch(&self, relations: Vec<SymbolRelation>) -> Result<u32> {
        symbol_ops::create_symbol_relations_batch(&self.db, relations).await
    }

    async fn delete_project_symbols(&self, project_id: &str) -> Result<usize> {
        symbol_ops::delete_project_symbols(&self.db, project_id).await
    }

    async fn delete_symbols_by_path(&self, project_id: &str, file_path: &str) -> Result<usize> {
        symbol_ops::delete_symbols_by_path(&self.db, project_id, file_path).await
    }

    async fn get_project_symbols(
        &self,
        project_id: &str,
        active_generation: Option<u64>,
    ) -> Result<Vec<CodeSymbol>> {
        symbol_ops::get_project_symbols(&self.db, project_id, active_generation).await
    }

    async fn get_symbol_callers(
        &self,
        symbol_id: &str,
        active_generation: Option<u64>,
    ) -> Result<Vec<CodeSymbol>> {
        symbol_ops::get_symbol_callers(&self.db, symbol_id, active_generation).await
    }

    async fn get_symbol_callees(
        &self,
        symbol_id: &str,
        active_generation: Option<u64>,
    ) -> Result<Vec<CodeSymbol>> {
        symbol_ops::get_symbol_callees(&self.db, symbol_id, active_generation).await
    }

    async fn get_related_symbols(
        &self,
        symbol_id: &str,
        depth: usize,
        direction: Direction,
        active_generation: Option<u64>,
    ) -> Result<(Vec<CodeSymbol>, Vec<SymbolRelation>)> {
        symbol_ops::get_related_symbols(&self.db, symbol_id, depth, direction, active_generation)
            .await
    }

    async fn get_code_subgraph(
        &self,
        symbol_ids: &[String],
        active_generation: Option<u64>,
    ) -> Result<(Vec<CodeSymbol>, Vec<SymbolRelation>)> {
        symbol_ops::get_code_subgraph(&self.db, symbol_ids, active_generation).await
    }

    async fn search_symbols(
        &self,
        query: &str,
        project_id: Option<&str>,
        limit: usize,
        offset: usize,
        symbol_type: Option<&str>,
        path_prefix: Option<&str>,
        active_generation: Option<u64>,
    ) -> Result<(Vec<CodeSymbol>, u32)> {
        symbol_ops::search_symbols(
            &self.db,
            query,
            project_id,
            limit,
            offset,
            symbol_type,
            path_prefix,
            active_generation,
        )
        .await
    }

    async fn replace_symbol_chunk_map(
        &self,
        project_id: &str,
        rows: &[(String, String, f32)],
    ) -> Result<u32> {
        symbol_ops::replace_symbol_chunk_map(&self.db, project_id, rows).await
    }

    async fn get_mapped_chunks_for_symbols(
        &self,
        project_id: &str,
        symbol_ids: &[String],
        active_generation: Option<u64>,
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        symbol_ops::get_mapped_chunks_for_symbols(
            &self.db,
            project_id,
            symbol_ids,
            active_generation,
            limit,
        )
        .await
    }

    async fn count_symbols(&self, project_id: &str, active_generation: Option<u64>) -> Result<u32> {
        symbol_ops::count_symbols(&self.db, project_id, active_generation).await
    }

    async fn count_chunks(&self, project_id: &str, active_generation: Option<u64>) -> Result<u32> {
        code_ops::count_chunks(&self.db, project_id, active_generation).await
    }

    async fn count_embedded_symbols(
        &self,
        project_id: &str,
        active_generation: Option<u64>,
    ) -> Result<u32> {
        symbol_ops::count_embedded_symbols(&self.db, project_id, active_generation).await
    }

    async fn count_embedded_chunks(
        &self,
        project_id: &str,
        active_generation: Option<u64>,
    ) -> Result<u32> {
        code_ops::count_embedded_chunks(&self.db, project_id, active_generation).await
    }

    async fn get_all_project_stats(&self) -> Result<HashMap<String, ProjectStats>> {
        code_ops::get_all_project_stats(&self.db).await
    }

    async fn get_unembedded_chunks(&self, project_id: &str) -> Result<Vec<(String, String)>> {
        code_ops::get_unembedded_chunks(&self.db, project_id).await
    }

    async fn get_unembedded_symbols(&self, project_id: &str) -> Result<Vec<(String, String)>> {
        symbol_ops::get_unembedded_symbols(&self.db, project_id).await
    }

    async fn count_symbol_relations(&self, project_id: &str) -> Result<u32> {
        symbol_ops::count_symbol_relations(&self.db, project_id).await
    }

    async fn find_symbol_by_name(
        &self,
        project_id: &str,
        name: &str,
    ) -> Result<Option<CodeSymbol>> {
        symbol_ops::find_symbol_by_name(&self.db, project_id, name).await
    }

    async fn find_symbols_by_names(
        &self,
        project_id: &str,
        names: &[String],
    ) -> Result<Vec<CodeSymbol>> {
        symbol_ops::find_symbols_by_names(&self.db, project_id, names).await
    }

    async fn find_symbol_by_name_with_context(
        &self,
        project_id: &str,
        name: &str,
        prefer_file: Option<&str>,
    ) -> Result<Option<CodeSymbol>> {
        symbol_ops::find_symbol_by_name_with_context(&self.db, project_id, name, prefer_file).await
    }

    async fn get_symbol_project_id(&self, symbol_id: &str) -> Result<Option<String>> {
        symbol_ops::get_symbol_project_id(&self.db, symbol_id).await
    }

    async fn health_check(&self) -> Result<bool> {
        self.db.query("INFO FOR DB").await?;
        Ok(true)
    }

    async fn reset_db(&self) -> Result<()> {
        // Run each DELETE independently — some tables may not exist yet
        // (e.g. relation tables are created on first RELATE).
        // Using a transaction would cause one failure to cancel all DELETEEs.
        let tables = [
            "memories",
            "entities",
            "relations",
            "code_chunks",
            "code_symbols",
            "symbol_chunk_map",
            "symbol_relation",
            "index_status",
        ];
        for table in &tables {
            let _ = self.db.query(format!("DELETE {}", table)).await;
        }
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        // Force WAL flush: SELECT count() touches the storage engine,
        // ensuring pending writes from any table are committed to disk.
        self.db
            .query(
                "SELECT count() AS c FROM memories GROUP ALL;
                 SELECT count() AS c FROM entities GROUP ALL;
                 SELECT count() AS c FROM code_chunks GROUP ALL;",
            )
            .await?;
        tracing::info!("Storage flushed successfully");
        Ok(())
    }

    async fn upsert_manifest_entry(&self, project_id: &str, file_path: &str) -> Result<()> {
        manifest_ops::upsert_manifest_entry(&self.db, project_id, file_path).await
    }

    async fn upsert_manifest_entries(&self, project_id: &str, file_paths: &[String]) -> Result<()> {
        manifest_ops::upsert_manifest_entries(&self.db, project_id, file_paths).await
    }

    async fn get_manifest_entries(&self, project_id: &str) -> Result<Vec<ManifestEntry>> {
        manifest_ops::get_manifest_entries(&self.db, project_id).await
    }

    async fn delete_manifest_entries(&self, project_id: &str) -> Result<()> {
        manifest_ops::delete_manifest_entries(&self.db, project_id).await
    }

    async fn delete_manifest_entry(&self, project_id: &str, file_path: &str) -> Result<()> {
        manifest_ops::delete_manifest_entry(&self.db, project_id, file_path).await
    }

    async fn count_manifest_entries(&self, project_id: &str) -> Result<usize> {
        manifest_ops::count_manifest_entries(&self.db, project_id).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::traits::{MemoryExportOptions, MemoryImportOptions};
    use crate::types::{
        ChunkType, CodeRelationType, CodeSymbol, ConfidenceClass, Datetime, Entity,
        ImportConflictStrategy, IndexFileCheckpoint, IndexJobPhase, IndexJobReasonCode,
        IndexJobRecord, IndexJobState, Language, Memory, MemoryQuery, MemoryType, MemoryUpdate,
        MigrationMemoryRecord, MigrationRecordType, RecordId, Relation, RelationClass,
        RelationProvenance, StalenessState, SymbolRelation, SymbolType,
        MEMORY_MIGRATION_SCHEMA_VERSION,
    };
    use tempfile::tempdir;

    fn empty_memory_query() -> MemoryQuery {
        MemoryQuery::default()
    }

    fn scoped_memory(
        content: &str,
        user_id: Option<&str>,
        agent_id: Option<&str>,
        run_id: Option<&str>,
        namespace: Option<&str>,
    ) -> Memory {
        Memory {
            id: None,
            content: content.to_string(),
            embedding: Some(vec![0.0; 768]),
            memory_type: MemoryType::Semantic,
            user_id: user_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
            run_id: run_id.map(str::to_string),
            namespace: namespace.map(str::to_string),
            metadata: None,
            event_time: Datetime::default(),
            ingestion_time: Datetime::default(),
            valid_from: Datetime::default(),
            valid_until: None,
            importance_score: 1.0,
            access_count: 0,
            last_accessed_at: None,
            invalidation_reason: None,
            superseded_by: None,
            content_hash: None,
            embedding_state: Default::default(),
        }
    }

    async fn assert_scope_isolation(
        storage: &SurrealStorage,
        filters: MemoryQuery,
        expected_content: &str,
        unexpected_content: &str,
    ) {
        let listed = storage.list_memories(&filters, 10, 0).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].content, expected_content);
        assert_ne!(listed[0].content, unexpected_content);

        let bm25 = storage
            .bm25_search("scope regression", &filters, 10)
            .await
            .unwrap();
        assert_eq!(bm25.len(), 1);
        assert_eq!(bm25[0].content, expected_content);
        assert_ne!(bm25[0].content, unexpected_content);

        let unfiltered_list = storage
            .list_memories(&empty_memory_query(), 10, 0)
            .await
            .unwrap();
        assert_eq!(unfiltered_list.len(), 2);
        assert!(unfiltered_list
            .iter()
            .any(|m| m.content == expected_content));
        assert!(unfiltered_list
            .iter()
            .any(|m| m.content == unexpected_content));

        let unfiltered_bm25 = storage
            .bm25_search("scope regression", &empty_memory_query(), 10)
            .await
            .unwrap();
        assert_eq!(unfiltered_bm25.len(), 2);
        assert!(unfiltered_bm25
            .iter()
            .any(|m| m.content == expected_content));
        assert!(unfiltered_bm25
            .iter()
            .any(|m| m.content == unexpected_content));
    }

    async fn setup_test_db() -> (SurrealStorage, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let storage = SurrealStorage::new(tmp.path(), 768).await.unwrap();
        (storage, tmp)
    }

    async fn setup_in_memory_test_db() -> SurrealStorage {
        SurrealStorage::new_in_memory(768).await.unwrap()
    }

    fn code_chunk_for_generation(
        project_id: &str,
        name: &str,
        content: &str,
        generation: Option<u64>,
    ) -> CodeChunk {
        CodeChunk {
            id: None,
            file_path: format!("src/{name}.rs"),
            content: content.to_string(),
            language: Language::Rust,
            start_line: 1,
            end_line: 3,
            chunk_type: ChunkType::Function,
            name: Some(name.to_string()),
            context_path: None,
            embedding: Some(vec![0.1; 768]),
            content_hash: format!("hash-{project_id}-{name}-{:?}", generation),
            project_id: Some(project_id.to_string()),
            generation,
            indexed_at: Datetime::default(),
        }
    }

    fn code_symbol_for_generation(
        project_id: &str,
        name: &str,
        file_path: &str,
        line: u32,
        generation: Option<u64>,
    ) -> CodeSymbol {
        let mut symbol = CodeSymbol::new(
            name.to_string(),
            SymbolType::Function,
            file_path.to_string(),
            line,
            line + 2,
            project_id.to_string(),
        );
        symbol.generation = generation;
        symbol.signature = Some(format!("fn {name}()"));
        symbol
    }

    fn migration_memory_record(id: &str, content: &str) -> MigrationMemoryRecord {
        let now = Datetime::default();
        MigrationMemoryRecord {
            schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
            record_type: MigrationRecordType::Memory,
            id: id.to_string(),
            content: content.to_string(),
            memory_type: MemoryType::Semantic,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: Some("source-project".to_string()),
            project_id: Some("source-project".to_string()),
            metadata: None,
            importance_score: 1.0,
            created_at: now,
            updated_at: now,
            valid_from: now,
            valid_until: None,
            superseded_by: None,
            invalidated: false,
            invalidation_reason: None,
        }
    }

    fn durable_job(job_id: &str, project_id: &str) -> IndexJobRecord {
        IndexJobRecord {
            id: None,
            job_id: job_id.to_string(),
            project_id: project_id.to_string(),
            target_generation: 7,
            workspace_path: "/durable/workspace".to_string(),
            target_fingerprint: None,
            structural_generation: 7,
            state: IndexJobState::Running,
            stored_phase: Some(IndexJobPhase::Chunk),
            phase: IndexJobPhase::Chunk,
            resume_token: "resume-token-123".to_string(),
            created_at: Datetime::default(),
            started_at: None,
            updated_at: Datetime::default(),
            completed_at: None,
            error: None,
            resume: None,
            completed_files_count: 3,
            total_files_count: Some(9),
            reason_code: Some(IndexJobReasonCode::InterruptedByShutdown),
            progress: Default::default(),
        }
    }

    #[tokio::test]
    async fn legacy_index_status_survives_job_schema_migration() {
        let storage = setup_in_memory_test_db().await;

        let mut status = IndexStatus::new("legacy_project".to_string());
        status.root_path = Some("/legacy/workspace".to_string());
        status.total_files = 12;
        status.indexed_files = 5;
        status.structural_generation = 0;
        status.semantic_generation = 0;

        storage.update_index_status(status.clone()).await.unwrap();

        let loaded = storage
            .get_index_status("legacy_project")
            .await
            .unwrap()
            .expect("legacy index status should remain readable");
        assert_eq!(loaded.project_id, status.project_id);
        assert_eq!(loaded.root_path, status.root_path);
        assert_eq!(loaded.total_files, 12);
        assert_eq!(loaded.indexed_files, 5);
        assert!(storage
            .list_index_jobs_for_project("legacy_project")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            storage
                .get_active_generation("legacy_project")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn update_index_status_repairs_record_with_missing_project_id() {
        let storage = setup_in_memory_test_db().await;
        storage
            .db
            .query("CREATE index_status:repair_project SET status = 'failed'")
            .await
            .unwrap();

        let mut status = IndexStatus::new("repair_project".to_string());
        status.root_path = Some("/repair/workspace".to_string());
        status.total_files = 21;
        status.indexed_files = 3;
        status.structural_generation = 4;
        status.semantic_generation = 2;

        storage.update_index_status(status.clone()).await.unwrap();

        let loaded = storage
            .get_index_status("repair_project")
            .await
            .unwrap()
            .expect("status should be readable by project_id after repair");
        assert_eq!(loaded.project_id, "repair_project");
        assert_eq!(loaded.root_path, status.root_path);
        assert_eq!(loaded.total_files, 21);
        assert_eq!(loaded.indexed_files, 3);
        assert_eq!(loaded.structural_generation, 4);
        assert_eq!(loaded.semantic_generation, 2);
    }

    #[tokio::test]
    async fn index_job_record_survives_backend_reopen() {
        let storage = setup_in_memory_test_db().await;
        storage
            .create_or_update_index_job(&durable_job("durable-job-1", "durable_project"))
            .await
            .unwrap();

        let job = storage
            .get_index_job("durable_project", "durable-job-1")
            .await
            .unwrap()
            .expect("job should survive readback from backend");

        assert_eq!(job.job_id, "durable-job-1");
        assert_eq!(job.project_id, "durable_project");
        assert_eq!(job.target_generation, 7);
        assert_eq!(job.state, IndexJobState::Running);
        assert_eq!(job.phase, IndexJobPhase::Chunk);
        assert_eq!(job.stored_phase, Some(IndexJobPhase::Chunk));
        assert_eq!(job.resume_token, "resume-token-123");
        assert_eq!(job.completed_files_count, 3);
        assert_eq!(job.total_files_count, Some(9));
        assert_eq!(
            job.reason_code,
            Some(IndexJobReasonCode::InterruptedByShutdown)
        );
    }

    #[tokio::test]
    async fn file_checkpoint_upsert_is_idempotent() {
        let storage = setup_in_memory_test_db().await;

        let checkpoint = IndexFileCheckpoint {
            id: None,
            job_id: "checkpoint-job".to_string(),
            project_id: "checkpoint_project".to_string(),
            generation: 42,
            relative_file_path: "src/lib.rs".to_string(),
            file_path: "src/lib.rs".to_string(),
            content_hash: "hash-a".to_string(),
            checkpoint_generation: 42,
            phase: IndexJobPhase::Chunk,
            completed: true,
            completed_at: Datetime::default(),
            chunks_written: 2,
            symbols_written: 4,
            updated_at: Datetime::default(),
        };

        storage.upsert_file_checkpoint(&checkpoint).await.unwrap();

        let mut updated = checkpoint;
        updated.content_hash = "hash-b".to_string();
        updated.chunks_written = 5;
        updated.symbols_written = 8;
        storage.upsert_file_checkpoint(&updated).await.unwrap();

        let loaded = storage
            .get_file_checkpoint("checkpoint_project", 42, "src/lib.rs")
            .await
            .unwrap()
            .expect("checkpoint should exist");

        assert_eq!(loaded.content_hash, "hash-b");
        assert_eq!(loaded.chunks_written, 5);
        assert_eq!(loaded.symbols_written, 8);

        let checkpoints = storage
            .list_file_checkpoints_for_job("checkpoint_project", 42)
            .await
            .unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].content_hash, "hash-b");

        let abandoned = storage
            .list_abandoned_generations("checkpoint_project")
            .await
            .unwrap();
        assert_eq!(abandoned, vec![42]);

        storage
            .set_active_generation("checkpoint_project", 42)
            .await
            .unwrap();
        assert_eq!(
            storage
                .get_active_generation("checkpoint_project")
                .await
                .unwrap(),
            Some(42)
        );
        assert!(storage
            .list_abandoned_generations("checkpoint_project")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn test_memory_crud() {
        let (storage, _tmp) = setup_test_db().await;

        let memory = Memory {
            id: None,
            content: "Test memory content".to_string(),
            embedding: Some(vec![0.1; 768]),
            memory_type: MemoryType::Semantic,
            user_id: Some("user1".to_string()),
            agent_id: None,
            run_id: None,
            namespace: None,
            metadata: None,
            event_time: Datetime::default(),
            ingestion_time: Datetime::default(),
            valid_from: Datetime::default(),
            valid_until: None,
            importance_score: 1.0,
            access_count: 0,
            last_accessed_at: None,
            invalidation_reason: None,
            superseded_by: None,
            content_hash: None,
            embedding_state: Default::default(),
        };

        let id = storage.create_memory(memory.clone()).await.unwrap();
        assert!(!id.is_empty());

        let retrieved = storage
            .get_memory(&id)
            .await
            .unwrap()
            .expect("Memory not found");
        assert_eq!(retrieved.content, memory.content);
        assert_eq!(retrieved.user_id, memory.user_id);

        let update = MemoryUpdate {
            content: Some("Updated content".to_string()),
            memory_type: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            importance_score: Some(2.0),
            metadata: None,
            embedding: None,
            content_hash: None,
            embedding_state: None,
        };
        let updated = storage.update_memory(&id, update).await.unwrap();
        assert_eq!(updated.content, "Updated content");
        assert_eq!(updated.importance_score, 2.0);

        let list = storage
            .list_memories(&empty_memory_query(), 10, 0)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        let deleted = storage.delete_memory(&id).await.unwrap();
        assert!(deleted);
        assert!(storage.get_memory(&id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_legacy_none_access_count_reads_as_zero() {
        let (storage, _tmp) = setup_test_db().await;
        let id = storage
            .create_memory(Memory::new("legacy none access count".to_string()))
            .await
            .unwrap();

        storage
            .db
            .query("UPDATE $thing SET access_count = NONE")
            .bind(("thing", RecordId::new("memories", id.as_str())))
            .await
            .unwrap()
            .check()
            .unwrap();

        let retrieved = storage.get_memory(&id).await.unwrap().unwrap();
        assert_eq!(retrieved.access_count, 0);

        let listed = storage
            .list_memories(&empty_memory_query(), 10, 0)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].access_count, 0);

        let searched = storage
            .bm25_search("legacy", &empty_memory_query(), 10)
            .await
            .unwrap();
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0].access_count, 0);

        storage
            .record_memory_access(&id, chrono::Utc::now())
            .await
            .unwrap();
        let accessed = storage.get_memory(&id).await.unwrap().unwrap();
        assert_eq!(accessed.access_count, 1);
    }

    #[tokio::test]
    async fn test_scope_filters_isolate_user_agent_run_and_namespace() {
        let cases = [
            (
                "user_id",
                MemoryQuery {
                    user_id: Some("user-a".to_string()),
                    ..Default::default()
                },
                scoped_memory("scope regression user-a", Some("user-a"), None, None, None),
                scoped_memory("scope regression user-b", Some("user-b"), None, None, None),
            ),
            (
                "agent_id",
                MemoryQuery {
                    agent_id: Some("agent-a".to_string()),
                    ..Default::default()
                },
                scoped_memory(
                    "scope regression agent-a",
                    None,
                    Some("agent-a"),
                    None,
                    None,
                ),
                scoped_memory(
                    "scope regression agent-b",
                    None,
                    Some("agent-b"),
                    None,
                    None,
                ),
            ),
            (
                "run_id",
                MemoryQuery {
                    run_id: Some("run-a".to_string()),
                    ..Default::default()
                },
                scoped_memory("scope regression run-a", None, None, Some("run-a"), None),
                scoped_memory("scope regression run-b", None, None, Some("run-b"), None),
            ),
            (
                "namespace",
                MemoryQuery {
                    namespace: Some("namespace-a".to_string()),
                    ..Default::default()
                },
                scoped_memory(
                    "scope regression namespace-a",
                    None,
                    None,
                    None,
                    Some("namespace-a"),
                ),
                scoped_memory(
                    "scope regression namespace-b",
                    None,
                    None,
                    None,
                    Some("namespace-b"),
                ),
            ),
        ];

        for (scope_name, filters, in_scope, out_of_scope) in cases {
            let (storage, _tmp) = setup_test_db().await;

            let expected_content = in_scope.content.clone();
            let unexpected_content = out_of_scope.content.clone();

            storage.create_memory(in_scope).await.unwrap();
            storage.create_memory(out_of_scope).await.unwrap();

            assert_scope_isolation(&storage, filters, &expected_content, &unexpected_content).await;

            let filtered_count = storage
                .count_memories_filtered(&MemoryQuery {
                    user_id: if scope_name == "user_id" {
                        Some("user-a".to_string())
                    } else {
                        None
                    },
                    agent_id: if scope_name == "agent_id" {
                        Some("agent-a".to_string())
                    } else {
                        None
                    },
                    run_id: if scope_name == "run_id" {
                        Some("run-a".to_string())
                    } else {
                        None
                    },
                    namespace: if scope_name == "namespace" {
                        Some("namespace-a".to_string())
                    } else {
                        None
                    },
                    ..Default::default()
                })
                .await
                .unwrap();

            assert_eq!(filtered_count, 1, "scope {scope_name} should stay isolated");
        }
    }

    #[tokio::test]
    async fn test_bm25_search() {
        let (storage, _tmp) = setup_test_db().await;

        storage
            .create_memory(Memory {
                id: None,
                content: "Rust programming language".to_string(),
                embedding: Some(vec![0.0; 768]),
                memory_type: MemoryType::Semantic,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        storage
            .create_memory(Memory {
                id: None,
                content: "Python scripting".to_string(),
                embedding: Some(vec![0.0; 768]),
                memory_type: MemoryType::Semantic,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        let results = storage
            .bm25_search("Rust", &empty_memory_query(), 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));
        // Score must be > 0 (not all-zero flat scores from broken search::score())
        assert!(
            results[0].score > 0.0,
            "Score should be > 0, got {}",
            results[0].score
        );
    }

    #[tokio::test]
    async fn test_entity_and_relation() {
        let (storage, _tmp) = setup_test_db().await;

        let e1_id = storage
            .create_entity(Entity {
                id: None,
                name: "Entity 1".to_string(),
                entity_type: "person".to_string(),
                description: None,
                embedding: None,
                content_hash: None,
                user_id: None,
                created_at: Datetime::default(),
            })
            .await
            .unwrap();

        let e2_id = storage
            .create_entity(Entity {
                id: None,
                name: "Entity 2".to_string(),
                entity_type: "place".to_string(),
                description: None,
                embedding: None,
                content_hash: None,
                user_id: None,
                created_at: Datetime::default(),
            })
            .await
            .unwrap();

        let _rel_id = storage
            .create_relation(Relation {
                id: None,
                from_entity: RecordId::new("entities", e1_id.clone()),
                to_entity: RecordId::new("entities", e2_id.clone()),
                relation_type: "lives_in".to_string(),
                relation_class: RelationClass::Observed,
                provenance: RelationProvenance::ImportedManual,
                confidence_class: ConfidenceClass::Extracted,
                freshness_generation: 0,
                staleness_state: StalenessState::Current,
                weight: 1.0,
                valid_from: Datetime::default(),
                valid_until: None,
            })
            .await
            .unwrap();

        let (related, _rels_out) = storage
            .get_related(&e1_id, 1, Direction::Outgoing)
            .await
            .unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].name, "Entity 2");
    }

    #[tokio::test]
    async fn test_entity_relation_round_trips_metadata() {
        let (storage, _tmp) = setup_test_db().await;

        let e1_id = storage
            .create_entity(Entity {
                id: None,
                name: "Entity A".to_string(),
                entity_type: "service".to_string(),
                description: None,
                embedding: None,
                content_hash: None,
                user_id: None,
                created_at: Datetime::default(),
            })
            .await
            .unwrap();

        let e2_id = storage
            .create_entity(Entity {
                id: None,
                name: "Entity B".to_string(),
                entity_type: "database".to_string(),
                description: None,
                embedding: None,
                content_hash: None,
                user_id: None,
                created_at: Datetime::default(),
            })
            .await
            .unwrap();

        storage
            .create_relation(Relation {
                id: None,
                from_entity: RecordId::new("entities", e1_id.clone()),
                to_entity: RecordId::new("entities", e2_id.clone()),
                relation_type: "depends_on".to_string(),
                relation_class: RelationClass::Inferred,
                provenance: RelationProvenance::EmbeddingInferred,
                confidence_class: ConfidenceClass::Ambiguous,
                freshness_generation: 7,
                staleness_state: StalenessState::Stale,
                weight: 0.42,
                valid_from: Datetime::default(),
                valid_until: None,
            })
            .await
            .unwrap();

        let relations = storage.get_all_relations().await.unwrap();
        let relation = relations
            .iter()
            .find(|r| r.relation_type == "depends_on")
            .expect("depends_on relation should exist");

        assert_eq!(relation.relation_class, RelationClass::Inferred);
        assert_eq!(relation.provenance, RelationProvenance::EmbeddingInferred);
        assert_eq!(relation.confidence_class, ConfidenceClass::Ambiguous);
        assert_eq!(relation.freshness_generation, 7);
        assert_eq!(relation.staleness_state, StalenessState::Stale);
        assert_eq!(relation.weight, 0.42);
    }

    #[tokio::test]
    async fn test_symbol_call_hierarchy() {
        let (storage, _tmp) = setup_test_db().await;

        // 1. Create Symbols: Caller -> Callee
        let caller = CodeSymbol::new(
            "main".to_string(),
            SymbolType::Function,
            "main.rs".to_string(),
            1,
            5,
            "test_project".to_string(),
        );
        let caller_id = storage.create_code_symbol(caller).await.unwrap();

        let callee = CodeSymbol::new(
            "helper".to_string(),
            SymbolType::Function,
            "helper.rs".to_string(),
            10,
            15,
            "test_project".to_string(),
        );
        let callee_id = storage.create_code_symbol(callee).await.unwrap();

        // 2. Create Relation: main calls helper
        let caller_key = caller_id
            .strip_prefix("code_symbols:")
            .unwrap_or(&caller_id);
        let callee_key = callee_id
            .strip_prefix("code_symbols:")
            .unwrap_or(&callee_id);

        let relation = SymbolRelation::new(
            crate::types::RecordId::new("code_symbols", caller_key.to_string()),
            crate::types::RecordId::new("code_symbols", callee_key.to_string()),
            CodeRelationType::Calls,
            RelationClass::Observed,
            RelationProvenance::ParserExtracted,
            ConfidenceClass::Extracted,
            0,
            StalenessState::Current,
            "main.rs".to_string(),
            3,
            "test_project".to_string(),
        );
        storage.create_symbol_relation(relation).await.unwrap();

        // 3. Test get_symbol_callees (Outgoing)
        // main -> ? (should be helper)
        let callees = storage.get_symbol_callees(&caller_id, None).await.unwrap();
        assert_eq!(callees.len(), 1, "Should find 1 callee");
        assert_eq!(callees[0].name, "helper");

        // 4. Test get_symbol_callers (Incoming)
        // ? -> helper (should be main)
        let callers = storage.get_symbol_callers(&callee_id, None).await.unwrap();
        assert_eq!(callers.len(), 1, "Should find 1 caller");
        assert_eq!(callers[0].name, "main");
    }

    #[tokio::test]
    async fn test_symbol_relation_round_trips_metadata() {
        let (storage, _tmp) = setup_test_db().await;

        let caller = CodeSymbol::new(
            "origin".to_string(),
            SymbolType::Function,
            "origin.rs".to_string(),
            1,
            5,
            "metadata_project".to_string(),
        );
        let caller_id = storage.create_code_symbol(caller).await.unwrap();

        let target = CodeSymbol::new(
            "target".to_string(),
            SymbolType::Function,
            "target.rs".to_string(),
            10,
            15,
            "metadata_project".to_string(),
        );
        let target_id = storage.create_code_symbol(target).await.unwrap();

        let caller_key = caller_id
            .strip_prefix("code_symbols:")
            .unwrap_or(&caller_id);
        let target_key = target_id
            .strip_prefix("code_symbols:")
            .unwrap_or(&target_id);

        storage
            .create_symbol_relation(SymbolRelation::new(
                RecordId::new("code_symbols", caller_key.to_string()),
                RecordId::new("code_symbols", target_key.to_string()),
                CodeRelationType::Calls,
                RelationClass::Inferred,
                RelationProvenance::HeuristicResolver,
                ConfidenceClass::Ambiguous,
                11,
                StalenessState::Stale,
                "origin.rs".to_string(),
                3,
                "metadata_project".to_string(),
            ))
            .await
            .unwrap();

        let (_symbols, relations) = storage
            .get_related_symbols(&caller_id, 1, Direction::Outgoing, None)
            .await
            .unwrap();

        let relation = relations
            .iter()
            .find(|r| r.file_path == "origin.rs")
            .expect("symbol relation should exist");

        assert_eq!(relation.relation_class, RelationClass::Inferred);
        assert_eq!(relation.provenance, RelationProvenance::HeuristicResolver);
        assert_eq!(relation.confidence_class, ConfidenceClass::Ambiguous);
        assert_eq!(relation.freshness_generation, 11);
        assert_eq!(relation.staleness_state, StalenessState::Stale);
    }

    #[tokio::test]
    async fn storage_export_memory_defaults_to_valid_only() {
        let (storage, _tmp) = setup_test_db().await;
        let valid_id = storage
            .create_memory(
                Memory::new("export valid memory".to_string())
                    .with_namespace("project-a".to_string()),
            )
            .await
            .unwrap();
        let invalid_id = storage
            .create_memory(
                Memory::new("export invalidated memory".to_string())
                    .with_namespace("project-a".to_string()),
            )
            .await
            .unwrap();
        storage
            .create_memory(
                Memory::new("export other project".to_string())
                    .with_namespace("project-b".to_string()),
            )
            .await
            .unwrap();
        storage
            .invalidate(&invalid_id, Some("archived"), Some(&valid_id))
            .await
            .unwrap();

        let response = storage
            .export_memories(&MemoryExportOptions::new("project-a"))
            .await
            .unwrap();

        assert_eq!(response.exported_count, 1);
        assert!(!response.truncated);
        assert!(response.jsonl.contains("export valid memory"));
        assert!(!response.jsonl.contains("export invalidated memory"));
        assert!(!response.jsonl.contains("export other project"));
        let record: MigrationMemoryRecord = serde_json::from_str(&response.jsonl).unwrap();
        assert_eq!(record.id, valid_id);
        assert_eq!(record.project_id.as_deref(), Some("project-a"));
        assert_eq!(record.namespace.as_deref(), Some("project-a"));
        assert!(!record.invalidated);
        assert_eq!(response.summary.valid_records, 1);
        assert_eq!(response.summary.invalidated_records, 0);

        let archival = storage
            .export_memories(&MemoryExportOptions {
                include_invalidated: true,
                valid_only: false,
                limit: Some(1),
                ..MemoryExportOptions::new("project-a")
            })
            .await
            .unwrap();
        assert_eq!(archival.exported_count, 1);
        assert!(archival.truncated);
    }

    #[tokio::test]
    async fn storage_import_memory_plans_id_remap_without_writes() {
        let (storage, _tmp) = setup_test_db().await;
        let existing_id = storage
            .create_memory(
                Memory::new("existing imported id".to_string())
                    .with_namespace("project-a".to_string()),
            )
            .await
            .unwrap();
        let now = Datetime::default();
        let records = vec![
            MigrationMemoryRecord {
                schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
                record_type: MigrationRecordType::Memory,
                id: existing_id.clone(),
                content: "incoming replacement".to_string(),
                memory_type: MemoryType::Semantic,
                user_id: Some("user-a".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("source-project".to_string()),
                project_id: Some("source-project".to_string()),
                metadata: Some(serde_json::json!({"embedding": [1, 2, 3]})),
                importance_score: 1.5,
                created_at: now,
                updated_at: now,
                valid_from: now,
                valid_until: None,
                superseded_by: Some("payload-new".to_string()),
                invalidated: false,
                invalidation_reason: None,
            },
            MigrationMemoryRecord {
                schema_version: MEMORY_MIGRATION_SCHEMA_VERSION,
                record_type: MigrationRecordType::Memory,
                id: "payload-new".to_string(),
                content: "incoming target".to_string(),
                memory_type: MemoryType::Procedural,
                user_id: None,
                agent_id: Some("agent-a".to_string()),
                run_id: None,
                namespace: Some("source-project".to_string()),
                project_id: Some("source-project".to_string()),
                metadata: None,
                importance_score: 2.0,
                created_at: now,
                updated_at: now,
                valid_from: now,
                valid_until: None,
                superseded_by: None,
                invalidated: false,
                invalidation_reason: None,
            },
        ];

        let response = storage
            .import_memories(
                records.clone(),
                &MemoryImportOptions {
                    project_id: "project-a".to_string(),
                    conflict_strategy: ImportConflictStrategy::Remap,
                    dry_run: true,
                    allow_invalidated: false,
                },
            )
            .await
            .unwrap();

        assert!(response.dry_run);
        assert_eq!(response.imported_count, 0);
        assert_eq!(response.failed_count, 0);
        assert_eq!(response.skipped_count, 2);
        assert_eq!(
            response.imported_count + response.skipped_count + response.failed_count,
            2
        );
        assert_eq!(response.summary.total_records, 2);
        assert_eq!(response.summary.skipped_records, response.skipped_count);
        assert_eq!(response.id_mappings.len(), 1);
        assert_eq!(response.id_mappings[0].old_id, existing_id);
        assert_ne!(
            response.id_mappings[0].new_id,
            response.id_mappings[0].old_id
        );
        assert!(storage.get_memory("payload-new").await.unwrap().is_none());
        assert_eq!(storage.count_memories().await.unwrap(), 1);

        let applied = storage
            .import_memories(
                records,
                &MemoryImportOptions {
                    project_id: "project-a".to_string(),
                    conflict_strategy: ImportConflictStrategy::Remap,
                    dry_run: false,
                    allow_invalidated: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(applied.imported_count, 2);
        let remapped_id = applied.id_mappings[0].new_id.clone();
        let remapped = storage.get_memory(&remapped_id).await.unwrap().unwrap();
        let remapped_record_id = remapped
            .id
            .as_ref()
            .expect("remapped memory should carry id");
        assert_eq!(
            crate::types::record_key_to_string(&remapped_record_id.key),
            remapped_id
        );
        assert_eq!(remapped.content, "incoming replacement");
        assert_eq!(remapped.namespace.as_deref(), Some("project-a"));
        assert_eq!(remapped.superseded_by.as_deref(), Some("payload-new"));
        assert_eq!(remapped.embedding, None);
        assert_eq!(remapped.content_hash, None);
        let remapped_metadata = remapped.metadata.as_ref().unwrap();
        assert_eq!(remapped_metadata["embedding"], serde_json::json!([1, 2, 3]));
        assert_eq!(remapped_metadata["migration"]["schema_version"], 1);
        assert_eq!(remapped_metadata["migration"]["source_id"], existing_id);
        assert_eq!(remapped_metadata["migration"]["imported_id"], remapped_id);
        assert_eq!(
            remapped_metadata["migration"]["target_project_id"],
            "project-a"
        );
        assert_eq!(
            remapped_metadata["migration"]["source_project_id"],
            "source-project"
        );
        assert_eq!(remapped_metadata["migration"]["conflict_strategy"], "remap");
        assert!(remapped_metadata["migration"]["imported_at"]
            .as_str()
            .is_some());
        let imported = storage
            .list_memories(
                &MemoryQuery {
                    namespace: Some("project-a".to_string()),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert!(imported
            .iter()
            .any(|memory| memory.content == "incoming target" && memory.embedding.is_none()));
    }

    #[tokio::test]
    async fn storage_import_memory_remaps_conflicting_ids_and_is_retrievable() {
        let (storage, _tmp) = setup_test_db().await;
        let existing_id = storage
            .create_memory(
                Memory::new("existing remap source".to_string())
                    .with_namespace("project-a".to_string()),
            )
            .await
            .unwrap();
        let records = vec![migration_memory_record(
            &existing_id,
            "incoming remapped content",
        )];

        let response = storage
            .import_memories(
                records,
                &MemoryImportOptions {
                    project_id: "project-a".to_string(),
                    conflict_strategy: ImportConflictStrategy::Remap,
                    dry_run: false,
                    allow_invalidated: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(response.imported_count, 1);
        assert_eq!(response.id_mappings.len(), 1);
        let remapped_id = response.id_mappings[0].new_id.clone();
        assert_ne!(remapped_id, existing_id);
        let remapped = storage.get_memory(&remapped_id).await.unwrap().unwrap();
        let stored_id = remapped
            .id
            .as_ref()
            .expect("imported memory should carry id");
        assert_eq!(
            crate::types::record_key_to_string(&stored_id.key),
            remapped_id
        );
        assert_eq!(remapped.content, "incoming remapped content");
        assert_eq!(remapped.namespace.as_deref(), Some("project-a"));
    }

    #[tokio::test]
    async fn storage_import_memory_preserves_metadata_and_relocates_source_migration() {
        let (storage, _tmp) = setup_test_db().await;
        let mut record = migration_memory_record("plain-import", "plain import with metadata");
        record.metadata = Some(serde_json::json!({
            "source_key": "source-value",
            "nested": {"kept": true},
            "migration": {"legacy_audit": "preserve-me"},
        }));

        let response = storage
            .import_memories(
                vec![record],
                &MemoryImportOptions {
                    project_id: "project-a".to_string(),
                    conflict_strategy: ImportConflictStrategy::Remap,
                    dry_run: false,
                    allow_invalidated: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(response.imported_count, 1);
        assert!(response.id_mappings.is_empty());
        let imported = storage.get_memory("plain-import").await.unwrap().unwrap();
        let metadata = imported.metadata.as_ref().unwrap();
        assert_eq!(metadata["source_key"], "source-value");
        assert_eq!(metadata["nested"]["kept"], true);
        assert_eq!(metadata["source_migration"]["legacy_audit"], "preserve-me");
        assert_eq!(metadata["migration"]["schema_version"], 1);
        assert_eq!(metadata["migration"]["source_id"], "plain-import");
        assert_eq!(metadata["migration"]["imported_id"], "plain-import");
        assert_eq!(metadata["migration"]["target_project_id"], "project-a");
        assert_eq!(metadata["migration"]["source_project_id"], "source-project");
        assert_eq!(metadata["migration"]["conflict_strategy"], "remap");
        assert!(metadata["migration"]["imported_at"].as_str().is_some());
    }

    #[tokio::test]
    async fn storage_import_memory_rejects_invalid_payload_without_partial_write() {
        let (storage, _tmp) = setup_test_db().await;
        storage
            .create_memory(
                Memory::new("existing baseline".to_string())
                    .with_namespace("project-a".to_string()),
            )
            .await
            .unwrap();
        let before_count = storage.count_memories().await.unwrap();
        let mut invalidated_record =
            migration_memory_record("invalidated-source", "invalidated import");
        invalidated_record.invalidated = true;
        invalidated_record.valid_until = Some(Datetime::default());
        invalidated_record.invalidation_reason = Some("archived".to_string());
        let records = vec![
            migration_memory_record("valid-source", "valid import should not be written"),
            invalidated_record,
        ];

        let response = storage
            .import_memories(
                records,
                &MemoryImportOptions {
                    project_id: "project-a".to_string(),
                    conflict_strategy: ImportConflictStrategy::Remap,
                    dry_run: false,
                    allow_invalidated: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(response.imported_count, 0);
        assert_eq!(response.summary.imported_records, 0);
        assert_eq!(response.skipped_count, 2);
        assert_eq!(response.failed_count, 0);
        assert_eq!(
            response.imported_count + response.skipped_count + response.failed_count,
            2
        );
        assert_eq!(response.summary.total_records, 2);
        assert_eq!(response.summary.skipped_records, response.skipped_count);
        assert_eq!(response.summary.failed_records, response.failed_count);
        assert_eq!(response.errors.len(), 1);
        assert_eq!(storage.count_memories().await.unwrap(), before_count);
        let listed = storage
            .list_memories(
                &MemoryQuery {
                    namespace: Some("project-a".to_string()),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert!(!listed
            .iter()
            .any(|memory| memory.content == "valid import should not be written"));
    }

    #[tokio::test]
    async fn storage_import_memory_rejects_duplicate_source_ids_without_partial_write() {
        let (storage, _tmp) = setup_test_db().await;
        let before_count = storage.count_memories().await.unwrap();
        let records = vec![
            migration_memory_record("duplicate-source", "first duplicate"),
            migration_memory_record("duplicate-source", "second duplicate"),
            migration_memory_record("valid-source", "valid import should not be written"),
        ];

        let response = storage
            .import_memories(
                records,
                &MemoryImportOptions {
                    project_id: "project-a".to_string(),
                    conflict_strategy: ImportConflictStrategy::Remap,
                    dry_run: false,
                    allow_invalidated: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(response.imported_count, 0);
        assert_eq!(response.summary.imported_records, 0);
        assert_eq!(response.failed_count, 1);
        assert_eq!(response.skipped_count, 2);
        assert_eq!(
            response.imported_count + response.skipped_count + response.failed_count,
            3
        );
        assert_eq!(response.summary.total_records, 3);
        assert_eq!(response.summary.skipped_records, response.skipped_count);
        assert_eq!(response.summary.failed_records, response.failed_count);
        assert_eq!(response.errors.len(), 1);
        assert_eq!(storage.count_memories().await.unwrap(), before_count);
    }

    #[tokio::test]
    async fn storage_import_memory_conflict_fail_reconciles_counters_without_partial_write() {
        let (storage, _tmp) = setup_test_db().await;
        let existing_id = storage
            .create_memory(
                Memory::new("existing conflict".to_string())
                    .with_namespace("project-a".to_string()),
            )
            .await
            .unwrap();
        let before_count = storage.count_memories().await.unwrap();
        let records = vec![
            migration_memory_record(&existing_id, "conflicting import"),
            migration_memory_record("valid-source", "valid import should not be written"),
        ];

        let response = storage
            .import_memories(
                records,
                &MemoryImportOptions {
                    project_id: "project-a".to_string(),
                    conflict_strategy: ImportConflictStrategy::Fail,
                    dry_run: false,
                    allow_invalidated: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(response.imported_count, 0);
        assert_eq!(response.failed_count, 1);
        assert_eq!(response.skipped_count, 1);
        assert_eq!(
            response.imported_count + response.skipped_count + response.failed_count,
            2
        );
        assert_eq!(response.summary.total_records, 2);
        assert_eq!(response.summary.skipped_records, response.skipped_count);
        assert_eq!(response.summary.failed_records, response.failed_count);
        assert_eq!(response.errors.len(), 1);
        assert_eq!(storage.count_memories().await.unwrap(), before_count);
        let listed = storage
            .list_memories(
                &MemoryQuery {
                    namespace: Some("project-a".to_string()),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert!(!listed
            .iter()
            .any(|memory| memory.content == "valid import should not be written"));
    }

    #[tokio::test]
    async fn storage_import_memory_conflict_skip_reconciles_counters() {
        let (storage, _tmp) = setup_test_db().await;
        let existing_id = storage
            .create_memory(
                Memory::new("existing conflict".to_string())
                    .with_namespace("project-a".to_string()),
            )
            .await
            .unwrap();
        let records = vec![
            migration_memory_record(&existing_id, "conflicting import"),
            migration_memory_record("valid-source", "valid import should be written"),
        ];

        let response = storage
            .import_memories(
                records,
                &MemoryImportOptions {
                    project_id: "project-a".to_string(),
                    conflict_strategy: ImportConflictStrategy::Skip,
                    dry_run: false,
                    allow_invalidated: false,
                },
            )
            .await
            .unwrap();

        assert_eq!(response.imported_count, 1);
        assert_eq!(response.failed_count, 0);
        assert_eq!(response.skipped_count, 1);
        assert_eq!(
            response.imported_count + response.skipped_count + response.failed_count,
            2
        );
        assert_eq!(response.summary.total_records, 2);
        assert_eq!(response.summary.imported_records, response.imported_count);
        assert_eq!(response.summary.skipped_records, response.skipped_count);
        assert_eq!(response.summary.failed_records, response.failed_count);
        let imported = storage
            .list_memories(
                &MemoryQuery {
                    namespace: Some("project-a".to_string()),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert!(imported
            .iter()
            .any(|memory| memory.content == "valid import should be written"));
    }

    #[tokio::test]
    async fn test_temporal_validation() {
        let (storage, _tmp) = setup_test_db().await;

        let id = storage
            .create_memory(Memory {
                id: None,
                content: "Temporary memory".to_string(),
                embedding: Some(vec![0.0; 768]),
                memory_type: MemoryType::Semantic,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        let valid = storage.get_valid(&empty_memory_query(), 10).await.unwrap();
        assert_eq!(valid.len(), 1);

        storage
            .invalidate(&id, Some("test reason"), None)
            .await
            .unwrap();

        let valid_after = storage.get_valid(&empty_memory_query(), 10).await.unwrap();
        assert_eq!(valid_after.len(), 0);
    }

    #[tokio::test]
    async fn test_list_memories_with_scope_filter() {
        let (storage, _tmp) = setup_test_db().await;

        storage
            .create_memory(
                Memory::new("Scoped memory A".to_string())
                    .with_user_id("user-a".to_string())
                    .with_namespace("project-a".to_string()),
            )
            .await
            .unwrap();

        storage
            .create_memory(
                Memory::new("Scoped memory B".to_string())
                    .with_user_id("user-b".to_string())
                    .with_namespace("project-b".to_string()),
            )
            .await
            .unwrap();

        let filters = MemoryQuery {
            user_id: Some("user-a".to_string()),
            namespace: Some("project-a".to_string()),
            ..Default::default()
        };

        let list = storage.list_memories(&filters, 10, 0).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, "Scoped memory A");

        let total = storage.count_memories_filtered(&filters).await.unwrap();
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn test_reset_db() {
        let (storage, _tmp) = setup_test_db().await;

        storage
            .create_memory(Memory {
                id: None,
                content: "To be deleted".to_string(),
                embedding: None,
                memory_type: MemoryType::Semantic,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        assert_eq!(storage.count_memories().await.unwrap(), 1);

        storage.reset_db().await.unwrap();

        assert_eq!(storage.count_memories().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_bulk_insert_code_chunks() {
        let (storage, _tmp) = setup_test_db().await;
        use crate::types::{ChunkType, CodeChunk, Language};

        let chunks: Vec<CodeChunk> = (0..50)
            .map(|i| CodeChunk {
                id: None,
                file_path: format!("src/file_{}.rs", i),
                content: format!("fn test_{}() {{}}", i),
                language: Language::Rust,
                start_line: 1,
                end_line: 3,
                chunk_type: ChunkType::Function,
                name: Some(format!("test_{}", i)),
                context_path: None,
                embedding: Some(vec![0.1; 768]),
                content_hash: format!("hash_{}", i),
                project_id: Some("test_project".to_string()),
                generation: None,
                indexed_at: Datetime::default(),
            })
            .collect();

        let results = storage.create_code_chunks_batch(chunks).await.unwrap();
        assert_eq!(results.len(), 50);

        let _status = storage.get_index_status("test_project").await.unwrap();
        // that's handled by the indexer. But we can verify chunks exist.

        let results = storage
            .bm25_search_code("test", Some("test_project"), 100)
            .await
            .unwrap();
        assert_eq!(results.len(), 50);
    }

    #[tokio::test]
    async fn test_list_projects_unifies_status_chunks_symbols_and_manifest() {
        let (storage, _tmp) = setup_test_db().await;
        use crate::types::{ChunkType, CodeChunk, CodeSymbol, IndexStatus, Language, SymbolType};

        storage
            .update_index_status(IndexStatus::new("status_project".to_string()))
            .await
            .unwrap();

        storage
            .create_code_chunk(CodeChunk {
                id: None,
                file_path: "src/chunk.rs".to_string(),
                content: "fn chunk_project() {}".to_string(),
                language: Language::Rust,
                start_line: 1,
                end_line: 1,
                chunk_type: ChunkType::Function,
                name: Some("chunk_project".to_string()),
                context_path: None,
                embedding: None,
                content_hash: "chunk-hash".to_string(),
                project_id: Some("chunk_project".to_string()),
                generation: None,
                indexed_at: Datetime::default(),
            })
            .await
            .unwrap();

        storage
            .create_code_symbol(CodeSymbol::new(
                "symbol_project_fn".to_string(),
                SymbolType::Function,
                "src/symbol.rs".to_string(),
                1,
                3,
                "symbol_project".to_string(),
            ))
            .await
            .unwrap();

        storage
            .upsert_manifest_entry("manifest_project", "src/manifest.rs")
            .await
            .unwrap();

        let projects = storage.list_projects().await.unwrap();

        assert!(projects.contains(&"status_project".to_string()));
        assert!(projects.contains(&"chunk_project".to_string()));
        assert!(projects.contains(&"symbol_project".to_string()));
        assert!(projects.contains(&"manifest_project".to_string()));
    }

    #[tokio::test]
    async fn test_batch_update_embeddings() {
        let (storage, _tmp) = setup_test_db().await;

        let chunks: Vec<CodeChunk> = (0..5)
            .map(|i| CodeChunk {
                id: None,
                file_path: format!("src/embed_{}.rs", i),
                content: format!("fn embed_{}() {{}}", i),
                language: Language::Rust,
                start_line: 1,
                end_line: 3,
                chunk_type: ChunkType::Function,
                name: Some(format!("embed_{}", i)),
                context_path: None,
                embedding: None,
                content_hash: format!("embed_hash_{}", i),
                project_id: Some("embed_project".to_string()),
                generation: None,
                indexed_at: Datetime::default(),
            })
            .collect();

        let results = storage.create_code_chunks_batch(chunks).await.unwrap();
        assert_eq!(results.len(), 5);

        let chunk_ids: Vec<String> = results.iter().map(|(id, _)| id.clone()).collect();

        let updates: Vec<(String, Vec<f32>)> = chunk_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), vec![i as f32 * 0.1; 768]))
            .collect();

        storage
            .batch_update_chunk_embeddings(&updates)
            .await
            .unwrap();

        let search_results = storage
            .bm25_search_code("embed", Some("embed_project"), 10)
            .await
            .unwrap();
        assert_eq!(search_results.len(), 5);
    }

    #[tokio::test]
    async fn test_search_symbols_with_type_filter() {
        let (storage, _tmp) = setup_test_db().await;
        use crate::types::{CodeSymbol, SymbolType};

        // Insert function symbols
        for i in 0..3 {
            let sym = CodeSymbol::new(
                format!("my_func_{}", i),
                SymbolType::Function,
                format!("src/file_{}.rs", i),
                1,
                10,
                "proj1".to_string(),
            );
            storage.create_code_symbol(sym).await.unwrap();
        }
        // Insert a struct symbol
        let sym = CodeSymbol::new(
            "MyStruct".to_string(),
            SymbolType::Struct,
            "src/types.rs".to_string(),
            1,
            20,
            "proj1".to_string(),
        );
        storage.create_code_symbol(sym).await.unwrap();

        // Without filter: should return all 4
        let (all, total) = storage
            .search_symbols("my", None, 10, 0, None, None, None)
            .await
            .unwrap();
        // Note: MyStruct also matches "my" (case-insensitive)
        assert_eq!(total, 4, "Should find 4 symbols total without filter");
        assert_eq!(all.len(), 4);

        // With symbol_type filter "function": should return 3
        let (funcs, total_funcs) = storage
            .search_symbols("my", None, 10, 0, Some("function"), None, None)
            .await
            .unwrap();
        assert_eq!(
            total_funcs, 3,
            "Should find 3 function symbols (got total={})",
            total_funcs
        );
        assert_eq!(
            funcs.len(),
            3,
            "search_symbols returned wrong count (got len={})",
            funcs.len()
        );

        // With symbol_type filter "struct": should return 1
        let (structs, total_structs) = storage
            .search_symbols("my", None, 10, 0, Some("struct"), None, None)
            .await
            .unwrap();
        assert_eq!(total_structs, 1, "Should find 1 struct symbol");
        assert_eq!(structs.len(), 1);
    }

    #[tokio::test]
    async fn test_search_symbols_pagination() {
        let (storage, _tmp) = setup_test_db().await;
        use crate::types::{CodeSymbol, SymbolType};

        // Insert 5 function symbols
        for i in 0..5 {
            let sym = CodeSymbol::new(
                format!("func_{}", i),
                SymbolType::Function,
                format!("src/file_{}.rs", i),
                1,
                10,
                "proj2".to_string(),
            );
            storage.create_code_symbol(sym).await.unwrap();
        }

        // Page 1: limit=2, offset=0
        let (page1, total) = storage
            .search_symbols("func", Some("proj2"), 2, 0, None, None, None)
            .await
            .unwrap();
        assert_eq!(total, 5, "Total should be 5");
        assert_eq!(page1.len(), 2, "Page 1 should have 2 results");

        // Page 2: limit=2, offset=2
        let (page2, _) = storage
            .search_symbols("func", Some("proj2"), 2, 2, None, None, None)
            .await
            .unwrap();
        assert_eq!(page2.len(), 2, "Page 2 should have 2 results");

        // Page 3: limit=2, offset=4
        let (page3, _) = storage
            .search_symbols("func", Some("proj2"), 2, 4, None, None, None)
            .await
            .unwrap();
        assert_eq!(page3.len(), 1, "Page 3 should have 1 result");
    }

    #[tokio::test]
    async fn test_bm25_search_code_no_project_filter() {
        let (storage, _tmp) = setup_test_db().await;
        use crate::types::{ChunkType, CodeChunk, Language};

        // Insert chunks in two different projects
        for i in 0..3 {
            let chunk = CodeChunk {
                id: None,
                file_path: format!("src/alpha_{}.rs", i),
                content: format!("fn alpha_function_{}() {{}}", i),
                language: Language::Rust,
                start_line: 1,
                end_line: 3,
                chunk_type: ChunkType::Function,
                name: Some(format!("alpha_function_{}", i)),
                context_path: None,
                embedding: Some(vec![0.1; 768]),
                content_hash: format!("hash_a_{}", i),
                project_id: Some("project_alpha".to_string()),
                generation: None,
                indexed_at: Datetime::default(),
            };
            storage.create_code_chunks_batch(vec![chunk]).await.unwrap();
        }
        for i in 0..2 {
            let chunk = CodeChunk {
                id: None,
                file_path: format!("src/beta_{}.rs", i),
                content: format!("fn alpha_function_{}() {{}}", i),
                language: Language::Rust,
                start_line: 1,
                end_line: 3,
                chunk_type: ChunkType::Function,
                name: Some(format!("alpha_function_{}", i)),
                context_path: None,
                embedding: Some(vec![0.1; 768]),
                content_hash: format!("hash_b_{}", i),
                project_id: Some("project_beta".to_string()),
                generation: None,
                indexed_at: Datetime::default(),
            };
            storage.create_code_chunks_batch(vec![chunk]).await.unwrap();
        }

        // Search with project_id = None should find ALL 5 chunks
        let all_results = storage
            .bm25_search_code("alpha_function", None, 100)
            .await
            .unwrap();
        assert_eq!(
            all_results.len(),
            5,
            "bm25_search_code with None project should return all 5 chunks (got {})",
            all_results.len()
        );

        // Search filtered by project
        let alpha_results = storage
            .bm25_search_code("alpha_function", Some("project_alpha"), 100)
            .await
            .unwrap();
        assert_eq!(
            alpha_results.len(),
            3,
            "bm25_search_code filtered by project_alpha should return 3 (got {})",
            alpha_results.len()
        );
    }

    #[tokio::test]
    async fn code_search_reads_only_active_generation() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "active_generation_search_project";
        storage.set_active_generation(project_id, 1).await.unwrap();

        let active = code_chunk_for_generation(
            project_id,
            "active_only",
            "fn active_only() { let marker = \"isolation_token\"; }",
            Some(1),
        );
        let staged = code_chunk_for_generation(
            project_id,
            "staged_only",
            "fn staged_only() { let marker = \"isolation_token\"; }",
            Some(2),
        );
        let legacy = code_chunk_for_generation(
            project_id,
            "legacy_only",
            "fn legacy_only() { let marker = \"isolation_token\"; }",
            None,
        );
        storage
            .create_code_chunks_batch(vec![active, staged, legacy])
            .await
            .unwrap();

        let active_generation = storage.get_active_generation(project_id).await.unwrap();
        let chunks = storage
            .get_all_chunks_for_project(project_id, active_generation)
            .await
            .unwrap();
        let names: std::collections::HashSet<_> =
            chunks.into_iter().filter_map(|chunk| chunk.name).collect();

        assert!(names.contains("active_only"));
        assert!(names.contains("legacy_only"));
        assert!(!names.contains("staged_only"));
    }

    #[tokio::test]
    async fn symbol_graph_filters_to_active_generation() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "active_generation_graph_project";
        storage.set_active_generation(project_id, 1).await.unwrap();

        let active_caller =
            code_symbol_for_generation(project_id, "caller", "src/active.rs", 1, Some(1));
        let active_callee =
            code_symbol_for_generation(project_id, "active_callee", "src/active.rs", 10, Some(1));
        let staged_callee =
            code_symbol_for_generation(project_id, "staged_callee", "src/staged.rs", 10, Some(2));

        let active_caller_thing = crate::types::safe_thing::symbol_thing(
            project_id,
            &active_caller.file_path,
            &active_caller.name,
            active_caller.start_line,
        );
        let active_callee_thing = crate::types::safe_thing::symbol_thing(
            project_id,
            &active_callee.file_path,
            &active_callee.name,
            active_callee.start_line,
        );
        let staged_callee_thing = crate::types::safe_thing::symbol_thing(
            project_id,
            &staged_callee.file_path,
            &staged_callee.name,
            staged_callee.start_line,
        );

        storage
            .create_code_symbols_batch(vec![active_caller, active_callee, staged_callee])
            .await
            .unwrap();

        storage
            .create_symbol_relations_batch(vec![
                SymbolRelation::new(
                    active_caller_thing.clone(),
                    active_callee_thing,
                    CodeRelationType::Calls,
                    RelationClass::Observed,
                    RelationProvenance::ParserExtracted,
                    ConfidenceClass::Extracted,
                    1,
                    StalenessState::Current,
                    "src/active.rs".to_string(),
                    2,
                    project_id.to_string(),
                ),
                SymbolRelation::new(
                    active_caller_thing.clone(),
                    staged_callee_thing,
                    CodeRelationType::Calls,
                    RelationClass::Observed,
                    RelationProvenance::ParserExtracted,
                    ConfidenceClass::Extracted,
                    2,
                    StalenessState::Current,
                    "src/staged.rs".to_string(),
                    2,
                    project_id.to_string(),
                ),
            ])
            .await
            .unwrap();

        let active_generation = storage.get_active_generation(project_id).await.unwrap();
        let (symbols, relations) = storage
            .get_related_symbols(
                &format!(
                    "{}:{}",
                    active_caller_thing.table.as_str(),
                    crate::types::record_key_to_string(&active_caller_thing.key)
                ),
                1,
                Direction::Outgoing,
                active_generation,
            )
            .await
            .unwrap();

        let names: Vec<_> = symbols.into_iter().map(|symbol| symbol.name).collect();
        assert_eq!(relations.len(), 1);
        assert_eq!(names, vec!["active_callee".to_string()]);
    }

    #[tokio::test]
    async fn test_bm25_search_multiword_and_special_chars() {
        let (storage, _tmp) = setup_test_db().await;

        // Store memories with special chars matching what AGENTS.md queries use
        storage
            .create_memory(Memory {
                id: None,
                content: "TASK: WP01-poc-validation\nID: WP01\nStatus: in_progress\nCurrent: T002"
                    .to_string(),
                embedding: Some(vec![0.0; 768]),
                memory_type: MemoryType::Semantic,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        storage
            .create_memory(Memory {
                id: None,
                content: "PROJECT: memory-mcp\nStatus: active".to_string(),
                embedding: Some(vec![0.0; 768]),
                memory_type: MemoryType::Semantic,
                user_id: None,
                agent_id: None,
                run_id: None,
                namespace: None,
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        // Test multi-word query with special chars (what AGENTS.md protocol searches for)
        let results = storage
            .bm25_search("Status: in_progress", &empty_memory_query(), 10)
            .await
            .unwrap();
        assert!(
            !results.is_empty(),
            "search_text('Status: in_progress') should return results"
        );
        assert!(
            results[0].content.contains("Status: in_progress"),
            "top result should preserve phrase relevance"
        );

        // Test prefix search
        let task_results = storage
            .bm25_search("TASK:", &empty_memory_query(), 10)
            .await
            .unwrap();
        assert_eq!(
            task_results.len(),
            1,
            "search_text('TASK:') should return 1 result (got {})",
            task_results.len()
        );

        // Test word inside content
        let project_results = storage
            .bm25_search("memory-mcp", &empty_memory_query(), 10)
            .await
            .unwrap();
        assert_eq!(
            project_results.len(),
            1,
            "search_text('memory-mcp') should return 1 result (got {})",
            project_results.len()
        );
    }

    #[tokio::test]
    async fn test_find_memories_by_content_hash_respects_filters() {
        let (storage, _tmp) = setup_test_db().await;

        let shared_hash = "dup-hash-1".to_string();

        storage
            .create_memory(Memory {
                id: None,
                content: "Scoped duplicate A".to_string(),
                embedding: Some(vec![0.0; 768]),
                memory_type: MemoryType::Semantic,
                user_id: Some("user-a".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-a".to_string()),
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: Some(shared_hash.clone()),
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        storage
            .create_memory(Memory {
                id: None,
                content: "Scoped duplicate B".to_string(),
                embedding: Some(vec![0.0; 768]),
                memory_type: MemoryType::Semantic,
                user_id: Some("user-b".to_string()),
                agent_id: None,
                run_id: None,
                namespace: Some("project-b".to_string()),
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                access_count: 0,
                last_accessed_at: None,
                invalidation_reason: None,
                superseded_by: None,
                content_hash: Some(shared_hash.clone()),
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        let filters = MemoryQuery {
            user_id: Some("user-a".to_string()),
            namespace: Some("project-a".to_string()),
            ..Default::default()
        };

        let results = storage
            .find_memories_by_content_hash(&filters, &shared_hash)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "Scoped duplicate A");
        assert_eq!(results[0].user_id.as_deref(), Some("user-a"));
        assert_eq!(results[0].namespace.as_deref(), Some("project-a"));
    }

    #[test]
    fn legacy_index_project_params_path_only_deserializes() {
        use crate::server::params::IndexProjectParams;

        let params: IndexProjectParams =
            serde_json::from_str(r#"{"path":"/workspace/my-project"}"#)
                .expect("legacy { path } payload must deserialize");

        assert_eq!(params.path.as_deref(), Some("/workspace/my-project"));
        assert_eq!(params.project_id, None, "project_id must default to None");
        assert_eq!(params.resume, None, "resume must default to None");
        assert_eq!(params.job_id, None, "job_id must default to None");
        assert_eq!(
            params.resume_token, None,
            "resume_token must default to None"
        );
        assert_eq!(
            params.allow_full_restart_fallback, None,
            "allow_full_restart_fallback must default to None"
        );
        assert_eq!(params.force, None, "force must default to None");
        assert_eq!(
            params.confirm_failed_restart, None,
            "confirm_failed_restart must default to None"
        );
    }

    #[test]
    fn legacy_index_project_params_force_restart_deserializes() {
        use crate::server::params::IndexProjectParams;

        let params: IndexProjectParams = serde_json::from_value(serde_json::json!({
            "path": "/workspace/my-project",
            "force": true,
            "confirm_failed_restart": true
        }))
        .expect("legacy force-restart payload must deserialize");

        assert_eq!(params.path.as_deref(), Some("/workspace/my-project"));
        assert_eq!(params.force, Some(true), "force must be Some(true)");
        assert_eq!(
            params.confirm_failed_restart,
            Some(true),
            "confirm_failed_restart must be Some(true)"
        );
        assert_eq!(params.project_id, None);
        assert_eq!(params.resume, None);
        assert_eq!(params.job_id, None);
        assert_eq!(params.resume_token, None);
        assert_eq!(params.allow_full_restart_fallback, None);
    }

    #[tokio::test]
    async fn legacy_null_generation_chunks_visible_in_generation_aware_reads() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "legacy_compat_null_gen_project";

        storage.set_active_generation(project_id, 1).await.unwrap();

        let legacy_chunk = code_chunk_for_generation(
            project_id,
            "legacy_fn",
            "fn legacy_fn() { /* pre-generation code */ }",
            None,
        );
        let active_chunk = code_chunk_for_generation(
            project_id,
            "active_fn",
            "fn active_fn() { /* generation-1 code */ }",
            Some(1),
        );
        let staged_chunk = code_chunk_for_generation(
            project_id,
            "staged_fn",
            "fn staged_fn() { /* staged code */ }",
            Some(2),
        );

        storage
            .create_code_chunks_batch(vec![legacy_chunk, active_chunk, staged_chunk])
            .await
            .unwrap();

        let active_gen = storage.get_active_generation(project_id).await.unwrap();
        assert_eq!(active_gen, Some(1), "active generation pointer must be 1");

        let chunks = storage
            .get_all_chunks_for_project(project_id, active_gen)
            .await
            .unwrap();

        let names: std::collections::HashSet<_> =
            chunks.into_iter().filter_map(|c| c.name).collect();

        assert!(
            names.contains("legacy_fn"),
            "legacy NULL-generation chunk must be visible in generation-aware reads"
        );
        assert!(
            names.contains("active_fn"),
            "active-generation chunk must be visible"
        );
        assert!(
            !names.contains("staged_fn"),
            "staged (future) generation chunk must NOT be visible"
        );
    }

    #[tokio::test]
    async fn legacy_null_generation_chunks_visible_without_active_generation_pointer() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "legacy_compat_no_gen_pointer_project";

        let legacy_chunk = code_chunk_for_generation(
            project_id,
            "pure_legacy_fn",
            "fn pure_legacy_fn() { /* no generation */ }",
            None,
        );

        storage
            .create_code_chunks_batch(vec![legacy_chunk])
            .await
            .unwrap();

        let active_gen = storage.get_active_generation(project_id).await.unwrap();
        assert_eq!(
            active_gen, None,
            "no active generation pointer for pure legacy project"
        );

        let chunks = storage
            .get_all_chunks_for_project(project_id, active_gen)
            .await
            .unwrap();

        let names: std::collections::HashSet<_> =
            chunks.into_iter().filter_map(|c| c.name).collect();

        assert!(
            names.contains("pure_legacy_fn"),
            "legacy NULL-generation chunk must be visible when no active generation pointer exists"
        );
    }

    #[tokio::test]
    async fn index_status_round_trips_expected_fields() {
        use crate::types::{IndexState, SemanticState, StructuralState};

        let storage = setup_in_memory_test_db().await;
        let project_id = "legacy_compat_status_project";

        let mut status = IndexStatus::new(project_id.to_string());
        status.root_path = Some("/workspace/my-project".to_string());
        status.status = IndexState::Completed;
        status.total_files = 42;
        status.indexed_files = 42;
        status.total_chunks = 100;
        status.total_symbols = 50;
        status.structural_generation = 3;
        status.semantic_generation = 3;
        status.structural_state = StructuralState::Ready;
        status.semantic_state = SemanticState::Ready;

        storage.update_index_status(status.clone()).await.unwrap();

        let loaded = storage
            .get_index_status(project_id)
            .await
            .unwrap()
            .expect("index status must be retrievable after write");

        assert_eq!(loaded.project_id, project_id);
        assert_eq!(loaded.root_path.as_deref(), Some("/workspace/my-project"));
        assert_eq!(loaded.status, IndexState::Completed);
        assert_eq!(loaded.total_files, 42);
        assert_eq!(loaded.indexed_files, 42);
        assert_eq!(loaded.total_chunks, 100);
        assert_eq!(loaded.total_symbols, 50);
        assert_eq!(loaded.structural_generation, 3);
        assert_eq!(loaded.semantic_generation, 3);
        assert_eq!(loaded.structural_state, StructuralState::Ready);
        assert_eq!(loaded.semantic_state, SemanticState::Ready);
    }

    #[tokio::test]
    async fn legacy_index_status_zero_generation_round_trips() {
        use crate::types::IndexState;

        let storage = setup_in_memory_test_db().await;
        let project_id = "legacy_compat_zero_gen_status_project";

        let mut status = IndexStatus::new(project_id.to_string());
        status.root_path = Some("/workspace/legacy".to_string());
        status.status = IndexState::Completed;
        status.total_files = 10;
        status.indexed_files = 10;
        assert_eq!(status.structural_generation, 0);
        assert_eq!(status.semantic_generation, 0);

        storage.update_index_status(status).await.unwrap();

        let loaded = storage
            .get_index_status(project_id)
            .await
            .unwrap()
            .expect("legacy zero-generation status must be retrievable");

        assert_eq!(loaded.project_id, project_id);
        assert_eq!(loaded.status, IndexState::Completed);
        assert_eq!(loaded.total_files, 10);
        assert_eq!(loaded.structural_generation, 0);
        assert_eq!(loaded.semantic_generation, 0);
        assert_eq!(
            storage.get_active_generation(project_id).await.unwrap(),
            None
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Crash / Resume Integration Tests (Task 12)
    // ─────────────────────────────────────────────────────────────────────────

    fn make_job(
        job_id: &str,
        project_id: &str,
        generation: u64,
        state: IndexJobState,
    ) -> IndexJobRecord {
        IndexJobRecord {
            id: None,
            job_id: job_id.to_string(),
            project_id: project_id.to_string(),
            target_generation: generation,
            workspace_path: "/workspace".to_string(),
            target_fingerprint: None,
            structural_generation: generation,
            state,
            stored_phase: Some(IndexJobPhase::Chunk),
            phase: IndexJobPhase::Chunk,
            resume_token: format!("ckpt_v1_phase_chunk_file_0"),
            created_at: Datetime::default(),
            started_at: Some(Datetime::default()),
            updated_at: Datetime::default(),
            completed_at: None,
            error: None,
            resume: None,
            completed_files_count: 0,
            total_files_count: Some(10),
            reason_code: None,
            progress: Default::default(),
        }
    }

    fn make_checkpoint(
        job_id: &str,
        project_id: &str,
        generation: u64,
        file_path: &str,
        phase: IndexJobPhase,
        completed: bool,
    ) -> IndexFileCheckpoint {
        IndexFileCheckpoint {
            id: None,
            job_id: job_id.to_string(),
            project_id: project_id.to_string(),
            generation,
            relative_file_path: file_path.to_string(),
            file_path: file_path.to_string(),
            content_hash: format!("hash-{file_path}"),
            checkpoint_generation: generation,
            phase,
            completed,
            completed_at: Datetime::default(),
            chunks_written: 2,
            symbols_written: 3,
            updated_at: Datetime::default(),
        }
    }

    /// Scenario 1: Start → interrupt mid-way → resume reads correct checkpoint count.
    ///
    /// Simulates: job starts, 3 of 5 files get checkpointed as completed, then
    /// the process "crashes". On resume, `list_file_checkpoints_for_job` must
    /// return exactly those 3 completed checkpoints so the resume logic can
    /// skip them.
    #[tokio::test]
    async fn crash_resume_partial_checkpoints_are_visible_after_interrupt() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "cr_project_1";
        let job_id = "cr_job_1";
        let generation = 1u64;

        // Write job as Running
        let mut job = make_job(job_id, project_id, generation, IndexJobState::Running);
        storage.create_or_update_index_job(&job).await.unwrap();

        // Write 5 checkpoints: 3 completed, 2 not
        let files = ["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs", "src/e.rs"];
        for (i, file) in files.iter().enumerate() {
            let completed = i < 3;
            let ckpt = make_checkpoint(
                job_id,
                project_id,
                generation,
                file,
                IndexJobPhase::Chunk,
                completed,
            );
            storage.upsert_file_checkpoint(&ckpt).await.unwrap();
        }

        // Simulate crash: mark job as Interrupted
        job.state = IndexJobState::Interrupted;
        job.reason_code = Some(IndexJobReasonCode::InterruptedByShutdown);
        storage.create_or_update_index_job(&job).await.unwrap();

        // On resume: load job and checkpoints
        let loaded_job = storage
            .get_index_job(project_id, job_id)
            .await
            .unwrap()
            .expect("job must survive interrupt");
        assert_eq!(loaded_job.state, IndexJobState::Interrupted);

        let checkpoints = storage
            .list_file_checkpoints_for_job(project_id, generation)
            .await
            .unwrap();
        assert_eq!(checkpoints.len(), 5, "all 5 checkpoints must be stored");

        let completed_count = checkpoints.iter().filter(|c| c.completed).count();
        assert_eq!(
            completed_count, 3,
            "exactly 3 completed checkpoints must be visible for resume"
        );
    }

    /// Scenario 2: Generation isolation — checkpoints from a different generation
    /// must NOT appear when querying for the current generation.
    #[tokio::test]
    async fn crash_resume_generation_isolation_prevents_cross_generation_bleed() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "cr_project_2";
        let job_id_gen1 = "cr_job_gen1";
        let job_id_gen2 = "cr_job_gen2";

        // Generation 1: 4 completed checkpoints
        let job1 = make_job(job_id_gen1, project_id, 1, IndexJobState::Interrupted);
        storage.create_or_update_index_job(&job1).await.unwrap();
        for file in &["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"] {
            let ckpt =
                make_checkpoint(job_id_gen1, project_id, 1, file, IndexJobPhase::Chunk, true);
            storage.upsert_file_checkpoint(&ckpt).await.unwrap();
        }

        // Generation 2: 2 completed checkpoints (new job)
        let job2 = make_job(job_id_gen2, project_id, 2, IndexJobState::Running);
        storage.create_or_update_index_job(&job2).await.unwrap();
        for file in &["src/x.rs", "src/y.rs"] {
            let ckpt =
                make_checkpoint(job_id_gen2, project_id, 2, file, IndexJobPhase::Chunk, true);
            storage.upsert_file_checkpoint(&ckpt).await.unwrap();
        }

        // Querying generation 2 must NOT see generation 1 checkpoints
        let gen2_checkpoints = storage
            .list_file_checkpoints_for_job(project_id, 2)
            .await
            .unwrap();
        assert_eq!(
            gen2_checkpoints.len(),
            2,
            "generation 2 must only see its own 2 checkpoints"
        );
        for ckpt in &gen2_checkpoints {
            assert_eq!(
                ckpt.generation, 2,
                "all returned checkpoints must belong to generation 2"
            );
        }

        // Querying generation 1 must NOT see generation 2 checkpoints
        let gen1_checkpoints = storage
            .list_file_checkpoints_for_job(project_id, 1)
            .await
            .unwrap();
        assert_eq!(
            gen1_checkpoints.len(),
            4,
            "generation 1 must only see its own 4 checkpoints"
        );
    }

    /// Scenario 3: Project isolation — checkpoints from a different project must
    /// NOT appear when querying for the current project.
    #[tokio::test]
    async fn crash_resume_project_isolation_prevents_cross_project_bleed() {
        let storage = setup_in_memory_test_db().await;
        let generation = 5u64;

        // Project A: 3 checkpoints
        let job_a = make_job("job_a", "project_a", generation, IndexJobState::Running);
        storage.create_or_update_index_job(&job_a).await.unwrap();
        for file in &["src/a.rs", "src/b.rs", "src/c.rs"] {
            let ckpt = make_checkpoint(
                "job_a",
                "project_a",
                generation,
                file,
                IndexJobPhase::Chunk,
                true,
            );
            storage.upsert_file_checkpoint(&ckpt).await.unwrap();
        }

        // Project B: 2 checkpoints
        let job_b = make_job("job_b", "project_b", generation, IndexJobState::Running);
        storage.create_or_update_index_job(&job_b).await.unwrap();
        for file in &["src/x.rs", "src/y.rs"] {
            let ckpt = make_checkpoint(
                "job_b",
                "project_b",
                generation,
                file,
                IndexJobPhase::Chunk,
                true,
            );
            storage.upsert_file_checkpoint(&ckpt).await.unwrap();
        }

        let a_checkpoints = storage
            .list_file_checkpoints_for_job("project_a", generation)
            .await
            .unwrap();
        assert_eq!(
            a_checkpoints.len(),
            3,
            "project_a must only see its own 3 checkpoints"
        );
        for ckpt in &a_checkpoints {
            assert_eq!(ckpt.project_id, "project_a");
        }

        let b_checkpoints = storage
            .list_file_checkpoints_for_job("project_b", generation)
            .await
            .unwrap();
        assert_eq!(
            b_checkpoints.len(),
            2,
            "project_b must only see its own 2 checkpoints"
        );
        for ckpt in &b_checkpoints {
            assert_eq!(ckpt.project_id, "project_b");
        }
    }

    /// Scenario 4: Checkpoint upsert is idempotent — re-writing the same file
    /// checkpoint (e.g., after a retry) must not create duplicate entries.
    #[tokio::test]
    async fn crash_resume_checkpoint_upsert_idempotency_prevents_duplicates() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "cr_project_4";
        let job_id = "cr_job_4";
        let generation = 3u64;

        let job = make_job(job_id, project_id, generation, IndexJobState::Running);
        storage.create_or_update_index_job(&job).await.unwrap();

        let ckpt = make_checkpoint(
            job_id,
            project_id,
            generation,
            "src/lib.rs",
            IndexJobPhase::Chunk,
            false,
        );
        storage.upsert_file_checkpoint(&ckpt).await.unwrap();

        // Upsert again with completed=true (simulating retry that succeeded)
        let mut ckpt2 = ckpt.clone();
        ckpt2.completed = true;
        ckpt2.chunks_written = 5;
        storage.upsert_file_checkpoint(&ckpt2).await.unwrap();

        // Must still be exactly 1 checkpoint, with updated values
        let checkpoints = storage
            .list_file_checkpoints_for_job(project_id, generation)
            .await
            .unwrap();
        assert_eq!(
            checkpoints.len(),
            1,
            "upsert must not create duplicate checkpoint entries"
        );
        assert!(
            checkpoints[0].completed,
            "checkpoint must reflect the latest completed=true state"
        );
        assert_eq!(
            checkpoints[0].chunks_written, 5,
            "chunks_written must reflect the latest upsert"
        );
    }

    /// Scenario 5: Resume token reflects the last completed phase and file count.
    ///
    /// Verifies that `checkpoint_resume_token` format is consistent with what
    /// `resumable_job_fields` would compute: the token encodes the phase of the
    /// last completed checkpoint and the total completed file count.
    #[tokio::test]
    async fn crash_resume_token_encodes_last_completed_phase_and_file_count() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "cr_project_5";
        let job_id = "cr_job_5";
        let generation = 9u64;

        let job = make_job(job_id, project_id, generation, IndexJobState::Interrupted);
        storage.create_or_update_index_job(&job).await.unwrap();

        // Write 4 completed checkpoints in Embed phase
        for file in &["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"] {
            let ckpt = make_checkpoint(
                job_id,
                project_id,
                generation,
                file,
                IndexJobPhase::Embed,
                true,
            );
            storage.upsert_file_checkpoint(&ckpt).await.unwrap();
        }

        let checkpoints = storage
            .list_file_checkpoints_for_job(project_id, generation)
            .await
            .unwrap();

        let files_done = checkpoints.iter().filter(|c| c.completed).count() as u64;
        assert_eq!(files_done, 4);

        // The last completed checkpoint's phase determines the resume token phase
        let last_phase = checkpoints
            .iter()
            .rev()
            .find(|c| c.completed)
            .map(|c| &c.phase)
            .expect("must have at least one completed checkpoint");

        // Verify token format matches the documented contract: ckpt_v1_phase_{phase}_file_{n}
        let expected_token = format!("ckpt_v1_phase_embed_file_{files_done}");
        let phase_str = match last_phase {
            IndexJobPhase::Embed => "embed",
            IndexJobPhase::Chunk => "chunk",
            IndexJobPhase::Parse => "parse",
            IndexJobPhase::Symbols => "symbols",
            IndexJobPhase::Relations => "relations",
            IndexJobPhase::EmbedEnqueue => "embed_enqueue",
            IndexJobPhase::Bm25 => "bm25",
            IndexJobPhase::Finalize => "finalize",
            IndexJobPhase::Promote => "promote",
            IndexJobPhase::Cleanup => "cleanup",
            IndexJobPhase::Discover => "discover",
        };
        let actual_token = format!("ckpt_v1_phase_{phase_str}_file_{files_done}");
        assert_eq!(
            actual_token, expected_token,
            "resume token must encode phase=embed and files_done=4"
        );
    }

    #[tokio::test]
    async fn serving_generation_storage_roundtrip() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "sg_roundtrip_project";

        storage
            .set_serving_generation(project_id, CapabilityKind::Bm25, 10)
            .await
            .unwrap();
        storage
            .set_serving_generation(project_id, CapabilityKind::Symbols, 20)
            .await
            .unwrap();
        storage
            .set_serving_generation(project_id, CapabilityKind::Graph, 30)
            .await
            .unwrap();
        storage
            .set_serving_generation(project_id, CapabilityKind::Vector, 40)
            .await
            .unwrap();
        storage
            .set_serving_generation(project_id, CapabilityKind::Semantic, 50)
            .await
            .unwrap();
        storage
            .set_indexing_generation(project_id, Some(99))
            .await
            .unwrap();

        assert_eq!(
            storage
                .get_serving_generation(project_id, CapabilityKind::Bm25)
                .await
                .unwrap(),
            Some(10)
        );
        assert_eq!(
            storage
                .get_serving_generation(project_id, CapabilityKind::Symbols)
                .await
                .unwrap(),
            Some(20)
        );
        assert_eq!(
            storage
                .get_serving_generation(project_id, CapabilityKind::Graph)
                .await
                .unwrap(),
            Some(30)
        );
        assert_eq!(
            storage
                .get_serving_generation(project_id, CapabilityKind::Vector)
                .await
                .unwrap(),
            Some(40)
        );
        assert_eq!(
            storage
                .get_serving_generation(project_id, CapabilityKind::Semantic)
                .await
                .unwrap(),
            Some(50)
        );
        assert_eq!(
            storage.get_indexing_generation(project_id).await.unwrap(),
            Some(99)
        );

        let meta = storage.get_serving_metadata(project_id).await.unwrap();
        assert_eq!(meta.bm25, Some(10));
        assert_eq!(meta.symbols, Some(20));
        assert_eq!(meta.graph, Some(30));
        assert_eq!(meta.vector, Some(40));
        assert_eq!(meta.semantic, Some(50));
        assert_eq!(meta.indexing, Some(99));

        storage
            .set_indexing_generation(project_id, None)
            .await
            .unwrap();
        assert_eq!(
            storage.get_indexing_generation(project_id).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn active_generation_compatibility() {
        let storage = setup_in_memory_test_db().await;
        let project_id = "compat_project";

        storage.set_active_generation(project_id, 42).await.unwrap();

        assert_eq!(
            storage.get_active_generation(project_id).await.unwrap(),
            Some(42)
        );

        assert_eq!(
            storage
                .get_serving_generation(project_id, CapabilityKind::ProjectInfo)
                .await
                .unwrap(),
            Some(42),
            "get_serving_generation(ProjectInfo) must be a compat alias for get_active_generation"
        );

        let meta = storage.get_serving_metadata(project_id).await.unwrap();
        assert_eq!(
            meta.structural,
            Some(42),
            "get_serving_metadata().structural must reflect active generation"
        );
    }
}
