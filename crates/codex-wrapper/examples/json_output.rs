//! Structured JSONL output: the raw event stream, and the typed result
//! assembled from it.
//!
//! ```sh
//! cargo run --example json_output
//! ```

use codex_wrapper::{Codex, ExecCommand, SandboxMode};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;

    let cmd = ExecCommand::new("Reply with exactly: ok")
        .sandbox(SandboxMode::ReadOnly)
        .skip_git_repo_check()
        .ephemeral();

    // The raw stream, one parsed event per JSONL line.
    let events = cmd.execute_json_lines(&codex).await?;
    println!("{} events", events.len());
    for event in &events {
        println!("  {}", event.event_type);
    }

    // The same run, assembled into a typed result.
    let result = cmd.execute_json(&codex).await?;
    println!("\nresult: {}", result.result);
    println!("thread: {:?}", result.thread_id);

    // The CLI reports token counts and no monetary cost. `total()` falls back
    // to input plus output, because a real turn.completed carries no
    // total_tokens field.
    match result.usage.and_then(|usage| usage.total()) {
        Some(tokens) => println!("tokens: {tokens}"),
        None => println!("tokens: not reported"),
    }

    Ok(())
}
