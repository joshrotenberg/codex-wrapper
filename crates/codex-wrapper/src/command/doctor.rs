//! Diagnose the local Codex installation (`codex doctor`).

use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Diagnose local Codex installation, config, auth, and runtime health.
///
/// Wraps `codex doctor`. Use [`json`](DoctorCommand::json) for a redacted
/// machine-readable report on stdout.
#[derive(Debug, Clone)]
pub struct DoctorCommand {
    json: bool,
    summary: bool,
    all: bool,
    no_color: bool,
    ascii: bool,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl DoctorCommand {
    /// Create a new doctor command.
    #[must_use]
    pub fn new() -> Self {
        Self {
            json: false,
            summary: false,
            all: false,
            no_color: false,
            ascii: false,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            retry_policy: None,
        }
    }

    /// Emit a redacted machine-readable report (`--json`).
    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    /// Only show grouped check rows and the final count summary (`--summary`).
    #[must_use]
    pub fn summary(mut self) -> Self {
        self.summary = true;
        self
    }

    /// Expand long lists in detailed human output (`--all`).
    #[must_use]
    pub fn all(mut self) -> Self {
        self.all = true;
        self
    }

    /// Disable ANSI color in human output (`--no-color`).
    #[must_use]
    pub fn no_color(mut self) -> Self {
        self.no_color = true;
        self
    }

    /// Use ASCII status labels and separators in human output (`--ascii`).
    #[must_use]
    pub fn ascii(mut self) -> Self {
        self.ascii = true;
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

impl Default for DoctorCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for DoctorCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["doctor".to_string()];
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
        if self.json {
            args.push("--json".into());
        }
        if self.summary {
            args.push("--summary".into());
        }
        if self.all {
            args.push("--all".into());
        }
        if self.no_color {
            args.push("--no-color".into());
        }
        if self.ascii {
            args.push("--ascii".into());
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
    fn doctor_args_default() {
        let args = DoctorCommand::new().args();
        assert_eq!(args, vec!["doctor"]);
    }

    #[test]
    fn doctor_args_flags() {
        let args = DoctorCommand::new().json().summary().all().args();
        assert_eq!(args, vec!["doctor", "--json", "--summary", "--all"]);
    }

    #[test]
    fn doctor_args_config_and_output() {
        let args = DoctorCommand::new()
            .config("model=o3")
            .no_color()
            .ascii()
            .args();
        assert_eq!(
            args,
            vec!["doctor", "-c", "model=o3", "--no-color", "--ascii"]
        );
    }
}
