/// Feature flag management.
///
/// Wraps `codex features <list|enable|disable>`.
use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// List known feature flags with their stage and effective state.
#[derive(Debug, Clone, Default)]
pub struct FeaturesListCommand;

impl FeaturesListCommand {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl CodexCommand for FeaturesListCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        vec!["features".into(), "list".into()]
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

/// Enable a feature flag in config.toml.
#[derive(Debug, Clone)]
pub struct FeaturesEnableCommand {
    feature: String,
}

impl FeaturesEnableCommand {
    #[must_use]
    pub fn new(feature: impl Into<String>) -> Self {
        Self {
            feature: feature.into(),
        }
    }
}

impl CodexCommand for FeaturesEnableCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        vec!["features".into(), "enable".into(), self.feature.clone()]
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

/// Disable a feature flag in config.toml.
#[derive(Debug, Clone)]
pub struct FeaturesDisableCommand {
    feature: String,
}

impl FeaturesDisableCommand {
    #[must_use]
    pub fn new(feature: impl Into<String>) -> Self {
        Self {
            feature: feature.into(),
        }
    }
}

impl CodexCommand for FeaturesDisableCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        vec!["features".into(), "disable".into(), self.feature.clone()]
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_list_args() {
        assert_eq!(FeaturesListCommand::new().args(), vec!["features", "list"]);
    }

    #[test]
    fn features_enable_args() {
        assert_eq!(
            FeaturesEnableCommand::new("web-search").args(),
            vec!["features", "enable", "web-search"]
        );
    }

    #[test]
    fn features_disable_args() {
        assert_eq!(
            FeaturesDisableCommand::new("web-search").args(),
            vec!["features", "disable", "web-search"]
        );
    }
}
