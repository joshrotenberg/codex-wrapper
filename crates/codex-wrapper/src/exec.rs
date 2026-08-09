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

    fn settle_from_ref(&mut self, result: &Result<CommandOutput>) {
        self.settle_from(result);
    }

    fn settle_from(&mut self, result: &Result<CommandOutput>) {
        match result {
            Ok(output) => self.settle("ok", Some(output.exit_code)),
            Err(Error::Timeout { .. }) => self.settle("timeout", None),
            // Covers the classified failures too, which are no longer
            // CommandFailed but did still come from a process exit.
            Err(e) => match e.exit_code() {
                Some(code) => self.settle("failed", Some(code)),
                None => self.settle("error", None),
            },
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

/// Put the child in its own process group, so the whole run can be signalled
/// as a unit.
///
/// `kill_on_drop` reaps the direct child only, and codex spawns its own
/// subprocesses for tool use. Without a group of its own, cancelling leaves
/// those running (#78).
#[cfg(unix)]
pub(crate) fn own_process_group(cmd: &mut Command, enabled: bool) {
    if enabled {
        cmd.process_group(0);
    }
}

#[cfg(not(unix))]
pub(crate) fn own_process_group(_cmd: &mut Command, _enabled: bool) {}

/// Signal a process group, given the group leader's pid.
///
/// Unix only: there is no non-unix counterpart, because every caller of this
/// is itself gated on unix. A stub would only be dead code.
///
/// Negating the pid is what makes this reach the group rather than the leader
/// alone. Errors are ignored: the only interesting failure is that the group
/// is already gone, which is the desired state.
#[cfg(unix)]
pub(crate) fn signal_group(pid: u32, signal: i32) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    // SAFETY: `kill` with a negative pid signals a process group. Passing a
    // pid the OS has already reaped is defined and simply fails.
    unsafe {
        libc::kill(-pid, signal);
    }
}

/// SIGKILLs the run's process group when dropped.
///
/// `kill_on_drop` handles the direct child; this is what reaches the
/// subprocesses codex started. Drop cannot await, so this is the abrupt path.
/// For a graceful one, see
/// [`ExecCommand::execute_cancellable`](crate::ExecCommand::execute_cancellable).
pub(crate) struct GroupKillGuard {
    pid: Option<u32>,
}

impl GroupKillGuard {
    pub(crate) fn new(pid: Option<u32>) -> Self {
        Self { pid }
    }

    /// Stop killing on drop, once the run has finished on its own.
    pub(crate) fn disarm(&mut self) {
        self.pid = None;
    }

    /// Ask the group to stop, then insist after `grace`.
    ///
    /// Async, so it can wait, which is why it cannot live in `Drop`.
    #[cfg(unix)]
    pub(crate) async fn terminate(&mut self, grace: Duration) {
        let Some(pid) = self.pid.take() else {
            return;
        };
        signal_group(pid, libc::SIGTERM);
        tokio::time::sleep(grace).await;
        signal_group(pid, libc::SIGKILL);
    }

    /// No process groups here, so there is nothing to ask politely. The
    /// child still dies with the dropped future via `kill_on_drop`.
    #[cfg(not(unix))]
    pub(crate) async fn terminate(&mut self, _grace: Duration) {
        let _ = self.pid.take();
    }
}

impl Drop for GroupKillGuard {
    fn drop(&mut self) {
        // Taken unconditionally so the field is read on every platform, not
        // just the one that can act on it.
        if let Some(pid) = self.pid.take() {
            #[cfg(unix)]
            signal_group(pid, libc::SIGKILL);
            #[cfg(not(unix))]
            let _ = pid;
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
    let protects_rollout_budget = args
        .windows(2)
        .any(|pair| pair[0] == "-c" && crate::RolloutBudgetConfig::is_config_override(&pair[1]));
    let mut global_args = codex.global_args.iter().peekable();
    while let Some(arg) = global_args.next() {
        if protects_rollout_budget
            && matches!(arg.as_str(), "--enable" | "--disable")
            && global_args
                .peek()
                .is_some_and(|feature| feature.as_str() == "rollout_budget")
        {
            global_args.next();
            continue;
        }
        if protects_rollout_budget
            && matches!(
                arg.as_str(),
                "--enable=rollout_budget" | "--disable=rollout_budget"
            )
        {
            continue;
        }
        command_args.push(arg.clone());
    }
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
                    codex.process_group,
                )
                .await
            }
            None => {
                run_internal(
                    &codex.binary,
                    &command_args,
                    &codex.env,
                    codex.working_dir.as_deref(),
                    codex.process_group,
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

/// Run a codex command, stopping it gracefully if `cancel` resolves first.
///
/// On cancellation the run's process group is sent SIGTERM, given the client's
/// [`termination_grace`](crate::CodexBuilder::termination_grace), then killed.
/// Signalling the group rather than the pid is what reaches the subprocesses
/// codex started for tool use; killing only the direct child leaves those
/// running (#78).
///
/// Dropping the future instead is still safe, and still kills the group, but
/// abruptly: `Drop` cannot wait out a grace period.
///
/// Retry does not apply. A cancelled run is a decision, not a transient
/// failure.
///
/// On platforms without process groups this degrades to killing the child.
pub async fn run_codex_cancellable<C>(
    codex: &Codex,
    args: Vec<String>,
    cancel: C,
) -> Result<CommandOutput>
where
    C: std::future::Future<Output = ()> + Send,
{
    let span = command_span("codex.exec", codex, &args);
    let outcome_span = span.clone();
    let command_args = assemble_args(codex, args);

    async move {
        debug!(binary = %codex.binary.display(), args = ?command_args, "executing cancellable codex command");

        let mut outcome = SpanOutcome::start(outcome_span);
        let result = run_internal_inner(
            SpawnSpec {
                binary: &codex.binary,
                args: &command_args,
                env: &codex.env,
                working_dir: codex.working_dir.as_deref(),
                stdin_prompt: None,
                process_group: codex.process_group,
            },
            Some(Box::pin(cancel)),
            codex.termination_grace,
        )
        .await;

        match &result {
            Err(Error::Cancelled { .. }) => outcome.settle("cancelled", None),
            other => outcome.settle_from_ref(other),
        }
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
        // Matched on the exit code rather than the variant: classification
        // moves some failures off CommandFailed, and an allowed code is
        // allowed whatever the wrapper made of the message.
        Err(e)
            if e.exit_code()
                .is_some_and(|code| allowed_codes.contains(&code)) =>
        {
            let exit_code = e.exit_code().unwrap_or(-1);
            let (stdout, stderr) = match &e {
                Error::CommandFailed { stdout, stderr, .. } => (stdout.clone(), stderr.clone()),
                Error::Auth { message, .. }
                | Error::Config { message, .. }
                | Error::NotTrustedDirectory { message, .. }
                | Error::SessionNotFound { message, .. } => (String::new(), message.clone()),
                _ => (String::new(), String::new()),
            };
            Ok(CommandOutput {
                stdout,
                stderr,
                exit_code,
                success: false,
            })
        }
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
            SpawnSpec {
                binary: &codex.binary,
                args: &command_args,
                env: &codex.env,
                working_dir: codex.working_dir.as_deref(),
                stdin_prompt: Some(prompt),
                process_group: codex.process_group,
            },
            None,
            Duration::from_secs(0),
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
    process_group: bool,
) -> Result<CommandOutput> {
    run_internal_inner(
        SpawnSpec {
            binary,
            args,
            env,
            working_dir,
            stdin_prompt: None,
            process_group,
        },
        None,
        Duration::from_secs(0),
    )
    .await
}

type CancelFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

/// Everything one spawn needs, gathered so the signature stays readable.
struct SpawnSpec<'a> {
    binary: &'a std::path::Path,
    args: &'a [String],
    env: &'a std::collections::HashMap<String, String>,
    working_dir: Option<&'a std::path::Path>,
    /// A prompt to deliver on stdin, for `codex exec -`.
    stdin_prompt: Option<&'a str>,
    /// Whether the run leads its own process group.
    process_group: bool,
}

async fn run_internal_inner(
    spec: SpawnSpec<'_>,
    cancel: Option<CancelFuture<'_>>,
    grace: Duration,
) -> Result<CommandOutput> {
    let SpawnSpec {
        binary,
        args,
        env,
        working_dir,
        stdin_prompt,
        process_group,
    } = spec;
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
    own_process_group(&mut cmd, process_group);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    for (key, value) in env {
        cmd.env(key, value);
    }

    // Always spawn rather than using `Command::output`: the pid is needed to
    // signal the process group, and `output` does not surrender it. It also
    // forces stdin to null, which the stdin-prompt path cannot use.
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| Error::Io {
        message: format!("failed to spawn codex: {e}"),
        source: e,
        working_dir: working_dir.map(|p| p.to_path_buf()),
    })?;

    // Armed for the whole run. If this future is dropped, its Drop signals the
    // group, which is what reaches the subprocesses codex started.
    // Only meaningful when the run leads its own group. Sharing the parent's
    // means signalling it would hit the parent too.
    let mut group = GroupKillGuard::new(process_group.then(|| child.id()).flatten());
    let child_stdin = child.stdin.take();

    let write = async move {
        let (Some(prompt), Some(mut stdin)) = (stdin_prompt, child_stdin) else {
            return Ok(());
        };
        use tokio::io::AsyncWriteExt;
        stdin.write_all(prompt.as_bytes()).await?;
        // Closing the write half is what tells the CLI the prompt is
        // complete. Without it the child waits for more input.
        stdin.shutdown().await
    };

    // Concurrently, not sequentially: a prompt larger than the pipe buffer
    // blocks until the child reads it, and a child that writes to stdout
    // meanwhile blocks until we read that. Waiting for the write to finish
    // before draining stdout would deadlock both.
    let run = async { tokio::join!(write, child.wait_with_output()) };

    let finished = match cancel {
        None => Some(run.await),
        // Racing the run against the caller's signal. Cancellation here is
        // graceful, which is the whole reason it cannot live in `Drop`:
        // asking a process to stop and then waiting requires awaiting.
        Some(cancel) => tokio::select! {
            outcome = run => Some(outcome),
            () = cancel => None,
        },
    };

    let Some((write_result, output_result)) = finished else {
        group.terminate(grace).await;
        return Err(Error::Cancelled {
            grace_seconds: grace.as_secs(),
        });
    };

    write_result.map_err(|e| Error::Io {
        message: format!("failed to write the prompt to codex stdin: {e}"),
        source: e,
        working_dir: working_dir.map(|p| p.to_path_buf()),
    })?;
    let output = output_result.map_err(|e| Error::Io {
        message: format!("failed to wait on codex: {e}"),
        source: e,
        working_dir: working_dir.map(|p| p.to_path_buf()),
    })?;

    // Finished on its own, so there is no group left to kill.
    group.disarm();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    if !output.status.success() {
        return Err(Error::from_command_failure(
            format!("{} {}", binary.display(), args.join(" ")),
            exit_code,
            stdout,
            stderr,
            working_dir.map(|p| p.to_path_buf()),
        ));
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
    process_group: bool,
) -> Result<CommandOutput> {
    tokio::time::timeout(
        timeout,
        run_internal(binary, args, env, working_dir, process_group),
    )
    .await
    .map_err(|_| Error::Timeout {
        timeout_seconds: timeout.as_secs(),
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CodexCommand;

    #[test]
    fn typed_rollout_budget_suppresses_conflicting_client_global_toggles() {
        let codex = Codex::builder()
            .binary("/bin/echo")
            .config("features.rollout_budget=false")
            .enable("rollout_budget")
            .disable("rollout_budget")
            .arg("--enable=rollout_budget")
            .arg("--disable=rollout_budget")
            .arg("--disable")
            .arg("rollout_budget")
            .enable("keep-enabled")
            .disable("keep-disabled")
            .build()
            .expect("echo must exist");
        let budget = crate::RolloutBudgetConfig::builder(10_000)
            .build()
            .expect("valid budget");
        let expected = budget.config_override();
        let opening = crate::ExecCommand::new("hi")
            .rollout_budget(budget.clone())
            .args();
        let resumed = crate::ExecResumeCommand::new()
            .session_id("thread")
            .rollout_budget(budget)
            .args();

        for args in [opening, resumed] {
            let assembled = assemble_args(&codex, args);
            assert!(assembled.iter().any(|arg| arg == &expected));
            assert!(
                !assembled.windows(2).any(|pair| {
                    matches!(pair[0].as_str(), "--enable" | "--disable")
                        && pair[1] == "rollout_budget"
                }),
                "typed budget must suppress paired client toggles: {assembled:?}"
            );
            assert!(
                !assembled.iter().any(|arg| {
                    matches!(
                        arg.as_str(),
                        "--enable=rollout_budget" | "--disable=rollout_budget"
                    )
                }),
                "typed budget must suppress equals-form client toggles: {assembled:?}"
            );
            assert!(
                assembled
                    .windows(2)
                    .any(|pair| pair == ["--enable", "keep-enabled"])
            );
            assert!(
                assembled
                    .windows(2)
                    .any(|pair| pair == ["--disable", "keep-disabled"])
            );
        }
    }

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
    /// new dependency and `tracing-subscriber` would be one.
    ///
    /// Installed once as the **global** default, fanning out to a per-thread
    /// sink. A thread-local `set_default` is not enough: tracing caches
    /// callsite interest globally, so a test running concurrently against no
    /// subscriber caches this crate's span callsites as `never` and the
    /// recording test then sees nothing. That produced a real flake, about one
    /// run in three. A single always-enabled global subscriber keeps interest
    /// stable, and the per-thread sink keeps tests isolated.
    #[cfg(unix)]
    mod recorder {
        use std::cell::RefCell;
        use std::sync::{Arc, Mutex, Once};

        use tracing::field::{Field, Visit};
        use tracing::span::{Attributes, Id, Record};
        use tracing::{Event, Metadata, Subscriber};

        type Sink = Arc<Mutex<Vec<(String, String)>>>;

        thread_local! {
            static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        }

        struct Global;

        impl Global {
            fn collect(f: impl FnOnce(&mut Vec<(String, String)>)) {
                SINK.with(|sink| {
                    if let Some(sink) = sink.borrow().as_ref() {
                        f(&mut sink.lock().unwrap());
                    }
                });
            }
        }

        impl Subscriber for Global {
            /// Always true, so callsite interest is never cached as `never`.
            fn enabled(&self, _: &Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, attrs: &Attributes<'_>) -> Id {
                Self::collect(|fields| attrs.record(&mut Collect(fields)));
                Id::from_u64(1)
            }
            fn record(&self, _: &Id, values: &Record<'_>) {
                Self::collect(|fields| values.record(&mut Collect(fields)));
            }
            fn record_follows_from(&self, _: &Id, _: &Id) {}
            fn event(&self, _: &Event<'_>) {}
            fn enter(&self, _: &Id) {}
            fn exit(&self, _: &Id) {}
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

        pub(super) struct Recorder(Sink);

        impl Recorder {
            /// Start recording spans raised on this thread.
            pub(super) fn install() -> Self {
                static INIT: Once = Once::new();
                INIT.call_once(|| {
                    let _ = tracing::subscriber::set_global_default(Global);
                });
                let sink: Sink = Arc::new(Mutex::new(Vec::new()));
                SINK.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&sink)));
                Self(sink)
            }

            pub(super) fn dump(&self) -> String {
                format!("{:?}", self.0.lock().unwrap())
            }

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

        impl Drop for Recorder {
            fn drop(&mut self) {
                SINK.with(|slot| *slot.borrow_mut() = None);
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn span_records_the_subcommand_and_a_clean_outcome() {
        let recorder = recorder::Recorder::install();

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
        let recorder = recorder::Recorder::install();

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
        let recorder = recorder::Recorder::install();

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

        assert_eq!(
            recorder.value("outcome").as_deref(),
            Some("cancelled"),
            "recorded: {}",
            recorder.dump()
        );
    }

    /// A wrapper timeout is its own outcome, distinct from a caller
    /// cancelling: the run reached its deadline rather than being abandoned.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_run_is_recorded_as_timeout() {
        let recorder = recorder::Recorder::install();

        let pid_file = crate::test_support::PidFile::new("span-timeout");
        let codex = crate::test_support::blocking_codex(&pid_file)
            .timeout(Duration::from_millis(300))
            .build()
            .expect("bash must exist");

        let result = run_codex(&codex, vec!["exec".into()]).await;
        assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");

        assert_eq!(recorder.value("outcome").as_deref(), Some("timeout"));
    }

    // -----------------------------------------------------------------
    // Classification end to end (#85)
    // -----------------------------------------------------------------

    #[cfg(unix)]
    fn failing_codex(case: &str) -> Codex {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-failure.sh");
        Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .env("CODEX_WRAPPER_TEST_FAILURE", case)
            .build()
            .expect("bash must exist")
    }

    /// Classification has to happen on the spawn path, not only in the
    /// constructor, or a caller still gets a bare CommandFailed.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_real_spawn_returns_a_classified_error() {
        use crate::error::FailureKind;

        for (case, expected) in [
            ("auth", FailureKind::Auth),
            ("not-trusted", FailureKind::NotTrustedDirectory),
            ("config", FailureKind::Config),
            ("session", FailureKind::SessionNotFound),
            ("mystery", FailureKind::Unclassified),
        ] {
            let codex = failing_codex(case);
            let err = run_codex(&codex, vec!["exec".into()]).await.unwrap_err();
            assert_eq!(err.failure_kind(), Some(expected), "case {case}: {err}");
        }
    }

    /// A deterministic failure must not be retried even when its exit code is
    /// on the retry list. Re-running gets the same rejection, and the auth
    /// case has already been retried inside the CLI before it reaches here.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_classified_failure_is_not_retried() {
        let policy = crate::retry::RetryPolicy::new()
            .max_attempts(3)
            .initial_backoff(Duration::from_millis(1))
            .retry_on_exit_codes([1]);

        let started = Instant::now();
        let codex = failing_codex("auth");
        let err = run_codex_with_retry(&codex, vec!["exec".into()], Some(&policy))
            .await
            .unwrap_err();

        assert!(matches!(err, Error::Auth { .. }), "{err}");
        // Three attempts with backoff would be visibly slower; this asserts
        // the shape rather than the timing, but the timing corroborates it.
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "looks like it retried: {:?}",
            started.elapsed()
        );
    }

    /// An unclassified failure keeps the old retry behaviour.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unclassified_failure_still_retries() {
        let policy = crate::retry::RetryPolicy::new()
            .max_attempts(2)
            .initial_backoff(Duration::from_millis(1))
            .retry_on_exit_codes([1]);

        let codex = failing_codex("mystery");
        let err = run_codex_with_retry(&codex, vec!["exec".into()], Some(&policy))
            .await
            .unwrap_err();

        assert!(matches!(err, Error::CommandFailed { .. }), "{err}");
    }

    /// The allow-list works on the exit code, so it still applies to a
    /// failure that classification moved off CommandFailed.
    #[cfg(unix)]
    #[tokio::test]
    async fn allowed_exit_codes_still_apply_to_a_classified_failure() {
        let codex = failing_codex("auth");
        let output = run_codex_allow_exit_codes(&codex, vec!["exec".into()], &[1])
            .await
            .expect("exit code 1 was allowed");

        assert_eq!(output.exit_code, 1);
        assert!(!output.success);
        assert!(
            output.stderr.contains("401 Unauthorized"),
            "{}",
            output.stderr
        );
    }

    // -----------------------------------------------------------------
    // Process groups and cancellation (#78)
    // -----------------------------------------------------------------

    /// A fake codex that spawns a child of its own, the way the real CLI does
    /// for tool use, and the pids of both.
    #[cfg(unix)]
    fn spawning_codex(label: &str) -> (Codex, crate::test_support::PidFile) {
        let pid_file = crate::test_support::PidFile::new(label);
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-spawns-child.sh");
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .env(
                "CODEX_WRAPPER_TEST_PIDFILE",
                pid_file.path().to_str().unwrap(),
            )
            .build()
            .expect("bash must exist");
        (codex, pid_file)
    }

    #[cfg(unix)]
    async fn read_pids(pid_file: &crate::test_support::PidFile) -> (u32, u32) {
        for _ in 0..200 {
            if let Ok(contents) = std::fs::read_to_string(pid_file.path()) {
                let parse = |prefix: &str| -> Option<u32> {
                    contents
                        .lines()
                        .find_map(|l| l.strip_prefix(prefix))
                        .and_then(|v| v.trim().parse().ok())
                };
                if let (Some(parent), Some(child)) = (parse("parent="), parse("child=")) {
                    return (parent, child);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake codex never recorded both pids");
    }

    /// The point of #78. `kill_on_drop` reaps the direct child only, so before
    /// process groups the grandchild outlived a cancelled run.
    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_kills_the_whole_process_group() {
        use crate::test_support::wait_until_gone;

        let (codex, pid_file) = spawning_codex("group-drop");

        let cancelled = tokio::time::timeout(
            Duration::from_millis(400),
            run_codex(&codex, vec!["exec".into()]),
        )
        .await;
        assert!(cancelled.is_err(), "the run should still have been going");

        let (parent, child) = read_pids(&pid_file).await;
        assert!(wait_until_gone(parent).await, "codex ({parent}) survived");
        assert!(
            wait_until_gone(child).await,
            "the subprocess ({child}) survived the cancelled run"
        );
    }

    /// The graceful path: SIGTERM, a grace period, then SIGKILL. It cannot
    /// live in `Drop`, which is why there is an explicit entry point.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_codex_cancellable_stops_the_group_gracefully() {
        use crate::test_support::wait_until_gone;

        let (codex, pid_file) = spawning_codex("group-cancel");
        let codex = Codex::builder()
            .binary(codex.binary())
            .arg(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests")
                    .join("fake-codex-spawns-child.sh")
                    .to_str()
                    .unwrap(),
            )
            .env(
                "CODEX_WRAPPER_TEST_PIDFILE",
                pid_file.path().to_str().unwrap(),
            )
            .termination_grace(Duration::from_millis(50))
            .build()
            .unwrap();

        let cancel = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        let result = run_codex_cancellable(&codex, vec!["exec".into()], cancel).await;

        assert!(
            matches!(result, Err(Error::Cancelled { .. })),
            "expected a cancellation, got: {result:?}"
        );

        let (parent, child) = read_pids(&pid_file).await;
        assert!(wait_until_gone(parent).await, "codex ({parent}) survived");
        assert!(
            wait_until_gone(child).await,
            "the subprocess ({child}) survived cancellation"
        );
    }

    /// A run that finishes before the signal must not be reported as
    /// cancelled, and must not be killed on the way out.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_that_finishes_first_is_not_cancelled() {
        let codex = Codex::builder()
            .binary("/bin/echo")
            .build()
            .expect("echo must exist");

        let never = std::future::pending::<()>();
        let output = run_codex_cancellable(&codex, vec!["exec".into()], never)
            .await
            .unwrap();
        assert!(output.success);
    }

    /// Opting out is not cosmetic: without a group of its own, cancelling
    /// reaches the direct child only and its subprocesses survive. That is the
    /// terminal-attached contract, where the terminal is the supervisor and
    /// Ctrl-C reaches the whole run directly instead.
    #[cfg(unix)]
    #[tokio::test]
    async fn opting_out_of_process_groups_leaves_the_subprocess() {
        use crate::test_support::wait_until_gone;

        let pid_file = crate::test_support::PidFile::new("group-optout");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-spawns-child.sh");
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .env(
                "CODEX_WRAPPER_TEST_PIDFILE",
                pid_file.path().to_str().unwrap(),
            )
            .process_group(false)
            .build()
            .expect("bash must exist");

        let cancelled = tokio::time::timeout(
            Duration::from_millis(400),
            run_codex(&codex, vec!["exec".into()]),
        )
        .await;
        assert!(cancelled.is_err(), "the run should still have been going");

        let (parent, child) = read_pids(&pid_file).await;
        assert!(
            wait_until_gone(parent).await,
            "kill_on_drop still reaps the direct child ({parent})"
        );
        // The point of the contrast with
        // `cancelling_kills_the_whole_process_group`.
        assert!(
            crate::test_support::is_running_for_test(child),
            "with groups off, the subprocess ({child}) is expected to survive"
        );
        // Do not leave it behind.
        signal_group(child, libc::SIGKILL);
        unsafe { libc::kill(i32::try_from(child).unwrap_or(0), libc::SIGKILL) };
    }
}
