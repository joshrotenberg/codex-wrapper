//! Structured output: owned `--output-schema` files and typed decoding.
//!
//! `codex exec`, `exec resume` and `exec review` take a JSON Schema as a file
//! path (`--output-schema <path>`). [`OutputSchema`] serializes a
//! [`serde_json::Value`] to a private temporary file and removes it on drop,
//! so a caller can build the schema in code:
//!
//! ```no_run
//! # #[cfg(feature = "json")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use codex_wrapper::structured::OutputSchema;
//! use codex_wrapper::{Codex, ExecCommand};
//!
//! #[derive(serde::Deserialize)]
//! struct Person {
//!     name: String,
//! }
//!
//! let codex = Codex::builder().build()?;
//! let schema = OutputSchema::new(&serde_json::json!({
//!     "type": "object",
//!     "properties": {"name": {"type": "string"}},
//!     "required": ["name"],
//!     "additionalProperties": false
//! }))?;
//!
//! let result = ExecCommand::new("Extract the name from: Hi, I'm Ada Lovelace.")
//!     .output_schema(schema.path().to_string_lossy())
//!     .execute_json(&codex)
//!     .await?;
//! let person: Person = result.decode_structured()?;
//! # let _ = person.name;
//! // `schema` is dropped here, after the process has exited.
//! # Ok(()) }
//! ```
//!
//! The file lives exactly as long as the [`OutputSchema`] value, so keep it in
//! scope until the awaited call or the stream that uses it has finished. The
//! builders only store the path; building or formatting argv never creates a
//! file.
//!
//! ## Decoding
//!
//! Codex enforces the schema natively. This crate does not re-validate the
//! decoded value against it; decoding into a caller-chosen
//! [`DeserializeOwned`] type is the caller-side check.
//! [`QueryResult::decode_structured`](crate::types::QueryResult::decode_structured)
//! reads the final agent message, the last completed `agent_message` item
//! after the last `turn.started`, and applies these rules in order:
//!
//! 1. The last terminal turn event is `turn.failed`:
//!    [`StructuredOutputError::TurnFailed`], even when an agent message was
//!    produced before the failure.
//! 2. No `turn.completed` event: [`StructuredOutputError::NoTerminal`].
//! 3. No final agent message, or its text is empty or `null`:
//!    [`StructuredOutputError::Missing`].
//! 4. The text is not JSON: [`StructuredOutputError::InvalidJson`].
//! 5. The JSON is neither an object nor an array:
//!    [`StructuredOutputError::NotObjectOrArray`].
//! 6. It does not deserialize into `T`: [`StructuredOutputError::Decode`].
//!
//! Process failures (a non-zero exit, unparseable JSONL, timeouts,
//! cancellation) are reported by the execution call as [`crate::Error`]
//! before any decoding happens. Decoding never starts a process.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "json")]
use serde::de::DeserializeOwned;

#[cfg(feature = "json")]
use crate::types::JsonLineEvent;

/// A JSON Schema written to a private temporary file for `--output-schema`.
///
/// The file is created with mode `0600` on Unix and removed when this value
/// is dropped.
#[derive(Debug)]
pub struct OutputSchema {
    path: PathBuf,
}

impl OutputSchema {
    /// Serialize `schema` to a new temporary file.
    #[cfg(feature = "json")]
    pub fn new(schema: &serde_json::Value) -> std::io::Result<Self> {
        Self::from_json_string(&schema.to_string())
    }

    /// Write already-encoded schema JSON to a new temporary file.
    ///
    /// The text is written as given; Codex reports an invalid schema when the
    /// run starts.
    pub fn from_json_string(schema: &str) -> std::io::Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let path = std::env::temp_dir().join(format!(
            "codex-wrapper-schema-{}-{}.json",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let guard = Self { path };
        let mut file = options.open(&guard.path)?;
        std::io::Write::write_all(&mut file, schema.as_bytes())?;
        Ok(guard)
    }

    /// The schema file's path, to pass to `output_schema(...)`.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for OutputSchema {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Why a run's structured output could not be decoded.
#[cfg(feature = "json")]
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StructuredOutputError {
    /// The turn ended with `turn.failed`. `message` is the native failure
    /// message, when present.
    #[error("the turn failed: {}", message.as_deref().unwrap_or("no message"))]
    TurnFailed {
        /// The `turn.failed` error message, when present.
        message: Option<String>,
    },
    /// The event stream had no `turn.completed` event.
    #[error("the event stream had no turn.completed event")]
    NoTerminal,
    /// The final turn had no agent message, or its text was empty or `null`.
    #[error("the final turn had no structured agent message")]
    Missing,
    /// The final agent message was not valid JSON.
    #[error("the final agent message was not valid JSON: {0}")]
    InvalidJson(#[source] serde_json::Error),
    /// The final agent message was JSON, but neither an object nor an array.
    #[error("the final agent message was neither a JSON object nor an array")]
    NotObjectOrArray,
    /// The JSON did not deserialize into the requested type.
    #[error("the final agent message did not match the requested type: {0}")]
    Decode(#[source] serde_json::Error),
}

/// The text of the last completed agent message after the last
/// `turn.started`.
#[cfg(feature = "json")]
pub(crate) fn final_message(events: &[JsonLineEvent]) -> Option<String> {
    let start = events
        .iter()
        .rposition(|event| event.event_type == "turn.started")
        .map_or(0, |index| index + 1);

    events[start..]
        .iter()
        .rev()
        .filter(|event| event.event_type == "item.completed")
        .find_map(JsonLineEvent::agent_message_text)
}

/// Apply the decoding rules in the module docs.
#[cfg(feature = "json")]
pub(crate) fn decode<T: DeserializeOwned>(
    events: &[JsonLineEvent],
) -> Result<T, StructuredOutputError> {
    let terminal = events
        .iter()
        .rev()
        .find(|event| event.is_turn_completed() || event.is_turn_failed());

    match terminal {
        Some(event) if event.is_turn_failed() => {
            return Err(StructuredOutputError::TurnFailed {
                message: event.turn_failure_message().map(str::to_owned),
            });
        }
        Some(_) => {}
        None => return Err(StructuredOutputError::NoTerminal),
    }

    let text = final_message(events).ok_or(StructuredOutputError::Missing)?;
    if text.trim().is_empty() {
        return Err(StructuredOutputError::Missing);
    }

    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(StructuredOutputError::InvalidJson)?;

    match value {
        serde_json::Value::Null => Err(StructuredOutputError::Missing),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            T::deserialize(value).map_err(StructuredOutputError::Decode)
        }
        _ => Err(StructuredOutputError::NotObjectOrArray),
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;

    fn events(lines: &[&str]) -> Vec<JsonLineEvent> {
        lines
            .iter()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn message(id: &str, text: &str) -> String {
        serde_json::json!({
            "type": "item.completed",
            "item": {"id": id, "type": "agent_message", "text": text}
        })
        .to_string()
    }

    #[derive(Debug, serde::Deserialize)]
    struct Person {
        name: String,
    }

    #[test]
    fn schema_file_is_private_and_removed_on_drop() {
        let schema = OutputSchema::new(&serde_json::json!({"type": "object"})).unwrap();
        let path = schema.path().to_path_buf();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"type":"object"}"#
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        drop(schema);
        assert!(!path.exists());
    }

    #[test]
    fn final_message_is_the_last_agent_message_of_the_last_turn() {
        let draft = message("a", r#"{"name":"draft"}"#);
        let last = message("b", r#"{"name":"final"}"#);
        let stream = events(&[
            r#"{"type":"turn.started"}"#,
            &draft,
            r#"{"type":"item.completed","item":{"id":"r","type":"reasoning","text":"x"}}"#,
            &last,
            r#"{"type":"turn.completed"}"#,
        ]);
        assert_eq!(decode::<Person>(&stream).unwrap().name, "final");
    }

    #[test]
    fn failures_are_distinct() {
        let partial = message("a", r#"{"name":"partial"}"#);
        let failed = events(&[
            r#"{"type":"turn.started"}"#,
            &partial,
            r#"{"type":"turn.failed","error":{"message":"boom"}}"#,
        ]);
        assert!(matches!(
            decode::<Person>(&failed),
            Err(StructuredOutputError::TurnFailed { message: Some(m) }) if m == "boom"
        ));

        let ok = message("a", r#"{"name":"Ada"}"#);
        let unterminated = events(&[r#"{"type":"turn.started"}"#, &ok]);
        assert!(matches!(
            decode::<Person>(&unterminated),
            Err(StructuredOutputError::NoTerminal)
        ));

        let cases = [
            (None, "missing"),
            (Some("null"), "missing"),
            (Some(r#"{"name":"#), "invalid"),
            (Some(r#""Ada""#), "scalar"),
            (Some(r#"{"nope":1}"#), "decode"),
        ];
        for (text, kind) in cases {
            let item = text.map(|t| message("a", t));
            let mut lines = vec![r#"{"type":"turn.started"}"#];
            if let Some(item) = item.as_deref() {
                lines.push(item);
            }
            lines.push(r#"{"type":"turn.completed"}"#);
            let err = decode::<Person>(&events(&lines)).unwrap_err();
            let matched = match kind {
                "missing" => matches!(err, StructuredOutputError::Missing),
                "invalid" => matches!(err, StructuredOutputError::InvalidJson(_)),
                "scalar" => matches!(err, StructuredOutputError::NotObjectOrArray),
                _ => matches!(err, StructuredOutputError::Decode(_)),
            };
            assert!(matched, "{kind}: unexpected {err:?}");
        }
    }
}
