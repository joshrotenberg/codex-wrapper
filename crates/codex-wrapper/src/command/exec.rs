use crate::Codex;
use crate::command::CodexCommand;
#[cfg(feature = "json")]
use crate::error::Error;
use crate::error::Result;
use crate::exec::{self, CommandOutput};
use crate::types::{ApprovalPolicyConfig, Color, SandboxMode, WebSearchMode};
#[cfg(feature = "json")]
use crate::types::{JsonLineEvent, QueryResult};

/// Push the typed config-key overrides shared by the exec-family builders.
///
/// `codex-cli` 0.145.0 removed `--ask-for-approval` and `--search` from the
/// exec family; both settings moved to `-c` config keys. These are pushed
/// before any caller-supplied [`config`](ExecCommand::config) strings because
/// `-c` is last-wins, so a raw override still beats the typed setter.
pub(crate) fn push_typed_config(
    args: &mut Vec<String>,
    approval_policy: Option<ApprovalPolicyConfig>,
    web_search: Option<WebSearchMode>,
) {
    if let Some(policy) = approval_policy {
        args.push("-c".into());
        args.push(format!("approval_policy=\"{}\"", policy.as_config_value()));
    }
    if let Some(mode) = web_search {
        args.push("-c".into());
        args.push(format!("web_search=\"{}\"", mode.as_config_value()));
    }
}

/// Resolve the sandbox mode, folding in the deprecated `full_auto` shim.
///
/// `--full-auto` is hidden on the exec family (and rejected outright by `fork`
/// and `resume`); the CLI's own advice is `--sandbox workspace-write`. An
/// explicit `sandbox()` call is more specific and wins.
pub(crate) fn effective_sandbox(
    sandbox: Option<SandboxMode>,
    full_auto: bool,
) -> Option<SandboxMode> {
    sandbox.or(full_auto.then_some(SandboxMode::WorkspaceWrite))
}

/// Run Codex non-interactively (`codex exec <prompt>`).
///
/// This is the primary command for programmatic use. It supports the full
/// range of exec flags: model selection, sandbox policy, images, config
/// overrides, feature flags, JSON output, and more.
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
    prompt_via_stdin: bool,
    approval_policy: Option<ApprovalPolicyConfig>,
    web_search: Option<WebSearchMode>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    images: Vec<String>,
    model: Option<String>,
    oss: bool,
    local_provider: Option<String>,
    sandbox: Option<SandboxMode>,
    strict_config: bool,
    dangerously_bypass_hook_trust: bool,
    ignore_user_config: bool,
    ignore_rules: bool,
    profile: Option<String>,
    full_auto: bool,
    dangerously_bypass_approvals_and_sandbox: bool,
    cd: Option<String>,
    skip_git_repo_check: bool,
    add_dirs: Vec<String>,
    ephemeral: bool,
    output_schema: Option<String>,
    color: Option<Color>,
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
            prompt_via_stdin: false,
            approval_policy: None,
            web_search: None,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            images: Vec::new(),
            model: None,
            oss: false,
            local_provider: None,
            sandbox: None,
            strict_config: false,
            dangerously_bypass_hook_trust: false,
            ignore_user_config: false,
            ignore_rules: false,
            profile: None,
            full_auto: false,
            dangerously_bypass_approvals_and_sandbox: false,
            cd: None,
            skip_git_repo_check: false,
            add_dirs: Vec::new(),
            ephemeral: false,
            output_schema: None,
            color: None,
            json: false,
            output_last_message: None,
            retry_policy: None,
        }
    }

    /// Send the prompt on stdin instead of as an argument (`codex exec -`).
    ///
    /// Shorthand for [`new`](Self::new) followed by
    /// [`prompt_via_stdin`](Self::prompt_via_stdin). Use it for prompts that
    /// are large or awkward to pass through argv.
    ///
    /// ```no_run
    /// use codex_wrapper::{Codex, CodexCommand, ExecCommand};
    ///
    /// # async fn example() -> codex_wrapper::Result<()> {
    /// let codex = Codex::builder().build()?;
    /// let diff = std::fs::read_to_string("huge.patch")?;
    /// let output = ExecCommand::from_stdin(format!("Review this patch:\n{diff}"))
    ///     .execute(&codex)
    ///     .await?;
    /// # let _ = output;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Before 0.3 this took no argument and set the prompt to the literal
    /// `-`, which could not work: nothing wrote to the child's stdin, so the
    /// CLI saw an immediate EOF and an empty prompt (#81).
    #[must_use]
    pub fn from_stdin(prompt: impl Into<String>) -> Self {
        Self::new(prompt).prompt_via_stdin()
    }

    /// Deliver this command's prompt on stdin rather than in argv.
    ///
    /// The prompt is replaced by `-` in the argument list and written to the
    /// child's stdin instead.
    ///
    /// Retry does not apply to a stdin prompt, and any policy set on the
    /// command or the client is ignored for it. A second attempt would need to
    /// write the prompt again, into a pipe the first attempt has already
    /// consumed, and retrying with an empty stdin would be worse than not
    /// retrying.
    #[must_use]
    pub fn prompt_via_stdin(mut self) -> Self {
        self.prompt_via_stdin = true;
        self
    }

    /// The prompt to write to the child's stdin, if this command sends it
    /// there. `None` when the prompt travels in argv.
    ///
    /// Only the streaming path needs this, and that path is `json`-gated.
    #[cfg(feature = "json")]
    pub(crate) fn stdin_prompt(&self) -> Option<&str> {
        self.prompt_via_stdin
            .then(|| self.prompt.as_deref().unwrap_or_default())
    }

    /// Override a config key (`-c key=value`).
    ///
    /// May be called multiple times to set several keys. Because `-c` is
    /// last-wins, a key set here overrides the same key set by
    /// [`approval_policy`](Self::approval_policy) or
    /// [`search_mode`](Self::search_mode).
    #[must_use]
    pub fn config(mut self, key_value: impl Into<String>) -> Self {
        self.config_overrides.push(key_value.into());
        self
    }

    /// Set when the model asks for approval (`-c approval_policy="<value>"`).
    ///
    /// `codex-cli` 0.145.0 removed `--ask-for-approval` from `codex exec`; the
    /// config key is the supported equivalent. Accepts an
    /// [`ApprovalPolicy`](crate::ApprovalPolicy) directly, or an
    /// [`ApprovalPolicyConfig`] for the two values the flag never took.
    ///
    /// ```
    /// use codex_wrapper::{ApprovalPolicyConfig, CodexCommand, ExecCommand};
    ///
    /// let args = ExecCommand::new("hi")
    ///     .approval_policy(ApprovalPolicyConfig::Never)
    ///     .args();
    /// assert!(args.windows(2).any(|w| w == ["-c", "approval_policy=\"never\""]));
    /// ```
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
    /// `codex-cli` 0.145.0 removed `--search` from `codex exec`; the config
    /// key is the supported equivalent, and it is an enum rather than the
    /// flag's boolean.
    #[must_use]
    pub fn search_mode(mut self, mode: WebSearchMode) -> Self {
        self.web_search = Some(mode);
        self
    }

    /// Enable an optional feature flag (`--enable <feature>`).
    ///
    /// May be called multiple times.
    #[must_use]
    pub fn enable(mut self, feature: impl Into<String>) -> Self {
        self.enabled_features.push(feature.into());
        self
    }

    /// Disable an optional feature flag (`--disable <feature>`).
    ///
    /// May be called multiple times.
    #[must_use]
    pub fn disable(mut self, feature: impl Into<String>) -> Self {
        self.disabled_features.push(feature.into());
        self
    }

    /// Attach an image to the prompt (`--image <path>`).
    ///
    /// May be called multiple times to attach several images.
    #[must_use]
    pub fn image(mut self, path: impl Into<String>) -> Self {
        self.images.push(path.into());
        self
    }

    /// Set the model to use (`--model <model>`).
    ///
    /// Panics if `model` is an empty string.
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        let model = model.into();
        assert!(!model.is_empty(), "model name must not be empty");
        self.model = Some(model);
        self
    }

    /// Use the OSS model tier (`--oss`).
    #[must_use]
    pub fn oss(mut self) -> Self {
        self.oss = true;
        self
    }

    /// Use a local model provider (`--local-provider <provider>`).
    #[must_use]
    pub fn local_provider(mut self, provider: impl Into<String>) -> Self {
        self.local_provider = Some(provider.into());
        self
    }

    /// Set the sandbox policy (`--sandbox <mode>`).
    #[must_use]
    pub fn sandbox(mut self, sandbox: SandboxMode) -> Self {
        self.sandbox = Some(sandbox);
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

    /// Select a named configuration profile (`--profile <name>`).
    #[must_use]
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }

    /// Run in full-auto mode, emitted as `--sandbox workspace-write`.
    ///
    /// `--full-auto` is deprecated upstream. `codex-cli` 0.145.0 hides it from
    /// `codex exec --help` and warns when it is used:
    ///
    /// ```text
    /// warning: `--full-auto` is deprecated; use `--sandbox workspace-write` instead.
    /// ```
    ///
    /// This method emits the replacement the CLI names. An explicit
    /// [`sandbox`](Self::sandbox) call is more specific and wins over it.
    #[must_use]
    pub fn full_auto(mut self) -> Self {
        self.full_auto = true;
        self
    }

    /// Bypass all approval prompts and sandbox restrictions.
    ///
    /// Passes `--dangerously-bypass-approvals-and-sandbox`. Use with caution.
    #[must_use]
    pub fn dangerously_bypass_approvals_and_sandbox(mut self) -> Self {
        self.dangerously_bypass_approvals_and_sandbox = true;
        self
    }

    /// Change the working directory before running (`--cd <dir>`).
    #[must_use]
    pub fn cd(mut self, dir: impl Into<String>) -> Self {
        self.cd = Some(dir.into());
        self
    }

    /// Skip the git repository check (`--skip-git-repo-check`).
    #[must_use]
    pub fn skip_git_repo_check(mut self) -> Self {
        self.skip_git_repo_check = true;
        self
    }

    /// Add an extra directory to the context (`--add-dir <dir>`).
    ///
    /// May be called multiple times.
    #[must_use]
    pub fn add_dir(mut self, dir: impl Into<String>) -> Self {
        self.add_dirs.push(dir.into());
        self
    }

    /// Run in ephemeral mode — no session is persisted (`--ephemeral`).
    #[must_use]
    pub fn ephemeral(mut self) -> Self {
        self.ephemeral = true;
        self
    }

    /// Require output to conform to a JSON schema (`--output-schema <path>`).
    #[must_use]
    pub fn output_schema(mut self, path: impl Into<String>) -> Self {
        self.output_schema = Some(path.into());
        self
    }

    /// Control terminal color output (`--color <mode>`).
    #[must_use]
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Emit JSON Lines output (`--json`).
    ///
    /// When set, stdout will contain one JSON object per line. Use
    /// [`execute_json_lines`](ExecCommand::execute_json_lines) to parse the
    /// events automatically (requires the `json` feature).
    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    /// Write the last assistant message to a file (`--output-last-message <path>`).
    #[must_use]
    pub fn output_last_message(mut self, path: impl Into<String>) -> Self {
        self.output_last_message = Some(path.into());
        self
    }

    /// Override the retry policy for this command.
    ///
    /// Takes precedence over the client-level policy set on [`Codex`].
    #[must_use]
    pub fn retry(mut self, policy: crate::retry::RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Stream JSONL events from the command, invoking `handler` for each
    /// parsed [`JsonLineEvent`] as it arrives.
    ///
    /// Automatically appends `--json` if not already set. Requires the `json`
    /// feature.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use codex_wrapper::{Codex, ExecCommand, JsonLineEvent};
    ///
    /// # async fn example() -> codex_wrapper::Result<()> {
    /// let codex = Codex::builder().build()?;
    /// ExecCommand::new("what is 2+2?")
    ///     .ephemeral()
    ///     .stream(&codex, |event: JsonLineEvent| {
    ///         println!("{}: {:?}", event.event_type, event.extra);
    ///     })
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "json")]
    pub async fn stream<F>(&self, codex: &Codex, handler: F) -> Result<()>
    where
        F: FnMut(JsonLineEvent),
    {
        crate::streaming::stream_exec(codex, self, handler).await
    }

    /// Execute the command and parse the output as JSON Lines events.
    ///
    /// Automatically appends `--json` if not already set. Requires the `json`
    /// feature.
    #[cfg(feature = "json")]
    pub async fn execute_json_lines(&self, codex: &Codex) -> Result<Vec<JsonLineEvent>> {
        let mut args = self.args();
        if !self.json {
            args.push("--json".into());
        }

        let output = if self.prompt_via_stdin {
            let prompt = self.prompt.as_deref().unwrap_or_default();
            exec::run_codex_with_stdin_prompt(codex, args, prompt).await?
        } else {
            exec::run_codex_with_retry(codex, args, self.retry_policy.as_ref()).await?
        };
        parse_json_lines(&output.stdout)
    }

    /// Execute the command and return a typed [`QueryResult`].
    ///
    /// Assembles the final result text, ids, and token usage from the JSONL
    /// event stream. Use [`execute_json_lines`](ExecCommand::execute_json_lines) for
    /// the raw event stream. Requires the `json` feature.
    #[cfg(feature = "json")]
    pub async fn execute_json(&self, codex: &Codex) -> Result<QueryResult> {
        let events = self.execute_json_lines(codex).await?;
        Ok(QueryResult::from_events(events))
    }
}

impl CodexCommand for ExecCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["exec".to_string()];

        push_typed_config(&mut args, self.approval_policy, self.web_search);
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
        if let Some(sandbox) = effective_sandbox(self.sandbox, self.full_auto) {
            args.push("--sandbox".into());
            args.push(sandbox.as_arg().into());
        }
        if self.strict_config {
            args.push("--strict-config".into());
        }
        if let Some(profile) = &self.profile {
            args.push("--profile".into());
            args.push(profile.clone());
        }
        if self.dangerously_bypass_approvals_and_sandbox {
            args.push("--dangerously-bypass-approvals-and-sandbox".into());
        }
        if self.dangerously_bypass_hook_trust {
            args.push("--dangerously-bypass-hook-trust".into());
        }
        if let Some(cd) = &self.cd {
            args.push("--cd".into());
            args.push(cd.clone());
        }
        if self.skip_git_repo_check {
            args.push("--skip-git-repo-check".into());
        }
        push_repeat(&mut args, "--add-dir", &self.add_dirs);
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
        if let Some(color) = self.color {
            args.push("--color".into());
            args.push(color.as_arg().into());
        }
        if self.json {
            args.push("--json".into());
        }
        if let Some(path) = &self.output_last_message {
            args.push("--output-last-message".into());
            args.push(path.clone());
        }
        if self.prompt_via_stdin {
            // The prompt travels on stdin; `-` is how the CLI is told to read
            // it from there.
            args.push("-".into());
        } else if let Some(prompt) = &self.prompt {
            args.push(prompt.clone());
        }

        args
    }

    async fn execute(&self, codex: &Codex) -> Result<CommandOutput> {
        if self.prompt_via_stdin {
            let prompt = self.prompt.as_deref().unwrap_or_default();
            return exec::run_codex_with_stdin_prompt(codex, self.args(), prompt).await;
        }
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
    approval_policy: Option<ApprovalPolicyConfig>,
    web_search: Option<WebSearchMode>,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    images: Vec<String>,
    model: Option<String>,
    strict_config: bool,
    dangerously_bypass_hook_trust: bool,
    ignore_user_config: bool,
    ignore_rules: bool,
    output_schema: Option<String>,
    full_auto: bool,
    dangerously_bypass_approvals_and_sandbox: bool,
    skip_git_repo_check: bool,
    ephemeral: bool,
    json: bool,
    output_last_message: Option<String>,
    retry_policy: Option<crate::retry::RetryPolicy>,
}

impl ExecResumeCommand {
    /// Create a new resume command with no options set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            session_id: None,
            prompt: None,
            last: false,
            all: false,
            approval_policy: None,
            web_search: None,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            images: Vec::new(),
            model: None,
            strict_config: false,
            dangerously_bypass_hook_trust: false,
            ignore_user_config: false,
            ignore_rules: false,
            output_schema: None,
            full_auto: false,
            dangerously_bypass_approvals_and_sandbox: false,
            skip_git_repo_check: false,
            ephemeral: false,
            json: false,
            output_last_message: None,
            retry_policy: None,
        }
    }

    /// Resume a specific session by its ID.
    #[must_use]
    pub fn session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Append an additional prompt to the resumed session.
    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Resume the most recent session (`--last`).
    #[must_use]
    pub fn last(mut self) -> Self {
        self.last = true;
        self
    }

    /// Resume all sessions (`--all`).
    #[must_use]
    pub fn all(mut self) -> Self {
        self.all = true;
        self
    }

    /// Set the model to use (`--model <model>`).
    ///
    /// Panics if `model` is an empty string.
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        let model = model.into();
        assert!(!model.is_empty(), "model name must not be empty");
        self.model = Some(model);
        self
    }

    /// Attach an image to the prompt (`--image <path>`).
    ///
    /// May be called multiple times to attach several images.
    #[must_use]
    pub fn image(mut self, path: impl Into<String>) -> Self {
        self.images.push(path.into());
        self
    }

    /// Emit JSON Lines output (`--json`).
    #[must_use]
    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    /// Write the last assistant message to a file (`--output-last-message <path>`).
    #[must_use]
    pub fn output_last_message(mut self, path: impl Into<String>) -> Self {
        self.output_last_message = Some(path.into());
        self
    }

    /// Override a config key (`-c key=value`).
    ///
    /// May be called multiple times to set several keys. Because `-c` is
    /// last-wins, a key set here overrides the same key set by
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

    /// Enable an optional feature flag (`--enable <feature>`).
    ///
    /// May be called multiple times.
    #[must_use]
    pub fn enable(mut self, feature: impl Into<String>) -> Self {
        self.enabled_features.push(feature.into());
        self
    }

    /// Disable an optional feature flag (`--disable <feature>`).
    ///
    /// May be called multiple times.
    #[must_use]
    pub fn disable(mut self, feature: impl Into<String>) -> Self {
        self.disabled_features.push(feature.into());
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

    /// Run in full-auto mode, emitted as `-c sandbox_mode="workspace-write"`.
    ///
    /// `--full-auto` is deprecated upstream; `codex-cli` 0.145.0 hides it and
    /// warns to use `--sandbox workspace-write` instead. `codex exec resume`
    /// has no `--sandbox` flag, so this sets the equivalent config key.
    #[must_use]
    pub fn full_auto(mut self) -> Self {
        self.full_auto = true;
        self
    }

    /// Bypass all approval prompts and sandbox restrictions.
    ///
    /// Passes `--dangerously-bypass-approvals-and-sandbox`. Use with caution.
    #[must_use]
    pub fn dangerously_bypass_approvals_and_sandbox(mut self) -> Self {
        self.dangerously_bypass_approvals_and_sandbox = true;
        self
    }

    /// Skip the git repository check (`--skip-git-repo-check`).
    #[must_use]
    pub fn skip_git_repo_check(mut self) -> Self {
        self.skip_git_repo_check = true;
        self
    }

    /// Run in ephemeral mode — no session is persisted (`--ephemeral`).
    #[must_use]
    pub fn ephemeral(mut self) -> Self {
        self.ephemeral = true;
        self
    }

    /// Override the retry policy for this command.
    ///
    /// Takes precedence over the client-level policy set on [`Codex`].
    #[must_use]
    pub fn retry(mut self, policy: crate::retry::RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Execute the command and parse the output as JSON Lines events.
    ///
    /// Automatically appends `--json` if not already set. Requires the `json`
    /// feature.
    #[cfg(feature = "json")]
    pub async fn execute_json_lines(&self, codex: &Codex) -> Result<Vec<JsonLineEvent>> {
        let mut args = self.args();
        if !self.json {
            args.push("--json".into());
        }

        let output = exec::run_codex_with_retry(codex, args, self.retry_policy.as_ref()).await?;
        parse_json_lines(&output.stdout)
    }

    /// Execute the resume command and return a typed [`QueryResult`].
    ///
    /// Assembles the final result text, ids, and token usage from the JSONL
    /// event stream. Requires the `json` feature.
    #[cfg(feature = "json")]
    pub async fn execute_json(&self, codex: &Codex) -> Result<QueryResult> {
        let events = self.execute_json_lines(codex).await?;
        Ok(QueryResult::from_events(events))
    }

    /// Stream JSONL events from the resume command, invoking `handler` for
    /// each parsed [`JsonLineEvent`] as it arrives.
    ///
    /// Automatically appends `--json` if not already set. Requires the `json`
    /// feature.
    #[cfg(feature = "json")]
    pub async fn stream<F>(&self, codex: &Codex, handler: F) -> Result<()>
    where
        F: FnMut(JsonLineEvent),
    {
        crate::streaming::stream_exec_resume(codex, self, handler).await
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
        push_typed_config(&mut args, self.approval_policy, self.web_search);
        // `exec resume` has no `--sandbox` flag, so the `--full-auto`
        // replacement has to go through the config key.
        if self.full_auto {
            args.push("-c".into());
            args.push(format!(
                "sandbox_mode=\"{}\"",
                SandboxMode::WorkspaceWrite.as_arg()
            ));
        }
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
    use crate::types::ApprovalPolicy;

    #[test]
    fn exec_args() {
        let args = ExecCommand::new("fix the test")
            .model("gpt-5")
            .sandbox(SandboxMode::WorkspaceWrite)
            .strict_config()
            .skip_git_repo_check()
            .ephemeral()
            .ignore_user_config()
            .ignore_rules()
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
                "--strict-config",
                "--skip-git-repo-check",
                "--ephemeral",
                "--ignore-user-config",
                "--ignore-rules",
                "--json",
                "fix the test",
            ]
        );
    }

    #[test]
    fn exec_args_hook_trust() {
        let args = ExecCommand::new("go")
            .dangerously_bypass_approvals_and_sandbox()
            .dangerously_bypass_hook_trust()
            .args();

        assert_eq!(
            args,
            vec![
                "exec",
                "--dangerously-bypass-approvals-and-sandbox",
                "--dangerously-bypass-hook-trust",
                "go",
            ]
        );
    }

    #[test]
    #[should_panic(expected = "model name must not be empty")]
    fn exec_model_empty_panics() {
        let _ = ExecCommand::new("prompt").model("");
    }

    #[test]
    #[should_panic(expected = "model name must not be empty")]
    fn exec_resume_model_empty_panics() {
        let _ = ExecResumeCommand::new().model("");
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

    #[test]
    fn exec_resume_new_flags() {
        let args = ExecResumeCommand::new()
            .last()
            .strict_config()
            .dangerously_bypass_hook_trust()
            .args();

        assert_eq!(
            args,
            vec![
                "exec",
                "resume",
                "--last",
                "--strict-config",
                "--dangerously-bypass-hook-trust",
            ]
        );
    }

    /// #53: `--ask-for-approval` and `--search` were removed from `codex exec`
    /// in codex-cli 0.145.0; the settings live on as config keys.
    #[test]
    fn exec_approval_and_search_emit_config_keys() {
        let args = ExecCommand::new("hi")
            .approval_policy(ApprovalPolicy::Never)
            .search()
            .args();
        assert_eq!(
            args,
            vec![
                "exec",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "web_search=\"live\"",
                "hi"
            ]
        );
        assert!(
            !args
                .iter()
                .any(|a| a == "--ask-for-approval" || a == "--search")
        );
    }

    /// `granular` and `on-failure` are accepted by the config key but not by
    /// the flag, which is why `ApprovalPolicyConfig` exists.
    #[test]
    fn exec_approval_accepts_config_only_values() {
        let args = ExecCommand::new("hi")
            .approval_policy(ApprovalPolicyConfig::Granular)
            .args();
        assert_eq!(
            args,
            vec!["exec", "-c", "approval_policy=\"granular\"", "hi"]
        );
    }

    #[test]
    fn exec_search_mode_variants() {
        for (mode, expected) in [
            (WebSearchMode::Disabled, "disabled"),
            (WebSearchMode::Cached, "cached"),
            (WebSearchMode::Indexed, "indexed"),
            (WebSearchMode::Live, "live"),
        ] {
            let args = ExecCommand::new("hi").search_mode(mode).args();
            assert_eq!(args[2], format!("web_search=\"{expected}\""));
        }
    }

    /// `-c` is last-wins, so a raw override has to be emitted after the typed
    /// setters for it to take effect.
    #[test]
    fn exec_raw_config_is_emitted_after_typed_config() {
        let args = ExecCommand::new("hi")
            .approval_policy(ApprovalPolicy::Never)
            .config("approval_policy=\"untrusted\"")
            .args();
        let typed = args
            .iter()
            .position(|a| a == "approval_policy=\"never\"")
            .unwrap();
        let raw = args
            .iter()
            .position(|a| a == "approval_policy=\"untrusted\"")
            .unwrap();
        assert!(typed < raw, "raw override must win: {args:?}");
    }

    /// #55: `--full-auto` is hidden and deprecated on the exec family.
    #[test]
    fn exec_full_auto_emits_sandbox_workspace_write() {
        let args = ExecCommand::new("hi").full_auto().args();
        assert_eq!(args, vec!["exec", "--sandbox", "workspace-write", "hi"]);
        assert!(!args.iter().any(|a| a == "--full-auto"));
    }

    #[test]
    fn exec_explicit_sandbox_wins_over_full_auto() {
        let args = ExecCommand::new("hi")
            .full_auto()
            .sandbox(SandboxMode::ReadOnly)
            .args();
        assert_eq!(args, vec!["exec", "--sandbox", "read-only", "hi"]);
    }

    /// `codex exec resume` has no `--sandbox` flag, so the replacement goes
    /// through the config key instead.
    #[test]
    fn exec_resume_full_auto_emits_sandbox_config_key() {
        let args = ExecResumeCommand::new().last().full_auto().args();
        assert_eq!(
            args,
            vec![
                "exec",
                "resume",
                "-c",
                "sandbox_mode=\"workspace-write\"",
                "--last"
            ]
        );
        assert!(!args.iter().any(|a| a == "--full-auto"));
    }

    #[test]
    fn exec_resume_approval_and_search_emit_config_keys() {
        let args = ExecResumeCommand::new()
            .last()
            .approval_policy(ApprovalPolicyConfig::OnFailure)
            .search_mode(WebSearchMode::Cached)
            .args();
        assert_eq!(
            args,
            vec![
                "exec",
                "resume",
                "-c",
                "approval_policy=\"on-failure\"",
                "-c",
                "web_search=\"cached\"",
                "--last"
            ]
        );
    }

    /// #65: these three were listed in #41 P1 but never landed on
    /// `ExecResumeCommand`.
    #[test]
    fn exec_resume_ignore_and_output_schema_args() {
        let args = ExecResumeCommand::new()
            .last()
            .ignore_user_config()
            .ignore_rules()
            .output_schema("/tmp/schema.json")
            .args();
        assert_eq!(
            args,
            vec![
                "exec",
                "resume",
                "--last",
                "--ignore-user-config",
                "--ignore-rules",
                "--output-schema",
                "/tmp/schema.json"
            ]
        );
    }

    /// #81: the builder emitted `codex exec -` while every spawn path closed
    /// the child's stdin, so the prompt was never delivered. This drives a
    /// fake codex that echoes back what it read, which is the only way to see
    /// the difference: the argv is identical either way.
    #[cfg(all(unix, feature = "json"))]
    #[tokio::test]
    async fn stdin_prompt_reaches_the_child() {
        let codex = echoing_stdin_codex();
        let prompt = "a prompt too awkward for argv\nwith a second line";

        let result = ExecCommand::from_stdin(prompt)
            .execute_json(&codex)
            .await
            .unwrap();

        assert_eq!(result.result, prompt);
    }

    /// The same delivery, on the streaming path, which pipes stdin separately.
    #[cfg(all(unix, feature = "json"))]
    #[tokio::test]
    async fn stdin_prompt_reaches_the_child_when_streaming() {
        let codex = echoing_stdin_codex();
        let prompt = "streamed stdin prompt";
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&seen);

        ExecCommand::from_stdin(prompt)
            .stream(&codex, move |event| {
                if let Some(text) = event.agent_message_text() {
                    sink.lock().unwrap().push(text);
                }
            })
            .await
            .unwrap();

        assert_eq!(seen.lock().unwrap().as_slice(), &[prompt.to_string()]);
    }

    /// A prompt larger than a pipe buffer must not deadlock: the write and the
    /// output drain have to run concurrently.
    #[cfg(all(unix, feature = "json"))]
    #[tokio::test]
    async fn a_prompt_larger_than_the_pipe_buffer_still_completes() {
        let codex = echoing_stdin_codex();
        // Well past the usual 64 KiB pipe capacity.
        let prompt = "x".repeat(512 * 1024);

        let result = ExecCommand::from_stdin(&prompt)
            .execute_json(&codex)
            .await
            .unwrap();

        assert_eq!(result.result.len(), prompt.len());
    }

    #[cfg(all(unix, feature = "json"))]
    fn echoing_stdin_codex() -> Codex {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-echo-stdin.sh");
        Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .build()
            .expect("bash must exist")
    }

    #[test]
    fn from_stdin_emits_the_dash_positional_not_the_prompt() {
        let args = ExecCommand::from_stdin("secret prompt").ephemeral().args();
        assert_eq!(args, vec!["exec", "--ephemeral", "-"]);
        // The prompt must not leak into argv, which is the whole point of
        // sending it on stdin.
        assert!(!args.iter().any(|a| a.contains("secret")));
    }

    #[test]
    fn prompt_via_stdin_converts_an_existing_prompt() {
        let args = ExecCommand::new("hello").prompt_via_stdin().args();
        assert_eq!(args, vec!["exec", "-"]);
    }
}
