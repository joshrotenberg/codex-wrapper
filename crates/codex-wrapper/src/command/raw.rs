use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Run an arbitrary codex subcommand with raw arguments.
///
/// Use this when the higher-level command builders do not cover the subcommand
/// or flags you need. The first argument passed to [`RawCommand::new`] becomes
/// the subcommand name; additional arguments are appended with
/// [`arg`](RawCommand::arg) or [`args`](RawCommand::args).
///
/// # Example
///
/// ```no_run
/// use codex_wrapper::{Codex, CodexCommand, RawCommand};
///
/// # async fn example() -> codex_wrapper::Result<()> {
/// let codex = Codex::builder().build()?;
/// let output = RawCommand::new("mcp")
///     .args(["list", "--json"])
///     .execute(&codex)
///     .await?;
/// println!("{}", output.stdout);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RawCommand {
    command_args: Vec<String>,
}

impl RawCommand {
    /// Create a new raw command with the given subcommand name.
    #[must_use]
    pub fn new(subcommand: impl Into<String>) -> Self {
        Self {
            command_args: vec![subcommand.into()],
        }
    }

    /// Append a single argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.command_args.push(arg.into());
        self
    }

    /// Append multiple arguments.
    #[must_use]
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.command_args.extend(args.into_iter().map(Into::into));
        self
    }
}

impl CodexCommand for RawCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        self.command_args.clone()
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
    fn raw_command_args() {
        let cmd = RawCommand::new("mcp").args(["list", "--json"]);
        assert_eq!(CodexCommand::args(&cmd), vec!["mcp", "list", "--json"]);
    }
}
