/// Generate shell completion scripts.
///
/// Wraps `codex completion [SHELL]`.
use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Supported shells for completion generation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shell {
    #[default]
    Bash,
    Elvish,
    Fish,
    Powershell,
    Zsh,
}

impl Shell {
    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Elvish => "elvish",
            Self::Fish => "fish",
            Self::Powershell => "powershell",
            Self::Zsh => "zsh",
        }
    }
}

/// Generate shell completion scripts for the Codex CLI.
#[derive(Debug, Clone)]
pub struct CompletionCommand {
    shell: Option<Shell>,
}

impl CompletionCommand {
    #[must_use]
    pub fn new() -> Self {
        Self { shell: None }
    }

    /// Set the target shell (defaults to bash if not specified).
    #[must_use]
    pub fn shell(mut self, shell: Shell) -> Self {
        self.shell = Some(shell);
        self
    }
}

impl Default for CompletionCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for CompletionCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["completion".into()];
        if let Some(shell) = self.shell {
            args.push(shell.as_arg().into());
        }
        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_default_args() {
        assert_eq!(CompletionCommand::new().args(), vec!["completion"]);
    }

    #[test]
    fn completion_zsh_args() {
        assert_eq!(
            CompletionCommand::new().shell(Shell::Zsh).args(),
            vec!["completion", "zsh"]
        );
    }

    #[test]
    fn completion_fish_args() {
        assert_eq!(
            CompletionCommand::new().shell(Shell::Fish).args(),
            vec!["completion", "fish"]
        );
    }
}
