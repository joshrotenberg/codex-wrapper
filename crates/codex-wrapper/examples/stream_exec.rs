//! Stream events as they arrive instead of buffering the whole run.
//!
//! ```sh
//! cargo run --example stream_exec
//! ```

use codex_wrapper::{Codex, ExecCommand, JsonLineEvent, SandboxMode};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;

    let cmd = ExecCommand::new("Count from one to five, one number per line.")
        .sandbox(SandboxMode::ReadOnly)
        .skip_git_repo_check()
        .ephemeral();

    let mut turns = 0;
    cmd.stream(&codex, |event: JsonLineEvent| {
        if let Some(text) = event.agent_message_text() {
            println!("{text}");
        }
        if event.is_turn_completed() {
            turns += 1;
        }
    })
    .await?;

    println!("\n{turns} turn(s) completed");
    Ok(())
}
