//! Error types for `codex-wrapper`.

use std::path::PathBuf;

/// Errors returned by `codex-wrapper` operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The `codex` binary was not found in PATH.
    #[error("codex binary not found in PATH")]
    NotFound,

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
            minimum: crate::version::CliVersion::new(0, 116, 0),
        };
        assert_eq!(
            err.to_string(),
            "CLI version 0.100.0 does not meet minimum requirement 0.116.0"
        );
    }
}
