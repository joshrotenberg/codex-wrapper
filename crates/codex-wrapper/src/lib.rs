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
//! use codex_wrapper::{Codex, CodexCommand, ExecCommand, RetryPolicy};
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
//!     .dangerously_bypass_approvals_and_sandbox()
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
//! | [`ArchiveCommand`] | `codex archive` |
//! | [`DeleteCommand`] | `codex delete` |
//! | [`UnarchiveCommand`] | `codex unarchive` |
//! | [`DoctorCommand`] | `codex doctor` |
//! | [`UpdateCommand`] | `codex update` |
//! | [`PluginAddCommand`] | `codex plugin add` |
//! | [`PluginListCommand`] | `codex plugin list` |
//! | [`PluginRemoveCommand`] | `codex plugin remove` |
//! | [`PluginMarketplaceAddCommand`] | `codex plugin marketplace add` |
//! | [`PluginMarketplaceListCommand`] | `codex plugin marketplace list` |
//! | [`PluginMarketplaceUpgradeCommand`] | `codex plugin marketplace upgrade` |
//! | [`PluginMarketplaceRemoveCommand`] | `codex plugin marketplace remove` |
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
//! # Cancellation
//!
//! Dropping the future returned by a command kills the spawned `codex`
//! process. That covers a timeout, an aborted task, and a caller that stops
//! awaiting during a graceful shutdown: cancelling the future cancels the
//! work, rather than leaving codex running and billing with no handle left to
//! stop it.
//!
//! Two limits are worth knowing:
//!
//! - The kill reaps the `codex` process itself. Subprocesses codex spawned for
//!   tool use are not signalled and can outlive it.
//! - Reaping needs the tokio runtime to still be running. A future dropped as
//!   part of runtime shutdown may not get far enough to kill the child.
//!
//! # Features
//!
//! - `json` *(enabled by default)* - JSONL output parsing via `serde_json`

#[cfg(feature = "json")]
pub mod auth;
#[cfg(feature = "json")]
pub mod budget;
pub mod command;
pub mod error;
pub mod exec;
pub mod retry;
#[cfg(feature = "json")]
pub mod session;
#[cfg(feature = "json")]
pub mod streaming;
#[cfg(all(test, unix))]
mod test_support;
pub mod types;
pub mod version;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(feature = "json")]
pub use auth::{AuthStatus, AuthStrategy};
#[cfg(feature = "json")]
pub use budget::{TokenBudget, TokenBudgetBuilder};
pub use command::CodexCommand;
pub use command::apply::ApplyCommand;
pub use command::completion::{CompletionCommand, Shell};
pub use command::doctor::DoctorCommand;
pub use command::exec::{ExecCommand, ExecResumeCommand};
pub use command::features::{FeaturesDisableCommand, FeaturesEnableCommand, FeaturesListCommand};
pub use command::fork::ForkCommand;
pub use command::login::{LoginCommand, LoginStatusCommand, LogoutCommand};
pub use command::mcp::{
    McpAddCommand, McpGetCommand, McpListCommand, McpLoginCommand, McpLogoutCommand,
    McpRemoveCommand,
};
pub use command::mcp_server::McpServerCommand;
pub use command::plugin::{
    PluginAddCommand, PluginListCommand, PluginMarketplaceAddCommand, PluginMarketplaceListCommand,
    PluginMarketplaceRemoveCommand, PluginMarketplaceUpgradeCommand, PluginRemoveCommand,
};
pub use command::raw::RawCommand;
pub use command::resume::ResumeCommand;
pub use command::review::ReviewCommand;
pub use command::sandbox::SandboxCommand;
pub use command::session_mgmt::{ArchiveCommand, DeleteCommand, UnarchiveCommand};
pub use command::update::UpdateCommand;
pub use command::version::VersionCommand;
pub use error::{Error, FailureKind, Result};
pub use exec::CommandOutput;
pub use retry::{BackoffStrategy, RetryPolicy};
#[cfg(feature = "json")]
pub use session::{Session, TurnRecord};
pub use types::*;
pub use version::{
    CliVersion, CliVersionStatus, TESTED_CLI_VERSION_MAX, TESTED_CLI_VERSION_MIN, VersionParseError,
};

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
    pub(crate) tested_cli_version_range: (CliVersion, CliVersion),
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
    /// Which credential this client's CLI would use, without spawning it.
    ///
    /// Honors a `CODEX_HOME` set on this client via
    /// [`env`](CodexBuilder::env), falling back to the process environment.
    /// See [`crate::auth`] for what the strategies mean and how they were
    /// determined.
    ///
    /// ```no_run
    /// use codex_wrapper::Codex;
    ///
    /// # fn example() -> codex_wrapper::Result<()> {
    /// let codex = Codex::builder().build()?;
    /// if !codex.auth_status().is_configured() {
    ///     eprintln!("no credentials; run `codex login`");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "json")]
    #[must_use]
    pub fn auth_status(&self) -> crate::auth::AuthStatus {
        crate::auth::detect_with(|key| {
            self.env
                .get(key)
                .cloned()
                .or_else(|| std::env::var(key).ok())
        })
    }

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

    /// The tested-against CLI version range this client reports on.
    ///
    /// Defaults to [`TESTED_CLI_VERSION_MIN`] and [`TESTED_CLI_VERSION_MAX`];
    /// override with [`CodexBuilder::tested_cli_version_range`].
    #[must_use]
    pub fn tested_cli_version_range(&self) -> (CliVersion, CliVersion) {
        self.tested_cli_version_range
    }

    /// Classify the installed CLI against the tested-against range.
    ///
    /// Emits a `tracing::warn!` when outside the range, and returns the typed
    /// status either way. This reports; it does not fail. Most CLI releases
    /// break nothing, so refusing to run against an unrecognized version is
    /// worse than saying so. Use
    /// [`ensure_tested_cli_version`](Self::ensure_tested_cli_version) when you
    /// do want a hard gate.
    ///
    /// Intended for one-shot use at startup rather than before every command:
    /// it spawns `codex --version`.
    pub async fn cli_version_status(&self) -> Result<CliVersionStatus> {
        let (min, max) = self.tested_cli_version_range;
        let status = self.cli_version().await?.status_within(&min, &max);
        warn_on_drift(&status);
        Ok(status)
    }

    /// Like [`cli_version_status`](Self::cli_version_status), but returns
    /// [`Error::UntestedCliVersion`] when the installed CLI is outside the
    /// tested range.
    ///
    /// This is the opt-in hard gate. It is a method rather than a
    /// [`CodexBuilder`] option because [`CodexBuilder::build`] is synchronous
    /// and never spawns the binary; enforcing a version there would mean
    /// running a subprocess inside a constructor.
    pub async fn ensure_tested_cli_version(&self) -> Result<CliVersion> {
        let (min, max) = self.tested_cli_version_range;
        let found = self.cli_version().await?;
        match found.status_within(&min, &max) {
            CliVersionStatus::Tested => Ok(found),
            status => {
                warn_on_drift(&status);
                Err(Error::UntestedCliVersion {
                    found,
                    tested_min: min,
                    tested_max: max,
                })
            }
        }
    }
}

fn warn_on_drift(status: &CliVersionStatus) {
    match status {
        CliVersionStatus::Tested => {}
        CliVersionStatus::NewerUntested { found, tested_max } => {
            tracing::warn!(
                found = %found,
                tested_max = %tested_max,
                "codex CLI is newer than this wrapper's tested-against range; \
                 semantics may have drifted"
            );
        }
        CliVersionStatus::OlderThanMinimum { found, minimum } => {
            tracing::warn!(
                found = %found,
                minimum = %minimum,
                "codex CLI is older than this wrapper's tested-against range; \
                 some emitted arguments are likely to be rejected"
            );
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
    tested_cli_version_range: Option<(CliVersion, CliVersion)>,
}

impl CodexBuilder {
    /// Override the tested-against CLI version range.
    ///
    /// Defaults to the range this crate declares and verifies in CI. Set this
    /// only when you have validated a different range yourself; widening it
    /// does not make the wrapper work against versions it was not tested on.
    #[must_use]
    pub fn tested_cli_version_range(mut self, min: CliVersion, max: CliVersion) -> Self {
        self.tested_cli_version_range = Some((min, max));
        self
    }

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
            tested_cli_version_range: self.tested_cli_version_range.unwrap_or((
                version::TESTED_CLI_VERSION_MIN,
                version::TESTED_CLI_VERSION_MAX,
            )),
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

    #[test]
    fn client_defaults_to_the_crate_tested_range() {
        let codex = Codex::builder().binary("/bin/echo").build().unwrap();
        assert_eq!(
            codex.tested_cli_version_range(),
            (
                version::TESTED_CLI_VERSION_MIN,
                version::TESTED_CLI_VERSION_MAX
            )
        );
    }

    #[test]
    fn builder_can_override_the_tested_range() {
        let min = CliVersion::new(1, 0, 0);
        let max = CliVersion::new(2, 0, 0);
        let codex = Codex::builder()
            .binary("/bin/echo")
            .tested_cli_version_range(min, max)
            .build()
            .unwrap();
        assert_eq!(codex.tested_cli_version_range(), (min, max));
    }

    #[test]
    fn untested_version_error_names_both_bounds() {
        let err = Error::UntestedCliVersion {
            found: CliVersion::new(0, 200, 0),
            tested_min: CliVersion::new(0, 145, 0),
            tested_max: CliVersion::new(0, 146, 0),
        };
        assert_eq!(
            err.to_string(),
            "CLI version 0.200.0 is outside the tested range 0.145.0..=0.146.0"
        );
    }

    /// A CODEX_HOME set on the client must win over the process environment,
    /// or a client pointed at a different home reports the wrong credentials.
    #[cfg(feature = "json")]
    #[test]
    fn auth_status_honors_a_client_codex_home() {
        let dir =
            std::env::temp_dir().join(format!("codex-wrapper-client-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-secret"}"#,
        )
        .unwrap();

        let codex = Codex::builder()
            .binary("/bin/echo")
            .env("CODEX_HOME", dir.to_str().unwrap())
            .build()
            .unwrap();

        let status = codex.auth_status();
        assert_eq!(status.codex_home, dir);
        assert!(status.is_configured());
        assert!(
            !format!("{status:?}").contains("sk-secret"),
            "the credential leaked into the status"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
