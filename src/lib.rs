pub mod codebase;
pub mod config;
pub mod embedding;
pub mod graph;
pub mod lifecycle;
pub mod search;
pub mod server;
pub mod storage;
pub mod types;

#[cfg(test)]
pub mod test_utils;

pub use config::{AppConfig, AppState};
pub use types::error::{AppError, Result};
