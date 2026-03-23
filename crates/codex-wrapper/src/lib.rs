//! A type-safe Codex CLI wrapper for Rust.
//!
//! `codex-wrapper` mirrors the builder-oriented shape of `claude-wrapper`, but
//! targets the current Codex CLI surface.
//!
//! # Quick Start
//!
//! ```no_run
//! use codex_wrapper::{Codex, ExecCommand, SandboxMode, ApprovalPolicy};
//!
//! # async fn example() -> codex_wrapper::Result<()> {
//! let codex = Codex::builder().build()?;
//!
//! // SandboxMode defaults to WorkspaceWrite; ApprovalPolicy defaults to OnRequest.
//! // Both can be overridden per-command:
//! let cmd = ExecCommand::new("fix the failing tests")
//!     .sandbox_mode(SandboxMode::ReadOnly)
//!     .approval_policy(ApprovalPolicy::Never);
//!
//! let output = cmd.execute(&codex).await?;
//! println!("{}", output.stdout);
//! # Ok(())
//! # }
//! ```
//!
//! ## Defaults
//!
//! | Type | Default variant |
//! |------|-----------------|
//! | [`SandboxMode`] | [`SandboxMode::WorkspaceWrite`] |
//! | [`ApprovalPolicy`] | [`ApprovalPolicy::OnRequest`] |

pub mod command;
pub mod error;
pub mod exec;
pub mod retry;
pub mod types;
pub mod version;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use command::CodexCommand;
pub use command::exec::{ExecCommand, ExecResumeCommand};
pub use command::login::{LoginCommand, LoginStatusCommand, LogoutCommand};
pub use command::mcp::{
    McpAddCommand, McpGetCommand, McpListCommand, McpLoginCommand, McpLogoutCommand,
    McpRemoveCommand,
};
pub use command::raw::RawCommand;
pub use command::review::ReviewCommand;
pub use command::version::VersionCommand;
pub use error::{Error, Result};
pub use exec::CommandOutput;
pub use retry::{BackoffStrategy, RetryPolicy};
pub use types::*;
pub use version::{CliVersion, VersionParseError};

#[derive(Debug, Clone)]
pub struct Codex {
    pub(crate) binary: PathBuf,
    pub(crate) working_dir: Option<PathBuf>,
    pub(crate) env: HashMap<String, String>,
    pub(crate) global_args: Vec<String>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) retry_policy: Option<RetryPolicy>,
}

impl Codex {
    #[must_use]
    pub fn builder() -> CodexBuilder {
        CodexBuilder::default()
    }

    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    #[must_use]
    pub fn working_dir(&self) -> Option<&Path> {
        self.working_dir.as_deref()
    }

    #[must_use]
    pub fn with_working_dir(&self, dir: impl Into<PathBuf>) -> Self {
        let mut clone = self.clone();
        clone.working_dir = Some(dir.into());
        clone
    }

    pub async fn cli_version(&self) -> Result<CliVersion> {
        let output = VersionCommand::new().execute(self).await?;
        CliVersion::parse_version_output(&output.stdout).map_err(|e| Error::Io {
            message: format!("failed to parse CLI version: {e}"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
            working_dir: None,
        })
    }

    pub async fn check_version(&self, minimum: &CliVersion) -> Result<CliVersion> {
        let version = self.cli_version().await?;
        if version.satisfies_minimum(minimum) {
            Ok(version)
        } else {
            Err(Error::VersionMismatch {
                found: version,
                minimum: *minimum,
            })
        }
    }
}

#[derive(Debug, Default)]
pub struct CodexBuilder {
    binary: Option<PathBuf>,
    working_dir: Option<PathBuf>,
    env: HashMap<String, String>,
    global_args: Vec<String>,
    timeout: Option<Duration>,
    retry_policy: Option<RetryPolicy>,
}

impl CodexBuilder {
    #[must_use]
    pub fn binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary = Some(path.into());
        self
    }

    #[must_use]
    pub fn working_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(path.into());
        self
    }

    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    #[must_use]
    pub fn envs(
        mut self,
        vars: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (key, value) in vars {
            self.env.insert(key.into(), value.into());
        }
        self
    }

    #[must_use]
    pub fn timeout_secs(mut self, seconds: u64) -> Self {
        self.timeout = Some(Duration::from_secs(seconds));
        self
    }

    #[must_use]
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.global_args.push(arg.into());
        self
    }

    #[must_use]
    pub fn config(mut self, key_value: impl Into<String>) -> Self {
        self.global_args.push("-c".into());
        self.global_args.push(key_value.into());
        self
    }

    #[must_use]
    pub fn enable(mut self, feature: impl Into<String>) -> Self {
        self.global_args.push("--enable".into());
        self.global_args.push(feature.into());
        self
    }

    #[must_use]
    pub fn disable(mut self, feature: impl Into<String>) -> Self {
        self.global_args.push("--disable".into());
        self.global_args.push(feature.into());
        self
    }

    #[must_use]
    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    pub fn build(self) -> Result<Codex> {
        let binary = match self.binary {
            Some(path) => path,
            None => which::which("codex").map_err(|_| Error::NotFound)?,
        };

        Ok(Codex {
            binary,
            working_dir: self.working_dir,
            env: self.env,
            global_args: self.global_args,
            timeout: self.timeout,
            retry_policy: self.retry_policy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_with_binary() {
        let codex = Codex::builder()
            .binary("/usr/local/bin/codex")
            .env("FOO", "bar")
            .timeout_secs(60)
            .build()
            .unwrap();

        assert_eq!(codex.binary, PathBuf::from("/usr/local/bin/codex"));
        assert_eq!(codex.env.get("FOO").unwrap(), "bar");
        assert_eq!(codex.timeout, Some(Duration::from_secs(60)));
    }

    #[test]
    fn builder_global_args() {
        let codex = Codex::builder()
            .binary("/usr/local/bin/codex")
            .config("model=\"gpt-5\"")
            .enable("foo")
            .disable("bar")
            .build()
            .unwrap();

        assert_eq!(
            codex.global_args,
            vec![
                "-c",
                "model=\"gpt-5\"",
                "--enable",
                "foo",
                "--disable",
                "bar"
            ]
        );
    }
}
