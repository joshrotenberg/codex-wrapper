/// Apply the latest diff produced by a Codex agent task.
///
/// Wraps `codex apply <task-id>`.
use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};

/// Apply a Codex agent diff as `git apply` to the local working tree.
#[derive(Debug, Clone)]
pub struct ApplyCommand {
    task_id: String,
}

impl ApplyCommand {
    /// Create an apply command for the given task ID.
    #[must_use]
    pub fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
        }
    }
}

impl CodexCommand for ApplyCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        vec!["apply".into(), self.task_id.clone()]
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        exec::run_codex(codex, self.args()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_args() {
        let args = ApplyCommand::new("abc-123").args();
        assert_eq!(args, vec!["apply", "abc-123"]);
    }
}
