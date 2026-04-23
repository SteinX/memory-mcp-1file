//! Pure temporal decay scoring helpers for forgetting-aware retrieval.

use chrono::Utc;

use crate::types::{MemoryType, SearchResult};

use super::config::{
    DEFAULT_DECAY_LAMBDA, DEFAULT_EPISODIC_HALF_LIFE_DAYS, DEFAULT_MIN_DECAY,
    DEFAULT_PROCEDURAL_HALF_LIFE_DAYS, DEFAULT_REINFORCEMENT_ALPHA, DEFAULT_REINFORCEMENT_CAP,
    DEFAULT_SEMANTIC_HALF_LIFE_DAYS, ForgettingConfig,
};

/// Compute effective age in days, slowed by access reinforcement.
///
/// Formula:
/// `effective_age = max(0, actual_age_days) / (1 + alpha * ln(1 + access_count))`
///
/// Negative ages are clamped so future-dated timestamps cannot make memories "negative old".
#[must_use]
pub fn effective_age_days(actual_age_days: f64, access_count: u32) -> f64 {
    let clamped_age_days = actual_age_days.max(0.0);
    let reinforcement_scale = 1.0
        + f64::from(DEFAULT_REINFORCEMENT_ALPHA) * (1.0 + f64::from(access_count)).ln();

    clamped_age_days / reinforcement_scale
}

/// Compute decay factor for a memory type from its effective age.
///
/// Formula:
/// `decay_factor = max(min_decay, e^(-lambda * effective_age / half_life_days))`
///
/// The floor keeps very old memories retrievable instead of collapsing to zero.
#[must_use]
pub fn decay_factor(effective_age: f64, memory_type: &MemoryType) -> f32 {
    let half_life_days = f64::from(match memory_type {
        MemoryType::Episodic => DEFAULT_EPISODIC_HALF_LIFE_DAYS,
        MemoryType::Semantic => DEFAULT_SEMANTIC_HALF_LIFE_DAYS,
        MemoryType::Procedural => DEFAULT_PROCEDURAL_HALF_LIFE_DAYS,
    });
    let exponent = -f64::from(DEFAULT_DECAY_LAMBDA) * effective_age / half_life_days;
    let decay = exponent.exp().max(f64::from(DEFAULT_MIN_DECAY));

    decay as f32
}

/// Compute reinforcement bonus from repeated access, capped to avoid runaway growth.
///
/// Formula:
/// `reinforcement_bonus = min(alpha * ln(1 + access_count), reinforcement_cap)`
///
/// The cap prevents highly accessed memories from overwhelming decay entirely.
#[must_use]
pub fn reinforcement_bonus(access_count: u32) -> f32 {
    let bonus = f64::from(DEFAULT_REINFORCEMENT_ALPHA) * (1.0 + f64::from(access_count)).ln();

    bonus.min(f64::from(DEFAULT_REINFORCEMENT_CAP)) as f32
}

/// Apply decay and reinforcement to an already importance-adjusted score.
///
/// Formula:
/// `final_score = current_score * decay_factor * (1 + reinforcement_bonus)`
#[must_use]
pub fn apply_decay_scoring(
    current_score: f32,
    decay_factor: f32,
    reinforcement_bonus: f32,
) -> f32 {
    current_score * decay_factor * (1.0 + reinforcement_bonus)
}

/// Compute all decay components for a search result.
///
/// Uses `event_time` as the primary timestamp, then `ingestion_time`, then `now()`.
/// Negative ages are clamped to zero before effective aging is computed.
///
/// This preserves deterministic behavior for records missing timestamps while
/// still favoring the most specific available time source.
#[must_use]
pub fn compute_decay(
    config: &ForgettingConfig,
    result: &SearchResult,
    current_score: f32,
) -> (f32, f32, f32) {
    let now = Utc::now();
    let anchor_time = result
        .event_time
        .or(result.ingestion_time)
        .unwrap_or(now);
    let actual_age_days = (now - anchor_time).num_milliseconds() as f64 / 86_400_000.0;
    let effective_age = effective_age_days(actual_age_days, result.access_count);
    let half_life_days = f64::from(config.half_life_days_for(&result.memory_type));
    let exponent = -f64::from(config.decay_lambda) * effective_age / half_life_days;
    let decay = exponent.exp().max(f64::from(config.min_decay)) as f32;
    let bonus = reinforcement_bonus(result.access_count);
    let final_score = apply_decay_scoring(current_score, decay, bonus);

    (decay, bonus, final_score)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;

    fn approx_eq(left: f32, right: f32, epsilon: f32) {
        assert!(
            (left - right).abs() <= epsilon,
            "left={left}, right={right}, epsilon={epsilon}"
        );
    }

    fn search_result(memory_type: MemoryType) -> SearchResult {
        SearchResult {
            id: "memory-1".to_string(),
            content: "content".to_string(),
            content_hash: None,
            memory_type,
            score: 1.0,
            importance_score: 1.0,
            event_time: None,
            ingestion_time: None,
            access_count: 0,
            last_accessed_at: None,
            user_id: None,
            agent_id: None,
            run_id: None,
            namespace: None,
            metadata: None,
            superseded_by: None,
            valid_until: None,
            invalidation_reason: None,
            consolidation_trace: None,
            replacement_lineage: None,
            attention_summary: None,
            operator_summary: None,
        }
    }

    #[test]
    fn effective_age_clamps_negative_age_to_zero() {
        approx_eq(effective_age_days(-5.0, 0) as f32, 0.0, 1e-6);
    }

    #[test]
    fn episodic_one_half_life_decays_to_half() {
        approx_eq(decay_factor(30.0, &MemoryType::Episodic), 0.5, 1e-5);
    }

    #[test]
    fn episodic_two_half_lives_decay_to_quarter() {
        approx_eq(decay_factor(60.0, &MemoryType::Episodic), 0.25, 1e-5);
    }

    #[test]
    fn semantic_one_half_life_decays_to_half() {
        approx_eq(decay_factor(180.0, &MemoryType::Semantic), 0.5, 1e-5);
    }

    #[test]
    fn procedural_one_half_life_decays_to_half() {
        approx_eq(decay_factor(365.0, &MemoryType::Procedural), 0.5, 1e-5);
    }

    #[test]
    fn high_access_count_slows_decay() {
        let without_reinforcement = decay_factor(60.0, &MemoryType::Episodic);
        let with_reinforcement = decay_factor(
            effective_age_days(60.0, 10),
            &MemoryType::Episodic,
        );

        assert!(with_reinforcement > without_reinforcement);
        assert!(with_reinforcement > 0.25);
    }

    #[test]
    fn reinforcement_bonus_is_capped() {
        approx_eq(reinforcement_bonus(99_999), 3.0, 1e-6);
    }

    #[test]
    fn decay_factor_respects_minimum_floor() {
        approx_eq(decay_factor(10_000.0, &MemoryType::Episodic), 0.05, 1e-6);
    }

    #[test]
    fn apply_decay_scoring_matches_formula() {
        approx_eq(apply_decay_scoring(1.0, 0.5, 0.0), 0.5, 1e-6);
        approx_eq(apply_decay_scoring(1.0, 0.5, 1.0), 1.0, 1e-6);
    }

    #[test]
    fn compute_decay_uses_event_time_and_clamps_future_age() {
        let mut result = search_result(MemoryType::Episodic);
        result.event_time = Some(Utc::now() + Duration::days(7));

        let (decay, bonus, final_score) = compute_decay(&ForgettingConfig::default(), &result, 1.0);

        approx_eq(decay, 1.0, 1e-5);
        approx_eq(bonus, 0.0, 1e-6);
        approx_eq(final_score, 1.0, 1e-5);
    }

    #[test]
    fn compute_decay_falls_back_to_ingestion_time() {
        let mut result = search_result(MemoryType::Semantic);
        result.ingestion_time = Some(Utc::now() - Duration::days(180));

        let (decay, bonus, final_score) = compute_decay(&ForgettingConfig::default(), &result, 2.0);

        approx_eq(decay, 0.5, 0.02);
        approx_eq(bonus, 0.0, 1e-6);
        approx_eq(final_score, 1.0, 0.04);
    }
}
