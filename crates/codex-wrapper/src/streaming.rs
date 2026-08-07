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
    run_streaming(codex, args, cmd.stdin_prompt(), handler).await
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
    run_streaming(codex, args, None, handler).await
}

/// Core streaming implementation shared by both exec variants.
///
/// `stdin_prompt` carries the prompt for a `codex exec -` run, where it is
/// delivered on stdin rather than in argv.
async fn run_streaming<F>(
    codex: &Codex,
    args: Vec<String>,
    stdin_prompt: Option<&str>,
    mut handler: F,
) -> Result<()>
where
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

    if let Some(dir) = &codex.working_dir {
        child_cmd.current_dir(dir);
    }
    for (key, value) in &codex.env {
        child_cmd.env(key, value);
    }

    let mut child = child_cmd.spawn().map_err(|e| Error::Io {
        message: format!("failed to spawn codex: {e}"),
        source: e,
        working_dir: codex.working_dir.clone(),
    })?;

    // Armed for the whole stream. Dropping this future signals the group,
    // which reaches the subprocesses codex started for tool use; kill_on_drop
    // alone would leave those running (#78).
    let mut group =
        crate::exec::GroupKillGuard::new(codex.process_group.then(|| child.id()).flatten());

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
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut events = Vec::new();
        while let Some(line) = lines.next_line().await.map_err(|e| Error::Io {
            message: format!("failed to read stdout line: {e}"),
            source: e,
            working_dir: codex.working_dir.clone(),
        })? {
            if line.trim_start().starts_with('{') {
                match serde_json::from_str::<JsonLineEvent>(&line) {
                    Ok(event) => events.push(event),
                    Err(source) => {
                        return Err(Error::Json {
                            message: format!("failed to parse JSONL event: {line}"),
                            source,
                        });
                    }
                }
            }
        }
        Ok::<Vec<JsonLineEvent>, Error>(events)
    };

    let stderr_task = async {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        let mut collected = String::new();
        while let Some(line) = lines.next_line().await.map_err(|e| Error::Io {
            message: format!("failed to read stderr line: {e}"),
            source: e,
            working_dir: codex.working_dir.clone(),
        })? {
            if !collected.is_empty() {
                collected.push('\n');
            }
            collected.push_str(&line);
        }
        Ok::<String, Error>(collected)
    };

    let stream_future = async {
        let (stdin_result, events_result, stderr_result) =
            tokio::join!(stdin_task, stdout_task, stderr_task);
        stdin_result?;
        let events = events_result?;
        let stderr_output = stderr_result?;

        for event in events {
            handler(event);
        }

        let status = child.wait().await.map_err(|e| Error::Io {
            message: format!("failed to wait on codex process: {e}"),
            source: e,
            working_dir: codex.working_dir.clone(),
        })?;

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
    };

    // Dropped explicitly before awaiting: the guard exists so the span is the
    // parent of everything above, while the await below must not hold it
    // across a yield point.
    drop(_span_guard);

    if let Some(timeout) = codex.timeout {
        // On elapse the stream future is dropped, taking `outcome` with it,
        // whose drop records the run as cancelled. That is the same path a
        // caller dropping this future takes.
        match tokio::time::timeout(timeout, stream_future.instrument(span.clone())).await {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout {
                timeout_seconds: timeout.as_secs(),
            }),
        }
    } else {
        stream_future.instrument(span).await
    }
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
    async fn stream_exec_parse_error() {
        let codex = fake_codex("fake-codex-bad-json.sh");
        let cmd = crate::command::exec::ExecCommand::new("test").json();
        let result = stream_exec(&codex, &cmd, |_| {}).await;

        assert!(
            matches!(result, Err(Error::Json { .. })),
            "expected json parse error, got: {result:?}"
        );
    }
}
