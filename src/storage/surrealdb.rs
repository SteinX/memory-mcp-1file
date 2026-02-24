use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::types::Datetime;
use async_trait::async_trait;
use surrealdb::engine::local::{Db, SurrealKv};
use surrealdb::Surreal;

use super::StorageBackend;
use crate::graph::GraphTraversalStorage;
use crate::types::{
    CodeChunk, CodeRelationType, CodeSymbol, Direction, Entity, IndexStatus, Memory, MemoryUpdate,
    Relation, ScoredCodeChunk, SearchResult, SurrealValue, SymbolRelation,
};
use crate::Result;

pub struct SurrealStorage {
    db: Surreal<Db>,
}

impl SurrealStorage {
    pub async fn new(data_dir: &Path, model_dim: usize) -> Result<Self> {
        let db_path = data_dir.join("db");
        std::fs::create_dir_all(&db_path)?;

        let db: Surreal<Db> = Surreal::new::<SurrealKv>(db_path).await?;
        db.use_ns("memory").use_db("main").await?;

        // Drop old fulltext index on code_chunks that caused startup errors on existing databases
        db.query("REMOVE INDEX IF EXISTS idx_chunks_fts ON code_chunks;")
            .await?;

        let schema = include_str!("schema.surql").replace("{dim}", &model_dim.to_string());
        db.query(&schema).await?;

        Ok(Self { db })
    }

    pub async fn check_dimension(&self, expected: usize) -> Result<()> {
        let mut response = self.db.query("INFO FOR TABLE memories").await?;
        let result: Option<serde_json::Value> = response.take(0)?;

        if let Some(info) = result {
            if let Some(indexes) = info.get("indexes").and_then(|i| i.as_object()) {
                if let Some(idx_def) = indexes.get("idx_memories_vec").and_then(|v| v.as_str()) {
                    if let Some(dim) = self.extract_dimension(idx_def) {
                        if dim != expected {
                            tracing::warn!(
                                old = dim,
                                new = expected,
                                "Dimension mismatch detected, rebuilding vector indices"
                            );
                            self.rebuild_vector_indices(expected).await?;
                            self.db
                                .query(
                                    "UPDATE memories SET embedding_state = 'stale', embedding = NONE;
                                     UPDATE entities SET embedding = NONE;
                                     UPDATE code_chunks SET embedding = NONE;
                                     UPDATE code_symbols SET embedding = NONE;",
                                )
                                .await?;
                            tracing::info!("Indices rebuilt, old embeddings marked stale");
                            return Ok(());
                        }
                        tracing::info!(model = expected, db = dim, "Dimension check passed");
                        return Ok(());
                    }
                }
            }
        }

        Ok(())
    }

    async fn rebuild_vector_indices(&self, dim: usize) -> Result<()> {
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
}

fn generate_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tid = std::thread::current().id();
    let input = format!("{}-{}-{:?}-{}", now, std::process::id(), tid, seq);
    let hash = blake3::hash(input.as_bytes());
    hash.to_hex()[..20].to_string()
}

fn parse_thing(id: &str) -> crate::Result<crate::types::Thing> {
    if let Some((table, key)) = id.split_once(':') {
        Ok(crate::types::RecordId::new(
            table.to_string(),
            key.to_string(),
        ))
    } else {
        Err(crate::AppError::Database(format!(
            "Invalid thing ID format: {}",
            id
        )))
    }
}

/// SurrealDB v3 workaround: The SurrealValue derive macro generates `from_value()`
/// that fails with "Expected any, got record" when a struct contains RecordId fields.
/// Also, serde_json intermediary fails because Value serializes with Rust enum wrappers.
/// This helper manually extracts fields from Value::Object to construct Relation.
fn value_to_relations(value: surrealdb_types::Value) -> Vec<Relation> {
    use surrealdb_types::Value;

    let arr = match value {
        Value::Array(arr) => arr.into_vec(),
        Value::None | Value::Null => return vec![],
        other => vec![other],
    };

    let mut relations = Vec::with_capacity(arr.len());
    for item in arr {
        if let Value::Object(obj) = item {
            // Extract RecordId fields
            let id = obj.get("id").and_then(|v| {
                if let Value::RecordId(r) = v {
                    Some(r.clone())
                } else {
                    None
                }
            });
            let from_entity = match obj.get("in") {
                Some(Value::RecordId(r)) => r.clone(),
                _ => continue,
            };
            let to_entity = match obj.get("out") {
                Some(Value::RecordId(r)) => r.clone(),
                _ => continue,
            };
            // Extract string fields
            let relation_type = match obj.get("relation_type") {
                Some(Value::String(s)) => s.to_string(),
                _ => continue,
            };
            // Extract weight
            let weight = match obj.get("weight") {
                Some(Value::Number(n)) => n.to_f64().unwrap_or(1.0) as f32,
                _ => 1.0,
            };
            // Extract datetimes
            let valid_from = match obj.get("valid_from") {
                Some(Value::Datetime(d)) => *d,
                _ => Default::default(),
            };
            let valid_until = match obj.get("valid_until") {
                Some(Value::Datetime(d)) => Some(*d),
                _ => None,
            };

            relations.push(Relation {
                id,
                from_entity,
                to_entity,
                relation_type,
                weight,
                valid_from,
                valid_until,
            });
        }
    }
    relations
}

/// Same workaround for SymbolRelation which also has RecordId fields (in/out).
fn value_to_symbol_relations(value: surrealdb_types::Value) -> Vec<SymbolRelation> {
    use surrealdb_types::Value;

    let arr = match value {
        Value::Array(arr) => arr.into_vec(),
        Value::None | Value::Null => return vec![],
        other => vec![other],
    };

    let mut relations = Vec::with_capacity(arr.len());
    for item in arr {
        if let Value::Object(obj) = item {
            let id = obj.get("id").and_then(|v| {
                if let Value::RecordId(r) = v {
                    Some(r.clone())
                } else {
                    None
                }
            });
            let from_symbol = match obj.get("in") {
                Some(Value::RecordId(r)) => r.clone(),
                _ => continue,
            };
            let to_symbol = match obj.get("out") {
                Some(Value::RecordId(r)) => r.clone(),
                _ => continue,
            };
            let relation_type_str = match obj.get("relation_type") {
                Some(Value::String(s)) => s.to_string(),
                _ => continue,
            };
            let relation_type: CodeRelationType =
                serde_json::from_value(serde_json::Value::String(relation_type_str.clone()))
                    .unwrap_or(CodeRelationType::Calls);
            let file_path = match obj.get("file_path") {
                Some(Value::String(s)) => s.to_string(),
                _ => String::new(),
            };
            let line_number = match obj.get("line_number") {
                Some(Value::Number(n)) => n.to_f64().unwrap_or(0.0) as u32,
                _ => 0,
            };
            let project_id = match obj.get("project_id") {
                Some(Value::String(s)) => s.to_string(),
                _ => String::new(),
            };
            let created_at = match obj.get("created_at") {
                Some(Value::Datetime(d)) => *d,
                _ => Default::default(),
            };

            relations.push(SymbolRelation {
                id,
                from_symbol,
                to_symbol,
                relation_type,
                file_path,
                line_number,
                project_id,
                created_at,
            });
        }
    }
    relations
}

#[async_trait]
impl GraphTraversalStorage for SurrealStorage {
    async fn get_direct_relations(
        &self,
        entity_id: &str,
        direction: Direction,
    ) -> Result<(Vec<Entity>, Vec<Relation>)> {
        use crate::types::ThingId;

        let entity_thing = ThingId::new("entities", entity_id)?.to_string();

        let sql = match direction {
            Direction::Outgoing => "SELECT * FROM relations WHERE `in` = type::record($entity_id)",
            Direction::Incoming => "SELECT * FROM relations WHERE `out` = type::record($entity_id)",
            Direction::Both => {
                "SELECT * FROM relations WHERE `in` = type::record($entity_id) OR `out` = type::record($entity_id)"
            }
        };

        let mut response = self
            .db
            .query(sql)
            .bind(("entity_id", entity_thing.clone()))
            .await?;

        // Use Value intermediary to bypass SurrealValue RecordId bug
        let raw: surrealdb_types::Value = response.take(0)?;
        let relations = value_to_relations(raw);

        let mut entity_ids: HashSet<String> = HashSet::new();
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
        let relations = value_to_relations(raw);

        let source_ids: HashSet<&String> = entity_ids.iter().collect();
        let mut new_entity_ids: HashSet<String> = HashSet::new();

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

#[async_trait]
impl StorageBackend for SurrealStorage {
    async fn create_memory(&self, mut memory: Memory) -> Result<String> {
        let id = generate_id();
        memory.id = Some(crate::types::RecordId::new("memories", id.as_str()));
        let _: Option<Memory> = self
            .db
            .create(("memories", id.as_str()))
            .content(memory)
            .await?;
        Ok(id)
    }

    async fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        let result: Option<Memory> = self.db.select(("memories", id)).await?;
        Ok(result)
    }

    async fn update_memory(&self, id: &str, update: MemoryUpdate) -> Result<Memory> {
        let existing: Option<Memory> = self.db.select(("memories", id)).await?;
        let mut memory =
            existing.ok_or_else(|| crate::types::AppError::NotFound(id.to_string()))?;

        if let Some(content) = update.content {
            memory.content = content;
        }
        if let Some(memory_type) = update.memory_type {
            memory.memory_type = memory_type;
        }
        if let Some(metadata) = update.metadata {
            memory.metadata = Some(metadata);
        }

        let updated: Option<Memory> = self.db.update(("memories", id)).content(memory).await?;
        updated.ok_or_else(|| crate::types::AppError::NotFound(id.to_string()))
    }

    async fn delete_memory(&self, id: &str) -> Result<bool> {
        let deleted: Option<Memory> = self.db.delete(("memories", id)).await?;
        Ok(deleted.is_some())
    }

    async fn list_memories(&self, limit: usize, offset: usize) -> Result<Vec<Memory>> {
        let query =
            "SELECT * FROM memories ORDER BY ingestion_time DESC LIMIT $limit START $offset";
        let mut response = self
            .db
            .query(query)
            .bind(("limit", limit))
            .bind(("offset", offset))
            .await?;
        let memories: Vec<Memory> = response.take(0)?;
        Ok(memories)
    }

    async fn count_memories(&self) -> Result<usize> {
        let mut response = self
            .db
            .query("SELECT count() FROM memories GROUP ALL")
            .await?;
        let result: Option<serde_json::Value> = response.take(0)?;
        let count = result
            .and_then(|v| v.get("count").and_then(|c| c.as_u64()))
            .unwrap_or(0) as usize;
        Ok(count)
    }

    async fn vector_search(&self, embedding: &[f32], limit: usize) -> Result<Vec<SearchResult>> {
        let query = r#"
            SELECT meta::id(id) AS id, content, memory_type,
                vector::similarity::cosine(embedding, $vec) AS score, metadata 
            FROM memories 
            WHERE embedding IS NOT NONE 
              AND (valid_until IS NONE OR valid_until > time::now())
            ORDER BY score DESC 
            LIMIT $limit
        "#;
        let mut response = self
            .db
            .query(query)
            .bind(("vec", embedding.to_vec()))
            .bind(("limit", limit))
            .await?;
        let results: Vec<SearchResult> = response.take(0)?;
        Ok(results)
    }

    async fn vector_search_code(
        &self,
        embedding: &[f32],
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ScoredCodeChunk>> {
        let query = r#"
            SELECT 
                meta::id(id) AS id,
                file_path,
                content,
                language,
                start_line,
                end_line,
                chunk_type,
                name,
                vector::similarity::cosine(embedding, $vec) AS score 
            FROM code_chunks
            WHERE embedding IS NOT NONE
              AND ($project_id IS NONE OR project_id = $project_id)
            ORDER BY score DESC 
            LIMIT $limit
        "#;
        let mut response = self
            .db
            .query(query)
            .bind(("vec", embedding.to_vec()))
            .bind(("project_id", project_id.map(String::from)))
            .bind(("limit", limit))
            .await?;
        let results: Vec<ScoredCodeChunk> = response.take(0)?;
        Ok(results)
    }

    async fn vector_search_symbols(
        &self,
        embedding: &[f32],
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CodeSymbol>> {
        let sql = r#"
            SELECT *,
                vector::similarity::cosine(embedding, $vec) AS _score
            FROM code_symbols
            WHERE embedding IS NOT NONE
              AND ($project_id IS NONE OR project_id = $project_id)
            ORDER BY _score DESC
            LIMIT $limit
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("vec", embedding.to_vec()))
            .bind(("project_id", project_id.map(String::from)))
            .bind(("limit", limit))
            .await?;
        let results: Vec<CodeSymbol> = response.take(0)?;
        Ok(results)
    }

    async fn bm25_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        // TODO: SurrealDB v3.0.0 FULLTEXT @@ + search::score(0) is broken.
        // Revert to @0@ + search::score(0) when fixed upstream.
        let sql = r#"
            SELECT meta::id(id) AS id, content, memory_type, 1.0f AS score, metadata 
            FROM memories 
            WHERE string::lowercase(content) CONTAINS string::lowercase($query)
              AND (valid_until IS NONE OR valid_until > time::now())
            LIMIT $limit
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("query", query.to_string()))
            .bind(("limit", limit))
            .await?;
        let results: Vec<SearchResult> = response.take(0)?;
        Ok(results)
    }

    async fn bm25_search_code(
        &self,
        query: &str,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ScoredCodeChunk>> {
        // TODO: SurrealDB v3.0.0 FULLTEXT @@ + search::score(0) is broken.
        let sql = r#"
            SELECT 
                meta::id(id) AS id,
                file_path,
                content,
                language,
                start_line,
                end_line,
                chunk_type,
                name,
                1.0f AS score 
            FROM code_chunks 
            WHERE string::lowercase(content) CONTAINS string::lowercase($query)
              AND ($project_id IS NONE OR project_id = $project_id)
            LIMIT $limit
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("query", query.to_string()))
            .bind(("project_id", project_id.map(String::from)))
            .bind(("limit", limit))
            .await?;
        let results: Vec<ScoredCodeChunk> = response.take(0)?;
        Ok(results)
    }

    async fn create_entity(&self, mut entity: Entity) -> Result<String> {
        let id = generate_id();
        entity.id = Some(crate::types::RecordId::new("entities", id.as_str()));
        let _: Option<Entity> = self
            .db
            .create(("entities", id.as_str()))
            .content(entity)
            .await?;
        Ok(id)
    }

    async fn get_entity(&self, id: &str) -> Result<Option<Entity>> {
        let result: Option<Entity> = self.db.select(("entities", id)).await?;
        Ok(result)
    }

    async fn search_entities(&self, query: &str, limit: usize) -> Result<Vec<Entity>> {
        // TODO: SurrealDB v3.0.0 FULLTEXT @@ + search::score(0) is broken.
        let sql = r#"
            SELECT * 
            FROM entities 
            WHERE string::lowercase(name) CONTAINS string::lowercase($query)
            LIMIT $limit
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("query", query.to_string()))
            .bind(("limit", limit))
            .await?;
        let results: Vec<Entity> = response.take(0)?;
        Ok(results)
    }

    async fn create_relation(&self, relation: Relation) -> Result<String> {
        use crate::types::ThingId;

        let id = generate_id();
        let from_thing = ThingId::new(
            relation.from_entity.table.as_str(),
            &crate::types::record_key_to_string(&relation.from_entity.key),
        )?;
        let to_thing = ThingId::new(
            relation.to_entity.table.as_str(),
            &crate::types::record_key_to_string(&relation.to_entity.key),
        )?;

        // SurrealDB v3: RELATE with bound RecordId causes "Expected any, got record",
        // CREATE on TYPE RELATION tables causes "not a relation" error.
        // Use inline RELATE with validated ThingId (SQL injection safe).
        let sql = format!(
            "RELATE {}->relations->{} SET relation_type = $rel_type, weight = $weight",
            from_thing, to_thing
        );

        let _response = self
            .db
            .query(&sql)
            .bind(("rel_type", relation.relation_type))
            .bind(("weight", relation.weight))
            .await?;

        // Skip response check — v3 RELATE returns record types

        Ok(id)
    }

    async fn get_related(
        &self,
        entity_id: &str,
        depth: usize,
        direction: Direction,
    ) -> Result<(Vec<Entity>, Vec<Relation>)> {
        use crate::graph::GraphTraverser;

        let traverser = GraphTraverser::new(self);
        let result = traverser.traverse(entity_id, depth, direction).await?;

        Ok((result.entities, result.relations))
    }

    async fn get_subgraph(&self, entity_ids: &[String]) -> Result<(Vec<Entity>, Vec<Relation>)> {
        use crate::types::ThingId;

        if entity_ids.is_empty() {
            return Ok((vec![], vec![]));
        }

        let validated_ids: Vec<ThingId> = entity_ids
            .iter()
            .map(|id| ThingId::new("entities", id))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let ids: Vec<crate::types::Thing> = validated_ids.iter().map(|t| t.to_thing()).collect();

        let sql = "SELECT * FROM relations WHERE in IN $ids AND out IN $ids";
        let mut response = self.db.query(sql).bind(("ids", ids.clone())).await?;
        let raw: surrealdb_types::Value = response.take(0)?;
        let relations = value_to_relations(raw);

        let entity_sql = "SELECT * FROM entities WHERE id IN $ids";
        let mut entity_response = self.db.query(entity_sql).bind(("ids", ids)).await?;
        let entities: Vec<Entity> = entity_response.take(0)?;

        Ok((entities, relations))
    }

    async fn get_node_degrees(&self, entity_ids: &[String]) -> Result<HashMap<String, usize>> {
        use crate::types::ThingId;

        if entity_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let things: Vec<String> = entity_ids
            .iter()
            .filter_map(|id| ThingId::new("entities", id).ok().map(|t| t.to_string()))
            .collect();

        // Single batch query for all degrees
        let sql = r#"
            SELECT meta::id(`in`.id) AS node, count() AS degree FROM relations
            WHERE `in` IN $ids OR `out` IN $ids
            GROUP BY node
        "#;

        let mut response = self.db.query(sql).bind(("ids", things)).await?;

        #[derive(serde::Deserialize, SurrealValue)]
        struct DegreeResult {
            node: String,
            degree: u64,
        }

        let results: Vec<DegreeResult> = response.take(0).unwrap_or_default();
        let mut degrees: HashMap<String, usize> =
            entity_ids.iter().map(|id| (id.clone(), 0)).collect();
        for r in results {
            degrees.insert(r.node, r.degree as usize);
        }
        Ok(degrees)
    }

    async fn get_all_entities(&self) -> Result<Vec<Entity>> {
        let mut response = self.db.query("SELECT * FROM entities").await?;
        let entities: Vec<Entity> = response.take(0)?;
        Ok(entities)
    }

    async fn get_all_relations(&self) -> Result<Vec<Relation>> {
        let mut response = self.db.query("SELECT * FROM relations").await?;
        let raw: surrealdb_types::Value = response.take(0)?;
        let relations = value_to_relations(raw);
        Ok(relations)
    }

    async fn get_valid(&self, user_id: Option<&str>, limit: usize) -> Result<Vec<Memory>> {
        let sql = r#"
            SELECT * FROM memories 
            WHERE (valid_until IS NONE OR valid_until > time::now())
              AND ($user_id IS NONE OR user_id = $user_id)
            ORDER BY ingestion_time DESC
            LIMIT $limit
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("user_id", user_id.map(String::from)))
            .bind(("limit", limit))
            .await?;
        let memories: Vec<Memory> = response.take(0)?;
        Ok(memories)
    }

    async fn get_valid_at(
        &self,
        timestamp: Datetime,
        user_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let sql = r#"
            SELECT * FROM memories 
            WHERE valid_from <= $timestamp 
              AND (valid_until IS NONE OR valid_until > $timestamp)
              AND ($user_id IS NONE OR user_id = $user_id)
            ORDER BY ingestion_time DESC
            LIMIT $limit
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("timestamp", timestamp))
            .bind(("user_id", user_id.map(String::from)))
            .bind(("limit", limit))
            .await?;
        let memories: Vec<Memory> = response.take(0)?;
        Ok(memories)
    }

    async fn invalidate(
        &self,
        id: &str,
        reason: Option<&str>,
        superseded_by: Option<&str>,
    ) -> Result<bool> {
        let thing = crate::types::RecordId::new("memories", id);
        let sql = r#"
            UPDATE $thing SET 
                valid_until = time::now(),
                invalidation_reason = $reason,
                superseded_by = $superseded_by
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("thing", thing))
            .bind(("reason", reason.map(String::from)))
            .bind(("superseded_by", superseded_by.map(String::from)))
            .await?;
        let updated: Option<Memory> = response.take(0).ok().flatten();
        Ok(updated.is_some())
    }

    async fn create_code_chunk(&self, mut chunk: CodeChunk) -> Result<String> {
        let id = generate_id();
        chunk.id = Some(crate::types::RecordId::new("code_chunks", id.as_str()));
        let _: Option<CodeChunk> = self
            .db
            .create(("code_chunks", id.as_str()))
            .content(chunk)
            .await?;
        Ok(id)
    }

    async fn create_code_chunks_batch(
        &self,
        mut chunks: Vec<CodeChunk>,
    ) -> Result<Vec<(String, CodeChunk)>> {
        let count = chunks.len();
        if count == 0 {
            return Ok(vec![]);
        }

        for chunk in &mut chunks {
            if chunk.id.is_none() {
                let id = generate_id();
                chunk.id = Some(crate::types::RecordId::new("code_chunks", id.as_str()));
            }
        }

        let created: Vec<CodeChunk> = self.db.insert("code_chunks").content(chunks).await?;

        let pairs = created
            .into_iter()
            .filter_map(|c| {
                c.id.as_ref().map(|t| {
                    (
                        format!(
                            "{}:{}",
                            t.table.as_str(),
                            crate::types::record_key_to_string(&t.key)
                        ),
                        c.clone(),
                    )
                })
            })
            .collect();

        Ok(pairs)
    }

    async fn delete_project_chunks(&self, project_id: &str) -> Result<usize> {
        let sql = "DELETE FROM code_chunks WHERE project_id = $project_id RETURN BEFORE";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;
        let deleted: Vec<CodeChunk> = response.take(0).unwrap_or_default();
        Ok(deleted.len())
    }

    async fn delete_chunks_by_path(&self, project_id: &str, file_path: &str) -> Result<usize> {
        let sql = "DELETE FROM code_chunks WHERE project_id = $project_id AND file_path = $file_path RETURN BEFORE";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .bind(("file_path", file_path.to_string()))
            .await?;
        let deleted: Vec<CodeChunk> = response.take(0).unwrap_or_default();
        Ok(deleted.len())
    }

    async fn get_chunks_by_path(
        &self,
        project_id: &str,
        file_path: &str,
    ) -> Result<Vec<CodeChunk>> {
        let sql =
            "SELECT * FROM code_chunks WHERE project_id = $project_id AND file_path = $file_path";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .bind(("file_path", file_path.to_string()))
            .await?;
        let chunks: Vec<CodeChunk> = response.take(0).unwrap_or_default();
        Ok(chunks)
    }

    async fn get_index_status(&self, project_id: &str) -> Result<Option<IndexStatus>> {
        let sql = "SELECT * FROM index_status WHERE project_id = $project_id LIMIT 1";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;
        let result: Vec<IndexStatus> = response.take(0).unwrap_or_default();
        Ok(result.into_iter().next())
    }

    async fn update_index_status(&self, status: IndexStatus) -> Result<()> {
        let sql = r#"
            UPDATE index_status SET 
                status = $status,
                total_files = $total_files,
                indexed_files = $indexed_files,
                total_chunks = $total_chunks,
                total_symbols = $total_symbols,
                started_at = $started_at,
                completed_at = $completed_at,
                error_message = $error_message,
                failed_files = $failed_files,
                failed_embeddings = $failed_embeddings
            WHERE project_id = $project_id
        "#;

        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", status.project_id.clone()))
            .bind(("status", status.status.clone()))
            .bind(("total_files", status.total_files))
            .bind(("indexed_files", status.indexed_files))
            .bind(("total_chunks", status.total_chunks))
            .bind(("total_symbols", status.total_symbols))
            .bind(("started_at", status.started_at))
            .bind(("completed_at", status.completed_at))
            .bind(("error_message", status.error_message.clone()))
            .bind(("failed_files", status.failed_files.clone()))
            .bind(("failed_embeddings", status.failed_embeddings))
            .await?;

        let updated: Vec<IndexStatus> = response.take(0).unwrap_or_default();

        if updated.is_empty() {
            let id = ("index_status", status.project_id.as_str());
            let _: Option<IndexStatus> = self.db.create(id).content(status).await?;
        }

        Ok(())
    }

    async fn delete_index_status(&self, project_id: &str) -> Result<()> {
        let sql = "DELETE FROM index_status WHERE project_id = $project_id";
        self.db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;
        Ok(())
    }

    async fn list_projects(&self) -> Result<Vec<String>> {
        let sql = "SELECT project_id FROM code_chunks GROUP BY project_id";
        let mut response = self.db.query(sql).await?;
        let results: Vec<serde_json::Value> = response.take(0).unwrap_or_default();
        let projects = results
            .into_iter()
            .filter_map(|v| {
                v.get("project_id")
                    .and_then(|p| p.as_str())
                    .map(String::from)
            })
            .collect();
        Ok(projects)
    }

    async fn get_file_hash(&self, project_id: &str, file_path: &str) -> Result<Option<String>> {
        let sql = "SELECT content_hash FROM file_hashes WHERE project_id = $project_id AND file_path = $file_path LIMIT 1";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .bind(("file_path", file_path.to_string()))
            .await?;
        let result: Vec<serde_json::Value> = response.take(0).unwrap_or_default();
        Ok(result.into_iter().next().and_then(|v| {
            v.get("content_hash")
                .and_then(|h| h.as_str())
                .map(String::from)
        }))
    }

    async fn set_file_hash(&self, project_id: &str, file_path: &str, hash: &str) -> Result<()> {
        let sql = r#"
            UPSERT file_hashes SET
                project_id = $project_id,
                file_path = $file_path,
                content_hash = $hash,
                indexed_at = time::now()
            WHERE project_id = $project_id AND file_path = $file_path
        "#;
        self.db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .bind(("file_path", file_path.to_string()))
            .bind(("hash", hash.to_string()))
            .await?;
        Ok(())
    }

    async fn delete_file_hashes(&self, project_id: &str) -> Result<()> {
        let sql = "DELETE FROM file_hashes WHERE project_id = $project_id";
        self.db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;
        Ok(())
    }

    async fn delete_file_hash(&self, project_id: &str, file_path: &str) -> Result<()> {
        let sql =
            "DELETE FROM file_hashes WHERE project_id = $project_id AND file_path = $file_path";
        self.db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .bind(("file_path", file_path.to_string()))
            .await?;
        Ok(())
    }

    async fn create_code_symbol(&self, mut symbol: CodeSymbol) -> Result<String> {
        let key = symbol.unique_key();
        let id = ("code_symbols", key.as_str());
        symbol.id = None;
        let _: Option<CodeSymbol> = self.db.create(id).content(symbol).await?;
        Ok(format!("code_symbols:{}", key))
    }

    async fn create_code_symbols_batch(&self, symbols: Vec<CodeSymbol>) -> Result<Vec<String>> {
        if symbols.is_empty() {
            return Ok(vec![]);
        }

        // Use typed SDK upsert per symbol — preserves Datetime type correctly.
        // Raw SQL FOR loop + serde_json loses Datetime → string, which SCHEMAFULL
        // silently rejects (SurrealDB #6816).
        let mut ids = Vec::with_capacity(symbols.len());

        for mut symbol in symbols {
            let key = symbol.unique_key();
            symbol.id = None;
            let _: Option<CodeSymbol> = self
                .db
                .upsert(("code_symbols", key.as_str()))
                .content(symbol)
                .await?;
            ids.push(format!("code_symbols:{}", key));
        }

        Ok(ids)
    }

    async fn update_symbol_embedding(&self, id: &str, embedding: Vec<f32>) -> Result<()> {
        let sql = "UPDATE code_symbols SET embedding = $embedding WHERE id = type::record($id)";
        let _ = self
            .db
            .query(sql)
            .bind(("embedding", embedding))
            .bind(("id", id.to_string()))
            .await?;
        Ok(())
    }

    async fn update_chunk_embedding(&self, id: &str, embedding: Vec<f32>) -> Result<()> {
        let sql = "UPDATE code_chunks SET embedding = $embedding WHERE id = type::record($id)";
        let _ = self
            .db
            .query(sql)
            .bind(("embedding", embedding))
            .bind(("id", id.to_string()))
            .await?;
        Ok(())
    }

    async fn batch_update_symbol_embeddings(&self, updates: &[(String, Vec<f32>)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        let sql = r#"
            FOR $u IN $updates {
                UPDATE type::record($u.id) SET embedding = $u.embedding;
            };
        "#;

        let data: Vec<_> = updates
            .iter()
            .map(|(id, emb)| serde_json::json!({"id": id, "embedding": emb}))
            .collect();

        self.db.query(sql).bind(("updates", data)).await?;
        Ok(())
    }

    async fn batch_update_chunk_embeddings(&self, updates: &[(String, Vec<f32>)]) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }

        let sql = r#"
            FOR $u IN $updates {
                UPDATE type::record($u.id) SET embedding = $u.embedding;
            };
        "#;

        let data: Vec<_> = updates
            .iter()
            .map(|(id, emb)| serde_json::json!({"id": id, "embedding": emb}))
            .collect();

        self.db.query(sql).bind(("updates", data)).await?;
        Ok(())
    }

    async fn create_symbol_relation(&self, relation: SymbolRelation) -> Result<String> {
        let sql = "RELATE $from->symbol_relation->$to SET relation_type = $rtype, project_id = $pid, file_path = $fpath, line_number = $lnum, created_at = $cat";
        let from = relation.from_symbol.clone();
        let to = relation.to_symbol.clone();

        let _response = self
            .db
            .query(sql)
            .bind(("from", from))
            .bind(("to", to))
            .bind(("rtype", relation.relation_type.to_string()))
            .bind(("pid", relation.project_id))
            .bind(("fpath", relation.file_path))
            .bind(("lnum", relation.line_number as i64))
            .bind(("cat", relation.created_at))
            .await?;
        Ok("relation_created".to_string())
    }

    async fn delete_project_symbols(&self, project_id: &str) -> Result<usize> {
        let sql = r#"
            BEGIN TRANSACTION;
            DELETE symbol_relation WHERE project_id = $project_id;
            DELETE code_symbols WHERE project_id = $project_id;
            COMMIT TRANSACTION;
        "#;
        let _ = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;
        Ok(0)
    }

    async fn delete_symbols_by_path(&self, project_id: &str, file_path: &str) -> Result<usize> {
        // symbol_relation is an edge table (from RELATE) — it has no file_path field.
        // Delete relations where either endpoint is a symbol from this file.
        let sql = r#"
            BEGIN TRANSACTION;
            DELETE symbol_relation WHERE in IN (
                SELECT id FROM code_symbols
                WHERE project_id = $project_id AND file_path = $file_path
            ) OR out IN (
                SELECT id FROM code_symbols
                WHERE project_id = $project_id AND file_path = $file_path
            );
            DELETE code_symbols WHERE project_id = $project_id AND file_path = $file_path;
            COMMIT TRANSACTION;
        "#;
        let _ = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .bind(("file_path", file_path.to_string()))
            .await?;
        Ok(0)
    }

    async fn get_project_symbols(&self, project_id: &str) -> Result<Vec<CodeSymbol>> {
        let sql = "SELECT * FROM code_symbols WHERE project_id = $project_id";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;
        let symbols: Vec<CodeSymbol> = response.take(0)?;
        Ok(symbols)
    }

    async fn get_symbol_callers(&self, symbol_id: &str) -> Result<Vec<CodeSymbol>> {
        let thing = parse_thing(symbol_id)?;
        let sql = r#"
            SELECT * FROM code_symbols 
            WHERE id IN (
                SELECT VALUE in FROM symbol_relation 
                WHERE out = $thing AND relation_type = 'calls'
            )
        "#;

        let mut response = self.db.query(sql).bind(("thing", thing)).await?;

        let symbols: Vec<CodeSymbol> = response.take(0)?;
        Ok(symbols)
    }

    async fn get_symbol_callees(&self, symbol_id: &str) -> Result<Vec<CodeSymbol>> {
        let thing = parse_thing(symbol_id)?;
        let sql = r#"
            SELECT * FROM code_symbols 
            WHERE id IN (
                SELECT VALUE out FROM symbol_relation 
                WHERE in = $thing AND relation_type = 'calls'
            )
        "#;
        let mut response = self.db.query(sql).bind(("thing", thing)).await?;
        let result: Vec<CodeSymbol> = response.take(0)?;
        Ok(result)
    }

    async fn get_related_symbols(
        &self,
        symbol_id: &str,
        depth: usize,
        direction: Direction,
    ) -> Result<(Vec<CodeSymbol>, Vec<SymbolRelation>)> {
        use crate::types::ThingId;

        let _depth = depth.clamp(1, 3);

        let symbol_thing = if !symbol_id.contains(':') {
            ThingId::new("code_symbols", symbol_id)?.to_thing()
        } else {
            let parts: Vec<&str> = symbol_id.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(crate::types::AppError::Database(format!(
                    "Invalid symbol ID format: {}",
                    symbol_id
                )));
            }
            ThingId::new(parts[0], parts[1])?.to_thing()
        };

        let sql = match direction {
            Direction::Outgoing => "SELECT * FROM symbol_relation WHERE `in` = $id",
            Direction::Incoming => "SELECT * FROM symbol_relation WHERE `out` = $id",
            Direction::Both => "SELECT * FROM symbol_relation WHERE `in` = $id OR `out` = $id",
        };

        let mut response = self
            .db
            .query(sql)
            .bind(("id", symbol_thing.clone()))
            .await?;

        // Use Value intermediary to bypass SurrealValue RecordId bug
        let raw: surrealdb_types::Value = response.take(0)?;
        let relations = value_to_symbol_relations(raw);

        let mut symbol_ids: Vec<String> = vec![];
        for rel in &relations {
            match direction {
                Direction::Outgoing => {
                    symbol_ids.push(format!(
                        "{}:{}",
                        rel.to_symbol.table.as_str(),
                        crate::types::record_key_to_string(&rel.to_symbol.key)
                    ));
                }
                Direction::Incoming => {
                    symbol_ids.push(format!(
                        "{}:{}",
                        rel.from_symbol.table.as_str(),
                        crate::types::record_key_to_string(&rel.from_symbol.key)
                    ));
                }
                Direction::Both => {
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
                    let symbol_thing_str = format!(
                        "{}:{}",
                        symbol_thing.table.as_str(),
                        crate::types::record_key_to_string(&symbol_thing.key)
                    );

                    if from_str != symbol_thing_str {
                        symbol_ids.push(from_str);
                    }
                    if to_str != symbol_thing_str {
                        symbol_ids.push(to_str);
                    }
                }
            }
        }

        // Fetch symbols by ID
        // Note: SurrealDB thing IDs in relation are strings like "code_symbols:id"
        // We can fetch them directly.
        let mut symbols: Vec<CodeSymbol> = vec![];
        for sid in symbol_ids {
            // Need to parse ID part if it's "table:id" format
            let id_part = if let Some(idx) = sid.find(':') {
                &sid[idx + 1..]
            } else {
                &sid
            };

            // Re-using a get_symbol logic would be better, but we don't have get_symbol_by_id yet.
            // Let's do a direct select
            let s: Option<CodeSymbol> = self.db.select(("code_symbols", id_part)).await?;
            if let Some(sym) = s {
                symbols.push(sym);
            }
        }

        Ok((symbols, relations))
    }

    async fn get_code_subgraph(
        &self,
        symbol_ids: &[String],
    ) -> Result<(Vec<CodeSymbol>, Vec<SymbolRelation>)> {
        if symbol_ids.is_empty() {
            return Ok((vec![], vec![]));
        }

        // Build things from symbol IDs
        let things: Vec<crate::types::Thing> = symbol_ids
            .iter()
            .filter_map(|id| {
                let id_part = if let Some(idx) = id.find(':') {
                    &id[idx + 1..]
                } else {
                    id
                };
                crate::types::ThingId::new("code_symbols", id_part)
                    .ok()
                    .map(|t| t.to_thing())
            })
            .collect();

        if things.is_empty() {
            return Ok((vec![], vec![]));
        }

        // Fetch all relations where in OR out is in our symbol set
        let sql = "SELECT * FROM symbol_relation WHERE `in` IN $ids OR `out` IN $ids";
        let mut response = self.db.query(sql).bind(("ids", things)).await?;
        let raw: surrealdb_types::Value = response.take(0)?;
        let relations = value_to_symbol_relations(raw);

        // Collect all unique symbol IDs from relations
        let mut all_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
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
            all_ids.insert(from_str);
            all_ids.insert(to_str);
        }

        // Fetch all symbols
        let mut symbols: Vec<CodeSymbol> = Vec::new();
        for sid in &all_ids {
            let id_part = if let Some(idx) = sid.find(':') {
                &sid[idx + 1..]
            } else {
                sid.as_str()
            };
            let s: Option<CodeSymbol> = self.db.select(("code_symbols", id_part)).await?;
            if let Some(sym) = s {
                symbols.push(sym);
            }
        }

        Ok((symbols, relations))
    }

    async fn search_symbols(
        &self,
        query: &str,
        project_id: Option<&str>,
        limit: usize,
        offset: usize,
        symbol_type: Option<&str>,
        path_prefix: Option<&str>,
    ) -> Result<(Vec<CodeSymbol>, u32)> {
        let limit = limit.clamp(1, 100);

        let mut conditions = vec!["(string::lowercase(name) CONTAINS string::lowercase($query) OR string::lowercase(signature) CONTAINS string::lowercase($query))".to_string()];

        if project_id.is_some() {
            conditions.push("project_id = $project_id".to_string());
        }
        if symbol_type.is_some() {
            conditions.push("symbol_type = $symbol_type".to_string());
        }
        if path_prefix.is_some() {
            conditions.push("string::starts_with(file_path, $path_prefix)".to_string());
        }

        let where_clause = conditions.join(" AND ");
        let sql = format!(
            "SELECT * FROM code_symbols WHERE {} ORDER BY name ASC LIMIT $limit START $offset",
            where_clause
        );

        let count_sql = format!(
            "SELECT count() FROM code_symbols WHERE {} GROUP ALL",
            where_clause
        );

        let mut query_builder = self
            .db
            .query(&sql)
            .bind(("query", query.to_string()))
            .bind(("limit", limit))
            .bind(("offset", offset));
        let mut count_builder = self.db.query(&count_sql).bind(("query", query.to_string()));

        if let Some(pid) = project_id {
            query_builder = query_builder.bind(("project_id", pid.to_string()));
            count_builder = count_builder.bind(("project_id", pid.to_string()));
        }
        if let Some(st) = symbol_type {
            query_builder = query_builder.bind(("symbol_type", st.to_string()));
            count_builder = count_builder.bind(("symbol_type", st.to_string()));
        }
        if let Some(pp) = path_prefix {
            query_builder = query_builder.bind(("path_prefix", pp.to_string()));
            count_builder = count_builder.bind(("path_prefix", pp.to_string()));
        }

        let mut response = query_builder.await?;
        let symbols: Vec<CodeSymbol> = response.take(0)?;

        #[derive(serde::Deserialize, SurrealValue)]
        struct CountResult {
            count: u32,
        }

        let mut count_response = count_builder.await?;
        let total: u32 = count_response
            .take::<Option<CountResult>>(0)?
            .map(|r| r.count)
            .unwrap_or(0);

        Ok((symbols, total))
    }

    async fn count_symbols(&self, project_id: &str) -> Result<u32> {
        let sql = "SELECT count() FROM code_symbols WHERE project_id = $project_id GROUP ALL";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;

        #[derive(serde::Deserialize, SurrealValue)]
        struct CountResult {
            count: u32,
        }

        let result: Option<CountResult> = response.take(0)?;
        Ok(result.map(|r| r.count).unwrap_or(0))
    }

    async fn count_chunks(&self, project_id: &str) -> Result<u32> {
        let sql = "SELECT count() FROM code_chunks WHERE project_id = $project_id GROUP ALL";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;

        #[derive(serde::Deserialize, SurrealValue)]
        struct CountResult {
            count: u32,
        }

        let result: Option<CountResult> = response.take(0)?;
        Ok(result.map(|r| r.count).unwrap_or(0))
    }

    async fn count_embedded_symbols(&self, project_id: &str) -> Result<u32> {
        let sql = "SELECT count() FROM code_symbols WHERE project_id = $project_id AND embedding IS NOT NONE GROUP ALL";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;

        #[derive(serde::Deserialize, SurrealValue)]
        struct CountResult {
            count: u32,
        }

        let result: Option<CountResult> = response.take(0)?;
        Ok(result.map(|r| r.count).unwrap_or(0))
    }

    async fn count_embedded_chunks(&self, project_id: &str) -> Result<u32> {
        let sql = "SELECT count() FROM code_chunks WHERE project_id = $project_id AND embedding IS NOT NONE GROUP ALL";
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;

        #[derive(serde::Deserialize, SurrealValue)]
        struct CountResult {
            count: u32,
        }

        let result: Option<CountResult> = response.take(0)?;
        Ok(result.map(|r| r.count).unwrap_or(0))
    }

    async fn count_symbol_relations(&self, project_id: &str) -> Result<u32> {
        let sql = r#"
            SELECT count() FROM symbol_relation 
            WHERE project_id = $project_id 
            GROUP ALL
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .await?;

        #[derive(serde::Deserialize, SurrealValue)]
        struct CountResult {
            count: u32,
        }

        let result: Option<CountResult> = response.take(0)?;
        Ok(result.map(|r| r.count).unwrap_or(0))
    }

    async fn find_symbol_by_name(
        &self,
        project_id: &str,
        name: &str,
    ) -> Result<Option<CodeSymbol>> {
        let sql = r#"
            SELECT * FROM code_symbols 
            WHERE project_id = $project_id AND name = $name 
            LIMIT 1
        "#;
        let mut response = self
            .db
            .query(sql)
            .bind(("project_id", project_id.to_string()))
            .bind(("name", name.to_string()))
            .await?;

        let symbols: Vec<CodeSymbol> = response.take(0)?;
        Ok(symbols.into_iter().next())
    }

    async fn find_symbol_by_name_with_context(
        &self,
        project_id: &str,
        name: &str,
        prefer_file: Option<&str>,
    ) -> Result<Option<CodeSymbol>> {
        // Try same file first for better resolution
        if let Some(file) = prefer_file {
            let sql = r#"
            SELECT * FROM code_symbols 
            WHERE project_id = $project_id AND name = $name AND file_path = $file
            LIMIT 1
        "#;
            let mut response = self
                .db
                .query(sql)
                .bind(("project_id", project_id.to_string()))
                .bind(("name", name.to_string()))
                .bind(("file", file.to_string()))
                .await?;

            let symbols: Vec<CodeSymbol> = response.take(0)?;
            if let Some(sym) = symbols.into_iter().next() {
                return Ok(Some(sym));
            }
        }

        // Fallback to any file in project
        self.find_symbol_by_name(project_id, name).await
    }

    async fn health_check(&self) -> Result<bool> {
        self.db.query("INFO FOR DB").await?;
        Ok(true)
    }

    async fn reset_db(&self) -> Result<()> {
        // Run each DELETE independently — some tables may not exist yet
        // (e.g. relation tables are created on first RELATE).
        // Using a transaction would cause one failure to cancel all DELETEs.
        let tables = [
            "memories",
            "entities",
            "relations",
            "code_chunks",
            "code_symbols",
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ChunkType, Datetime, Entity, Language, Memory, MemoryType, MemoryUpdate, RecordId, Relation,
    };
    use tempfile::tempdir;

    async fn setup_test_db() -> (SurrealStorage, tempfile::TempDir) {
        let tmp = tempdir().unwrap();
        let storage = SurrealStorage::new(tmp.path(), 768).await.unwrap();
        (storage, tmp)
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
            metadata: None,
            event_time: Datetime::default(),
            ingestion_time: Datetime::default(),
            valid_from: Datetime::default(),
            valid_until: None,
            importance_score: 1.0,
            invalidation_reason: None,
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
            metadata: None,
            embedding: None,
            content_hash: None,
            embedding_state: None,
        };
        let updated = storage.update_memory(&id, update).await.unwrap();
        assert_eq!(updated.content, "Updated content");

        let list = storage.list_memories(10, 0).await.unwrap();
        assert_eq!(list.len(), 1);

        let deleted = storage.delete_memory(&id).await.unwrap();
        assert!(deleted);
        assert!(storage.get_memory(&id).await.unwrap().is_none());
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
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                invalidation_reason: None,
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
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                invalidation_reason: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        let results = storage.bm25_search("Rust", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));
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
    async fn test_symbol_call_hierarchy() {
        let (storage, _tmp) = setup_test_db().await;
        use crate::types::{CodeRelationType, CodeSymbol, SymbolRelation, SymbolType};

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
            "main.rs".to_string(),
            3,
            "test_project".to_string(),
        );
        storage.create_symbol_relation(relation).await.unwrap();

        // 3. Test get_symbol_callees (Outgoing)
        // main -> ? (should be helper)
        let callees = storage.get_symbol_callees(&caller_id).await.unwrap();
        assert_eq!(callees.len(), 1, "Should find 1 callee");
        assert_eq!(callees[0].name, "helper");

        // 4. Test get_symbol_callers (Incoming)
        // ? -> helper (should be main)
        let callers = storage.get_symbol_callers(&callee_id).await.unwrap();
        assert_eq!(callers.len(), 1, "Should find 1 caller");
        assert_eq!(callers[0].name, "main");
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
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                invalidation_reason: None,
                content_hash: None,
                embedding_state: Default::default(),
            })
            .await
            .unwrap();

        let valid = storage.get_valid(None, 10).await.unwrap();
        assert_eq!(valid.len(), 1);

        storage
            .invalidate(&id, Some("test reason"), None)
            .await
            .unwrap();

        let valid_after = storage.get_valid(None, 10).await.unwrap();
        assert_eq!(valid_after.len(), 0);
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
                metadata: None,
                event_time: Datetime::default(),
                ingestion_time: Datetime::default(),
                valid_from: Datetime::default(),
                valid_until: None,
                importance_score: 1.0,
                invalidation_reason: None,
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
                embedding: Some(vec![0.1; 768]),
                content_hash: format!("hash_{}", i),
                project_id: Some("test_project".to_string()),
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
                embedding: None,
                content_hash: format!("embed_hash_{}", i),
                project_id: Some("embed_project".to_string()),
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
}
