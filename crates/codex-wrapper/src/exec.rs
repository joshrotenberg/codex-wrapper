//! Process execution layer for spawning and communicating with the `codex`
//! binary, including timeout and retry support.

use std::fmt;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tracing::{Instrument, Span, debug, field, info_span};

use crate::Codex;
use crate::error::{Error, OutputStream, Result};

/// Build the span covering one invocation.
///
/// The fields carry what identifies a run, never what it contains: no prompt
/// and no environment. Both routinely hold content a host would not want in
/// its logs, and a span is recorded whenever any subscriber is installed.
pub(crate) fn command_span(name: &'static str, codex: &Codex, args: &[String]) -> Span {
    info_span!(
        parent: Span::current(),
        "codex",
        otel.name = name,
        subcommand = trace_subcommand(args),
        arg_count = args.len(),
        working_dir_set = codex.working_dir.is_some(),
        outcome = field::Empty,
        exit_code = field::Empty,
        duration_ms = field::Empty,
    )
}

/// RawCommand permits arbitrary first tokens; only fixed operation labels may
/// enter diagnostics. Counts retain observability for future/unknown commands.
fn trace_subcommand(args: &[String]) -> &'static str {
    match args.first().map(String::as_str) {
        None => "(none)",
        Some("exec") => "exec",
        Some("review") => "review",
        Some("login") => "login",
        Some("logout") => "logout",
        Some("mcp") => "mcp",
        Some("mcp-server") => "mcp-server",
        Some("app-server") => "app-server",
        Some("completion") => "completion",
        Some("sandbox") => "sandbox",
        Some("apply") => "apply",
        Some("resume") => "resume",
        Some("fork") => "fork",
        Some("features") => "features",
        Some("plugin") => "plugin",
        Some("doctor") => "doctor",
        Some("update") => "update",
        Some(_) => "other",
    }
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
            // Its own outcome so the span never reads as a settled timeout or
            // cancellation when the child may not have been reaped.
            Err(Error::CleanupFailed { .. }) => self.settle("cleanup_failed", None),
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

/// Put the child in its own process group so signals also reach descendants
/// that stay in that group. Tools that detach require cooperative CLI cleanup;
/// `kill_on_drop` alone only reaches the direct child.
#[cfg(unix)]
pub(crate) fn own_process_group(cmd: &mut Command, enabled: bool) {
    if enabled {
        cmd.process_group(0);
    }
}

#[cfg(not(unix))]
pub(crate) fn own_process_group(_cmd: &mut Command, _enabled: bool) {}

/// Whether [`CodexBuilder::die_with_parent`](crate::CodexBuilder::die_with_parent)
/// has a kernel-level effect on this platform.
///
/// This is true only on Linux, where `PR_SET_PDEATHSIG` is available. A
/// supervisor that needs portable containment should use
/// [`CodexBuilder::on_spawn`](crate::CodexBuilder::on_spawn) to maintain its
/// own watchdog on other targets.
#[must_use]
pub const fn die_with_parent_supported() -> bool {
    cfg!(target_os = "linux")
}

/// Ask the kernel to SIGKILL the child when its immediate parent dies.
///
/// The parent pid is captured before the fork. After arming the signal in the
/// child, the hook checks it again and exits if the parent changed in the
/// fork-to-prctl window. Every call here is async-signal-safe because this hook
/// runs after `fork` and before `exec`.
#[cfg(unix)]
fn pdeathsig_hook() -> impl FnMut() -> std::io::Result<()> + Send + Sync + 'static {
    let parent = std::process::id();
    move || {
        #[cfg(target_os = "linux")]
        {
            // SAFETY: prctl, getppid, and _exit are async-signal-safe calls.
            unsafe {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() as u32 != parent {
                    libc::_exit(1);
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = parent;
        }
        Ok(())
    }
}

/// Apply the parent-death policy to a spawn command.
pub(crate) fn apply_die_with_parent(cmd: &mut Command, enabled: bool) {
    #[cfg(unix)]
    if enabled {
        // SAFETY: `pdeathsig_hook` uses only async-signal-safe operations.
        unsafe {
            cmd.pre_exec(pdeathsig_hook());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (cmd, enabled);
    }
}

/// Arm group cleanup and notify the observer from one process receipt.
pub(crate) fn arm_and_notify(
    process_group: bool,
    termination_signal: crate::TerminationSignal,
    pid: Option<u32>,
    on_spawn: Option<&crate::SpawnObserver>,
) -> GroupKillGuard {
    if let (Some(pid), Some(observer)) = (pid, on_spawn) {
        observer(crate::SpawnInfo {
            pid,
            pgid: process_group.then_some(pid),
        });
    }
    GroupKillGuard::new(process_group.then_some(pid).flatten(), termination_signal)
}

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
    // SAFETY: `kill` with a negative pid signals a process group. Callers must
    // retain ownership of the group leader until their final signal is sent.
    unsafe {
        libc::kill(-pid, signal);
    }
}

/// SIGKILLs the run's process group when dropped.
///
/// `kill_on_drop` handles the direct child; this is what reaches the
/// descendants that remain in the group. Detached tools require CLI cooperation.
/// Drop cannot await, so this is the abrupt path.
/// For a graceful one, see
/// [`ExecCommand::execute_cancellable`](crate::ExecCommand::execute_cancellable).
pub(crate) struct GroupKillGuard {
    pid: Option<u32>,
    signal: crate::TerminationSignal,
}

impl GroupKillGuard {
    pub(crate) fn new(pid: Option<u32>, signal: crate::TerminationSignal) -> Self {
        Self { pid, signal }
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
        let Some(pid) = self.pid else {
            return;
        };
        signal_group(pid, self.signal.as_raw());
        // Keep drop cleanup armed while this await can be cancelled. Once
        // SIGKILL is sent, disarm before the caller can reap the child.
        tokio::time::sleep(grace).await;
        signal_group(pid, libc::SIGKILL);
        self.disarm();
    }

    /// No process groups here, so there is nothing to ask politely. The
    /// child still dies with the dropped future via `kill_on_drop`.
    #[cfg(not(unix))]
    pub(crate) async fn terminate(&mut self, _grace: Duration) {
        let _ = (self.pid.take(), self.signal);
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
                subcommand = trace_subcommand(&args),
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
        debug!(arg_count = command_args.len(), "executing codex command");

        let mut outcome = SpanOutcome::start(outcome_span);
        let result = match codex.timeout {
            Some(timeout) => {
                run_internal_inner(
                    SpawnSpec {
                        binary: &codex.binary,
                        args: &command_args,
                        env: &codex.env,
                        clear_env: codex.clear_env,
                        working_dir: codex.working_dir.as_deref(),
                        stdin_prompt: None,
                        process_group: codex.process_group,
                        termination_signal: codex.termination_signal,
                        output_limit: codex.output_limit,
                        die_with_parent: codex.die_with_parent,
                        on_spawn: codex.on_spawn.as_ref(),
                    },
                    Some(timeout_stop(timeout)),
                    codex.termination_grace,
                )
                .await
            }
            None => {
                run_internal_inner(
                    SpawnSpec {
                        binary: &codex.binary,
                        args: &command_args,
                        env: &codex.env,
                        clear_env: codex.clear_env,
                        working_dir: codex.working_dir.as_deref(),
                        stdin_prompt: None,
                        process_group: codex.process_group,
                        termination_signal: codex.termination_signal,
                        output_limit: codex.output_limit,
                        die_with_parent: codex.die_with_parent,
                        on_spawn: codex.on_spawn.as_ref(),
                    },
                    None,
                    Duration::ZERO,
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
/// On cancellation the run's process group is sent the configured signal,
/// given the client's [`termination_grace`](crate::CodexBuilder::termination_grace),
/// then killed.
/// This reaches descendants in the owned group. Detached tools require the
/// CLI's cooperation; [`TerminationSignal::Interrupt`](crate::TerminationSignal::Interrupt)
/// lets its interrupt handler run before the hard fallback. Output during
/// shutdown is drained and discarded, and the cancellation reason stays latched
/// even if the CLI exits successfully. If stopping or reaping the child fails,
/// the stop reason is returned inside [`Error::CleanupFailed`] rather than as a
/// bare [`Error::Cancelled`] or [`Error::Timeout`].
///
/// Dropping the future instead kills the owned group abruptly: `Drop` cannot
/// wait out a grace period or allow cooperative cleanup of detached tools.
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
        debug!(
            arg_count = command_args.len(),
            "executing cancellable codex command"
        );

        let mut outcome = SpanOutcome::start(outcome_span);
        let stop = cancellation_or_timeout(cancel, codex.timeout, codex.termination_grace);
        let result = run_internal_inner(
            SpawnSpec {
                binary: &codex.binary,
                args: &command_args,
                env: &codex.env,
                clear_env: codex.clear_env,
                working_dir: codex.working_dir.as_deref(),
                stdin_prompt: None,
                process_group: codex.process_group,
                termination_signal: codex.termination_signal,
                output_limit: codex.output_limit,
                die_with_parent: codex.die_with_parent,
                on_spawn: codex.on_spawn.as_ref(),
            },
            Some(stop),
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
            arg_count = command_args.len(),
            prompt_bytes = prompt.len(),
            "executing codex command with a stdin prompt"
        );

        let mut outcome = SpanOutcome::start(outcome_span);
        let stop = codex.timeout.map(timeout_stop);
        let result = run_internal_inner(
            SpawnSpec {
                binary: &codex.binary,
                args: &command_args,
                env: &codex.env,
                clear_env: codex.clear_env,
                working_dir: codex.working_dir.as_deref(),
                stdin_prompt: Some(prompt),
                process_group: codex.process_group,
                termination_signal: codex.termination_signal,
                output_limit: codex.output_limit,
                die_with_parent: codex.die_with_parent,
                on_spawn: codex.on_spawn.as_ref(),
            },
            stop,
            codex.termination_grace,
        )
        .await;
        outcome.settle_from(&result);
        result
    }
    .instrument(span)
    .await
}

/// Run a codex command with a stdin prompt and an explicit cancellation signal.
///
/// Cancellation and the client's timeout both terminate the owned process
/// group and await the direct child before returning. Retry does not apply.
pub async fn run_codex_with_stdin_prompt_cancellable<C>(
    codex: &Codex,
    args: Vec<String>,
    prompt: &str,
    cancel: C,
) -> Result<CommandOutput>
where
    C: std::future::Future<Output = ()> + Send,
{
    let span = command_span("codex.exec", codex, &args);
    let outcome_span = span.clone();
    let command_args = assemble_args(codex, args);

    async move {
        debug!(
            arg_count = command_args.len(),
            prompt_bytes = prompt.len(),
            "executing cancellable codex command with a stdin prompt"
        );

        let mut outcome = SpanOutcome::start(outcome_span);
        let stop = cancellation_or_timeout(cancel, codex.timeout, codex.termination_grace);
        let result = run_internal_inner(
            SpawnSpec {
                binary: &codex.binary,
                args: &command_args,
                env: &codex.env,
                clear_env: codex.clear_env,
                working_dir: codex.working_dir.as_deref(),
                stdin_prompt: Some(prompt),
                process_group: codex.process_group,
                termination_signal: codex.termination_signal,
                output_limit: codex.output_limit,
                die_with_parent: codex.die_with_parent,
                on_spawn: codex.on_spawn.as_ref(),
            },
            Some(stop),
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

#[derive(Clone, Copy, Debug)]
enum StopReason {
    Cancelled { grace_seconds: u64 },
    Timeout { timeout_seconds: u64 },
}

impl StopReason {
    fn into_error(self) -> Error {
        match self {
            Self::Cancelled { grace_seconds } => Error::Cancelled { grace_seconds },
            Self::Timeout { timeout_seconds } => Error::Timeout { timeout_seconds },
        }
    }
}

type StopFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = StopReason> + Send + 'a>>;

fn timeout_stop(timeout: Duration) -> StopFuture<'static> {
    Box::pin(async move {
        tokio::time::sleep(timeout).await;
        StopReason::Timeout {
            timeout_seconds: timeout.as_secs(),
        }
    })
}

fn cancellation_or_timeout<'a, C>(
    cancel: C,
    timeout: Option<Duration>,
    grace: Duration,
) -> StopFuture<'a>
where
    C: std::future::Future<Output = ()> + Send + 'a,
{
    Box::pin(async move {
        match timeout {
            Some(timeout) => tokio::select! {
                () = cancel => StopReason::Cancelled {
                    grace_seconds: grace.as_secs(),
                },
                () = tokio::time::sleep(timeout) => StopReason::Timeout {
                    timeout_seconds: timeout.as_secs(),
                },
            },
            None => {
                cancel.await;
                StopReason::Cancelled {
                    grace_seconds: grace.as_secs(),
                }
            }
        }
    })
}

/// Everything one spawn needs, gathered so the signature stays readable.
struct SpawnSpec<'a> {
    binary: &'a std::path::Path,
    args: &'a [String],
    env: &'a std::collections::HashMap<String, String>,
    /// Whether the child starts without the wrapper's ambient environment.
    clear_env: bool,
    working_dir: Option<&'a std::path::Path>,
    /// A prompt to deliver on stdin, for `codex exec -`.
    stdin_prompt: Option<&'a str>,
    /// Whether the run leads its own process group.
    process_group: bool,
    termination_signal: crate::TerminationSignal,
    /// Stop capturing a stream once it passes this many bytes.
    output_limit: Option<usize>,
    /// Whether the child should die with its immediate parent on Linux.
    die_with_parent: bool,
    /// Who to notify immediately after a successful spawn.
    on_spawn: Option<&'a crate::SpawnObserver>,
}

/// Read one captured stream to EOF, or stop once it passes `limit`.
///
/// With no limit this is `read_to_end`, unchanged. With one, the buffer never
/// exceeds the ceiling, so a run holds at most twice it across the two
/// streams no matter what the child prints.
///
/// Exceeding it is an error rather than a truncated success. A prefix
/// returned as though it were the whole result would be indistinguishable
/// from a short answer, which is the failure worth preventing. Enough of the
/// prefix to identify the runaway is logged instead.
async fn capture<R>(
    reader: &mut R,
    limit: Option<usize>,
    stream: OutputStream,
    working_dir: Option<&std::path::Path>,
) -> Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let io_error = |e: std::io::Error| Error::Io {
        message: format!("failed to read codex {stream}: {e}"),
        source: e,
        working_dir: working_dir.map(std::path::Path::to_path_buf),
    };

    let Some(limit) = limit else {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(io_error)?;
        return Ok(bytes);
    };

    let mut bytes = Vec::new();
    let mut chunk = [0u8; CAPTURE_CHUNK];
    loop {
        let read = reader.read(&mut chunk).await.map_err(io_error)?;
        if read == 0 {
            return Ok(bytes);
        }
        if bytes.len() + read > limit {
            tracing::warn!(
                %stream,
                limit_bytes = limit,
                captured_bytes = bytes.len(),
                "codex output exceeded its capture limit; stopping the run"
            );
            return Err(Error::OutputLimitExceeded {
                stream,
                limit_bytes: limit,
            });
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
}

/// Read size for a bounded capture. Large enough not to syscall per line,
/// small enough that the overshoot before noticing stays trivial.
const CAPTURE_CHUNK: usize = 8 * 1024;

async fn run_internal_inner(
    spec: SpawnSpec<'_>,
    stop: Option<StopFuture<'_>>,
    grace: Duration,
) -> Result<CommandOutput> {
    let SpawnSpec {
        binary,
        args,
        env,
        clear_env,
        working_dir,
        stdin_prompt,
        process_group,
        termination_signal,
        output_limit,
        die_with_parent,
        on_spawn,
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
    apply_die_with_parent(&mut cmd, die_with_parent);

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    apply_child_environment(&mut cmd, clear_env, env);

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
    // group, which also reaches descendants still in that group.
    // Only meaningful when the run leads its own group. Sharing the parent's
    // means signalling it would hit the parent too.
    let mut group = arm_and_notify(process_group, termination_signal, child.id(), on_spawn);
    let child_stdin = child.stdin.take();
    let mut child_stdout = child.stdout.take().expect("stdout was configured as piped");
    let mut child_stderr = child.stderr.take().expect("stderr was configured as piped");

    let write = async move {
        let (Some(prompt), Some(mut stdin)) = (stdin_prompt, child_stdin) else {
            return Ok(());
        };
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| Error::Io {
                message: format!("failed to write the prompt to codex stdin: {e}"),
                source: e,
                working_dir: working_dir.map(|p| p.to_path_buf()),
            })?;
        // Closing the write half is what tells the CLI the prompt is
        // complete. Without it the child waits for more input.
        stdin.shutdown().await.map_err(|e| Error::Io {
            message: format!("failed to close codex stdin: {e}"),
            source: e,
            working_dir: working_dir.map(|p| p.to_path_buf()),
        })
    };

    // Concurrently, not sequentially: a prompt larger than the pipe buffer
    // blocks until the child reads it, and a child that writes to stdout
    // meanwhile blocks until we read that. Waiting for the write to finish
    // before draining stdout would deadlock both.
    let read_stdout = async {
        capture(
            &mut child_stdout,
            output_limit,
            OutputStream::Stdout,
            working_dir,
        )
        .await
    };
    let read_stderr = async {
        capture(
            &mut child_stderr,
            output_limit,
            OutputStream::Stderr,
            working_dir,
        )
        .await
    };
    let run = async {
        let ((), stdout, stderr) = tokio::try_join!(write, read_stdout, read_stderr)?;
        // Do not reap while pipes are still open: if cancellation wins then,
        // delayed group signaling must not target a recycled group-leader PID.
        let status = child.wait().await.map_err(|e| wait_error(e, working_dir))?;
        Ok::<_, Error>((stdout, stderr, status))
    };

    let finished = match stop {
        None => Ok(run.await),
        // Racing the run against the caller's signal. Cancellation here is
        // graceful, which is the whole reason it cannot live in `Drop`:
        // asking a process to stop and then waiting requires awaiting.
        Some(stop) => tokio::select! {
            outcome = run => Ok(outcome),
            reason = stop => Err(reason),
        },
    };

    let (stdout, stderr, status) = match finished {
        Ok(Ok(finished)) => finished,
        Ok(Err(error)) => {
            // A stdin or pipe failure can happen while the child continues
            // running. Treat it as a stop request and settle ownership before
            // exposing the I/O error.
            let cleanup = terminate_and_reap_draining(
                &mut child,
                &mut group,
                grace,
                working_dir,
                &mut child_stdout,
                &mut child_stderr,
            )
            .await;
            return Err(Error::after_cleanup(error, cleanup));
        }
        Err(reason) => {
            // The run future is gone before this branch executes, returning
            // ownership of `child` so cleanup can explicitly reap it.
            // `GroupKillGuard::terminate` reaches same-group descendants. The
            // direct-child kill below is the portable fallback and makes the
            // wait authoritative on every platform.
            let cleanup = terminate_and_reap_draining(
                &mut child,
                &mut group,
                grace,
                working_dir,
                &mut child_stdout,
                &mut child_stderr,
            )
            .await;
            return Err(Error::after_cleanup(reason.into_error(), cleanup));
        }
    };

    // Finished on its own, so there is no group left to kill.
    group.disarm();

    let stdout = String::from_utf8_lossy(&stdout).to_string();
    let stderr = String::from_utf8_lossy(&stderr).to_string();
    let exit_code = status.code().unwrap_or(-1);

    if !status.success() {
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

/// Keep both pipes open and flowing while shutdown runs. Shutdown output is
/// discarded in fixed-size chunks, never parsed or sent to event handlers.
/// The drain is dropped when cleanup finishes even if an escaped descendant
/// still holds a pipe open. No child wait is polled before group signaling ends.
pub(crate) async fn terminate_and_reap_draining<O, E>(
    child: &mut tokio::process::Child,
    group: &mut GroupKillGuard,
    grace: Duration,
    working_dir: Option<&std::path::Path>,
    stdout: &mut O,
    stderr: &mut E,
) -> Result<()>
where
    O: tokio::io::AsyncRead + Unpin,
    E: tokio::io::AsyncRead + Unpin,
{
    let drain = async {
        let mut stdout_sink = tokio::io::sink();
        let mut stderr_sink = tokio::io::sink();
        let _ = tokio::join!(
            tokio::io::copy(stdout, &mut stdout_sink),
            tokio::io::copy(stderr, &mut stderr_sink),
        );
        std::future::pending::<()>().await;
    };
    let result = tokio::select! {
        result = terminate_and_reap(child, group, grace, working_dir) => result,
        () = drain => unreachable!("the drain waits until shutdown finishes"),
    };
    #[cfg(test)]
    let result = result.and_then(|()| injected_cleanup_failure(working_dir));
    if result.is_err() {
        tracing::warn!("codex cleanup failed; the child may not have been reaped");
    }
    result
}

#[cfg(test)]
thread_local! {
    static INJECT_CLEANUP_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test-only fault injection for [`terminate_and_reap_draining`].
///
/// While armed, a cleanup on this thread that actually succeeded reports a
/// synthetic I/O error instead. The real termination and wait still run first,
/// so the fixture child is reaped either way. This exercises how callers
/// report a failed cleanup; it does not show that any syscall can fail.
/// Thread-local because `#[tokio::test]` runs each test on its own
/// current-thread runtime, which keeps concurrent tests isolated. Every caller
/// is a unix-only test, hence the `dead_code` allowance elsewhere.
#[cfg(test)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) struct CleanupFailureInjection(std::marker::PhantomData<std::rc::Rc<()>>);

#[cfg(test)]
#[cfg_attr(not(unix), allow(dead_code))]
impl CleanupFailureInjection {
    pub(crate) fn arm() -> Self {
        INJECT_CLEANUP_FAILURE.with(|armed| armed.set(true));
        // The thread-local guard must stay on its originating test thread.
        Self(std::marker::PhantomData)
    }
}

#[cfg(test)]
impl Drop for CleanupFailureInjection {
    fn drop(&mut self) {
        INJECT_CLEANUP_FAILURE.with(|armed| armed.set(false));
    }
}

#[cfg(test)]
fn injected_cleanup_failure(working_dir: Option<&std::path::Path>) -> Result<()> {
    if INJECT_CLEANUP_FAILURE.with(std::cell::Cell::get) {
        return Err(wait_error(
            std::io::Error::other("injected cleanup failure"),
            working_dir,
        ));
    }
    Ok(())
}

pub(crate) async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    group: &mut GroupKillGuard,
    grace: Duration,
    working_dir: Option<&std::path::Path>,
) -> Result<()> {
    group.terminate(grace).await;
    if child
        .try_wait()
        .map_err(|e| wait_error(e, working_dir))?
        .is_none()
        && let Err(error) = child.start_kill()
        && error.kind() != std::io::ErrorKind::InvalidInput
    {
        return Err(wait_error(error, working_dir));
    }
    child.wait().await.map_err(|e| wait_error(e, working_dir))?;
    Ok(())
}

fn wait_error(error: std::io::Error, working_dir: Option<&std::path::Path>) -> Error {
    Error::Io {
        message: format!("failed to wait on codex: {error}"),
        source: error,
        working_dir: working_dir.map(|p| p.to_path_buf()),
    }
}

/// Apply the client environment policy to a direct child process.
///
/// Kept in one helper because buffered and streaming execution construct
/// separate commands. Clearing must happen before explicit values are added.
pub(crate) fn apply_child_environment(
    cmd: &mut Command,
    clear_env: bool,
    env: &std::collections::HashMap<String, String>,
) {
    if clear_env {
        cmd.env_clear();
    }
    cmd.envs(env);
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

    /// Compatibility is explicit: callers that do not opt in keep the
    /// ambient environment they relied on before `clear_env` existed.
    #[cfg(unix)]
    #[tokio::test]
    async fn child_environment_is_inherited_by_default() {
        let capture = crate::test_support::EnvCapture::new("env-default");
        let codex = crate::test_support::env_capturing_codex(&capture)
            .build()
            .expect("bash must exist");

        crate::ExecCommand::new("probe")
            .execute(&codex)
            .await
            .unwrap();

        let environment = capture.read();
        assert_eq!(
            environment.get("PATH"),
            Some(&std::env::var("PATH").expect("test process must have PATH"))
        );
    }

    /// The timeout and ordinary buffered paths use different wrapper entry
    /// points. Opening and resume both have to reach the same environment
    /// policy, and explicit values on either side of `clear_env` must survive.
    #[cfg(unix)]
    #[tokio::test]
    async fn cleared_environment_reaches_buffered_open_and_resume() {
        let capture = crate::test_support::EnvCapture::new("env-buffered");
        let opening_client = crate::test_support::env_capturing_codex(&capture)
            .clear_env()
            .env("CODEX_WRAPPER_EXPLICIT", "opening")
            .timeout(Duration::from_secs(2))
            .build()
            .expect("bash must exist");

        crate::ExecCommand::new("probe")
            .execute(&opening_client)
            .await
            .unwrap();
        let opening_environment = capture.read();
        assert!(!opening_environment.contains_key("PATH"));
        assert_eq!(
            opening_environment
                .get("CODEX_WRAPPER_EXPLICIT")
                .map(String::as_str),
            Some("opening")
        );
        assert!(opening_environment.contains_key("CODEX_WRAPPER_ENV_CAPTURE"));

        let resume_client = crate::test_support::env_capturing_codex(&capture)
            .clear_env()
            .env("CODEX_WRAPPER_EXPLICIT", "resume")
            .build()
            .expect("bash must exist");
        crate::ExecResumeCommand::new()
            .last()
            .execute(&resume_client)
            .await
            .unwrap();
        let resume_environment = capture.read();
        assert!(!resume_environment.contains_key("PATH"));
        assert_eq!(
            resume_environment
                .get("CODEX_WRAPPER_EXPLICIT")
                .map(String::as_str),
            Some("resume")
        );
    }

    /// Stdin delivery and graceful cancellation each construct their own
    /// spawn specification, so cover both instead of treating the buffered
    /// test as proof for them.
    #[cfg(unix)]
    #[tokio::test]
    async fn cleared_environment_reaches_stdin_and_cancellable_runs() {
        let capture = crate::test_support::EnvCapture::new("env-specialized");
        let codex = crate::test_support::env_capturing_codex(&capture)
            .env("CODEX_WRAPPER_EXPLICIT", "specialized")
            .clear_env()
            .build()
            .expect("bash must exist");

        crate::ExecCommand::new("stdin prompt")
            .prompt_via_stdin()
            .execute(&codex)
            .await
            .unwrap();
        let stdin_environment = capture.read();
        assert!(!stdin_environment.contains_key("PATH"));
        assert_eq!(
            stdin_environment
                .get("CODEX_WRAPPER_EXPLICIT")
                .map(String::as_str),
            Some("specialized")
        );

        let never = std::future::pending::<()>();
        run_codex_cancellable(&codex, crate::ExecCommand::new("cancellable").args(), never)
            .await
            .unwrap();
        let cancellable_environment = capture.read();
        assert!(!cancellable_environment.contains_key("PATH"));
        assert_eq!(
            cancellable_environment
                .get("CODEX_WRAPPER_EXPLICIT")
                .map(String::as_str),
            Some("specialized")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn environment_values_do_not_leak_into_spawn_errors() {
        let secret = "spawn-error-must-not-leak-this";
        let codex = Codex::builder()
            .binary("/codex-wrapper/this-binary-does-not-exist")
            .clear_env()
            .env("CODEX_WRAPPER_SECRET", secret)
            .build()
            .unwrap();

        let error = run_codex(&codex, vec!["exec".into()])
            .await
            .expect_err("the fake path must not spawn");
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }

    /// A wrapper timeout is not terminal until the direct child is reaped.
    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_spawned_process() {
        use crate::test_support::{PidFile, blocking_codex, is_running_for_test};

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
            !is_running_for_test(pid),
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
            fn event(&self, event: &Event<'_>) {
                Self::collect(|fields| event.record(&mut Collect(fields)));
            }
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
    // Bounded capture (#132)
    // -----------------------------------------------------------------

    #[cfg(unix)]
    fn flooding_codex(stream: &str, limit: Option<usize>) -> Codex {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-floods.sh");
        let mut builder = Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .env("CODEX_WRAPPER_TEST_FLOOD_STREAM", stream);
        if let Some(max_bytes) = limit {
            builder = builder.output_limit(max_bytes);
        }
        builder.build().expect("bash must exist")
    }

    /// The ceiling has to hold on a single unbroken line, or it is really
    /// just a line-length check that a runaway one-line writer defeats.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stream_past_the_ceiling_fails_without_carrying_its_bytes() {
        for stream in ["stdout", "stderr"] {
            let codex = flooding_codex(stream, Some(64 * 1024));
            let error = crate::ExecCommand::new("hello")
                .execute(&codex)
                .await
                .expect_err("the flood exceeds the ceiling");

            let Error::OutputLimitExceeded {
                stream: reported,
                limit_bytes,
            } = error
            else {
                panic!("expected OutputLimitExceeded for {stream}, got {error:?}");
            };
            assert_eq!(reported.to_string(), stream);
            assert_eq!(limit_bytes, 64 * 1024);

            // The captured bytes must not ride along in the error, or the
            // ceiling has bounded nothing.
            let error = Error::OutputLimitExceeded {
                stream: reported,
                limit_bytes,
            };
            let rendered = format!("{error} {error:?}");
            assert!(!rendered.contains("xxxxxxxx"), "{rendered}");
            assert!(rendered.len() < 512, "error text grew with the output");
        }
    }

    /// Off by default, so adding the control changes nothing until a host
    /// asks for it. This is also the measurement of what the ceiling buys:
    /// the same child hands back a megabyte when nothing bounds it.
    #[cfg(unix)]
    #[tokio::test]
    async fn without_a_ceiling_the_whole_flood_is_captured() {
        let codex = flooding_codex("stdout", None);
        let result = crate::ExecCommand::new("hello")
            .execute(&codex)
            .await
            .expect("an unbounded capture succeeds, however large");
        assert_eq!(result.stdout.len(), 128 * 8192);
    }

    /// A run that stays under its ceiling is untouched.
    #[cfg(unix)]
    #[tokio::test]
    async fn output_within_the_ceiling_is_returned_whole() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-echo-stdin.sh");
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .output_limit(64 * 1024)
            .build()
            .expect("bash must exist");
        let result = crate::ExecCommand::new("a short prompt")
            .execute(&codex)
            .await
            .expect("well under the ceiling");
        // Whole means whole: the last line of the stream has to survive, not
        // just the first, or the ceiling is silently truncating.
        assert!(
            result.stdout.contains("thread.started"),
            "{}",
            result.stdout
        );
        assert!(
            result.stdout.contains("turn.completed"),
            "{}",
            result.stdout
        );
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
        use crate::test_support::is_running_for_test;

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
        assert!(!is_running_for_test(parent), "codex ({parent}) survived");
        assert!(
            !is_running_for_test(child),
            "the subprocess ({child}) survived cancellation"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_during_cancellation_grace_kills_the_group() {
        let fixture = crate::test_support::GracePeriodFixture::new("buffered-cancel-grace-drop");
        let codex = fixture.builder().build().unwrap();
        fixture
            .assert_drop_during_grace(run_codex_cancellable(
                &codex,
                vec!["exec".into()],
                fixture.ready(),
            ))
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_during_timeout_grace_kills_the_group() {
        let fixture = crate::test_support::GracePeriodFixture::new("buffered-timeout-grace-drop");
        let codex = fixture
            .builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        fixture
            .assert_drop_during_grace(run_codex(&codex, vec!["exec".into()]))
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn default_shutdown_still_sends_term_and_drains_both_pipes() {
        let fixture = crate::test_support::InterruptFixture::new("default-term-drain");
        let codex = fixture.builder().build().unwrap();
        let result =
            run_codex_cancellable(&codex, vec!["exec".into()], fixture.detached_ready()).await;
        assert!(matches!(result, Err(Error::Cancelled { .. })), "{result:?}");
        assert!(fixture.marker(".term").exists());
        assert!(!fixture.marker(".int").exists());
        assert!(
            fixture.marker(".cleaned").exists(),
            "shutdown output blocked cleanup"
        );
        fixture.assert_stopped().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn interrupt_cancellation_drains_large_shutdown_output_and_cleans_detached_tool() {
        let fixture = crate::test_support::InterruptFixture::new("buffered-interrupt-drain");
        let codex = fixture
            .builder()
            .termination_signal(crate::TerminationSignal::Interrupt)
            // The shutdown output must not be captured against this limit.
            .output_limit(64)
            .build()
            .unwrap();
        let result = run_codex_with_stdin_prompt_cancellable(
            &codex,
            vec!["exec".into(), "-".into()],
            "prompt",
            fixture.detached_ready(),
        )
        .await;
        assert!(matches!(result, Err(Error::Cancelled { .. })), "{result:?}");
        assert!(fixture.marker(".int").exists());
        assert!(!fixture.marker(".term").exists());
        assert!(
            fixture.marker(".cleaned").exists(),
            "shutdown output blocked cleanup"
        );
        fixture.assert_stopped().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn interrupt_timeout_keeps_timeout_error_after_cooperative_exit() {
        let fixture = crate::test_support::InterruptFixture::new("buffered-interrupt-timeout");
        let codex = fixture
            .builder()
            .termination_signal(crate::TerminationSignal::Interrupt)
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let run = run_codex(&codex, vec!["exec".into()]);
        tokio::pin!(run);
        tokio::select! {
            result = &mut run => panic!("run ended before readiness: {result:?}"),
            () = fixture.detached_ready() => {}
        }
        let result = run.await;
        assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");
        assert!(fixture.marker(".int").exists());
        assert!(fixture.marker(".cleaned").exists());
        fixture.assert_stopped().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn interrupt_ignored_uses_bounded_hard_fallback() {
        let fixture = crate::test_support::InterruptFixture::new("buffered-interrupt-ignored");
        let codex = fixture
            .builder()
            .env("CODEX_WRAPPER_TEST_SHUTDOWN_MODE", "ignore")
            .termination_signal(crate::TerminationSignal::Interrupt)
            .termination_grace(Duration::from_millis(100))
            .build()
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            run_codex_cancellable(&codex, vec!["exec".into()], fixture.ready()),
        )
        .await
        .expect("hard fallback exceeded the grace budget");
        assert!(matches!(result, Err(Error::Cancelled { .. })), "{result:?}");
        assert!(fixture.marker(".int").exists());
        assert!(!fixture.marker(".cleaned").exists());
        fixture.assert_stopped().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_during_interrupt_grace_kills_immediately() {
        let fixture = crate::test_support::InterruptFixture::new("buffered-interrupt-drop");
        let codex = fixture
            .builder()
            .env("CODEX_WRAPPER_TEST_SHUTDOWN_MODE", "ignore")
            .termination_signal(crate::TerminationSignal::Interrupt)
            .termination_grace(Duration::from_secs(30))
            .build()
            .unwrap();
        let mut run = Box::pin(run_codex_cancellable(
            &codex,
            vec!["exec".into()],
            fixture.ready(),
        ));
        tokio::select! {
            result = &mut run => panic!("run ended before its grace period: {result:?}"),
            () = fixture.wait_for(".int") => {}
        }
        drop(run);
        fixture.assert_stopped().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_group_optout_skips_interrupt_and_grace() {
        let fixture = crate::test_support::InterruptFixture::new("buffered-interrupt-optout");
        let codex = fixture
            .builder()
            .process_group(false)
            .termination_signal(crate::TerminationSignal::Interrupt)
            .termination_grace(Duration::from_secs(30))
            .build()
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            run_codex_cancellable(&codex, vec!["exec".into()], fixture.detached_ready()),
        )
        .await
        .expect("optout must not wait through the grace period");
        assert!(matches!(result, Err(Error::Cancelled { .. })), "{result:?}");
        assert!(!fixture.marker(".int").exists());
        assert!(!fixture.marker(".term").exists());
        let (parent, child) = fixture.pids().unwrap();
        assert!(crate::test_support::wait_until_gone(parent).await);
        assert!(crate::test_support::is_running_for_test(child));
        // Fixture Drop cleans the deliberately surviving detached child.
    }

    /// Buffered timeout uses the same settled process-group cleanup as an
    /// explicit cancellation signal.
    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_stops_the_group_before_returning() {
        use crate::test_support::is_running_for_test;

        let pid_file = crate::test_support::PidFile::new("group-timeout-settled");
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
            .timeout(Duration::from_millis(300))
            .termination_grace(Duration::from_millis(10))
            .build()
            .unwrap();

        let result = run_codex(&codex, vec!["exec".into()]).await;
        assert!(
            matches!(result, Err(Error::Timeout { .. })),
            "expected a timeout, got: {result:?}"
        );

        let (parent, child) = read_pids(&pid_file).await;
        assert!(!is_running_for_test(parent), "codex ({parent}) survived");
        assert!(
            !is_running_for_test(child),
            "the subprocess ({child}) survived the timeout"
        );
    }

    /// A broken stdin pipe is terminal only after the owned process group has been
    /// stopped. Returning the write error first would release the caller while
    /// the provider could still mutate the workspace.
    #[cfg(unix)]
    #[tokio::test]
    async fn stdin_write_failure_stops_and_reaps_before_returning() {
        use crate::test_support::is_running_for_test;

        let pid_file = crate::test_support::PidFile::new("stdin-write-failure");
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex-closes-stdin-spawns-child.sh");
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .env(
                "CODEX_WRAPPER_TEST_PIDFILE",
                pid_file.path().to_str().unwrap(),
            )
            .termination_grace(Duration::from_millis(10))
            .build()
            .unwrap();

        let prompt = "x".repeat(4 * 1024 * 1024);
        let result = run_codex_with_stdin_prompt(
            &codex,
            crate::ExecCommand::from_stdin(&prompt).args(),
            &prompt,
        )
        .await;
        assert!(
            matches!(result, Err(Error::Io { ref message, .. }) if message.contains("stdin")),
            "expected a stdin error, got: {result:?}"
        );

        let (parent, child) = read_pids(&pid_file).await;
        assert!(!is_running_for_test(parent), "codex ({parent}) survived");
        assert!(
            !is_running_for_test(child),
            "the subprocess ({child}) survived the stdin failure"
        );
    }

    // -----------------------------------------------------------------
    // Failed cleanup is not reported as a settled stop
    //
    // These inject the cleanup result after the real termination and wait
    // have run, so they cover how a failure is reported, not that one occurs.
    // The real-child tests above remain the evidence for normal reaping.
    // -----------------------------------------------------------------

    /// The stop reason inside a [`Error::CleanupFailed`] whose cleanup half
    /// is the injected failure, not some other error.
    #[cfg(unix)]
    fn injected_cleanup_reason(result: Result<CommandOutput>) -> Error {
        match result {
            Err(Error::CleanupFailed { reason, cleanup }) => {
                assert!(
                    cleanup.to_string().contains("injected cleanup failure"),
                    "{cleanup:?}"
                );
                *reason
            }
            other => panic!("expected CleanupFailed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_cleanup_after_cancellation_is_not_a_bare_cancel() {
        let pid_file = crate::test_support::PidFile::new("cleanup-failed-cancel");
        let codex = crate::test_support::blocking_codex(&pid_file)
            .termination_grace(Duration::from_millis(10))
            .build()
            .unwrap();
        let cancel_soon = || tokio::time::sleep(Duration::from_millis(100));

        let injection = CleanupFailureInjection::arm();
        let result = run_codex_cancellable(&codex, vec!["exec".into()], cancel_soon()).await;
        let reason = injected_cleanup_reason(result);
        assert!(matches!(reason, Error::Cancelled { .. }), "{reason:?}");

        // The same run with cleanup succeeding keeps the bare typed reason.
        drop(injection);
        let result = run_codex_cancellable(&codex, vec!["exec".into()], cancel_soon()).await;
        assert!(matches!(result, Err(Error::Cancelled { .. })), "{result:?}");
    }

    /// The timeout stays readable as the reason, the span does not record a
    /// settled timeout, and a policy that retries timeouts does not start a
    /// second process while the first is not known to be gone.
    #[cfg(unix)]
    #[tokio::test]
    async fn failed_cleanup_after_timeout_is_recorded_and_not_retried() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let recorder = recorder::Recorder::install();
        let spawns = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&spawns);
        let pid_file = crate::test_support::PidFile::new("cleanup-failed-timeout");
        let codex = crate::test_support::blocking_codex(&pid_file)
            .timeout(Duration::from_millis(200))
            .termination_grace(Duration::from_millis(10))
            .on_spawn(Arc::new(move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
            }))
            .build()
            .unwrap();
        let policy = crate::retry::RetryPolicy::new()
            .max_attempts(3)
            .initial_backoff(Duration::from_millis(1))
            .retry_on_timeout(true);

        let _injection = CleanupFailureInjection::arm();
        let result = run_codex_with_retry(&codex, vec!["exec".into()], Some(&policy)).await;
        let reason = injected_cleanup_reason(result);
        assert!(matches!(reason, Error::Timeout { .. }), "{reason:?}");
        assert_eq!(spawns.load(Ordering::SeqCst), 1, "a timeout was retried");
        assert_eq!(
            recorder.value("outcome").as_deref(),
            Some("cleanup_failed"),
            "recorded: {}",
            recorder.dump()
        );
    }

    /// The run's own error branch, as opposed to a stop signal, goes through
    /// the same construction.
    #[cfg(unix)]
    #[tokio::test]
    async fn failed_cleanup_after_an_output_limit_keeps_the_limit_as_reason() {
        let codex = flooding_codex("stdout", Some(64 * 1024));

        let injection = CleanupFailureInjection::arm();
        let result = run_codex(&codex, vec!["exec".into()]).await;
        let reason = injected_cleanup_reason(result);
        assert!(
            matches!(
                reason,
                Error::OutputLimitExceeded {
                    stream: OutputStream::Stdout,
                    limit_bytes: 65_536,
                }
            ),
            "{reason:?}"
        );

        drop(injection);
        let result = run_codex(&codex, vec!["exec".into()]).await;
        assert!(
            matches!(result, Err(Error::OutputLimitExceeded { .. })),
            "{result:?}"
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

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_observer_reports_each_buffered_child_before_returning() {
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let codex = Codex::builder()
            .binary("/bin/echo")
            .on_spawn(Arc::new(move |info| sink.lock().unwrap().push(info)))
            .build()
            .expect("echo must exist");

        let output = run_codex(&codex, vec!["probe".into()]).await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].pid > 0);
        assert_eq!(seen[0].pgid, Some(seen[0].pid));
        assert!(output.stdout.contains("probe"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_receipt_never_claims_a_shared_group() {
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let codex = Codex::builder()
            .binary("/bin/echo")
            .process_group(false)
            .on_spawn(Arc::new(move |info| sink.lock().unwrap().push(info)))
            .build()
            .expect("echo must exist");

        run_codex(&codex, vec!["probe".into()]).await.unwrap();
        assert_eq!(seen.lock().unwrap()[0].pgid, None);
    }

    #[test]
    fn parent_death_support_matches_the_target() {
        assert_eq!(die_with_parent_supported(), cfg!(target_os = "linux"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn parent_death_policy_composes_with_spawn_observation() {
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let codex = Codex::builder()
            .binary("/bin/echo")
            .die_with_parent(true)
            .on_spawn(Arc::new(move |info| sink.lock().unwrap().push(info)))
            .build()
            .expect("echo must exist");

        run_codex(&codex, vec!["probe".into()]).await.unwrap();
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    const PDEATHSIG_HELPER_ENV: &str = "CODEX_WRAPPER_PDEATHSIG_HELPER";

    #[test]
    fn pdeathsig_helper_process() {
        if std::env::var(PDEATHSIG_HELPER_ENV).is_err() {
            return;
        }
        use std::io::Write;
        use std::sync::Arc;

        let codex = Codex::builder()
            .binary("/bin/sleep")
            .die_with_parent(true)
            .on_spawn(Arc::new(|info| {
                println!("PID {}", info.pid);
                let _ = std::io::stdout().flush();
            }))
            .build()
            .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = runtime.block_on(run_codex(&codex, vec!["300".into()]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_dies_when_its_parent_is_sigkilled() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};

        let executable = std::env::current_exe().expect("test binary path");
        let mut helper = Command::new(executable)
            .args([
                "--exact",
                "exec::tests::pdeathsig_helper_process",
                "--nocapture",
            ])
            .env(PDEATHSIG_HELPER_ENV, "1")
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn helper");
        let stdout = helper.stdout.take().expect("piped stdout");
        let pid = BufReader::new(stdout)
            .lines()
            .map_while(std::result::Result::ok)
            .find_map(|line| line.strip_prefix("PID ").and_then(|pid| pid.parse().ok()))
            .expect("helper reported child pid");

        // SAFETY: this deliberately simulates an uncatchable supervisor crash.
        unsafe { libc::kill(helper.id() as i32, libc::SIGKILL) };
        let _ = helper.wait();

        for _ in 0..50 {
            if !crate::test_support::is_running_for_test(pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("child {pid} survived its SIGKILLed parent");
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
    #[cfg(unix)]
    struct PrivateTraceDir(std::path::PathBuf);

    #[cfg(unix)]
    impl Drop for PrivateTraceDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn private_trace_client(body: &str) -> (PrivateTraceDir, Codex) {
        use std::os::unix::fs::symlink;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = format!(
            "TRACE_PRIVATE_path-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let dir = PrivateTraceDir(std::env::temp_dir().join(name));
        std::fs::create_dir(&dir.0).unwrap();
        let binary = dir.0.join("TRACE_PRIVATE_binary");
        symlink("/bin/bash", &binary).unwrap();
        let script = dir.0.join("TRACE_PRIVATE_script");
        std::fs::write(&script, body).unwrap();
        let codex = Codex::builder()
            .binary(binary)
            .arg(script.to_string_lossy())
            .arg("TRACE_PRIVATE_config")
            .working_dir(&dir.0)
            .termination_grace(Duration::ZERO)
            .build()
            .unwrap();
        (dir, codex)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn invocation_traces_exclude_private_content_at_every_level() {
        let recorder = recorder::Recorder::install();
        let body = concat!(
            "cat >/dev/null\n",
            "printf 'TRACE_PRIVATE_output'\n",
            "printf 'TRACE_PRIVATE_stderr' >&2\n"
        );
        let (_dir, codex) = private_trace_client(body);
        let args = vec![
            "exec".into(),
            "TRACE_PRIVATE_prompt".into(),
            "--resume".into(),
            "TRACE_PRIVATE_resume".into(),
        ];
        assert!(
            run_codex(&codex, args.clone())
                .await
                .unwrap()
                .stdout
                .contains("TRACE_PRIVATE")
        );
        run_codex_cancellable(&codex, args.clone(), std::future::pending())
            .await
            .unwrap();
        run_codex_with_stdin_prompt(&codex, args.clone(), "TRACE_PRIVATE_stdin")
            .await
            .unwrap();
        run_codex_with_stdin_prompt_cancellable(
            &codex,
            args,
            "TRACE_PRIVATE_stdin",
            std::future::pending(),
        )
        .await
        .unwrap();
        run_codex(&codex, vec!["TRACE_PRIVATE_unknown_command".into()])
            .await
            .unwrap();
        assert!(
            recorder.value("outcome").is_some(),
            "subscriber must observe outcomes"
        );
        assert!(
            !recorder.dump().contains("TRACE_PRIVATE"),
            "{}",
            recorder.dump()
        );
        assert!(
            recorder.value("arg_count").is_some(),
            "useful argument count remains available"
        );
    }

    #[cfg(all(unix, feature = "json"))]
    #[tokio::test]
    async fn stream_and_resume_traces_exclude_private_content() {
        use crate::{CodexCommand, ExecCommand, ExecResumeCommand};
        let recorder = recorder::Recorder::install();
        // Existing verified wire shapes, with synthetic content and identifiers.
        let body = format!(
            "cat >/dev/null\n{}",
            include_str!("../tests/fake-codex.sh")
                .replace("thread_test", "TRACE_PRIVATE_thread")
                .replace("hello", "TRACE_PRIVATE_output")
        );
        let (_dir, codex) = private_trace_client(&body);
        let cmd = ExecCommand::new("TRACE_PRIVATE_prompt");
        let mut observed = 0;
        crate::streaming::stream_exec(&codex, &cmd, |_| observed += 1)
            .await
            .unwrap();
        crate::streaming::stream_exec_cancellable(&codex, &cmd, std::future::pending(), |_| {})
            .await
            .unwrap();
        let stdin_cmd = ExecCommand::new("TRACE_PRIVATE_stdin").prompt_via_stdin();
        crate::streaming::stream_exec(&codex, &stdin_cmd, |_| {})
            .await
            .unwrap();
        let resume = ExecResumeCommand::new()
            .session_id("TRACE_PRIVATE_resume")
            .prompt("TRACE_PRIVATE_prompt");
        crate::streaming::stream_exec_resume(&codex, &resume, |_| {})
            .await
            .unwrap();
        crate::streaming::stream_exec_resume_cancellable(
            &codex,
            &resume,
            std::future::pending(),
            |_| {},
        )
        .await
        .unwrap();
        assert!(observed > 0, "protocol delivery remains intact");
        assert!(
            cmd.to_command_string(&codex).contains("TRACE_PRIVATE"),
            "explicit preview stays caller-controlled"
        );
        assert!(
            !recorder.dump().contains("TRACE_PRIVATE"),
            "{}",
            recorder.dump()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failure_and_retry_traces_exclude_rendered_errors_and_output() {
        let recorder = recorder::Recorder::install();
        let (_dir, mut codex) = private_trace_client(
            "printf 'TRACE_PRIVATE_output'; printf 'TRACE_PRIVATE_stderr' >&2; exit 7",
        );
        codex.retry_policy = Some(
            crate::RetryPolicy::new()
                .max_attempts(2)
                .initial_backoff(Duration::ZERO)
                .retry_on_exit_codes([7]),
        );
        let error = run_codex(&codex, vec!["exec".into(), "TRACE_PRIVATE_prompt".into()])
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("TRACE_PRIVATE"),
            "explicit error retains diagnostics"
        );
        codex.retry_policy = None;
        codex.output_limit = Some(8);
        assert!(matches!(
            run_codex(&codex, vec!["exec".into()]).await,
            Err(Error::OutputLimitExceeded { .. })
        ));
        assert!(
            !recorder.dump().contains("TRACE_PRIVATE"),
            "{}",
            recorder.dump()
        );
        assert!(
            recorder.value("attempt").is_some(),
            "retry event must be captured"
        );
    }

    #[cfg(all(unix, feature = "app-server"))]
    #[tokio::test]
    async fn app_server_startup_trace_excludes_private_arguments() {
        let recorder = recorder::Recorder::install();
        let codex = Codex::builder()
            .binary("/bin/false")
            .arg("TRACE_PRIVATE_configuration")
            .build()
            .unwrap();
        assert!(
            crate::app_server::AppServer::builder(&codex)
                .start()
                .await
                .is_err()
        );
        assert!(
            !recorder.dump().contains("TRACE_PRIVATE"),
            "{}",
            recorder.dump()
        );
        assert!(
            recorder.value("arg_count").is_some(),
            "startup event must be captured"
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn output_limit_trace_omits_captured_prefix() {
        let recorder = recorder::Recorder::install();
        // Separate writes let the reader retain a prefix before the breach.
        let (_dir, mut codex) = private_trace_client(
            "printf 'TRACE_PRIVATE_output'; sleep 0.05; head -c 4096 /dev/zero",
        );
        codex.output_limit = Some(64);
        assert!(matches!(
            run_codex(&codex, vec!["exec".into()]).await,
            Err(Error::OutputLimitExceeded { .. })
        ));
        assert!(
            recorder.value("captured_bytes").is_some(),
            "capture warning was observed"
        );
        assert!(
            !recorder.dump().contains("TRACE_PRIVATE"),
            "{}",
            recorder.dump()
        );
    }
}
