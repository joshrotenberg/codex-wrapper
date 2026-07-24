//! Session-lifecycle commands: archive, delete, and unarchive a saved session.
//!
//! These wrap `codex archive`, `codex delete`, and `codex unarchive`. Each
//! targets a saved session by id (UUID) or name; UUIDs take precedence when the
//! value parses as one.
//!
//! The underlying subcommands inherit a large shared option block from the CLI
//! (model, image, sandbox, remote, and so on), almost none of which is
//! meaningful for a lifecycle operation. These builders expose only the useful
//! surface: the session target, the `-c` / `--enable` / `--disable` config
//! passthrough, and (for delete) `--force`.

use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Append `-c`/`--enable`/`--disable` passthrough args shared by the trio.
fn push_config(
    args: &mut Vec<String>,
    config_overrides: &[String],
    enabled: &[String],
    disabled: &[String],
) {
    for value in config_overrides {
        args.push("-c".into());
        args.push(value.clone());
    }
    for value in enabled {
        args.push("--enable".into());
        args.push(value.clone());
    }
    for value in disabled {
        args.push("--disable".into());
        args.push(value.clone());
    }
}

/// Archive a saved session (`codex archive <SESSION>`).
#[derive(Debug, Clone)]
pub struct ArchiveCommand {
    session: String,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl ArchiveCommand {
    /// Create an archive command targeting the given session id or name.
    #[must_use]
    pub fn new(session: impl Into<String>) -> Self {
        Self {
            session: session.into(),
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

impl CodexCommand for ArchiveCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["archive".to_string()];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        args.push(self.session.clone());
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

/// Permanently delete a saved session (`codex delete <SESSION>`).
#[derive(Debug, Clone)]
pub struct DeleteCommand {
    session: String,
    force: bool,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl DeleteCommand {
    /// Create a delete command targeting the given session id or name.
    #[must_use]
    pub fn new(session: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            force: false,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Skip the confirmation prompt (`--force`).
    #[must_use]
    pub fn force(mut self) -> Self {
        self.force = true;
        self
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

impl CodexCommand for DeleteCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["delete".to_string()];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if self.force {
            args.push("--force".into());
        }
        args.push(self.session.clone());
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

/// Unarchive a previously archived session (`codex unarchive <SESSION>`).
#[derive(Debug, Clone)]
pub struct UnarchiveCommand {
    session: String,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl UnarchiveCommand {
    /// Create an unarchive command targeting the given session id or name.
    #[must_use]
    pub fn new(session: impl Into<String>) -> Self {
        Self {
            session: session.into(),
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

impl CodexCommand for UnarchiveCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["unarchive".to_string()];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        args.push(self.session.clone());
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
    fn archive_args() {
        let args = ArchiveCommand::new("sess-1").args();
        assert_eq!(args, vec!["archive", "sess-1"]);
    }

    #[test]
    fn archive_args_with_config() {
        let args = ArchiveCommand::new("sess-1")
            .config("foo=bar")
            .enable("beta")
            .args();
        assert_eq!(
            args,
            vec!["archive", "-c", "foo=bar", "--enable", "beta", "sess-1"]
        );
    }

    #[test]
    fn delete_args() {
        let args = DeleteCommand::new("sess-2").args();
        assert_eq!(args, vec!["delete", "sess-2"]);
    }

    #[test]
    fn delete_args_force() {
        let args = DeleteCommand::new("sess-2").force().args();
        assert_eq!(args, vec!["delete", "--force", "sess-2"]);
    }

    #[test]
    fn unarchive_args() {
        let args = UnarchiveCommand::new("sess-3").args();
        assert_eq!(args, vec!["unarchive", "sess-3"]);
    }
}
