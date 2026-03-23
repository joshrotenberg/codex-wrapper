/// Run commands within a Codex-provided sandbox.
///
/// Wraps `codex sandbox <macos|linux|windows> -- <command> [args...]`.
use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Target sandbox platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxPlatform {
    /// macOS Seatbelt sandbox.
    MacOs,
    /// Linux sandbox (bubblewrap by default).
    Linux,
    /// Windows restricted token sandbox.
    Windows,
}

impl SandboxPlatform {
    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            Self::MacOs => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }
}

/// Run a command within a Codex-provided sandbox.
#[derive(Debug, Clone)]
pub struct SandboxCommand {
    platform: SandboxPlatform,
    command: String,
    command_args: Vec<String>,
}

impl SandboxCommand {
    /// Create a sandbox command for the given platform and command.
    #[must_use]
    pub fn new(platform: SandboxPlatform, command: impl Into<String>) -> Self {
        Self {
            platform,
            command: command.into(),
            command_args: Vec::new(),
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
}

impl CodexCommand for SandboxCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec![
            "sandbox".into(),
            self.platform.as_arg().into(),
            "--".into(),
            self.command.clone(),
        ];
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
    fn sandbox_macos_args() {
        let cmd = SandboxCommand::new(SandboxPlatform::MacOs, "ls").arg("-la");
        assert_eq!(
            CodexCommand::args(&cmd),
            vec!["sandbox", "macos", "--", "ls", "-la"]
        );
    }

    #[test]
    fn sandbox_linux_args() {
        let cmd = SandboxCommand::new(SandboxPlatform::Linux, "cat").arg("/etc/hosts");
        assert_eq!(
            CodexCommand::args(&cmd),
            vec!["sandbox", "linux", "--", "cat", "/etc/hosts"]
        );
    }
}
