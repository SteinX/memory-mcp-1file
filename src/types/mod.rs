pub mod code;
pub mod embedding_state;
pub mod entity;
pub mod error;
pub mod export;
pub mod memory;
pub mod safe_thing;
pub mod search;
pub mod symbol;
pub mod thing_id;

// --- SurrealDB SDK type aliases ---
// Centralized re-exports: v3 uses surrealdb::types instead of surrealdb::sql.
// RecordId replaces Thing. SurrealValue derive macro is required for .take() results.
pub use surrealdb::types::Datetime;
pub use surrealdb::types::RecordId;
pub use surrealdb::types::RecordIdKey;
pub use surrealdb::types::SurrealValue;
pub use surrealdb_types::Value;
/// Backward-compatible alias: Thing → RecordId
pub type Thing = RecordId;

/// Macro to implement SurrealValue for enums stored as strings in DB.
/// The derive macro serializes enums as objects `{ Variant: {} }`, but DB expects `TYPE string`.
/// This macro implements SurrealValue using serde serialization which respects `#[serde(rename_all)]`.
macro_rules! impl_string_surreal_value {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl SurrealValue for $ty {
                fn kind_of() -> surrealdb_types::Kind {
                    surrealdb_types::kind!(string)
                }

                fn is_value(value: &Value) -> bool {
                    matches!(value, Value::String(_))
                }

                fn into_value(self) -> Value {
                    // serde_json serializes `#[serde(rename_all)]` enums as quoted strings
                    let s = serde_json::to_value(&self)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    Value::String(s)
                }

                fn from_value(value: Value) -> std::result::Result<Self, surrealdb_types::Error> {
                    match &value {
                        Value::String(s) => {
                            let json_str = serde_json::Value::String(s.clone());
                            serde_json::from_value(json_str).map_err(|e| {
                                surrealdb_types::Error::internal(format!(
                                    "Failed to deserialize {}: {}", stringify!($ty), e
                                ))
                            })
                        }
                        _ => Err(surrealdb_types::ConversionError::from_value(
                            Self::kind_of(), &value
                        ).into())
                    }
                }
            }
        )+
    };
}

// Implement SurrealValue as string for all string-serialized enums.
// These use #[serde(rename_all)] and are stored as TYPE string in DB.
impl_string_surreal_value!(
    EmbeddingState,
    MemoryType,
    ChunkType,
    Language,
    IndexState,
    StructuralState,
    SemanticState,
    ProjectionState,
    RelationClass,
    RelationProvenance,
    ConfidenceClass,
    StalenessState,
    SymbolType,
    CodeRelationType,
    Direction,
);

/// Convert RecordIdKey to String — v3 RecordIdKey has no Display trait.
/// All our IDs are string-type keys, so this extracts the inner string.
pub fn record_key_to_string(key: &RecordIdKey) -> String {
    match key {
        RecordIdKey::String(s) => s.clone(),
        other => format!("{:?}", other),
    }
}

pub use code::{
    ChunkType, CodeChunk, IndexState, IndexStatus, Language, ManifestEntry, ProjectionState,
    SemanticState, StructuralState,
};
pub use embedding_state::{EmbedResult, EmbedTarget, EmbeddingState};
pub use entity::{Direction, Entity, Relation};
pub use error::{AppError, Result};
pub use export::{
    CompatibilityPolicy, ContractReasonCode, CountSummary, ExportContractMeta,
    ExportIdentity, ExportResponseSummary, ExportedGraphEdge, ExportedGraphNode,
    ExportedProjectProjection, ExportedSymbolEdge, ExportedSymbolNode, FrontierSummary,
    GenerationBasis, LifecycleView, PartialSummary, ProjectProjectionInputs,
    ProjectProjectionRequest, ProjectionContractState, ProjectionEnvelope,
    ProjectionLifecycleView, ProjectionLocatorLifecycle, ProjectionLocatorLookup,
    ProjectionLocatorLookupState, ProjectionLocatorRecord, ProjectionMaterializationEnvelope,
    ProjectionShapingSummary, SemanticLifecycleView, StructuralLifecycleView,
    SurfaceGuidance, TraversalDefaults, TraversalSummary,
};
pub use memory::{Memory, MemoryType, MemoryUpdate};
pub use search::{
    CodeSearchResult, MemoryQuery, RecallResult, ScoredCodeChunk, ScoredMemory, SearchResult,
};
pub use symbol::{
    CodeReference, CodeRelationType, CodeSymbol, ScoredSymbol, SymbolRelation, SymbolType,
};
pub use entity::{ConfidenceClass, RelationClass, RelationProvenance, StalenessState};
pub use thing_id::ThingId;
