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
//!   that starts with `{` but is not valid JSON) kills the child: on Unix as
//!   soon as it is detected, and on Windows through its owned process handle.
//!   The error also reaches any request still waiting.
//!
//! # Limits
//!
//! [`CodexBuilder::output_limit`](crate::CodexBuilder::output_limit) caps the
//! total bytes read from stdout over the life of the session, as it does for
//! streaming. [`CodexBuilder::timeout`](crate::CodexBuilder::timeout) applies
//! to each request, including the handshake, and not to the session; without
//! it a request waits until the server answers or exits. The server's stderr is
//! drained continuously and the last 16 KiB is kept for error messages.
//!
//! By default messages wait in an unbounded queue until
//! [`AppServer::next_message`] returns them, so a caller that does not read
//! them, or reads more slowly than the server writes, holds them in memory.
//! `output_limit` bounds the total. Streaming deltas are most of the volume,
//! and [`AppServerBuilder::opt_out_notification`] stops the server sending
//! them.
//!
//! Five opt-in limits bound memory instead of the lifetime total, and each is
//! off by default:
//!
//! - [`max_frame_bytes`](AppServerBuilder::max_frame_bytes): one line from
//!   the server, newline included. A longer line is rejected before more than
//!   the limit is buffered, whether or not it ever ends.
//! - [`max_queued_messages`](AppServerBuilder::max_queued_messages) and
//!   [`max_queued_bytes`](AppServerBuilder::max_queued_bytes): notifications
//!   and server requests read but not yet returned by `next_message`. Answers
//!   to the client's own requests do not pass through this queue, so they are
//!   delivered while events wait, and the reader never blocks on it.
//! - [`max_writer_queue_messages`](AppServerBuilder::max_writer_queue_messages)
//!   and [`max_writer_queue_bytes`](AppServerBuilder::max_writer_queue_bytes):
//!   lines queued for the server's stdin, including the one being written. A
//!   server that stops reading its input fills this.
//!
//! Exceeding any of them ends the connection with
//! [`Error::AppServerLimitExceeded`]. No message is dropped or merged to stay
//! under a limit: an approval request or a `turn/completed` that does not fit
//! is not delivered and the session fails instead, so a caller never mistakes
//! a lossy stream for a complete one. Messages queued before the failure are
//! still returned by `next_message`, then the error. Every waiting request
//! fails with the same error, later sends fail at once, and the process the
//! client owns is killed, as for the output limit.
//!
//! # Capabilities
//!
//! `initialize` is sent without `capabilities` unless something asks for one.
//! [`AppServerBuilder::capability`] sets any field of it by its protocol name,
//! for example `experimentalApi`, which
//! [`experimental_api`](AppServerBuilder::experimental_api) also sets. The
//! wrapper does not check other fields; what a field means and whether the
//! server accepts it depends on the CLI version.
//!
//! `optOutNotificationMethods` can be set both ways. The two lists are
//! combined: the entries set through `capability` first, then those from
//! [`opt_out_notification`](AppServerBuilder::opt_out_notification), each
//! method once. A value for it that is neither an array of strings nor `null`
//! is rejected by [`AppServerBuilder::start`] before anything is spawned.
//! [`AppServerBuilder::initialize_params`] shows the result.
//!
//! # Connecting to an existing server
//!
//! On Unix, `AppServerBuilder::connect_unix(path)` attaches over a WebSocket
//! connection to an explicitly selected Unix socket. It spawns no process:
//! [`AppServer::pid`] is `None`, and shutdown, terminate, drop, or a transport
//! failure close only this connection. They never signal the shared server
//! or promise to interrupt its turns. Client binary/env/global arguments,
//! spawn observers, and process-group policies do not apply in this mode.
//!
//! The connection performs a fresh initialize handshake with the configured
//! capabilities. Client timeout bounds connect/upgrade and each RPC wait.
//! Frame and queue byte limits count UTF-8 JSON message payloads, without
//! WebSocket headers; stdio counts raw line bytes including delimiters.
//! These budgets are not exact heap measurements. WebSocket frame and
//! aggregate-message limits prevent fragmentation from bypassing the cap.
//!
//! Attached transport support was qualified on CLI 0.163.0 and remains
//! experimental. Nothing discovers or starts a daemon, adopts a private TUI
//! process, reconnects automatically, or replays uncertain requests. A new
//! connection must explicitly resume/reconcile threads, subscriptions and
//! approvals. Empty, unmaterialized threads may not yet be resumable by a
//! second client. Two-client rejoin, steering, pending approval delivery and
//! cancellation, observer disconnect/reconnect, and server loss were probed
//! with an isolated server and deterministic offline backend. See the shipped
//! `tests/app-server-qualification.md` for versioned evidence and limits.
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

#[cfg(unix)]
use futures_util::{SinkExt, StreamExt};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::{Notify, mpsc, oneshot, watch};
#[cfg(unix)]
use tokio_tungstenite::tungstenite::{Message as WsMessage, protocol::WebSocketConfig};
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

/// The `initialize` capability that [`AppServerBuilder::opt_out_notification`]
/// also writes.
const OPT_OUT_KEY: &str = "optOutNotificationMethods";

/// One of the opt-in transport limits, as reported by
/// [`Error::AppServerLimitExceeded`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AppServerLimit {
    /// The bytes in one line from the server, newline included. Set with
    /// [`AppServerBuilder::max_frame_bytes`].
    FrameBytes,
    /// Messages waiting for [`AppServer::next_message`]. Set with
    /// [`AppServerBuilder::max_queued_messages`].
    QueuedMessages,
    /// Bytes waiting for [`AppServer::next_message`]. Set with
    /// [`AppServerBuilder::max_queued_bytes`].
    QueuedBytes,
    /// Lines waiting to be written to the server. Set with
    /// [`AppServerBuilder::max_writer_queue_messages`].
    WriterQueueMessages,
    /// Bytes waiting to be written to the server. Set with
    /// [`AppServerBuilder::max_writer_queue_bytes`].
    WriterQueueBytes,
}

impl std::fmt::Display for AppServerLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FrameBytes => "frame byte",
            Self::QueuedMessages => "queued message",
            Self::QueuedBytes => "queued byte",
            Self::WriterQueueMessages => "writer queue message",
            Self::WriterQueueBytes => "writer queue byte",
        })
    }
}

/// How an [`AppServer`] reaches the server, and so what it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AppServerTransport {
    /// `app-server --listen stdio://`: the client started the server and owns
    /// it.
    Stdio,
    /// An existing server reached by WebSocket over an explicitly selected
    /// Unix socket. The client owns only its connection.
    #[cfg(unix)]
    UnixSocket {
        /// The selected local socket path.
        socket: PathBuf,
    },
}

impl AppServerTransport {
    /// Whether this client owns the server process. Attached connections never
    /// signal the shared server.
    #[must_use]
    pub fn owns_server(&self) -> bool {
        matches!(self, Self::Stdio)
    }

    /// How the owned process is named in error messages.
    fn process_name(&self) -> &'static str {
        match self {
            Self::Stdio => "codex app-server",
            #[cfg(unix)]
            Self::UnixSocket { .. } => "codex app-server connection",
        }
    }
}

/// The opt-in transport limits. `None` is unlimited.
#[derive(Debug, Clone, Copy, Default)]
struct Limits {
    frame_bytes: Option<usize>,
    queued_messages: Option<usize>,
    queued_bytes: Option<usize>,
    writer_messages: Option<usize>,
    writer_bytes: Option<usize>,
}

/// Counts what is in one queue against its limits.
///
/// A message is reserved before it is queued and released when it leaves the
/// queue: returned to the caller, written, or discarded. The lock is never held
/// while another is taken.
struct Budget {
    max_messages: Option<usize>,
    max_bytes: Option<usize>,
    messages_limit: AppServerLimit,
    bytes_limit: AppServerLimit,
    used: Mutex<(usize, usize)>,
}

impl Budget {
    fn new(
        max_messages: Option<usize>,
        max_bytes: Option<usize>,
        messages_limit: AppServerLimit,
        bytes_limit: AppServerLimit,
    ) -> Self {
        Self {
            max_messages,
            max_bytes,
            messages_limit,
            bytes_limit,
            used: Mutex::new((0, 0)),
        }
    }

    /// Count one message of `bytes`, or say which limit it would pass.
    fn reserve(&self, bytes: usize) -> std::result::Result<(), Closed> {
        let mut used = locked(&self.used);
        let messages = used.0.saturating_add(1);
        let total = used.1.saturating_add(bytes);
        if let Some(max) = self.max_messages
            && messages > max
        {
            return Err(Closed::Limit {
                limit: self.messages_limit,
                max,
            });
        }
        if let Some(max) = self.max_bytes
            && total > max
        {
            return Err(Closed::Limit {
                limit: self.bytes_limit,
                max,
            });
        }
        *used = (messages, total);
        Ok(())
    }

    fn release(&self, bytes: usize) {
        let mut used = locked(&self.used);
        used.0 = used.0.saturating_sub(1);
        used.1 = used.1.saturating_sub(bytes);
    }

    #[cfg(test)]
    fn used(&self) -> (usize, usize) {
        *locked(&self.used)
    }
}

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
    /// One of the opt-in transport limits was passed.
    Limit { limit: AppServerLimit, max: usize },
}

/// What a waiting request gets back.
enum Reply {
    Result(Value),
    Rpc(RpcFailure),
    /// A response with neither `result` nor `error`.
    Malformed,
    Closed(Closed),
}

/// What the reader hands to [`AppServer::next_message`].
enum Event {
    /// A message and the bytes it holds in the inbound budget.
    Message(ServerMessage, usize),
    Closed(Closed),
}

/// What the writer task is asked to do.
enum Outgoing {
    Line(ReservedLine),
    /// Stop writing and drop stdin, which the server sees as end of input.
    Close,
}

/// Owns the reservation through queued, in-flight, cancelled and aborted writes.
struct ReservedLine {
    bytes: Vec<u8>,
    budget: Arc<Budget>,
}
impl Drop for ReservedLine {
    fn drop(&mut self) {
        self.budget.release(self.bytes.len());
    }
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
    /// What the reader task may kill when the stream breaks. Taken away, under
    /// this lock, before the client reaps the child, so the reader can never
    /// signal a pid that has been released.
    kill: Mutex<Option<KillTarget>>,
    events: mpsc::UnboundedSender<Event>,
    /// Messages routed to `events` and not yet returned by `next_message`.
    inbound: Budget,
    /// Lines handed to the writer and not yet written, the one being written
    /// included.
    outbound: Arc<Budget>,
    closed: watch::Sender<bool>,
    newline_delimited: bool,
    frame_bytes: Option<usize>,
    /// The owned process, as error messages name it.
    process: &'static str,
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
        self.end(reason, true);
    }

    fn end(&self, reason: Closed, stop_writer: bool) {
        self.end_with_cleanup(reason, stop_writer, false);
    }

    fn end_with_cleanup(&self, reason: Closed, stop_writer: bool, kill: bool) {
        let pending = {
            let mut state = locked(&self.state);
            if state.closed.is_some() {
                if stop_writer {
                    self.closed.send_replace(true);
                }
                return;
            }
            state.closed = Some(reason.clone());
            if kill {
                self.fire_kill();
            }
            if stop_writer {
                self.closed.send_replace(true);
            }
            std::mem::take(&mut state.pending)
        };
        for (_, waiter) in pending {
            let _ = waiter.send(Reply::Closed(reason.clone()));
        }
        let _ = self.events.send(Event::Closed(reason));
    }

    /// Preserve the causal failure before cleanup can produce EOF.
    fn fatal(&self, reason: Closed) {
        // Hold admission closed while cleanup starts, then publish the error.
        // EOF can neither replace the cause nor reach waiters before the kill.
        self.end_with_cleanup(reason, true, true);
    }

    async fn wait_closed(&self) {
        let mut closed = self.closed.subscribe();
        if !*closed.borrow_and_update() {
            let _ = closed.changed().await;
        }
    }

    fn new(
        events: mpsc::UnboundedSender<Event>,
        kill: Option<KillTarget>,
        limits: Limits,
        process: &'static str,
    ) -> Self {
        Self {
            state: Mutex::new(State {
                pending: HashMap::new(),
                closed: None,
            }),
            next_id: AtomicI64::new(0),
            stderr_tail: Mutex::new(Vec::new()),
            stderr_done: AtomicBool::new(false),
            stderr_finished: Notify::new(),
            kill: Mutex::new(kill),
            events,
            inbound: Budget::new(
                limits.queued_messages,
                limits.queued_bytes,
                AppServerLimit::QueuedMessages,
                AppServerLimit::QueuedBytes,
            ),
            outbound: Arc::new(Budget::new(
                limits.writer_messages,
                limits.writer_bytes,
                AppServerLimit::WriterQueueMessages,
                AppServerLimit::WriterQueueBytes,
            )),
            closed: watch::channel(false).0,
            newline_delimited: true,
            frame_bytes: limits.frame_bytes,
            process,
        }
    }

    fn closed_reason(&self) -> Option<Closed> {
        locked(&self.state).closed.clone()
    }

    /// Hand a message to `next_message`, counting it against the inbound
    /// limits. Never waits: answers to requests are routed by the same task,
    /// so blocking here would hold them behind events nobody is reading.
    fn deliver(&self, message: ServerMessage, bytes: usize) -> std::result::Result<(), Closed> {
        let state = locked(&self.state);
        if let Some(reason) = &state.closed {
            // Intentional shutdown has closed admission, but incoming traffic
            // must not cancel the owned writer's graceful drain.
            return if matches!(reason, Closed::Terminated) {
                Ok(())
            } else {
                Err(reason.clone())
            };
        }
        self.inbound.reserve(bytes)?;
        if self.events.send(Event::Message(message, bytes)).is_err() {
            // The receiver is gone with the `AppServer`.
            self.inbound.release(bytes);
        }
        Ok(())
    }

    /// Kill the server, if the reader still has the authority to. Called when
    /// the stream is unusable and nothing will read what the server writes.
    fn fire_kill(&self) {
        // Held across the signal, so `revoke_kill` cannot return while one is
        // being sent.
        if let Some(target) = *locked(&self.kill) {
            target.signal();
        }
    }

    /// Take away the reader's authority to kill. Must be called before the
    /// child is reaped: from then on the pid may belong to another process.
    fn revoke_kill(&self) {
        *locked(&self.kill) = None;
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
            Closed::Limit { limit, max } => Error::AppServerLimitExceeded {
                limit: *limit,
                max: *max,
            },
            Closed::Terminated => protocol("the app-server session was ended by the client".into()),
            Closed::Broken(message) => protocol(message.clone()),
            Closed::Eof => {
                let process = self.process;
                let stderr = self.stderr_summary();
                let while_waiting =
                    method.map_or_else(String::new, |m| format!(" before answering `{m}`"));
                if stderr.is_empty() {
                    protocol(format!("{process} closed its output{while_waiting}"))
                } else {
                    protocol(format!(
                        "{process} closed its output{while_waiting}; stderr: {stderr}"
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
        match self.shared.closed_reason() {
            Some(reason) => self.shared.error_for(&reason, method),
            None => Error::AppServerProtocol {
                message: format!(
                    "{} is no longer accepting messages (its stdin is closed)",
                    self.shared.process
                ),
            },
        }
    }

    /// Count a line of `bytes` against the writer limits, or end the session
    /// if it does not fit. A line that would pass a limit is never queued: the
    /// server would see a gap in what the client sent.
    fn admit(&self, bytes: usize, method: Option<&str>) -> Result<()> {
        if let Some(reason) = self.shared.closed_reason() {
            return Err(self.shared.error_for(&reason, method));
        }
        if let Err(reason) = self.shared.outbound.reserve(bytes) {
            // No lock is held here: `fail` takes the state lock and wakes
            // waiters, which may call back into this handle.
            self.shared.fatal(reason);
            return Err(self.closed_error(method));
        }
        Ok(())
    }

    /// Queue a reserved line for the writer, releasing the reservation if the
    /// writer has gone.
    fn send_line(&self, line: Vec<u8>, method: Option<&str>) -> Result<()> {
        let line = ReservedLine {
            bytes: line,
            budget: Arc::clone(&self.shared.outbound),
        };
        let state = locked(&self.shared.state);
        if let Some(reason) = &state.closed {
            return Err(self.shared.error_for(reason, method));
        }
        let sent = self.writer.send(Outgoing::Line(line));
        drop(state);
        sent.map_err(|_| self.closed_error(method))
    }

    fn wire_line(&self, mut line: Vec<u8>) -> Vec<u8> {
        if !self.shared.newline_delimited && line.last() == Some(&b'\n') {
            line.pop();
        }
        line
    }

    fn enqueue(&self, line: Vec<u8>, method: Option<&str>) -> Result<()> {
        let line = self.wire_line(line);
        self.admit(line.len(), method)?;
        self.send_line(line, method)
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
        let line = self.wire_line(request_line(id, method, &params)?);
        let (waiter, answer) = oneshot::channel();
        // Reserve the line's place before registering for its answer, so a
        // request that does not fit fails without having been half accepted.
        self.admit(line.len(), Some(method))?;
        let closed = {
            let mut state = locked(&self.shared.state);
            let closed = state.closed.clone();
            if closed.is_none() {
                state.pending.insert(id, waiter);
            }
            closed
        };
        if let Some(reason) = closed {
            self.shared.outbound.release(line.len());
            return Err(self.shared.error_for(&reason, Some(method)));
        }
        let _slot = PendingSlot {
            shared: &self.shared,
            id,
        };
        self.send_line(line, Some(method))?;

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
            Ok(Reply::Malformed) => Err(Error::AppServerProtocol {
                message: format!(
                    "codex app-server answered `{method}` with neither result nor error"
                ),
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
    capabilities: Map<String, Value>,
    limits: Limits,
    transport: AppServerTransport,
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

    /// Set a field of the `initialize` request's `capabilities` by its
    /// protocol name, replacing an earlier value for the same name. Sent as
    /// given and not checked, except `optOutNotificationMethods`, which is
    /// combined with [`opt_out_notification`](Self::opt_out_notification) as
    /// the [module documentation](self#capabilities) describes.
    #[must_use]
    pub fn capability(mut self, name: impl Into<String>, value: Value) -> Self {
        self.capabilities.insert(name.into(), value);
        self
    }

    /// Set the `experimentalApi` capability, which asks the server to allow
    /// methods and fields it marks experimental. Shorthand for
    /// `capability("experimentalApi", Value::Bool(enabled))`.
    #[must_use]
    pub fn experimental_api(self, enabled: bool) -> Self {
        self.capability("experimentalApi", Value::Bool(enabled))
    }

    /// The most bytes one line from the server may hold, newline included. A
    /// longer line ends the session with [`Error::AppServerLimitExceeded`]
    /// once the limit is reached, without buffering the rest. Unlimited by
    /// default. Attached WebSocket mode counts JSON payload bytes instead.
    #[must_use]
    pub fn max_frame_bytes(mut self, bytes: usize) -> Self {
        self.limits.frame_bytes = Some(bytes);
        self
    }

    /// The most notifications and server requests that may wait for
    /// [`AppServer::next_message`]. One more ends the session. Unlimited by
    /// default.
    #[must_use]
    pub fn max_queued_messages(mut self, messages: usize) -> Self {
        self.limits.queued_messages = Some(messages);
        self
    }

    /// The most bytes, counted as each message's line plus its newline, that
    /// may wait for [`AppServer::next_message`]. Attached mode counts JSON
    /// payload bytes without WebSocket framing. Unlimited by default.
    #[must_use]
    pub fn max_queued_bytes(mut self, bytes: usize) -> Self {
        self.limits.queued_bytes = Some(bytes);
        self
    }

    /// The most lines that may wait to be written to the server, the one being
    /// written included. Unlimited by default.
    #[must_use]
    pub fn max_writer_queue_messages(mut self, messages: usize) -> Self {
        self.limits.writer_messages = Some(messages);
        self
    }

    /// The most bytes, newlines included, that may wait to be written to the
    /// server, the line being written included. Attached mode counts JSON
    /// payload bytes without WebSocket framing. Unlimited by default.
    #[must_use]
    pub fn max_writer_queue_bytes(mut self, bytes: usize) -> Self {
        self.limits.writer_bytes = Some(bytes);
        self
    }

    /// Attach to an existing server over a Unix WebSocket connection.
    ///
    /// Spawns no process and never signals the shared server. The endpoint
    /// must be selected explicitly. Verified with CLI 0.163.0; this provider
    /// transport is experimental. Reconnect by creating a fresh connection
    /// and explicitly reconciling threads and approvals; requests are not replayed.
    #[cfg(unix)]
    #[must_use]
    pub fn connect_unix(mut self, socket: impl Into<PathBuf>) -> Self {
        self.transport = AppServerTransport::UnixSocket {
            socket: socket.into(),
        };
        self
    }

    /// The `params` of the `initialize` request [`start`](Self::start) sends.
    ///
    /// # Errors
    ///
    /// [`Error::AppServerConfig`] if `optOutNotificationMethods` was set
    /// through [`capability`](Self::capability) to something other than an
    /// array of strings or `null`.
    pub fn initialize_params(&self) -> Result<Value> {
        let mut params = json!({
            "clientInfo": { "name": self.client_name, "version": self.client_version }
        });
        let mut capabilities = self.capabilities.clone();
        if let Some(given) = capabilities.get(OPT_OUT_KEY) {
            let valid = match given {
                Value::Null => true,
                Value::Array(items) => items.iter().all(Value::is_string),
                _ => false,
            };
            if !valid {
                return Err(Error::AppServerConfig {
                    message: format!(
                        "capabilities.{OPT_OUT_KEY} must be an array of method names or null"
                    ),
                });
            }
        }
        if !self.opt_out.is_empty() {
            let mut methods: Vec<String> = Vec::new();
            let given = capabilities.get(OPT_OUT_KEY).and_then(Value::as_array);
            let given = given.into_iter().flatten().filter_map(Value::as_str);
            for method in given.chain(self.opt_out.iter().map(String::as_str)) {
                if !methods.iter().any(|seen| seen == method) {
                    methods.push(method.to_string());
                }
            }
            capabilities.insert(OPT_OUT_KEY.into(), json!(methods));
        }
        if !capabilities.is_empty() {
            params["capabilities"] = Value::Object(capabilities);
        }
        Ok(params)
    }

    /// The owned-server arguments: `app-server --listen stdio://`. Empty for
    /// an attached connection, which spawns no command.
    ///
    /// `--listen stdio://` is the default transport. It is passed anyway so a
    /// change of default cannot move this client onto a socket.
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        match &self.transport {
            AppServerTransport::Stdio => {
                vec!["app-server".into(), "--listen".into(), "stdio://".into()]
            }
            #[cfg(unix)]
            AppServerTransport::UnixSocket { .. } => Vec::new(),
        }
    }

    /// Reject a setting that cannot work, before anything is spawned.
    fn validate(&self) -> Result<Value> {
        #[cfg(unix)]
        if let AppServerTransport::UnixSocket { socket } = &self.transport
            && socket.as_os_str().is_empty()
        {
            return Err(Error::AppServerConfig {
                message: "the socket path is empty".into(),
            });
        }
        self.initialize_params()
    }

    /// The command [`start`](Self::start) will run, as a copy-pasteable shell
    /// string, including the client's global args. Empty for an attached connection.
    #[must_use]
    pub fn to_command_string(&self) -> String {
        if !self.transport.owns_server() {
            return String::new();
        }
        crate::exec::command_string(self.codex, self.args())
    }

    /// Spawn `codex app-server` and complete the handshake.
    ///
    /// Owned mode runs `codex [global args] app-server --listen stdio://`. The
    /// client's binary, environment, working directory, process-group policy,
    /// `die_with_parent`, and `on_spawn` observer apply to that process. The
    /// handshake is bounded by the client's timeout, when set.
    ///
    /// # Errors
    ///
    /// [`Error::AppServerConfig`] for an invalid setting, before anything is
    /// spawned. [`Error::Io`] if the process cannot be spawned. If the process
    /// exits before the handshake finishes, the failure is classified from its
    /// stderr as any failed command is, so a rejected `-c` key is
    /// [`Error::Config`]. Otherwise the handshake's own error.
    pub async fn start(self) -> Result<AppServer> {
        let initialize_params = self.validate()?;
        #[cfg(unix)]
        if let AppServerTransport::UnixSocket { socket } = &self.transport {
            let socket = socket.clone();
            return self.start_unix(socket, initialize_params).await;
        }
        let codex = self.codex;
        let process = self.transport.process_name();
        let operation_args = self.args();
        let span = crate::exec::command_span("codex.app_server", codex, &operation_args);
        let args = crate::exec::assemble_args(codex, operation_args);
        let command_text = format!("{} {}", codex.binary.display(), args.join(" "));
        debug!(
            arg_count = args.len(),
            owns_server = self.transport.owns_server(),
            "starting codex app-server"
        );

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
                    message: format!("failed to spawn {process}: {e}"),
                    source: e,
                    working_dir: codex.working_dir.clone(),
                });
            }
        };
        let group = crate::exec::arm_and_notify(
            codex.process_group,
            codex.termination_signal,
            child.id(),
            codex.on_spawn.as_ref(),
        );
        let pid = child.id();

        let stdin = child.stdin.take().expect("stdin was configured as piped");
        let stdout = child.stdout.take().expect("stdout was configured as piped");
        let stderr = child.stderr.take().expect("stderr was configured as piped");

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (writer_tx, writer_rx) = mpsc::unbounded_channel();
        // Only owned mode creates a process kill target.
        let shared = Arc::new(Shared::new(
            events_tx,
            Some(KillTarget {
                pid,
                group: codex.process_group,
                #[cfg(windows)]
                handle: child.raw_handle().map(|handle| handle as usize),
            }),
            self.limits,
            process,
        ));

        let reader = tokio::spawn(read_stdout(
            stdout,
            Arc::clone(&shared),
            codex.output_limit,
            codex.working_dir.clone(),
        ));
        let stderr_reader = tokio::spawn(drain_stderr(stderr, Arc::clone(&shared)));
        let writer = tokio::spawn(write_stdin(stdin, writer_rx, Arc::clone(&shared)));

        let handle = AppServerHandle {
            shared,
            writer: writer_tx,
            timeout: codex.timeout,
        };
        let mut server = AppServer {
            handle,
            events: events_rx,
            finished: false,
            child: Some(child),
            tasks: vec![
                reader.abort_handle(),
                stderr_reader.abort_handle(),
                writer.abort_handle(),
            ],
            group,
            outcome,
            termination_grace: codex.termination_grace,
            working_dir: codex.working_dir.clone(),
            command: command_text,
            info: ServerInfo::default(),
            transport: self.transport,
        };

        let handshake = initialize(&server.handle, initialize_params)
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

#[cfg(unix)]
impl AppServerBuilder<'_> {
    async fn start_unix(self, socket: PathBuf, params: Value) -> Result<AppServer> {
        let codex = self.codex;
        let span = tracing::info_span!("codex.app_server", transport = "unix");
        let outcome = SpanOutcome::start(span.clone());
        let max = match (self.limits.frame_bytes, codex.output_limit) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        // Both aggregate-message and fragment limits are needed: a peer can
        // split a large message across individually small WebSocket frames.
        let config = WebSocketConfig::default()
            .max_frame_size(max)
            .max_message_size(max)
            .write_buffer_size(0);
        let connect = async {
            let stream = tokio::net::UnixStream::connect(&socket)
                .await
                .map_err(|source| Error::Io {
                    message: "failed to connect to app-server Unix socket".into(),
                    source,
                    working_dir: codex.working_dir.clone(),
                })?;
            let (stream, _) = tokio_tungstenite::client_async_with_config(
                "ws://localhost/",
                stream,
                Some(config),
            )
            .await
            .map_err(|error| Error::AppServerProtocol {
                message: format!("app-server WebSocket handshake failed: {error}"),
            })?;
            Ok::<_, Error>(stream)
        };
        let stream = if let Some(limit) = codex.timeout {
            tokio::time::timeout(limit, connect)
                .await
                .map_err(|_| Error::Timeout {
                    timeout_seconds: limit.as_secs(),
                })??
        } else {
            connect.await?
        };
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (writer_tx, writer_rx) = mpsc::unbounded_channel();
        let mut state = Shared::new(events_tx, None, self.limits, self.transport.process_name());
        state.newline_delimited = false;
        state.stderr_done.store(true, Ordering::Release);
        let shared = Arc::new(state);
        let task = tokio::spawn(socket_io(
            stream,
            writer_rx,
            Arc::clone(&shared),
            codex.output_limit,
            codex.termination_grace,
        ));
        // Construct the owner before awaiting initialize: cancelling start()
        // must abort its I/O task even if cloned internal state survives.
        let mut server = AppServer {
            handle: AppServerHandle {
                shared,
                writer: writer_tx,
                timeout: codex.timeout,
            },
            events: events_rx,
            finished: false,
            child: None,
            tasks: vec![task.abort_handle()],
            group: GroupKillGuard::new(None, codex.termination_signal),
            outcome,
            termination_grace: codex.termination_grace,
            working_dir: codex.working_dir.clone(),
            command: String::new(),
            info: ServerInfo::default(),
            transport: self.transport,
        };
        match initialize(&server.handle, params).instrument(span).await {
            Ok(info) => {
                server.info = info;
                Ok(server)
            }
            Err(error) => Err(server.abandon(error).await),
        }
    }
}

#[cfg(unix)]
async fn socket_io(
    stream: tokio_tungstenite::WebSocketStream<tokio::net::UnixStream>,
    mut lines: mpsc::UnboundedReceiver<Outgoing>,
    shared: Arc<Shared>,
    output_limit: Option<usize>,
    close_grace: Duration,
) {
    let (mut sink, mut source) = stream.split();
    let acknowledge_close = AtomicBool::new(false);
    let read = async {
        let mut captured = 0usize;
        loop {
            let next = tokio::select! {
                biased;
                () = shared.wait_closed() => return,
                next = source.next() => next,
            };
            let reason = match next {
                Some(Ok(WsMessage::Text(text))) => {
                    captured = captured.saturating_add(text.len());
                    if let Some(limit) = output_limit
                        && captured > limit
                    {
                        Some(Closed::OutputLimit { limit_bytes: limit })
                    } else {
                        route_bytes(&text, text.len(), &shared).err()
                    }
                }
                // Tungstenite handles pong and close replies; reading keeps
                // those control frames progressing independently of events.
                Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => None,
                Some(Ok(WsMessage::Close(_))) => {
                    acknowledge_close.store(true, Ordering::Release);
                    Some(Closed::Eof)
                }
                None => Some(Closed::Eof),
                Some(Ok(_)) => Some(Closed::Broken(
                    "app-server sent a non-text WebSocket message".into(),
                )),
                Some(Err(tokio_tungstenite::tungstenite::Error::Capacity(_))) => {
                    if let Some(limit) = output_limit
                        && shared.frame_bytes.is_none_or(|frame| limit <= frame)
                    {
                        Some(Closed::OutputLimit { limit_bytes: limit })
                    } else if let Some(max) = shared.frame_bytes {
                        Some(Closed::Limit {
                            limit: AppServerLimit::FrameBytes,
                            max,
                        })
                    } else {
                        Some(Closed::Broken("WebSocket capacity exceeded".into()))
                    }
                }
                Some(Err(error)) => Some(Closed::Broken(format!(
                    "app-server WebSocket failed: {error}"
                ))),
            };
            if let Some(reason) = reason {
                shared.fail(reason);
                return;
            }
        }
    };
    let write = async {
        loop {
            let next = tokio::select! {
                biased;
                () = shared.wait_closed() => break,
                next = lines.recv() => next,
            };
            let Some(Outgoing::Line(line)) = next else {
                break;
            };
            let text = String::from_utf8(line.bytes.clone()).expect("serialized JSON is UTF-8");
            let result = tokio::select! {
                biased;
                () = shared.wait_closed() => break,
                result = sink.send(WsMessage::Text(text.into())) => result,
            };
            if let Err(error) = result {
                shared.fail(Closed::Broken(format!(
                    "failed to write app-server WebSocket: {error}"
                )));
                break;
            }
        }
        if acknowledge_close.load(Ordering::Acquire) {
            // Reading Close queues the reply inside tungstenite. Waiters have
            // already failed; bound the remaining protocol cleanup separately.
            let _ = tokio::time::timeout(close_grace, sink.flush()).await;
        }
    };
    // Neither half depends on the application draining the event queue. On
    // failure or owner closure both futures wake and release the socket.
    tokio::join!(read, write);
}

async fn initialize(handle: &AppServerHandle, params: Value) -> Result<ServerInfo> {
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
    #[cfg(windows)]
    handle: Option<usize>,
}

impl KillTarget {
    /// Stop the server now. The stream is unusable and nothing will read what
    /// the server writes next, so leaving it running would only fill the pipe.
    ///
    /// Unix signals the owned group or child. Windows terminates the owned
    /// process using its still-valid native handle.
    fn signal(self) {
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
        #[cfg(windows)]
        if let Some(handle) = self.handle {
            // SAFETY: this is the owned child's handle, not a reopened PID.
            // The Shared kill lock is held across this call and revoked before
            // reaping or dropping the child can invalidate the handle.
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(handle as _, 1);
            }
        }
        #[cfg(not(unix))]
        let _ = (self.pid, self.group);
    }
}

/// Read the server's stdout: route answers to waiting requests and everything
/// else to the event queue.
async fn read_stdout(
    stdout: ChildStdout,
    shared: Arc<Shared>,
    output_limit: Option<usize>,
    working_dir: Option<PathBuf>,
) {
    let mut reader = BufReader::new(stdout);
    let mut captured = 0usize;
    let reason = loop {
        let line = match read_frame(
            &mut reader,
            &mut captured,
            output_limit,
            shared.frame_bytes,
            &working_dir,
        )
        .await
        {
            Ok(Some(line)) => line,
            Ok(None) => {
                shared.stderr_settled().await;
                break Closed::Eof;
            }
            Err(reason) => {
                shared.fatal(reason.clone());
                break reason;
            }
        };
        let (line, wire_bytes) = line;
        if !line.trim_start().starts_with('{') {
            debug!(
                bytes = line.len(),
                "ignoring a non-JSON line from codex app-server"
            );
            continue;
        }
        if let Err(reason) = route_bytes(&line, wire_bytes, &shared) {
            shared.fatal(reason.clone());
            break reason;
        }
    };
    shared.fail(reason);
}

/// Read one line, as [`crate::streaming`] reads one, but also bounded by
/// `frame_limit`. The check comes before each chunk is copied, so a line that
/// never ends is rejected having buffered at most the limit.
async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    captured: &mut usize,
    output_limit: Option<usize>,
    frame_limit: Option<usize>,
    working_dir: &Option<PathBuf>,
) -> std::result::Result<Option<(String, usize)>, Closed> {
    let mut line = Vec::new();
    let mut wire_bytes = 0;
    loop {
        let available = reader.fill_buf().await.map_err(|source| {
            let error = Error::Io {
                message: format!("failed to read stdout line: {source}"),
                source,
                working_dir: working_dir.clone(),
            };
            Closed::Broken(error.to_string())
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
        if let Some(limit) = output_limit
            && captured.saturating_add(take) > limit
        {
            return Err(Closed::OutputLimit { limit_bytes: limit });
        }
        if let Some(max) = frame_limit
            && line.len().saturating_add(take) > max
        {
            return Err(Closed::Limit {
                limit: AppServerLimit::FrameBytes,
                max,
            });
        }
        line.extend_from_slice(&available[..take]);
        wire_bytes += take;
        *captured = captured.saturating_add(take);
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
        .map(|line| Some((line, wire_bytes)))
        .map_err(|_| Closed::Broken("io error: failed to decode stdout line".into()))
}

/// Classify one line from the server and deliver it.
#[cfg(test)]
fn route(line: &str, shared: &Shared) -> std::result::Result<(), Closed> {
    route_bytes(line, line.len() + 1, shared)
}

fn route_bytes(line: &str, bytes: usize, shared: &Shared) -> std::result::Result<(), Closed> {
    let value: Value = serde_json::from_str(line).map_err(|e| {
        let head: String = line.chars().take(120).collect();
        Closed::Broken(format!("codex app-server sent invalid JSON ({e}): {head}"))
    })?;
    let method = value.get("method").and_then(Value::as_str);
    let id = value.get("id");
    match (method, id) {
        (Some(method), Some(id)) => {
            let Ok(id) = serde_json::from_value::<RequestId>(id.clone()) else {
                warn!(
                    method_bytes = method.len(),
                    "ignoring a server request with an unusable id"
                );
                return Ok(());
            };
            let request = ServerRequest {
                id,
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            };
            shared.deliver(ServerMessage::Request(request), bytes)?;
        }
        (Some(method), None) => {
            let notification = Notification {
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            };
            shared.deliver(ServerMessage::Notification(notification), bytes)?;
        }
        (None, Some(id)) if value.get("result").is_some() || value.get("error").is_some() => {
            deliver_answer(&value, id, shared);
        }
        (None, Some(id)) => {
            // Shaped like an answer, but neither a result nor an error. If it
            // names a waiting request, fail that request rather than leave it
            // waiting for an answer that will not come.
            let waiter = id
                .as_i64()
                .and_then(|id| locked(&shared.state).pending.remove(&id));
            match waiter {
                Some(waiter) => {
                    let _ = waiter.send(Reply::Malformed);
                }
                None => warn!("ignoring an answer with neither result nor error"),
            }
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
async fn write_stdin<W: AsyncWrite + Unpin>(
    mut stdin: W,
    mut lines: mpsc::UnboundedReceiver<Outgoing>,
    shared: Arc<Shared>,
) {
    loop {
        let outgoing = tokio::select! {
            biased;
            () = shared.wait_closed() => break,
            outgoing = lines.recv() => outgoing,
        };
        let Some(Outgoing::Line(line)) = outgoing else {
            break;
        };
        let written = tokio::select! {
            biased;
            () = shared.wait_closed() => break,
            result = async { stdin.write_all(&line.bytes).await?; stdin.flush().await } => result,
        };
        if let Err(error) = written {
            if error.kind() == std::io::ErrorKind::BrokenPipe {
                // Preserve the reader's native startup-error classification.
                debug!("codex app-server closed its stdin");
            } else {
                shared.fatal(Closed::Broken(format!(
                    "failed to write to codex app-server stdin: {error}"
                )));
            }
            break;
        }
    }
    // Dropping the receiver also drops each queued reservation.
    lines.close();
    while lines.try_recv().is_ok() {}

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
/// and wait. An attached connection closes only its socket; the shared server
/// and its turns remain the server owner's responsibility.
pub struct AppServer {
    handle: AppServerHandle,
    events: mpsc::UnboundedReceiver<Event>,
    finished: bool,
    child: Option<Child>,
    tasks: Vec<tokio::task::AbortHandle>,
    group: GroupKillGuard,
    outcome: SpanOutcome,
    termination_grace: Duration,
    working_dir: Option<PathBuf>,
    command: String,
    info: ServerInfo,
    transport: AppServerTransport,
}

impl std::fmt::Debug for AppServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppServer")
            .field("pid", &self.pid())
            .field("transport", &self.transport)
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
            capabilities: Map::new(),
            limits: Limits::default(),
            transport: AppServerTransport::Stdio,
        }
    }

    /// What the server reported about itself during the handshake.
    #[must_use]
    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    /// How this client reaches the server, and whether it owns it.
    #[must_use]
    pub fn transport(&self) -> &AppServerTransport {
        &self.transport
    }

    /// The owned server's process ID while running. `None` for an attached
    /// connection, which neither starts nor owns a process.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
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
    /// [`Error::OutputLimitExceeded`], [`Error::AppServerLimitExceeded`], or
    /// [`Error::AppServerProtocol`] when the stream became unusable. The owned
    /// process has been killed by then. The error
    /// is returned once; later calls return `Ok(None)`.
    pub async fn next_message(&mut self) -> Result<Option<ServerMessage>> {
        if self.finished {
            return Ok(None);
        }
        match self.events.recv().await {
            Some(Event::Message(message, bytes)) => {
                self.handle.shared.inbound.release(bytes);
                Ok(Some(message))
            }
            Some(Event::Closed(reason)) => {
                self.finished = true;
                match reason {
                    Closed::Eof | Closed::Terminated => Ok(None),
                    reason => {
                        // Already killed on Unix, where the reader signals
                        // the group as soon as the stream fails. This covers
                        // the platforms that cannot.
                        if let Some(child) = &mut self.child {
                            let _ = child.start_kill();
                        }
                        Err(self.handle.shared.error_for(&reason, None))
                    }
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
    /// for the end. An attached connection closes its local socket immediately;
    /// the shared server keeps running. Pending requests fail on both paths.
    ///
    /// # Errors
    ///
    /// If the server exits with a non-zero status by itself, the failure is
    /// classified from its stderr like any failed command
    /// ([`Error::CommandFailed`] and its refinements).
    pub async fn shutdown(mut self) -> Result<()> {
        // The child is about to be reaped, after which its pid is not ours to
        // signal.
        self.handle.shared.revoke_kill();
        // Stop admission now, but let owned stdio flush already accepted
        // messages before Close. The termination grace bounds a stalled writer.
        self.handle
            .shared
            .end(Closed::Terminated, self.child.is_none());
        let Some(child) = &mut self.child else {
            self.outcome.settle("ok", None);
            return Ok(());
        };
        let _ = self.handle.writer.send(Outgoing::Close);
        match tokio::time::timeout(self.termination_grace, child.wait()).await {
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
    /// Sends the client's configured termination signal to the group,
    /// waits through the client's termination grace,
    /// kills what remains, and reaps the child. Requests still waiting fail.
    ///
    /// Shell commands the server started run in groups of their own and are
    /// not reached by group signaling. Cooperative cleanup depends on the
    /// server; see the [module documentation](self#lifecycle-and-cancellation).
    /// [`shutdown`](Self::shutdown) is the way to stop those.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the child cannot be waited on.
    pub async fn terminate(mut self) -> Result<()> {
        self.handle.shared.revoke_kill();
        self.handle.shared.fail(Closed::Terminated);
        let Some(child) = &mut self.child else {
            self.outcome.settle("cancelled", None);
            return Ok(());
        };
        let result = crate::exec::terminate_and_reap(
            child,
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
        self.handle.shared.revoke_kill();
        let outcome = if matches!(error, Error::Timeout { .. }) {
            "timeout"
        } else {
            "failed"
        };
        let Some(child) = &mut self.child else {
            self.handle.shared.fail(Closed::Terminated);
            self.outcome.settle(outcome, None);
            return error;
        };
        // Only a server that closed its own output has failed by itself. When
        // the reader gave up on the stream it killed the child, so the exit
        // status says nothing and the error already says what happened.
        let by_server = matches!(locked(&self.handle.shared.state).closed, Some(Closed::Eof));

        // Has the child already ended? One that closed its output is given a
        // moment to finish exiting. Otherwise only look, so a server that is
        // merely silent is not waited on.
        let ended = if by_server {
            tokio::time::timeout(STARTUP_EXIT_WAIT, child.wait())
                .await
                .ok()
        } else {
            child.try_wait().transpose()
        };
        if let Some(Ok(status)) = ended {
            // Reaped: nothing is left to terminate, and its pid is no longer
            // ours to signal.
            self.group.disarm();
            self.handle.shared.stderr_settled().await;
            let code = status.code().unwrap_or(-1);
            self.outcome.settle(outcome, Some(code));
            if by_server && !status.success() {
                // A server that rejects its arguments or config exits before
                // it can answer. Its exit status and stderr say more than
                // "closed its output".
                return Error::from_command_failure(
                    self.command.clone(),
                    code,
                    String::new(),
                    self.handle.shared.stderr_summary(),
                    self.working_dir.clone(),
                );
            }
            return error;
        }

        self.handle.shared.fail(Closed::Terminated);
        let _ = crate::exec::terminate_and_reap(
            child,
            &mut self.group,
            self.termination_grace,
            self.working_dir.as_deref(),
        )
        .await;
        self.outcome.settle(outcome, None);
        error
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        // The fields that follow kill the group and let tokio reap the child.
        // After that the reader task must not signal the pid.
        self.handle.shared.revoke_kill();
        self.handle.shared.fail(Closed::Terminated);
        for task in &self.tasks {
            task.abort();
        }
        self.events.close();
        while let Ok(event) = self.events.try_recv() {
            if let Event::Message(_, bytes) = event {
                self.handle.shared.inbound.release(bytes);
            }
        }
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

    /// A server that exits 0 before it answers has been reaped by the time the
    /// failure is reported, so there is nothing left to terminate. Waiting out
    /// the grace period (and signalling a pid that is no longer ours) is the
    /// failure this guards.
    #[tokio::test]
    async fn a_server_that_exits_cleanly_before_answering_fails_the_start_without_waiting() {
        let codex = fake("exit-clean")
            .termination_grace(Duration::from_secs(4))
            .build()
            .unwrap();
        let started_at = std::time::Instant::now();
        let error = AppServer::builder(&codex).start().await.unwrap_err();
        assert!(
            matches!(error, Error::AppServerProtocol { .. }),
            "{error:?}"
        );
        assert!(
            started_at.elapsed() < Duration::from_secs(3),
            "start waited out the grace period for a server that had already exited"
        );
    }

    /// The reader kills the server when the stream breaks. That must not be
    /// mistaken for the server failing by itself, or the useful message is
    /// replaced by an exit status.
    #[tokio::test]
    async fn invalid_json_during_the_handshake_is_reported_as_invalid_json() {
        let codex = fake("bad-init").build().unwrap();
        let error = AppServer::builder(&codex).start().await.unwrap_err();
        assert!(
            matches!(&error, Error::AppServerProtocol { message } if message.contains("invalid JSON")),
            "{error:?}"
        );
        assert_eq!(error.exit_code(), None);
    }

    /// A response that names a waiting request but has neither `result` nor
    /// `error` is not a valid answer. Ignoring it would leave the request
    /// waiting for one that will never come.
    #[tokio::test]
    async fn a_response_with_neither_result_nor_error_fails_the_request_it_names() {
        let server = started("no-result").await;
        let error = server
            .thread_start(ThreadStartParams::new())
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::AppServerProtocol { message }
                if message.contains("neither result nor error") && message.contains("thread/start")),
            "{error:?}"
        );
        // The session itself is still usable.
        assert!(server.notify("initialized", Value::Null).is_ok());
        server.shutdown().await.unwrap();
    }

    /// The reader task holds a way to kill the server. It must stop being able
    /// to once the client is done with the child: after that the pid may
    /// belong to someone else.
    #[tokio::test]
    async fn the_readers_kill_switch_is_revoked_when_the_client_is_done_with_the_child() {
        // Never signals this process: the target is a `sleep` started here.
        let sleeper = |seconds: &str| {
            std::process::Command::new("sleep")
                .arg(seconds)
                .spawn()
                .unwrap()
        };
        let (events, _received) = mpsc::unbounded_channel();

        let mut live = sleeper("60");
        let shared = Shared::new(
            events.clone(),
            Some(KillTarget {
                pid: Some(live.id()),
                group: false,
            }),
            Limits::default(),
            "test",
        );
        shared.fire_kill();
        assert!(
            live.wait().unwrap().code().is_none(),
            "the switch should have killed the process it was armed with"
        );

        let mut revoked = sleeper("60");
        let shared = Shared::new(
            events,
            Some(KillTarget {
                pid: Some(revoked.id()),
                group: false,
            }),
            Limits::default(),
            "test",
        );
        shared.revoke_kill();
        shared.fire_kill();
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            revoked.try_wait().unwrap().is_none(),
            "a revoked switch still killed the process"
        );
        revoked.kill().unwrap();
        revoked.wait().unwrap();
    }

    #[tokio::test]
    async fn every_way_of_ending_the_session_revokes_the_kill_switch() {
        let armed = |shared: &Shared| locked(&shared.kill).is_some();

        let server = started("turn").await;
        let shared = Arc::clone(&server.handle.shared);
        assert!(armed(&shared));
        server.shutdown().await.unwrap();
        assert!(!armed(&shared), "shutdown left the switch armed");

        let server = started("turn").await;
        let shared = Arc::clone(&server.handle.shared);
        server.terminate().await.unwrap();
        assert!(!armed(&shared), "terminate left the switch armed");

        let server = started("turn").await;
        let shared = Arc::clone(&server.handle.shared);
        drop(server);
        assert!(!armed(&shared), "dropping left the switch armed");
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

    /// A server that dies at startup can be noticed by the client's write (the
    /// pipe is closed, EPIPE) before its read (the stream has ended). Either
    /// way the server went away by itself, and what says why is its stderr and
    /// exit status, which the reader reports at the end of the stream. A write
    /// error must not become the session's failure and hide that; this was
    /// seen as a flaky failure on Linux CI.
    ///
    /// An in-memory pipe stands in for the child's stdin. A real one made this
    /// test hang under load: on macOS a pipe's close-on-exec flag is set after
    /// it is created, so a process forked by a concurrent test can inherit the
    /// read end, and then the write never fails.
    #[tokio::test]
    async fn a_broken_stdin_pipe_leaves_the_verdict_to_the_reader() {
        let (stdin, server_end) = tokio::io::duplex(64);
        drop(server_end);

        let (events, _received) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared::new(events, None, Limits::default(), "test"));
        let (lines, receiver) = mpsc::unbounded_channel();
        lines
            .send(Outgoing::Line(ReservedLine {
                bytes: b"{}\n".to_vec(),
                budget: Arc::clone(&shared.outbound),
            }))
            .unwrap();

        // Ends by itself when the write fails. If it did not fail, `lines` is
        // still open and this would wait forever, so bound it.
        tokio::time::timeout(
            Duration::from_secs(10),
            write_stdin(stdin, receiver, Arc::clone(&shared)),
        )
        .await
        .expect("the write to a closed pipe did not fail");

        assert!(
            locked(&shared.state).closed.is_none(),
            "a write to a closed pipe was recorded as the session's failure"
        );
        drop(lines);
    }

    /// End to end: a server that closes its input and then fails is reported
    /// by its exit status and stderr. The race that once made this flaky on
    /// Linux CI is pinned by `a_broken_stdin_pipe_leaves_the_verdict_to_the_reader`.
    #[tokio::test]
    async fn a_server_that_closes_its_input_and_then_fails_is_reported_by_its_exit_status() {
        let codex = fake("close-stdin").build().unwrap();
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
            // Long enough for the fixture's SIGTERM handler to run on a
            // loaded machine, so the marker below means what it says.
            .termination_grace(Duration::from_secs(1))
            .build()
            .unwrap();
        let server = AppServer::builder(&codex).start().await.unwrap();
        let (parent, child) = read_pids(&pid_file).await;
        assert!(is_running_for_test(parent) && is_running_for_test(child));

        server.terminate().await.unwrap();

        // The direct child has been reaped. Its subprocess got SIGKILL at the
        // same moment, but nothing waits for it, so allow it a moment to die.
        assert!(
            wait_until_gone(parent).await,
            "the server ({parent}) survived"
        );
        assert!(
            wait_until_gone(child).await,
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
    async fn dropping_during_app_server_termination_grace_kills_the_group() {
        let fixture = crate::test_support::GracePeriodFixture::new("app-server-grace-drop");
        let codex = fixture.configure(fake("ignores-eof")).build().unwrap();
        let server = AppServer::builder(&codex).start().await.unwrap();
        fixture.assert_drop_during_grace(server.terminate()).await;
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
            // Long enough for the fixture's SIGTERM handler to run on a
            // loaded machine, so the marker below means what it says.
            .termination_grace(Duration::from_secs(1))
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
            wait_until_gone(parent).await,
            "the server ({parent}) survived"
        );
        assert!(
            wait_until_gone(child).await,
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
        // Up to ten seconds: starting bash can be slow on a loaded machine.
        for _ in 0..1000 {
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
        // The short timeout is for the requests below, not the handshake,
        // which has to be allowed to take as long as a loaded machine needs.
        let server = started("mute-after-init").await;
        let mut handle = server.handle();
        handle.timeout = Some(Duration::from_millis(150));

        for _ in 0..2 {
            let error = handle.request("thread/start", json!({})).await.unwrap_err();
            assert!(matches!(error, Error::Timeout { .. }), "{error:?}");
        }
        assert!(
            locked(&handle.shared.state).pending.is_empty(),
            "a timed-out request left its slot behind"
        );
        // A timeout is a verdict on one request, not on the session.
        assert!(handle.notify("initialized", Value::Null).is_ok());
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
        // over and over while messages arrive. Bounded by time, not by the
        // number of polls, so a slow machine only makes it take longer.
        let mut seen = Vec::new();
        let mut finished = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
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
        let shared = Shared::new(events, None, Limits::default(), "test");
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
            Ok(Event::Message(ServerMessage::Notification(n), _)) if n.method == "a/b"
        ));
        assert!(route(r#"{"method":"a/b","id":"s"}"#, &shared).is_ok());
        assert!(matches!(
            received.try_recv(),
            Ok(Event::Message(ServerMessage::Request(r), _)) if r.id == RequestId::String("s".into())
        ));
        assert!(matches!(
            route(r#"{"method": oops"#, &shared),
            Err(Closed::Broken(_))
        ));
    }

    fn temp_log(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-wrapper-{}-app-server-{label}.log",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn wire_lines(log: &std::path::Path) -> Vec<Value> {
        let lines = std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).expect("every client line is JSON"))
            .collect();
        let _ = std::fs::remove_file(log);
        lines
    }

    fn offline() -> Codex {
        // Never spawned: these settings are rejected first, and if one were
        // not, spawning this path would fail with a different error.
        Codex::builder()
            .binary("/nonexistent/codex-wrapper-test-binary")
            .build()
            .unwrap()
    }

    #[test]
    fn initialize_carries_no_capabilities_unless_one_is_set() {
        let codex = offline();
        let params = AppServer::builder(&codex)
            .client_info("probe", "1")
            .initialize_params()
            .unwrap();
        assert_eq!(
            params,
            json!({ "clientInfo": { "name": "probe", "version": "1" } })
        );
    }

    #[test]
    fn capabilities_are_kept_and_both_opt_out_sources_are_combined_once_each() {
        let codex = offline();
        let params = AppServer::builder(&codex)
            .client_info("probe", "1")
            .opt_out_notification("item/agentMessage/delta")
            .experimental_api(true)
            .capability(
                "optOutNotificationMethods",
                json!(["thread/status/changed", "item/agentMessage/delta"]),
            )
            .capability("someFutureCapability", json!({ "k": [1, 2] }))
            .opt_out_notification("mcpServer/startupStatus/updated")
            .opt_out_notification("thread/status/changed")
            .initialize_params()
            .unwrap();
        assert_eq!(
            params["capabilities"],
            json!({
                "experimentalApi": true,
                "someFutureCapability": { "k": [1, 2] },
                "optOutNotificationMethods": [
                    "thread/status/changed",
                    "item/agentMessage/delta",
                    "mcpServer/startupStatus/updated"
                ]
            })
        );

        // A null list is sent as given, or replaced by the builder's list.
        let null = AppServer::builder(&codex).capability(OPT_OUT_KEY, Value::Null);
        assert_eq!(
            null.initialize_params().unwrap()["capabilities"],
            json!({ "optOutNotificationMethods": null })
        );
        let merged = null.opt_out_notification("a/b");
        assert_eq!(
            merged.initialize_params().unwrap()["capabilities"],
            json!({ "optOutNotificationMethods": ["a/b"] })
        );
    }

    #[tokio::test]
    async fn invalid_settings_are_rejected_before_anything_is_spawned() {
        let codex = offline();
        for value in [json!("item/agentMessage/delta"), json!([1]), json!({})] {
            let error = AppServer::builder(&codex)
                .capability(OPT_OUT_KEY, value.clone())
                .start()
                .await
                .unwrap_err();
            assert!(
                matches!(&error, Error::AppServerConfig { message } if message.contains(OPT_OUT_KEY)),
                "{value}: {error:?}"
            );
        }
        let error = AppServer::builder(&codex)
            .connect_unix("")
            .start()
            .await
            .unwrap_err();
        assert!(matches!(error, Error::AppServerConfig { .. }), "{error:?}");
    }

    #[tokio::test]
    async fn the_first_handshake_sends_the_combined_capabilities() {
        let log = temp_log("capabilities");
        let codex = fake("turn")
            .env("FAKE_APP_SERVER_LOG", log.to_str().unwrap())
            .build()
            .unwrap();
        let server = AppServer::builder(&codex)
            .client_info("probe", "1.2.3")
            .experimental_api(true)
            .capability(OPT_OUT_KEY, json!(["thread/status/changed"]))
            .opt_out_notification("item/agentMessage/delta")
            .start()
            .await
            .unwrap();
        server.shutdown().await.unwrap();
        let lines = wire_lines(&log);
        assert_eq!(
            lines[0],
            json!({
                "id": 0,
                "method": "initialize",
                "params": {
                    "clientInfo": { "name": "probe", "version": "1.2.3" },
                    "capabilities": {
                        "experimentalApi": true,
                        "optOutNotificationMethods": [
                            "thread/status/changed",
                            "item/agentMessage/delta"
                        ]
                    }
                }
            })
        );
        assert_eq!(lines[1], json!({ "method": "initialized" }));
    }

    #[test]
    fn a_budget_counts_messages_and_bytes_and_frees_them_on_release() {
        let budget = Budget::new(
            Some(2),
            Some(10),
            AppServerLimit::QueuedMessages,
            AppServerLimit::QueuedBytes,
        );
        budget.reserve(4).unwrap();
        budget.reserve(4).unwrap();
        assert!(matches!(
            budget.reserve(1),
            Err(Closed::Limit {
                limit: AppServerLimit::QueuedMessages,
                max: 2
            })
        ));
        budget.release(4);
        assert!(matches!(
            budget.reserve(7),
            Err(Closed::Limit {
                limit: AppServerLimit::QueuedBytes,
                max: 10
            })
        ));
        // A refused reservation counts nothing.
        assert_eq!(budget.used(), (1, 4));
        budget.reserve(6).unwrap();
        assert_eq!(budget.used(), (2, 10));

        let unlimited = Budget::new(
            None,
            None,
            AppServerLimit::QueuedMessages,
            AppServerLimit::QueuedBytes,
        );
        for _ in 0..1000 {
            unlimited.reserve(usize::MAX / 2).unwrap();
        }
    }

    #[test]
    fn queued_events_count_until_read_and_answers_never_wait_behind_them() {
        let line = r#"{"method":"a/b"}"#;
        let cost = line.len() + 1;
        let (events, mut received) = mpsc::unbounded_channel();
        let limits = Limits {
            queued_messages: Some(2),
            ..Limits::default()
        };
        let shared = Shared::new(events, None, limits, "test");
        route(line, &shared).unwrap();
        route(line, &shared).unwrap();
        assert_eq!(shared.inbound.used(), (2, 2 * cost));

        // The queue is full, and an answer still reaches its request.
        let (waiter, mut answer) = oneshot::channel();
        locked(&shared.state).pending.insert(7, waiter);
        route(r#"{"id":7,"result":{"ok":true}}"#, &shared).unwrap();
        assert!(matches!(answer.try_recv(), Ok(Reply::Result(v)) if v["ok"] == true));

        assert!(matches!(
            route(line, &shared),
            Err(Closed::Limit {
                limit: AppServerLimit::QueuedMessages,
                max: 2
            })
        ));
        // Reading one, as `next_message` does, makes room for one more.
        let Ok(Event::Message(_, bytes)) = received.try_recv() else {
            panic!("a message was queued");
        };
        assert_eq!(bytes, cost);
        shared.inbound.release(bytes);
        route(line, &shared).unwrap();

        let (events, _received) = mpsc::unbounded_channel();
        let limits = Limits {
            queued_bytes: Some(2 * cost),
            ..Limits::default()
        };
        let shared = Shared::new(events, None, limits, "test");
        route(line, &shared).unwrap();
        route(line, &shared).unwrap();
        assert!(matches!(
            route(line, &shared),
            Err(Closed::Limit {
                limit: AppServerLimit::QueuedBytes,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn a_line_longer_than_the_frame_limit_is_rejected_without_buffering_it() {
        let read = |input: &'static [u8], max: usize| async move {
            let mut reader = BufReader::new(input);
            read_frame(&mut reader, &mut 0, None, Some(max), &None).await
        };
        assert_eq!(
            read(b"abc\n", 4)
                .await
                .unwrap()
                .as_ref()
                .map(|(line, _)| line.as_str()),
            Some("abc")
        );
        assert!(matches!(
            read(b"abc\n", 3).await,
            Err(Closed::Limit {
                limit: AppServerLimit::FrameBytes,
                max: 3
            })
        ));

        // A line that never ends: bounded by the limit, not the input.
        let mut endless = BufReader::new(tokio::io::repeat(b'x'));
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            read_frame(&mut endless, &mut 0, None, Some(64 * 1024), &None),
        )
        .await
        .expect("an unterminated line was read without bound");
        assert!(matches!(
            result,
            Err(Closed::Limit {
                limit: AppServerLimit::FrameBytes,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn the_frame_limit_ends_the_session_and_kills_the_server() {
        let pid = Arc::new(Mutex::new(None));
        let observed = Arc::clone(&pid);
        let codex = fake("flood")
            .on_spawn(Arc::new(move |info| {
                *observed.lock().unwrap() = Some(info.pid);
            }))
            .build()
            .unwrap();
        let mut server = AppServer::builder(&codex)
            .max_frame_bytes(1024)
            .start()
            .await
            .unwrap();
        let (_, error) = read_until_error(&mut server).await;
        assert!(
            matches!(
                error,
                Error::AppServerLimitExceeded {
                    limit: AppServerLimit::FrameBytes,
                    max: 1024
                }
            ),
            "{error:?}"
        );
        let pid = pid.lock().unwrap().expect("the server was spawned");
        assert!(wait_until_gone(pid).await, "the server ({pid}) survived");
    }

    async fn wait_until_closed(server: &AppServer) {
        for _ in 0..2000 {
            if server.handle.shared.closed_reason().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the session never failed");
    }

    async fn read_until_error(server: &mut AppServer) -> (Vec<ServerMessage>, Error) {
        let mut seen = Vec::new();
        loop {
            let next = tokio::time::timeout(Duration::from_secs(10), server.next_message())
                .await
                .expect("the stream should end with an error");
            match next {
                Ok(Some(message)) => seen.push(message),
                Ok(None) => panic!("the stream ended without an error: {:?}", methods(&seen)),
                Err(error) => return (seen, error),
            }
        }
    }

    #[tokio::test]
    async fn answers_pass_a_burst_of_unread_events_and_reading_them_frees_the_queue() {
        // remoteControl/status/changed, 30 status changes, thread/started.
        let codex = fake("burst")
            .env("FAKE_APP_SERVER_BURST", "30")
            .build()
            .unwrap();
        let mut server = AppServer::builder(&codex)
            .max_queued_messages(32)
            .start()
            .await
            .unwrap();
        // Answered while the burst sits unread in the queue.
        server.thread_start(ThreadStartParams::new()).await.unwrap();
        for _ in 0..32 {
            tokio::time::timeout(Duration::from_secs(10), server.next_message())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        assert_eq!(server.handle.shared.inbound.used(), (0, 0));
        // Another 31 fit only because the first 32 were read.
        server.thread_start(ThreadStartParams::new()).await.unwrap();
        for _ in 0..31 {
            tokio::time::timeout(Duration::from_secs(10), server.next_message())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_queue_overflow_fails_the_request_waiting_behind_it() {
        let codex = fake("burst")
            .env("FAKE_APP_SERVER_BURST", "30")
            .build()
            .unwrap();
        let mut server = AppServer::builder(&codex)
            .max_queued_messages(5)
            .start()
            .await
            .unwrap();
        let error = server
            .thread_start(ThreadStartParams::new())
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::AppServerLimitExceeded {
                    limit: AppServerLimit::QueuedMessages,
                    max: 5
                }
            ),
            "{error:?}"
        );
        let (seen, error) = read_until_error(&mut server).await;
        assert_eq!(seen.len(), 5);
        assert!(matches!(error, Error::AppServerLimitExceeded { .. }));
        server.terminate().await.unwrap();
    }

    /// Neither an approval request nor a turn's terminal event is dropped to
    /// stay under the limit: the one that does not fit ends the session, and
    /// the caller sees the error after the messages that did fit.
    #[tokio::test]
    async fn approvals_and_terminal_events_that_do_not_fit_fail_the_session() {
        // Five messages precede the approval request, ten end the turn.
        for (mode, max, last_fitting) in [
            ("approval", 5, "item/started"),
            ("turn", 9, "thread/status/changed"),
        ] {
            let codex = fake(mode).build().unwrap();
            let mut server = AppServer::builder(&codex)
                .max_queued_messages(max)
                .start()
                .await
                .unwrap();
            let thread = server.thread_start(ThreadStartParams::new()).await.unwrap();
            server
                .turn_start(TurnStartParams::text(&thread.id, "go"))
                .await
                .unwrap();
            // Read nothing until the overflow, so it happens where expected.
            wait_until_closed(&server).await;

            let (seen, error) = read_until_error(&mut server).await;
            assert!(
                matches!(
                    error,
                    Error::AppServerLimitExceeded {
                        limit: AppServerLimit::QueuedMessages,
                        max: m
                    } if m == max
                ),
                "{mode}: {error:?}"
            );
            assert_eq!(seen.len(), max, "{mode}");
            assert_eq!(methods(&seen).last(), Some(&last_fitting), "{mode}");
            assert!(
                !seen.iter().any(|m| matches!(m, ServerMessage::Request(_))),
                "{mode}"
            );
            assert!(server.next_message().await.unwrap().is_none());

            // Nothing more is accepted.
            let again = server.request("thread/start", json!({})).await.unwrap_err();
            assert!(
                matches!(again, Error::AppServerLimitExceeded { .. }),
                "{again:?}"
            );
            assert!(server.respond(RequestId::Number(0), json!({})).is_err());
            server.terminate().await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_line_being_written_holds_its_place_until_the_write_finishes() {
        let (stdin, mut server_end) = tokio::io::duplex(8);
        let line = notification_line("a/b", &json!({ "pad": "x".repeat(10) })).unwrap();
        let cost = line.len();
        let limits = Limits {
            // Room for one line, not two.
            writer_bytes: Some(cost + cost / 2),
            ..Limits::default()
        };
        let (events, _received) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared::new(events, None, limits, "test"));
        let (writer, lines) = mpsc::unbounded_channel();
        let task = tokio::spawn(write_stdin(stdin, lines, Arc::clone(&shared)));
        let handle = AppServerHandle {
            shared: Arc::clone(&shared),
            writer,
            timeout: None,
        };

        handle
            .notify("a/b", json!({ "pad": "x".repeat(10) }))
            .unwrap();
        // The writer has taken the line and is stuck: the pipe holds 8 bytes.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(shared.outbound.used(), (1, cost));

        let drained = |shared: Arc<Shared>| async move {
            for _ in 0..500 {
                if shared.outbound.used() == (0, 0) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!(
                "a written line kept its place: {:?}",
                shared.outbound.used()
            );
        };
        let mut received = vec![0; cost];
        server_end.read_exact(&mut received).await.unwrap();
        assert_eq!(received, line);
        drained(Arc::clone(&shared)).await;
        // Fits again now that the first line is written.
        handle
            .notify("a/b", json!({ "pad": "x".repeat(10) }))
            .unwrap();
        server_end.read_exact(&mut received).await.unwrap();
        drained(Arc::clone(&shared)).await;

        // Two at once do not fit while the writer is stuck on the first.
        handle
            .notify("a/b", json!({ "pad": "x".repeat(10) }))
            .unwrap();
        let error = handle
            .notify("a/b", json!({ "pad": "x".repeat(10) }))
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::AppServerLimitExceeded {
                    limit: AppServerLimit::WriterQueueBytes,
                    ..
                }
            ),
            "{error:?}"
        );
        drop(handle);
        drop(server_end);
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("the writer stopped")
            .unwrap();
        assert_eq!(shared.outbound.used(), (0, 0));
    }

    #[tokio::test]
    async fn a_server_that_stops_reading_fails_the_session_at_the_writer_limit() {
        let pid = Arc::new(Mutex::new(None));
        let observed = Arc::clone(&pid);
        let codex = fake("stops-reading")
            .on_spawn(Arc::new(move |info| {
                *observed.lock().unwrap() = Some(info.pid);
            }))
            .build()
            .unwrap();
        let limit = 256 * 1024;
        let server = AppServer::builder(&codex)
            .max_writer_queue_bytes(limit)
            .start()
            .await
            .unwrap();
        let handle = server.handle();
        let waiting = tokio::spawn(async move { handle.request("thread/start", json!({})).await });

        let pad = "x".repeat(4096);
        let mut failure = None;
        // Far more than a pipe and the limit hold together.
        for _ in 0..4096 {
            match server.notify("fixture/pad", json!({ "pad": pad })) {
                // Let the writer fill the pipe, so the limit is reached with
                // a write blocked rather than only by queueing.
                Ok(()) => tokio::task::yield_now().await,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let error = failure.expect("the writer limit was never reached");
        assert!(
            matches!(
                error,
                Error::AppServerLimitExceeded {
                    limit: AppServerLimit::WriterQueueBytes,
                    max
                } if max == limit
            ),
            "{error:?}"
        );
        let waited = tokio::time::timeout(Duration::from_secs(10), waiting)
            .await
            .expect("the waiting request was not settled")
            .unwrap()
            .unwrap_err();
        assert!(
            matches!(waited, Error::AppServerLimitExceeded { .. }),
            "{waited:?}"
        );
        assert!(server.notify("fixture/pad", json!({})).is_err());

        let pid = pid.lock().unwrap().expect("the server was spawned");
        assert!(wait_until_gone(pid).await, "the server ({pid}) survived");
        server.terminate().await.unwrap();
    }
    struct SocketFixture {
        path: PathBuf,
        listener: tokio::net::UnixListener,
    }
    impl SocketFixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "cw-{}-{}.sock",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            Self { path, listener }
        }
        async fn accept(&self) -> tokio_tungstenite::WebSocketStream<tokio::net::UnixStream> {
            let (stream, _) = self.listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let init: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(init["method"], "initialize");
            assert_eq!(init["params"]["capabilities"]["experimentalApi"], true);
            socket
                .send(WsMessage::Text(
                    json!({"id":init["id"],"result":{"userAgent":"fixture"}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let initialized: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(initialized["method"], "initialized");
            socket
        }
        fn builder<'a>(&self, codex: &'a Codex) -> AppServerBuilder<'a> {
            AppServer::builder(codex)
                .connect_unix(&self.path)
                .experimental_api(true)
        }
    }
    impl Drop for SocketFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[tokio::test]
    async fn attached_clients_own_no_process_and_teardown_is_connection_local() {
        let fixture = SocketFixture::new();
        let codex = Codex::builder()
            .binary("/bin/echo")
            .on_spawn(Arc::new(|_| panic!("attached mode must not spawn")))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let (client, peer) = tokio::join!(fixture.builder(&codex).start(), fixture.accept());
        let client = client.unwrap();
        let (other, mut other_peer) =
            tokio::join!(fixture.builder(&codex).start(), fixture.accept());
        let other = other.unwrap();
        assert_eq!(client.pid(), None);
        assert!(!client.transport().owns_server());
        assert!(fixture.builder(&codex).args().is_empty());
        assert!(fixture.builder(&codex).to_command_string().is_empty());
        let handle = client.handle();
        let pending = tokio::spawn(async move { handle.request("thread/list", Value::Null).await });
        let mut peer = peer;
        peer.next().await.unwrap().unwrap();
        drop(client);
        assert!(pending.await.unwrap().is_err());
        tokio::time::timeout(Duration::from_secs(2), peer.next())
            .await
            .unwrap();
        let answer = async {
            let request: Value =
                serde_json::from_str(other_peer.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            other_peer
                .send(WsMessage::Text(
                    json!({"id":request["id"],"result":{"data":[]}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(other.request("thread/list", Value::Null), answer);
        assert_eq!(result.unwrap(), json!({"data":[]}));
        other.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), other_peer.next())
            .await
            .unwrap();
        // The listening endpoint is still usable after both owners disappear.
        let (third, mut peer) = tokio::join!(fixture.builder(&codex).start(), fixture.accept());
        third.unwrap().terminate().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), peer.next())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn attached_peer_loss_fails_pending_without_reading_events() {
        let fixture = SocketFixture::new();
        let codex = offline();
        let (client, mut peer) = tokio::join!(fixture.builder(&codex).start(), fixture.accept());
        let client = client.unwrap();
        let handle = client.handle();
        let pending = tokio::spawn(async move { handle.request("thread/list", Value::Null).await });
        peer.next().await.unwrap().unwrap();
        drop(peer);
        let result = tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(Error::AppServerProtocol { .. })));
        assert!(client.notify("initialized", Value::Null).is_err());
    }

    #[tokio::test]
    async fn attached_handshake_cancellation_closes_the_socket() {
        let fixture = SocketFixture::new();
        let socket = fixture.path.clone();
        let task = tokio::spawn(async move {
            let codex = offline();
            AppServer::builder(&codex)
                .connect_unix(socket)
                .start()
                .await
        });
        let (stream, _) = fixture.listener.accept().await.unwrap();
        let mut peer = tokio_tungstenite::accept_async(stream).await.unwrap();
        peer.next().await.unwrap().unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), peer.next())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn attached_frames_queues_and_timeouts_fail_explicitly() {
        for mode in ["frame", "queue", "timeout", "binary"] {
            let fixture = SocketFixture::new();
            let codex = Codex::builder()
                .binary("/bin/echo")
                .timeout(Duration::from_millis(500))
                .build()
                .unwrap();
            let builder = fixture
                .builder(&codex)
                .max_frame_bytes(256)
                .max_queued_messages(1);
            let (client, mut peer) = tokio::join!(builder.start(), fixture.accept());
            let mut client = client.unwrap();
            let act = async {
                let request: Value =
                    serde_json::from_str(peer.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                match mode {
                    "frame" => peer
                        .send(WsMessage::Text("x".repeat(257).into()))
                        .await
                        .unwrap(),
                    "binary" => peer
                        .send(WsMessage::Binary(vec![0; 4].into()))
                        .await
                        .unwrap(),
                    "queue" => {
                        let event = WsMessage::Text(json!({"method":"thread/status/changed","params":{"threadId":"fixture","status":{"type":"idle"}}}).to_string().into());
                        peer.send(event.clone()).await.unwrap();
                        peer.send(event).await.unwrap();
                    }
                    _ => {
                        let _ = request;
                    }
                }
            };
            let (result, ()) = tokio::join!(client.request("thread/list", Value::Null), act);
            match mode {
                "frame" => assert!(matches!(
                    result,
                    Err(Error::AppServerLimitExceeded {
                        limit: AppServerLimit::FrameBytes,
                        max: 256
                    })
                )),
                "queue" => {
                    assert!(matches!(
                        result,
                        Err(Error::AppServerLimitExceeded {
                            limit: AppServerLimit::QueuedMessages,
                            max: 1
                        })
                    ));
                    assert!(client.next_message().await.unwrap().is_some());
                    assert!(matches!(
                        client.next_message().await,
                        Err(Error::AppServerLimitExceeded { .. })
                    ));
                }
                "timeout" => assert!(matches!(result, Err(Error::Timeout { .. }))),
                _ => assert!(matches!(result, Err(Error::AppServerProtocol { .. }))),
            }
            client.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn attached_response_routing_and_ping_progress_without_consuming_events() {
        let fixture = SocketFixture::new();
        let codex = offline();
        let (client, mut peer) = tokio::join!(
            fixture.builder(&codex).max_queued_messages(1).start(),
            fixture.accept()
        );
        let mut client = client.unwrap();
        let act = async {
            let request: Value =
                serde_json::from_str(peer.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            peer.send(WsMessage::Text(
                json!({"method":"thread/status/changed","params":{}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            peer.send(WsMessage::Ping(b"fixture".to_vec().into()))
                .await
                .unwrap();
            let pong = tokio::time::timeout(Duration::from_secs(2), peer.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(matches!(pong, WsMessage::Pong(_)));
            peer.send(WsMessage::Text(
                json!({"id":request["id"],"result":{"data":[]}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        };
        let (result, ()) = tokio::join!(client.request("thread/list", Value::Null), act);
        assert!(result.is_ok());
        assert!(client.next_message().await.unwrap().is_some());
        client.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn closure_wakes_idle_and_blocked_writers_with_surviving_handles() {
        for blocked in [false, true] {
            let (stdin, _peer) = tokio::io::duplex(1);
            let (events, _received) = mpsc::unbounded_channel();
            let shared = Arc::new(Shared::new(events, None, Limits::default(), "test"));
            let (writer, receiver) = mpsc::unbounded_channel();
            let handle = AppServerHandle {
                shared: Arc::clone(&shared),
                writer,
                timeout: None,
            };
            let task = tokio::spawn(write_stdin(stdin, receiver, Arc::clone(&shared)));
            if blocked {
                handle.notify("initialized", Value::Null).unwrap();
            }
            tokio::task::yield_now().await;
            shared.fail(Closed::Limit {
                limit: AppServerLimit::QueuedMessages,
                max: 0,
            });
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(*locked(&shared.outbound.used), (0, 0));
            assert!(handle.notify("initialized", Value::Null).is_err());
        }
    }

    #[tokio::test]
    async fn queued_bytes_use_actual_crlf_and_unterminated_wire_lengths() {
        let line = r#"{"method":"thread/status/changed","params":{}}"#;
        for delimiter in ["\r\n", "\n", ""] {
            let input = format!("{line}{delimiter}");
            let wire_bytes = input.len();
            let mut reader = BufReader::new(input.as_bytes());
            let (text, bytes) = read_frame(&mut reader, &mut 0, None, None, &None)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(text, line);
            assert_eq!(bytes, wire_bytes);
            let (events, _received) = mpsc::unbounded_channel();
            let shared = Shared::new(
                events,
                None,
                Limits {
                    queued_bytes: Some(wire_bytes),
                    ..Limits::default()
                },
                "test",
            );
            route_bytes(&text, bytes, &shared).unwrap();
            assert_eq!(*locked(&shared.inbound.used), (1, wire_bytes));
            assert!(route_bytes(&text, bytes, &shared).is_err());
        }
    }
    #[tokio::test]
    async fn attached_fragmented_messages_cannot_bypass_frame_budget() {
        use tokio_tungstenite::tungstenite::protocol::frame::{
            Frame,
            coding::{Data, OpCode},
        };
        let fixture = SocketFixture::new();
        let codex = offline();
        let (client, mut peer) = tokio::join!(
            fixture.builder(&codex).max_frame_bytes(256).start(),
            fixture.accept()
        );
        let client = client.unwrap();
        let send = async {
            peer.next().await.unwrap().unwrap();
            for (opcode, final_) in [
                (Data::Text, false),
                (Data::Continue, false),
                (Data::Continue, true),
            ] {
                peer.send(WsMessage::Frame(Frame::message(
                    vec![b'x'; 100],
                    OpCode::Data(opcode),
                    final_,
                )))
                .await
                .unwrap();
            }
        };
        let (result, ()) = tokio::join!(client.request("thread/list", Value::Null), send);
        assert!(matches!(
            result,
            Err(Error::AppServerLimitExceeded {
                limit: AppServerLimit::FrameBytes,
                max: 256
            })
        ));
    }

    #[tokio::test]
    async fn attached_blocked_writer_overflow_settles_and_releases_reservations() {
        let fixture = SocketFixture::new();
        let codex = offline();
        let (client, _peer) = tokio::join!(
            fixture.builder(&codex).max_writer_queue_messages(1).start(),
            fixture.accept()
        );
        let client = client.unwrap();
        let handle = client.handle();
        let pending = tokio::spawn(async move {
            handle
                .request(
                    "thread/start",
                    json!({"fixture": "x".repeat(4 * 1024 * 1024)}),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            while locked(&client.handle.shared.outbound.used).0 == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            client.notify("initialized", Value::Null),
            Err(Error::AppServerLimitExceeded {
                limit: AppServerLimit::WriterQueueMessages,
                max: 1
            })
        ));
        assert!(matches!(
            pending.await.unwrap(),
            Err(Error::AppServerLimitExceeded { .. })
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while *locked(&client.handle.shared.outbound.used) != (0, 0) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        client.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn review_regression_shutdown_notifications_preserve_accepted_writes() {
        let (stdin, mut peer) = tokio::io::duplex(4);
        let (events, _received) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared::new(events, None, Limits::default(), "test"));
        let (writer, receiver) = mpsc::unbounded_channel();
        let handle = AppServerHandle {
            shared: Arc::clone(&shared),
            writer,
            timeout: None,
        };
        let expected = notification_line("initialized", &Value::Null).unwrap();
        handle.notify("initialized", Value::Null).unwrap();
        let task = tokio::spawn(write_stdin(stdin, receiver, Arc::clone(&shared)));
        tokio::task::yield_now().await;
        shared.end(Closed::Terminated, false);
        if let Err(reason) = route(r#"{"method":"thread/status/changed","params":{}}"#, &shared) {
            shared.fatal(reason);
        }
        handle.writer.send(Outgoing::Close).unwrap();
        let mut output = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), peer.read_to_end(&mut output))
            .await
            .unwrap()
            .unwrap();
        task.await.unwrap();
        assert_eq!(output, expected, "shutdown truncated an accepted message");
    }

    #[tokio::test]
    async fn review_regression_peer_close_is_acknowledged() {
        let fixture = SocketFixture::new();
        let codex = offline();
        let (client, mut peer) = tokio::join!(fixture.builder(&codex).start(), fixture.accept());
        let client = client.unwrap();
        peer.close(None).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(2), peer.next())
            .await
            .unwrap();
        assert!(
            matches!(reply, Some(Ok(WsMessage::Close(_)))),
            "missing close reply: {reply:?}"
        );
        assert!(client.request("thread/list", Value::Null).await.is_err());
    }
}

#[cfg(all(test, windows))]
#[tokio::test]
async fn a_fatal_limit_terminates_the_windows_child_without_an_event_consumer() {
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let (events, _unread) = mpsc::unbounded_channel();
    let shared = Shared::new(
        events,
        Some(KillTarget {
            pid: child.id(),
            group: false,
            handle: child.raw_handle().map(|handle| handle as usize),
        }),
        Limits::default(),
        "test",
    );
    shared.fatal(Closed::Limit {
        limit: AppServerLimit::QueuedMessages,
        max: 0,
    });
    shared.revoke_kill();
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
    assert!(matches!(shared.closed_reason(), Some(Closed::Limit { .. })));
}
