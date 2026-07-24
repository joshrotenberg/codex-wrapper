//! Update Codex to the latest version (`codex update`).

use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Update Codex to the latest version.
///
/// Wraps `codex update`.
#[derive(Debug, Clone)]
pub struct UpdateCommand {
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl UpdateCommand {
    /// Create a new update command.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Override a config key (`-c key=value`). May be called multiple times.
    #[must_use]
    pub fn config(mut self, key_value: impl Into<String>) -> Self {
        self.config_overrides.push(key_value.into());
        self
    }

    /// Enable an optional feature flag (`--enable <feature>`).
    #[must_use]
    pub fn enable(mut self, feature: impl Into<String>) -> Self {
        self.enabled_features.push(feature.into());
        self
    }

    /// Disable an optional feature flag (`--disable <feature>`).
    #[must_use]
    pub fn disable(mut self, feature: impl Into<String>) -> Self {
        self.disabled_features.push(feature.into());
        self
    }

    /// Override the retry policy for this command.
    #[must_use]
    pub fn retry(mut self, policy: crate::retry::RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }
}

impl Default for UpdateCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for UpdateCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["update".to_string()];
        for value in &self.config_overrides {
            args.push("-c".into());
            args.push(value.clone());
        }
        for value in &self.enabled_features {
            args.push("--enable".into());
            args.push(value.clone());
        }
        for value in &self.disabled_features {
            args.push("--disable".into());
            args.push(value.clone());
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_args_default() {
        let args = UpdateCommand::new().args();
        assert_eq!(args, vec!["update"]);
    }

    #[test]
    fn update_args_config() {
        let args = UpdateCommand::new().config("foo=bar").enable("beta").args();
        assert_eq!(args, vec!["update", "-c", "foo=bar", "--enable", "beta"]);
    }
}
