//! Cumulative token budget tracking across turns.
//!
//! # Why tokens and not money
//!
//! `claude-wrapper`'s equivalent tracks USD, because that CLI reports a cost
//! per turn. The codex CLI does not: a completed turn carries token counts and
//! no monetary field (see the schema block in [`crate::types`]). Converting
//! tokens to dollars needs a per-model price table the CLI does not provide,
//! and a hardcoded one would go stale silently, which is the failure mode #73
//! was filed about. So the ceiling here is denominated in what the CLI
//! actually reports.
//!
//! # What a budget cannot see
//!
//! A tracker only counts what it is told, and two codex behaviours mean that
//! is less than everything:
//!
//! - A `turn.completed` event without a `usage` object contributes nothing.
//!   [`TokenBudget::turns_missing_usage`] counts those separately, so an
//!   unmeasured turn is distinguishable from a genuinely cheap one.
//! - A review reports usage as all zeros. A session of reviews never advances
//!   the total at all.
//!
//! Treat a budget as a floor on consumption rather than an exact measure.
//!
//! # Example
//!
//! ```
//! use codex_wrapper::TokenBudget;
//!
//! let budget = TokenBudget::builder()
//!     .max_tokens(100_000)
//!     .warn_at_tokens(80_000)
//!     .on_warning(|total| eprintln!("at {total} tokens"))
//!     .build();
//!
//! budget.record(Some(50_000));
//! assert_eq!(budget.total_tokens(), 50_000);
//! assert_eq!(budget.remaining_tokens(), Some(50_000));
//! assert!(budget.check().is_ok());
//!
//! budget.record(Some(60_000));
//! assert!(budget.check().is_err());
//! ```

use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};

type Callback = Arc<dyn Fn(u64) + Send + Sync>;

#[derive(Default)]
struct Config {
    max_tokens: Option<u64>,
    warn_at_tokens: Option<u64>,
    on_warning: Option<Callback>,
    on_exceeded: Option<Callback>,
}

#[derive(Default)]
struct State {
    total_tokens: u64,
    turns_missing_usage: usize,
    warned: bool,
    exceeded: bool,
}

struct Inner {
    config: Config,
    state: Mutex<State>,
}

/// Cumulative token budget with threshold callbacks.
///
/// Cloning shares one running total, so a single budget can span several
/// [`Session`](crate::Session)s. See the [module docs](crate::budget) for what
/// a budget can and cannot see.
#[derive(Clone)]
pub struct TokenBudget {
    inner: Arc<Inner>,
}

impl TokenBudget {
    /// Start building a budget.
    #[must_use]
    pub fn builder() -> TokenBudgetBuilder {
        TokenBudgetBuilder::default()
    }

    /// Add a turn's token usage to the running total.
    ///
    /// `None` records a turn the CLI reported no usage for. It does not move
    /// the total, and is counted by
    /// [`turns_missing_usage`](Self::turns_missing_usage) so the gap stays
    /// visible rather than reading as zero consumption.
    ///
    /// Fires `on_warning` the first time the total reaches `warn_at_tokens`,
    /// and `on_exceeded` the first time it reaches `max_tokens`.
    pub fn record(&self, tokens: Option<u64>) {
        let Some(tokens) = tokens else {
            self.inner
                .state
                .lock()
                .expect("budget mutex poisoned")
                .turns_missing_usage += 1;
            return;
        };

        let (warn_fired, exceeded_fired, total) = {
            let mut state = self.inner.state.lock().expect("budget mutex poisoned");
            state.total_tokens = state.total_tokens.saturating_add(tokens);

            let warn_fired = match self.inner.config.warn_at_tokens {
                Some(threshold) if !state.warned && state.total_tokens >= threshold => {
                    state.warned = true;
                    true
                }
                _ => false,
            };

            let exceeded_fired = match self.inner.config.max_tokens {
                Some(threshold) if !state.exceeded && state.total_tokens >= threshold => {
                    state.exceeded = true;
                    true
                }
                _ => false,
            };

            (warn_fired, exceeded_fired, state.total_tokens)
        };

        // Fired outside the lock: a callback that touches this budget would
        // otherwise deadlock on it.
        if warn_fired && let Some(cb) = &self.inner.config.on_warning {
            cb(total);
        }
        if exceeded_fired && let Some(cb) = &self.inner.config.on_exceeded {
            cb(total);
        }
    }

    /// `Err(Error::TokenBudgetExceeded)` once the total reaches `max_tokens`.
    ///
    /// `Ok(())` when no ceiling is set.
    pub fn check(&self) -> Result<()> {
        let Some(max_tokens) = self.inner.config.max_tokens else {
            return Ok(());
        };
        let total_tokens = self.total_tokens();
        if total_tokens >= max_tokens {
            Err(Error::TokenBudgetExceeded {
                total_tokens,
                max_tokens,
            })
        } else {
            Ok(())
        }
    }

    /// Tokens recorded so far.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.inner
            .state
            .lock()
            .expect("budget mutex poisoned")
            .total_tokens
    }

    /// Turns recorded with no usage reported.
    ///
    /// These consumed tokens the total does not include, so a non-zero count
    /// means the total is a floor rather than a measure.
    #[must_use]
    pub fn turns_missing_usage(&self) -> usize {
        self.inner
            .state
            .lock()
            .expect("budget mutex poisoned")
            .turns_missing_usage
    }

    /// Tokens left before the ceiling, or `None` when there is no ceiling.
    ///
    /// Saturates at zero rather than going negative: a turn can overshoot,
    /// since usage is only known once it has been spent.
    #[must_use]
    pub fn remaining_tokens(&self) -> Option<u64> {
        self.inner
            .config
            .max_tokens
            .map(|max| max.saturating_sub(self.total_tokens()))
    }

    /// The configured ceiling, if any.
    #[must_use]
    pub fn max_tokens(&self) -> Option<u64> {
        self.inner.config.max_tokens
    }

    /// The configured warning threshold, if any.
    #[must_use]
    pub fn warn_at_tokens(&self) -> Option<u64> {
        self.inner.config.warn_at_tokens
    }

    /// Clear the running total and re-arm both thresholds.
    pub fn reset(&self) {
        *self.inner.state.lock().expect("budget mutex poisoned") = State::default();
    }
}

impl std::fmt::Debug for TokenBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBudget")
            .field("total_tokens", &self.total_tokens())
            .field("max_tokens", &self.max_tokens())
            .field("warn_at_tokens", &self.warn_at_tokens())
            .field("turns_missing_usage", &self.turns_missing_usage())
            .finish()
    }
}

/// Builder for [`TokenBudget`].
#[derive(Default)]
pub struct TokenBudgetBuilder {
    config: Config,
}

impl TokenBudgetBuilder {
    /// Stop at this many tokens. Without one, the budget only counts.
    #[must_use]
    pub fn max_tokens(mut self, max: u64) -> Self {
        self.config.max_tokens = Some(max);
        self
    }

    /// Fire `on_warning` once the total reaches this many tokens.
    #[must_use]
    pub fn warn_at_tokens(mut self, warn: u64) -> Self {
        self.config.warn_at_tokens = Some(warn);
        self
    }

    /// Called once, with the running total, when `warn_at_tokens` is reached.
    #[must_use]
    pub fn on_warning<F>(mut self, f: F) -> Self
    where
        F: Fn(u64) + Send + Sync + 'static,
    {
        self.config.on_warning = Some(Arc::new(f));
        self
    }

    /// Called once, with the running total, when `max_tokens` is reached.
    #[must_use]
    pub fn on_exceeded<F>(mut self, f: F) -> Self
    where
        F: Fn(u64) + Send + Sync + 'static,
    {
        self.config.on_exceeded = Some(Arc::new(f));
        self
    }

    /// Build the budget.
    #[must_use]
    pub fn build(self) -> TokenBudget {
        TokenBudget {
            inner: Arc::new(Inner {
                config: self.config,
                state: Mutex::new(State::default()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[test]
    fn records_and_reports_a_running_total() {
        let budget = TokenBudget::builder().max_tokens(1000).build();
        budget.record(Some(400));
        budget.record(Some(350));

        assert_eq!(budget.total_tokens(), 750);
        assert_eq!(budget.remaining_tokens(), Some(250));
        assert!(budget.check().is_ok());
    }

    #[test]
    fn check_fails_once_the_ceiling_is_reached() {
        let budget = TokenBudget::builder().max_tokens(100).build();
        budget.record(Some(100));

        let err = budget.check().unwrap_err();
        assert!(
            matches!(
                err,
                Error::TokenBudgetExceeded {
                    total_tokens: 100,
                    max_tokens: 100
                }
            ),
            "{err:?}"
        );
    }

    /// A turn can only be measured after it has run, so the total can land
    /// past the ceiling. Remaining must floor at zero rather than wrap.
    #[test]
    fn remaining_saturates_instead_of_wrapping() {
        let budget = TokenBudget::builder().max_tokens(100).build();
        budget.record(Some(250));

        assert_eq!(budget.remaining_tokens(), Some(0));
        assert_eq!(budget.total_tokens(), 250);
    }

    #[test]
    fn no_ceiling_means_counting_only() {
        let budget = TokenBudget::builder().build();
        budget.record(Some(u64::MAX));

        assert!(budget.check().is_ok());
        assert_eq!(budget.remaining_tokens(), None);
    }

    /// An unreported turn must not read as zero consumption, which would let
    /// a session run indefinitely against a ceiling it never approaches.
    #[test]
    fn a_turn_without_usage_is_counted_not_ignored() {
        let budget = TokenBudget::builder().max_tokens(100).build();
        budget.record(None);
        budget.record(Some(10));
        budget.record(None);

        assert_eq!(budget.total_tokens(), 10);
        assert_eq!(budget.turns_missing_usage(), 2);
    }

    #[test]
    fn callbacks_fire_once_each() {
        let warnings = Arc::new(AtomicU64::new(0));
        let exceeded = Arc::new(AtomicU64::new(0));
        let w = Arc::clone(&warnings);
        let e = Arc::clone(&exceeded);

        let budget = TokenBudget::builder()
            .warn_at_tokens(50)
            .max_tokens(100)
            .on_warning(move |_| {
                w.fetch_add(1, Ordering::SeqCst);
            })
            .on_exceeded(move |_| {
                e.fetch_add(1, Ordering::SeqCst);
            })
            .build();

        for _ in 0..10 {
            budget.record(Some(30));
        }

        assert_eq!(warnings.load(Ordering::SeqCst), 1);
        assert_eq!(exceeded.load(Ordering::SeqCst), 1);
    }

    /// A callback that reads the budget must not deadlock, which it would if
    /// callbacks fired while the state lock was held.
    #[test]
    fn a_callback_can_read_the_budget_it_belongs_to() {
        let seen = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        let budget = TokenBudget::builder().max_tokens(10).build();
        let handle = budget.clone();

        let budget = TokenBudget::builder()
            .max_tokens(10)
            .on_exceeded(move |_| {
                *sink.lock().unwrap() = Some(handle.total_tokens());
            })
            .build();

        budget.record(Some(20));
        assert!(seen.lock().unwrap().is_some());
    }

    #[test]
    fn clones_share_one_total() {
        let budget = TokenBudget::builder().max_tokens(100).build();
        let other = budget.clone();

        budget.record(Some(60));
        other.record(Some(50));

        assert_eq!(budget.total_tokens(), 110);
        assert!(other.check().is_err());
    }

    #[test]
    fn reset_clears_the_total_and_rearms_the_thresholds() {
        let fired = Arc::new(AtomicU64::new(0));
        let f = Arc::clone(&fired);
        let budget = TokenBudget::builder()
            .max_tokens(100)
            .on_exceeded(move |_| {
                f.fetch_add(1, Ordering::SeqCst);
            })
            .build();

        budget.record(Some(150));
        budget.record(None);
        budget.reset();

        assert_eq!(budget.total_tokens(), 0);
        assert_eq!(budget.turns_missing_usage(), 0);
        assert!(budget.check().is_ok());

        budget.record(Some(150));
        assert_eq!(fired.load(Ordering::SeqCst), 2, "threshold must re-arm");
    }
}
