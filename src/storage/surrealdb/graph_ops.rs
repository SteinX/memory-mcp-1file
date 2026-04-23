use std::collections::HashMap;

use surrealdb::engine::local::Db;
use surrealdb::Surreal;

use crate::types::{Direction, Entity, Relation};
use crate::Result;

use super::helpers::value_to_relations;

pub(super) async fn create_entity(db: &Surreal<Db>, mut entity: Entity) -> Result<String> {
    use super::helpers::generate_id;
    let id = generate_id();
    entity.id = Some(crate::types::RecordId::new("entities", id.as_str()));
    let _: Option<Entity> = db.create(("entities", id.as_str())).content(entity).await?;
    Ok(id)
}

pub(super) async fn get_entity(db: &Surreal<Db>, id: &str) -> Result<Option<Entity>> {
    let result: Option<Entity> = db.select(("entities", id)).await?;
    Ok(result)
}

pub(super) async fn search_entities(
    db: &Surreal<Db>,
    query: &str,
    limit: usize,
) -> Result<Vec<Entity>> {
    // TODO: SurrealDB v3.0.0 FULLTEXT @@ + search::score(0) is broken.
    let sql = r#"
        SELECT * 
        FROM entities 
        WHERE string::lowercase(name) CONTAINS string::lowercase($query)
        LIMIT $limit
    "#;
    let mut response = db
        .query(sql)
        .bind(("query", query.to_string()))
        .bind(("limit", limit))
        .await?;
    let results: Vec<Entity> = response.take(0)?;
    Ok(results)
}

pub(super) async fn create_relation(db: &Surreal<Db>, relation: Relation) -> Result<String> {
    use super::helpers::generate_id;
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
        "RELATE {}->relations->{} SET relation_type = $rel_type, relation_class = $rel_class, provenance = $prov, confidence_class = $conf, freshness_generation = $fgen, staleness_state = $sstate, weight = $weight",
        from_thing, to_thing
    );

    let _response = db
        .query(&sql)
        .bind(("rel_type", relation.relation_type))
        .bind(("rel_class", relation.relation_class.to_string()))
        .bind(("prov", relation.provenance.to_string()))
        .bind(("conf", relation.confidence_class.to_string()))
        .bind(("fgen", relation.freshness_generation as i64))
        .bind(("sstate", relation.staleness_state.to_string()))
        .bind(("weight", relation.weight))
        .await?;

    // Skip response check — v3 RELATE returns record types
    Ok(id)
}

pub(super) async fn get_related(
    db: &Surreal<Db>,
    entity_id: &str,
    depth: usize,
    direction: Direction,
) -> Result<(Vec<Entity>, Vec<Relation>)> {
    use super::SurrealStorage;
    use crate::graph::GraphTraverser;

    // GraphTraverser needs a &SurrealStorage, but we only have &Surreal<Db>.
    // We reconstruct a temporary SurrealStorage by wrapping the db reference.
    // This is possible because SurrealStorage is just a newtype around Surreal<Db>.
    // SAFETY: We shadow-construct using the db field — transmuting is not needed,
    // just wrap via the struct literal. Since Surreal<Db> is Clone, we clone it.
    let storage = SurrealStorage { db: db.clone() };
    let traverser = GraphTraverser::new(&storage);
    let result = traverser.traverse(entity_id, depth, direction).await?;
    Ok((result.entities, result.relations))
}

pub(super) async fn get_subgraph(
    db: &Surreal<Db>,
    entity_ids: &[String],
) -> Result<(Vec<Entity>, Vec<Relation>)> {
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
    let mut response = db.query(sql).bind(("ids", ids.clone())).await?;
    let raw: surrealdb_types::Value = response.take(0)?;
    let relations = value_to_relations(raw);

    let entity_sql = "SELECT * FROM entities WHERE id IN $ids";
    let mut entity_response = db.query(entity_sql).bind(("ids", ids)).await?;
    let entities: Vec<Entity> = entity_response.take(0)?;

    Ok((entities, relations))
}

pub(super) async fn get_node_degrees(
    db: &Surreal<Db>,
    entity_ids: &[String],
) -> Result<HashMap<String, usize>> {
    use crate::types::ThingId;
    use surrealdb_types::SurrealValue;

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

    let mut response = db.query(sql).bind(("ids", things)).await?;

    #[derive(serde::Deserialize, SurrealValue)]
    struct DegreeResult {
        node: String,
        degree: u64,
    }

    let results: Vec<DegreeResult> = response.take(0).unwrap_or_default();
    let mut degrees: HashMap<String, usize> = entity_ids.iter().map(|id| (id.clone(), 0)).collect();
    for r in results {
        degrees.insert(r.node, r.degree as usize);
    }
    Ok(degrees)
}

pub(super) async fn get_all_entities(db: &Surreal<Db>) -> Result<Vec<Entity>> {
    let mut response = db.query("SELECT * FROM entities").await?;
    let entities: Vec<Entity> = response.take(0)?;
    Ok(entities)
}

pub(super) async fn get_all_relations(db: &Surreal<Db>) -> Result<Vec<Relation>> {
    let mut response = db.query("SELECT * FROM relations").await?;
    let raw: surrealdb_types::Value = response.take(0)?;
    let relations = value_to_relations(raw);
    Ok(relations)
}
