use crate::Codex;
use crate::command::CodexCommand;
use crate::error::{Error, Result};
use crate::exec::{self, CommandOutput};
use crate::types::JsonLineEvent;

#[derive(Debug, Clone)]
pub struct ReviewCommand {
    prompt: Option<String>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    uncommitted: bool,
    base: Option<String>,
    commit: Option<String>,
    model: Option<String>,
    title: Option<String>,
    full_auto: bool,
    dangerously_bypass_approvals_and_sandbox: bool,
    skip_git_repo_check: bool,
    ephemeral: bool,
    json: bool,
    output_last_message: Option<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl ReviewCommand {
    #[must_use]
    pub fn new() -> Self {
        Self {
            prompt: None,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            uncommitted: false,
            base: None,
            commit: None,
            model: None,
            title: None,
            full_auto: false,
            dangerously_bypass_approvals_and_sandbox: false,
            skip_git_repo_check: false,
            ephemeral: false,
            json: false,
            output_last_message: None,
            retry_policy: None,
        }
    }

    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
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
    pub fn uncommitted(mut self) -> Self {
        self.uncommitted = true;
        self
    }

    #[must_use]
    pub fn base(mut self, branch: impl Into<String>) -> Self {
        self.base = Some(branch.into());
        self
    }

    #[must_use]
    pub fn commit(mut self, sha: impl Into<String>) -> Self {
        self.commit = Some(sha.into());
        self
    }

    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    #[must_use]
    pub fn full_auto(mut self) -> Self {
        self.full_auto = true;
        self
    }

    #[must_use]
    pub fn dangerously_bypass_approvals_and_sandbox(mut self) -> Self {
        self.dangerously_bypass_approvals_and_sandbox = true;
        self
    }

    #[must_use]
    pub fn skip_git_repo_check(mut self) -> Self {
        self.skip_git_repo_check = true;
        self
    }

    #[must_use]
    pub fn ephemeral(mut self) -> Self {
        self.ephemeral = true;
        self
    }

    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    #[must_use]
    pub fn output_last_message(mut self, path: impl Into<String>) -> Self {
        self.output_last_message = Some(path.into());
        self
    }

    #[must_use]
    pub fn retry(mut self, policy: crate::retry::RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    #[cfg(feature = "json")]
    pub async fn execute_json_lines(&self, codex: &Codex) -> Result<Vec<JsonLineEvent>> {
        let mut args = self.args();
        if !self.json {
            args.push("--json".into());
        }

        let output = exec::run_codex_with_retry(codex, args, self.retry_policy.as_ref()).await?;
        output
            .stdout
            .lines()
            .filter(|line| line.trim_start().starts_with('{'))
            .map(|line| {
                serde_json::from_str(line).map_err(|source| Error::Json {
                    message: format!("failed to parse JSONL event: {line}"),
                    source,
                })
            })
            .collect()
    }
}

impl Default for ReviewCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for ReviewCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["exec".into(), "review".into()];
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
        if self.uncommitted {
            args.push("--uncommitted".into());
        }
        if let Some(base) = &self.base {
            args.push("--base".into());
            args.push(base.clone());
        }
        if let Some(commit) = &self.commit {
            args.push("--commit".into());
            args.push(commit.clone());
        }
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
        }
        if let Some(title) = &self.title {
            args.push("--title".into());
            args.push(title.clone());
        }
        if self.full_auto {
            args.push("--full-auto".into());
        }
        if self.dangerously_bypass_approvals_and_sandbox {
            args.push("--dangerously-bypass-approvals-and-sandbox".into());
        }
        if self.skip_git_repo_check {
            args.push("--skip-git-repo-check".into());
        }
        if self.ephemeral {
            args.push("--ephemeral".into());
        }
        if self.json {
            args.push("--json".into());
        }
        if let Some(path) = &self.output_last_message {
            args.push("--output-last-message".into());
            args.push(path.clone());
        }
        if let Some(prompt) = &self.prompt {
            args.push(prompt.clone());
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
    fn review_args() {
        let args = ReviewCommand::new()
            .uncommitted()
            .model("gpt-5")
            .json()
            .prompt("focus on correctness")
            .args();

        assert_eq!(
            args,
            vec![
                "exec",
                "review",
                "--uncommitted",
                "--model",
                "gpt-5",
                "--json",
                "focus on correctness",
            ]
        );
    }
}
