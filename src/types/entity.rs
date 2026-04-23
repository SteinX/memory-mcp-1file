use super::{Datetime, SurrealValue, Thing};
use serde::{Deserialize, Serialize};

fn default_weight() -> f32 {
    1.0
}

fn default_datetime() -> Datetime {
    Datetime::default()
}

fn default_freshness_generation() -> u64 {
    0
}

fn default_relation_class() -> RelationClass {
    RelationClass::Observed
}

fn default_relation_provenance() -> RelationProvenance {
    RelationProvenance::ImportedManual
}

fn default_confidence_class() -> ConfidenceClass {
    ConfidenceClass::Extracted
}

fn default_staleness_state() -> StalenessState {
    StalenessState::Current
}

fn default_entity_type() -> String {
    "unknown".to_string()
}

fn default_name() -> String {
    String::new()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RelationClass {
    Observed,
    Inferred,
}

impl std::fmt::Display for RelationClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelationClass::Observed => write!(f, "observed"),
            RelationClass::Inferred => write!(f, "inferred"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RelationProvenance {
    ParserExtracted,
    ContainmentDerived,
    DeterministicSymbolLink,
    HeuristicResolver,
    EmbeddingInferred,
    ImportedManual,
}

impl std::fmt::Display for RelationProvenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelationProvenance::ParserExtracted => write!(f, "parser_extracted"),
            RelationProvenance::ContainmentDerived => write!(f, "containment_derived"),
            RelationProvenance::DeterministicSymbolLink => write!(f, "deterministic_symbol_link"),
            RelationProvenance::HeuristicResolver => write!(f, "heuristic_resolver"),
            RelationProvenance::EmbeddingInferred => write!(f, "embedding_inferred"),
            RelationProvenance::ImportedManual => write!(f, "imported_manual"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceClass {
    Extracted,
    Inferred,
    Ambiguous,
}

impl std::fmt::Display for ConfidenceClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfidenceClass::Extracted => write!(f, "extracted"),
            ConfidenceClass::Inferred => write!(f, "inferred"),
            ConfidenceClass::Ambiguous => write!(f, "ambiguous"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StalenessState {
    Current,
    Stale,
}

impl std::fmt::Display for StalenessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StalenessState::Current => write!(f, "current"),
            StalenessState::Stale => write!(f, "stale"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, SurrealValue)]
pub struct Entity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Thing>,

    #[serde(default = "default_name")]
    pub name: String,

    #[serde(default = "default_entity_type")]
    pub entity_type: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content_hash: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,

    #[serde(default = "default_datetime")]
    pub created_at: Datetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct Relation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Thing>,

    #[serde(rename = "in")]
    pub from_entity: Thing,

    #[serde(rename = "out")]
    pub to_entity: Thing,

    pub relation_type: String,

    #[serde(default = "default_relation_class")]
    pub relation_class: RelationClass,

    #[serde(default = "default_relation_provenance")]
    pub provenance: RelationProvenance,

    #[serde(default = "default_confidence_class")]
    pub confidence_class: ConfidenceClass,

    #[serde(default = "default_freshness_generation")]
    pub freshness_generation: u64,

    #[serde(default = "default_staleness_state")]
    pub staleness_state: StalenessState,

    #[serde(default = "default_weight")]
    pub weight: f32,

    #[serde(default = "default_datetime")]
    pub valid_from: Datetime,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<Datetime>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    #[default]
    Outgoing,
    Incoming,
    Both,
}

impl Entity {
    pub fn new(name: String) -> Self {
        Self {
            id: None,
            name,
            entity_type: "unknown".to_string(),
            description: None,
            embedding: None,
            content_hash: None,
            user_id: None,
            created_at: Datetime::default(),
        }
    }

    pub fn with_type(mut self, entity_type: String) -> Self {
        self.entity_type = entity_type;
        self
    }

    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }
}

impl std::str::FromStr for Direction {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "outgoing" | "out" => Ok(Direction::Outgoing),
            "incoming" | "in" => Ok(Direction::Incoming),
            "both" => Ok(Direction::Both),
            _ => Ok(Direction::default()),
        }
    }
}
