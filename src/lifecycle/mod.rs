mod component;
mod registry;
mod runtime_diagnostics;
mod shutdown;

pub use component::{Component, ComponentHealth, HealthStatus, ShutdownPriority, ShutdownResult};
pub use registry::ComponentRegistry;
pub use runtime_diagnostics::{
    install_panic_hook, record_runtime_event, record_runtime_event_with_details, spawn_heartbeat,
};
