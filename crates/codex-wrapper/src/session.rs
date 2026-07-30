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
use crate::types::{JsonLineEvent, QueryResult};

/// A record of a single turn within a session.
#[derive(Debug, Clone)]
pub struct TurnRecord {
    /// The typed result for this turn, assembled from its JSONL events.
    ///
    /// Holding the assembled [`QueryResult`] rather than the raw event vector
    /// is what lets a session report cost: `cost_usd` lives on the terminal
    /// `completed` event and is otherwise discarded.
    pub result: QueryResult,
}

impl TurnRecord {
    /// The parsed JSONL events returned by this turn.
    #[must_use]
    pub fn events(&self) -> &[JsonLineEvent] {
        &self.result.events
    }

    /// Cost in USD for this turn, if the CLI reported one.
    #[must_use]
    pub fn cost_usd(&self) -> Option<f64> {
        self.result.cost_usd
    }
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

    /// Send a prompt, streaming events to `handler` as they arrive.
    ///
    /// The streaming equivalent of [`send`](Session::send): routes to `exec`
    /// or `exec resume` the same way, and leaves the session in the same
    /// state. Events are handed to `handler` as the CLI emits them and are
    /// also retained, so `thread_id`, history, and cost are captured exactly
    /// as they would be on the buffered path.
    ///
    /// Returns the full event stream for the turn.
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use codex_wrapper::{Codex, Session};
    ///
    /// # async fn example() -> codex_wrapper::Result<()> {
    /// let codex = Arc::new(Codex::builder().build()?);
    /// let mut session = Session::new(codex);
    ///
    /// session
    ///     .stream("summarize this repo", |event| {
    ///         println!("{}", event.event_type);
    ///     })
    ///     .await?;
    ///
    /// assert_eq!(session.total_turns(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stream<F>(
        &mut self,
        prompt: impl Into<String>,
        handler: F,
    ) -> Result<Vec<JsonLineEvent>>
    where
        F: FnMut(JsonLineEvent),
    {
        let prompt = prompt.into();
        match &self.thread_id {
            None => {
                self.stream_execute(ExecCommand::new(&prompt), handler)
                    .await
            }
            Some(id) => {
                let cmd = ExecResumeCommand::new()
                    .session_id(id.clone())
                    .prompt(prompt);
                self.stream_execute_resume(cmd, handler).await
            }
        }
    }

    /// Stream an [`ExecCommand`] with full control over its options.
    ///
    /// The streaming equivalent of [`execute`](Session::execute).
    pub async fn stream_execute<F>(
        &mut self,
        cmd: ExecCommand,
        mut handler: F,
    ) -> Result<Vec<JsonLineEvent>>
    where
        F: FnMut(JsonLineEvent),
    {
        let codex = Arc::clone(&self.codex);
        let mut collected = Vec::new();
        let outcome = cmd
            .stream(&codex, |event| {
                collected.push(event.clone());
                handler(event);
            })
            .await;
        self.finish_stream(collected, outcome)
    }

    /// Stream an [`ExecResumeCommand`] with full control over its options.
    ///
    /// The streaming equivalent of [`execute_resume`](Session::execute_resume).
    pub async fn stream_execute_resume<F>(
        &mut self,
        cmd: ExecResumeCommand,
        mut handler: F,
    ) -> Result<Vec<JsonLineEvent>>
    where
        F: FnMut(JsonLineEvent),
    {
        let codex = Arc::clone(&self.codex);
        let mut collected = Vec::new();
        let outcome = cmd
            .stream(&codex, |event| {
                collected.push(event.clone());
                handler(event);
            })
            .await;
        self.finish_stream(collected, outcome)
    }

    /// Record a streamed turn, or salvage `thread_id` from a failed one.
    ///
    /// A stream that fails partway has still delivered real events, so the
    /// `thread_id` is taken from what arrived. This mirrors the buffered
    /// path's recovery, which re-parses stdout for the same reason. No turn is
    /// recorded on failure: an incomplete stream has no terminal `completed`
    /// event, so its cost would be missing and would silently undercount
    /// [`total_cost`](Self::total_cost).
    fn finish_stream(
        &mut self,
        collected: Vec<JsonLineEvent>,
        outcome: Result<()>,
    ) -> Result<Vec<JsonLineEvent>> {
        match outcome {
            Ok(()) => Ok(self.record_turn(collected)),
            Err(e) => {
                self.capture_thread_id(&collected);
                Err(e)
            }
        }
    }

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

    /// The typed result of the most recent completed turn.
    #[must_use]
    pub fn last_result(&self) -> Option<&QueryResult> {
        self.history.last().map(|turn| &turn.result)
    }

    /// Sum of the per-turn costs the CLI reported, in USD.
    ///
    /// Turns where no cost was reported contribute nothing. The CLI does not
    /// always report `cost_usd`, so a total of `0.0` can mean either "nothing
    /// was spent" or "nothing was reported". Pair this with
    /// [`turns_missing_cost`](Self::turns_missing_cost) to tell those apart.
    #[must_use]
    pub fn total_cost(&self) -> f64 {
        self.history.iter().filter_map(TurnRecord::cost_usd).sum()
    }

    /// How many completed turns reported no cost.
    ///
    /// Non-zero means [`total_cost`](Self::total_cost) is an undercount rather
    /// than a full accounting.
    #[must_use]
    pub fn turns_missing_cost(&self) -> usize {
        self.history
            .iter()
            .filter(|turn| turn.cost_usd().is_none())
            .count()
    }

    /// Capture state from a completed turn and record it.
    ///
    /// Shared by the buffered and streaming paths so a streaming turn leaves
    /// the session in the same state a buffered one would.
    fn record_turn(&mut self, events: Vec<JsonLineEvent>) -> Vec<JsonLineEvent> {
        self.capture_thread_id(&events);
        let result = QueryResult::from_events(events);
        let events = result.events.clone();
        self.history.push(TurnRecord { result });
        events
    }

    /// Run an [`ExecCommand`] and record the turn.
    async fn run_exec(&mut self, cmd: ExecCommand) -> Result<Vec<JsonLineEvent>> {
        match cmd.execute_json_lines(&self.codex).await {
            Ok(events) => Ok(self.record_turn(events)),
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
            Ok(events) => Ok(self.record_turn(events)),
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

    /// Build a session backed by the fake-codex script the streaming tests use.
    ///
    /// Unix-only: the fake CLI is a bash script, matching how
    /// [`crate::streaming`] gates its own tests.
    #[cfg(unix)]
    fn streaming_session() -> Session {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-codex.sh");
        Session::new(Arc::new(
            Codex::builder()
                .binary("/bin/bash")
                .arg(script.to_str().unwrap())
                .build()
                .expect("bash must exist"),
        ))
    }

    fn turn(json: &[&str]) -> Vec<JsonLineEvent> {
        json.iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn record_turn_captures_cost_and_thread_id() {
        let mut session = Session::new(test_codex());
        session.record_turn(turn(&[
            r#"{"type":"thread.started","thread_id":"thread_1"}"#,
            r#"{"type":"completed","result":{"text":"hi","cost":0.25}}"#,
        ]));

        assert_eq!(session.id(), Some("thread_1"));
        assert_eq!(session.total_turns(), 1);
        assert_eq!(session.last_result().unwrap().cost_usd, Some(0.25));
        assert_eq!(session.last_result().unwrap().result, "hi");
        assert!((session.total_cost() - 0.25).abs() < f64::EPSILON);
        assert_eq!(session.turns_missing_cost(), 0);
    }

    #[test]
    fn total_cost_sums_across_turns() {
        let mut session = Session::new(test_codex());
        for cost in ["0.10", "0.20", "0.30"] {
            session.record_turn(turn(&[&format!(
                r#"{{"type":"completed","result":{{"text":"x","cost":{cost}}}}}"#
            )]));
        }
        assert_eq!(session.total_turns(), 3);
        assert!((session.total_cost() - 0.60).abs() < 1e-9);
        assert_eq!(session.turns_missing_cost(), 0);
    }

    /// The CLI does not always report cost. A total that silently skipped
    /// those turns would look authoritative while undercounting, so the count
    /// of unreported turns is exposed alongside it.
    #[test]
    fn unreported_cost_is_counted_not_hidden() {
        let mut session = Session::new(test_codex());
        session.record_turn(turn(&[
            r#"{"type":"completed","result":{"text":"x","cost":0.4}}"#,
        ]));
        session.record_turn(turn(&[r#"{"type":"completed","result":{"text":"y"}}"#]));

        assert!((session.total_cost() - 0.4).abs() < f64::EPSILON);
        assert_eq!(session.turns_missing_cost(), 1);
        assert_eq!(session.total_turns(), 2);
    }

    #[test]
    fn zero_cost_and_unreported_cost_are_distinguishable() {
        let mut reported = Session::new(test_codex());
        reported.record_turn(turn(&[
            r#"{"type":"completed","result":{"text":"x","cost":0.0}}"#,
        ]));

        let mut unreported = Session::new(test_codex());
        unreported.record_turn(turn(&[r#"{"type":"completed","result":{"text":"x"}}"#]));

        assert_eq!(reported.total_cost(), unreported.total_cost());
        assert_eq!(reported.turns_missing_cost(), 0);
        assert_eq!(unreported.turns_missing_cost(), 1);
    }

    #[test]
    fn turn_record_exposes_events_and_cost() {
        let mut session = Session::new(test_codex());
        session.record_turn(turn(&[
            r#"{"type":"message.created"}"#,
            r#"{"type":"completed","result":{"text":"x","cost":0.5}}"#,
        ]));

        let record = &session.history()[0];
        assert_eq!(record.events().len(), 2);
        assert_eq!(record.cost_usd(), Some(0.5));
    }

    #[test]
    fn last_result_is_none_before_any_turn() {
        let session = Session::new(test_codex());
        assert!(session.last_result().is_none());
        assert_eq!(session.total_cost(), 0.0);
        assert_eq!(session.turns_missing_cost(), 0);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn stream_delivers_events_and_records_the_turn() {
        let mut session = streaming_session();
        let mut seen = Vec::new();

        let events = session
            .stream("test prompt", |event| seen.push(event.event_type.clone()))
            .await
            .unwrap();

        assert!(seen.contains(&"completed".to_string()), "saw: {seen:?}");
        assert_eq!(events.len(), seen.len(), "handler and return value agree");

        // The streaming path must leave the same state a buffered turn would.
        assert_eq!(session.total_turns(), 1);
        assert_eq!(session.id(), Some("thread_test"));
        assert!((session.total_cost() - 0.001).abs() < f64::EPSILON);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn streaming_turns_accumulate_like_buffered_ones() {
        let mut session = streaming_session();
        session.stream("first", |_| {}).await.unwrap();
        // Second turn routes through exec resume, since thread_id is now set.
        session.stream("second", |_| {}).await.unwrap();

        assert_eq!(session.total_turns(), 2);
        assert!((session.total_cost() - 0.002).abs() < 1e-9);
        assert_eq!(session.turns_missing_cost(), 0);
    }
}
