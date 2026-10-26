//! Structured-output tests against `tests/fake-codex-structured.sh`.
//!
//! The fake never runs a real CLI. Every scenario is synthetic; see the
//! script header.

#![cfg(all(unix, feature = "json"))]

use std::path::{Path, PathBuf};

use codex_wrapper::structured::{OutputSchema, StructuredOutputError};
use codex_wrapper::types::QueryResult;
use codex_wrapper::{Codex, Error, ExecCommand, ExecResumeCommand, ReviewCommand};

#[derive(Debug, serde::Deserialize, PartialEq)]
struct Person {
    name: String,
}

fn schema_value() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {"name": {"type": "string"}},
        "required": ["name"],
        "additionalProperties": false
    })
}

struct Calls(PathBuf);

impl Calls {
    fn new() -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "codex-wrapper-structured-{}-{n}.calls",
            std::process::id()
        )))
    }

    /// One `(argv, schema contents seen by the child)` pair per invocation.
    fn read(&self) -> Vec<(String, String)> {
        let text = std::fs::read_to_string(&self.0).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines
            .chunks(2)
            .map(|pair| {
                (
                    pair[0].to_string(),
                    pair[1].trim_start_matches("schema:").to_string(),
                )
            })
            .collect()
    }
}

impl Drop for Calls {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn fake(scenario: &str, calls: &Calls) -> Codex {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fake-codex-structured.sh");
    Codex::builder()
        .binary("/bin/bash")
        .arg(script.to_str().unwrap())
        .env("FAKE_CODEX_SCENARIO", scenario)
        .env("FAKE_CODEX_CALLS", calls.0.to_string_lossy())
        .build()
        .unwrap()
}

/// Run an exec against `scenario` with an owned schema, then drop the schema.
async fn run(
    scenario: &str,
) -> (
    codex_wrapper::Result<QueryResult>,
    Vec<(String, String)>,
    PathBuf,
) {
    let calls = Calls::new();
    let schema = OutputSchema::new(&schema_value()).unwrap();
    let path = schema.path().to_path_buf();
    let outcome = ExecCommand::new("extract")
        .output_schema(path.to_string_lossy())
        .execute_json(&fake(scenario, &calls))
        .await;
    drop(schema);
    (outcome, calls.read(), path)
}

#[tokio::test]
async fn decodes_an_object_with_an_owned_schema_file() {
    let (outcome, calls, path) = run("object").await;
    let result = outcome.unwrap();

    assert_eq!(
        result.decode_structured::<Person>().unwrap(),
        Person {
            name: "Ada Lovelace".into()
        }
    );
    assert_eq!(calls.len(), 1);
    let (argv, schema_seen) = &calls[0];
    assert!(argv.contains("--json"), "{argv}");
    assert!(argv.contains("--output-schema"), "{argv}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(schema_seen).unwrap(),
        schema_value()
    );
    assert!(!path.exists(), "schema file must be removed on drop");
}

#[tokio::test]
async fn decodes_an_array_and_selects_the_final_message() {
    let (outcome, _, _) = run("array").await;
    let people: Vec<Person> = outcome.unwrap().decode_structured().unwrap();
    assert_eq!(people.len(), 2);

    let (outcome, _, _) = run("multiple_messages").await;
    let result = outcome.unwrap();
    assert_eq!(result.decode_structured::<Person>().unwrap().name, "final");
    // The concatenated text and every event stay available.
    assert!(result.result.contains("draft"));
    assert!(
        result
            .events
            .iter()
            .any(|e| e.event_type == "thread.started")
    );

    let (outcome, _, _) = run("transient_error").await;
    assert_eq!(
        outcome.unwrap().decode_structured::<Person>().unwrap().name,
        "Ada"
    );
}

#[tokio::test]
async fn decoding_failures_are_distinct_and_start_no_process() {
    type Check = fn(&StructuredOutputError) -> bool;
    let cases: [(&str, Check); 6] = [
        ("missing", |e| matches!(e, StructuredOutputError::Missing)),
        ("null", |e| matches!(e, StructuredOutputError::Missing)),
        ("invalid_json", |e| {
            matches!(e, StructuredOutputError::InvalidJson(_))
        }),
        ("scalar", |e| {
            matches!(e, StructuredOutputError::NotObjectOrArray)
        }),
        ("no_terminal", |e| {
            matches!(e, StructuredOutputError::NoTerminal)
        }),
        (
            "turn_failed_exit_zero",
            |e| matches!(e, StructuredOutputError::TurnFailed { message: Some(m) } if m == "synthetic failure"),
        ),
    ];

    for (scenario, expected) in cases {
        let (outcome, calls, _) = run(scenario).await;
        let result = outcome.unwrap_or_else(|e| panic!("{scenario}: process failed: {e}"));
        let err = result.decode_structured::<Person>().unwrap_err();
        assert!(expected(&err), "{scenario}: unexpected {err:?}");
        assert_eq!(
            calls.len(),
            1,
            "{scenario}: decoding must not start a process"
        );
    }

    let (outcome, _, _) = run("object").await;
    let err = outcome
        .unwrap()
        .decode_structured::<Vec<u32>>()
        .unwrap_err();
    assert!(matches!(err, StructuredOutputError::Decode(_)));
}

#[tokio::test]
async fn non_zero_exits_are_process_failures_and_remove_the_schema() {
    for scenario in ["turn_failed", "error_only", "success_then_exit", "crash"] {
        let (outcome, calls, path) = run(scenario).await;
        assert!(
            matches!(outcome, Err(Error::CommandFailed { .. })),
            "{scenario}: {outcome:?}"
        );
        assert_eq!(calls.len(), 1, "{scenario}");
        assert!(!path.exists(), "{scenario}: schema file must be removed");
    }
}

#[tokio::test]
async fn resume_and_review_accept_the_owned_schema() {
    let schema = OutputSchema::new(&schema_value()).unwrap();
    let schema_path = schema.path().to_string_lossy().into_owned();

    let calls = Calls::new();
    let result = ExecResumeCommand::new()
        .last()
        .prompt("again")
        .output_schema(&schema_path)
        .execute_json(&fake("object", &calls))
        .await
        .unwrap();
    assert_eq!(
        result.decode_structured::<Person>().unwrap().name,
        "Ada Lovelace"
    );
    assert!(calls.read()[0].0.contains("resume"));

    let calls = Calls::new();
    let result = ReviewCommand::new()
        .output_schema(&schema_path)
        .execute_json(&fake("object", &calls))
        .await
        .unwrap();
    assert_eq!(
        result.decode_structured::<Person>().unwrap().name,
        "Ada Lovelace"
    );
    assert!(calls.read()[0].0.contains("review"));
}
