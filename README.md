# codex-wrapper

A type-safe Codex CLI wrapper for Rust.

[![Crates.io](https://img.shields.io/crates/v/codex-wrapper.svg)](https://crates.io/crates/codex-wrapper)
[![Documentation](https://docs.rs/codex-wrapper/badge.svg)](https://docs.rs/codex-wrapper)
[![CI](https://github.com/joshrotenberg/codex-wrapper/actions/workflows/ci.yml/badge.svg)](https://github.com/joshrotenberg/codex-wrapper/actions/workflows/ci.yml)
[![License](https://img.shields.io/crates/l/codex-wrapper.svg)](LICENSE-MIT)

## Overview

`codex-wrapper` provides a builder-pattern interface for invoking the
[Codex CLI](https://github.com/openai/codex) programmatically. It follows
the same design philosophy as
[`claude-wrapper`](https://crates.io/crates/claude-wrapper) and
[`docker-wrapper`](https://crates.io/crates/docker-wrapper): each CLI
subcommand is a builder struct that produces typed output.

## Installation

```bash
cargo add codex-wrapper
```

Requires the `codex` CLI to be installed and available in `PATH` (or
configured via `Codex::builder().binary()`).

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
use codex_wrapper::{Codex, RetryPolicy};

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

Each CLI subcommand is a separate builder:

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
| `ArchiveCommand` | `codex archive` | Archive a saved session |
| `DeleteCommand` | `codex delete` | Permanently delete a session |
| `UnarchiveCommand` | `codex unarchive` | Restore an archived session |
| `DoctorCommand` | `codex doctor` | Diagnose local install health |
| `UpdateCommand` | `codex update` | Update Codex to the latest version |
| `PluginAddCommand` | `codex plugin add` | Install a plugin |
| `PluginListCommand` | `codex plugin list` | List available plugins |
| `PluginRemoveCommand` | `codex plugin remove` | Remove an installed plugin |
| `PluginMarketplaceAddCommand` | `codex plugin marketplace add` | Add a marketplace source |
| `PluginMarketplaceListCommand` | `codex plugin marketplace list` | List marketplace sources |
| `PluginMarketplaceUpgradeCommand` | `codex plugin marketplace upgrade` | Refresh Git marketplaces |
| `PluginMarketplaceRemoveCommand` | `codex plugin marketplace remove` | Remove a marketplace source |
| `CompletionCommand` | `codex completion` | Generate shell completions |
| `FeaturesListCommand` | `codex features list` | List feature flags |
| `FeaturesEnableCommand` | `codex features enable` | Enable a feature |
| `FeaturesDisableCommand` | `codex features disable` | Disable a feature |
| `VersionCommand` | `codex --version` | Get CLI version |
| `RawCommand` | *(any)* | Escape hatch for arbitrary args |

## ExecCommand

Full coverage of `codex exec` options:

```rust
use codex_wrapper::{ExecCommand, SandboxMode};

let output = ExecCommand::new("fix the failing tests")
    .model("o3")
    .sandbox(SandboxMode::WorkspaceWrite)
    .skip_git_repo_check()
    .ephemeral()
    .json()
    .execute(&codex)
    .await?;
```

| Method | CLI Flag | Description |
|--------|----------|-------------|
| `model()` | `--model` | Model to use |
| `sandbox()` | `--sandbox` | Sandbox policy |
| `strict_config()` | `--strict-config` | Error on unknown config keys |
| `profile()` | `--profile` | Config profile |
| `full_auto()` | `--sandbox workspace-write` | Deprecated shim; `sandbox()` wins |
| `approval_policy()` | `-c approval_policy=` | When the model asks for approval |
| `search()` / `search_mode()` | `-c web_search=` | Web search mode |
| `cd()` | `--cd` | Working directory |
| `skip_git_repo_check()` | `--skip-git-repo-check` | Run outside git repo |
| `add_dir()` | `--add-dir` | Additional writable dirs |
| `ignore_user_config()` | `--ignore-user-config` | Ignore user-level config |
| `ignore_rules()` | `--ignore-rules` | Ignore project rules files |
| `dangerously_bypass_hook_trust()` | `--dangerously-bypass-hook-trust` | Skip the hook trust prompt |
| `ephemeral()` | `--ephemeral` | Don't persist session |
| `output_schema()` | `--output-schema` | JSON Schema for response |
| `color()` | `--color` | Color output mode |
| `json()` | `--json` | JSONL event output |
| `output_last_message()` | `--output-last-message` | Write last message to file |
| `image()` | `--image` | Attach image(s) |
| `config()` | `-c` | Config override |
| `enable()` / `disable()` | `--enable` / `--disable` | Feature flags |
| `oss()` | `--oss` | Use local OSS provider |
| `local_provider()` | `--local-provider` | Specify lmstudio/ollama |
| `prompt_via_stdin()` | *(prompt becomes `-`)* | Send the prompt on stdin |
| `retry()` | *(client-side)* | Per-command retry policy |

### Prompts on stdin

For prompts too large or awkward for argv, `ExecCommand::from_stdin` sends the prompt on the
child's stdin and emits `codex exec -`:

```rust
use codex_wrapper::{CodexCommand, ExecCommand};

let patch = std::fs::read_to_string("huge.patch")?;
let output = ExecCommand::from_stdin(format!("Review this patch:\n{patch}"))
    .execute(&codex)
    .await?;
```

Retry does not apply to a stdin prompt. Any policy set on the command or the client is ignored
for it: a second attempt would need to write the prompt into a pipe the first has already
consumed, and retrying with an empty stdin would be worse than not retrying.

## Typed Result

Use `execute_json()` for a typed `QueryResult` summarizing the run, assembled
from the JSONL event stream. This mirrors `claude-wrapper`'s `QueryResult` so a
downstream abstraction can treat both wrappers uniformly:

```rust
use codex_wrapper::ExecCommand;

let result = ExecCommand::new("what is 2+2?")
    .ephemeral()
    .execute_json(&codex)
    .await?;

println!("{}", result.result);
println!("thread: {:?}", result.thread_id);
println!("tokens: {:?}", result.usage.and_then(|u| u.total()));
```

`QueryResult` fields: `result`, `session_id`, `thread_id`, `usage`, and the
full `events` stream as an escape hatch.

The CLI reports **token counts, not money**. There is no cost field to read.
Converting tokens to dollars needs a per-model price table the CLI does not
provide, so this crate does not guess at one.

## JSONL Output Parsing

Use `execute_json_lines()` to parse the raw structured events from `--json`
mode. Available on both `ExecCommand` and `ExecResumeCommand`:

```rust
use codex_wrapper::ExecCommand;

let events = ExecCommand::new("what is 2+2?")
    .ephemeral()
    .execute_json_lines(&codex)
    .await?;

for event in &events {
    println!("{}: {:?}", event.event_type, event.extra);
}
```

### Typed Accessors

`JsonLineEvent` provides convenience methods for common fields:

```rust
for event in &events {
    if let Some(id) = event.thread_id() {
        println!("thread: {id}");
    }
    if event.is_turn_completed() {
        println!("tokens: {:?}", event.usage().and_then(|u| u.total()));
    }
    if let Some(text) = event.agent_message_text() {
        println!("assistant: {text}");
    }
}
```

Available accessors: `session_id()`, `thread_id()`, `is_turn_completed()`,
`is_turn_failed()`, `usage()`, `agent_message_text()`, `role()`,
`content_text()`.

The event vocabulary is `thread.started`, `turn.started`, `turn.completed`,
`turn.failed`, `item.started`, `item.updated`, `item.completed`. See the
schema notes in the `types` module docs for which parts of the payload layout
are verified against the CLI and which are assumed.

## Streaming

Stream JSONL events via a callback as they arrive, instead of buffering
all output:

```rust
use codex_wrapper::{Codex, ExecCommand, JsonLineEvent};

let codex = Codex::builder().build()?;

ExecCommand::new("explain this codebase")
    .ephemeral()
    .stream(&codex, |event: JsonLineEvent| {
        println!("{}: {:?}", event.event_type, event.extra);
    })
    .await?;
```

Also available on `ExecResumeCommand::stream()`. The child process's stderr
is drained concurrently; timeout handling mirrors the buffered exec path.

## Multi-Turn Sessions

`Session` manages conversation state across turns automatically. The first
call dispatches via `ExecCommand`; subsequent calls use `ExecResumeCommand`
with the captured `thread_id`:

```rust
use std::sync::Arc;
use codex_wrapper::{Codex, Session};

let codex = Arc::new(Codex::builder().build()?);
let mut session = Session::new(codex);

let events = session.send("create a hello world program").await?;
println!("turn 1: {} events", events.len());

let events = session.send("now add error handling").await?;
println!("turn 2: {} events, thread_id={:?}", events.len(), session.id());
```

You can also resume an existing session by thread ID:

```rust
let mut session = Session::resume(codex, "thread_abc123");
let events = session.send("continue where we left off").await?;
```

The `thread_id` is preserved even on error paths, as long as at least one
event carried it.

### Token usage

Each turn records a typed `QueryResult`, so cost accumulates across the
session:

```rust
session.send("first").await?;
session.send("second").await?;

println!("{} turns, {} tokens", session.total_turns(), session.total_tokens());

if let Some(result) = session.last_result() {
    println!("last turn: {:?}", result.usage);
}
```

The CLI does not always report usage. `total_tokens()` sums what was reported,
so a total of `0` can mean either "nothing was used" or "nothing was
reported". `turns_missing_usage()` tells those apart:

```rust
if session.turns_missing_usage() > 0 {
    eprintln!("token total is an undercount: {} turns reported none",
              session.turns_missing_usage());
}
```

### Streaming turns

`stream()` is the streaming equivalent of `send()`. Events reach the handler as
the CLI emits them, and the session still captures `thread_id`, history, and
cost, so a streaming turn and a buffered turn leave identical state:

```rust
session.stream("summarize this repo", |event| {
    println!("{}", event.event_type);
}).await?;

assert_eq!(session.total_turns(), 1);
```

`stream_execute()` and `stream_execute_resume()` take a fully configured
command, mirroring `execute()` and `execute_resume()`.

## Code Review

```rust
use codex_wrapper::ReviewCommand;

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

// Typed result, same shape ExecCommand returns
let result = ReviewCommand::new()
    .uncommitted()
    .execute_json(&codex)
    .await?;
println!("{}", result.result);
```

Review emits the same event vocabulary as `codex exec`, so `execute_json()` assembles the
review comments into `QueryResult::result`. One difference: a review's `turn.completed`
reports a usage object of all zeros, so `result.usage` carries no counts.

## MCP Server Management

```rust
use codex_wrapper::{McpListCommand, McpAddCommand, McpRemoveCommand};

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

The platform is auto-detected (Seatbelt on macOS, and so on); as of
`codex-cli` 0.145.0 the old `<macos|linux|windows>` positional was removed.

```rust
use codex_wrapper::SandboxCommand;

let output = SandboxCommand::new("ls")
    .arg("-la")
    .execute(&codex)
    .await?;
```

## Session Resume and Fork

```rust
use codex_wrapper::{ResumeCommand, ForkCommand};

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
use codex_wrapper::{CompletionCommand, Shell};

let output = CompletionCommand::new()
    .shell(Shell::Zsh)
    .execute(&codex)
    .await?;
std::fs::write("_codex", &output.stdout)?;
```

## Feature Flags

```rust
use codex_wrapper::{FeaturesListCommand, FeaturesEnableCommand, FeaturesDisableCommand};

// List all feature flags
FeaturesListCommand::new().execute(&codex).await?;

// Enable/disable features persistently
FeaturesEnableCommand::new("web-search").execute(&codex).await?;
FeaturesDisableCommand::new("web-search").execute(&codex).await?;
```

## CLI Version

This wrapper is tested against a declared range of `codex-cli` versions. Both
ends of the range run the flag-contract check in CI, so the range reflects what
is actually verified rather than what is hoped.

```rust
use codex_wrapper::{CliVersionStatus, TESTED_CLI_VERSION_MIN, TESTED_CLI_VERSION_MAX};

// Report, do not fail. Warns via `tracing` when outside the range.
match codex.cli_version_status().await? {
    CliVersionStatus::Tested => {}
    CliVersionStatus::NewerUntested { found, tested_max } => {
        eprintln!("codex {found} is newer than tested ({tested_max})");
    }
    CliVersionStatus::OlderThanMinimum { found, minimum } => {
        eprintln!("codex {found} is older than tested ({minimum})");
    }
}
```

Most CLI releases break nothing, so this reports by default rather than
refusing to run. When you do want a hard gate:

```rust
// Returns Error::UntestedCliVersion when outside the range.
let version = codex.ensure_tested_cli_version().await?;
```

This is a method rather than a builder option because `build()` is synchronous
and never spawns the binary.

Override the range with `CodexBuilder::tested_cli_version_range(min, max)` if
you have validated a different one yourself. `check_version(&minimum)` remains
available for a plain minimum-version gate.

## Error Handling

All commands return `Result<T>`, with errors typed via `thiserror`:

```rust
use codex_wrapper::{ExecCommand, Error};

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

## Previewing the Command

Every builder can render the exact command line it will spawn, without spawning it:

```rust
use codex_wrapper::{Codex, CodexCommand, ExecCommand};

let codex = Codex::builder().config("model=\"gpt-5\"").build()?;
let cmd = ExecCommand::new("fix the failing tests").ephemeral();

println!("{}", cmd.to_command_string(&codex));
// codex -c 'model="gpt-5"' exec --ephemeral 'fix the failing tests'
```

Global args precede the subcommand, the same order the spawn uses, because the preview and both
spawn paths share one assembly function. The output is quoted for a POSIX shell so it can be
pasted, but no shell is involved at spawn time: args go to the process directly.

## Cancellation

Dropping the future returned by a command kills the spawned `codex` process. That covers a
timeout, an aborted task, and a caller that stops awaiting during a graceful shutdown:
cancelling the future cancels the work, rather than leaving codex running and billing with no
handle left to stop it.

```rust
use codex_wrapper::{ExecCommand, CodexCommand};
use std::time::Duration;

// The codex process is killed when the timeout drops the future.
let result = tokio::time::timeout(
    Duration::from_secs(30),
    ExecCommand::new("long task").execute(&codex),
)
.await;
```

Two limits are worth knowing:

- The kill reaps the `codex` process itself. Subprocesses codex spawned for tool use are not
  signalled and can outlive it.
- Reaping needs the tokio runtime to still be running. A future dropped as part of runtime
  shutdown may not get far enough to kill the child.

## Retry Policy

Configure automatic retries for transient failures:

```rust
use codex_wrapper::{Codex, ExecCommand, RetryPolicy};
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

## Escape Hatch: RawCommand

For subcommands or flags not yet covered by typed builders:

```rust
use codex_wrapper::RawCommand;

let output = RawCommand::new("cloud")
    .arg("--json")
    .execute(&codex)
    .await?;
```

## Cargo Features

| Feature | Default | Description |
|---------|---------|-------------|
| `json` | Yes | JSONL output parsing via `serde_json` -- enables `execute_json_lines()`, `execute_json()`, `stream()`, `Session`, `QueryResult`, `JsonLineEvent` and typed accessors |

To disable default features:

```toml
[dependencies]
codex-wrapper = { version = "0.2", default-features = false }
```

## Examples

Runnable programs live in
[`crates/codex-wrapper/examples`](https://github.com/joshrotenberg/codex-wrapper/tree/main/crates/codex-wrapper/examples).
Each needs a working `codex` on `PATH`:

```bash
cargo run --example oneshot        # one prompt, raw output
cargo run --example json_output    # JSONL events and the typed QueryResult
cargo run --example stream_exec    # events delivered as they arrive
cargo run --example session        # multi-turn via exec resume
cargo run --example review         # code review, text and typed
cargo run --example mcp_servers    # add / list / inspect / remove MCP servers
cargo run --example health_check   # installed CLI vs the tested version range
```

All but `oneshot` and `health_check` need the `json` feature, which is on by default. Each is
declared with its `required-features` in the manifest, so a reduced-feature build skips it
rather than failing.

## Testing

```bash
cargo test --lib --all-features           # Unit tests (no CLI required)
cargo test --test integration -- --ignored # Integration tests (requires codex in PATH)
```

## CI and Release

GitHub Actions workflows handle CI (Linux, macOS, Windows), dependency
audits, changelog automation, and `release-plz`-driven crates.io releases.

## License

MIT OR Apache-2.0
