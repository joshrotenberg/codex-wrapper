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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicy {
    /// Only run trusted commands without asking.
    Untrusted,
    /// Ask on failure (deprecated -- prefer `OnRequest` or `Never`).
    OnFailure,
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
            Self::OnFailure => "on-failure",
            Self::OnRequest => "on-request",
            Self::Never => "never",
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
        let version = CliVersion::parse_version_output("codex-cli 0.116.0").unwrap();
        assert_eq!(version, CliVersion::new(0, 116, 0));
    }

    #[test]
    fn parses_plain_version_output() {
        let version = CliVersion::parse_version_output("0.116.0").unwrap();
        assert_eq!(version, CliVersion::new(0, 116, 0));
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
}
