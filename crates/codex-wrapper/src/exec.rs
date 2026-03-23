//! Process execution layer for spawning and communicating with the `codex`
//! binary, including timeout and retry support.

use std::fmt;
use std::time::Duration;

use tokio::process::Command;
use tracing::debug;

use crate::Codex;
use crate::error::{Error, Result};

/// Raw output from a Codex CLI invocation.
///
/// Contains captured stdout/stderr, the process exit code, and a convenience
/// `success` flag.
#[derive(Clone)]
pub struct CommandOutput {
    /// Standard output as a UTF-8 string.
    pub stdout: String,
    /// Standard error as a UTF-8 string.
    pub stderr: String,
    /// Process exit code (`-1` if the process was killed by a signal).
    pub exit_code: i32,
    /// `true` when the process exited with code 0.
    pub success: bool,
}

const DEBUG_TRUNCATE_LEN: usize = 200;

fn truncate_for_debug(s: &str) -> String {
    if s.len() > DEBUG_TRUNCATE_LEN {
        format!("{}... ({} bytes total)", &s[..DEBUG_TRUNCATE_LEN], s.len())
    } else {
        s.to_string()
    }
}

impl fmt::Debug for CommandOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandOutput")
            .field("stdout", &truncate_for_debug(&self.stdout))
            .field("stderr", &truncate_for_debug(&self.stderr))
            .field("exit_code", &self.exit_code)
            .field("success", &self.success)
            .finish()
    }
}

/// Run a codex command with the given arguments.
///
/// If the [`Codex`] client has a retry policy set, transient errors will be
/// retried according to that policy. A per-command retry policy can be passed
/// to override the client default.
pub async fn run_codex(codex: &Codex, args: Vec<String>) -> Result<CommandOutput> {
    run_codex_with_retry(codex, args, None).await
}

/// Run a codex command with an optional per-command retry policy override.
pub async fn run_codex_with_retry(
    codex: &Codex,
    args: Vec<String>,
    retry_override: Option<&crate::retry::RetryPolicy>,
) -> Result<CommandOutput> {
    let policy = retry_override.or(codex.retry_policy.as_ref());

    match policy {
        Some(policy) => {
            crate::retry::with_retry(policy, || run_codex_once(codex, args.clone())).await
        }
        None => run_codex_once(codex, args).await,
    }
}

async fn run_codex_once(codex: &Codex, args: Vec<String>) -> Result<CommandOutput> {
    let mut command_args = Vec::new();

    // Global args first (before subcommand)
    command_args.extend(codex.global_args.clone());

    // Then command-specific args
    command_args.extend(args);

    debug!(binary = %codex.binary.display(), args = ?command_args, "executing codex command");

    let output = if let Some(timeout) = codex.timeout {
        run_with_timeout(
            &codex.binary,
            &command_args,
            &codex.env,
            codex.working_dir.as_deref(),
            timeout,
        )
        .await?
    } else {
        run_internal(
            &codex.binary,
            &command_args,
            &codex.env,
            codex.working_dir.as_deref(),
        )
        .await?
    };

    Ok(output)
}

/// Run a codex command and allow specific non-zero exit codes.
pub async fn run_codex_allow_exit_codes(
    codex: &Codex,
    args: Vec<String>,
    allowed_codes: &[i32],
) -> Result<CommandOutput> {
    let output = run_codex(codex, args).await;

    match output {
        Err(Error::CommandFailed {
            exit_code,
            stdout,
            stderr,
            ..
        }) if allowed_codes.contains(&exit_code) => Ok(CommandOutput {
            stdout,
            stderr,
            exit_code,
            success: false,
        }),
        other => other,
    }
}

async fn run_internal(
    binary: &std::path::Path,
    args: &[String],
    env: &std::collections::HashMap<String, String>,
    working_dir: Option<&std::path::Path>,
) -> Result<CommandOutput> {
    let mut cmd = Command::new(binary);
    cmd.args(args);

    // Prevent child from inheriting/blocking on parent's stdin.
    cmd.stdin(std::process::Stdio::null());

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    for (key, value) in env {
        cmd.env(key, value);
    }

    let output = cmd.output().await.map_err(|e| Error::Io {
        message: format!("failed to spawn codex: {e}"),
        source: e,
        working_dir: working_dir.map(|p| p.to_path_buf()),
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    if !output.status.success() {
        return Err(Error::CommandFailed {
            command: format!("{} {}", binary.display(), args.join(" ")),
            exit_code,
            stdout,
            stderr,
            working_dir: working_dir.map(|p| p.to_path_buf()),
        });
    }

    Ok(CommandOutput {
        stdout,
        stderr,
        exit_code,
        success: true,
    })
}

async fn run_with_timeout(
    binary: &std::path::Path,
    args: &[String],
    env: &std::collections::HashMap<String, String>,
    working_dir: Option<&std::path::Path>,
    timeout: Duration,
) -> Result<CommandOutput> {
    tokio::time::timeout(timeout, run_internal(binary, args, env, working_dir))
        .await
        .map_err(|_| Error::Timeout {
            timeout_seconds: timeout.as_secs(),
        })?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_output(stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code: 0,
            success: true,
        }
    }

    #[test]
    fn debug_short_output_not_truncated() {
        let output = make_output("hello", "world");
        let debug = format!("{output:?}");
        assert!(debug.contains("hello"));
        assert!(debug.contains("world"));
        assert!(!debug.contains("bytes total"));
    }

    #[test]
    fn debug_long_output_truncated() {
        let long = "x".repeat(300);
        let output = make_output(&long, &long);
        let debug = format!("{output:?}");
        assert!(debug.contains("... (300 bytes total)"));
        assert!(!debug.contains(&long));
    }
}
