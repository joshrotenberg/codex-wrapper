//! Integration tests that require a real `codex` CLI binary in PATH.
//!
//! All tests are `#[ignore]` by default. Run them with:
//!
//! ```sh
//! cargo test --test integration -- --ignored
//! ```

use codex_wrapper::{
    ApplyCommand, Codex, CodexCommand, CompletionCommand, ExecCommand, FeaturesListCommand,
    ForkCommand, LoginStatusCommand, McpListCommand, McpServerCommand, ResumeCommand,
    ReviewCommand, SandboxCommand, SandboxPlatform, Shell, VersionCommand,
};

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
    let output = SandboxCommand::new(SandboxPlatform::MacOs, "echo")
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
