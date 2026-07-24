//! Manage Codex plugins (`codex plugin`).
//!
//! Covers the direct plugin operations (`add`, `list`, `remove`) and the
//! nested marketplace-source management commands (`plugin marketplace add /
//! list / upgrade / remove`).
//!
//! Plugin selectors follow the CLI's `PLUGIN@MARKETPLACE` form; the
//! [`marketplace`](PluginAddCommand::marketplace) builder covers the
//! `PLUGIN` + `-m MARKETPLACE` alternative.

use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Append the `-c`/`--enable`/`--disable` passthrough shared by every builder.
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

/// Install a plugin from a configured marketplace snapshot
/// (`codex plugin add <PLUGIN[@MARKETPLACE]>`).
#[derive(Debug, Clone)]
pub struct PluginAddCommand {
    selector: String,
    marketplace: Option<String>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl PluginAddCommand {
    /// Create an add command for a plugin selector (`PLUGIN` or
    /// `PLUGIN@MARKETPLACE`).
    #[must_use]
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            marketplace: None,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Marketplace name to use when the selector omits `@MARKETPLACE` (`-m`).
    #[must_use]
    pub fn marketplace(mut self, name: impl Into<String>) -> Self {
        self.marketplace = Some(name.into());
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

impl CodexCommand for PluginAddCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["plugin".to_string(), "add".to_string()];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if let Some(name) = &self.marketplace {
            args.push("--marketplace".into());
            args.push(name.clone());
        }
        args.push(self.selector.clone());
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

/// List plugins available from configured marketplace snapshots
/// (`codex plugin list`).
#[derive(Debug, Clone)]
pub struct PluginListCommand {
    marketplace: Option<String>,
    json: bool,
    available: bool,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl PluginListCommand {
    /// Create a plugin list command.
    #[must_use]
    pub fn new() -> Self {
        Self {
            marketplace: None,
            json: false,
            available: false,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Only list plugins from this configured marketplace name (`-m`).
    #[must_use]
    pub fn marketplace(mut self, name: impl Into<String>) -> Self {
        self.marketplace = Some(name.into());
        self
    }

    /// Output the plugin list as JSON (`--json`).
    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    /// Include uninstalled marketplace plugins in the JSON output
    /// (`--available`).
    #[must_use]
    pub fn available(mut self) -> Self {
        self.available = true;
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

impl Default for PluginListCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for PluginListCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["plugin".to_string(), "list".to_string()];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if let Some(name) = &self.marketplace {
            args.push("--marketplace".into());
            args.push(name.clone());
        }
        if self.json {
            args.push("--json".into());
        }
        if self.available {
            args.push("--available".into());
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

/// Remove an installed plugin from local config and cache
/// (`codex plugin remove <PLUGIN[@MARKETPLACE]>`).
#[derive(Debug, Clone)]
pub struct PluginRemoveCommand {
    selector: String,
    marketplace: Option<String>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl PluginRemoveCommand {
    /// Create a remove command for a plugin selector (`PLUGIN` or
    /// `PLUGIN@MARKETPLACE`).
    #[must_use]
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            marketplace: None,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Marketplace name to use when the selector omits `@MARKETPLACE` (`-m`).
    #[must_use]
    pub fn marketplace(mut self, name: impl Into<String>) -> Self {
        self.marketplace = Some(name.into());
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

impl CodexCommand for PluginRemoveCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["plugin".to_string(), "remove".to_string()];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if let Some(name) = &self.marketplace {
            args.push("--marketplace".into());
            args.push(name.clone());
        }
        args.push(self.selector.clone());
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

/// Add a local or Git marketplace source
/// (`codex plugin marketplace add <SOURCE>`).
#[derive(Debug, Clone)]
pub struct PluginMarketplaceAddCommand {
    source: String,
    git_ref: Option<String>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl PluginMarketplaceAddCommand {
    /// Create a marketplace-add command for a source (local path,
    /// `owner/repo[@ref]`, HTTPS Git URL, or SSH Git URL).
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            git_ref: None,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Git ref to fetch for Git marketplace sources (`--ref`).
    #[must_use]
    pub fn git_ref(mut self, git_ref: impl Into<String>) -> Self {
        self.git_ref = Some(git_ref.into());
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

impl CodexCommand for PluginMarketplaceAddCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec![
            "plugin".to_string(),
            "marketplace".to_string(),
            "add".to_string(),
        ];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if let Some(git_ref) = &self.git_ref {
            args.push("--ref".into());
            args.push(git_ref.clone());
        }
        args.push(self.source.clone());
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

/// List configured plugin marketplaces (`codex plugin marketplace list`).
#[derive(Debug, Clone)]
pub struct PluginMarketplaceListCommand {
    json: bool,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl PluginMarketplaceListCommand {
    /// Create a marketplace list command.
    #[must_use]
    pub fn new() -> Self {
        Self {
            json: false,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Output the marketplace list as JSON (`--json`).
    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
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

impl Default for PluginMarketplaceListCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for PluginMarketplaceListCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec![
            "plugin".to_string(),
            "marketplace".to_string(),
            "list".to_string(),
        ];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if self.json {
            args.push("--json".into());
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

/// Refresh configured Git marketplace snapshots
/// (`codex plugin marketplace upgrade [MARKETPLACE_NAME]`).
///
/// Omit the name to upgrade all configured Git marketplaces.
#[derive(Debug, Clone)]
pub struct PluginMarketplaceUpgradeCommand {
    name: Option<String>,
    json: bool,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl PluginMarketplaceUpgradeCommand {
    /// Create a marketplace upgrade command (upgrades all Git marketplaces
    /// unless [`name`](PluginMarketplaceUpgradeCommand::name) is set).
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: None,
            json: false,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Upgrade only this configured marketplace name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Output the upgrade result as JSON (`--json`).
    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
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

impl Default for PluginMarketplaceUpgradeCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for PluginMarketplaceUpgradeCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec![
            "plugin".to_string(),
            "marketplace".to_string(),
            "upgrade".to_string(),
        ];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if self.json {
            args.push("--json".into());
        }
        if let Some(name) = &self.name {
            args.push(name.clone());
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex_with_retry(codex, self.args(), self.retry_policy.as_ref()).await
    }
}

/// Remove a configured marketplace source by name
/// (`codex plugin marketplace remove <MARKETPLACE_NAME>`).
#[derive(Debug, Clone)]
pub struct PluginMarketplaceRemoveCommand {
    name: String,
    json: bool,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl PluginMarketplaceRemoveCommand {
    /// Create a marketplace remove command for the given marketplace name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            json: false,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Output the remove result as JSON (`--json`).
    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
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

impl CodexCommand for PluginMarketplaceRemoveCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec![
            "plugin".to_string(),
            "marketplace".to_string(),
            "remove".to_string(),
        ];
        push_config(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if self.json {
            args.push("--json".into());
        }
        args.push(self.name.clone());
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
    fn plugin_add_args() {
        let args = PluginAddCommand::new("hello@official").args();
        assert_eq!(args, vec!["plugin", "add", "hello@official"]);
    }

    #[test]
    fn plugin_add_args_with_marketplace_and_config() {
        let args = PluginAddCommand::new("hello")
            .marketplace("official")
            .config("foo=bar")
            .args();
        assert_eq!(
            args,
            vec![
                "plugin",
                "add",
                "-c",
                "foo=bar",
                "--marketplace",
                "official",
                "hello",
            ]
        );
    }

    #[test]
    fn plugin_list_args() {
        let args = PluginListCommand::new().json().available().args();
        assert_eq!(args, vec!["plugin", "list", "--json", "--available"]);
    }

    #[test]
    fn plugin_remove_args() {
        let args = PluginRemoveCommand::new("hello")
            .marketplace("official")
            .args();
        assert_eq!(
            args,
            vec!["plugin", "remove", "--marketplace", "official", "hello"]
        );
    }

    #[test]
    fn marketplace_add_args() {
        let args = PluginMarketplaceAddCommand::new("owner/repo")
            .git_ref("main")
            .args();
        assert_eq!(
            args,
            vec![
                "plugin",
                "marketplace",
                "add",
                "--ref",
                "main",
                "owner/repo",
            ]
        );
    }

    #[test]
    fn marketplace_list_args() {
        let args = PluginMarketplaceListCommand::new().json().args();
        assert_eq!(args, vec!["plugin", "marketplace", "list", "--json"]);
    }

    #[test]
    fn marketplace_upgrade_args_all() {
        let args = PluginMarketplaceUpgradeCommand::new().args();
        assert_eq!(args, vec!["plugin", "marketplace", "upgrade"]);
    }

    #[test]
    fn marketplace_upgrade_args_named() {
        let args = PluginMarketplaceUpgradeCommand::new()
            .name("official")
            .args();
        assert_eq!(args, vec!["plugin", "marketplace", "upgrade", "official"]);
    }

    #[test]
    fn marketplace_remove_args() {
        let args = PluginMarketplaceRemoveCommand::new("official")
            .json()
            .args();
        assert_eq!(
            args,
            vec!["plugin", "marketplace", "remove", "--json", "official"]
        );
    }
}
