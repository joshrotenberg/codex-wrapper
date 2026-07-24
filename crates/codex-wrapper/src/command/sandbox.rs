//! Run commands within a Codex-provided sandbox (`codex sandbox`).
//!
//! As of `codex-cli` 0.145.0 the platform is auto-detected (Seatbelt on macOS,
//! and so on); the old `codex sandbox <macos|linux|windows>` positional was
//! removed. The command to run is passed after a `--` separator.

use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Run a command within a Codex-provided sandbox.
///
/// Wraps `codex sandbox [OPTIONS] -- <command> [args...]`.
#[derive(Debug, Clone)]
pub struct SandboxCommand {
    command: String,
    command_args: Vec<String>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    permission_profile: Option<String>,
    profile: Option<String>,
    cd: Option<String>,
}

impl SandboxCommand {
    /// Create a sandbox command for the given program.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            command_args: Vec::new(),
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            permission_profile: None,
            profile: None,
            cd: None,
        }
    }

    /// Add an argument to the sandboxed command.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.command_args.push(arg.into());
        self
    }

    /// Add multiple arguments to the sandboxed command.
    #[must_use]
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.command_args.extend(args.into_iter().map(Into::into));
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

    /// Named permissions profile to apply (`-P, --permission-profile <NAME>`).
    #[must_use]
    pub fn permission_profile(mut self, name: impl Into<String>) -> Self {
        self.permission_profile = Some(name.into());
        self
    }

    /// Named config profile to layer on top of the base config
    /// (`-p, --profile <NAME>`).
    #[must_use]
    pub fn profile(mut self, name: impl Into<String>) -> Self {
        self.profile = Some(name.into());
        self
    }

    /// Working directory for profile resolution and command execution
    /// (`-C, --cd <DIR>`).
    #[must_use]
    pub fn cd(mut self, dir: impl Into<String>) -> Self {
        self.cd = Some(dir.into());
        self
    }
}

impl CodexCommand for SandboxCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["sandbox".to_string()];
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
        if let Some(name) = &self.permission_profile {
            args.push("--permission-profile".into());
            args.push(name.clone());
        }
        if let Some(name) = &self.profile {
            args.push("--profile".into());
            args.push(name.clone());
        }
        if let Some(dir) = &self.cd {
            args.push("--cd".into());
            args.push(dir.clone());
        }
        args.push("--".into());
        args.push(self.command.clone());
        args.extend(self.command_args.clone());
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CodexCommand;

    #[test]
    fn sandbox_basic_args() {
        let cmd = SandboxCommand::new("ls").arg("-la");
        assert_eq!(CodexCommand::args(&cmd), vec!["sandbox", "--", "ls", "-la"]);
    }

    #[test]
    fn sandbox_args_with_options() {
        let cmd = SandboxCommand::new("cat")
            .permission_profile("readonly")
            .cd("/tmp")
            .args(["/etc/hosts"]);
        assert_eq!(
            CodexCommand::args(&cmd),
            vec![
                "sandbox",
                "--permission-profile",
                "readonly",
                "--cd",
                "/tmp",
                "--",
                "cat",
                "/etc/hosts",
            ]
        );
    }
}
