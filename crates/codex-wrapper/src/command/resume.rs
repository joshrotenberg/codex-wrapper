/// Resume a previous interactive session.
///
/// Wraps the top-level `codex resume` command (distinct from `codex exec resume`).
use crate::Codex;
use crate::command::CodexCommand;
use crate::command::exec::effective_sandbox;
use crate::error::Result;
use crate::exec::{self, CommandOutput};
use crate::types::{ApprovalPolicy, SandboxMode};

/// Resume a previous interactive Codex session.
#[derive(Debug, Clone)]
pub struct ResumeCommand {
    approve_for_me: bool,
    session_id: Option<String>,
    prompt: Option<String>,
    last: bool,
    all: bool,
    config_overrides: Vec<String>,
    enabled_features: Vec<String>,
    disabled_features: Vec<String>,
    images: Vec<String>,
    model: Option<String>,
    oss: bool,
    local_provider: Option<String>,
    profile: Option<String>,
    sandbox: Option<SandboxMode>,
    approval_policy: Option<ApprovalPolicy>,
    full_auto: bool,
    dangerously_bypass_approvals_and_sandbox: bool,
    dangerously_bypass_hook_trust: bool,
    strict_config: bool,
    no_alt_screen: bool,
    include_non_interactive: bool,
    remote: Option<String>,
    remote_auth_token_env: Option<String>,
    cd: Option<String>,
    search: bool,
    add_dirs: Vec<String>,
}

impl ResumeCommand {
    #[must_use]
    pub fn new() -> Self {
        Self {
            approve_for_me: false,
            session_id: None,
            prompt: None,
            last: false,
            all: false,
            config_overrides: Vec::new(),
            enabled_features: Vec::new(),
            disabled_features: Vec::new(),
            images: Vec::new(),
            model: None,
            oss: false,
            local_provider: None,
            profile: None,
            sandbox: None,
            approval_policy: None,
            full_auto: false,
            dangerously_bypass_approvals_and_sandbox: false,
            dangerously_bypass_hook_trust: false,
            strict_config: false,
            no_alt_screen: false,
            include_non_interactive: false,
            remote: None,
            remote_auth_token_env: None,
            cd: None,
            search: false,
            add_dirs: Vec::new(),
        }
    }

    /// Session ID (UUID) or thread name to resume.
    #[must_use]
    pub fn session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// Optional prompt to start the resumed session with.
    #[must_use]
    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Continue the most recent session without showing the picker.
    #[must_use]
    pub fn last(mut self) -> Self {
        self.last = true;
        self
    }

    /// Show all sessions (disables cwd filtering).
    #[must_use]
    pub fn all(mut self) -> Self {
        self.all = true;
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
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
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

    /// Run in full-auto mode, emitted as `--sandbox workspace-write`.
    ///
    /// `codex resume` rejects `--full-auto` outright in `codex-cli` 0.145.0
    /// ("unexpected argument"), so this emits the replacement the CLI names
    /// for the exec family. An explicit [`sandbox`](Self::sandbox) call is
    /// more specific and wins over it.
    #[must_use]
    pub fn full_auto(mut self) -> Self {
        self.full_auto = true;
        self
    }

    /// Route approval requests through automatic review, using the
    /// workspace-write sandbox (`--approve-for-me`).
    ///
    /// Added in `codex-cli` 0.147.0. Older releases reject it as an unexpected
    /// argument, so this is the one builder method with a floor above the
    /// wrapper's tested minimum. `codex exec review` and `codex exec resume`
    /// do not accept it.
    #[must_use]
    pub fn approve_for_me(mut self) -> Self {
        self.approve_for_me = true;
        self
    }

    #[must_use]
    pub(crate) fn set_bypass_approvals_and_sandbox(mut self) -> Self {
        self.dangerously_bypass_approvals_and_sandbox = true;
        self
    }

    /// Bypass the hook trust prompt (`--dangerously-bypass-hook-trust`).
    #[must_use]
    pub(crate) fn set_bypass_hook_trust(mut self) -> Self {
        self.dangerously_bypass_hook_trust = true;
        self
    }

    /// Error on unrecognized config keys (`--strict-config`).
    #[must_use]
    pub fn strict_config(mut self) -> Self {
        self.strict_config = true;
        self
    }

    /// Do not use the terminal alternate screen (`--no-alt-screen`).
    #[must_use]
    pub fn no_alt_screen(mut self) -> Self {
        self.no_alt_screen = true;
        self
    }

    /// Include non-interactive sessions in the picker
    /// (`--include-non-interactive`).
    #[must_use]
    pub fn include_non_interactive(mut self) -> Self {
        self.include_non_interactive = true;
        self
    }

    /// Connect the TUI to a remote app server endpoint (`--remote <ADDR>`).
    #[must_use]
    pub fn remote(mut self, addr: impl Into<String>) -> Self {
        self.remote = Some(addr.into());
        self
    }

    /// Env var holding the bearer token for the remote app server
    /// (`--remote-auth-token-env <ENV_VAR>`).
    #[must_use]
    pub fn remote_auth_token_env(mut self, env_var: impl Into<String>) -> Self {
        self.remote_auth_token_env = Some(env_var.into());
        self
    }

    #[must_use]
    pub fn cd(mut self, dir: impl Into<String>) -> Self {
        self.cd = Some(dir.into());
        self
    }

    /// Enable live web search.
    #[must_use]
    pub fn search(mut self) -> Self {
        self.search = true;
        self
    }

    #[must_use]
    pub fn add_dir(mut self, dir: impl Into<String>) -> Self {
        self.add_dirs.push(dir.into());
        self
    }
}

impl Default for ResumeCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexCommand for ResumeCommand {
    type Output = CommandOutput;

    fn args(&self) -> Vec<String> {
        let mut args = vec!["resume".into()];

        for v in &self.config_overrides {
            args.push("-c".into());
            args.push(v.clone());
        }
        for v in &self.enabled_features {
            args.push("--enable".into());
            args.push(v.clone());
        }
        for v in &self.disabled_features {
            args.push("--disable".into());
            args.push(v.clone());
        }
        if self.last {
            args.push("--last".into());
        }
        if self.all {
            args.push("--all".into());
        }
        for v in &self.images {
            args.push("--image".into());
            args.push(v.clone());
        }
        if let Some(model) = &self.model {
            args.push("--model".into());
            args.push(model.clone());
        }
        if self.oss {
            args.push("--oss".into());
        }
        if let Some(provider) = &self.local_provider {
            args.push("--local-provider".into());
            args.push(provider.clone());
        }
        if let Some(profile) = &self.profile {
            args.push("--profile".into());
            args.push(profile.clone());
        }
        if let Some(sandbox) = effective_sandbox(self.sandbox, self.full_auto) {
            args.push("--sandbox".into());
            args.push(sandbox.as_arg().into());
        }
        if let Some(policy) = self.approval_policy {
            args.push("--ask-for-approval".into());
            args.push(policy.as_arg().into());
        }
        if self.approve_for_me {
            args.push("--approve-for-me".into());
        }
        if self.dangerously_bypass_approvals_and_sandbox {
            args.push("--dangerously-bypass-approvals-and-sandbox".into());
        }
        if self.dangerously_bypass_hook_trust {
            args.push("--dangerously-bypass-hook-trust".into());
        }
        if self.strict_config {
            args.push("--strict-config".into());
        }
        if self.no_alt_screen {
            args.push("--no-alt-screen".into());
        }
        if self.include_non_interactive {
            args.push("--include-non-interactive".into());
        }
        if let Some(remote) = &self.remote {
            args.push("--remote".into());
            args.push(remote.clone());
        }
        if let Some(env_var) = &self.remote_auth_token_env {
            args.push("--remote-auth-token-env".into());
            args.push(env_var.clone());
        }
        if let Some(cd) = &self.cd {
            args.push("--cd".into());
            args.push(cd.clone());
        }
        if self.search {
            args.push("--search".into());
        }
        for v in &self.add_dirs {
            args.push("--add-dir".into());
            args.push(v.clone());
        }
        if let Some(id) = &self.session_id {
            args.push(id.clone());
        }
        if let Some(prompt) = &self.prompt {
            args.push(prompt.clone());
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
    fn resume_last_args() {
        let args = ResumeCommand::new()
            .last()
            .model("gpt-5")
            .prompt("continue")
            .args();
        assert_eq!(
            args,
            vec!["resume", "--last", "--model", "gpt-5", "continue"]
        );
    }

    #[test]
    fn resume_session_id_args() {
        let args = ResumeCommand::new()
            .session_id("abc-123")
            .sandbox(SandboxMode::WorkspaceWrite)
            .search()
            .args();
        assert_eq!(
            args,
            vec![
                "resume",
                "--sandbox",
                "workspace-write",
                "--search",
                "abc-123"
            ]
        );
    }

    #[test]
    fn resume_new_flags_args() {
        let args = ResumeCommand::new()
            .last()
            .strict_config()
            .include_non_interactive()
            .no_alt_screen()
            .remote("ws://host:9000")
            .args();
        assert_eq!(
            args,
            vec![
                "resume",
                "--last",
                "--strict-config",
                "--no-alt-screen",
                "--include-non-interactive",
                "--remote",
                "ws://host:9000",
            ]
        );
    }

    /// `codex resume` rejects `--full-auto` outright. See #55.
    #[test]
    fn resume_full_auto_emits_sandbox_workspace_write() {
        let args = ResumeCommand::new().last().full_auto().args();
        assert_eq!(
            args,
            vec!["resume", "--last", "--sandbox", "workspace-write"]
        );
        assert!(!args.iter().any(|a| a == "--full-auto"));
    }

    #[test]
    fn resume_explicit_sandbox_wins_over_full_auto() {
        let args = ResumeCommand::new()
            .last()
            .full_auto()
            .sandbox(SandboxMode::DangerFullAccess)
            .args();
        assert_eq!(
            args,
            vec!["resume", "--last", "--sandbox", "danger-full-access"]
        );
    }
}
