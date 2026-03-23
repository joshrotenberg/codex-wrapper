use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Print the codex version string (`codex --version`).
///
/// # Example
///
/// ```no_run
/// use codex_wrapper::{Codex, CodexCommand, VersionCommand};
///
/// # async fn example() -> codex_wrapper::Result<()> {
/// let codex = Codex::builder().build()?;
/// let output = VersionCommand::new().execute(&codex).await?;
/// println!("{}", output.stdout);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct VersionCommand;

impl VersionCommand {
    /// Create a new version command.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl CodexCommand for VersionCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        vec!["--version".to_string()]
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_args() {
        assert_eq!(VersionCommand::new().args(), vec!["--version"]);
    }
}
