use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

#[derive(Debug, Clone)]
pub struct RawCommand {
    command_args: Vec<String>,
}

impl RawCommand {
    #[must_use]
    pub fn new(subcommand: impl Into<String>) -> Self {
        Self {
            command_args: vec![subcommand.into()],
        }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.command_args.push(arg.into());
        self
    }

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
