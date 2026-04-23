//! Forgetting and decay subsystem for ranking, access reinforcement, and capacity control.
//!
//! This module coordinates the server's memory decay behavior in three layers:
//! - `config`: feature flags, tuning constants, and runtime configuration loading.
//! - `decay`: pure score math used to age memories and apply access reinforcement.
//! - `access`: fire-and-forget access tracking so repeated reads can slow effective aging.
//! - `capacity`: background cleanup that archives low-value memories when storage grows too large.
//!
//! The decay pipeline uses:
//! `final_score = current_score × decay_factor × (1 + min(reinforcement_bonus, 3.0))`
//! where `decay_factor = max(0.05, e^(-LN2 × effective_age / half_life))` and
//! `effective_age_days = max(0, actual_age) / (1 + 0.5 × ln(1 + access_count))`.
//!
//! Half-lives are memory-type specific: episodic memories decay over 30 days,
//! semantic memories over 180 days, and procedural memories over 365 days.
//!
//! Feature flags:
//! - `MEMORY_DECAY_ENABLED` controls whether decay-aware ranking is active.
//! - `MEMORY_CAPACITY_CONTROLLER_ENABLED` controls the background capacity controller.

pub mod access;
pub mod capacity;
pub mod config;
pub mod decay;

pub use access::*;
pub use capacity::*;
pub use config::*;
pub use decay::*;

#[cfg(test)]
mod tests;
