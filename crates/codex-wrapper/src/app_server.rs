//! A JSON-RPC client for `codex app-server`.
//!
//! `codex exec` takes one prompt per process and cannot change a turn that is
//! already running. The app-server can: [`AppServerHandle::turn_steer`] adds
//! input to the active turn and [`AppServerHandle::turn_interrupt`] stops it.
//! [`AppServer`] runs the server over stdio and gives a caller its events as
//! they arrive, plus a way to answer the requests the server sends back, such
//! as approvals.
//!
//! The CLI marks `app-server` experimental, so this module is behind the
//! `app-server` feature and wraps the transport, not the whole protocol. The
//! four methods with typed helpers ([`thread_start`], [`turn_start`],
//! [`turn_steer`], [`turn_interrupt`]) were exercised against a real server;
//! [`request`] reaches any other method without a wrapper change.
//!
//! [`thread_start`]: AppServerHandle::thread_start
//! [`turn_start`]: AppServerHandle::turn_start
//! [`turn_steer`]: AppServerHandle::turn_steer
//! [`turn_interrupt`]: AppServerHandle::turn_interrupt
//! [`request`]: AppServerHandle::request
//!
//! # Example
//!
//! ```no_run
//! use codex_wrapper::Codex;
//! use codex_wrapper::app_server::{
//!     AppServer, ServerMessage, ThreadStartParams, TurnStartParams, UserInput,
//! };
//!
//! # async fn example() -> codex_wrapper::Result<()> {
//! let codex = Codex::builder().build()?;
//! let mut server = AppServer::builder(&codex).start().await?;
//!
//! let thread = server
//!     .thread_start(ThreadStartParams::new().ephemeral(true))
//!     .await?;
//! let turn = server
//!     .turn_start(TurnStartParams::text(&thread.id, "run `sleep 10`, then say DONE"))
//!     .await?;
//!
//! // A handle can be cloned into another task, for example one that
//! // serves a "steer" command while this task reads events.
//! let handle = server.handle();
//! handle
//!     .turn_steer(&thread.id, &turn.id, vec![UserInput::text("also say STEERED")])
//!     .await?;
//!
//! let mut answer = None;
//! while let Some(message) = server.next_message().await? {
//!     match message {
//!         ServerMessage::Notification(notification) => {
//!             if let Some(message) = notification.agent_message()
//!                 && message.is_final_answer()
//!             {
//!                 answer = Some(message.text);
//!             }
//!             if let Some(done) = notification.turn_completed() {
//!                 println!("{:?}: {answer:?}", done.turn.status);
//!                 break;
//!             }
//!         }
//!         ServerMessage::Request(request) => {
//!             // An unanswered request stalls the turn.
//!             server.respond_error(request.id, -32601, "not supported")?;
//!         }
//!         _ => {}
//!     }
//! }
//! server.shutdown().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Lifecycle and cancellation
//!
//! The server runs in its own process group (unless the client disables
//! [`CodexBuilder::process_group`](crate::CodexBuilder::process_group)).
//! That group does not include the shell commands a turn runs: the server
//! starts each in a group of its own. So there are two ways to stop, and they
//! differ in what happens to a command that is still running:
//!
//! - [`AppServer::shutdown`] closes the server's stdin and waits up to the
//!   client's termination grace for it to exit. The server stops its running
//!   commands and exits, in milliseconds when a turn is active. If it does
//!   not exit in time, this falls back to [`AppServer::terminate`]. Prefer it.
//! - [`AppServer::terminate`] has the settled contract of
//!   [`stream_exec_cancellable`](crate::streaming::stream_exec_cancellable):
//!   it asks the server's group to stop, waits through the grace period, kills
//!   what is left, and reaps the child before returning. A command the server
//!   started in its own group keeps running, as it does when `codex exec` is
//!   cancelled; the caller has to find and stop it.
//! - Dropping an [`AppServer`] kills the server's group without waiting, as
//!   dropping a streaming future does. The same caveat applies.
//!
//! [`AppServerHandle::turn_interrupt`] ends the turn, not the command it is
//! running.
//! - A failure that leaves the stream unusable (the output limit, or a line
//!   that is not JSON) kills the child. The error is returned from
//!   [`AppServer::next_message`] and from any request still waiting.
//!
//! # Limits
//!
//! [`CodexBuilder::output_limit`](crate::CodexBuilder::output_limit) caps the
//! total bytes read from stdout over the life of the session, as it does for
//! streaming. [`CodexBuilder::timeout`](crate::CodexBuilder::timeout) applies
//! to each request, including the handshake, and not to the session. The
//! server's stderr is drained continuously and the last 16 KiB is kept for
//! error messages.
//!
//! # Verified against codex-cli 0.149.0
//!
//! Each of these was read from a captured session, not inferred. Re-capture
//! after a CLI upgrade and correct this block in the same change.
//!
//! - Messages are newline-delimited JSON with no `jsonrpc` field. A
//!   notification may carry extra top-level fields (`emittedAtMs`).
//! - A request before `initialize` gets `{"error":{"code":-32600,"message":
//!   "Not initialized"},"id":N}`, a second `initialize` gets `Already
//!   initialized`, and an unknown method gets code -32600 with the valid
//!   method names in the message.
//! - `initialize` takes `clientInfo` and optionally
//!   `capabilities.optOutNotificationMethods`, which suppresses those
//!   notifications for the connection. The client then sends an `initialized`
//!   notification.
//! - `thread/start` accepts `cwd`, `ephemeral`, `approvalPolicy`, `sandbox`,
//!   and `model`. It answers with `{"thread":{"id":...},"model":...}` and
//!   sends a `thread/started` notification.
//! - `turn/start` accepts `threadId`, `input`, `model`, `effort`,
//!   `outputSchema`, `cwd`, and `approvalPolicy`. It answers with
//!   `{"turn":{"id":...,"status":"inProgress"}}`. With `outputSchema` the
//!   captured final message was JSON that followed the schema.
//! - A turn ends with a `turn/completed` notification whose `turn.status` is
//!   `completed` or `interrupted`. Whether `turn.items` holds the final
//!   answer depends on the CLI: 0.149.0 includes it (`itemsView` `summary`),
//!   0.145.0 always sends `items: []` (`notLoaded`). Both send an
//!   `item/completed` notification for the final `agentMessage` (`phase`
//!   `final_answer`) before `turn/completed`, so that is what
//!   [`Notification::agent_message`] decodes.
//! - `turn/steer` answers `{"turnId":...}` at once. The text reaches the model
//!   at the next item boundary, not immediately. With no active turn it fails
//!   with `no active turn to steer`; with one, but a different
//!   `expectedTurnId`, with ``expected active turn id `X` but found `Y` ``.
//! - `turn/interrupt` answers `{}`. With no active turn it fails with `no
//!   active turn to interrupt`; with a different `turnId`, with `expected
//!   active turn id X but found Y`. It ends the turn and leaves a running
//!   shell command alone.
//! - The server runs each shell command in a process group of its own, with
//!   the server as its parent. With a command running, closing stdin made the
//!   server exit 0 and took the command with it; SIGTERM to the server, SIGTERM
//!   to its group, and SIGKILL to its group each left the command running.
//! - When the approval policy asks, the server sends a request
//!   (`item/commandExecution/requestApproval`, numeric id starting at 0) and
//!   the turn waits. The reply is `{"id":0,"result":{"decision":"decline"}}`,
//!   after which the server sends a `serverRequest/resolved` notification.
//! - The server logs to stderr, in colour, including MCP startup errors.
//! - Closing stdin ends the server with status 0. That took 5 seconds in one
//!   capture where no thread had been started, and milliseconds after a turn.
//! - A startup failure (an unknown `-c` key under `--strict-config`) writes one
//!   line to stderr and exits 1.
//!
//! # Assumed, not verified
//!
//! - Server-initiated requests may use string ids; [`RequestId`] accepts both.
//! - A JSON object with neither `method` nor a matching response `id` is
//!   logged and ignored. A line that does not start with `{` is ignored, as
//!   [`streaming`](crate::streaming) ignores it. A line that starts with `{`
//!   and is not valid JSON ends the session.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{Notify, mpsc, oneshot};
use tracing::{Instrument, debug, warn};

use crate::Codex;
use crate::error::{Error, OutputStream, Result};
use crate::exec::{GroupKillGuard, SpanOutcome};
use crate::types::{ApprovalPolicy, SandboxMode};

/// How much of the server's stderr is kept for error messages.
const STDERR_TAIL_BYTES: usize = 16 * 1024;

/// How long a server that failed during the handshake is given to report why
/// it exited before it is terminated.
const STARTUP_EXIT_WAIT: Duration = Duration::from_secs(2);

/// How long to wait for the stderr reader to finish once the child has exited,
/// so the last lines it wrote make it into an error message.
const STDERR_SETTLE: Duration = Duration::from_millis(500);

/// A JSON-RPC request id.
///
/// The server has been seen to send numbers. The protocol allows strings, so
/// both are accepted, and a reply echoes whichever the server used.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    /// A numeric id.
    Number(i64),
    /// A string id.
    String(String),
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(n) => write!(f, "{n}"),
            Self::String(s) => f.write_str(s),
        }
    }
}

/// A message the server sent without being asked.
///
/// `params` is `null` when the server sent none.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    /// The method name, for example `turn/completed`.
    pub method: String,
    /// The method's parameters.
    pub params: Value,
}

impl Notification {
    /// Decode an `item/completed` notification for an agent message, or `None`
    /// for anything else.
    ///
    /// A turn may produce several: progress remarks (`commentary`) and then
    /// the answer (`final_answer`). Unlike `turn/completed`, this notification
    /// carries the text on every CLI version tested.
    #[must_use]
    pub fn agent_message(&self) -> Option<AgentMessage> {
        if self.method != "item/completed" {
            return None;
        }
        let item = self.params.get("item")?;
        if item.get("type")?.as_str()? != "agentMessage" {
            return None;
        }
        Some(AgentMessage {
            thread_id: self.params.get("threadId")?.as_str()?.to_string(),
            turn_id: self.params.get("turnId")?.as_str()?.to_string(),
            text: item.get("text")?.as_str()?.to_string(),
            phase: item
                .get("phase")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// Decode a `turn/completed` notification, or `None` for any other method
    /// or a payload that does not have the verified shape.
    #[must_use]
    pub fn turn_completed(&self) -> Option<TurnCompleted> {
        if self.method != "turn/completed" {
            return None;
        }
        serde_json::from_value(self.params.clone()).ok()
    }
}

/// A finished message from the model, decoded from an `item/completed`
/// notification by [`Notification::agent_message`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessage {
    /// The thread the message belongs to.
    pub thread_id: String,
    /// The turn the message belongs to.
    pub turn_id: String,
    /// The message text.
    pub text: String,
    /// `commentary` for a progress remark, `final_answer` for the answer.
    /// `None` if the server sent no phase.
    pub phase: Option<String>,
}

impl AgentMessage {
    /// Whether this is the turn's answer rather than a progress remark.
    #[must_use]
    pub fn is_final_answer(&self) -> bool {
        self.phase.as_deref() == Some("final_answer")
    }
}

/// A request the server sent and is waiting on, such as an approval.
///
/// Answer it with [`AppServerHandle::respond`] or
/// [`AppServerHandle::respond_error`]. The turn that raised it makes no
/// progress until then.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerRequest {
    /// The id to echo in the reply.
    pub id: RequestId,
    /// The method name, for example `item/commandExecution/requestApproval`.
    pub method: String,
    /// The method's parameters.
    pub params: Value,
}

/// One message from the server that is not the answer to a request.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ServerMessage {
    /// An event, such as `turn/started` or `item/completed`.
    Notification(Notification),
    /// A request that needs a reply.
    Request(ServerRequest),
}

/// How a turn is going, from its `status` field.
///
/// `inProgress`, `completed`, and `interrupted` were captured. `failed` comes
/// from the protocol schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum TurnStatus {
    /// The turn is running.
    InProgress,
    /// The turn finished normally.
    Completed,
    /// The turn was stopped by `turn/interrupt`.
    Interrupted,
    /// The turn failed; see [`Turn::error`].
    Failed,
    /// A status this version of the wrapper does not know.
    #[serde(other)]
    Unknown,
}

/// A turn, as reported in `turn/start` responses and `turn/completed`
/// notifications.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    /// The turn's id, needed for [`AppServerHandle::turn_steer`] and
    /// [`AppServerHandle::turn_interrupt`].
    pub id: String,
    /// The turn's status.
    pub status: TurnStatus,
    /// The items the payload includes. Empty for a turn that was interrupted.
    #[serde(default)]
    pub items: Vec<Value>,
    /// Why the turn failed. Only set when the status is
    /// [`TurnStatus::Failed`].
    #[serde(default)]
    pub error: Option<Value>,
    /// How long the turn ran, in milliseconds, once it has ended.
    #[serde(default)]
    pub duration_ms: Option<i64>,
}

impl Turn {
    /// The text of the turn's final answer: the last `agentMessage` item in
    /// [`items`](Self::items) whose `phase` is `final_answer`.
    ///
    /// Only newer servers put the answer in `turn/completed` (0.149.0 does,
    /// 0.145.0 sends no items), and an interrupted turn never has one. To get
    /// the answer from any version, keep the last
    /// [`Notification::agent_message`] whose
    /// [`is_final_answer`](AgentMessage::is_final_answer) is true.
    #[must_use]
    pub fn final_message(&self) -> Option<&str> {
        self.items
            .iter()
            .rev()
            .filter(|item| {
                item.get("type").and_then(Value::as_str) == Some("agentMessage")
                    && item.get("phase").and_then(Value::as_str) == Some("final_answer")
            })
            .find_map(|item| item.get("text").and_then(Value::as_str))
    }
}

/// The payload of a `turn/completed` notification.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCompleted {
    /// The thread the turn belongs to.
    pub thread_id: String,
    /// The turn, in its final state.
    pub turn: Turn,
}

/// A thread, as returned by `thread/start`.
#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    /// The thread id, needed to start turns on it.
    pub id: String,
    /// The model the thread will use, when the server reports one.
    pub model: Option<String>,
    /// The complete response, for fields this type does not name.
    pub raw: Value,
}

/// What the server reported about itself in the `initialize` response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerInfo {
    /// The server's user agent string, which includes the CLI version.
    pub user_agent: String,
    /// The Codex home directory the server is using.
    pub codex_home: Option<String>,
    /// `unix` or `windows`.
    pub platform_family: Option<String>,
    /// The operating system name.
    pub platform_os: Option<String>,
}

/// One piece of user input for a turn.
#[derive(Debug, Clone, PartialEq)]
pub enum UserInput {
    /// Plain text.
    Text(String),
    /// Any other input item, sent as given. See the protocol schema for the
    /// shapes the server accepts.
    Raw(Value),
}

impl UserInput {
    /// A text input.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

impl Serialize for UserInput {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Text(text) => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("type", "text")?;
                map.serialize_entry("text", text)?;
                // Sent as the captured client sent it. The schema gives the
                // field a default, but a server that requires it is not ruled
                // out by that.
                map.serialize_entry("text_elements", &Vec::<Value>::new())?;
                map.end()
            }
            Self::Raw(value) => value.serialize(serializer),
        }
    }
}

/// Parameters for `thread/start`.
///
/// Every field is optional. Fields this type does not name can be set with
/// [`param`](Self::param).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_policy: Option<ApprovalPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<SandboxMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ephemeral: Option<bool>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl ThreadStartParams {
    /// Parameters with nothing set, which starts a thread with the server's
    /// defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The thread's working directory.
    #[must_use]
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// The model the thread uses.
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// When the server asks before running a command or applying a change.
    #[must_use]
    pub fn approval_policy(mut self, policy: ApprovalPolicy) -> Self {
        self.approval_policy = Some(policy);
        self
    }

    /// The sandbox for commands the model runs.
    #[must_use]
    pub fn sandbox(mut self, mode: SandboxMode) -> Self {
        self.sandbox = Some(mode);
        self
    }

    /// Whether the thread is kept out of the session history.
    #[must_use]
    pub fn ephemeral(mut self, ephemeral: bool) -> Self {
        self.ephemeral = Some(ephemeral);
        self
    }

    /// Set a parameter this type has no method for, using the protocol's
    /// camelCase name. It is sent as given and not checked.
    #[must_use]
    pub fn param(mut self, name: impl Into<String>, value: Value) -> Self {
        self.extra.insert(name.into(), value);
        self
    }
}

/// Parameters for `turn/start`.
///
/// The thread and the input are required. The other fields override the
/// thread's settings for this turn and, per the protocol, later ones. Fields
/// this type does not name can be set with [`param`](Self::param).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartParams {
    thread_id: String,
    input: Vec<UserInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_policy: Option<ApprovalPolicy>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl TurnStartParams {
    /// A turn on `thread_id` with the given input.
    #[must_use]
    pub fn new(thread_id: impl Into<String>, input: Vec<UserInput>) -> Self {
        Self {
            thread_id: thread_id.into(),
            input,
            model: None,
            effort: None,
            output_schema: None,
            cwd: None,
            approval_policy: None,
            extra: Map::new(),
        }
    }

    /// A turn on `thread_id` whose input is one text prompt.
    #[must_use]
    pub fn text(thread_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self::new(thread_id, vec![UserInput::text(prompt)])
    }

    /// The model for this turn.
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// The reasoning effort for this turn. The valid values depend on the
    /// model, so this is a string.
    #[must_use]
    pub fn effort(mut self, effort: impl Into<String>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    /// A JSON Schema the final message must follow.
    #[must_use]
    pub fn output_schema(mut self, schema: Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// The working directory for this turn.
    #[must_use]
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// When the server asks before running a command or applying a change.
    #[must_use]
    pub fn approval_policy(mut self, policy: ApprovalPolicy) -> Self {
        self.approval_policy = Some(policy);
        self
    }

    /// Set a parameter this type has no method for, using the protocol's
    /// camelCase name. It is sent as given and not checked.
    #[must_use]
    pub fn param(mut self, name: impl Into<String>, value: Value) -> Self {
        self.extra.insert(name.into(), value);
        self
    }
}

/// A JSON-RPC error the server returned for a request.
struct RpcFailure {
    code: i64,
    message: String,
    data: Option<Value>,
}

/// Why the connection to the server ended.
#[derive(Debug, Clone)]
enum Closed {
    /// The server closed its stdout: it exited, or was stopped.
    Eof,
    /// Stdout passed the byte ceiling.
    OutputLimit { limit_bytes: usize },
    /// The stream cannot be trusted any more, or could not be read or written.
    Broken(String),
    /// The client ended the session.
    Terminated,
}

/// What a waiting request gets back.
enum Reply {
    Result(Value),
    Rpc(RpcFailure),
    Closed(Closed),
}

/// What the reader hands to [`AppServer::next_message`].
enum Event {
    Message(ServerMessage),
    Closed(Closed),
}

/// What the writer task is asked to do.
enum Outgoing {
    Line(Vec<u8>),
    /// Stop writing and drop stdin, which the server sees as end of input.
    Close,
}

struct State {
    pending: HashMap<i64, oneshot::Sender<Reply>>,
    closed: Option<Closed>,
}

/// State shared by the handles, the owner, and the three I/O tasks.
struct Shared {
    state: Mutex<State>,
    next_id: AtomicI64,
    stderr_tail: Mutex<Vec<u8>>,
    /// Set, and `stderr_finished` notified, when the stderr reader has seen
    /// end of file, so its last lines are in `stderr_tail`.
    stderr_done: AtomicBool,
    stderr_finished: Notify,
    events: mpsc::UnboundedSender<Event>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Nothing here panics while holding a lock, so a poisoned mutex holds
    // consistent data. Recovering keeps one panicking task from taking the
    // whole session down with it.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// End the session: fail every waiting request and tell the reader side.
    /// The first reason wins.
    fn fail(&self, reason: Closed) {
        let pending = {
            let mut state = locked(&self.state);
            if state.closed.is_some() {
                return;
            }
            state.closed = Some(reason.clone());
            std::mem::take(&mut state.pending)
        };
        for (_, waiter) in pending {
            let _ = waiter.send(Reply::Closed(reason.clone()));
        }
        let _ = self.events.send(Event::Closed(reason));
    }

    fn new(events: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            state: Mutex::new(State {
                pending: HashMap::new(),
                closed: None,
            }),
            next_id: AtomicI64::new(0),
            stderr_tail: Mutex::new(Vec::new()),
            stderr_done: AtomicBool::new(false),
            stderr_finished: Notify::new(),
            events,
        }
    }

    /// Wait, for a short while, until the stderr reader has read everything
    /// the server wrote. Stdout and stderr close together when the server
    /// exits, but the two readers are separate tasks, so an error built from
    /// the tail straight away could miss the last lines.
    async fn stderr_settled(&self) {
        let wait = async {
            loop {
                let notified = self.stderr_finished.notified();
                if self.stderr_done.load(Ordering::Acquire) {
                    return;
                }
                notified.await;
            }
        };
        let _ = tokio::time::timeout(STDERR_SETTLE, wait).await;
    }

    /// The last lines the server wrote to stderr, without colour codes.
    fn stderr_summary(&self) -> String {
        let tail = locked(&self.stderr_tail);
        let text = strip_ansi(&String::from_utf8_lossy(&tail));
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let start = lines.len().saturating_sub(5);
        lines[start..].join("\n")
    }

    fn error_for(&self, reason: &Closed, method: Option<&str>) -> Error {
        let protocol = |message: String| Error::AppServerProtocol { message };
        match reason {
            Closed::OutputLimit { limit_bytes } => Error::OutputLimitExceeded {
                stream: OutputStream::Stdout,
                limit_bytes: *limit_bytes,
            },
            Closed::Terminated => protocol("the app-server session was ended by the client".into()),
            Closed::Broken(message) => protocol(message.clone()),
            Closed::Eof => {
                let stderr = self.stderr_summary();
                let while_waiting =
                    method.map_or_else(String::new, |m| format!(" before answering `{m}`"));
                if stderr.is_empty() {
                    protocol(format!("codex app-server closed its output{while_waiting}"))
                } else {
                    protocol(format!(
                        "codex app-server closed its output{while_waiting}; stderr: {stderr}"
                    ))
                }
            }
        }
    }
}

/// Remove ANSI colour sequences (`ESC [ ... letter`) from server log lines.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Removes a waiting request's entry when the request future is dropped, so a
/// cancelled request does not leave its slot behind.
struct PendingSlot<'a> {
    shared: &'a Shared,
    id: i64,
}

impl Drop for PendingSlot<'_> {
    fn drop(&mut self) {
        locked(&self.shared.state).pending.remove(&self.id);
    }
}

fn json_error(context: &str, source: serde_json::Error) -> Error {
    Error::Json {
        message: format!("{context}: {source}"),
        source,
    }
}

fn request_line(id: i64, method: &str, params: &Value) -> Result<Vec<u8>> {
    let method = serde_json::to_string(method).map_err(|e| json_error("method name", e))?;
    let mut line = format!("{{\"id\":{id},\"method\":{method}");
    if !params.is_null() {
        let params = serde_json::to_string(params).map_err(|e| json_error("request params", e))?;
        line.push_str(",\"params\":");
        line.push_str(&params);
    }
    line.push_str("}\n");
    Ok(line.into_bytes())
}

fn notification_line(method: &str, params: &Value) -> Result<Vec<u8>> {
    let method = serde_json::to_string(method).map_err(|e| json_error("method name", e))?;
    let mut line = format!("{{\"method\":{method}");
    if !params.is_null() {
        let params =
            serde_json::to_string(params).map_err(|e| json_error("notification params", e))?;
        line.push_str(",\"params\":");
        line.push_str(&params);
    }
    line.push_str("}\n");
    Ok(line.into_bytes())
}

fn reply_line(id: &RequestId, body_key: &str, body: &Value) -> Result<Vec<u8>> {
    let id = serde_json::to_string(id).map_err(|e| json_error("request id", e))?;
    let body = serde_json::to_string(body).map_err(|e| json_error("reply body", e))?;
    Ok(format!("{{\"id\":{id},\"{body_key}\":{body}}}\n").into_bytes())
}

/// A cloneable handle for sending to the server.
///
/// It can send requests and answer the server's, but not read events: that
/// stays with the [`AppServer`], so a task can steer or interrupt a turn while
/// another reads its notifications. Every method fails once the session has
/// ended.
#[derive(Clone)]
pub struct AppServerHandle {
    shared: Arc<Shared>,
    writer: mpsc::UnboundedSender<Outgoing>,
    timeout: Option<Duration>,
}

impl std::fmt::Debug for AppServerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppServerHandle")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl AppServerHandle {
    fn closed_error(&self, method: Option<&str>) -> Error {
        let reason = locked(&self.shared.state).closed.clone();
        match reason {
            Some(reason) => self.shared.error_for(&reason, method),
            None => Error::AppServerProtocol {
                message: "the app-server session is not accepting messages".into(),
            },
        }
    }

    fn enqueue(&self, line: Vec<u8>, method: Option<&str>) -> Result<()> {
        if let Some(reason) = &locked(&self.shared.state).closed {
            return Err(self.shared.error_for(reason, method));
        }
        self.writer
            .send(Outgoing::Line(line))
            .map_err(|_| self.closed_error(method))
    }

    /// Send a request and wait for its answer, returning the `result` value.
    ///
    /// Requests may run concurrently; each answer is matched by id. A JSON-RPC
    /// error from the server is [`Error::AppServerRpc`]. The client's
    /// [`timeout`](crate::CodexBuilder::timeout), when set, bounds the wait
    /// and yields [`Error::Timeout`]. Pass [`Value::Null`] for no parameters.
    ///
    /// # Errors
    ///
    /// Fails if the server rejects the request, the wait times out, or the
    /// session ends first.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let line = request_line(id, method, &params)?;
        let (waiter, answer) = oneshot::channel();
        {
            let mut state = locked(&self.shared.state);
            if let Some(reason) = &state.closed {
                return Err(self.shared.error_for(reason, Some(method)));
            }
            state.pending.insert(id, waiter);
        }
        let _slot = PendingSlot {
            shared: &self.shared,
            id,
        };
        self.writer
            .send(Outgoing::Line(line))
            .map_err(|_| self.closed_error(Some(method)))?;

        let reply = match self.timeout {
            Some(limit) => {
                tokio::time::timeout(limit, answer)
                    .await
                    .map_err(|_| Error::Timeout {
                        timeout_seconds: limit.as_secs(),
                    })?
            }
            None => answer.await,
        };
        match reply {
            Ok(Reply::Result(result)) => Ok(result),
            Ok(Reply::Rpc(failure)) => Err(Error::AppServerRpc {
                method: method.to_string(),
                code: failure.code,
                message: failure.message,
                data: failure.data,
            }),
            Ok(Reply::Closed(reason)) => Err(self.shared.error_for(&reason, Some(method))),
            Err(_) => Err(self.closed_error(Some(method))),
        }
    }

    /// Send a notification, which has no answer. Returns once it is queued
    /// for writing.
    ///
    /// # Errors
    ///
    /// Fails if the session has ended.
    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.enqueue(notification_line(method, &params)?, Some(method))
    }

    /// Answer a [`ServerRequest`] with a result. Returns once it is queued for
    /// writing.
    ///
    /// # Errors
    ///
    /// Fails if the session has ended.
    pub fn respond(&self, id: RequestId, result: Value) -> Result<()> {
        self.enqueue(reply_line(&id, "result", &result)?, None)
    }

    /// Answer a [`ServerRequest`] with a JSON-RPC error. Use it for a request
    /// the caller does not handle, so the turn that raised it is not left
    /// waiting. Returns once it is queued for writing.
    ///
    /// # Errors
    ///
    /// Fails if the session has ended.
    pub fn respond_error(
        &self,
        id: RequestId,
        code: i64,
        message: impl Into<String>,
    ) -> Result<()> {
        let body = json!({ "code": code, "message": message.into() });
        self.enqueue(reply_line(&id, "error", &body)?, None)
    }

    /// Start a thread (`thread/start`).
    ///
    /// # Errors
    ///
    /// As [`request`](Self::request), and a protocol error if the response has
    /// no thread id.
    pub async fn thread_start(&self, params: ThreadStartParams) -> Result<Thread> {
        let params =
            serde_json::to_value(params).map_err(|e| json_error("thread/start params", e))?;
        let raw = self.request("thread/start", params).await?;
        let id = raw
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::AppServerProtocol {
                message: "thread/start returned no thread id".into(),
            })?
            .to_string();
        let model = raw.get("model").and_then(Value::as_str).map(str::to_string);
        Ok(Thread { id, model, raw })
    }

    /// Start a turn (`turn/start`). The turn runs in the background: watch for
    /// its `turn/completed` notification.
    ///
    /// # Errors
    ///
    /// As [`request`](Self::request), and a protocol error if the response has
    /// no turn.
    pub async fn turn_start(&self, params: TurnStartParams) -> Result<Turn> {
        let params =
            serde_json::to_value(params).map_err(|e| json_error("turn/start params", e))?;
        let result = self.request("turn/start", params).await?;
        let turn = result.get("turn").cloned().unwrap_or(Value::Null);
        serde_json::from_value(turn).map_err(|e| Error::AppServerProtocol {
            message: format!("turn/start returned no usable turn: {e}"),
        })
    }

    /// Add input to a running turn (`turn/steer`) and return the id of the
    /// turn it landed on.
    ///
    /// The server delivers it to the model at the next item boundary, so a
    /// long tool call delays it. `expected_turn_id` must be the active turn.
    /// With no active turn the server fails the request with `no active turn
    /// to steer`, and with a different one with `expected active turn id ...`;
    /// both are [`Error::AppServerRpc`].
    ///
    /// # Errors
    ///
    /// As [`request`](Self::request).
    pub async fn turn_steer(
        &self,
        thread_id: &str,
        expected_turn_id: &str,
        input: Vec<UserInput>,
    ) -> Result<String> {
        let params = json!({
            "threadId": thread_id,
            "expectedTurnId": expected_turn_id,
            "input": input,
        });
        let result = self.request("turn/steer", params).await?;
        result
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::AppServerProtocol {
                message: "turn/steer returned no turn id".into(),
            })
    }

    /// Stop a running turn (`turn/interrupt`). The turn then ends with a
    /// `turn/completed` notification whose status is
    /// [`TurnStatus::Interrupted`].
    ///
    /// With no active turn the server fails the request with `no active turn
    /// to interrupt`, and with a different one with `expected active turn id
    /// ...`; both are [`Error::AppServerRpc`].
    ///
    /// # Errors
    ///
    /// As [`request`](Self::request).
    pub async fn turn_interrupt(&self, thread_id: &str, turn_id: &str) -> Result<()> {
        let params = json!({ "threadId": thread_id, "turnId": turn_id });
        self.request("turn/interrupt", params).await.map(|_| ())
    }
}

/// Builds and starts an [`AppServer`].
///
/// Created by [`AppServer::builder`].
#[derive(Debug, Clone)]
pub struct AppServerBuilder<'a> {
    codex: &'a Codex,
    client_name: String,
    client_version: String,
    opt_out: Vec<String>,
}

impl<'a> AppServerBuilder<'a> {
    /// Name and version reported to the server in `initialize`. The server
    /// includes them in its user agent. Defaults to `codex-wrapper` and this
    /// crate's version.
    #[must_use]
    pub fn client_info(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.client_name = name.into();
        self.client_version = version.into();
        self
    }

    /// Ask the server not to send a notification method on this connection,
    /// by exact name. Streaming deltas (`item/agentMessage/delta`) are the
    /// usual candidate for a caller that only needs completed items.
    #[must_use]
    pub fn opt_out_notification(mut self, method: impl Into<String>) -> Self {
        self.opt_out.push(method.into());
        self
    }

    /// The arguments [`start`](Self::start) passes after the client's global
    /// args: `app-server --listen stdio://`.
    ///
    /// `--listen stdio://` is the default transport. It is passed anyway so a
    /// change of default cannot move this client onto a socket.
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        vec!["app-server".into(), "--listen".into(), "stdio://".into()]
    }

    /// The command [`start`](Self::start) will run, as a copy-pasteable shell
    /// string, including the client's global args.
    #[must_use]
    pub fn to_command_string(&self) -> String {
        crate::exec::command_string(self.codex, self.args())
    }

    /// Spawn `codex app-server` and complete the handshake.
    ///
    /// The command is `codex [global args] app-server --listen stdio://`. The
    /// client's binary, environment, working directory, process-group policy,
    /// `die_with_parent`, and `on_spawn` observer apply. The handshake is
    /// bounded by the client's timeout, when set.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the process cannot be spawned. If the server exits
    /// before the handshake finishes, the failure is classified from its
    /// stderr as any failed command is, so a rejected `-c` key is
    /// [`Error::Config`]. Otherwise the handshake's own error.
    pub async fn start(self) -> Result<AppServer> {
        let codex = self.codex;
        let args = crate::exec::assemble_args(codex, self.args());
        let span = crate::exec::command_span("codex.app_server", codex, &args);
        let command_text = format!("{} {}", codex.binary.display(), args.join(" "));
        debug!(binary = %codex.binary.display(), args = ?args, "starting codex app-server");

        let mut outcome = SpanOutcome::start(span.clone());
        let mut command = Command::new(&codex.binary);
        command.args(&args);
        command.stdin(std::process::Stdio::piped());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        // As for streaming: dropping the client must not leave the server
        // running with no handle left to stop it.
        command.kill_on_drop(true);
        crate::exec::own_process_group(&mut command, codex.process_group);
        crate::exec::apply_die_with_parent(&mut command, codex.die_with_parent);
        if let Some(dir) = &codex.working_dir {
            command.current_dir(dir);
        }
        crate::exec::apply_child_environment(&mut command, codex.clear_env, &codex.env);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                outcome.settle("error", None);
                return Err(Error::Io {
                    message: format!("failed to spawn codex app-server: {e}"),
                    source: e,
                    working_dir: codex.working_dir.clone(),
                });
            }
        };
        let group =
            crate::exec::arm_and_notify(codex.process_group, child.id(), codex.on_spawn.as_ref());
        let pid = child.id();

        let stdin = child.stdin.take().expect("stdin was configured as piped");
        let stdout = child.stdout.take().expect("stdout was configured as piped");
        let stderr = child.stderr.take().expect("stderr was configured as piped");

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (writer_tx, writer_rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared::new(events_tx));

        tokio::spawn(read_stdout(
            stdout,
            Arc::clone(&shared),
            codex.output_limit,
            codex.working_dir.clone(),
            KillTarget {
                pid,
                group: codex.process_group,
            },
        ));
        tokio::spawn(drain_stderr(stderr, Arc::clone(&shared)));
        tokio::spawn(write_stdin(stdin, writer_rx, Arc::clone(&shared)));

        let handle = AppServerHandle {
            shared,
            writer: writer_tx,
            timeout: codex.timeout,
        };
        let mut server = AppServer {
            handle,
            events: events_rx,
            finished: false,
            child,
            group,
            outcome,
            termination_grace: codex.termination_grace,
            working_dir: codex.working_dir.clone(),
            command: command_text,
            info: ServerInfo::default(),
        };

        let handshake = initialize(
            &server.handle,
            &self.client_name,
            &self.client_version,
            &self.opt_out,
        )
        .instrument(span)
        .await;
        match handshake {
            Ok(info) => {
                server.info = info;
                Ok(server)
            }
            Err(error) => Err(server.abandon(error).await),
        }
    }
}

async fn initialize(
    handle: &AppServerHandle,
    name: &str,
    version: &str,
    opt_out: &[String],
) -> Result<ServerInfo> {
    let mut params = json!({ "clientInfo": { "name": name, "version": version } });
    if !opt_out.is_empty() {
        params["capabilities"] = json!({ "optOutNotificationMethods": opt_out });
    }
    let result = handle.request("initialize", params).await?;
    let info = serde_json::from_value(result).map_err(|e| Error::AppServerProtocol {
        message: format!("initialize returned an unexpected response: {e}"),
    })?;
    handle.notify("initialized", Value::Null)?;
    Ok(info)
}

/// Which process a fatal stream error stops.
#[derive(Clone, Copy)]
struct KillTarget {
    pid: Option<u32>,
    group: bool,
}

impl KillTarget {
    /// Stop the server now. The stream is unusable and nothing will read what
    /// the server writes next, so leaving it running would only fill the pipe.
    fn kill(self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            if self.group {
                crate::exec::signal_group(pid, libc::SIGKILL);
            } else if let Ok(pid) = i32::try_from(pid) {
                // SAFETY: `kill` on a pid this client spawned. If the child
                // has already been reaped the call fails, which is the state
                // we want.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
        #[cfg(not(unix))]
        let _ = self;
    }
}

/// Read the server's stdout: route answers to waiting requests and everything
/// else to the event queue.
async fn read_stdout(
    stdout: ChildStdout,
    shared: Arc<Shared>,
    output_limit: Option<usize>,
    working_dir: Option<PathBuf>,
    kill: KillTarget,
) {
    let mut reader = BufReader::new(stdout);
    let mut captured = 0usize;
    let reason = loop {
        let line = match crate::streaming::read_bounded_line(
            &mut reader,
            &mut captured,
            output_limit,
            OutputStream::Stdout,
            &working_dir,
        )
        .await
        {
            Ok(Some(line)) => line,
            Ok(None) => {
                shared.stderr_settled().await;
                break Closed::Eof;
            }
            Err(Error::OutputLimitExceeded { limit_bytes, .. }) => {
                kill.kill();
                break Closed::OutputLimit { limit_bytes };
            }
            Err(error) => {
                kill.kill();
                break Closed::Broken(error.to_string());
            }
        };
        if !line.trim_start().starts_with('{') {
            debug!(
                bytes = line.len(),
                "ignoring a non-JSON line from codex app-server"
            );
            continue;
        }
        if let Err(reason) = route(&line, &shared) {
            kill.kill();
            break reason;
        }
    };
    shared.fail(reason);
}

/// Classify one line from the server and deliver it.
fn route(line: &str, shared: &Shared) -> std::result::Result<(), Closed> {
    let value: Value = serde_json::from_str(line).map_err(|e| {
        let head: String = line.chars().take(120).collect();
        Closed::Broken(format!("codex app-server sent invalid JSON ({e}): {head}"))
    })?;
    let method = value.get("method").and_then(Value::as_str);
    let id = value.get("id");
    match (method, id) {
        (Some(method), Some(id)) => {
            let Ok(id) = serde_json::from_value::<RequestId>(id.clone()) else {
                warn!(method, "ignoring a server request with an unusable id");
                return Ok(());
            };
            let request = ServerRequest {
                id,
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            };
            let _ = shared
                .events
                .send(Event::Message(ServerMessage::Request(request)));
        }
        (Some(method), None) => {
            let notification = Notification {
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            };
            let _ = shared
                .events
                .send(Event::Message(ServerMessage::Notification(notification)));
        }
        (None, Some(id)) if value.get("result").is_some() || value.get("error").is_some() => {
            deliver_answer(&value, id, shared);
        }
        _ => warn!("ignoring a JSON object from codex app-server that is not a message"),
    }
    Ok(())
}

fn deliver_answer(value: &Value, id: &Value, shared: &Shared) {
    let Some(id) = id.as_i64() else {
        // The server answers a request it could not parse with a null id.
        debug!("ignoring an answer with no request id");
        return;
    };
    let waiter = locked(&shared.state).pending.remove(&id);
    let Some(waiter) = waiter else {
        // The request was cancelled or timed out before the answer arrived.
        debug!(id, "ignoring an answer nobody is waiting for");
        return;
    };
    let reply = match value.get("error") {
        Some(error) => Reply::Rpc(RpcFailure {
            code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            data: error.get("data").cloned(),
        }),
        None => Reply::Result(value.get("result").cloned().unwrap_or(Value::Null)),
    };
    let _ = waiter.send(reply);
}

/// Drain stderr for the life of the server. The server logs a lot there, and a
/// pipe nobody reads fills and stops it.
async fn drain_stderr(mut stderr: ChildStderr, shared: Arc<Shared>) {
    let mut buffer = [0u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => {
                shared.stderr_done.store(true, Ordering::Release);
                shared.stderr_finished.notify_waiters();
                break;
            }
            Ok(read) => {
                let mut tail = locked(&shared.stderr_tail);
                tail.extend_from_slice(&buffer[..read]);
                if tail.len() > STDERR_TAIL_BYTES {
                    let excess = tail.len() - STDERR_TAIL_BYTES;
                    tail.drain(..excess);
                }
            }
        }
    }
}

/// Write queued lines to the server's stdin, one whole line at a time.
///
/// Writing happens here, not in the request future, so a request that is
/// dropped part way through cannot leave half a line on the wire.
async fn write_stdin(
    mut stdin: ChildStdin,
    mut lines: mpsc::UnboundedReceiver<Outgoing>,
    shared: Arc<Shared>,
) {
    while let Some(outgoing) = lines.recv().await {
        match outgoing {
            Outgoing::Line(line) => {
                let written = async {
                    stdin.write_all(&line).await?;
                    stdin.flush().await
                }
                .await;
                if let Err(error) = written {
                    shared.fail(Closed::Broken(format!(
                        "failed to write to codex app-server stdin: {error}"
                    )));
                    break;
                }
            }
            Outgoing::Close => break,
        }
    }
    // Dropping stdin closes the pipe, which the server treats as the end of
    // its input.
}

/// A running `codex app-server`.
///
/// Requests go through [`AppServerHandle`], which this type dereferences to,
/// so `server.turn_start(..)` works directly. [`handle`](Self::handle) gives a
/// clone that can be moved to another task. Events are read with
/// [`next_message`](Self::next_message), which needs `&mut self`.
///
/// Dropping the value kills the server's process group without waiting; use
/// [`shutdown`](Self::shutdown) or [`terminate`](Self::terminate) to stop it
/// and wait.
pub struct AppServer {
    handle: AppServerHandle,
    events: mpsc::UnboundedReceiver<Event>,
    finished: bool,
    child: Child,
    group: GroupKillGuard,
    outcome: SpanOutcome,
    termination_grace: Duration,
    working_dir: Option<PathBuf>,
    command: String,
    info: ServerInfo,
}

impl std::fmt::Debug for AppServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppServer")
            .field("pid", &self.child.id())
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Deref for AppServer {
    type Target = AppServerHandle;

    fn deref(&self) -> &AppServerHandle {
        &self.handle
    }
}

impl AppServer {
    /// Start configuring a server for `codex`.
    #[must_use]
    pub fn builder(codex: &Codex) -> AppServerBuilder<'_> {
        AppServerBuilder {
            codex,
            client_name: "codex-wrapper".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            opt_out: Vec::new(),
        }
    }

    /// What the server reported about itself during the handshake.
    #[must_use]
    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    /// The server process's id, while it is running.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// A cloneable handle for sending requests from other tasks.
    #[must_use]
    pub fn handle(&self) -> AppServerHandle {
        self.handle.clone()
    }

    /// The next notification or server request, in the order the server sent
    /// them.
    ///
    /// Returns `Ok(None)` once the server's output has ended, after every
    /// earlier message has been returned. That is how a server that exited on
    /// its own looks; [`shutdown`](Self::shutdown) then reports its exit
    /// status. Cancel-safe: dropping the future loses no message.
    ///
    /// # Errors
    ///
    /// [`Error::OutputLimitExceeded`] or [`Error::AppServerProtocol`] when the
    /// stream became unusable. The server has been killed by then. The error
    /// is returned once; later calls return `Ok(None)`.
    pub async fn next_message(&mut self) -> Result<Option<ServerMessage>> {
        if self.finished {
            return Ok(None);
        }
        match self.events.recv().await {
            Some(Event::Message(message)) => Ok(Some(message)),
            Some(Event::Closed(reason)) => {
                self.finished = true;
                match reason {
                    Closed::Eof | Closed::Terminated => Ok(None),
                    reason => Err(self.handle.shared.error_for(&reason, None)),
                }
            }
            None => {
                self.finished = true;
                Ok(None)
            }
        }
    }

    /// End the session and wait for the server to exit.
    ///
    /// Closes the server's stdin and waits up to the client's termination
    /// grace for it to exit on its own. If it does not, this falls back to
    /// [`terminate`](Self::terminate) and returns `Ok`, since the caller asked
    /// for the end.
    ///
    /// # Errors
    ///
    /// If the server exits with a non-zero status by itself, the failure is
    /// classified from its stderr like any failed command
    /// ([`Error::CommandFailed`] and its refinements).
    pub async fn shutdown(mut self) -> Result<()> {
        let _ = self.handle.writer.send(Outgoing::Close);
        match tokio::time::timeout(self.termination_grace, self.child.wait()).await {
            Ok(Ok(status)) => {
                self.group.disarm();
                self.handle.shared.stderr_settled().await;
                self.handle.shared.fail(Closed::Terminated);
                let code = status.code().unwrap_or(-1);
                if status.success() {
                    self.outcome.settle("ok", Some(code));
                    return Ok(());
                }
                self.outcome.settle("failed", Some(code));
                Err(Error::from_command_failure(
                    self.command.clone(),
                    code,
                    String::new(),
                    self.handle.shared.stderr_summary(),
                    self.working_dir.clone(),
                ))
            }
            Ok(Err(error)) => Err(Error::Io {
                message: format!("failed to wait on codex app-server: {error}"),
                source: error,
                working_dir: self.working_dir.clone(),
            }),
            Err(_) => self.terminate().await,
        }
    }

    /// Stop the server and its process group, and wait until it is gone.
    ///
    /// Asks the group to stop, waits through the client's termination grace,
    /// kills what remains, and reaps the child. Requests still waiting fail.
    ///
    /// Shell commands the server started run in groups of their own and are
    /// not stopped by this; see the [module documentation](self#lifecycle-and-cancellation).
    /// [`shutdown`](Self::shutdown) is the way to stop those.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the child cannot be waited on.
    pub async fn terminate(mut self) -> Result<()> {
        self.handle.shared.fail(Closed::Terminated);
        let result = crate::exec::terminate_and_reap(
            &mut self.child,
            &mut self.group,
            self.termination_grace,
            self.working_dir.as_deref(),
        )
        .await;
        self.outcome.settle("cancelled", None);
        result
    }

    /// Clean up after a failed handshake and choose the error to report.
    async fn abandon(mut self, error: Error) -> Error {
        // A server that rejects its arguments or config exits before it can
        // answer. Its exit status and stderr say more than "closed its output".
        if matches!(error, Error::AppServerProtocol { .. })
            && let Ok(Ok(status)) = tokio::time::timeout(STARTUP_EXIT_WAIT, self.child.wait()).await
            && !status.success()
        {
            self.group.disarm();
            self.handle.shared.stderr_settled().await;
            let code = status.code().unwrap_or(-1);
            self.outcome.settle("failed", Some(code));
            return Error::from_command_failure(
                self.command.clone(),
                code,
                String::new(),
                self.handle.shared.stderr_summary(),
                self.working_dir.clone(),
            );
        }
        let outcome = if matches!(error, Error::Timeout { .. }) {
            "timeout"
        } else {
            "failed"
        };
        self.handle.shared.fail(Closed::Terminated);
        let _ = crate::exec::terminate_and_reap(
            &mut self.child,
            &mut self.group,
            self.termination_grace,
            self.working_dir.as_deref(),
        )
        .await;
        self.outcome.settle(outcome, None);
        error
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::CodexBuilder;
    use crate::test_support::{PidFile, is_running_for_test, wait_until_gone};

    fn fake(mode: &str) -> CodexBuilder {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-app-server.sh");
        Codex::builder()
            .binary("/bin/bash")
            .arg(script.to_str().expect("fixture path is utf-8"))
            .env("FAKE_APP_SERVER_MODE", mode)
            // Terminating always waits out the grace, so keep it short.
            .termination_grace(Duration::from_millis(100))
            // A lost answer should fail a test after this, not hang it.
            .timeout(Duration::from_secs(20))
    }

    async fn started(mode: &str) -> AppServer {
        let codex = fake(mode).build().expect("bash must exist");
        AppServer::builder(&codex)
            .start()
            .await
            .expect("the fake server starts")
    }

    /// Read messages until `turn/completed`, returning it and everything seen.
    async fn until_turn_completed(server: &mut AppServer) -> (TurnCompleted, Vec<ServerMessage>) {
        let mut seen = Vec::new();
        loop {
            let message = tokio::time::timeout(Duration::from_secs(10), server.next_message())
                .await
                .expect("the turn should finish")
                .expect("no stream error")
                .expect("the server should not close first");
            let done = match &message {
                ServerMessage::Notification(n) => n.turn_completed(),
                _ => None,
            };
            seen.push(message);
            if let Some(done) = done {
                return (done, seen);
            }
        }
    }

    fn methods(seen: &[ServerMessage]) -> Vec<&str> {
        seen.iter()
            .map(|message| match message {
                ServerMessage::Notification(n) => n.method.as_str(),
                ServerMessage::Request(r) => r.method.as_str(),
            })
            .collect()
    }

    #[tokio::test]
    async fn the_handshake_reports_the_server_and_delivers_unsolicited_notifications() {
        let mut server = started("turn").await;
        assert!(server.server_info().user_agent.contains("0.149.0"));
        assert_eq!(
            server.server_info().codex_home.as_deref(),
            Some("/home/user/.codex")
        );

        // The real server sends this right after `initialized`, unasked.
        let first = server.next_message().await.unwrap().unwrap();
        let ServerMessage::Notification(notification) = first else {
            panic!("expected a notification, got {first:?}");
        };
        assert_eq!(notification.method, "remoteControl/status/changed");
        assert_eq!(notification.params["status"], "disabled");
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_turn_runs_to_completion_and_yields_its_final_message() {
        let mut server = started("turn").await;
        let thread = server
            .thread_start(
                ThreadStartParams::new()
                    .cwd("/tmp/fixture")
                    .ephemeral(true)
                    .approval_policy(ApprovalPolicy::Never)
                    .sandbox(SandboxMode::ReadOnly),
            )
            .await
            .unwrap();
        assert_eq!(thread.id, "01a0d1a2-5f2e-7263-a97b-cabf3e78caff");
        assert_eq!(thread.model.as_deref(), Some("gpt-5.6-sol"));

        let turn = server
            .turn_start(TurnStartParams::text(&thread.id, "say ok"))
            .await
            .unwrap();
        assert_eq!(turn.status, TurnStatus::InProgress);
        assert_eq!(turn.id, "01a0d1a2-5f9f-75c1-9305-fa382e7b37bc");

        let (done, seen) = until_turn_completed(&mut server).await;
        assert_eq!(done.thread_id, thread.id);
        assert_eq!(done.turn.id, turn.id);
        assert_eq!(done.turn.status, TurnStatus::Completed);
        assert_eq!(done.turn.final_message(), Some("ok"));
        let methods = methods(&seen);
        assert!(methods.contains(&"thread/started"));
        assert!(methods.contains(&"turn/started"));
        assert!(methods.contains(&"item/agentMessage/delta"));
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn steering_adds_input_to_the_running_turn() {
        let mut server = started("held").await;
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        let turn = server
            .turn_start(TurnStartParams::text(&thread.id, "sleep, then say DONE"))
            .await
            .unwrap();

        let steered = server
            .turn_steer(
                &thread.id,
                &turn.id,
                vec![UserInput::text("Also append STEERED")],
            )
            .await
            .unwrap();
        assert_eq!(steered, turn.id);

        let (done, seen) = until_turn_completed(&mut server).await;
        assert_eq!(done.turn.final_message(), Some("DONE STEERED"));
        // The fixture echoes the steered text as a user message, so this shows
        // the text reached the server.
        let echoed = seen.iter().any(|message| match message {
            ServerMessage::Notification(n) => {
                n.method == "item/completed"
                    && n.params.pointer("/item/content/0/text")
                        == Some(&json!("Also append STEERED"))
            }
            _ => false,
        });
        assert!(echoed, "the server never saw the steered text: {seen:?}");
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn steering_from_another_task_while_events_are_read() {
        let mut server = started("held").await;
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        let turn = server
            .turn_start(TurnStartParams::text(&thread.id, "go"))
            .await
            .unwrap();

        let handle = server.handle();
        let (thread_id, turn_id) = (thread.id.clone(), turn.id.clone());
        let steer = tokio::spawn(async move {
            handle
                .turn_steer(&thread_id, &turn_id, vec![UserInput::text("more")])
                .await
        });

        let (done, _) = until_turn_completed(&mut server).await;
        assert_eq!(done.turn.status, TurnStatus::Completed);
        assert_eq!(steer.await.unwrap().unwrap(), turn.id);
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn steering_the_wrong_turn_is_an_rpc_error_that_names_both_ids() {
        let server = started("held").await;
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        let turn = server
            .turn_start(TurnStartParams::text(&thread.id, "go"))
            .await
            .unwrap();

        let error = server
            .turn_steer(&thread.id, "not-the-turn", vec![UserInput::text("x")])
            .await
            .unwrap_err();
        match error {
            Error::AppServerRpc {
                method,
                code,
                message,
                ..
            } => {
                assert_eq!(method, "turn/steer");
                assert_eq!(code, -32600);
                assert_eq!(
                    message,
                    format!(
                        "expected active turn id `not-the-turn` but found `{}`",
                        turn.id
                    )
                );
            }
            other => panic!("expected an rpc error, got {other:?}"),
        }

        let error = server
            .turn_interrupt(&thread.id, "not-the-turn")
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::AppServerRpc { message, .. }
                if message == &format!("expected active turn id not-the-turn but found {}", turn.id)),
            "{error:?}"
        );
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn steering_or_interrupting_with_no_active_turn_is_an_rpc_error() {
        let server = started("held").await;
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();

        let error = server
            .turn_steer(&thread.id, "any", vec![UserInput::text("x")])
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::AppServerRpc { message, .. } if message == "no active turn to steer"),
            "{error:?}"
        );
        let error = server.turn_interrupt(&thread.id, "any").await.unwrap_err();
        assert!(
            matches!(&error, Error::AppServerRpc { message, .. } if message == "no active turn to interrupt"),
            "{error:?}"
        );
        server.shutdown().await.unwrap();
    }

    /// codex-cli 0.145.0 sends `items: []` (`itemsView: notLoaded`) in
    /// `turn/completed` even when the turn completed, so the answer has to come
    /// from the `item/completed` stream.
    #[tokio::test]
    async fn the_final_answer_is_available_when_turn_completed_carries_no_items() {
        let codex = fake("turn")
            .env("FAKE_APP_SERVER_ITEMS", "legacy")
            .build()
            .unwrap();
        let mut server = AppServer::builder(&codex).start().await.unwrap();
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        server
            .turn_start(TurnStartParams::text(&thread.id, "say ok"))
            .await
            .unwrap();

        let (done, seen) = until_turn_completed(&mut server).await;
        assert_eq!(done.turn.status, TurnStatus::Completed);
        assert!(done.turn.items.is_empty());
        assert_eq!(done.turn.final_message(), None);
        let answer = seen
            .iter()
            .filter_map(|message| match message {
                ServerMessage::Notification(n) => n.agent_message(),
                _ => None,
            })
            .rfind(AgentMessage::is_final_answer)
            .expect("the item stream carries the answer");
        assert_eq!(answer.text, "ok");
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn interrupting_ends_the_turn_as_interrupted_with_no_final_message() {
        let mut server = started("held").await;
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        let turn = server
            .turn_start(TurnStartParams::text(&thread.id, "go"))
            .await
            .unwrap();

        server.turn_interrupt(&thread.id, &turn.id).await.unwrap();

        let (done, _) = until_turn_completed(&mut server).await;
        assert_eq!(done.turn.status, TurnStatus::Interrupted);
        assert!(done.turn.items.is_empty());
        assert_eq!(done.turn.final_message(), None);
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_server_request_is_delivered_and_the_reply_reaches_the_server() {
        let mut server = started("approval").await;
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        server
            .turn_start(TurnStartParams::text(&thread.id, "touch a file"))
            .await
            .unwrap();

        let mut seen = Vec::new();
        let request = loop {
            let message = tokio::time::timeout(Duration::from_secs(10), server.next_message())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let ServerMessage::Request(request) = message {
                break request;
            }
            seen.push(message);
        };
        assert_eq!(request.method, "item/commandExecution/requestApproval");
        assert_eq!(request.id, RequestId::Number(0));
        assert_eq!(
            request.params["command"],
            "/bin/zsh -lc 'touch created.txt'"
        );

        server
            .respond(request.id, json!({ "decision": "decline" }))
            .unwrap();

        let (done, after) = until_turn_completed(&mut server).await;
        // The fixture reports the decision it received.
        assert_eq!(done.turn.final_message(), Some("decision: decline"));
        assert!(methods(&after).contains(&"serverRequest/resolved"));
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_requests_are_matched_to_their_answers_by_id() {
        let server = started("turn").await;
        let (first, second, unknown) = tokio::join!(
            server.thread_start(ThreadStartParams::new()),
            server.request("thread/start", json!({})),
            server.request("no/such/method", json!({})),
        );
        assert!(first.is_ok());
        assert!(second.unwrap().pointer("/thread/id").is_some());
        match unknown.unwrap_err() {
            Error::AppServerRpc { message, .. } => {
                assert!(
                    message.contains("unknown variant `no/such/method`"),
                    "{message}"
                );
            }
            other => panic!("expected an rpc error, got {other:?}"),
        }
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_second_initialize_is_rejected_by_the_server() {
        let server = started("turn").await;
        let error = server
            .request(
                "initialize",
                json!({ "clientInfo": { "name": "x", "version": "0" } }),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::AppServerRpc { message, .. } if message == "Already initialized"),
            "{error:?}"
        );
        server.shutdown().await.unwrap();
    }

    /// What the client puts on the wire, read back from the fixture's log.
    #[tokio::test]
    async fn requests_are_written_as_the_server_expects_them() {
        let log = std::env::temp_dir().join(format!(
            "codex-wrapper-{}-app-server-wire.log",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&log);
        let codex = fake("held")
            .env("FAKE_APP_SERVER_LOG", log.to_str().unwrap())
            .build()
            .unwrap();
        let mut server = AppServer::builder(&codex)
            .client_info("probe", "1.2.3")
            .opt_out_notification("item/agentMessage/delta")
            .opt_out_notification("mcpServer/startupStatus/updated")
            .start()
            .await
            .unwrap();
        let thread = server
            .thread_start(
                ThreadStartParams::new()
                    .cwd("/tmp/fixture")
                    .model("gpt-5.6-sol")
                    .approval_policy(ApprovalPolicy::Never)
                    .sandbox(SandboxMode::ReadOnly)
                    .ephemeral(true),
            )
            .await
            .unwrap();
        let schema = json!({ "type": "object", "required": ["answer"] });
        let turn = server
            .turn_start(
                TurnStartParams::text(&thread.id, "hi")
                    .model("gpt-5.6-sol")
                    .effort("low")
                    .output_schema(schema.clone())
                    .cwd("/tmp/fixture")
                    .approval_policy(ApprovalPolicy::Never)
                    .param("summary", json!("none")),
            )
            .await
            .unwrap();
        server
            .turn_steer(&thread.id, &turn.id, vec![UserInput::text("more")])
            .await
            .unwrap();
        let _ = until_turn_completed(&mut server).await;
        server.shutdown().await.unwrap();

        let lines: Vec<Value> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).expect("every client line is JSON"))
            .collect();
        let _ = std::fs::remove_file(&log);

        assert_eq!(
            lines[0],
            json!({
                "id": 0,
                "method": "initialize",
                "params": {
                    "clientInfo": { "name": "probe", "version": "1.2.3" },
                    "capabilities": {
                        "optOutNotificationMethods": [
                            "item/agentMessage/delta",
                            "mcpServer/startupStatus/updated"
                        ]
                    }
                }
            })
        );
        assert_eq!(lines[1], json!({ "method": "initialized" }));
        assert_eq!(
            lines[2],
            json!({
                "id": 1,
                "method": "thread/start",
                "params": {
                    "cwd": "/tmp/fixture",
                    "model": "gpt-5.6-sol",
                    "approvalPolicy": "never",
                    "sandbox": "read-only",
                    "ephemeral": true
                }
            })
        );
        assert_eq!(
            lines[3],
            json!({
                "id": 2,
                "method": "turn/start",
                "params": {
                    "threadId": thread.id,
                    "input": [{ "type": "text", "text": "hi", "text_elements": [] }],
                    "model": "gpt-5.6-sol",
                    "effort": "low",
                    "outputSchema": schema,
                    "cwd": "/tmp/fixture",
                    "approvalPolicy": "never",
                    "summary": "none"
                }
            })
        );
        assert_eq!(
            lines[4],
            json!({
                "id": 3,
                "method": "turn/steer",
                "params": {
                    "threadId": thread.id,
                    "expectedTurnId": turn.id,
                    "input": [{ "type": "text", "text": "more", "text_elements": [] }]
                }
            })
        );
    }

    #[test]
    fn the_previewed_command_includes_the_clients_global_args() {
        let codex = Codex::builder()
            .binary("/bin/echo")
            .config("model=\"o3\"")
            .build()
            .unwrap();
        let builder = AppServer::builder(&codex);
        assert_eq!(builder.args(), ["app-server", "--listen", "stdio://"]);
        assert_eq!(
            builder.to_command_string(),
            "/bin/echo -c 'model=\"o3\"' app-server --listen stdio://"
        );
    }

    #[tokio::test]
    async fn a_silent_server_times_out_the_handshake_and_is_stopped() {
        let pid = Arc::new(Mutex::new(None));
        let observed = Arc::clone(&pid);
        let codex = fake("silent")
            .timeout(Duration::from_millis(300))
            .on_spawn(Arc::new(move |info| {
                *observed.lock().unwrap() = Some(info.pid);
            }))
            .build()
            .unwrap();

        let error = AppServer::builder(&codex).start().await.unwrap_err();
        assert!(matches!(error, Error::Timeout { .. }), "{error:?}");

        let pid = pid.lock().unwrap().expect("the server was spawned");
        assert!(
            wait_until_gone(pid).await,
            "the silent server ({pid}) survived a failed start"
        );
    }

    #[tokio::test]
    async fn a_server_that_exits_at_startup_is_reported_with_its_stderr() {
        let codex = fake("exit-on-start").build().unwrap();
        let error = AppServer::builder(&codex).start().await.unwrap_err();
        assert_eq!(error.exit_code(), Some(1), "{error:?}");
        assert_eq!(
            error.failure_kind(),
            Some(crate::FailureKind::Config),
            "{error:?}"
        );
        assert!(error.to_string().contains("bogus_key"), "{error}");
    }

    /// The server writes about 500 KB to stderr before its first answer. An
    /// undrained pipe holds about 64 KB, so this hangs if stderr is not read.
    #[tokio::test]
    async fn heavy_stderr_does_not_stall_the_session() {
        let codex = fake("noisy-stderr").build().unwrap();
        let server =
            tokio::time::timeout(Duration::from_secs(20), AppServer::builder(&codex).start())
                .await
                .expect("the handshake stalled behind an undrained stderr pipe")
                .unwrap();
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_server_that_exits_mid_request_fails_it_with_the_stderr() {
        let mut server = started("exit-on-turn").await;
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        let error = server
            .turn_start(TurnStartParams::text(&thread.id, "go"))
            .await
            .unwrap_err();
        let text = error.to_string();
        assert!(text.contains("turn/start"), "{text}");
        assert!(
            text.contains("connection to the model provider was lost"),
            "{text}"
        );

        // Earlier messages are still delivered, then the stream ends cleanly.
        while server.next_message().await.unwrap().is_some() {}
        assert!(server.next_message().await.unwrap().is_none());

        // New requests fail at once instead of waiting for a timeout.
        let again = server.request("thread/start", json!({})).await.unwrap_err();
        assert!(
            matches!(again, Error::AppServerProtocol { .. }),
            "{again:?}"
        );

        // The exit status is reported by shutdown.
        let error = server.shutdown().await.unwrap_err();
        assert_eq!(error.exit_code(), Some(5), "{error:?}");
    }

    #[tokio::test]
    async fn the_output_limit_ends_the_session_and_kills_the_server() {
        let pid = Arc::new(Mutex::new(None));
        let observed = Arc::clone(&pid);
        let codex = fake("flood")
            .output_limit(2048)
            .on_spawn(Arc::new(move |info| {
                *observed.lock().unwrap() = Some(info.pid);
            }))
            .build()
            .unwrap();
        let mut server = AppServer::builder(&codex).start().await.unwrap();

        let error = loop {
            match server.next_message().await {
                Ok(Some(_)) => {}
                Ok(None) => panic!("the limit error was swallowed"),
                Err(error) => break error,
            }
        };
        assert!(
            matches!(
                error,
                Error::OutputLimitExceeded {
                    stream: OutputStream::Stdout,
                    limit_bytes: 2048
                }
            ),
            "{error:?}"
        );
        assert!(server.next_message().await.unwrap().is_none());

        let pid = pid.lock().unwrap().expect("the server was spawned");
        assert!(
            wait_until_gone(pid).await,
            "the server ({pid}) kept running after the output limit"
        );
    }

    #[tokio::test]
    async fn invalid_json_ends_the_session() {
        let mut server = started("bad-json").await;
        let error = loop {
            match server.next_message().await {
                Ok(Some(_)) => {}
                Ok(None) => panic!("the malformed line was swallowed"),
                Err(error) => break error,
            }
        };
        assert!(
            matches!(&error, Error::AppServerProtocol { message } if message.contains("invalid JSON")),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn terminate_stops_the_whole_process_group() {
        let pid_file = PidFile::new("app-server-terminate");
        let codex = fake("spawns-child")
            .env(
                "CODEX_WRAPPER_TEST_PIDFILE",
                pid_file.path().to_str().unwrap(),
            )
            .build()
            .unwrap();
        let server = AppServer::builder(&codex).start().await.unwrap();
        let (parent, child) = read_pids(&pid_file).await;
        assert!(is_running_for_test(parent) && is_running_for_test(child));

        server.terminate().await.unwrap();

        assert!(
            !is_running_for_test(parent),
            "the server ({parent}) survived"
        );
        assert!(
            !is_running_for_test(child),
            "the server's subprocess ({child}) survived terminate"
        );
        // The fixture writes this marker only if it is asked to stop with
        // SIGTERM first, which is what separates terminate from a kill.
        let marker = term_marker(&pid_file);
        assert!(
            marker.exists(),
            "terminate killed the server without asking it to stop first"
        );
        let _ = std::fs::remove_file(marker);
    }

    #[tokio::test]
    async fn dropping_the_client_kills_the_process_group() {
        let pid_file = PidFile::new("app-server-drop");
        let codex = fake("spawns-child")
            .env(
                "CODEX_WRAPPER_TEST_PIDFILE",
                pid_file.path().to_str().unwrap(),
            )
            .build()
            .unwrap();
        let server = AppServer::builder(&codex).start().await.unwrap();
        let (parent, child) = read_pids(&pid_file).await;

        drop(server);

        assert!(
            wait_until_gone(parent).await,
            "the server ({parent}) survived"
        );
        assert!(
            wait_until_gone(child).await,
            "the server's subprocess ({child}) survived the drop"
        );
        // Dropping is the abrupt path: no SIGTERM, so no marker.
        assert!(!term_marker(&pid_file).exists());
    }

    #[tokio::test]
    async fn shutdown_lets_a_server_that_exits_on_eof_finish_by_itself() {
        // A fallback to terminate would wait out the whole grace, so a long
        // grace makes the two paths easy to tell apart without a tight bound.
        let codex = fake("turn")
            .termination_grace(Duration::from_secs(4))
            .build()
            .unwrap();
        let server = AppServer::builder(&codex).start().await.unwrap();
        let pid = server.pid().expect("running");
        let started_at = std::time::Instant::now();
        server.shutdown().await.unwrap();
        assert!(
            started_at.elapsed() < Duration::from_secs(3),
            "shutdown waited out the grace instead of seeing the exit"
        );
        assert!(wait_until_gone(pid).await);
    }

    #[tokio::test]
    async fn shutdown_falls_back_to_termination_when_the_server_ignores_eof() {
        let pid_file = PidFile::new("app-server-ignores-eof");
        let codex = fake("ignores-eof")
            .env(
                "CODEX_WRAPPER_TEST_PIDFILE",
                pid_file.path().to_str().unwrap(),
            )
            .build()
            .unwrap();
        let server = AppServer::builder(&codex).start().await.unwrap();
        let (parent, child) = read_pids(&pid_file).await;

        server.shutdown().await.unwrap();

        // Asked to stop before it was killed, not just killed.
        let marker = term_marker(&pid_file);
        assert!(
            marker.exists(),
            "shutdown killed the server without asking it to stop"
        );
        let _ = std::fs::remove_file(marker);

        assert!(
            !is_running_for_test(parent),
            "the server ({parent}) survived"
        );
        assert!(
            !is_running_for_test(child),
            "the server's subprocess ({child}) survived shutdown"
        );
    }

    #[tokio::test]
    async fn requests_after_shutdown_fail_on_a_surviving_handle() {
        let server = started("turn").await;
        let handle = server.handle();
        server.shutdown().await.unwrap();
        let error = handle.request("thread/start", json!({})).await.unwrap_err();
        assert!(
            matches!(error, Error::AppServerProtocol { .. }),
            "{error:?}"
        );
        assert!(handle.notify("initialized", Value::Null).is_err());
    }

    fn term_marker(pid_file: &PidFile) -> std::path::PathBuf {
        let mut path = pid_file.path().as_os_str().to_owned();
        path.push(".term");
        std::path::PathBuf::from(path)
    }

    async fn read_pids(pid_file: &PidFile) -> (u32, u32) {
        for _ in 0..100 {
            if let Ok(contents) = std::fs::read_to_string(pid_file.path()) {
                let field = |key: &str| {
                    contents
                        .lines()
                        .find_map(|line| line.strip_prefix(key))
                        .and_then(|value| value.trim().parse::<u32>().ok())
                };
                if let (Some(parent), Some(child)) = (field("parent="), field("child=")) {
                    return (parent, child);
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the fake server never recorded its pids");
    }

    #[tokio::test]
    async fn a_request_that_times_out_leaves_the_session_usable_and_no_entry_behind() {
        let codex = fake("mute-after-init")
            .timeout(Duration::from_millis(150))
            .build()
            .unwrap();
        let server = AppServer::builder(&codex).start().await.unwrap();

        for _ in 0..2 {
            let error = server.request("thread/start", json!({})).await.unwrap_err();
            assert!(matches!(error, Error::Timeout { .. }), "{error:?}");
        }
        assert!(
            locked(&server.handle.shared.state).pending.is_empty(),
            "a timed-out request left its slot behind"
        );
        // A timeout is a verdict on one request, not on the session.
        assert!(server.notify("initialized", Value::Null).is_ok());
        server.terminate().await.unwrap();
    }

    /// A request future that is dropped after it has queued its line must not
    /// leave half a line behind: the next request still gets a clean answer,
    /// and every line the server received is whole JSON.
    #[tokio::test]
    async fn dropping_a_request_future_does_not_corrupt_the_wire() {
        let log = std::env::temp_dir().join(format!(
            "codex-wrapper-{}-app-server-dropped.log",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&log);
        let codex = fake("turn")
            .env("FAKE_APP_SERVER_LOG", log.to_str().unwrap())
            .build()
            .unwrap();
        let server = AppServer::builder(&codex).start().await.unwrap();

        for _ in 0..20 {
            // Poll once, so the line is queued, then drop the future before
            // any answer can be read.
            let request = server.request("thread/start", json!({ "ephemeral": true }));
            let mut request = std::pin::pin!(request);
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(request.as_mut().poll(&mut context).is_pending());
        }
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        assert!(!thread.id.is_empty());
        server.shutdown().await.unwrap();

        let text = std::fs::read_to_string(&log).unwrap();
        let _ = std::fs::remove_file(&log);
        let mut count = 0;
        for line in text.lines() {
            serde_json::from_str::<Value>(line).expect("the server received a partial line");
            count += 1;
        }
        // initialize, initialized, twenty abandoned requests, one that was awaited.
        assert_eq!(count, 23, "{text}");
    }

    #[tokio::test]
    async fn next_message_loses_nothing_when_its_future_is_dropped() {
        let mut server = started("turn").await;
        let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
        server
            .turn_start(TurnStartParams::text(&thread.id, "go"))
            .await
            .unwrap();

        // Poll with a timeout that keeps expiring, so the future is dropped
        // over and over while messages arrive.
        let mut seen = Vec::new();
        let mut finished = false;
        for _ in 0..2000 {
            if let Ok(next) =
                tokio::time::timeout(Duration::from_micros(200), server.next_message()).await
            {
                let message = next.unwrap().expect("the stream ended early");
                if let ServerMessage::Notification(n) = &message {
                    finished |= n.turn_completed().is_some();
                }
                seen.push(message);
            }
            if finished {
                break;
            }
        }
        assert!(
            finished,
            "never saw the turn complete: {:?}",
            methods(&seen)
        );
        let methods = methods(&seen);
        // Every message the fixture sends after `initialized`, in order.
        let expected = [
            "remoteControl/status/changed",
            "thread/started",
            "thread/status/changed",
            "turn/started",
            "item/started",
            "item/agentMessage/delta",
            "item/completed",
            "thread/tokenUsage/updated",
            "thread/status/changed",
            "turn/completed",
        ];
        assert_eq!(methods, expected);
        server.shutdown().await.unwrap();
    }

    #[test]
    fn turn_final_message_takes_the_last_final_answer_and_ignores_commentary() {
        let turn: Turn = serde_json::from_value(json!({
            "id": "t",
            "status": "completed",
            "items": [
                { "type": "agentMessage", "text": "working", "phase": "commentary" },
                { "type": "commandExecution", "text": "not a message", "phase": "final_answer" },
                { "type": "agentMessage", "text": "first", "phase": "final_answer" },
                { "type": "agentMessage", "text": "last", "phase": "final_answer" }
            ]
        }))
        .unwrap();
        assert_eq!(turn.final_message(), Some("last"));
    }

    #[test]
    fn agent_messages_are_decoded_from_item_completed_only() {
        let message = |phase: Option<&str>| Notification {
            method: "item/completed".into(),
            params: json!({
                "item": {
                    "type": "agentMessage",
                    "id": "m",
                    "text": "DONE STEERED",
                    "phase": phase,
                },
                "threadId": "th",
                "turnId": "tu",
            }),
        };
        let answer = message(Some("final_answer")).agent_message().unwrap();
        assert_eq!(answer.text, "DONE STEERED");
        assert_eq!(
            (answer.thread_id.as_str(), answer.turn_id.as_str()),
            ("th", "tu")
        );
        assert!(answer.is_final_answer());
        assert!(
            !message(Some("commentary"))
                .agent_message()
                .unwrap()
                .is_final_answer()
        );
        assert!(!message(None).agent_message().unwrap().is_final_answer());

        // Other items, other methods, and a delta are not agent messages.
        let mut command = message(None);
        command.params["item"]["type"] = json!("commandExecution");
        assert!(command.agent_message().is_none());
        let mut delta = message(None);
        delta.method = "item/agentMessage/delta".into();
        assert!(delta.agent_message().is_none());
    }

    #[test]
    fn an_unknown_turn_status_is_kept_apart_from_the_known_ones() {
        let turn: Turn =
            serde_json::from_value(json!({ "id": "t", "status": "someFutureStatus" })).unwrap();
        assert_eq!(turn.status, TurnStatus::Unknown);
    }

    #[test]
    fn only_a_well_formed_turn_completed_notification_decodes() {
        let other = Notification {
            method: "turn/started".into(),
            params: json!({}),
        };
        assert!(other.turn_completed().is_none());
        let malformed = Notification {
            method: "turn/completed".into(),
            params: json!({ "threadId": "x" }),
        };
        assert!(malformed.turn_completed().is_none());
    }

    #[test]
    fn request_ids_round_trip_as_numbers_and_strings() {
        assert_eq!(serde_json::to_string(&RequestId::Number(7)).unwrap(), "7");
        assert_eq!(
            serde_json::to_string(&RequestId::String("a".into())).unwrap(),
            "\"a\""
        );
        assert_eq!(
            serde_json::from_str::<RequestId>("0").unwrap(),
            RequestId::Number(0)
        );
        assert_eq!(
            serde_json::from_str::<RequestId>("\"x\"").unwrap(),
            RequestId::String("x".into())
        );
    }

    #[test]
    fn reply_lines_echo_the_servers_id() {
        let line = reply_line(
            &RequestId::Number(0),
            "result",
            &json!({ "decision": "decline" }),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(line).unwrap(),
            "{\"id\":0,\"result\":{\"decision\":\"decline\"}}\n"
        );
        let line = reply_line(
            &RequestId::String("r-1".into()),
            "error",
            &json!({ "code": -32601, "message": "no" }),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(line).unwrap(),
            "{\"id\":\"r-1\",\"error\":{\"code\":-32601,\"message\":\"no\"}}\n"
        );
    }

    #[test]
    fn colour_codes_are_removed_from_server_logs() {
        let raw = "\u{1b}[2m2026-09-24T04:17:33Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m worker quit";
        assert_eq!(strip_ansi(raw), "2026-09-24T04:17:33Z ERROR worker quit");
    }

    #[test]
    fn a_line_that_is_not_a_message_is_ignored_and_bad_json_is_fatal() {
        let (events, mut received) = mpsc::unbounded_channel();
        let shared = Shared::new(events);
        assert!(route(r#"{"unrelated":true}"#, &shared).is_ok());
        assert!(
            route(
                r#"{"id":null,"error":{"code":-32600,"message":"x"}}"#,
                &shared
            )
            .is_ok()
        );
        assert!(route(r#"{"id":99,"result":{}}"#, &shared).is_ok());
        assert!(received.try_recv().is_err(), "nothing should be delivered");

        assert!(route(r#"{"method":"a/b","params":{"k":1}}"#, &shared).is_ok());
        assert!(matches!(
            received.try_recv(),
            Ok(Event::Message(ServerMessage::Notification(n))) if n.method == "a/b"
        ));
        assert!(route(r#"{"method":"a/b","id":"s"}"#, &shared).is_ok());
        assert!(matches!(
            received.try_recv(),
            Ok(Event::Message(ServerMessage::Request(r))) if r.id == RequestId::String("s".into())
        ));
        assert!(matches!(
            route(r#"{"method": oops"#, &shared),
            Err(Closed::Broken(_))
        ));
    }
}
