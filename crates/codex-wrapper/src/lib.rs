//! A type-safe Codex CLI wrapper for Rust.
//!
//! `codex-wrapper` provides a builder-pattern interface for invoking the
//! `codex` CLI programmatically. It follows the same design philosophy as
//! [`claude-wrapper`](https://crates.io/crates/claude-wrapper) and
//! [`docker-wrapper`](https://crates.io/crates/docker-wrapper):
//! each CLI subcommand is a builder struct that produces typed output.
//!
//! # Quick Start
//!
//! ```no_run
//! use codex_wrapper::{Codex, CodexCommand, ExecCommand, SandboxMode};
//!
//! # async fn example() -> codex_wrapper::Result<()> {
//! let codex = Codex::builder().build()?;
//!
//! let output = ExecCommand::new("summarize this repository")
//!     .sandbox(SandboxMode::WorkspaceWrite)
//!     .ephemeral()
//!     .execute(&codex)
//!     .await?;
//!
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
//!
//! # Two-Layer Builder
//!
//! The [`Codex`] client holds shared config (binary path, env vars, timeout,
//! retry policy). Command builders hold per-invocation options and call
//! `execute(&codex)`.
//!
//! ```no_run
//! use codex_wrapper::{Codex, CodexCommand, ExecCommand, ApprovalPolicy, RetryPolicy};
//! use std::time::Duration;
//!
//! # async fn example() -> codex_wrapper::Result<()> {
//! // Configure once, reuse across commands
//! let codex = Codex::builder()
//!     .env("OPENAI_API_KEY", "sk-...")
//!     .timeout_secs(300)
//!     .retry(RetryPolicy::new().max_attempts(3).exponential())
//!     .build()?;
//!
//! // Each command is a separate builder
//! let output = ExecCommand::new("fix the failing tests")
//!     .model("o3")
//!     .approval_policy(ApprovalPolicy::Never)
//!     .skip_git_repo_check()
//!     .ephemeral()
//!     .execute(&codex)
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! # JSONL Output Parsing
//!
//! Use `execute_json_lines()` to get structured events from `--json` mode:
//!
//! ```no_run
//! use codex_wrapper::{Codex, ExecCommand};
//!
//! # async fn example() -> codex_wrapper::Result<()> {
//! let codex = Codex::builder().build()?;
//! let events = ExecCommand::new("what is 2+2?")
//!     .ephemeral()
//!     .execute_json_lines(&codex)
//!     .await?;
//!
//! for event in &events {
//!     println!("{}: {:?}", event.event_type, event.extra);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Available Commands
//!
//! | Command | CLI equivalent |
//! |---------|---------------|
//! | [`ExecCommand`] | `codex exec <prompt>` |
//! | [`ExecResumeCommand`] | `codex exec resume` |
//! | [`ReviewCommand`] | `codex exec review` |
//! | [`ResumeCommand`] | `codex resume` |
//! | [`ForkCommand`] | `codex fork` |
//! | [`LoginCommand`] | `codex login` |
//! | [`LoginStatusCommand`] | `codex login status` |
//! | [`LogoutCommand`] | `codex logout` |
//! | [`McpListCommand`] | `codex mcp list` |
//! | [`McpGetCommand`] | `codex mcp get` |
//! | [`McpAddCommand`] | `codex mcp add` |
//! | [`McpRemoveCommand`] | `codex mcp remove` |
//! | [`McpLoginCommand`] | `codex mcp login` |
//! | [`McpLogoutCommand`] | `codex mcp logout` |
//! | [`McpServerCommand`] | `codex mcp-server` |
//! | [`CompletionCommand`] | `codex completion` |
//! | [`SandboxCommand`] | `codex sandbox` |
//! | [`ApplyCommand`] | `codex apply` |
//! | [`FeaturesListCommand`] | `codex features list` |
//! | [`FeaturesEnableCommand`] | `codex features enable` |
//! | [`FeaturesDisableCommand`] | `codex features disable` |
//! | [`VersionCommand`] | `codex --version` |
//! | [`RawCommand`] | Escape hatch for arbitrary args |
//!
//! # Error Handling
//!
//! All commands return [`Result<T>`], with typed errors via [`thiserror`]:
//!
//! ```no_run
//! use codex_wrapper::{Codex, CodexCommand, ExecCommand, Error};
//!
//! # async fn example() -> codex_wrapper::Result<()> {
//! let codex = Codex::builder().build()?;
//! match ExecCommand::new("test").execute(&codex).await {
//!     Ok(output) => println!("{}", output.stdout),
//!     Err(Error::CommandFailed { stderr, exit_code, .. }) => {
//!         eprintln!("failed (exit {}): {}", exit_code, stderr);
//!     }
//!     Err(Error::Timeout { .. }) => eprintln!("timed out"),
//!     Err(e) => eprintln!("{e}"),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! - `json` *(enabled by default)* - JSONL output parsing via `serde_json`

pub mod command;
pub mod error;
pub mod exec;
pub mod retry;
#[cfg(feature = "json")]
pub mod session;
#[cfg(feature = "json")]
pub mod streaming;
pub mod types;
pub mod version;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use command::CodexCommand;
pub use command::apply::ApplyCommand;
pub use command::completion::{CompletionCommand, Shell};
pub use command::exec::{ExecCommand, ExecResumeCommand};
pub use command::features::{FeaturesDisableCommand, FeaturesEnableCommand, FeaturesListCommand};
pub use command::fork::ForkCommand;
pub use command::login::{LoginCommand, LoginStatusCommand, LogoutCommand};
pub use command::mcp::{
    McpAddCommand, McpGetCommand, McpListCommand, McpLoginCommand, McpLogoutCommand,
    McpRemoveCommand,
};
pub use command::mcp_server::McpServerCommand;
pub use command::raw::RawCommand;
pub use command::resume::ResumeCommand;
pub use command::review::ReviewCommand;
pub use command::sandbox::{SandboxCommand, SandboxPlatform};
pub use command::version::VersionCommand;
pub use error::{Error, Result};
pub use exec::CommandOutput;
pub use retry::{BackoffStrategy, RetryPolicy};
#[cfg(feature = "json")]
pub use session::{Session, TurnRecord};
pub use types::*;
pub use version::{CliVersion, VersionParseError};

/// Shared Codex CLI client configuration.
///
/// Holds the binary path, working directory, environment variables, global
/// arguments, timeout, and retry policy. Cheap to [`Clone`]; intended to be
/// created once and reused across many command invocations.
///
/// # Example
///
/// ```no_run
/// # fn example() -> codex_wrapper::Result<()> {
/// let codex = codex_wrapper::Codex::builder()
///     .env("OPENAI_API_KEY", "sk-...")
///     .timeout_secs(120)
///     .build()?;
/// # Ok(())
/// # }
/// ```
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
    /// Create a new [`CodexBuilder`].
    #[must_use]
    pub fn builder() -> CodexBuilder {
        CodexBuilder::default()
    }

    /// Path to the resolved `codex` binary.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Working directory for command execution, if set.
    #[must_use]
    pub fn working_dir(&self) -> Option<&Path> {
        self.working_dir.as_deref()
    }

    /// Return a clone of this client with a different working directory.
    #[must_use]
    pub fn with_working_dir(&self, dir: impl Into<PathBuf>) -> Self {
        let mut clone = self.clone();
        clone.working_dir = Some(dir.into());
        clone
    }

    /// Query the installed Codex CLI version.
    pub async fn cli_version(&self) -> Result<CliVersion> {
        let output = VersionCommand::new().execute(self).await?;
        CliVersion::parse_version_output(&output.stdout).map_err(|e| Error::Io {
            message: format!("failed to parse CLI version: {e}"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
            working_dir: None,
        })
    }

    /// Verify the installed CLI meets a minimum version requirement.
    ///
    /// Returns [`Error::VersionMismatch`] if the installed version is too old.
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

/// Builder for creating a [`Codex`] client.
///
/// All options are optional. By default the builder discovers the `codex`
/// binary via `PATH`.
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
    /// Set an explicit path to the `codex` binary (skips `PATH` lookup).
    #[must_use]
    pub fn binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary = Some(path.into());
        self
    }

    /// Set the working directory for all commands.
    #[must_use]
    pub fn working_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(path.into());
        self
    }

    /// Set a single environment variable for child processes.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set multiple environment variables for child processes.
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

    /// Set the command timeout in seconds.
    #[must_use]
    pub fn timeout_secs(mut self, seconds: u64) -> Self {
        self.timeout = Some(Duration::from_secs(seconds));
        self
    }

    /// Set the command timeout as a [`Duration`].
    #[must_use]
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout = Some(duration);
        self
    }

    /// Append a raw global argument passed before any subcommand.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.global_args.push(arg.into());
        self
    }

    /// Add a global config override (`-c key=value`).
    #[must_use]
    pub fn config(mut self, key_value: impl Into<String>) -> Self {
        self.global_args.push("-c".into());
        self.global_args.push(key_value.into());
        self
    }

    /// Enable a feature flag globally (`--enable <name>`).
    #[must_use]
    pub fn enable(mut self, feature: impl Into<String>) -> Self {
        self.global_args.push("--enable".into());
        self.global_args.push(feature.into());
        self
    }

    /// Disable a feature flag globally (`--disable <name>`).
    #[must_use]
    pub fn disable(mut self, feature: impl Into<String>) -> Self {
        self.global_args.push("--disable".into());
        self.global_args.push(feature.into());
        self
    }

    /// Set a default [`RetryPolicy`] for all commands.
    #[must_use]
    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Build the [`Codex`] client.
    ///
    /// Returns [`Error::NotFound`] if no binary path was set and `codex` is
    /// not found in `PATH`.
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
