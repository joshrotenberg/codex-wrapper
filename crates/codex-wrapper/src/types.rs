//! Domain types shared across commands: enums for CLI options, version parsing,
//! and structured JSONL events.
//!
//! # JSONL schema: what is verified and what is assumed
//!
//! This parser was rewritten in #73 after the previous one was found to match
//! an event shape the CLI never emits. It matched a fixture that invented the
//! schema, so every test passed while real output produced empty results. The
//! split below exists so the next person can check the assumptions instead of
//! rediscovering the bug.
//!
//! **Verified** against `codex-cli` 0.145.0, from the compiled serde tag list
//! and live runs of both `codex exec --json` and `codex exec review --json`:
//!
//! - The event vocabulary is `thread.started`, `turn.started`, `turn.completed`,
//!   `turn.failed`, `item.started`, `item.updated`, `item.completed`. There is
//!   no bare `completed`. Review emits the same vocabulary as exec.
//! - `thread.started` carries `thread_id`.
//! - A completed turn reports token counts, never a monetary cost. The `usage`
//!   object carries `input_tokens`, `cached_input_tokens`,
//!   `cache_write_input_tokens`, `output_tokens`, and
//!   `reasoning_output_tokens`. It does **not** carry `total_tokens`, so
//!   [`TokenUsage::total`] reaches a total through its input plus output
//!   fallback on every real run.
//! - Assistant text arrives as an `item.completed` event whose `item` has a
//!   `type` of `agent_message` and its text in a `text` field. A review's
//!   diff-reading steps arrive as `command_execution` items on the same event.
//! - A review's `turn.completed` reports a usage object of all zeros.
//! - Exhausting the native rollout budget emits an `error` event followed by
//!   `turn.failed`, both with `shared rollout token budget exhausted`. The
//!   terminal event carries no usage object on 0.145.0 even though Codex used
//!   its internal meter to make the decision.
//! - An upstream API rejection emits an `error` event followed by
//!   `turn.failed`, and the CLI exits 1. Verified on 0.149.0 with two
//!   distinct output-schema rejections. Both terminal events carry the
//!   upstream error as a JSON **document nested inside the `error.message`
//!   string**, not as sibling fields: `{"type":"error","error":{"type":
//!   "invalid_request_error","code":"invalid_json_schema","message":...,
//!   "param":"text.format.schema"},"status":400}`. The human message differs
//!   between the two runs; `type`, `code`, and `status` do not. This is why
//!   [`JsonLineEvent::turn_failure_api_error`] parses the message rather than
//!   reading a sibling code, and it corrects the earlier note that Codex
//!   emits no machine-readable error code.
//!
//! - The stream carries **no incremental text**. Three captured runs, a
//!   one-word exec, a four-sentence exec, and a review, each delivered the
//!   whole assistant message in a single `item.completed`. No `item.updated`,
//!   no partial or delta fields, and `codex exec --help` has no flag that
//!   changes output granularity. There is nothing to assemble, which is why
//!   this module has no equivalent of `claude-wrapper`'s `PartialMessageEvent`
//!   (#84).
//!
//! **Assumed**, still: nothing load-bearing.
//! [`JsonLineEvent::agent_message_text`] also accepts an `item_type`
//! discriminator and a `content` block array, neither of which has been seen
//! in real output. That tolerance stays because the failure mode when this
//! parser guesses wrong is an empty result rather than an error, which is how
//! #73 went unnoticed.
//!
//! To re-confirm after a CLI upgrade, capture a run and check `result` is
//! non-empty:
//!
//! ```sh
//! codex exec --json --ephemeral --skip-git-repo-check "reply with: ok" > turn.jsonl
//! ```

#[cfg(feature = "json")]
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Sandbox policy for model-generated shell commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    /// Read-only filesystem access.
    ReadOnly,
    /// Write access limited to the workspace directory (default).
    #[default]
    WorkspaceWrite,
    /// Full filesystem access -- use with extreme caution.
    DangerFullAccess,
}

impl SandboxMode {
    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

/// When the model should ask for human approval before executing commands.
///
/// These are the values accepted by the `--ask-for-approval` flag on
/// [`ForkCommand`](crate::ForkCommand) and [`ResumeCommand`](crate::ResumeCommand).
/// The exec family sets the same setting through the `approval_policy` config
/// key, which accepts a larger value set -- see [`ApprovalPolicyConfig`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicy {
    /// Only run trusted commands without asking.
    Untrusted,
    /// The model decides when to ask (default).
    #[default]
    OnRequest,
    /// Never ask for approval.
    Never,
}

impl ApprovalPolicy {
    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

/// Approval policy values accepted by the `approval_policy` config key.
///
/// `codex-cli` 0.145.0 removed `--ask-for-approval` from the exec family; the
/// config key is the supported equivalent. It accepts two values the flag does
/// not ([`OnFailure`](Self::OnFailure) and [`Granular`](Self::Granular)), which
/// is why this is a separate type from [`ApprovalPolicy`] rather than an alias.
/// The three shared values convert implicitly:
///
/// ```
/// use codex_wrapper::{ApprovalPolicy, ApprovalPolicyConfig};
///
/// let config: ApprovalPolicyConfig = ApprovalPolicy::Never.into();
/// assert_eq!(config, ApprovalPolicyConfig::Never);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicyConfig {
    /// Only run trusted commands without asking.
    Untrusted,
    /// Ask after a command fails.
    ///
    /// Not accepted by `--ask-for-approval`, so it has no [`ApprovalPolicy`]
    /// counterpart.
    OnFailure,
    /// The model decides when to ask (default).
    #[default]
    OnRequest,
    /// Ask per-operation rather than per-command.
    ///
    /// Not accepted by `--ask-for-approval`, so it has no [`ApprovalPolicy`]
    /// counterpart.
    Granular,
    /// Never ask for approval.
    Never,
}

impl ApprovalPolicyConfig {
    pub(crate) fn as_config_value(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnFailure => "on-failure",
            Self::OnRequest => "on-request",
            Self::Granular => "granular",
            Self::Never => "never",
        }
    }
}

impl From<ApprovalPolicy> for ApprovalPolicyConfig {
    fn from(policy: ApprovalPolicy) -> Self {
        match policy {
            ApprovalPolicy::Untrusted => Self::Untrusted,
            ApprovalPolicy::OnRequest => Self::OnRequest,
            ApprovalPolicy::Never => Self::Never,
        }
    }
}

impl TryFrom<ApprovalPolicyConfig> for ApprovalPolicy {
    type Error = ApprovalPolicyConfig;

    /// Narrow to the flag-accepted subset.
    ///
    /// Returns the original value as the error for
    /// [`OnFailure`](ApprovalPolicyConfig::OnFailure) and
    /// [`Granular`](ApprovalPolicyConfig::Granular), which `--ask-for-approval`
    /// rejects.
    fn try_from(config: ApprovalPolicyConfig) -> std::result::Result<Self, Self::Error> {
        match config {
            ApprovalPolicyConfig::Untrusted => Ok(Self::Untrusted),
            ApprovalPolicyConfig::OnRequest => Ok(Self::OnRequest),
            ApprovalPolicyConfig::Never => Ok(Self::Never),
            other => Err(other),
        }
    }
}

/// Web search mode, set through the `web_search` config key.
///
/// `codex-cli` 0.145.0 removed `--search` from the exec family. The config key
/// replacing it is an enum rather than the flag's boolean;
/// [`Live`](Self::Live) is what `--search` meant.
///
/// `--search` is still a valid flag on [`ForkCommand`](crate::ForkCommand) and
/// [`ResumeCommand`](crate::ResumeCommand), which keep their boolean setters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebSearchMode {
    /// No web search.
    #[default]
    Disabled,
    /// Serve results from cache only.
    Cached,
    /// Search a prebuilt index.
    Indexed,
    /// Live web search. The `--search` flag's former behavior.
    Live,
}

impl WebSearchMode {
    pub(crate) fn as_config_value(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Cached => "cached",
            Self::Indexed => "indexed",
            Self::Live => "live",
        }
    }
}

/// Color output mode for exec commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Color {
    /// Always emit color codes.
    Always,
    /// Never emit color codes.
    Never,
    /// Auto-detect terminal support (default).
    #[default]
    Auto,
}

impl Color {
    pub(crate) fn as_arg(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Never => "never",
            Self::Auto => "auto",
        }
    }
}

/// A single parsed JSONL event from `--json` output.
///
/// The `event_type` field corresponds to the `"type"` key in the JSON.
/// All other fields are captured in `extra`.
#[cfg(feature = "json")]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonLineEvent {
    #[serde(rename = "type", default)]
    pub event_type: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// A terminal `turn.failed` classification from the Codex JSONL stream.
///
/// Two classes are typed. The budget class is pinned to the stable message
/// captured from 0.145.0. The API-rejection class reads the machine-readable
/// code the upstream API supplies, captured from 0.149.0; see
/// [`ApiFailure`]. Unknown failures remain distinguishable rather than being
/// forced into either class.
#[cfg(feature = "json")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TurnFailureKind {
    /// Codex stopped after exhausting its native shared rollout budget.
    RolloutBudgetExhausted,
    /// The upstream API rejected the request before generation began.
    ///
    /// The turn produced no output and had no side effects, so a caller may
    /// correct the request and try again. [`JsonLineEvent::turn_failure_api_error`]
    /// carries the code that distinguishes which part of the request was
    /// rejected.
    ApiRequestRejected,
    /// A terminal failure not classified by this wrapper.
    Other,
}

/// A structured API error carried by a terminal `turn.failed` event.
///
/// Codex nests the upstream API error as a JSON document inside the
/// `error.message` string rather than as sibling fields, so reaching the code
/// means parsing that string. Captured from `codex-cli` 0.149.0:
///
/// ```text
/// {"type":"turn.failed","error":{"message":"{\n  \"type\": \"error\",
///   \n  \"error\": {\n    \"type\": \"invalid_request_error\",
///   \n    \"code\": \"invalid_json_schema\", ... },\n  \"status\": 400\n}"}}
/// ```
#[cfg(feature = "json")]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ApiFailure {
    /// The API error class, such as `invalid_request_error`.
    pub error_type: Option<String>,
    /// The machine-readable code, such as `invalid_json_schema`.
    pub code: Option<String>,
    /// The HTTP status the API returned, such as 400.
    pub status: Option<u64>,
}

#[cfg(feature = "json")]
impl ApiFailure {
    /// Whether the API rejected the request itself, before generation.
    ///
    /// A 4xx status other than 429 means the request was refused rather than
    /// throttled or failed midway.
    #[must_use]
    pub fn rejected_request(&self) -> bool {
        if self.error_type.as_deref() == Some("invalid_request_error") {
            return true;
        }
        self.status
            .is_some_and(|status| (400..500).contains(&status) && status != 429)
    }
}

#[cfg(feature = "json")]
impl JsonLineEvent {
    /// Returns the `session_id` field, if present and a string.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.extra.get("session_id").and_then(|v| v.as_str())
    }

    /// Returns the `thread_id` field, if present and a string.
    #[must_use]
    pub fn thread_id(&self) -> Option<&str> {
        self.extra.get("thread_id").and_then(|v| v.as_str())
    }

    /// Returns `true` when this is the terminal event of a successful turn.
    #[must_use]
    pub fn is_turn_completed(&self) -> bool {
        self.event_type == "turn.completed"
    }

    /// Returns `true` when this is the terminal event of a failed turn.
    #[must_use]
    pub fn is_turn_failed(&self) -> bool {
        self.event_type == "turn.failed"
    }

    /// The message carried by a terminal `turn.failed` event.
    ///
    /// The captured CLI shape nests it under `error.message`.
    #[must_use]
    pub fn turn_failure_message(&self) -> Option<&str> {
        if !self.is_turn_failed() {
            return None;
        }
        self.extra
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(serde_json::Value::as_str)
    }

    /// The structured API error carried by a terminal `turn.failed` event.
    ///
    /// Returns `None` on any other event, and when the failure message is not
    /// the JSON document the API produces. A local failure such as an
    /// exhausted rollout budget carries plain prose and yields `None` here.
    #[must_use]
    pub fn turn_failure_api_error(&self) -> Option<ApiFailure> {
        let message = self.turn_failure_message()?;
        let document: serde_json::Value = serde_json::from_str(message).ok()?;
        let error = document.get("error")?;
        let failure = ApiFailure {
            error_type: error
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            code: error
                .get("code")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            status: document.get("status").and_then(serde_json::Value::as_u64),
        };
        (failure != ApiFailure::default()).then_some(failure)
    }

    /// Classify a terminal `turn.failed` event without downstream string matching.
    #[must_use]
    pub fn turn_failure_kind(&self) -> Option<TurnFailureKind> {
        self.is_turn_failed().then(|| {
            if self
                .turn_failure_message()
                .is_some_and(|message| message.contains("shared rollout token budget exhausted"))
            {
                TurnFailureKind::RolloutBudgetExhausted
            } else if self
                .turn_failure_api_error()
                .is_some_and(|error| error.rejected_request())
            {
                TurnFailureKind::ApiRequestRejected
            } else {
                TurnFailureKind::Other
            }
        })
    }

    /// Token counts reported by a `turn.completed` event.
    ///
    /// Returns `None` on any other event, or when no `usage` object is
    /// present. The CLI reports tokens, not money; see [`TokenUsage`].
    #[must_use]
    pub fn usage(&self) -> Option<TokenUsage> {
        self.extra.get("usage").map(TokenUsage::from_json)
    }

    /// Assistant text carried by an `item.completed` agent-message item.
    ///
    /// Returns `None` for any other event or item type.
    ///
    /// Real output uses a `type` discriminator and a `text` field; see the
    /// schema block at the top of this module. An `item_type` discriminator
    /// and a `content` block array are also accepted, neither of them
    /// observed. Tolerating the unobserved shapes is deliberate: the previous
    /// parser committed to one exact layout and silently yielded empty results
    /// when it was wrong, which is the bug this replaces.
    #[must_use]
    pub fn agent_message_text(&self) -> Option<String> {
        if self.event_type != "item.completed" {
            return None;
        }
        let item = self.extra.get("item")?;
        let kind = item
            .get("item_type")
            .or_else(|| item.get("type"))
            .and_then(|v| v.as_str())?;
        if kind != "agent_message" {
            return None;
        }

        if let Some(text) = item.get("text").and_then(|v| v.as_str())
            && !text.is_empty()
        {
            return Some(text.to_string());
        }

        let blocks = item.get("content").and_then(|v| v.as_array())?;
        let text: String = blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");
        if text.is_empty() { None } else { Some(text) }
    }

    /// Returns the `role` field, if present and a string.
    #[must_use]
    pub fn role(&self) -> Option<&str> {
        self.extra.get("role").and_then(|v| v.as_str())
    }

    /// Extracts concatenated text from a `content` blocks array.
    ///
    /// Each block with `"type": "text"` contributes its `"text"` value.
    /// Returns `None` if there is no `content` array or no text blocks.
    #[must_use]
    pub fn content_text(&self) -> Option<String> {
        let blocks = self.extra.get("content").and_then(|v| v.as_array())?;
        let text: String = blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");
        if text.is_empty() { None } else { Some(text) }
    }

    /// The `item` discriminator on an `item.*` event.
    ///
    /// `agent_message` and `command_execution` are the observed values. This
    /// is the field to match on when narrowing an item event, and it is
    /// present on `item.started` as well as `item.completed`.
    ///
    /// Accepts `item_type` as well as `type` for the same reason
    /// [`agent_message_text`](Self::agent_message_text) does.
    #[must_use]
    pub fn item_type(&self) -> Option<&str> {
        let item = self.extra.get("item")?;
        item.get("type").or_else(|| item.get("item_type"))?.as_str()
    }

    /// A shell command the model ran, from a `command_execution` item.
    ///
    /// `None` for any other item. `exit_code` and `status` are only populated
    /// on `item.completed`; an `item.started` for the same command carries the
    /// command alone.
    #[must_use]
    pub fn command_execution(&self) -> Option<CommandExecution> {
        if self.item_type()? != "command_execution" {
            return None;
        }
        let item = self.extra.get("item")?;
        let string = |key: &str| item.get(key).and_then(|v| v.as_str()).map(str::to_string);
        Some(CommandExecution {
            command: string("command"),
            status: string("status"),
            exit_code: item
                .get("exit_code")
                .and_then(serde_json::Value::as_i64)
                .and_then(|code| i32::try_from(code).ok()),
            aggregated_output: string("aggregated_output"),
        })
    }
}

/// A typed summary of a completed `codex exec` run, assembled from the JSONL
/// event stream.
///
/// This mirrors the shape of `claude-wrapper`'s `QueryResult` so a downstream
/// abstraction can treat both wrappers uniformly. The full parsed event stream
/// is retained in [`events`](QueryResult::events) as an escape hatch for fields
/// not surfaced here.
#[cfg(feature = "json")]
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Assistant text, concatenated from the turn's agent-message items.
    ///
    /// Empty when the turn produced no agent message. A failed turn can still
    /// carry text produced before its terminal failure; native rollout-budget
    /// exhaustion is one observed example.
    pub result: String,
    /// The `session_id` captured from the event stream, if any.
    ///
    /// Observed live runs carry only `thread_id`, so this is usually `None`.
    /// Retained because it costs nothing and the field may exist under auth
    /// modes not yet observed.
    pub session_id: Option<String>,
    /// The `thread_id` captured from the event stream, if any.
    ///
    /// Codex's native identifier for resuming a conversation, emitted on
    /// `thread.started`.
    pub thread_id: Option<String>,
    /// Token counts from the terminal `turn.completed` event, if present.
    ///
    /// The CLI reports tokens, not money. There is no cost field to read; see
    /// [`TokenUsage`].
    pub usage: Option<TokenUsage>,
    /// The full parsed event stream this result was assembled from.
    ///
    /// The escape hatch for anything not surfaced above, including whatever
    /// this parser gets wrong.
    pub events: Vec<JsonLineEvent>,
}

#[cfg(feature = "json")]
impl QueryResult {
    /// Assemble a [`QueryResult`] from a parsed JSONL event stream.
    ///
    /// `usage` comes from the last `turn.completed` event; `result` is every
    /// agent-message item concatenated in order; `session_id` and `thread_id`
    /// are the first occurrences in the stream.
    #[must_use]
    pub fn from_events(events: Vec<JsonLineEvent>) -> Self {
        let usage = events
            .iter()
            .rev()
            .find(|e| e.is_turn_completed())
            .and_then(JsonLineEvent::usage);
        let result = events
            .iter()
            .filter_map(JsonLineEvent::agent_message_text)
            .collect::<Vec<_>>()
            .join("");
        let session_id = events
            .iter()
            .find_map(JsonLineEvent::session_id)
            .map(str::to_string);
        let thread_id = events
            .iter()
            .find_map(JsonLineEvent::thread_id)
            .map(str::to_string);
        Self {
            result,
            session_id,
            thread_id,
            usage,
            events,
        }
    }
}

/// A shell command the model ran, from a `command_execution` item.
///
/// Every field is optional: an `item.started` carries the command with no
/// outcome yet, and the CLI has added fields to this item before.
#[cfg(feature = "json")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommandExecution {
    /// The command line, as the CLI recorded it.
    pub command: Option<String>,
    /// Reported status, `completed` being the observed value.
    pub status: Option<String>,
    /// Exit code, once the command has finished.
    pub exit_code: Option<i32>,
    /// Combined stdout and stderr, when the CLI included it.
    pub aggregated_output: Option<String>,
}

/// Token counts reported on a completed turn.
///
/// `codex-cli` 0.145.0 reports token usage and no monetary cost. Converting
/// tokens to dollars needs a per-model price table the CLI does not provide,
/// so this crate does not attempt it: a hardcoded table would go stale
/// silently, which is the same class of bug as #73.
///
/// Every field is optional, and absence is normal rather than exceptional:
/// observed runs carry five of the six and never `total_tokens`, which is why
/// [`TokenUsage::total`] falls back to input plus output.
#[cfg(feature = "json")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input tokens for the turn.
    pub input_tokens: Option<u64>,
    /// Input tokens served from cache.
    pub cached_input_tokens: Option<u64>,
    /// Input tokens written to cache.
    pub cache_write_input_tokens: Option<u64>,
    /// Output tokens for the turn.
    pub output_tokens: Option<u64>,
    /// Output tokens spent on reasoning.
    pub reasoning_output_tokens: Option<u64>,
    /// Total tokens, if the CLI ever reports one. Observed runs do not.
    pub total_tokens: Option<u64>,
}

#[cfg(feature = "json")]
impl TokenUsage {
    fn from_json(value: &serde_json::Value) -> Self {
        let field = |name: &str| value.get(name).and_then(serde_json::Value::as_u64);
        Self {
            input_tokens: field("input_tokens"),
            cached_input_tokens: field("cached_input_tokens"),
            cache_write_input_tokens: field("cache_write_input_tokens"),
            output_tokens: field("output_tokens"),
            reasoning_output_tokens: field("reasoning_output_tokens"),
            total_tokens: field("total_tokens"),
        }
    }

    /// Best available total for the turn.
    ///
    /// Prefers the CLI's own `total_tokens`, falls back to input plus output
    /// when it is absent, and returns `None` when neither is reported, so a
    /// missing total is never silently counted as zero.
    #[must_use]
    pub fn total(&self) -> Option<u64> {
        if let Some(total) = self.total_tokens {
            return Some(total);
        }
        match (self.input_tokens, self.output_tokens) {
            (None, None) => None,
            (input, output) => Some(input.unwrap_or(0) + output.unwrap_or(0)),
        }
    }
}

/// Parsed semantic version of the Codex CLI (`major.minor.patch`).
///
/// Supports comparison and ordering for version-gating logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl CliVersion {
    #[must_use]
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn parse_version_output(output: &str) -> Result<Self, VersionParseError> {
        output
            .split_whitespace()
            .find_map(|token| token.parse().ok())
            .ok_or_else(|| VersionParseError(output.trim().to_string()))
    }

    #[must_use]
    pub fn satisfies_minimum(&self, minimum: &CliVersion) -> bool {
        self >= minimum
    }

    /// Classify this version against a tested-against range.
    ///
    /// ```
    /// use codex_wrapper::{CliVersion, CliVersionStatus};
    ///
    /// let min = CliVersion::new(0, 145, 0);
    /// let max = CliVersion::new(0, 146, 0);
    ///
    /// assert!(CliVersion::new(0, 145, 3).status_within(&min, &max).is_tested());
    /// assert!(!CliVersion::new(0, 200, 0).status_within(&min, &max).is_tested());
    /// ```
    #[must_use]
    pub fn status_within(&self, min: &CliVersion, max: &CliVersion) -> CliVersionStatus {
        if self < min {
            CliVersionStatus::OlderThanMinimum {
                found: *self,
                minimum: *min,
            }
        } else if self > max {
            CliVersionStatus::NewerUntested {
                found: *self,
                tested_max: *max,
            }
        } else {
            CliVersionStatus::Tested
        }
    }
}

/// Classification of an installed CLI version against a tested range.
///
/// Returned by [`CliVersion::status_within`] and
/// [`Codex::cli_version_status`](crate::Codex::cli_version_status). Mirrors
/// `claude-wrapper`'s enum of the same name so a downstream abstraction can
/// treat both wrappers uniformly.
///
/// There is deliberately no `Unparseable` variant: unparseable output is an
/// error from [`Codex::cli_version`](crate::Codex::cli_version), not a status,
/// and modeling it twice would fork this shape away from the sibling crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CliVersionStatus {
    /// Within the tested-against range.
    Tested,
    /// Newer than the highest tested version.
    ///
    /// The wrapper should still generally work; semantics may have drifted.
    NewerUntested {
        /// The installed CLI version.
        found: CliVersion,
        /// Highest version this wrapper is tested against.
        tested_max: CliVersion,
    },
    /// Older than the lowest tested version.
    ///
    /// Incorrect behavior is likely rather than merely possible: the arguments
    /// this wrapper emits target the newer CLI, and older releases reject some
    /// of them outright.
    OlderThanMinimum {
        /// The installed CLI version.
        found: CliVersion,
        /// Lowest version this wrapper is tested against.
        minimum: CliVersion,
    },
}

impl CliVersionStatus {
    /// True only for [`Tested`](Self::Tested).
    ///
    /// For callers branching on "should I run?" without matching every
    /// variant.
    #[must_use]
    pub fn is_tested(self) -> bool {
        matches!(self, Self::Tested)
    }
}

impl PartialOrd for CliVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CliVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
    }
}

impl fmt::Display for CliVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for CliVersion {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(VersionParseError(s.to_string()));
        }

        Ok(Self {
            major: parts[0]
                .parse()
                .map_err(|_| VersionParseError(s.to_string()))?,
            minor: parts[1]
                .parse()
                .map_err(|_| VersionParseError(s.to_string()))?,
            patch: parts[2]
                .parse()
                .map_err(|_| VersionParseError(s.to_string()))?,
        })
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid version string: {0:?}")]
pub struct VersionParseError(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_version_output() {
        let version = CliVersion::parse_version_output("codex-cli 0.145.0").unwrap();
        assert_eq!(version, CliVersion::new(0, 145, 0));
    }

    #[test]
    fn parses_plain_version_output() {
        let version = CliVersion::parse_version_output("0.145.0").unwrap();
        assert_eq!(version, CliVersion::new(0, 145, 0));
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_session_and_thread_id() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"message.created","session_id":"sess_abc","thread_id":"thread_123"}"#,
        )
        .unwrap();
        assert_eq!(event.session_id(), Some("sess_abc"));
        assert_eq!(event.thread_id(), Some("thread_123"));
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_turn_terminal_types() {
        let completed: JsonLineEvent =
            serde_json::from_str(r#"{"type":"turn.completed"}"#).unwrap();
        assert!(completed.is_turn_completed());
        assert!(!completed.is_turn_failed());

        let failed: JsonLineEvent = serde_json::from_str(r#"{"type":"turn.failed"}"#).unwrap();
        assert!(failed.is_turn_failed());
        assert!(!failed.is_turn_completed());

        // The pre-#73 parser matched this. The CLI has never emitted it.
        let bogus: JsonLineEvent = serde_json::from_str(r#"{"type":"completed"}"#).unwrap();
        assert!(!bogus.is_turn_completed());
    }

    /// Transcribed from a paid `codex-cli` 0.145.0 run with a one-unit native
    /// rollout budget. The CLI exposes the limit decision but no usage object.
    #[cfg(feature = "json")]
    #[test]
    fn classifies_captured_rollout_budget_terminal_without_inventing_usage() {
        let failed: JsonLineEvent = serde_json::from_str(
            r#"{"type":"turn.failed","error":{"message":"shared rollout token budget exhausted"}}"#,
        )
        .unwrap();

        assert_eq!(
            failed.turn_failure_kind(),
            Some(TurnFailureKind::RolloutBudgetExhausted)
        );
        assert_eq!(failed.usage(), None);

        let other: JsonLineEvent = serde_json::from_str(
            r#"{"type":"turn.failed","error":{"message":"tool policy rejected"}}"#,
        )
        .unwrap();
        assert_eq!(other.turn_failure_kind(), Some(TurnFailureKind::Other));

        let missing_message: JsonLineEvent =
            serde_json::from_str(r#"{"type":"turn.failed"}"#).unwrap();
        assert_eq!(
            missing_message.turn_failure_kind(),
            Some(TurnFailureKind::Other)
        );

        let nonterminal: JsonLineEvent =
            serde_json::from_str(r#"{"type":"turn.started"}"#).unwrap();
        assert_eq!(nonterminal.turn_failure_kind(), None);
    }

    /// Both lines are transcriptions of captured `codex-cli` 0.149.0 runs:
    /// an output schema whose property omits `type`, and a root-level
    /// `anyOf`. The human message differs between them; the code does not.
    #[cfg(feature = "json")]
    #[test]
    fn classifies_an_api_request_rejection() {
        let missing_type: JsonLineEvent = serde_json::from_str(
            r#"{"type":"turn.failed","error":{"message":"{\n  \"type\": \"error\",\n  \"error\": {\n    \"type\": \"invalid_request_error\",\n    \"code\": \"invalid_json_schema\",\n    \"message\": \"Invalid schema for response_format 'codex_output_schema': In context=('properties', 'ok'), schema must have a 'type' key.\",\n    \"param\": \"text.format.schema\"\n  },\n  \"status\": 400\n}"}}"#,
        )
        .unwrap();
        assert_eq!(
            missing_type.turn_failure_kind(),
            Some(TurnFailureKind::ApiRequestRejected)
        );
        let error = missing_type.turn_failure_api_error().unwrap();
        assert_eq!(error.code.as_deref(), Some("invalid_json_schema"));
        assert_eq!(error.error_type.as_deref(), Some("invalid_request_error"));
        assert_eq!(error.status, Some(400));
        assert!(error.rejected_request());

        let root_any_of: JsonLineEvent = serde_json::from_str(
            r#"{"type":"turn.failed","error":{"message":"{\n  \"type\": \"error\",\n  \"error\": {\n    \"type\": \"invalid_request_error\",\n    \"code\": \"invalid_json_schema\",\n    \"message\": \"Invalid schema for response_format 'codex_output_schema': schema must be a JSON Schema of 'type: \\\"object\\\"', got 'type: \\\"None\\\"'.\",\n    \"param\": \"text.format.schema\"\n  },\n  \"status\": 400\n}"}}"#,
        )
        .unwrap();
        assert_eq!(
            root_any_of.turn_failure_kind(),
            Some(TurnFailureKind::ApiRequestRejected)
        );
        assert_eq!(
            root_any_of
                .turn_failure_api_error()
                .and_then(|error| error.code),
            Some("invalid_json_schema".to_string())
        );
    }

    /// A local failure carries prose, not the API's JSON document, and must
    /// not be mistaken for a request rejection.
    #[cfg(feature = "json")]
    #[test]
    fn a_prose_failure_message_carries_no_api_error() {
        let budget: JsonLineEvent = serde_json::from_str(
            r#"{"type":"turn.failed","error":{"message":"shared rollout token budget exhausted"}}"#,
        )
        .unwrap();
        assert_eq!(budget.turn_failure_api_error(), None);
        assert_eq!(
            budget.turn_failure_kind(),
            Some(TurnFailureKind::RolloutBudgetExhausted)
        );

        let other: JsonLineEvent = serde_json::from_str(
            r#"{"type":"turn.failed","error":{"message":"tool policy rejected"}}"#,
        )
        .unwrap();
        assert_eq!(other.turn_failure_api_error(), None);
        assert_eq!(other.turn_failure_kind(), Some(TurnFailureKind::Other));
    }

    /// A server-side or throttling failure is not a rejected request: the
    /// turn may have done work, so a caller must not treat it as safe to
    /// replay after editing the request.
    #[cfg(feature = "json")]
    #[test]
    fn server_and_throttling_failures_are_not_request_rejections() {
        for (status, error_type) in [(500u64, "server_error"), (429, "rate_limit_error")] {
            let message = format!(
                r#"{{"type":"error","error":{{"type":"{error_type}","code":"x"}},"status":{status}}}"#
            );
            let event = JsonLineEvent {
                event_type: "turn.failed".to_string(),
                extra: HashMap::from([(
                    "error".to_string(),
                    serde_json::json!({ "message": message }),
                )]),
            };
            assert!(!event.turn_failure_api_error().unwrap().rejected_request());
            assert_eq!(event.turn_failure_kind(), Some(TurnFailureKind::Other));
        }
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_usage() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"turn.completed","usage":{"input_tokens":120,"output_tokens":45,"total_tokens":165}}"#,
        )
        .unwrap();
        let usage = event.usage().unwrap();
        assert_eq!(usage.input_tokens, Some(120));
        assert_eq!(usage.output_tokens, Some(45));
        assert_eq!(usage.total_tokens, Some(165));
        // Absent from the observed payload, so absent here rather than zero.
        assert_eq!(usage.cache_write_input_tokens, None);
        assert_eq!(usage.total(), Some(165));
    }

    #[cfg(feature = "json")]
    #[test]
    fn token_usage_total_falls_back_to_input_plus_output() {
        let usage = TokenUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            ..TokenUsage::default()
        };
        assert_eq!(usage.total(), Some(15));
    }

    /// A missing total must not read as zero, which would silently understate
    /// a session's usage.
    #[cfg(feature = "json")]
    #[test]
    fn token_usage_total_is_none_when_nothing_reported() {
        assert_eq!(TokenUsage::default().total(), None);
    }

    /// The shape a real run emits: `type` on the item, text in `text`.
    #[cfg(feature = "json")]
    #[test]
    fn agent_message_text_from_item_completed() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"hello"}}"#,
        )
        .unwrap();
        assert_eq!(event.agent_message_text().as_deref(), Some("hello"));
    }

    /// `item_type` and content blocks are not shapes the CLI has been seen to
    /// emit. The accessor still tolerates them, because the cost of being
    /// wrong here is a silently empty result rather than a loud failure, which
    /// is how #73 stayed hidden. These cases keep that tolerance covered.
    #[cfg(feature = "json")]
    #[test]
    fn agent_message_text_tolerates_layout_variants() {
        let item_type_key: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"item_type":"agent_message","text":"a"}}"#,
        )
        .unwrap();
        assert_eq!(item_type_key.agent_message_text().as_deref(), Some("a"));

        let content_blocks: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"item_type":"agent_message","content":[{"text":"b"},{"text":"c"}]}}"#,
        )
        .unwrap();
        assert_eq!(content_blocks.agent_message_text().as_deref(), Some("bc"));
    }

    #[cfg(feature = "json")]
    #[test]
    fn agent_message_text_ignores_other_items_and_events() {
        // The item type a review's diff-reading steps arrive as.
        let other_item: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"git diff","exit_code":0}}"#,
        )
        .unwrap();
        assert_eq!(other_item.agent_message_text(), None);

        let other_event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.started","item":{"id":"item_0","type":"agent_message","text":"x"}}"#,
        )
        .unwrap();
        assert_eq!(other_event.agent_message_text(), None);
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_role() {
        let event: JsonLineEvent =
            serde_json::from_str(r#"{"type":"message.created","role":"assistant"}"#).unwrap();
        assert_eq!(event.role(), Some("assistant"));
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_content_text() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"message.delta","content":[{"type":"text","text":"Hello "},{"type":"text","text":"world"}]}"#,
        )
        .unwrap();
        assert_eq!(event.content_text(), Some("Hello world".to_string()));
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_content_text_skips_non_text_blocks() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"message.delta","content":[{"type":"image","url":"x"},{"type":"text","text":"only this"}]}"#,
        )
        .unwrap();
        assert_eq!(event.content_text(), Some("only this".to_string()));
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_content_text_none_when_empty() {
        let event: JsonLineEvent =
            serde_json::from_str(r#"{"type":"message.delta","content":[]}"#).unwrap();
        assert_eq!(event.content_text(), None);
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_content_text_none_when_missing() {
        let event: JsonLineEvent = serde_json::from_str(r#"{"type":"message.delta"}"#).unwrap();
        assert_eq!(event.content_text(), None);
    }

    #[cfg(feature = "json")]
    #[test]
    fn query_result_from_events() {
        let events: Vec<JsonLineEvent> = [
            r#"{"type":"thread.started","thread_id":"thread_1"}"#,
            r#"{"type":"item.completed","item":{"item_type":"agent_message","text":"the answer"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}"#,
        ]
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

        let result = QueryResult::from_events(events);
        assert_eq!(result.result, "the answer");
        assert_eq!(result.thread_id.as_deref(), Some("thread_1"));
        assert_eq!(result.usage.unwrap().total(), Some(10));
        assert_eq!(result.events.len(), 3);
    }

    #[cfg(feature = "json")]
    #[test]
    fn query_result_concatenates_multiple_agent_messages() {
        let events: Vec<JsonLineEvent> = [
            r#"{"type":"item.completed","item":{"item_type":"agent_message","text":"one "}}"#,
            r#"{"type":"item.completed","item":{"item_type":"agent_message","text":"two"}}"#,
            r#"{"type":"turn.completed","usage":{"total_tokens":4}}"#,
        ]
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

        assert_eq!(QueryResult::from_events(events).result, "one two");
    }

    /// A failed turn has no agent message and no usage. Both must come back
    /// empty rather than fabricated.
    #[cfg(feature = "json")]
    #[test]
    fn query_result_from_a_failed_turn() {
        let events: Vec<JsonLineEvent> = [
            r#"{"type":"thread.started","thread_id":"thread_2"}"#,
            r#"{"type":"turn.failed","error":{"message":"usage limit"}}"#,
        ]
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

        let result = QueryResult::from_events(events);
        assert_eq!(result.result, "");
        assert_eq!(result.usage, None);
        assert_eq!(result.thread_id.as_deref(), Some("thread_2"));
    }

    /// Both values transcribed from captured runs.
    #[cfg(feature = "json")]
    #[test]
    fn item_type_reads_the_discriminator() {
        let message: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"hi"}}"#,
        )
        .unwrap();
        assert_eq!(message.item_type(), Some("agent_message"));

        let command: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"git diff"}}"#,
        )
        .unwrap();
        assert_eq!(command.item_type(), Some("command_execution"));

        let turn: JsonLineEvent = serde_json::from_str(r#"{"type":"turn.completed"}"#).unwrap();
        assert_eq!(turn.item_type(), None);
    }

    /// Transcribed from a captured `codex exec review` run, where the reviewer
    /// reads the diff before commenting.
    #[cfg(feature = "json")]
    #[test]
    fn command_execution_reads_a_finished_command() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"git diff","aggregated_output":"","exit_code":0,"status":"completed"}}"#,
        )
        .unwrap();

        let command = event.command_execution().unwrap();
        assert_eq!(command.command.as_deref(), Some("git diff"));
        assert_eq!(command.exit_code, Some(0));
        assert_eq!(command.status.as_deref(), Some("completed"));
    }

    /// An `item.started` has no outcome yet, and must not invent one.
    #[cfg(feature = "json")]
    #[test]
    fn command_execution_tolerates_a_command_still_running() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"git diff"}}"#,
        )
        .unwrap();

        let command = event.command_execution().unwrap();
        assert_eq!(command.command.as_deref(), Some("git diff"));
        assert_eq!(command.exit_code, None);
        assert_eq!(command.status, None);
    }

    #[cfg(feature = "json")]
    #[test]
    fn command_execution_is_none_for_other_items() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"hi"}}"#,
        )
        .unwrap();
        assert!(event.command_execution().is_none());
    }
}
