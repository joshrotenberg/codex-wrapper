use crate::Codex;
use crate::command::CodexCommand;
use crate::command::exec::push_typed_config;
#[cfg(feature = "json")]
use crate::error::Error;
use crate::error::Result;
use crate::exec::{self, CommandOutput};
#[cfg(feature = "json")]
use crate::types::JsonLineEvent;
use crate::types::{ApprovalPolicyConfig, SandboxMode, WebSearchMode};

/// Run a code review non-interactively (`codex exec review`).
///
/// # Why not `codex review`
///
/// `codex-cli` exposes review at two paths, `codex review` and
/// `codex exec review`. They are the same command: same `[PROMPT]` positional,
/// same non-interactive behavior, and a byte-identical error when no review
/// scope is given.
///
/// They are not equally capable. As of 0.145.0, top-level `codex review`
/// accepts a strict subset of the flags, missing ten that `codex exec review`
/// has:
///
/// ```text
/// --dangerously-bypass-approvals-and-sandbox   --json
/// --dangerously-bypass-hook-trust              --output-schema <FILE>
/// --ephemeral                                  --skip-git-repo-check
/// --ignore-rules                               -m, --model <MODEL>
/// --ignore-user-config                         -o, --output-last-message <FILE>
/// ```
///
/// Nothing is available on `codex review` that is not also on
/// `codex exec review`, and the missing flags are rejected outright rather
/// than silently ignored.
///
/// `--json` is the decisive one:
/// [`execute_json_lines`](Self::execute_json_lines) depends on it, so a
/// builder targeting the top-level path could not offer structured output at
/// all. This wrapper therefore targets `codex exec review` only. Use
/// [`RawCommand`](crate::RawCommand) if you need the literal `codex review`
/// invocation.
///
/// `tests/contract.rs` asserts the subset relationship still holds, so if the
/// two surfaces ever diverge the other way, CI reports it.
#[derive(Debug, Clone)]
pub struct ReviewCommand {
    prompt: Option<String>,
    approval_policy: Option<ApprovalPolicyConfig>,
    web_search: Option<WebSearchMode>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    uncommitted: bool,
    base: Option<String>,
    commit: Option<String>,
    model: Option<String>,
    title: Option<String>,
    strict_config: bool,
    dangerously_bypass_hook_trust: bool,
    full_auto: bool,
    dangerously_bypass_approvals_and_sandbox: bool,
    skip_git_repo_check: bool,
    ephemeral: bool,
    ignore_user_config: bool,
    ignore_rules: bool,
    output_schema: Option<String>,
    json: bool,
    output_last_message: Option<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl ReviewCommand {
    #[must_use]
    pub fn new() -> Self {
        Self {
            prompt: None,
            approval_policy: None,
            web_search: None,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            uncommitted: false,
            base: None,
            commit: None,
            model: None,
            title: None,
            strict_config: false,
            dangerously_bypass_hook_trust: false,
            full_auto: false,
            dangerously_bypass_approvals_and_sandbox: false,
            skip_git_repo_check: false,
            ephemeral: false,
            ignore_user_config: false,
            ignore_rules: false,
            output_schema: None,
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

    /// Override a config key (`-c key=value`).
    ///
    /// Because `-c` is last-wins, a key set here overrides the same key set by
    /// [`approval_policy`](Self::approval_policy),
    /// [`search_mode`](Self::search_mode), or [`full_auto`](Self::full_auto).
    #[must_use]
    pub fn config(mut self, key_value: impl Into<String>) -> Self {
        self.config_overrides.push(key_value.into());
        self
    }

    /// Set when the model asks for approval (`-c approval_policy="<value>"`).
    ///
    /// `codex-cli` 0.145.0 removed `--ask-for-approval` from the exec family;
    /// the config key is the supported equivalent. Accepts an
    /// [`ApprovalPolicy`](crate::ApprovalPolicy) directly, or an
    /// [`ApprovalPolicyConfig`] for the two values the flag never took.
    #[must_use]
    pub fn approval_policy(mut self, policy: impl Into<ApprovalPolicyConfig>) -> Self {
        self.approval_policy = Some(policy.into());
        self
    }

    /// Enable live web search.
    ///
    /// Shorthand for `search_mode(WebSearchMode::Live)`, which is what the
    /// removed `--search` flag meant.
    #[must_use]
    pub fn search(self) -> Self {
        self.search_mode(WebSearchMode::Live)
    }

    /// Set the web search mode (`-c web_search="<value>"`).
    ///
    /// `codex-cli` 0.145.0 removed `--search` from the exec family; the config
    /// key is the supported equivalent, and it is an enum rather than the
    /// flag's boolean.
    #[must_use]
    pub fn search_mode(mut self, mode: WebSearchMode) -> Self {
        self.web_search = Some(mode);
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

    /// Error on unrecognized config keys (`--strict-config`).
    #[must_use]
    pub fn strict_config(mut self) -> Self {
        self.strict_config = true;
        self
    }

    /// Bypass the hook trust prompt (`--dangerously-bypass-hook-trust`).
    ///
    /// Allows configured hooks to run without confirmation. Use with caution.
    #[must_use]
    pub fn dangerously_bypass_hook_trust(mut self) -> Self {
        self.dangerously_bypass_hook_trust = true;
        self
    }

    /// Run in full-auto mode, emitted as `-c sandbox_mode="workspace-write"`.
    ///
    /// `--full-auto` is deprecated upstream; `codex-cli` 0.145.0 hides it and
    /// warns to use `--sandbox workspace-write` instead. `codex exec review`
    /// has no `--sandbox` flag, so this sets the equivalent config key.
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

    /// Ignore the user-level config file (`--ignore-user-config`).
    #[must_use]
    pub fn ignore_user_config(mut self) -> Self {
        self.ignore_user_config = true;
        self
    }

    /// Ignore project rules files (`--ignore-rules`).
    #[must_use]
    pub fn ignore_rules(mut self) -> Self {
        self.ignore_rules = true;
        self
    }

    /// Require output to conform to a JSON schema (`--output-schema <path>`).
    #[must_use]
    pub fn output_schema(mut self, path: impl Into<String>) -> Self {
        self.output_schema = Some(path.into());
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

    /// Execute the review and return a typed
    /// [`QueryResult`](crate::types::QueryResult).
    ///
    /// Review emits the same event vocabulary as `codex exec`, so the review
    /// comments arrive as the `agent_message` item that
    /// [`result`](crate::types::QueryResult::result) is assembled from. Use
    /// [`execute_json_lines`](Self::execute_json_lines) for the raw stream.
    /// Requires the `json` feature.
    ///
    /// One difference from exec, observed on `codex-cli` 0.145.0: the
    /// `turn.completed` event of a review reports a usage object of all
    /// zeros, so [`usage`](crate::types::QueryResult::usage) is present but
    /// carries no counts.
    #[cfg(feature = "json")]
    pub async fn execute_json(&self, codex: &Codex) -> Result<crate::types::QueryResult> {
        let events = self.execute_json_lines(codex).await?;
        Ok(crate::types::QueryResult::from_events(events))
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
        push_typed_config(&mut args, self.approval_policy, self.web_search);
        // `exec review` has no `--sandbox` flag, so the `--full-auto`
        // replacement has to go through the config key.
        if self.full_auto {
            args.push("-c".into());
            args.push(format!(
                "sandbox_mode=\"{}\"",
                SandboxMode::WorkspaceWrite.as_arg()
            ));
        }
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
        if self.strict_config {
            args.push("--strict-config".into());
        }
        if self.dangerously_bypass_approvals_and_sandbox {
            args.push("--dangerously-bypass-approvals-and-sandbox".into());
        }
        if self.dangerously_bypass_hook_trust {
            args.push("--dangerously-bypass-hook-trust".into());
        }
        if self.skip_git_repo_check {
            args.push("--skip-git-repo-check".into());
        }
        if self.ephemeral {
            args.push("--ephemeral".into());
        }
        if self.ignore_user_config {
            args.push("--ignore-user-config".into());
        }
        if self.ignore_rules {
            args.push("--ignore-rules".into());
        }
        if let Some(output_schema) = &self.output_schema {
            args.push("--output-schema".into());
            args.push(output_schema.clone());
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
    use crate::types::ApprovalPolicy;

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

    #[test]
    fn review_new_flags() {
        let args = ReviewCommand::new()
            .uncommitted()
            .strict_config()
            .dangerously_bypass_hook_trust()
            .args();

        assert_eq!(
            args,
            vec![
                "exec",
                "review",
                "--uncommitted",
                "--strict-config",
                "--dangerously-bypass-hook-trust",
            ]
        );
    }

    #[test]
    fn review_approval_and_search_emit_config_keys() {
        let args = ReviewCommand::new()
            .uncommitted()
            .approval_policy(ApprovalPolicy::Untrusted)
            .search()
            .args();
        assert_eq!(
            args,
            vec![
                "exec",
                "review",
                "-c",
                "approval_policy=\"untrusted\"",
                "-c",
                "web_search=\"live\"",
                "--uncommitted"
            ]
        );
    }

    /// `codex exec review` has no `--sandbox` flag, so the `--full-auto`
    /// replacement goes through the config key. See #55.
    #[test]
    fn review_full_auto_emits_sandbox_config_key() {
        let args = ReviewCommand::new().uncommitted().full_auto().args();
        assert_eq!(
            args,
            vec![
                "exec",
                "review",
                "-c",
                "sandbox_mode=\"workspace-write\"",
                "--uncommitted"
            ]
        );
        assert!(!args.iter().any(|a| a == "--full-auto"));
    }

    /// #65: these three were listed in #41 P1 but never landed on
    /// `ReviewCommand`.
    #[test]
    fn review_ignore_and_output_schema_args() {
        let args = ReviewCommand::new()
            .uncommitted()
            .ignore_user_config()
            .ignore_rules()
            .output_schema("/tmp/schema.json")
            .args();
        assert_eq!(
            args,
            vec![
                "exec",
                "review",
                "--uncommitted",
                "--ignore-user-config",
                "--ignore-rules",
                "--output-schema",
                "/tmp/schema.json"
            ]
        );
    }

    /// #70 asked whether `QueryResult::from_events` holds up on review output.
    /// It does: the fixture is a transcript of a real review run, and the
    /// review comments land in `result` the same way an exec answer does.
    #[cfg(all(unix, feature = "json"))]
    #[tokio::test]
    async fn review_execute_json_assembles_a_query_result() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-review.sh");
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .build()
            .expect("bash must exist");

        let result = ReviewCommand::new()
            .uncommitted()
            .execute_json(&codex)
            .await
            .unwrap();

        assert_eq!(result.result, "- [P1] Keep add performing addition");
        assert_eq!(
            result.thread_id.as_deref(),
            Some("019fd952-7ce9-7662-8a20-9c33c1718dca")
        );
        // The command_execution items the reviewer ran are in the stream but
        // must not leak into the result text.
        assert!(!result.result.contains("git diff"));
        assert_eq!(result.events.len(), 6);
        // Review reports usage, but a real run reports it as all zeros.
        assert_eq!(result.usage.and_then(|u| u.total()), Some(0));
    }
}
