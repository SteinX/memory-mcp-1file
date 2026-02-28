use surrealdb::engine::local::Db;
use surrealdb::Surreal;

use crate::types::{record_key_to_string, CodeSymbol, Direction, ScoredCodeChunk, SymbolRelation};
use crate::Result;

use super::helpers::{parse_thing, value_to_symbol_relations};

pub(super) async fn create_code_symbol(db: &Surreal<Db>, mut symbol: CodeSymbol) -> Result<String> {
    let key = symbol.unique_key();
    let id = ("code_symbols", key.as_str());
    symbol.id = None;
    let _: Option<CodeSymbol> = db.create(id).content(symbol).await?;
    Ok(format!("code_symbols:{}", key))
}

pub(super) async fn create_code_symbols_batch(
    db: &Surreal<Db>,
    symbols: Vec<CodeSymbol>,
) -> Result<Vec<String>> {
    if symbols.is_empty() {
        return Ok(vec![]);
    }

    // Single INSERT ... ON DUPLICATE KEY UPDATE instead of N individual upserts.
    //
    // We intentionally omit `indexed_at` and `embedding` from the JSON payload:
    //   - `indexed_at`: server DEFAULT time::now() applies on INSERT; explicit
    //     time::now() in ON DUPLICATE KEY UPDATE handles the upsert case.
    //     This side-steps SurrealDB #6816 where serde_json serializes Datetime
    //     as a string which SCHEMAFULL silently rejects.
    //   - `embedding`: always None for newly created symbols (set later by
    //     the embedding worker).
    let mut data = Vec::with_capacity(symbols.len());
    let mut ids = Vec::with_capacity(symbols.len());

    for symbol in &symbols {
        let key = symbol.unique_key();
        ids.push(format!("code_symbols:{}", key));

        data.push(serde_json::json!({
            "id": key,
            "name": symbol.name,
            "symbol_type": symbol.symbol_type,
            "file_path": symbol.file_path,
            "start_line": symbol.start_line,
            "end_line": symbol.end_line,
            "project_id": symbol.project_id,
            "signature": symbol.signature,
        }));
    }

    let sql = r#"
        INSERT INTO code_symbols $symbols
        ON DUPLICATE KEY UPDATE
            name = $input.name,
            symbol_type = $input.symbol_type,
            file_path = $input.file_path,
            start_line = $input.start_line,
            end_line = $input.end_line,
            project_id = $input.project_id,
            signature = $input.signature,
            indexed_at = time::now()
    "#;

    db.query(sql).bind(("symbols", data)).await?;
    Ok(ids)
}

pub(super) async fn update_symbol_embedding(
    db: &Surreal<Db>,
    id: &str,
    embedding: Vec<f32>,
) -> Result<()> {
    let sql = "UPDATE code_symbols SET embedding = $embedding WHERE id = (type::record($id))";
    let _ = db
        .query(sql)
        .bind(("embedding", embedding))
        .bind(("id", id.to_string()))
        .await?;
    Ok(())
}

pub(super) async fn batch_update_symbol_embeddings(
    db: &Surreal<Db>,
    updates: &[(String, Vec<f32>)],
) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }

    let sql = r#"
        FOR $u IN $updates {
            UPDATE (type::record($u.id)) SET embedding = $u.embedding;
        };
    "#;

    let data: Vec<_> = updates
        .iter()
        .map(|(id, emb)| serde_json::json!({"id": id, "embedding": emb}))
        .collect();

    db.query(sql).bind(("updates", data)).await?;
    Ok(())
}

pub(super) async fn create_symbol_relation(
    db: &Surreal<Db>,
    relation: SymbolRelation,
) -> Result<String> {
    let sql = "RELATE $from->symbol_relation->$to SET relation_type = $rtype, project_id = $pid, file_path = $fpath, line_number = $lnum, created_at = $cat";
    let from = relation.from_symbol.clone();
    let to = relation.to_symbol.clone();

    let _response = db
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

/// Batch-create symbol relations with a single FOR loop instead of N RELATE queries.
///
/// `created_at` is omitted from the data payload — the SCHEMAFULL DEFAULT
/// `time::now()` applies automatically, which side-steps SurrealDB #6816
/// (Datetime loses type in FOR loops with serde_json).
pub(super) async fn create_symbol_relations_batch(
    db: &Surreal<Db>,
    relations: Vec<SymbolRelation>,
) -> Result<u32> {
    if relations.is_empty() {
        return Ok(0);
    }

    let thing_to_string = |t: &surrealdb::types::RecordId| -> String {
        format!("{}:{}", t.table.as_str(), record_key_to_string(&t.key))
    };

    let data: Vec<_> = relations
        .iter()
        .map(|r| {
            serde_json::json!({
                "from": thing_to_string(&r.from_symbol),
                "to": thing_to_string(&r.to_symbol),
                "relation_type": r.relation_type.to_string(),
                "project_id": r.project_id,
                "file_path": r.file_path,
                "line_number": r.line_number as i64,
            })
        })
        .collect();

    let count = data.len() as u32;

    let sql = r#"
        FOR $r IN $relations {
            RELATE (type::record($r.from))->symbol_relation->(type::record($r.to))
            SET relation_type = $r.relation_type,
                project_id = $r.project_id,
                file_path = $r.file_path,
                line_number = $r.line_number;
        };
    "#;

    db.query(sql).bind(("relations", data)).await?;
    Ok(count)
}

pub(super) async fn delete_project_symbols(db: &Surreal<Db>, project_id: &str) -> Result<usize> {
    let sql = r#"
        BEGIN TRANSACTION;
        DELETE symbol_relation WHERE project_id = $project_id;
        DELETE code_symbols WHERE project_id = $project_id;
        COMMIT TRANSACTION;
    "#;
    let _ = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    Ok(0)
}

pub(super) async fn delete_symbols_by_path(
    db: &Surreal<Db>,
    project_id: &str,
    file_path: &str,
) -> Result<usize> {
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
    let _ = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("file_path", file_path.to_string()))
        .await?;
    Ok(0)
}

pub(super) async fn get_project_symbols(
    db: &Surreal<Db>,
    project_id: &str,
) -> Result<Vec<CodeSymbol>> {
    let sql = "SELECT * FROM code_symbols WHERE project_id = $project_id";
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .await?;
    let symbols: Vec<CodeSymbol> = response.take(0)?;
    Ok(symbols)
}

pub(super) async fn get_symbol_callers(
    db: &Surreal<Db>,
    symbol_id: &str,
) -> Result<Vec<CodeSymbol>> {
    let thing = parse_thing(symbol_id)?;
    let sql = r#"
        SELECT * FROM code_symbols 
        WHERE id IN (
            SELECT VALUE in FROM symbol_relation 
            WHERE out = $thing AND relation_type = 'calls'
        )
    "#;

    let mut response = db.query(sql).bind(("thing", thing)).await?;
    let symbols: Vec<CodeSymbol> = response.take(0)?;
    Ok(symbols)
}

pub(super) async fn get_symbol_callees(
    db: &Surreal<Db>,
    symbol_id: &str,
) -> Result<Vec<CodeSymbol>> {
    let thing = parse_thing(symbol_id)?;
    let sql = r#"
        SELECT * FROM code_symbols 
        WHERE id IN (
            SELECT VALUE out FROM symbol_relation 
            WHERE in = $thing AND relation_type = 'calls'
        )
    "#;
    let mut response = db.query(sql).bind(("thing", thing)).await?;
    let result: Vec<CodeSymbol> = response.take(0)?;
    Ok(result)
}

pub(super) async fn get_related_symbols(
    db: &Surreal<Db>,
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
            return Err(crate::types::AppError::Internal(
                format!("Invalid symbol ID format: {}", symbol_id).into(),
            ));
        }
        ThingId::new(parts[0], parts[1])?.to_thing()
    };

    let sql = match direction {
        Direction::Outgoing => "SELECT * FROM symbol_relation WHERE `in` = $id",
        Direction::Incoming => "SELECT * FROM symbol_relation WHERE `out` = $id",
        Direction::Both => "SELECT * FROM symbol_relation WHERE `in` = $id OR `out` = $id",
    };

    let mut response = db.query(sql).bind(("id", symbol_thing.clone())).await?;

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

    // Fetch all symbols in a single query (batch) — eliminates N+1
    let symbols: Vec<CodeSymbol> = if symbol_ids.is_empty() {
        vec![]
    } else {
        // Deduplicate and ensure table prefix
        let record_list: Vec<String> = symbol_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .map(|sid| {
                if sid.contains(':') {
                    sid.clone()
                } else {
                    format!("code_symbols:{}", sid)
                }
            })
            .collect();
        let sql = format!("SELECT * FROM {}", record_list.join(", "));
        let mut response = db.query(sql).await?;
        response.take(0).unwrap_or_default()
    };

    Ok((symbols, relations))
}

pub(super) async fn get_code_subgraph(
    db: &Surreal<Db>,
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
    let mut response = db.query(sql).bind(("ids", things)).await?;
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

    // Fetch all symbols in a single query (batch) — eliminates N+1
    let symbols: Vec<CodeSymbol> = if all_ids.is_empty() {
        vec![]
    } else {
        let record_list: Vec<String> = all_ids
            .iter()
            .map(|sid| {
                if sid.contains(':') {
                    sid.clone()
                } else {
                    format!("code_symbols:{}", sid)
                }
            })
            .collect();
        let sql = format!("SELECT * FROM {}", record_list.join(", "));
        let mut response = db.query(sql).await?;
        response.take(0).unwrap_or_default()
    };

    Ok((symbols, relations))
}

pub(super) async fn search_symbols(
    db: &Surreal<Db>,
    query: &str,
    project_id: Option<&str>,
    limit: usize,
    offset: usize,
    symbol_type: Option<&str>,
    path_prefix: Option<&str>,
) -> Result<(Vec<CodeSymbol>, u32)> {
    use surrealdb_types::SurrealValue;

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
        // NOTE: ORDER BY name ASC removed — SurrealDB v3.0.0 bug (#5611):
        // ORDER BY + LIMIT + START with composite index idx_symbols_type_name
        // causes empty results when symbol_type filter is active. The optimizer
        // incorrectly uses the index ordering with START offset. Without ORDER BY,
        // the planner uses a simple indexed scan which works correctly.
        "SELECT * FROM code_symbols WHERE {} LIMIT $limit START $offset",
        where_clause
    );

    let count_sql = format!(
        "SELECT count() FROM code_symbols WHERE {} GROUP ALL",
        where_clause
    );

    let mut query_builder = db
        .query(&sql)
        .bind(("query", query.to_string()))
        .bind(("limit", limit))
        .bind(("offset", offset));
    let mut count_builder = db.query(&count_sql).bind(("query", query.to_string()));

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

pub(super) async fn count_symbols(db: &Surreal<Db>, project_id: &str) -> Result<u32> {
    use surrealdb_types::SurrealValue;

    let sql = "SELECT count() FROM code_symbols WHERE project_id = $project_id GROUP ALL";
    let mut response = db
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

pub(super) async fn count_embedded_symbols(db: &Surreal<Db>, project_id: &str) -> Result<u32> {
    use surrealdb_types::SurrealValue;

    let sql = "SELECT count() FROM code_symbols WHERE project_id = $project_id AND embedding IS NOT NONE GROUP ALL";
    let mut response = db
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

pub(super) async fn count_symbol_relations(db: &Surreal<Db>, project_id: &str) -> Result<u32> {
    use surrealdb_types::SurrealValue;

    let sql = r#"
        SELECT count() FROM symbol_relation 
        WHERE project_id = $project_id 
        GROUP ALL
    "#;
    let mut response = db
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

pub(super) async fn find_symbol_by_name(
    db: &Surreal<Db>,
    project_id: &str,
    name: &str,
) -> Result<Option<CodeSymbol>> {
    let sql = r#"
        SELECT * FROM code_symbols 
        WHERE project_id = $project_id AND name = $name 
        LIMIT 1
    "#;
    let mut response = db
        .query(sql)
        .bind(("project_id", project_id.to_string()))
        .bind(("name", name.to_string()))
        .await?;

    let symbols: Vec<CodeSymbol> = response.take(0)?;
    Ok(symbols.into_iter().next())
}

pub(super) async fn find_symbol_by_name_with_context(
    db: &Surreal<Db>,
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
        let mut response = db
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
    find_symbol_by_name(db, project_id, name).await
}

pub(super) async fn vector_search_code(
    db: &Surreal<Db>,
    embedding: &[f32],
    project_id: Option<&str>,
    limit: usize,
) -> Result<Vec<ScoredCodeChunk>> {
    // Use HNSW index via <|K,EF|> KNN operator for fast candidate selection,
    // then compute exact cosine similarity for scoring.
    // Over-fetch when project_id filter is active since KNN runs before filtering.
    let knn_k = if project_id.is_some() {
        (limit * 4).min(200)
    } else {
        limit.min(200)
    };
    let ef = knn_k.max(150);

    let query = format!(
        r#"
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
        WHERE embedding <|{knn_k},{ef}|> $vec
          AND ($project_id IS NONE OR project_id = $project_id)
        ORDER BY score DESC 
        LIMIT $limit
    "#
    );
    let mut response = db
        .query(&query)
        .bind(("vec", embedding.to_vec()))
        .bind(("project_id", project_id.map(String::from)))
        .bind(("limit", limit))
        .await?;
    let results: Vec<ScoredCodeChunk> = response.take(0)?;
    Ok(results)
}

pub(super) async fn vector_search_symbols(
    db: &Surreal<Db>,
    embedding: &[f32],
    project_id: Option<&str>,
    limit: usize,
) -> Result<Vec<CodeSymbol>> {
    // Use HNSW index via <|K,EF|> KNN operator for fast candidate selection,
    // then compute exact cosine similarity for scoring.
    // Over-fetch when project_id filter is active since KNN runs before filtering.
    let knn_k = if project_id.is_some() {
        (limit * 4).min(200)
    } else {
        limit.min(200)
    };
    let ef = knn_k.max(150);

    let sql = format!(
        r#"
        SELECT *,
            vector::similarity::cosine(embedding, $vec) AS _score
        FROM code_symbols
        WHERE embedding <|{knn_k},{ef}|> $vec
          AND ($project_id IS NONE OR project_id = $project_id)
        ORDER BY _score DESC
        LIMIT $limit
    "#
    );
    let mut response = db
        .query(&sql)
        .bind(("vec", embedding.to_vec()))
        .bind(("project_id", project_id.map(String::from)))
        .bind(("limit", limit))
        .await?;
    let results: Vec<CodeSymbol> = response.take(0)?;
    Ok(results)
}
