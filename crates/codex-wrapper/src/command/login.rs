use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

#[derive(Debug, Clone, Default)]
pub struct LoginCommand {
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    with_api_key: bool,
    device_auth: bool,
}

impl LoginCommand {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn config(mut self, key_value: impl Into<String>) -> Self {
        self.config_overrides.push(key_value.into());
        self
    }

    #[must_use]
    pub fn enable(mut self, feature: impl Into<String>) -> Self {
        self.enabled_features.push(feature.into());
        self
    }

    #[must_use]
    pub fn disable(mut self, feature: impl Into<String>) -> Self {
        self.disabled_features.push(feature.into());
        self
    }

    #[must_use]
    pub fn with_api_key(mut self) -> Self {
        self.with_api_key = true;
        self
    }

    #[must_use]
    pub fn device_auth(mut self) -> Self {
        self.device_auth = true;
        self
    }
}

impl CodexCommand for LoginCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["login".into()];
        push_common(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        if self.with_api_key {
            args.push("--with-api-key".into());
        }
        if self.device_auth {
            args.push("--device-auth".into());
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoginStatusCommand {
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
}

impl LoginStatusCommand {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn config(mut self, key_value: impl Into<String>) -> Self {
        self.config_overrides.push(key_value.into());
        self
    }

    #[must_use]
    pub fn enable(mut self, feature: impl Into<String>) -> Self {
        self.enabled_features.push(feature.into());
        self
    }

    #[must_use]
    pub fn disable(mut self, feature: impl Into<String>) -> Self {
        self.disabled_features.push(feature.into());
        self
    }
}

impl CodexCommand for LoginStatusCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["login".into(), "status".into()];
        push_common(
            &mut args,
            &self.config_overrides,
            &self.enabled_features,
            &self.disabled_features,
        );
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[derive(Debug, Clone, Default)]
pub struct LogoutCommand;

impl LogoutCommand {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl CodexCommand for LogoutCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        vec!["logout".into()]
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

fn push_common(
    args: &mut Vec<String>,
    configs: &[String],
    enabled: &[String],
    disabled: &[String],
) {
    for value in configs {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_args() {
        assert_eq!(
            LoginCommand::new().with_api_key().args(),
            vec!["login", "--with-api-key"]
        );
    }

    #[test]
    fn login_status_args() {
        assert_eq!(LoginStatusCommand::new().args(), vec!["login", "status"]);
    }

    #[test]
    fn logout_args() {
        assert_eq!(LogoutCommand::new().args(), vec!["logout"]);
    }
}
