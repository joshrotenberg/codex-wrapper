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
use tracing::debug;

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
    run_streaming(codex, args, handler).await
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
    run_streaming(codex, args, handler).await
}

/// Core streaming implementation shared by both exec variants.
async fn run_streaming<F>(codex: &Codex, args: Vec<String>, mut handler: F) -> Result<()>
where
    F: FnMut(JsonLineEvent),
{
    let mut command_args = Vec::new();
    command_args.extend(codex.global_args.clone());
    command_args.extend(args);

    debug!(binary = %codex.binary.display(), args = ?command_args, "streaming codex command");

    let mut child_cmd = Command::new(&codex.binary);
    child_cmd.args(&command_args);
    child_cmd.stdin(std::process::Stdio::null());
    child_cmd.stdout(std::process::Stdio::piped());
    child_cmd.stderr(std::process::Stdio::piped());

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

    let stdout = child.stdout.take().expect("stdout was configured as piped");
    let stderr = child.stderr.take().expect("stderr was configured as piped");

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
        let (events_result, stderr_result) = tokio::join!(stdout_task, stderr_task);
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
            return Err(Error::CommandFailed {
                command: format!("{} {}", codex.binary.display(), command_args.join(" ")),
                exit_code,
                stdout: String::new(),
                stderr: stderr_output,
                working_dir: codex.working_dir.clone(),
            });
        }

        Ok(())
    };

    if let Some(timeout) = codex.timeout {
        tokio::time::timeout(timeout, stream_future)
            .await
            .map_err(|_| Error::Timeout {
                timeout_seconds: timeout.as_secs(),
            })?
    } else {
        stream_future.await
    }
}

#[cfg(test)]
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
            types.contains(&"completed"),
            "expected completed, got: {types:?}"
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
