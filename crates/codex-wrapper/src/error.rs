//! Error types for `codex-wrapper`.

use std::path::PathBuf;

/// Errors returned by `codex-wrapper` operations.
///
/// This enum is `#[non_exhaustive]`: match arms must include a `_` catch-all so
/// new variants can be added without a breaking change. This mirrors
/// `claude-wrapper`'s `Error` for cross-crate consistency.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The `codex` binary was not found in PATH.
    #[error("codex binary not found in PATH")]
    NotFound,

    /// The CLI could not authenticate.
    ///
    /// Classified from stderr by [`Error::from_command_failure`]; see
    /// [`FailureKind`] for what that costs. Not retried: the CLI has already
    /// retried internally by the time this surfaces, and the credentials will
    /// not have changed.
    #[error("codex authentication failed: {}", first_line(message))]
    Auth {
        /// The CLI's stderr, trimmed. Retained whole rather than reduced to
        /// the matched line, so nothing is lost to classification.
        message: String,
        command: String,
        exit_code: i32,
        working_dir: Option<PathBuf>,
    },

    /// The CLI rejected the configuration before running.
    ///
    /// An unknown key under `--strict-config`, or a malformed override.
    #[error("codex rejected the configuration: {}", first_line(message))]
    Config {
        /// The CLI's stderr, trimmed.
        message: String,
        command: String,
        exit_code: i32,
        working_dir: Option<PathBuf>,
    },

    /// The working directory is not a trusted directory or git repo, and
    /// `--skip-git-repo-check` was not set.
    #[error("codex refused an untrusted directory: {}", first_line(message))]
    NotTrustedDirectory {
        /// The CLI's stderr, trimmed.
        message: String,
        command: String,
        exit_code: i32,
        working_dir: Option<PathBuf>,
    },

    /// The session or thread being resumed does not exist.
    #[error("codex session not found: {}", first_line(message))]
    SessionNotFound {
        /// The CLI's stderr, trimmed.
        message: String,
        command: String,
        exit_code: i32,
        working_dir: Option<PathBuf>,
    },

    /// A codex command failed with a non-zero exit code.
    #[error("codex command failed: {command} (exit code {exit_code}){}{}{}", working_dir.as_ref().map(|d| format!(" (in {})", d.display())).unwrap_or_default(), if stdout.is_empty() { String::new() } else { format!("\nstdout: {stdout}") }, if stderr.is_empty() { String::new() } else { format!("\nstderr: {stderr}") })]
    CommandFailed {
        command: String,
        exit_code: i32,
        stdout: String,
        stderr: String,
        working_dir: Option<PathBuf>,
    },

    /// An I/O error occurred while spawning or communicating with the process.
    #[error("io error: {message}{}", working_dir.as_ref().map(|d| format!(" (in {})", d.display())).unwrap_or_default())]
    Io {
        message: String,
        #[source]
        source: std::io::Error,
        working_dir: Option<PathBuf>,
    },

    /// The command timed out.
    #[error("codex command timed out after {timeout_seconds}s")]
    Timeout { timeout_seconds: u64 },

    /// A session's token budget was reached.
    ///
    /// Denominated in tokens rather than money because the CLI reports token
    /// counts and no cost; see [`crate::budget`].
    #[error("token budget exceeded: {total_tokens} of {max_tokens} tokens")]
    TokenBudgetExceeded {
        /// Tokens recorded when the ceiling was hit. May exceed `max_tokens`,
        /// since a turn's usage is only known once it has been spent.
        total_tokens: u64,
        /// The configured ceiling.
        max_tokens: u64,
    },

    /// A file on disk could not be parsed.
    ///
    /// Distinct from [`Error::Config`], which is the CLI rejecting a
    /// configuration it was given. This one never ran a command, so it
    /// carries no exit code and is not a [`FailureKind`].
    #[cfg(feature = "config")]
    #[error("failed to parse {}: {message}", path.display())]
    ConfigParse {
        /// The file that could not be parsed.
        path: PathBuf,
        /// The parser's message.
        message: String,
    },

    /// A bypass of codex's safety controls was requested without permission.
    ///
    /// See [`crate::dangerous`]. Never a command failure: nothing ran.
    #[error("bypassing codex safety controls requires {variable} to be set")]
    DangerousNotAllowed {
        /// The environment variable that would have permitted it.
        variable: &'static str,
    },

    /// The run was cancelled by the caller.
    ///
    /// The process group was asked to stop, given `grace_seconds`, then
    /// killed. Distinct from [`Error::Timeout`], which is the client's own
    /// deadline rather than the caller's decision.
    #[error("codex run cancelled (after a {grace_seconds}s grace period)")]
    Cancelled {
        /// How long the group was given to exit before being killed.
        grace_seconds: u64,
    },

    /// JSON parsing failed.
    #[cfg(feature = "json")]
    #[error("json parse error: {message}")]
    Json {
        message: String,
        #[source]
        source: serde_json::Error,
    },

    /// The installed CLI version does not meet the minimum requirement.
    #[error("CLI version {found} does not meet minimum requirement {minimum}")]
    VersionMismatch {
        found: crate::version::CliVersion,
        minimum: crate::version::CliVersion,
    },

    /// The installed CLI is outside the wrapper's tested-against range.
    ///
    /// Only returned by
    /// [`Codex::ensure_tested_cli_version`](crate::Codex::ensure_tested_cli_version).
    /// The default path reports drift as a
    /// [`CliVersionStatus`](crate::CliVersionStatus) rather than an error.
    #[error("CLI version {found} is outside the tested range {tested_min}..={tested_max}")]
    UntestedCliVersion {
        found: crate::version::CliVersion,
        tested_min: crate::version::CliVersion,
        tested_max: crate::version::CliVersion,
    },
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io {
            message: e.to_string(),
            source: e,
            working_dir: None,
        }
    }
}

/// Result type alias for codex-wrapper operations.
pub type Result<T> = std::result::Result<T, Error>;

/// The first non-empty line of a message, for a one-line `Display`.
fn first_line(message: &str) -> &str {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(message)
}

/// The class of a failed command, for matching without destructuring.
///
/// Returned by [`Error::failure_kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureKind {
    /// The CLI could not authenticate.
    Auth,
    /// The CLI rejected the configuration before running.
    Config,
    /// The working directory was not trusted.
    NotTrustedDirectory,
    /// The session or thread being resumed does not exist.
    SessionNotFound,
    /// A non-zero exit that matched no known signature.
    Unclassified,
}

/// Signatures observed on `codex-cli` 0.145.0, each from a captured failing
/// run. Matched against stderr, since every one of them exits 1: the exit code
/// carries no information here.
///
/// Deliberately matched on the stable part of each message. The auth line, for
/// instance, also carries a request id and a `cf-ray` header that differ every
/// time.
const SIGNATURES: &[(&str, FailureKind)] = &[
    ("401 Unauthorized", FailureKind::Auth),
    ("Missing bearer or basic authentication", FailureKind::Auth),
    (
        "Not inside a trusted directory",
        FailureKind::NotTrustedDirectory,
    ),
    ("Error loading config.toml", FailureKind::Config),
    ("unknown configuration field", FailureKind::Config),
    (
        "no rollout found for thread id",
        FailureKind::SessionNotFound,
    ),
];

impl Error {
    /// Build an error from a failed command, classifying it by stderr.
    ///
    /// Every non-zero exit in this crate goes through here, so a caller can
    /// branch on the class rather than substring-matching stderr themselves.
    /// An unrecognized failure stays [`Error::CommandFailed`] with its output
    /// intact, so classification never loses information.
    ///
    /// Classification is by message, because it has to be: every failure
    /// observed on 0.145.0 exits 1. That makes it sensitive to the CLI
    /// rewording a message, which is the cost of doing it here instead of in
    /// every caller. `tests/contract.rs` is where a reworded message should be
    /// caught.
    #[must_use]
    pub fn from_command_failure(
        command: String,
        exit_code: i32,
        stdout: String,
        stderr: String,
        working_dir: Option<PathBuf>,
    ) -> Self {
        let message = stderr.trim().to_string();

        let kind = SIGNATURES
            .iter()
            .find(|(needle, _)| message.contains(needle))
            .map(|(_, kind)| *kind);

        match kind {
            Some(FailureKind::Auth) => Error::Auth {
                message,
                command,
                exit_code,
                working_dir,
            },
            Some(FailureKind::Config) => Error::Config {
                message,
                command,
                exit_code,
                working_dir,
            },
            Some(FailureKind::NotTrustedDirectory) => Error::NotTrustedDirectory {
                message,
                command,
                exit_code,
                working_dir,
            },
            Some(FailureKind::SessionNotFound) => Error::SessionNotFound {
                message,
                command,
                exit_code,
                working_dir,
            },
            _ => Error::CommandFailed {
                command,
                exit_code,
                stdout,
                stderr,
                working_dir,
            },
        }
    }

    /// The class of this failure, or `None` if it is not a command failure.
    #[must_use]
    pub fn failure_kind(&self) -> Option<FailureKind> {
        match self {
            Error::Auth { .. } => Some(FailureKind::Auth),
            Error::Config { .. } => Some(FailureKind::Config),
            Error::NotTrustedDirectory { .. } => Some(FailureKind::NotTrustedDirectory),
            Error::SessionNotFound { .. } => Some(FailureKind::SessionNotFound),
            Error::CommandFailed { .. } => Some(FailureKind::Unclassified),
            _ => None,
        }
    }

    /// The process exit code, for any variant that came from one.
    ///
    /// Classification moved some failures off [`Error::CommandFailed`], so
    /// anything reading an exit code should read it here rather than matching
    /// that one variant.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Error::CommandFailed { exit_code, .. }
            | Error::Auth { exit_code, .. }
            | Error::Config { exit_code, .. }
            | Error::NotTrustedDirectory { exit_code, .. }
            | Error::SessionNotFound { exit_code, .. } => Some(*exit_code),
            _ => None,
        }
    }

    /// Whether re-running the identical command could plausibly succeed.
    ///
    /// False for the classified failures: each is a deterministic rejection,
    /// and the CLI has already retried the auth case internally before it
    /// surfaces here.
    #[must_use]
    pub fn is_deterministic_failure(&self) -> bool {
        matches!(
            self,
            Error::Auth { .. }
                | Error::Config { .. }
                | Error::NotTrustedDirectory { .. }
                | Error::SessionNotFound { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_not_found() {
        let err = Error::NotFound;
        assert_eq!(err.to_string(), "codex binary not found in PATH");
    }

    #[test]
    fn display_command_failed_minimal() {
        let err = Error::CommandFailed {
            command: "exec".to_string(),
            exit_code: 1,
            stdout: String::new(),
            stderr: String::new(),
            working_dir: None,
        };
        assert_eq!(err.to_string(), "codex command failed: exec (exit code 1)");
    }

    #[test]
    fn display_command_failed_with_all_fields() {
        let err = Error::CommandFailed {
            command: "exec".to_string(),
            exit_code: 2,
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            working_dir: Some(PathBuf::from("/tmp")),
        };
        assert_eq!(
            err.to_string(),
            "codex command failed: exec (exit code 2) (in /tmp)\nstdout: out\nstderr: err"
        );
    }

    #[test]
    fn display_io_without_working_dir() {
        let source = std::io::Error::other("disk full");
        let err = Error::Io {
            message: source.to_string(),
            source,
            working_dir: None,
        };
        assert_eq!(err.to_string(), "io error: disk full");
    }

    #[test]
    fn display_io_with_working_dir() {
        let source = std::io::Error::other("disk full");
        let err = Error::Io {
            message: source.to_string(),
            source,
            working_dir: Some(PathBuf::from("/home/user")),
        };
        assert_eq!(err.to_string(), "io error: disk full (in /home/user)");
    }

    #[test]
    fn display_timeout() {
        let err = Error::Timeout {
            timeout_seconds: 30,
        };
        assert_eq!(err.to_string(), "codex command timed out after 30s");
    }

    #[cfg(feature = "json")]
    #[test]
    fn display_json() {
        let source: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("invalid").unwrap_err();
        let err = Error::Json {
            message: source.to_string(),
            source,
        };
        assert!(err.to_string().starts_with("json parse error:"));
    }

    #[test]
    fn display_version_mismatch() {
        let err = Error::VersionMismatch {
            found: crate::version::CliVersion::new(0, 100, 0),
            minimum: crate::version::CliVersion::new(0, 145, 0),
        };
        assert_eq!(
            err.to_string(),
            "CLI version 0.100.0 does not meet minimum requirement 0.145.0"
        );
    }

    // ---------------------------------------------------------------
    // Classification (#85)
    // ---------------------------------------------------------------

    fn classify(stderr: &str) -> Error {
        Error::from_command_failure(
            "codex exec hi".into(),
            1,
            String::new(),
            stderr.into(),
            None,
        )
    }

    /// Each string is transcribed from a captured codex-cli 0.145.0 failure,
    /// varying parts and all.
    #[test]
    fn classifies_every_captured_signature() {
        let cases: &[(&str, FailureKind)] = &[
            (
                "ERROR: unexpected status 401 Unauthorized: Missing bearer or basic authentication in header, url: https://api.openai.com/v1/responses, cf-ray: a272310168bcba62-SJC, request id: req_ef11",
                FailureKind::Auth,
            ),
            (
                "Not inside a trusted directory and --skip-git-repo-check was not specified.",
                FailureKind::NotTrustedDirectory,
            ),
            (
                "Error loading config.toml: unknown configuration field `bogus` in -c/--config override",
                FailureKind::Config,
            ),
            (
                "Error: thread/resume: thread/resume failed: no rollout found for thread id 00000000-0000-0000-0000-000000000000 (code -32600)",
                FailureKind::SessionNotFound,
            ),
        ];

        for (stderr, expected) in cases {
            let err = classify(stderr);
            assert_eq!(
                err.failure_kind(),
                Some(*expected),
                "misclassified: {stderr}"
            );
            assert_eq!(err.exit_code(), Some(1));
            assert!(err.is_deterministic_failure(), "{stderr}");
        }
    }

    /// An unrecognized failure must keep its output rather than being forced
    /// into a class. Classification is allowed to not know.
    #[test]
    fn an_unknown_failure_stays_command_failed_with_its_output() {
        let err = Error::from_command_failure(
            "codex exec hi".into(),
            2,
            "partial stdout".into(),
            "something new the CLI started saying".into(),
            None,
        );

        assert_eq!(err.failure_kind(), Some(FailureKind::Unclassified));
        assert!(!err.is_deterministic_failure());
        match err {
            Error::CommandFailed {
                stdout,
                stderr,
                exit_code,
                ..
            } => {
                assert_eq!(stdout, "partial stdout");
                assert_eq!(stderr, "something new the CLI started saying");
                assert_eq!(exit_code, 2);
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }

    /// The whole of stderr is kept, not just the matched line, so nothing is
    /// lost to classification.
    #[test]
    fn a_classified_failure_keeps_the_full_message() {
        let err = classify(
            "ERROR: Reconnecting... 5/5\nERROR: unexpected status 401 Unauthorized: Missing bearer",
        );
        match &err {
            Error::Auth { message, .. } => {
                assert!(message.contains("Reconnecting... 5/5"), "{message}");
                assert!(message.contains("401 Unauthorized"), "{message}");
            }
            other => panic!("expected Auth, got {other:?}"),
        }
        // Display stays one line, leading with the first thing stderr said.
        assert_eq!(
            err.to_string(),
            "codex authentication failed: ERROR: Reconnecting... 5/5"
        );
    }

    #[test]
    fn non_command_errors_have_no_failure_kind() {
        assert_eq!(Error::NotFound.failure_kind(), None);
        assert_eq!(Error::Timeout { timeout_seconds: 5 }.failure_kind(), None);
        assert_eq!(Error::NotFound.exit_code(), None);
    }
}
