//! Stateful multi-turn session manager for the Codex CLI.
//!
//! [`Session`] wraps a [`Codex`] client and automatically threads
//! conversation state across turns. The first call to [`send`](Session::send)
//! dispatches via [`ExecCommand`]; subsequent calls use
//! [`ExecResumeCommand`] with the captured `thread_id`.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use codex_wrapper::{Codex, Session};
//!
//! # async fn example() -> codex_wrapper::Result<()> {
//! let codex = Arc::new(Codex::builder().build()?);
//! let mut session = Session::new(codex);
//!
//! let events = session.send("create a hello world program").await?;
//! println!("turn 1: {} events", events.len());
//!
//! let events = session.send("now add error handling").await?;
//! println!("turn 2: {} events, thread_id={:?}", events.len(), session.id());
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use crate::Codex;
use crate::command::exec::{ExecCommand, ExecResumeCommand};
use crate::error::{Error, Result};
use crate::types::JsonLineEvent;

/// A record of a single turn within a session.
#[derive(Debug, Clone)]
pub struct TurnRecord {
    /// The parsed JSONL events returned by this turn.
    pub events: Vec<JsonLineEvent>,
}

/// Stateful multi-turn session manager.
///
/// Wraps a [`Codex`] client and automatically threads conversation state
/// across turns. On the first turn, an [`ExecCommand`] is used; on subsequent
/// turns, an [`ExecResumeCommand`] resumes the session using the `thread_id`
/// extracted from the JSONL event stream.
///
/// The `thread_id` is preserved even when a turn fails, as long as at least
/// one event in the output carried it.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use codex_wrapper::{Codex, Session};
///
/// # async fn example() -> codex_wrapper::Result<()> {
/// let codex = Arc::new(Codex::builder().build()?);
/// let mut session = Session::new(codex);
///
/// let events = session.send("summarize this repo").await?;
/// assert!(session.id().is_some());
/// assert_eq!(session.total_turns(), 1);
///
/// let events = session.send("now add more detail").await?;
/// assert_eq!(session.total_turns(), 2);
/// # Ok(())
/// # }
/// ```
pub struct Session {
    codex: Arc<Codex>,
    thread_id: Option<String>,
    history: Vec<TurnRecord>,
}

impl Session {
    /// Create a new session with no prior state.
    ///
    /// The first call to [`send`](Session::send) will use [`ExecCommand`].
    pub fn new(codex: Arc<Codex>) -> Self {
        Self {
            codex,
            thread_id: None,
            history: Vec::new(),
        }
    }

    /// Resume an existing session by its `thread_id`.
    ///
    /// The next call to [`send`](Session::send) will use
    /// [`ExecResumeCommand`] with the provided ID.
    pub fn resume(codex: Arc<Codex>, thread_id: impl Into<String>) -> Self {
        Self {
            codex,
            thread_id: Some(thread_id.into()),
            history: Vec::new(),
        }
    }

    /// Send a prompt, automatically routing to `exec` or `exec resume`.
    ///
    /// On the first turn (no `thread_id`), dispatches via [`ExecCommand`].
    /// On subsequent turns, dispatches via [`ExecResumeCommand`] with the
    /// captured `thread_id`.
    ///
    /// Returns the parsed JSONL events for this turn.
    pub async fn send(&mut self, prompt: impl Into<String>) -> Result<Vec<JsonLineEvent>> {
        let prompt = prompt.into();

        match &self.thread_id {
            None => {
                let cmd = ExecCommand::new(&prompt);
                self.run_exec(cmd).await
            }
            Some(id) => {
                let cmd = ExecResumeCommand::new()
                    .session_id(id.clone())
                    .prompt(prompt);
                self.run_resume(cmd).await
            }
        }
    }

    /// Execute an [`ExecCommand`] with full control over its options.
    ///
    /// Use this when you need to configure model, sandbox, approval policy,
    /// or other flags beyond what [`send`](Session::send) provides.
    /// The session still captures the `thread_id` from the output.
    pub async fn execute(&mut self, cmd: ExecCommand) -> Result<Vec<JsonLineEvent>> {
        self.run_exec(cmd).await
    }

    /// Execute an [`ExecResumeCommand`] with full control over its options.
    ///
    /// Use this when you need to configure flags on the resume command
    /// beyond what [`send`](Session::send) provides.
    /// The session still captures the `thread_id` from the output.
    pub async fn execute_resume(&mut self, cmd: ExecResumeCommand) -> Result<Vec<JsonLineEvent>> {
        self.run_resume(cmd).await
    }

    // TODO: streaming support depends on #20
    // pub async fn stream(&mut self, prompt: impl Into<String>) -> ...
    // pub async fn stream_execute(&mut self, cmd: ExecCommand) -> ...

    /// Returns the `thread_id` captured from the most recent turn, if any.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    /// Total number of completed turns in this session.
    #[must_use]
    pub fn total_turns(&self) -> usize {
        self.history.len()
    }

    /// Borrow the full turn history.
    #[must_use]
    pub fn history(&self) -> &[TurnRecord] {
        &self.history
    }

    /// Run an [`ExecCommand`] and record the turn.
    async fn run_exec(&mut self, cmd: ExecCommand) -> Result<Vec<JsonLineEvent>> {
        match cmd.execute_json_lines(&self.codex).await {
            Ok(events) => {
                self.capture_thread_id(&events);
                self.history.push(TurnRecord {
                    events: events.clone(),
                });
                Ok(events)
            }
            Err(Error::CommandFailed {
                stdout,
                stderr,
                exit_code,
                command,
                working_dir,
            }) => {
                self.try_capture_thread_id_from_stdout(&stdout);
                Err(Error::CommandFailed {
                    stdout,
                    stderr,
                    exit_code,
                    command,
                    working_dir,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Run an [`ExecResumeCommand`] and record the turn.
    async fn run_resume(&mut self, cmd: ExecResumeCommand) -> Result<Vec<JsonLineEvent>> {
        match cmd.execute_json_lines(&self.codex).await {
            Ok(events) => {
                self.capture_thread_id(&events);
                self.history.push(TurnRecord {
                    events: events.clone(),
                });
                Ok(events)
            }
            Err(Error::CommandFailed {
                stdout,
                stderr,
                exit_code,
                command,
                working_dir,
            }) => {
                self.try_capture_thread_id_from_stdout(&stdout);
                Err(Error::CommandFailed {
                    stdout,
                    stderr,
                    exit_code,
                    command,
                    working_dir,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Extract `thread_id` from parsed events (first match wins).
    fn capture_thread_id(&mut self, events: &[JsonLineEvent]) {
        if let Some(id) = events.iter().find_map(|e| e.thread_id()) {
            self.thread_id = Some(id.to_string());
        }
    }

    /// Best-effort extraction of `thread_id` from raw stdout on error paths.
    fn try_capture_thread_id_from_stdout(&mut self, stdout: &str) {
        for line in stdout.lines() {
            if let Ok(event) = serde_json::from_str::<JsonLineEvent>(line)
                && let Some(id) = event.thread_id()
            {
                self.thread_id = Some(id.to_string());
                return;
            }
        }
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("thread_id", &self.thread_id)
            .field("total_turns", &self.history.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_codex() -> Arc<Codex> {
        Arc::new(Codex::builder().binary("/usr/bin/false").build().unwrap())
    }

    #[test]
    fn new_session_has_no_state() {
        let session = Session::new(test_codex());
        assert!(session.id().is_none());
        assert_eq!(session.total_turns(), 0);
        assert!(session.history().is_empty());
    }

    #[test]
    fn resume_session_has_thread_id() {
        let session = Session::resume(test_codex(), "thread_abc");
        assert_eq!(session.id(), Some("thread_abc"));
        assert_eq!(session.total_turns(), 0);
    }

    #[test]
    fn capture_thread_id_from_events() {
        let mut session = Session::new(test_codex());
        let events: Vec<JsonLineEvent> = vec![
            serde_json::from_str(r#"{"type":"message.created","role":"assistant"}"#).unwrap(),
            serde_json::from_str(
                r#"{"type":"thread.started","thread_id":"thread_xyz","session_id":"sess_1"}"#,
            )
            .unwrap(),
        ];
        session.capture_thread_id(&events);
        assert_eq!(session.id(), Some("thread_xyz"));
    }

    #[test]
    fn capture_thread_id_noop_when_absent() {
        let mut session = Session::new(test_codex());
        let events: Vec<JsonLineEvent> =
            vec![serde_json::from_str(r#"{"type":"message.created"}"#).unwrap()];
        session.capture_thread_id(&events);
        assert!(session.id().is_none());
    }

    #[test]
    fn try_capture_thread_id_from_stdout_parses_json() {
        let mut session = Session::new(test_codex());
        let stdout = r#"{"type":"thread.started","thread_id":"thread_err"}
{"type":"error","message":"something went wrong"}"#;
        session.try_capture_thread_id_from_stdout(stdout);
        assert_eq!(session.id(), Some("thread_err"));
    }

    #[test]
    fn try_capture_thread_id_from_stdout_ignores_garbage() {
        let mut session = Session::new(test_codex());
        session.try_capture_thread_id_from_stdout("not json\nalso not json");
        assert!(session.id().is_none());
    }

    #[test]
    fn debug_impl() {
        let session = Session::resume(test_codex(), "thread_dbg");
        let debug = format!("{session:?}");
        assert!(debug.contains("thread_dbg"));
        assert!(debug.contains("total_turns: 0"));
    }
}
