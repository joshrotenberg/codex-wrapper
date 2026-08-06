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

/// Assemble the full argument list for an invocation: the client's global
/// args first, since they precede the subcommand, then the command's own.
///
/// Both spawn paths and [`CodexCommand::to_command_string`] go through here,
/// so a previewed command cannot drift from the one that actually runs.
///
/// [`CodexCommand::to_command_string`]: crate::command::CodexCommand::to_command_string
pub(crate) fn assemble_args(codex: &Codex, args: Vec<String>) -> Vec<String> {
    let mut command_args = Vec::with_capacity(codex.global_args.len() + args.len());
    command_args.extend(codex.global_args.iter().cloned());
    command_args.extend(args);
    command_args
}

/// Render an invocation as a copy-pasteable shell command.
pub(crate) fn command_string(codex: &Codex, args: Vec<String>) -> String {
    let mut out = shell_quote(&codex.binary.display().to_string());
    for arg in assemble_args(codex, args) {
        out.push(' ');
        out.push_str(&shell_quote(&arg));
    }
    out
}

/// Quote a single argument for a POSIX shell, if it needs it.
///
/// The empty string is quoted: unquoted it would vanish from the rendered
/// command, turning a preview into something that runs differently from what
/// it describes.
pub(crate) fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.contains(|c: char| c.is_whitespace() || "\"'$\\`|;<>&()[]{}*?!~#".contains(c)) {
        return format!("'{}'", arg.replace('\'', r"'\''"));
    }
    arg.to_string()
}

async fn run_codex_once(codex: &Codex, args: Vec<String>) -> Result<CommandOutput> {
    let command_args = assemble_args(codex, args);

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

/// Run a codex command, writing `prompt` to the child's stdin.
///
/// For `codex exec -`, where the prompt is delivered on stdin rather than as
/// an argument. The prompt is written and the handle dropped, so the CLI sees
/// EOF and stops waiting for more.
///
/// Retry does not apply. The policy is deliberately ignored rather than
/// honored: a retry would have to write the prompt again, and the first
/// attempt has already moved the caller's data into a pipe that cannot be
/// rewound. Silently retrying with an empty stdin would be worse than not
/// retrying at all.
pub async fn run_codex_with_stdin_prompt(
    codex: &Codex,
    args: Vec<String>,
    prompt: &str,
) -> Result<CommandOutput> {
    let command_args = assemble_args(codex, args);

    debug!(
        binary = %codex.binary.display(),
        args = ?command_args,
        prompt_bytes = prompt.len(),
        "executing codex command with a stdin prompt"
    );

    let run = run_internal_inner(
        &codex.binary,
        &command_args,
        &codex.env,
        codex.working_dir.as_deref(),
        Some(prompt),
    );

    match codex.timeout {
        Some(timeout) => tokio::time::timeout(timeout, run)
            .await
            .map_err(|_| Error::Timeout {
                timeout_seconds: timeout.as_secs(),
            })?,
        None => run.await,
    }
}

async fn run_internal(
    binary: &std::path::Path,
    args: &[String],
    env: &std::collections::HashMap<String, String>,
    working_dir: Option<&std::path::Path>,
) -> Result<CommandOutput> {
    run_internal_inner(binary, args, env, working_dir, None).await
}

async fn run_internal_inner(
    binary: &std::path::Path,
    args: &[String],
    env: &std::collections::HashMap<String, String>,
    working_dir: Option<&std::path::Path>,
    stdin_prompt: Option<&str>,
) -> Result<CommandOutput> {
    let mut cmd = Command::new(binary);
    cmd.args(args);

    // Pipe stdin only when there is a prompt to write. Otherwise close it, so
    // the child neither inherits nor blocks on the parent's.
    if stdin_prompt.is_some() {
        cmd.stdin(std::process::Stdio::piped());
    } else {
        cmd.stdin(std::process::Stdio::null());
    }

    // Kill the child if this future is dropped: on timeout, on caller
    // cancellation, or on task abort. Without this, tokio detaches the child
    // and codex keeps running with no handle left to stop it.
    cmd.kill_on_drop(true);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    for (key, value) in env {
        cmd.env(key, value);
    }

    let output = match stdin_prompt {
        // `Command::output` forces stdin to null, so the piped case cannot use
        // it and spawns directly.
        None => cmd.output().await.map_err(|e| Error::Io {
            message: format!("failed to spawn codex: {e}"),
            source: e,
            working_dir: working_dir.map(|p| p.to_path_buf()),
        })?,
        Some(prompt) => {
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            let mut child = cmd.spawn().map_err(|e| Error::Io {
                message: format!("failed to spawn codex: {e}"),
                source: e,
                working_dir: working_dir.map(|p| p.to_path_buf()),
            })?;

            let mut stdin = child.stdin.take().expect("stdin was configured as piped");
            let write = async move {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(prompt.as_bytes()).await?;
                // Closing the write half is what tells the CLI the prompt is
                // complete. Without it the child waits for more input.
                stdin.shutdown().await
            };

            // Concurrently, not sequentially: a prompt larger than the pipe
            // buffer blocks until the child reads it, and a child that writes
            // to stdout meanwhile blocks until we read that. Waiting for the
            // write to finish before draining stdout would deadlock both.
            let (write_result, output_result) = tokio::join!(write, child.wait_with_output());

            write_result.map_err(|e| Error::Io {
                message: format!("failed to write the prompt to codex stdin: {e}"),
                source: e,
                working_dir: working_dir.map(|p| p.to_path_buf()),
            })?;
            output_result.map_err(|e| Error::Io {
                message: format!("failed to wait on codex: {e}"),
                source: e,
                working_dir: working_dir.map(|p| p.to_path_buf()),
            })?
        }
    };

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
    fn shell_quote_leaves_plain_words_alone() {
        assert_eq!(shell_quote("exec"), "exec");
        assert_eq!(shell_quote("--ephemeral"), "--ephemeral");
        assert_eq!(shell_quote("model=gpt-5"), "model=gpt-5");
    }

    #[test]
    fn shell_quote_wraps_anything_a_shell_would_read() {
        assert_eq!(shell_quote("fix the tests"), "'fix the tests'");
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
        assert_eq!(shell_quote("a;b"), "'a;b'");
        assert_eq!(shell_quote("*.rs"), "'*.rs'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    /// An unquoted empty string would disappear from the rendered command,
    /// making the preview describe a different invocation than it previews.
    #[test]
    fn shell_quote_keeps_the_empty_argument_visible() {
        assert_eq!(shell_quote(""), "''");
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

    /// A wrapper timeout drops `run_internal`'s future. Without
    /// `kill_on_drop`, `Error::Timeout` would mean "we stopped waiting" while
    /// codex kept running.
    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_spawned_process() {
        use crate::test_support::{PidFile, blocking_codex, wait_until_gone};

        let pid_file = PidFile::new("exec-timeout");
        let codex = blocking_codex(&pid_file)
            .timeout(Duration::from_millis(500))
            .build()
            .expect("bash must exist");

        let result = run_codex(&codex, vec!["exec".into(), "probe".into()]).await;
        assert!(
            matches!(result, Err(Error::Timeout { .. })),
            "expected timeout error, got: {result:?}"
        );

        let pid = pid_file.read_pid().await;
        assert!(
            wait_until_gone(pid).await,
            "codex ({pid}) survived the timeout"
        );
    }

    /// The caller dropping the future is the case an operator kill or a
    /// graceful shutdown produces, with no wrapper timeout involved.
    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_kills_the_spawned_process() {
        use crate::test_support::{PidFile, blocking_codex, wait_until_gone};

        let pid_file = PidFile::new("exec-cancel");
        let codex = blocking_codex(&pid_file).build().expect("bash must exist");

        let cancelled = tokio::time::timeout(
            Duration::from_millis(500),
            run_codex(&codex, vec!["exec".into(), "probe".into()]),
        )
        .await;
        assert!(
            cancelled.is_err(),
            "fake codex should still have been running, got: {cancelled:?}"
        );

        let pid = pid_file.read_pid().await;
        assert!(
            wait_until_gone(pid).await,
            "codex ({pid}) survived the dropped future"
        );
    }
}
