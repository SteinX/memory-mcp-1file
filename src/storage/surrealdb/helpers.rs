use crate::types::{CodeRelationType, Relation, SymbolRelation};

pub(super) fn generate_id() -> String {
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

pub(super) fn parse_thing(id: &str) -> crate::Result<crate::types::Thing> {
    if let Some((table, key)) = id.split_once(':') {
        Ok(crate::types::RecordId::new(
            table.to_string(),
            key.to_string(),
        ))
    } else {
        Err(crate::AppError::Internal(
            format!("Invalid thing ID format: {}", id).into(),
        ))
    }
}

/// SurrealDB v3 workaround: The SurrealValue derive macro generates `from_value()`
/// that fails with "Expected any, got record" when a struct contains RecordId fields.
/// Also, serde_json intermediary fails because Value serializes with Rust enum wrappers.
/// This helper manually extracts fields from Value::Object to construct Relation.
pub(super) fn value_to_relations(value: surrealdb_types::Value) -> Vec<Relation> {
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
pub(super) fn value_to_symbol_relations(value: surrealdb_types::Value) -> Vec<SymbolRelation> {
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
