use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingState {
    #[default]
    None,
    Pending,
    Ready,
    Stale,
    Current,
}

impl std::fmt::Display for EmbeddingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Pending => write!(f, "pending"),
            Self::Ready => write!(f, "ready"),
            Self::Stale => write!(f, "stale"),
            Self::Current => write!(f, "current"),
        }
    }
}

impl std::str::FromStr for EmbeddingState {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(Self::None),
            "pending" => Ok(Self::Pending),
            "ready" => Ok(Self::Ready),
            "stale" => Ok(Self::Stale),
            "current" => Ok(Self::Current),
            _ => Ok(Self::default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedTarget {
    Memory,
    Entity,
    CodeChunk,
    Symbol,
}

impl EmbedTarget {
    pub fn priority(&self) -> u8 {
        match self {
            Self::Memory => 10,
            Self::Entity => 10,
            Self::CodeChunk => 5,
            Self::Symbol => 5,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EmbedResult {
    Ready {
        embedding: Vec<f32>,
        content_hash: String,
    },
    Pending {
        content_hash: String,
    },
    Unchanged,
    Skipped,
}
