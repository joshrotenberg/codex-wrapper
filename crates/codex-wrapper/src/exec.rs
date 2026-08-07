//! Process execution layer for spawning and communicating with the `codex`
//! binary, including timeout and retry support.

use std::fmt;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tracing::{Instrument, Span, debug, field, info_span};

use crate::Codex;
use crate::error::{Error, Result};

/// Build the span covering one invocation.
///
/// The fields carry what identifies a run, never what it contains: no prompt
/// and no environment. Both routinely hold content a host would not want in
/// its logs, and a span is recorded whenever any subscriber is installed.
pub(crate) fn command_span(name: &'static str, codex: &Codex, args: &[String]) -> Span {
    let working_dir = codex
        .working_dir
        .as_ref()
        .map_or_else(|| "(inherited)".to_string(), |p| p.display().to_string());

    info_span!(
        parent: Span::current(),
        "codex",
        otel.name = name,
        subcommand = args.first().map_or("(none)", String::as_str),
        binary = %codex.binary.display(),
        working_dir = %working_dir,
        outcome = field::Empty,
        exit_code = field::Empty,
        duration_ms = field::Empty,
    )
}

/// Records how a run ended on its span, including when it does not end at all.
///
/// The dropped-future case is the one worth the machinery. Cancellation kills
/// the process and runs nothing else, so without this the span would close
/// with no outcome and an abandoned run would be indistinguishable from one
/// still in progress. The span is passed in and held rather than read from
/// [`Span::current`]: current-span tracking is the subscriber's job, and a
/// subscriber that does not implement it would silently drop every record,
/// including the drop-time one this exists for.
pub(crate) struct SpanOutcome {
    span: Span,
    started: Instant,
    settled: bool,
}

impl SpanOutcome {
    pub(crate) fn start(span: Span) -> Self {
        Self {
            span,
            started: Instant::now(),
            settled: false,
        }
    }

    pub(crate) fn settle(&mut self, outcome: &'static str, exit_code: Option<i32>) {
        self.settled = true;
        self.span.record("outcome", outcome);
        self.span
            .record("duration_ms", self.started.elapsed().as_millis() as u64);
        if let Some(code) = exit_code {
            self.span.record("exit_code", code);
        }
    }

    fn settle_from(&mut self, result: &Result<CommandOutput>) {
        match result {
            Ok(output) => self.settle("ok", Some(output.exit_code)),
            Err(Error::CommandFailed { exit_code, .. }) => self.settle("failed", Some(*exit_code)),
            Err(Error::Timeout { .. }) => self.settle("timeout", None),
            Err(_) => self.settle("error", None),
        }
    }
}

impl Drop for SpanOutcome {
    fn drop(&mut self) {
        if !self.settled {
            self.settle("cancelled", None);
        }
    }
}

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
            // A parent span so the retry events and each attempt's own span
            // nest under one run rather than arriving as unrelated lines.
            let span = info_span!(
                "codex.retry",
                subcommand = args.first().map_or("(none)", String::as_str),
                max_attempts = policy.max_attempts,
            );
            crate::retry::with_retry(policy, || run_codex_once(codex, args.clone()))
                .instrument(span)
                .await
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
    let span = command_span("codex.exec", codex, &args);
    let outcome_span = span.clone();
    let command_args = assemble_args(codex, args);

    async move {
        debug!(binary = %codex.binary.display(), args = ?command_args, "executing codex command");

        let mut outcome = SpanOutcome::start(outcome_span);
        let result = match codex.timeout {
            Some(timeout) => {
                run_with_timeout(
                    &codex.binary,
                    &command_args,
                    &codex.env,
                    codex.working_dir.as_deref(),
                    timeout,
                )
                .await
            }
            None => {
                run_internal(
                    &codex.binary,
                    &command_args,
                    &codex.env,
                    codex.working_dir.as_deref(),
                )
                .await
            }
        };
        outcome.settle_from(&result);
        result
    }
    .instrument(span)
    .await
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
    let span = command_span("codex.exec", codex, &args);
    let outcome_span = span.clone();
    let command_args = assemble_args(codex, args);

    async move {
        debug!(
            binary = %codex.binary.display(),
            args = ?command_args,
            prompt_bytes = prompt.len(),
            "executing codex command with a stdin prompt"
        );

        let mut outcome = SpanOutcome::start(outcome_span);
        let run = run_internal_inner(
            &codex.binary,
            &command_args,
            &codex.env,
            codex.working_dir.as_deref(),
            Some(prompt),
        );

        let result = match codex.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, run).await {
                Ok(result) => result,
                Err(_) => Err(Error::Timeout {
                    timeout_seconds: timeout.as_secs(),
                }),
            },
            None => run.await,
        };
        outcome.settle_from(&result);
        result
    }
    .instrument(span)
    .await
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

    /// Minimal recording subscriber, written by hand because #63 rules out a
    /// new dependency and `tracing-subscriber` would be one. It collects every
    /// field recorded on any span, which is all these tests need.
    #[cfg(unix)]
    mod recorder {
        use std::sync::{Arc, Mutex};

        use tracing::field::{Field, Visit};
        use tracing::span::{Attributes, Id, Record};
        use tracing::{Event, Metadata, Subscriber};

        #[derive(Clone, Default)]
        pub(super) struct Recorder(Arc<Mutex<Vec<(String, String)>>>);

        impl Recorder {
            pub(super) fn value(&self, field: &str) -> Option<String> {
                self.0
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|(name, _)| name == field)
                    .map(|(_, value)| value.clone())
            }
        }

        struct Collect<'a>(&'a mut Vec<(String, String)>);

        impl Visit for Collect<'_> {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                self.0.push((field.name().into(), format!("{value:?}")));
            }
            fn record_str(&mut self, field: &Field, value: &str) {
                self.0.push((field.name().into(), value.into()));
            }
            fn record_i64(&mut self, field: &Field, value: i64) {
                self.0.push((field.name().into(), value.to_string()));
            }
            fn record_u64(&mut self, field: &Field, value: u64) {
                self.0.push((field.name().into(), value.to_string()));
            }
        }

        impl Subscriber for Recorder {
            fn enabled(&self, _: &Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, attrs: &Attributes<'_>) -> Id {
                attrs.record(&mut Collect(&mut self.0.lock().unwrap()));
                Id::from_u64(1)
            }
            fn record(&self, _: &Id, values: &Record<'_>) {
                values.record(&mut Collect(&mut self.0.lock().unwrap()));
            }
            fn record_follows_from(&self, _: &Id, _: &Id) {}
            fn event(&self, _: &Event<'_>) {}
            fn enter(&self, _: &Id) {}
            fn exit(&self, _: &Id) {}
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn span_records_the_subcommand_and_a_clean_outcome() {
        let recorder = recorder::Recorder::default();
        let _guard = tracing::subscriber::set_default(recorder.clone());

        let codex = Codex::builder()
            .binary("/bin/echo")
            .build()
            .expect("echo must exist");
        run_codex(&codex, vec!["exec".into()]).await.unwrap();

        assert_eq!(recorder.value("subcommand").as_deref(), Some("exec"));
        assert_eq!(recorder.value("outcome").as_deref(), Some("ok"));
        assert_eq!(recorder.value("exit_code").as_deref(), Some("0"));
        assert!(recorder.value("duration_ms").is_some());
    }

    /// The prompt must never reach the span. It is in argv, so recording the
    /// arguments would leak it into any host's logs.
    #[cfg(unix)]
    #[tokio::test]
    async fn span_does_not_carry_the_prompt() {
        let recorder = recorder::Recorder::default();
        let _guard = tracing::subscriber::set_default(recorder.clone());

        let codex = Codex::builder()
            .binary("/bin/echo")
            .build()
            .expect("echo must exist");
        run_codex(&codex, vec!["exec".into(), "a very secret prompt".into()])
            .await
            .unwrap();

        let recorded = format!("{:?}", recorder.value("subcommand"));
        assert!(!recorded.contains("secret"));
        for field in ["binary", "working_dir", "outcome", "exit_code"] {
            let value = recorder.value(field).unwrap_or_default();
            assert!(
                !value.contains("secret"),
                "{field} leaked the prompt: {value}"
            );
        }
    }

    /// A dropped future records an outcome rather than leaving the span open,
    /// so an abandoned run is distinguishable from one still in progress.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_cancelled_run_is_recorded_as_cancelled() {
        let recorder = recorder::Recorder::default();
        let _guard = tracing::subscriber::set_default(recorder.clone());

        let pid_file = crate::test_support::PidFile::new("span-cancel");
        let codex = crate::test_support::blocking_codex(&pid_file)
            .build()
            .expect("bash must exist");

        let cancelled = tokio::time::timeout(
            Duration::from_millis(300),
            run_codex(&codex, vec!["exec".into()]),
        )
        .await;
        assert!(cancelled.is_err(), "the run should still have been going");

        assert_eq!(recorder.value("outcome").as_deref(), Some("cancelled"));
    }

    /// A wrapper timeout is its own outcome, distinct from a caller
    /// cancelling: the run reached its deadline rather than being abandoned.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_run_is_recorded_as_timeout() {
        let recorder = recorder::Recorder::default();
        let _guard = tracing::subscriber::set_default(recorder.clone());

        let pid_file = crate::test_support::PidFile::new("span-timeout");
        let codex = crate::test_support::blocking_codex(&pid_file)
            .timeout(Duration::from_millis(300))
            .build()
            .expect("bash must exist");

        let result = run_codex(&codex, vec!["exec".into()]).await;
        assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");

        assert_eq!(recorder.value("outcome").as_deref(), Some("timeout"));
    }
}
