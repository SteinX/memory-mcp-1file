use std::time::Duration;

use crate::types::MemoryType;

/// # Tuning
/// Controls the default half-life, in days, for episodic memories.
pub const DEFAULT_EPISODIC_HALF_LIFE_DAYS: f32 = 30.0;

/// # Tuning
/// Controls the default half-life, in days, for semantic memories.
pub const DEFAULT_SEMANTIC_HALF_LIFE_DAYS: f32 = 180.0;

/// # Tuning
/// Controls the default half-life, in days, for procedural memories.
pub const DEFAULT_PROCEDURAL_HALF_LIFE_DAYS: f32 = 365.0;

/// # Tuning
/// Controls the decay curve constant so a memory reaches exactly 0.5 at one half-life.
pub const DEFAULT_DECAY_LAMBDA: f32 = std::f32::consts::LN_2;

/// # Tuning
/// Controls the lowest decay factor allowed before scores stop shrinking further.
pub const DEFAULT_MIN_DECAY: f32 = 0.05;

/// # Tuning
/// Controls how strongly repeated access slows effective aging.
pub const DEFAULT_REINFORCEMENT_ALPHA: f32 = 0.5;

/// # Tuning
/// Controls the maximum reinforcement multiplier applied from repeated access.
pub const DEFAULT_REINFORCEMENT_CAP: f32 = 3.0;

/// # Tuning
/// Controls the soft memory count threshold that triggers capacity cleanup work.
pub const DEFAULT_SOFT_LIMIT: usize = 10_000;

/// # Tuning
/// Controls the fraction of the soft limit to retain after capacity cleanup completes.
pub const DEFAULT_CLEANUP_TARGET_RATIO: f32 = 0.8;

/// # Tuning
/// Controls how often the capacity controller checks whether cleanup should run.
pub const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(3_600);

/// Runtime forgetting configuration.
#[derive(Debug, Clone)]
pub struct ForgettingConfig {
    /// Half-life, in days, applied to episodic memories.
    pub episodic_half_life_days: f32,
    /// Half-life, in days, applied to semantic memories.
    pub semantic_half_life_days: f32,
    /// Half-life, in days, applied to procedural memories.
    pub procedural_half_life_days: f32,
    /// Decay constant used in the exponential forgetting curve.
    pub decay_lambda: f32,
    /// Minimum allowed decay factor after exponential decay is applied.
    pub min_decay: f32,
    /// Reinforcement strength used to reduce effective age for frequently accessed memories.
    pub reinforcement_alpha: f32,
    /// Maximum reinforcement multiplier allowed from repeated access.
    pub reinforcement_cap: f32,
    /// Soft memory-count limit that begins capacity management.
    pub soft_limit: usize,
    /// Target ratio of retained memories after cleanup relative to the soft limit.
    pub cleanup_target_ratio: f32,
    /// Interval between capacity-controller checks.
    pub check_interval: Duration,
}

impl ForgettingConfig {
    pub fn from_env() -> Self {
        fn parse_env<T>(key: &str) -> Option<T>
        where
            T: std::str::FromStr,
        {
            std::env::var(key).ok()?.parse().ok()
        }

        let defaults = Self::default();

        Self {
            episodic_half_life_days: parse_env("MEMORY_EPISODIC_HALF_LIFE_DAYS")
                .unwrap_or(defaults.episodic_half_life_days),
            semantic_half_life_days: parse_env("MEMORY_SEMANTIC_HALF_LIFE_DAYS")
                .unwrap_or(defaults.semantic_half_life_days),
            procedural_half_life_days: parse_env("MEMORY_PROCEDURAL_HALF_LIFE_DAYS")
                .unwrap_or(defaults.procedural_half_life_days),
            decay_lambda: parse_env("MEMORY_DECAY_LAMBDA").unwrap_or(defaults.decay_lambda),
            min_decay: parse_env("MEMORY_MIN_DECAY").unwrap_or(defaults.min_decay),
            reinforcement_alpha: parse_env("MEMORY_REINFORCEMENT_ALPHA")
                .unwrap_or(defaults.reinforcement_alpha),
            reinforcement_cap: parse_env("MEMORY_REINFORCEMENT_CAP")
                .unwrap_or(defaults.reinforcement_cap),
            soft_limit: parse_env("MEMORY_SOFT_LIMIT").unwrap_or(defaults.soft_limit),
            cleanup_target_ratio: parse_env("MEMORY_CLEANUP_TARGET_RATIO")
                .unwrap_or(defaults.cleanup_target_ratio),
            check_interval: Duration::from_secs(
                parse_env::<u64>("MEMORY_CAPACITY_CHECK_INTERVAL_SECS")
                    .unwrap_or(defaults.check_interval.as_secs()),
            ),
        }
    }

    /// Returns the configured half-life, in days, for the provided memory type.
    pub fn half_life_days_for(&self, memory_type: &MemoryType) -> f32 {
        match memory_type {
            MemoryType::Episodic => self.episodic_half_life_days,
            MemoryType::Semantic => self.semantic_half_life_days,
            MemoryType::Procedural => self.procedural_half_life_days,
        }
    }
}

impl Default for ForgettingConfig {
    fn default() -> Self {
        Self {
            episodic_half_life_days: DEFAULT_EPISODIC_HALF_LIFE_DAYS,
            semantic_half_life_days: DEFAULT_SEMANTIC_HALF_LIFE_DAYS,
            procedural_half_life_days: DEFAULT_PROCEDURAL_HALF_LIFE_DAYS,
            decay_lambda: DEFAULT_DECAY_LAMBDA,
            min_decay: DEFAULT_MIN_DECAY,
            reinforcement_alpha: DEFAULT_REINFORCEMENT_ALPHA,
            reinforcement_cap: DEFAULT_REINFORCEMENT_CAP,
            soft_limit: DEFAULT_SOFT_LIMIT,
            cleanup_target_ratio: DEFAULT_CLEANUP_TARGET_RATIO,
            check_interval: DEFAULT_CHECK_INTERVAL,
        }
    }
}

pub fn decay_enabled() -> bool {
    std::env::var("MEMORY_DECAY_ENABLED")
        .map(|v| v != "false")
        .unwrap_or(true)
}

pub fn capacity_controller_enabled() -> bool {
    std::env::var("MEMORY_CAPACITY_CONTROLLER_ENABLED")
        .map(|v| v != "false")
        .unwrap_or(true)
}
