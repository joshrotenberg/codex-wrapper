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
//! and a live run:
//!
//! - The event vocabulary is `thread.started`, `turn.started`, `turn.completed`,
//!   `turn.failed`, `item.started`, `item.updated`, `item.completed`. There is
//!   no bare `completed`.
//! - `thread.started` carries `thread_id`.
//! - A completed turn reports token counts, never a monetary cost. The
//!   `TokenUsage` fields are `input_tokens`, `cached_input_tokens`,
//!   `cache_write_input_tokens`, `output_tokens`, `reasoning_output_tokens`,
//!   `total_tokens`.
//!
//! **Assumed**, reconstructed from binary string tables rather than a live
//! successful run:
//!
//! - Assistant text arrives as an `item.completed` event whose `item` has an
//!   `agent_message` discriminator.
//! - That the discriminator is `item_type` and the text is a `text` field.
//!   [`JsonLineEvent::agent_message_text`] accepts `type` and a `content`
//!   block array as alternatives rather than committing to one.
//!
//! To confirm, capture a real run and check `result` is non-empty:
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
    /// The item layout here is **assumed**, not verified against a live run;
    /// see the ASSUMPTIONS block on [`QueryResult`]. Both `item_type` and
    /// `type` are accepted as the discriminator, and the text is read from
    /// either a `text` field or a `content` block array, because which of
    /// those the CLI emits has not been confirmed. Tolerating both is
    /// deliberate: the previous parser assumed one exact shape and silently
    /// yielded empty results when it was wrong, which is the bug this
    /// replaces.
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
    /// Empty when the turn produced no agent message, which includes every
    /// failed turn.
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

/// Token counts reported on a completed turn.
///
/// `codex-cli` 0.145.0 reports token usage and no monetary cost. Converting
/// tokens to dollars needs a per-model price table the CLI does not provide,
/// so this crate does not attempt it: a hardcoded table would go stale
/// silently, which is the same class of bug as #73.
///
/// Every field is optional. An observed `turn.completed` carried only three of
/// the six, so absence is normal rather than exceptional.
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
    /// Total tokens as reported by the CLI.
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
    #[test]
    fn token_usage_total_is_none_when_nothing_reported() {
        assert_eq!(TokenUsage::default().total(), None);
    }

    #[test]
    fn agent_message_text_from_item_completed() {
        let event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"item_type":"agent_message","text":"hello"}}"#,
        )
        .unwrap();
        assert_eq!(event.agent_message_text().as_deref(), Some("hello"));
    }

    /// The item layout is assumed rather than verified, so the accessor
    /// tolerates the plausible variants instead of committing to one and
    /// silently returning nothing when wrong. See #73.
    #[test]
    fn agent_message_text_tolerates_layout_variants() {
        let type_key: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"a"}}"#,
        )
        .unwrap();
        assert_eq!(type_key.agent_message_text().as_deref(), Some("a"));

        let content_blocks: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"item_type":"agent_message","content":[{"text":"b"},{"text":"c"}]}}"#,
        )
        .unwrap();
        assert_eq!(content_blocks.agent_message_text().as_deref(), Some("bc"));
    }

    #[test]
    fn agent_message_text_ignores_other_items_and_events() {
        let other_item: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.completed","item":{"item_type":"command_execution","text":"ls"}}"#,
        )
        .unwrap();
        assert_eq!(other_item.agent_message_text(), None);

        let other_event: JsonLineEvent = serde_json::from_str(
            r#"{"type":"item.started","item":{"item_type":"agent_message","text":"x"}}"#,
        )
        .unwrap();
        assert_eq!(other_event.agent_message_text(), None);
    }

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
}
