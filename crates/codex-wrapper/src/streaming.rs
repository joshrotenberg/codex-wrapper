//! Streaming execution for `codex exec` commands.
//!
//! Instead of buffering all JSONL output and returning it at once,
//! the streaming API pipes stdout from the child process and delivers
//! each [`JsonLineEvent`] to a caller-supplied callback as soon as it
//! arrives.
//!
//! # Example
//!
//! ```no_run
//! use codex_wrapper::{Codex, ExecCommand, JsonLineEvent};
//!
//! # async fn example() -> codex_wrapper::Result<()> {
//! let codex = Codex::builder().build()?;
//! let cmd = ExecCommand::new("what is 2+2?").ephemeral();
//!
//! cmd.stream(&codex, |event: JsonLineEvent| {
//!     println!("{}: {:?}", event.event_type, event.extra);
//! })
//! .await?;
//! # Ok(())
//! # }
//! ```

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{Instrument, debug};

use crate::Codex;
use crate::command::CodexCommand;
use crate::error::{Error, Result};
use crate::types::JsonLineEvent;

/// Stream JSONL events from `codex exec <prompt>`, invoking `handler` for each
/// parsed [`JsonLineEvent`].
///
/// The child's stderr is drained concurrently and returned in the error if the
/// process exits with a non-zero status.
pub async fn stream_exec<F>(
    codex: &Codex,
    cmd: &crate::command::exec::ExecCommand,
    handler: F,
) -> Result<()>
where
    F: FnMut(JsonLineEvent),
{
    let mut args = cmd.args();
    if !args.contains(&"--json".to_string()) {
        args.push("--json".into());
    }
    run_streaming(
        codex,
        args,
        cmd.stdin_prompt(),
        handler,
        std::future::pending(),
    )
    .await
}

/// Stream JSONL events until the command completes or `cancel` resolves.
///
/// Cancellation terminates the owned process group, waits through the
/// client's configured grace period, and reaps the direct child before
/// returning [`Error::Cancelled`].
pub async fn stream_exec_cancellable<C, F>(
    codex: &Codex,
    cmd: &crate::command::exec::ExecCommand,
    cancel: C,
    handler: F,
) -> Result<()>
where
    C: std::future::Future<Output = ()> + Send,
    F: FnMut(JsonLineEvent),
{
    let mut args = cmd.args();
    if !args.contains(&"--json".to_string()) {
        args.push("--json".into());
    }
    run_streaming(codex, args, cmd.stdin_prompt(), handler, cancel).await
}

/// Stream JSONL events from `codex exec resume`, invoking `handler` for each
/// parsed [`JsonLineEvent`].
pub async fn stream_exec_resume<F>(
    codex: &Codex,
    cmd: &crate::command::exec::ExecResumeCommand,
    handler: F,
) -> Result<()>
where
    F: FnMut(JsonLineEvent),
{
    let mut args = cmd.args();
    if !args.contains(&"--json".to_string()) {
        args.push("--json".into());
    }
    run_streaming(
        codex,
        args,
        cmd.stdin_prompt(),
        handler,
        std::future::pending(),
    )
    .await
}

/// Stream resumed-turn JSONL events until completion or cancellation.
///
/// This has the same settled process-cleanup contract as
/// [`stream_exec_cancellable`].
pub async fn stream_exec_resume_cancellable<C, F>(
    codex: &Codex,
    cmd: &crate::command::exec::ExecResumeCommand,
    cancel: C,
    handler: F,
) -> Result<()>
where
    C: std::future::Future<Output = ()> + Send,
    F: FnMut(JsonLineEvent),
{
    let mut args = cmd.args();
    if !args.contains(&"--json".to_string()) {
        args.push("--json".into());
    }
    run_streaming(codex, args, cmd.stdin_prompt(), handler, cancel).await
}

/// Core streaming implementation shared by both exec variants.
///
/// `stdin_prompt` carries the prompt for a `codex exec -` run, where it is
/// delivered on stdin rather than in argv.
async fn run_streaming<C, F>(
    codex: &Codex,
    args: Vec<String>,
    stdin_prompt: Option<&str>,
    mut handler: F,
    cancel: C,
) -> Result<()>
where
    C: std::future::Future<Output = ()> + Send,
    F: FnMut(JsonLineEvent),
{
    let span = crate::exec::command_span("codex.stream", codex, &args);
    let command_args = crate::exec::assemble_args(codex, args);
    let _span_guard = span.clone().entered();

    debug!(binary = %codex.binary.display(), args = ?command_args, "streaming codex command");

    // Settles on every exit below, and on drop for the one that has no exit:
    // a cancelled stream, which is the outcome most easily missed.
    let mut outcome = crate::exec::SpanOutcome::start(span.clone());

    let mut child_cmd = Command::new(&codex.binary);
    child_cmd.args(&command_args);
    if stdin_prompt.is_some() {
        child_cmd.stdin(std::process::Stdio::piped());
    } else {
        child_cmd.stdin(std::process::Stdio::null());
    }
    child_cmd.stdout(std::process::Stdio::piped());
    child_cmd.stderr(std::process::Stdio::piped());

    // Kill the child if this future is dropped: on timeout, on caller
    // cancellation, or on task abort. Without this, tokio detaches the child
    // and codex keeps running with no handle left to stop it.
    child_cmd.kill_on_drop(true);
    crate::exec::own_process_group(&mut child_cmd, codex.process_group);
    crate::exec::apply_die_with_parent(&mut child_cmd, codex.die_with_parent);

    if let Some(dir) = &codex.working_dir {
        child_cmd.current_dir(dir);
    }
    crate::exec::apply_child_environment(&mut child_cmd, codex.clear_env, &codex.env);

    let mut child = child_cmd.spawn().map_err(|e| Error::Io {
        message: format!("failed to spawn codex: {e}"),
        source: e,
        working_dir: codex.working_dir.clone(),
    })?;

    // Armed for the whole stream. Dropping this future signals the group,
    // which reaches the subprocesses codex started for tool use; kill_on_drop
    // alone would leave those running (#78).
    let mut group =
        crate::exec::arm_and_notify(codex.process_group, child.id(), codex.on_spawn.as_ref());

    let stdout = child.stdout.take().expect("stdout was configured as piped");
    let stderr = child.stderr.take().expect("stderr was configured as piped");
    // Taken up front so the write does not borrow `child`, which the wait
    // below needs.
    let child_stdin = child.stdin.take();

    // Write the prompt and close the handle, so the CLI stops waiting for
    // more. This runs as part of the streamed future rather than before it,
    // so a prompt larger than the pipe buffer cannot block the readers.
    let stdin_task = async {
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
                working_dir: codex.working_dir.clone(),
            })?;
        stdin.shutdown().await.map_err(|e| Error::Io {
            message: format!("failed to close codex stdin: {e}"),
            source: e,
            working_dir: codex.working_dir.clone(),
        })
    };

    let stdout_task = async {
        let mut reader = BufReader::new(stdout);
        let mut captured_bytes = 0usize;
        while let Some(line) = read_bounded_line(
            &mut reader,
            &mut captured_bytes,
            codex.output_limit,
            crate::OutputStream::Stdout,
            &codex.working_dir,
        )
        .await?
        {
            if line.trim_start().starts_with('{') {
                match serde_json::from_str::<JsonLineEvent>(&line) {
                    Ok(event) => handler(event),
                    Err(source) => {
                        return Err(Error::Json {
                            message: format!("failed to parse JSONL event: {line}"),
                            source,
                        });
                    }
                }
            }
        }
        Ok::<(), Error>(())
    };

    let stderr_task = async {
        let mut reader = BufReader::new(stderr);
        let mut collected = String::new();
        let mut captured_bytes = 0usize;
        while let Some(line) = read_bounded_line(
            &mut reader,
            &mut captured_bytes,
            codex.output_limit,
            crate::OutputStream::Stderr,
            &codex.working_dir,
        )
        .await?
        {
            if !collected.is_empty() {
                collected.push('\n');
            }
            collected.push_str(&line);
        }
        Ok::<String, Error>(collected)
    };

    let wait_task = async {
        child.wait().await.map_err(|e| Error::Io {
            message: format!("failed to wait on codex process: {e}"),
            source: e,
            working_dir: codex.working_dir.clone(),
        })
    };
    let stream_future = async {
        let ((), (), stderr_output, status) =
            tokio::try_join!(stdin_task, stdout_task, stderr_task, wait_task)?;
        Ok::<_, Error>((stderr_output, status))
    };

    // Dropped explicitly before awaiting: the guard exists so the span is the
    // parent of everything above, while the await below must not hold it
    // across a yield point.
    drop(_span_guard);

    let stop = async {
        match codex.timeout {
            Some(timeout) => tokio::select! {
                () = cancel => StreamStop::Cancelled,
                () = tokio::time::sleep(timeout) => StreamStop::Timeout(timeout),
            },
            None => {
                cancel.await;
                StreamStop::Cancelled
            }
        }
    };
    let finished = tokio::select! {
        result = stream_future.instrument(span) => Ok(result),
        reason = stop => Err(reason),
    };

    let (stderr_output, status) = match finished {
        Ok(Ok(finished)) => finished,
        Ok(Err(error)) => {
            crate::exec::terminate_and_reap(
                &mut child,
                &mut group,
                codex.termination_grace,
                codex.working_dir.as_deref(),
            )
            .await?;
            outcome.settle("failed", None);
            return Err(error);
        }
        Err(reason) => {
            crate::exec::terminate_and_reap(
                &mut child,
                &mut group,
                codex.termination_grace,
                codex.working_dir.as_deref(),
            )
            .await?;
            let error = match reason {
                StreamStop::Cancelled => {
                    outcome.settle("cancelled", None);
                    Error::Cancelled {
                        grace_seconds: codex.termination_grace.as_secs(),
                    }
                }
                StreamStop::Timeout(timeout) => {
                    outcome.settle("timeout", None);
                    Error::Timeout {
                        timeout_seconds: timeout.as_secs(),
                    }
                }
            };
            return Err(error);
        }
    };

    let exit_code = status.code().unwrap_or(-1);
    if !status.success() {
        outcome.settle("failed", Some(exit_code));
        return Err(Error::from_command_failure(
            format!("{} {}", codex.binary.display(), command_args.join(" ")),
            exit_code,
            String::new(),
            stderr_output,
            codex.working_dir.clone(),
        ));
    }

    outcome.settle("ok", Some(exit_code));
    group.disarm();
    Ok(())
}

enum StreamStop {
    Cancelled,
    Timeout(std::time::Duration),
}

async fn read_bounded_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    captured_bytes: &mut usize,
    output_limit: Option<usize>,
    stream: crate::OutputStream,
    working_dir: &Option<std::path::PathBuf>,
) -> Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|source| Error::Io {
            message: format!("failed to read {stream} line: {source}"),
            source,
            working_dir: working_dir.clone(),
        })?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if output_limit.is_some_and(|limit| captured_bytes.saturating_add(take) > limit) {
            return Err(Error::OutputLimitExceeded {
                stream,
                limit_bytes: output_limit.unwrap_or_default(),
            });
        }
        line.extend_from_slice(&available[..take]);
        *captured_bytes = captured_bytes.saturating_add(take);
        let complete = line.last() == Some(&b'\n');
        reader.consume(take);
        if complete {
            line.pop();
            break;
        }
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|source| Error::Io {
            message: format!("failed to decode {stream} line"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
            working_dir: working_dir.clone(),
        })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Build a [`Codex`] client that uses `bash` to run the fake-codex script.
    fn fake_codex(script_name: &str) -> Codex {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join(script_name);
        Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .build()
            .expect("bash must exist")
    }

    #[tokio::test]
    async fn stream_exec_delivers_events() {
        let codex = fake_codex("fake-codex.sh");
        let cmd = crate::command::exec::ExecCommand::new("test prompt").json();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        stream_exec(&codex, &cmd, move |event| {
            events_clone.lock().unwrap().push(event);
        })
        .await
        .unwrap();

        let events = events.lock().unwrap();
        assert!(!events.is_empty(), "expected at least one event");

        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(
            types.contains(&"thread.started"),
            "expected thread.started, got: {types:?}"
        );
        assert!(
            types.contains(&"turn.completed"),
            "expected turn.completed, got: {types:?}"
        );
    }

    /// The API promises delivery as events arrive. Buffering until stdout
    /// closes loses a thread id when a long-running process or its host dies
    /// after `thread.started`, which is the durability use case for streaming.
    #[tokio::test]
    async fn stream_exec_delivers_an_event_while_the_child_is_still_running() {
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg("-c")
            .arg(
                "printf '%s\\n' '{\"type\":\"thread.started\",\"thread_id\":\"thread-early\"}'; sleep 10",
            )
            .build()
            .expect("bash must exist");
        let cmd = crate::command::exec::ExecCommand::new("probe").json();
        let (sent, delivered) = tokio::sync::oneshot::channel();
        let mut sent = Some(sent);

        let task = tokio::spawn(async move {
            stream_exec(&codex, &cmd, move |event| {
                if event.thread_id() == Some("thread-early")
                    && let Some(sent) = sent.take()
                {
                    let _ = sent.send(());
                }
            })
            .await
        });

        let delivered = tokio::time::timeout(std::time::Duration::from_secs(1), delivered).await;
        let still_running = !task.is_finished();
        task.abort();
        let _ = task.await;

        assert!(delivered.is_ok(), "callback waited for the child to exit");
        assert!(
            still_running,
            "the fixture must still be running when the callback fires"
        );
    }

    #[tokio::test]
    async fn stream_exec_resume_delivers_events() {
        let codex = fake_codex("fake-codex.sh");
        let cmd = crate::command::exec::ExecResumeCommand::new().last().json();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);

        stream_exec_resume(&codex, &cmd, move |event| {
            events_clone.lock().unwrap().push(event);
        })
        .await
        .unwrap();

        let events = events.lock().unwrap();
        assert!(!events.is_empty(), "expected at least one event");
    }

    #[tokio::test]
    async fn streaming_reports_the_child_before_events_are_delivered() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let spawn_order = Arc::clone(&order);
        let event_order = Arc::clone(&order);
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex.sh");
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().unwrap())
            .on_spawn(Arc::new(move |_| spawn_order.lock().unwrap().push("spawn")))
            .build()
            .expect("bash must exist");

        crate::ExecCommand::new("probe")
            .json()
            .stream(&codex, move |_| {
                let mut order = event_order.lock().unwrap();
                if !order.contains(&"event") {
                    order.push("event");
                }
            })
            .await
            .unwrap();

        assert_eq!(&order.lock().unwrap()[..2], ["spawn", "event"]);
    }

    /// Streaming owns a separate process builder from buffered execution.
    /// Cover opening, stdin opening, and resume so none can regress to ambient
    /// inheritance while the others stay isolated.
    #[tokio::test]
    async fn cleared_environment_reaches_every_streaming_variant() {
        let capture = crate::test_support::EnvCapture::new("env-streaming");
        let codex = crate::test_support::env_capturing_codex(&capture)
            .clear_env()
            .env("CODEX_WRAPPER_EXPLICIT", "streaming")
            .build()
            .expect("bash must exist");

        crate::ExecCommand::new("opening")
            .stream(&codex, |_| {})
            .await
            .unwrap();
        let opening_environment = capture.read();
        assert!(!opening_environment.contains_key("PATH"));
        assert_eq!(
            opening_environment
                .get("CODEX_WRAPPER_EXPLICIT")
                .map(String::as_str),
            Some("streaming")
        );

        crate::ExecCommand::new("stdin")
            .prompt_via_stdin()
            .stream(&codex, |_| {})
            .await
            .unwrap();
        let stdin_environment = capture.read();
        assert!(!stdin_environment.contains_key("PATH"));
        assert_eq!(
            stdin_environment
                .get("CODEX_WRAPPER_EXPLICIT")
                .map(String::as_str),
            Some("streaming")
        );

        crate::ExecResumeCommand::new()
            .last()
            .stream(&codex, |_| {})
            .await
            .unwrap();
        let resume_environment = capture.read();
        assert!(!resume_environment.contains_key("PATH"));
        assert_eq!(
            resume_environment
                .get("CODEX_WRAPPER_EXPLICIT")
                .map(String::as_str),
            Some("streaming")
        );
    }

    /// Contract captured from a paid 0.145.0 run: the callback receives the
    /// typed terminal before the process reports its non-zero exit, including
    /// an assistant message but no fabricated token counts.
    #[tokio::test]
    async fn stream_exec_classifies_native_rollout_budget_exhaustion() {
        let codex = fake_codex("fake-codex-rollout-budget.sh");
        let cmd = crate::command::exec::ExecCommand::new("probe").json();
        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&events);

        let error = stream_exec(&codex, &cmd, move |event| {
            collected.lock().unwrap().push(event);
        })
        .await
        .expect_err("captured rollout exhaustion exits non-zero");

        assert_eq!(error.exit_code(), Some(1));
        let events = events.lock().unwrap();
        let terminal = events
            .iter()
            .find(|event| event.is_turn_failed())
            .expect("turn.failed must be delivered");
        assert_eq!(
            terminal.turn_failure_kind(),
            Some(crate::TurnFailureKind::RolloutBudgetExhausted)
        );
        assert_eq!(terminal.usage(), None);
        assert_eq!(crate::QueryResult::from_events(events.clone()).result, "ok");
    }

    #[tokio::test]
    async fn stream_exec_timeout() {
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg("-c")
            .arg("sleep 10")
            .timeout(std::time::Duration::from_millis(50))
            .build()
            .unwrap();

        let cmd = crate::command::exec::ExecCommand::new("test").json();
        let result = stream_exec(&codex, &cmd, |_| {}).await;

        assert!(
            matches!(result, Err(Error::Timeout { .. })),
            "expected timeout error, got: {result:?}"
        );
    }

    /// The streaming path drops both `stream_future` and the child it borrows.
    /// Without `kill_on_drop`, the timeout above would leave codex running.
    #[tokio::test]
    async fn stream_exec_timeout_kills_the_spawned_process() {
        use crate::test_support::{PidFile, blocking_codex, wait_until_gone};

        let pid_file = PidFile::new("stream-timeout");
        let codex = blocking_codex(&pid_file)
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .expect("bash must exist");

        let cmd = crate::command::exec::ExecCommand::new("probe").json();
        let result = stream_exec(&codex, &cmd, |_| {}).await;
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

    /// The caller dropping the stream future, with no wrapper timeout.
    #[tokio::test]
    async fn stream_exec_cancellation_kills_the_spawned_process() {
        use crate::test_support::{PidFile, blocking_codex, wait_until_gone};

        let pid_file = PidFile::new("stream-cancel");
        let codex = blocking_codex(&pid_file).build().expect("bash must exist");

        let cmd = crate::command::exec::ExecCommand::new("probe").json();
        let cancelled = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            stream_exec(&codex, &cmd, |_| {}),
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

    #[tokio::test]
    async fn explicit_stream_cancellation_reaps_before_returning() {
        use crate::test_support::{PidFile, blocking_codex, wait_until_gone};

        let pid_file = PidFile::new("stream-explicit-cancel");
        let codex = blocking_codex(&pid_file).build().expect("bash must exist");
        let cmd = crate::command::exec::ExecCommand::new("probe").json();
        let cancel = async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };

        let result = stream_exec_cancellable(&codex, &cmd, cancel, |_| {}).await;
        assert!(matches!(result, Err(Error::Cancelled { .. })));

        let pid = pid_file.read_pid().await;
        assert!(
            wait_until_gone(pid).await,
            "codex ({pid}) survived explicit stream cancellation"
        );
    }

    #[tokio::test]
    async fn stream_exec_parse_error() {
        let codex = fake_codex("fake-codex-bad-json.sh");
        let cmd = crate::command::exec::ExecCommand::new("test").json();
        let result = stream_exec(&codex, &cmd, |_| {}).await;

        assert!(
            matches!(result, Err(Error::Json { .. })),
            "expected json parse error, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn streaming_enforces_the_raw_output_ceiling() {
        let codex = Codex::builder()
            .binary("/bin/bash")
            .arg("-c")
            .arg("for ((i=0; i<4096; i++)); do printf x; done; sleep 3")
            .output_limit(128)
            .timeout(std::time::Duration::from_secs(1))
            .build()
            .expect("bash must exist");
        let cmd = crate::command::exec::ExecCommand::new("probe").json();

        let result = stream_exec(&codex, &cmd, |_| {}).await;

        assert!(matches!(
            result,
            Err(Error::OutputLimitExceeded {
                stream: crate::OutputStream::Stdout,
                limit_bytes: 128,
            })
        ));
    }
}
