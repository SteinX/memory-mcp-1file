use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[source] Box<surrealdb::Error>),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Embedding service not ready. Please try again.")]
    EmbeddingNotReady,

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Memory not found: {0}")]
    MemoryNotFound(String),

    #[error("Entity not found: {0}")]
    EntityNotFound(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Indexing error: {0}")]
    Indexing(String),

    #[error("Dimension mismatch: model={model}, db={db}")]
    DimensionMismatch { model: usize, db: usize },

    #[error("IO error: {0}")]
    Io(#[source] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

pub type Result<T> = std::result::Result<T, AppError>;

impl From<surrealdb::Error> for AppError {
    fn from(e: surrealdb::Error) -> Self {
        AppError::Database(Box::new(e))
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        // anyhow::Error -> Box<dyn Error+Send+Sync> via into_boxed_error (1.82+) or manual
        AppError::Internal(e.into())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e)
    }
}

impl From<notify::Error> for AppError {
    fn from(e: notify::Error) -> Self {
        AppError::Internal(Box::new(e))
    }
}
