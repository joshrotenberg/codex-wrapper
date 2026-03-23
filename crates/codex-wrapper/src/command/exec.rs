use crate::Codex;
use crate::command::CodexCommand;
#[cfg(feature = "json")]
use crate::error::Error;
use crate::error::Result;
use crate::exec::{self, CommandOutput};
#[cfg(feature = "json")]
use crate::types::JsonLineEvent;
use crate::types::{ApprovalPolicy, Color, SandboxMode};

/// Run Codex non-interactively (`codex exec <prompt>`).
///
/// This is the primary command for programmatic use. It supports the full
/// range of exec flags: model selection, sandbox policy, approval policy,
/// images, config overrides, feature flags, JSON output, and more.
///
/// # Example
///
/// ```no_run
/// use codex_wrapper::{Codex, CodexCommand, ExecCommand, SandboxMode};
///
/// # async fn example() -> codex_wrapper::Result<()> {
/// let codex = Codex::builder().build()?;
/// let output = ExecCommand::new("fix the failing test")
///     .model("o3")
///     .sandbox(SandboxMode::WorkspaceWrite)
///     .ephemeral()
///     .execute(&codex)
///     .await?;
/// println!("{}", output.stdout);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ExecCommand {
    prompt: Option<String>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    images: Vec<String>,
    model: Option<String>,
    oss: bool,
    local_provider: Option<String>,
    sandbox: Option<SandboxMode>,
    approval_policy: Option<ApprovalPolicy>,
    profile: Option<String>,
    full_auto: bool,
    dangerously_bypass_approvals_and_sandbox: bool,
    cd: Option<String>,
    skip_git_repo_check: bool,
    add_dirs: Vec<String>,
    search: bool,
    ephemeral: bool,
    output_schema: Option<String>,
    color: Option<Color>,
    progress_cursor: bool,
    json: bool,
    output_last_message: Option<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl ExecCommand {
    /// Create a new exec command with the given prompt.
    #[must_use]
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            images: Vec::new(),
            model: None,
            oss: false,
            local_provider: None,
            sandbox: None,
            approval_policy: None,
            profile: None,
            full_auto: false,
            dangerously_bypass_approvals_and_sandbox: false,
            cd: None,
            skip_git_repo_check: false,
            add_dirs: Vec::new(),
            search: false,
            ephemeral: false,
            output_schema: None,
            color: None,
            progress_cursor: false,
            json: false,
            output_last_message: None,
            retry_policy: None,
        }
    }

    /// Read the prompt from stdin (`-`).
    #[must_use]
    pub fn from_stdin() -> Self {
        Self::new("-")
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
    pub fn image(mut self, path: impl Into<String>) -> Self {
        self.images.push(path.into());
        self
    }

    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    #[must_use]
    pub fn oss(mut self) -> Self {
        self.oss = true;
        self
    }

    #[must_use]
    pub fn local_provider(mut self, provider: impl Into<String>) -> Self {
        self.local_provider = Some(provider.into());
        self
    }

    #[must_use]
    pub fn sandbox(mut self, sandbox: SandboxMode) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    #[must_use]
    pub fn approval_policy(mut self, policy: ApprovalPolicy) -> Self {
        self.approval_policy = Some(policy);
        self
    }

    #[must_use]
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
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
    pub fn cd(mut self, dir: impl Into<String>) -> Self {
        self.cd = Some(dir.into());
        self
    }

    #[must_use]
    pub fn skip_git_repo_check(mut self) -> Self {
        self.skip_git_repo_check = true;
        self
    }

    #[must_use]
    pub fn add_dir(mut self, dir: impl Into<String>) -> Self {
        self.add_dirs.push(dir.into());
        self
    }

    /// Enable live web search.
    #[must_use]
    pub fn search(mut self) -> Self {
        self.search = true;
        self
    }

    #[must_use]
    pub fn ephemeral(mut self) -> Self {
        self.ephemeral = true;
        self
    }

    #[must_use]
    pub fn output_schema(mut self, path: impl Into<String>) -> Self {
        self.output_schema = Some(path.into());
        self
    }

    #[must_use]
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    #[must_use]
    pub fn progress_cursor(mut self) -> Self {
        self.progress_cursor = true;
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
        parse_json_lines(&output.stdout)
    }
}

impl CodexCommand for ExecCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["exec".to_string()];

        push_repeat(&mut args, "-c", &self.config_overrides);
        push_repeat(&mut args, "--enable", &self.enabled_features);
        push_repeat(&mut args, "--disable", &self.disabled_features);
        push_repeat(&mut args, "--image", &self.images);

        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
        }
        if self.oss {
            args.push("--oss".into());
        }
        if let Some(local_provider) = &self.local_provider {
            args.push("--local-provider".into());
            args.push(local_provider.clone());
        }
        if let Some(sandbox) = self.sandbox {
            args.push("--sandbox".into());
            args.push(sandbox.as_arg().into());
        }
        if let Some(policy) = self.approval_policy {
            args.push("--ask-for-approval".into());
            args.push(policy.as_arg().into());
        }
        if let Some(profile) = &self.profile {
            args.push("--profile".into());
            args.push(profile.clone());
        }
        if self.full_auto {
            args.push("--full-auto".into());
        }
        if self.dangerously_bypass_approvals_and_sandbox {
            args.push("--dangerously-bypass-approvals-and-sandbox".into());
        }
        if let Some(cd) = &self.cd {
            args.push("--cd".into());
            args.push(cd.clone());
        }
        if self.skip_git_repo_check {
            args.push("--skip-git-repo-check".into());
        }
        push_repeat(&mut args, "--add-dir", &self.add_dirs);
        if self.search {
            args.push("--search".into());
        }
        if self.ephemeral {
            args.push("--ephemeral".into());
        }
        if let Some(output_schema) = &self.output_schema {
            args.push("--output-schema".into());
            args.push(output_schema.clone());
        }
        if let Some(color) = self.color {
            args.push("--color".into());
            args.push(color.as_arg().into());
        }
        if self.progress_cursor {
            args.push("--progress-cursor".into());
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

/// Resume a previous non-interactive session (`codex exec resume`).
///
/// Use [`session_id`](ExecResumeCommand::session_id) to target a specific
/// session, or [`last`](ExecResumeCommand::last) to pick the most recent.
#[derive(Debug, Clone)]
pub struct ExecResumeCommand {
    session_id: Option<String>,
    prompt: Option<String>,
    last: bool,
    all: bool,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    images: Vec<String>,
    model: Option<String>,
    full_auto: bool,
    dangerously_bypass_approvals_and_sandbox: bool,
    skip_git_repo_check: bool,
    ephemeral: bool,
    json: bool,
    output_last_message: Option<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl ExecResumeCommand {
    #[must_use]
    pub fn new() -> Self {
        Self {
            session_id: None,
            prompt: None,
            last: false,
            all: false,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            images: Vec::new(),
            model: None,
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
    pub fn session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    #[must_use]
    pub fn last(mut self) -> Self {
        self.last = true;
        self
    }

    #[must_use]
    pub fn all(mut self) -> Self {
        self.all = true;
        self
    }

    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    #[must_use]
    pub fn image(mut self, path: impl Into<String>) -> Self {
        self.images.push(path.into());
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
    pub fn retry(mut self, policy: crate::retry::RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }
}

impl Default for ExecResumeCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for ExecResumeCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["exec".into(), "resume".into()];
        push_repeat(&mut args, "-c", &self.config_overrides);
        push_repeat(&mut args, "--enable", &self.enabled_features);
        push_repeat(&mut args, "--disable", &self.disabled_features);
        if self.last {
            args.push("--last".into());
        }
        if self.all {
            args.push("--all".into());
        }
        push_repeat(&mut args, "--image", &self.images);
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
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
        if let Some(session_id) = &self.session_id {
            args.push(session_id.clone());
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

fn push_repeat(args: &mut Vec<String>, flag: &str, values: &[String]) {
    for value in values {
        args.push(flag.into());
        args.push(value.clone());
    }
}

#[cfg(feature = "json")]
fn parse_json_lines(stdout: &str) -> Result<Vec<JsonLineEvent>> {
    stdout
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_args() {
        let args = ExecCommand::new("fix the test")
            .model("gpt-5")
            .sandbox(SandboxMode::WorkspaceWrite)
            .approval_policy(ApprovalPolicy::OnRequest)
            .skip_git_repo_check()
            .ephemeral()
            .json()
            .args();

        assert_eq!(
            args,
            vec![
                "exec",
                "--model",
                "gpt-5",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "on-request",
                "--skip-git-repo-check",
                "--ephemeral",
                "--json",
                "fix the test",
            ]
        );
    }

    #[test]
    fn exec_resume_args() {
        let args = ExecResumeCommand::new()
            .last()
            .model("gpt-5")
            .json()
            .prompt("continue")
            .args();

        assert_eq!(
            args,
            vec![
                "exec", "resume", "--last", "--model", "gpt-5", "--json", "continue",
            ]
        );
    }
}
