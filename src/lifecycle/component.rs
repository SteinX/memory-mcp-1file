use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ShutdownPriority {
    First = 0,
    #[default]
    Normal = 50,
    Last = 100,
}

#[derive(Debug)]
pub enum HealthStatus {
    Healthy,
    Degraded { reason: String },
    Unhealthy { reason: String },
}

#[derive(Debug)]
pub struct ComponentHealth {
    pub status: HealthStatus,
}

impl Default for ComponentHealth {
    fn default() -> Self {
        Self {
            status: HealthStatus::Healthy,
        }
    }
}

#[derive(Debug)]
pub enum ShutdownResult {
    Complete { items_processed: usize },
    Partial { remaining: usize },
    Error(String),
}

/// A dyn-compatible component lifecycle trait.
///
/// Async methods are exposed via `Box<dyn Future>` to remain object-safe
/// without requiring the `async-trait` crate (Rust 1.75+).
pub trait Component: Send + Sync {
    fn name(&self) -> &'static str;

    fn shutdown_priority(&self) -> ShutdownPriority {
        ShutdownPriority::Normal
    }

    fn health(&self) -> Pin<Box<dyn Future<Output = ComponentHealth> + Send + '_>> {
        Box::pin(async { ComponentHealth::default() })
    }

    fn shutdown(
        &self,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = ShutdownResult> + Send + '_>>;

    fn force_stop(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}
