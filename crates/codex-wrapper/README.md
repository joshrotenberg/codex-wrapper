# codex-wrapper

A type-safe Codex CLI wrapper for Rust

[![Crates.io](https://img.shields.io/crates/v/codex-wrapper.svg)](https://crates.io/crates/codex-wrapper)
[![Documentation](https://docs.rs/codex-wrapper/badge.svg)](https://docs.rs/codex-wrapper)
[![CI](https://github.com/joshrotenberg/codex-wrapper/actions/workflows/ci.yml/badge.svg)](https://github.com/joshrotenberg/codex-wrapper/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/codex-wrapper.svg)](LICENSE-MIT)

## Overview

`codex-wrapper` provides a type-safe builder-pattern interface for invoking the
`codex` CLI programmatically. It follows the same design philosophy as
[`claude-wrapper`](https://crates.io/crates/claude-wrapper) and
[`docker-wrapper`](https://crates.io/crates/docker-wrapper): each CLI
subcommand is a builder struct that produces typed output.

## Installation

```bash
cargo add codex-wrapper
```

## Quick Start

```rust
use codex_wrapper::{Codex, CodexCommand, ExecCommand, SandboxMode};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;
    let output = ExecCommand::new("explain this error")
        .model("o3")
        .sandbox(SandboxMode::WorkspaceWrite)
        .ephemeral()
        .execute(&codex)
        .await?;
    println!("{}", output.stdout);
    Ok(())
}
```

## Two-Layer Builder Architecture

The `Codex` client holds shared configuration (binary path, environment,
timeout, retry policy). Command builders hold per-invocation options and call
`execute(&codex)`.

### Codex Client

Configure once, reuse across commands:

```rust
let codex = Codex::builder()
    .env("OPENAI_API_KEY", "sk-...")
    .timeout_secs(300)
    .retry(RetryPolicy::new().max_attempts(3).exponential())
    .build()?;
```

Options:
- `binary()` -- path to `codex` binary (auto-detected via `PATH` by default)
- `working_dir()` -- working directory for commands
- `env()` / `envs()` -- environment variables
- `timeout_secs()` / `timeout()` -- command timeout
- `config()` -- global config overrides (`-c key=value`)
- `enable()` / `disable()` -- global feature flags
- `retry()` -- default retry policy

### Command Builders

Each CLI subcommand is a separate builder. Available commands:

| Command | CLI Equivalent | Description |
|---------|---------------|-------------|
| `ExecCommand` | `codex exec` | Run Codex non-interactively |
| `ExecResumeCommand` | `codex exec resume` | Resume a non-interactive session |
| `ReviewCommand` | `codex exec review` | Code review with git integration |
| `ResumeCommand` | `codex resume` | Resume an interactive session |
| `ForkCommand` | `codex fork` | Fork an interactive session |
| `LoginCommand` | `codex login` | Authenticate |
| `LoginStatusCommand` | `codex login status` | Check auth status |
| `LogoutCommand` | `codex logout` | Remove credentials |
| `McpListCommand` | `codex mcp list` | List MCP servers |
| `McpGetCommand` | `codex mcp get` | Get MCP server details |
| `McpAddCommand` | `codex mcp add` | Add stdio or HTTP MCP server |
| `McpRemoveCommand` | `codex mcp remove` | Remove MCP server |
| `McpLoginCommand` | `codex mcp login` | Auth to MCP server |
| `McpLogoutCommand` | `codex mcp logout` | Deauth from MCP server |
| `McpServerCommand` | `codex mcp-server` | Start Codex as MCP server |
| `SandboxCommand` | `codex sandbox` | Run command in sandbox |
| `ApplyCommand` | `codex apply` | Apply agent diff |
| `CompletionCommand` | `codex completion` | Generate shell completions |
| `FeaturesListCommand` | `codex features list` | List feature flags |
| `FeaturesEnableCommand` | `codex features enable` | Enable a feature |
| `FeaturesDisableCommand` | `codex features disable` | Disable a feature |
| `VersionCommand` | `codex --version` | Get CLI version |
| `RawCommand` | *(any)* | Escape hatch for arbitrary args |

## ExecCommand: The Workhorse

Full coverage of `codex exec` options:

```rust
let output = ExecCommand::new("fix the failing tests")
    .model("o3")
    .sandbox(SandboxMode::WorkspaceWrite)
    .approval_policy(ApprovalPolicy::Never)
    .skip_git_repo_check()
    .ephemeral()
    .json()
    .execute(&codex)
    .await?;
```

All ExecCommand options:

| Method | CLI Flag | Description |
|--------|----------|-------------|
| `model()` | `--model` | Model to use |
| `sandbox()` | `--sandbox` | Sandbox policy |
| `approval_policy()` | `--ask-for-approval` | Approval policy |
| `profile()` | `--profile` | Config profile |
| `full_auto()` | `--full-auto` | Auto sandbox + approval |
| `dangerously_bypass_approvals_and_sandbox()` | `--dangerously-bypass-approvals-and-sandbox` | Skip all safety |
| `cd()` | `--cd` | Working directory |
| `skip_git_repo_check()` | `--skip-git-repo-check` | Run outside git repo |
| `add_dir()` | `--add-dir` | Additional writable dirs |
| `search()` | `--search` | Enable web search |
| `ephemeral()` | `--ephemeral` | Don't persist session |
| `output_schema()` | `--output-schema` | JSON Schema for response |
| `color()` | `--color` | Color output mode |
| `progress_cursor()` | `--progress-cursor` | Cursor-based progress |
| `json()` | `--json` | JSONL event output |
| `output_last_message()` | `--output-last-message` | Write last message to file |
| `image()` | `--image` | Attach image(s) |
| `config()` | `-c` | Config override |
| `enable()` / `disable()` | `--enable` / `--disable` | Feature flags |
| `oss()` | `--oss` | Use local OSS provider |
| `local_provider()` | `--local-provider` | Specify lmstudio/ollama |
| `retry()` | *(client-side)* | Per-command retry policy |

## JSONL Output Parsing

Use `execute_json_lines()` to parse structured events from `--json` mode:

```rust
let events = ExecCommand::new("what is 2+2?")
    .ephemeral()
    .execute_json_lines(&codex)
    .await?;

for event in &events {
    println!("{}: {:?}", event.event_type, event.extra);
}
```

Event types include `thread.started`, `turn.started`, `item.completed`,
`turn.completed`, and more.

## Code Review

```rust
// Review uncommitted changes
let output = ReviewCommand::new()
    .uncommitted()
    .model("o3")
    .execute(&codex)
    .await?;

// Review against a base branch
let output = ReviewCommand::new()
    .base("main")
    .json()
    .execute(&codex)
    .await?;
```

## MCP Server Management

```rust
// List servers
let output = McpListCommand::new().execute(&codex).await?;

// List as JSON
let servers = McpListCommand::new().execute_json(&codex).await?;

// Add stdio server
McpAddCommand::stdio("my-tool", "npx")
    .arg("my-mcp-server")
    .env("API_KEY", "secret")
    .execute(&codex)
    .await?;

// Add HTTP server
McpAddCommand::http("sentry", "https://mcp.sentry.dev/mcp")
    .bearer_token_env_var("SENTRY_TOKEN")
    .execute(&codex)
    .await?;

// Remove server
McpRemoveCommand::new("old-server").execute(&codex).await?;
```

## Sandbox Execution

Run commands inside the Codex sandbox:

```rust
let output = SandboxCommand::new(SandboxPlatform::MacOs, "ls")
    .arg("-la")
    .execute(&codex)
    .await?;
```

## Session Management

```rust
// Resume the most recent interactive session
ResumeCommand::new()
    .last()
    .model("o3")
    .execute(&codex)
    .await?;

// Fork a session to try a different approach
ForkCommand::new()
    .session_id("abc-123")
    .prompt("try a different approach")
    .execute(&codex)
    .await?;
```

## Shell Completions

```rust
let output = CompletionCommand::new()
    .shell(Shell::Zsh)
    .execute(&codex)
    .await?;
std::fs::write("_codex", &output.stdout)?;
```

## Feature Flags

```rust
// List all feature flags
FeaturesListCommand::new().execute(&codex).await?;

// Enable/disable features persistently
FeaturesEnableCommand::new("web-search").execute(&codex).await?;
FeaturesDisableCommand::new("web-search").execute(&codex).await?;
```

## Escape Hatch: RawCommand

For subcommands or flags not yet covered by typed builders:

```rust
let output = RawCommand::new("cloud")
    .arg("--json")
    .execute(&codex)
    .await?;
```

## Error Handling

All commands return `Result<T>`, with errors typed via `thiserror`:

```rust
use codex_wrapper::Error;

match ExecCommand::new("test").execute(&codex).await {
    Ok(output) => println!("{}", output.stdout),
    Err(Error::CommandFailed { stderr, exit_code, .. }) => {
        eprintln!("failed (exit {}): {}", exit_code, stderr);
    }
    Err(Error::Timeout { .. }) => eprintln!("timed out"),
    Err(Error::NotFound) => eprintln!("codex binary not in PATH"),
    Err(e) => eprintln!("{e}"),
}
```

## Retry Policy

Configure automatic retries for transient failures:

```rust
use codex_wrapper::RetryPolicy;
use std::time::Duration;

let policy = RetryPolicy::new()
    .max_attempts(5)
    .initial_backoff(Duration::from_secs(2))
    .exponential()
    .retry_on_timeout(true)
    .retry_on_exit_codes([1, 2]);

// Set on the client (applies to all commands)
let codex = Codex::builder().retry(policy).build()?;

// Or override per-command
let output = ExecCommand::new("flaky task")
    .retry(RetryPolicy::new().max_attempts(10))
    .execute(&codex)
    .await?;
```

## Features

Optional Cargo features (enabled by default):

- `json` -- JSONL output parsing via `serde_json` (`execute_json_lines()`,
  `execute_json()`, `JsonLineEvent`)

## Testing

```bash
cargo test --lib --all-features           # Unit tests (no CLI required)
cargo test --test integration -- --ignored # Integration tests (requires codex in PATH)
```

## License

MIT OR Apache-2.0
