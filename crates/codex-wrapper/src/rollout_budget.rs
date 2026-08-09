//! Native per-execution rollout-budget configuration.
//!
//! This is deliberately separate from [`crate::TokenBudget`]. That type sees
//! usage only after a complete CLI turn and can refuse the next turn; this
//! one asks Codex itself to stop an in-progress `exec` at a response boundary.
//!
//! The native meter is not portable total-token usage. Codex 0.145 and 0.146
//! always compute `output_tokens * sampling_token_weight` plus non-cached
//! input tokens times `prefill_token_weight`. Starting with 0.147, Codex
//! prefers a provider-reported `codex_rollout_budget_units` value when one is
//! available and otherwise uses that weighted fallback. Cached input is not
//! included in the fallback. Provider-reported units may have different,
//! opaque semantics, so hosts must not treat a CLI upgrade as an unchanged
//! portable meter. One response can cross the limit before Codex observes it.

use crate::error::{Error, Result};

/// A validated Codex-native rollout budget for one CLI execution.
///
/// Use [`RolloutBudgetConfig::builder`] and pass the result to
/// [`crate::ExecCommand::rollout_budget`] or
/// [`crate::ExecResumeCommand::rollout_budget`]. The same config shape is
/// accepted by both opening and resumed `exec` commands.
#[derive(Debug, Clone, PartialEq)]
pub struct RolloutBudgetConfig {
    limit_tokens: u64,
    reminder_at_remaining_tokens: Vec<u64>,
    sampling_token_weight: f64,
    prefill_token_weight: f64,
}

impl RolloutBudgetConfig {
    /// Start a builder with the native rollout-budget-unit limit.
    #[must_use]
    pub fn builder(limit_tokens: u64) -> RolloutBudgetConfigBuilder {
        RolloutBudgetConfigBuilder {
            limit_tokens,
            reminder_at_remaining_tokens: Vec::new(),
            sampling_token_weight: 1.0,
            prefill_token_weight: 1.0,
        }
    }

    /// The configured native rollout-budget-unit limit.
    #[must_use]
    pub fn limit_tokens(&self) -> u64 {
        self.limit_tokens
    }

    /// Remaining-token thresholds that make Codex restate the budget.
    #[must_use]
    pub fn reminder_at_remaining_tokens(&self) -> &[u64] {
        &self.reminder_at_remaining_tokens
    }

    /// Weight applied to generated output tokens when provider units are absent.
    #[must_use]
    pub fn sampling_token_weight(&self) -> f64 {
        self.sampling_token_weight
    }

    /// Weight applied to non-cached input tokens when provider units are absent.
    #[must_use]
    pub fn prefill_token_weight(&self) -> f64 {
        self.prefill_token_weight
    }

    pub(crate) fn config_override(&self) -> String {
        let reminders = self
            .reminder_at_remaining_tokens
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "features.rollout_budget={{enabled=true,limit_tokens={},reminder_at_remaining_tokens=[{}],sampling_token_weight={},prefill_token_weight={}}}",
            self.limit_tokens, reminders, self.sampling_token_weight, self.prefill_token_weight,
        )
    }

    pub(crate) fn is_config_override(value: &str) -> bool {
        value.starts_with("features.rollout_budget={enabled=true,limit_tokens=")
    }
}

/// Builder for [`RolloutBudgetConfig`].
#[derive(Debug, Clone)]
pub struct RolloutBudgetConfigBuilder {
    limit_tokens: u64,
    reminder_at_remaining_tokens: Vec<u64>,
    sampling_token_weight: f64,
    prefill_token_weight: f64,
}

impl RolloutBudgetConfigBuilder {
    /// Replace the remaining-unit thresholds that make Codex restate the budget.
    ///
    /// An empty list is valid and disables threshold reminders. Codex still
    /// includes the initial remaining-budget message and enforces the limit.
    #[must_use]
    pub fn reminder_at_remaining_tokens(
        mut self,
        thresholds: impl IntoIterator<Item = u64>,
    ) -> Self {
        self.reminder_at_remaining_tokens = thresholds.into_iter().collect();
        self
    }

    /// Set the weight for generated output tokens when provider units are absent.
    #[must_use]
    pub fn sampling_token_weight(mut self, weight: f64) -> Self {
        self.sampling_token_weight = weight;
        self
    }

    /// Set the weight for non-cached input tokens when provider units are absent.
    #[must_use]
    pub fn prefill_token_weight(mut self, weight: f64) -> Self {
        self.prefill_token_weight = weight;
        self
    }

    /// Validate and build the native rollout-budget config.
    pub fn build(self) -> Result<RolloutBudgetConfig> {
        if self.limit_tokens == 0 || self.limit_tokens > i64::MAX as u64 {
            return Err(invalid("limit_tokens must be in 1..=i64::MAX"));
        }
        if self
            .reminder_at_remaining_tokens
            .iter()
            .any(|&threshold| threshold == 0 || threshold >= self.limit_tokens)
        {
            return Err(invalid(
                "reminder thresholds must be positive and below limit_tokens",
            ));
        }
        for (field, weight) in [
            ("sampling_token_weight", self.sampling_token_weight),
            ("prefill_token_weight", self.prefill_token_weight),
        ] {
            if !weight.is_finite() || weight < 0.0 {
                return Err(invalid(format!("{field} must be finite and non-negative")));
            }
        }
        Ok(RolloutBudgetConfig {
            limit_tokens: self.limit_tokens,
            reminder_at_remaining_tokens: self.reminder_at_remaining_tokens,
            sampling_token_weight: self.sampling_token_weight,
            prefill_token_weight: self.prefill_token_weight,
        })
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidRolloutBudget {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_config_serializes_as_one_strict_cli_override() {
        let budget = RolloutBudgetConfig::builder(100_000)
            .reminder_at_remaining_tokens([50_000, 10_000])
            .sampling_token_weight(1.5)
            .prefill_token_weight(0.25)
            .build()
            .expect("valid budget");

        assert_eq!(budget.limit_tokens(), 100_000);
        assert_eq!(budget.reminder_at_remaining_tokens(), [50_000, 10_000]);
        assert_eq!(
            budget.config_override(),
            "features.rollout_budget={enabled=true,limit_tokens=100000,reminder_at_remaining_tokens=[50000,10000],sampling_token_weight=1.5,prefill_token_weight=0.25}"
        );
    }

    #[test]
    fn invalid_limits_thresholds_and_weights_fail_before_launch() {
        for result in [
            RolloutBudgetConfig::builder(0).build(),
            RolloutBudgetConfig::builder(i64::MAX as u64 + 1).build(),
            RolloutBudgetConfig::builder(100)
                .reminder_at_remaining_tokens([0])
                .build(),
            RolloutBudgetConfig::builder(100)
                .reminder_at_remaining_tokens([100])
                .build(),
            RolloutBudgetConfig::builder(100)
                .sampling_token_weight(f64::NAN)
                .build(),
            RolloutBudgetConfig::builder(100)
                .prefill_token_weight(-1.0)
                .build(),
        ] {
            assert!(matches!(result, Err(Error::InvalidRolloutBudget { .. })));
        }
    }
}
