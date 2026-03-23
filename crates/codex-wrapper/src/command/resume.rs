/// Resume a previous interactive session.
///
/// Wraps the top-level `codex resume` command (distinct from `codex exec resume`).
use crate::Codex;
use crate::command::CodexCommand;
use crate::error::Result;
use crate::exec::{self, CommandOutput};
use crate::types::{ApprovalPolicy, SandboxMode};

/// Resume a previous interactive Codex session.
#[derive(Debug, Clone)]
pub struct ResumeCommand {
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
    cd: Option<String>,
    search: bool,
    add_dirs: Vec<String>,
}

impl ResumeCommand {
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
            oss: false,
            local_provider: None,
            profile: None,
            sandbox: None,
            approval_policy: None,
            full_auto: false,
            dangerously_bypass_approvals_and_sandbox: false,
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
        if let Some(sandbox) = self.sandbox {
            args.push("--sandbox".into());
            args.push(sandbox.as_arg().into());
        }
        if let Some(policy) = self.approval_policy {
            args.push("--ask-for-approval".into());
            args.push(policy.as_arg().into());
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
}
