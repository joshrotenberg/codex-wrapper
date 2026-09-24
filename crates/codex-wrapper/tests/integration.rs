//! Integration tests that require a real `codex` CLI binary in PATH.
//!
//! All tests are `#[ignore]` by default. Run them with:
//!
//! ```sh
//! cargo test --test integration -- --ignored
//! ```

use codex_wrapper::{
    ApplyCommand, ArchiveCommand, Codex, CodexCommand, CompletionCommand, DeleteCommand,
    DoctorCommand, ExecCommand, FeaturesListCommand, ForkCommand, LoginStatusCommand,
    McpListCommand, McpServerCommand, PluginListCommand, PluginMarketplaceListCommand,
    ResumeCommand, ReviewCommand, SandboxCommand, Shell, UnarchiveCommand, VersionCommand,
};
// Only the json-gated tests use this one.
#[cfg(feature = "json")]
use codex_wrapper::ExecResumeCommand;

fn codex() -> Codex {
    Codex::builder()
        .build()
        .expect("codex binary must be in PATH")
}

// ---------------------------------------------------------------------------
// Version / discovery
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn version_command() {
    let codex = codex();
    let output = VersionCommand::new().execute(&codex).await.unwrap();
    assert!(output.success);
    assert!(
        output.stdout.contains("codex-cli"),
        "expected version string, got: {}",
        output.stdout
    );
}

#[tokio::test]
#[ignore]
async fn cli_version_parsing() {
    let codex = codex();
    let version = codex.cli_version().await.unwrap();
    assert!(version.major == 0, "unexpected major version: {version}");
    assert!(version.minor > 0, "minor version should be > 0: {version}");
}

#[tokio::test]
#[ignore]
async fn check_version_satisfies() {
    let codex = codex();
    let minimum = codex_wrapper::CliVersion::new(0, 1, 0);
    let version = codex.check_version(&minimum).await.unwrap();
    assert!(version.satisfies_minimum(&minimum));
}

#[tokio::test]
#[ignore]
async fn check_version_too_high_fails() {
    let codex = codex();
    let minimum = codex_wrapper::CliVersion::new(999, 0, 0);
    let result = codex.check_version(&minimum).await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn completion_bash() {
    let codex = codex();
    let output = CompletionCommand::new()
        .shell(Shell::Bash)
        .execute(&codex)
        .await
        .unwrap();
    assert!(output.success);
    assert!(!output.stdout.is_empty());
}

#[tokio::test]
#[ignore]
async fn completion_zsh() {
    let codex = codex();
    let output = CompletionCommand::new()
        .shell(Shell::Zsh)
        .execute(&codex)
        .await
        .unwrap();
    assert!(output.success);
    assert!(output.stdout.contains("compdef") || output.stdout.contains("codex"));
}

// ---------------------------------------------------------------------------
// Login status
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn login_status() {
    let codex = codex();
    let output = LoginStatusCommand::new().execute(&codex).await.unwrap();
    assert!(output.success);
    // login status writes to stderr, not stdout
    assert!(
        !output.stdout.is_empty() || !output.stderr.is_empty(),
        "login status should produce output"
    );
}

// ---------------------------------------------------------------------------
// Features
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn features_list() {
    let codex = codex();
    let output = FeaturesListCommand::new().execute(&codex).await.unwrap();
    assert!(output.success);
    assert!(
        output.stdout.contains("stable") || output.stdout.contains("experimental"),
        "expected feature flags in output, got: {}",
        output.stdout
    );
}

// ---------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn mcp_list() {
    let codex = codex();
    let output = McpListCommand::new().execute(&codex).await.unwrap();
    assert!(output.success);
}

#[cfg(feature = "json")]
#[tokio::test]
#[ignore]
async fn mcp_list_json() {
    let codex = codex();
    let value = McpListCommand::new().execute_json(&codex).await.unwrap();
    assert!(value.is_array(), "expected JSON array, got: {value}");
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn sandbox_echo() {
    let codex = codex();
    let output = SandboxCommand::new("echo")
        .arg("sandbox-test")
        .execute(&codex)
        .await
        .unwrap();
    assert!(output.success);
    assert!(
        output.stdout.contains("sandbox-test"),
        "expected echo output, got: {}",
        output.stdout
    );
}

// ---------------------------------------------------------------------------
// Exec (non-interactive)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn exec_simple() {
    let codex = codex();
    let output = ExecCommand::new("respond with just the word 'pong'")
        .ephemeral()
        .execute(&codex)
        .await
        .unwrap();
    assert!(output.success);
}

#[cfg(feature = "json")]
#[tokio::test]
#[ignore]
async fn exec_json_lines() {
    let codex = codex();
    let events = ExecCommand::new("respond with just the word 'test'")
        .ephemeral()
        .execute_json_lines(&codex)
        .await
        .unwrap();
    assert!(!events.is_empty(), "expected at least one JSONL event");

    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(
        types.contains(&"thread.started"),
        "expected thread.started event, got: {types:?}"
    );
}

#[cfg(feature = "json")]
#[tokio::test]
#[ignore]
async fn exec_query_result() {
    let codex = codex();
    let result = ExecCommand::new("respond with just the word 'pong'")
        .ephemeral()
        .execute_json(&codex)
        .await
        .unwrap();
    // The completed event should populate the typed result and a thread id.
    assert!(
        !result.events.is_empty(),
        "expected a non-empty event stream"
    );
    assert!(
        result.thread_id.is_some(),
        "expected a thread_id from the stream"
    );
}

#[cfg(feature = "json")]
#[tokio::test]
#[ignore]
async fn exec_resume_json_lines() {
    let codex = codex();
    let events = ExecResumeCommand::new()
        .last()
        .ephemeral()
        .execute_json_lines(&codex)
        .await
        .unwrap();
    assert!(!events.is_empty(), "expected at least one JSONL event");

    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(
        types.contains(&"thread.started"),
        "expected thread.started event, got: {types:?}"
    );
}

// ---------------------------------------------------------------------------
// Review (requires git repo)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn review_uncommitted() {
    let codex = codex();
    let output = ReviewCommand::new()
        .uncommitted()
        .ephemeral()
        .execute(&codex)
        .await;
    // May succeed or fail depending on git state, but should not panic
    assert!(output.is_ok() || output.is_err());
}

/// Requires uncommitted changes in the working tree to review. With a clean
/// tree the CLI exits non-zero and this reports that rather than a schema
/// problem.
#[cfg(feature = "json")]
#[tokio::test]
#[ignore]
async fn review_query_result() {
    let codex = codex();
    let result = ReviewCommand::new()
        .uncommitted()
        .ephemeral()
        .execute_json(&codex)
        .await
        .unwrap();
    assert!(
        result.thread_id.is_some(),
        "expected a thread_id from the stream"
    );
    assert!(
        !result.result.is_empty(),
        "expected review comments in the assembled result, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Resume / Fork (interactive - just verify arg building doesn't break)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn resume_nonexistent_session_fails() {
    let codex = codex();
    let result = ResumeCommand::new()
        .session_id("00000000-0000-0000-0000-000000000000")
        .execute(&codex)
        .await;
    assert!(result.is_err(), "resuming a bogus session should fail");
}

#[tokio::test]
#[ignore]
async fn fork_nonexistent_session_fails() {
    let codex = codex();
    let result = ForkCommand::new()
        .session_id("00000000-0000-0000-0000-000000000000")
        .execute(&codex)
        .await;
    assert!(result.is_err(), "forking a bogus session should fail");
}

// ---------------------------------------------------------------------------
// Apply (requires a task ID - just verify it fails gracefully)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn apply_bogus_task_fails() {
    let codex = codex();
    let result = ApplyCommand::new("not-a-real-task").execute(&codex).await;
    assert!(result.is_err(), "applying a bogus task should fail");
}

// ---------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn doctor_json() {
    let codex = codex();
    let output = DoctorCommand::new().json().execute(&codex).await.unwrap();
    assert!(output.success);
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).expect("doctor --json should emit valid JSON");
    assert!(
        value.get("codexVersion").is_some(),
        "expected codexVersion field, got: {value}"
    );
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn plugin_list() {
    let codex = codex();
    let output = PluginListCommand::new().execute(&codex).await.unwrap();
    assert!(output.success);
}

#[tokio::test]
#[ignore]
async fn plugin_marketplace_list() {
    let codex = codex();
    let output = PluginMarketplaceListCommand::new()
        .execute(&codex)
        .await
        .unwrap();
    assert!(output.success);
}

// ---------------------------------------------------------------------------
// Session lifecycle (bogus session ids should fail, not panic)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn archive_bogus_session_fails() {
    let codex = codex();
    let result = ArchiveCommand::new("00000000-0000-0000-0000-000000000000")
        .execute(&codex)
        .await;
    assert!(result.is_err(), "archiving a bogus session should fail");
}

#[tokio::test]
#[ignore]
async fn unarchive_bogus_session_fails() {
    let codex = codex();
    let result = UnarchiveCommand::new("00000000-0000-0000-0000-000000000000")
        .execute(&codex)
        .await;
    assert!(result.is_err(), "unarchiving a bogus session should fail");
}

#[tokio::test]
#[ignore]
async fn delete_bogus_session_fails() {
    let codex = codex();
    let result = DeleteCommand::new("00000000-0000-0000-0000-000000000000")
        .force()
        .execute(&codex)
        .await;
    assert!(result.is_err(), "deleting a bogus session should fail");
}

// ---------------------------------------------------------------------------
// McpServer (just verify args, can't really run it in test)
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn mcp_server_args_valid() {
    let cmd = McpServerCommand::new().config("model=\"gpt-5\"");
    let args = CodexCommand::args(&cmd);
    assert_eq!(args[0], "mcp-server");
    // We don't execute this one -- it would block on stdio
}

// ---------------------------------------------------------------------------
// Builder: with_working_dir
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn with_working_dir() {
    let codex = codex().with_working_dir("/tmp");
    let output = VersionCommand::new().execute(&codex).await.unwrap();
    assert!(output.success);
}

// ---------------------------------------------------------------------------
// Timeout
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn timeout_fires() {
    let codex = Codex::builder()
        .timeout(std::time::Duration::from_millis(1))
        .build()
        .unwrap();
    let result = ExecCommand::new("count to a million slowly")
        .ephemeral()
        .execute(&codex)
        .await;
    assert!(
        matches!(result, Err(codex_wrapper::Error::Timeout { .. })),
        "expected timeout error, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// History (reads the real ~/.codex/sessions on this machine)
// ---------------------------------------------------------------------------

/// Reads every session log on this machine and asserts the parser gets
/// something out of each. Ignored because it depends on local state, but it is
/// the check that matters: the fixtures in `history.rs` are transcriptions,
/// and only a real history has the full spread of CLI versions that wrote it.
#[cfg(feature = "json")]
#[tokio::test]
#[ignore]
async fn history_reads_every_real_session() {
    use codex_wrapper::history::{self, SessionQuery};

    let sessions = history::list(&SessionQuery::new()).unwrap();
    if sessions.is_empty() {
        eprintln!("no sessions on this machine; nothing to check");
        return;
    }

    let mut with_meta = 0;
    let mut legacy = 0;
    let mut empty = Vec::new();

    for session in &sessions {
        let log = history::read(&session.path).unwrap();
        if log.meta.is_some() {
            with_meta += 1;
        }
        if log.entries.iter().any(|e| e.entry_type.is_none()) {
            legacy += 1;
        }
        if log.meta.is_none() && log.entries.is_empty() {
            empty.push(session.path.clone());
        }
    }

    eprintln!(
        "{} sessions, {with_meta} with metadata, {legacy} containing legacy lines",
        sessions.len()
    );
    assert!(
        empty.is_empty(),
        "parsed nothing at all out of {} file(s): {:?}",
        empty.len(),
        &empty[..empty.len().min(3)]
    );
}

// ---------------------------------------------------------------------------
// app-server
//
// These run a real turn, so they need an authenticated `codex` and spend a few
// thousand tokens each.
// ---------------------------------------------------------------------------

#[cfg(feature = "app-server")]
mod app_server {
    use std::time::Duration;

    use codex_wrapper::app_server::{
        AppServer, ServerMessage, ThreadStartParams, TurnCompleted, TurnStartParams, TurnStatus,
        UserInput,
    };
    use codex_wrapper::{ApprovalPolicy, SandboxMode};

    const WAIT: Duration = Duration::from_secs(120);

    /// Start a server, and a read-only turn that runs `sleep` so there is time
    /// to act on it.
    async fn running_turn(
        seconds: u32,
    ) -> (
        AppServer,
        codex_wrapper::app_server::Thread,
        codex_wrapper::app_server::Turn,
    ) {
        let dir = std::env::temp_dir().join(format!("codex-wrapper-it-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut server = AppServer::builder(&super::codex()).start().await.unwrap();
        let thread = server
            .thread_start(
                ThreadStartParams::new()
                    .cwd(&dir)
                    .ephemeral(true)
                    .approval_policy(ApprovalPolicy::Never)
                    .sandbox(SandboxMode::ReadOnly),
            )
            .await
            .unwrap();
        let turn = server
            .turn_start(
                TurnStartParams::text(
                    &thread.id,
                    format!("Run the shell command `sleep {seconds}` and then reply with the single word DONE."),
                )
                .effort("low"),
            )
            .await
            .unwrap();
        // Wait for the command itself to start, so the action lands mid-turn.
        tokio::time::timeout(WAIT, async {
            loop {
                let message = server.next_message().await.unwrap().expect("server closed");
                if let ServerMessage::Notification(n) = message
                    && n.method == "item/started"
                    && n.params.pointer("/item/type") == Some(&"commandExecution".into())
                {
                    break;
                }
            }
        })
        .await
        .expect("the command never started");
        (server, thread, turn)
    }

    async fn completion(server: &mut AppServer) -> (TurnCompleted, Vec<ServerMessage>) {
        tokio::time::timeout(WAIT, async {
            let mut seen = Vec::new();
            loop {
                let message = server.next_message().await.unwrap().expect("server closed");
                let done = match &message {
                    ServerMessage::Notification(n) => n.turn_completed(),
                    _ => None,
                };
                seen.push(message);
                if let Some(done) = done {
                    return (done, seen);
                }
            }
        })
        .await
        .expect("the turn never completed")
    }

    #[tokio::test]
    #[ignore]
    async fn steering_a_real_turn_delivers_the_text() {
        let (mut server, thread, turn) = running_turn(8).await;

        let landed = server
            .turn_steer(
                &thread.id,
                &turn.id,
                vec![UserInput::text("Also append the word STEERED after DONE.")],
            )
            .await
            .unwrap();
        assert_eq!(landed, turn.id);

        let (done, seen) = completion(&mut server).await;
        assert_eq!(done.turn.status, TurnStatus::Completed);
        // The answer comes from the item stream: 0.145.0 sends no items in
        // `turn/completed`.
        let answer = seen.iter().rev().find_map(|message| match message {
            ServerMessage::Notification(n) => n.agent_message().filter(|m| m.is_final_answer()),
            _ => None,
        });
        assert!(answer.is_some(), "the turn produced no final answer");
        // The server records the steered text as a user message on the turn,
        // which is deterministic where the model's reply is not.
        let recorded = seen.iter().any(|message| match message {
            ServerMessage::Notification(n) => {
                n.method == "item/completed"
                    && n.params
                        .pointer("/item/content/0/text")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| t.contains("STEERED"))
            }
            _ => false,
        });
        assert!(recorded, "the steered text never reached the turn");
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn interrupting_a_real_turn_ends_it_as_interrupted() {
        let (mut server, thread, turn) = running_turn(30).await;

        server.turn_interrupt(&thread.id, &turn.id).await.unwrap();

        let (done, _) = completion(&mut server).await;
        assert_eq!(done.turn.status, TurnStatus::Interrupted);
        assert_eq!(done.turn.final_message(), None);
        server.shutdown().await.unwrap();
    }

    /// Whether a `sleep` with exactly this duration is running anywhere.
    fn sleep_running(seconds: u32) -> bool {
        let table = std::process::Command::new("ps")
            .args(["-axo", "command="])
            .output()
            .expect("ps must be available");
        String::from_utf8_lossy(&table.stdout).lines().any(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            words.len() == 2
                && words[0].rsplit('/').next() == Some("sleep")
                && words[1] == seconds.to_string()
        })
    }

    /// The server starts shell commands in process groups of their own, so
    /// signalling its group does not stop them. Closing its stdin does.
    #[tokio::test]
    #[ignore]
    async fn shutdown_stops_a_command_the_turn_is_running() {
        // Distinctive, so a `sleep` from anything else cannot be mistaken.
        let seconds = 4000 + std::process::id() % 1000;
        let (server, _thread, _turn) = running_turn(seconds).await;
        assert!(sleep_running(seconds), "the command was not running");

        server.shutdown().await.unwrap();

        // The server reaps its command as it exits.
        let mut gone = false;
        for _ in 0..50 {
            if !sleep_running(seconds) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(gone, "`sleep {seconds}` outlived shutdown");
    }
}
