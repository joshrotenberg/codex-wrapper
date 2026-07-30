//! Domain types shared across commands: enums for CLI options, version parsing,
//! and structured JSONL events.

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

    /// Returns `true` when the event type is `"completed"`.
    #[must_use]
    pub fn is_completed(&self) -> bool {
        self.event_type == "completed"
    }

    /// Returns the nested `result.text` field, if present and a string.
    #[must_use]
    pub fn result_text(&self) -> Option<&str> {
        self.extra
            .get("result")
            .and_then(|v| v.get("text"))
            .and_then(|v| v.as_str())
    }

    /// Returns the nested `result.cost` field in USD, if present and numeric.
    #[must_use]
    pub fn cost_usd(&self) -> Option<f64> {
        self.extra
            .get("result")
            .and_then(|v| v.get("cost"))
            .and_then(|v| v.as_f64())
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
    /// Final assistant text from the terminal `completed` event.
    ///
    /// Empty if no `completed` event carried a `result.text` value.
    pub result: String,
    /// The `session_id` captured from the event stream, if any.
    pub session_id: Option<String>,
    /// The `thread_id` captured from the event stream, if any.
    ///
    /// This is Codex's native identifier for resuming a conversation.
    pub thread_id: Option<String>,
    /// Total cost in USD from the `completed` event, if reported.
    pub cost_usd: Option<f64>,
    /// The full parsed event stream this result was assembled from.
    pub events: Vec<JsonLineEvent>,
}

#[cfg(feature = "json")]
impl QueryResult {
    /// Assemble a [`QueryResult`] from a parsed JSONL event stream.
    ///
    /// `result` and `cost_usd` are taken from the last `completed` event;
    /// `session_id` and `thread_id` are the first occurrences in the stream.
    #[must_use]
    pub fn from_events(events: Vec<JsonLineEvent>) -> Self {
        let completed = events.iter().rev().find(|e| e.is_completed());
        let result = completed
            .and_then(JsonLineEvent::result_text)
            .unwrap_or_default()
            .to_string();
        let cost_usd = completed.and_then(JsonLineEvent::cost_usd);
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
            cost_usd,
            events,
        }
    }
}

/// Parsed semantic version of the Codex CLI (`major.minor.patch`).
///
/// Supports comparison and ordering for version-gating logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn json_line_event_is_completed() {
        let completed: JsonLineEvent = serde_json::from_str(r#"{"type":"completed"}"#).unwrap();
        assert!(completed.is_completed());

        let other: JsonLineEvent = serde_json::from_str(r#"{"type":"message.created"}"#).unwrap();
        assert!(!other.is_completed());
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_result_text_and_cost() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"completed","result":{"text":"hello world","cost":0.0042}}"#,
        )
        .unwrap();
        assert_eq!(event.result_text(), Some("hello world"));
        assert!((event.cost_usd().unwrap() - 0.0042).abs() < f64::EPSILON);
    }

    #[cfg(feature = "json")]
    #[test]
    fn json_line_event_result_text_missing() {
        let event: JsonLineEvent = serde_json::from_str(r#"{"type":"completed"}"#).unwrap();
        assert_eq!(event.result_text(), None);
        assert_eq!(event.cost_usd(), None);
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
        let events: Vec<JsonLineEvent> = vec![
            serde_json::from_str(
                r#"{"type":"thread.started","session_id":"sess_1","thread_id":"thread_1"}"#,
            )
            .unwrap(),
            serde_json::from_str(r#"{"type":"message.created","role":"assistant"}"#).unwrap(),
            serde_json::from_str(r#"{"type":"completed","result":{"text":"done","cost":0.02}}"#)
                .unwrap(),
        ];
        let result = QueryResult::from_events(events);
        assert_eq!(result.result, "done");
        assert_eq!(result.session_id.as_deref(), Some("sess_1"));
        assert_eq!(result.thread_id.as_deref(), Some("thread_1"));
        assert_eq!(result.cost_usd, Some(0.02));
        assert_eq!(result.events.len(), 3);
    }

    #[cfg(feature = "json")]
    #[test]
    fn query_result_from_events_no_completed() {
        let events: Vec<JsonLineEvent> =
            vec![serde_json::from_str(r#"{"type":"message.created"}"#).unwrap()];
        let result = QueryResult::from_events(events);
        assert_eq!(result.result, "");
        assert_eq!(result.cost_usd, None);
        assert!(result.session_id.is_none());
        assert!(result.thread_id.is_none());
    }
}
