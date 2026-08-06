//! Multi-turn conversation. The first turn runs `codex exec`, later turns
//! route through `codex exec resume` with the captured thread id.
//!
//! ```sh
//! cargo run --example session
//! ```

use std::sync::Arc;

use codex_wrapper::{Codex, ExecCommand, SandboxMode, Session};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Arc::new(Codex::builder().build()?);
    let mut session = Session::new(Arc::clone(&codex));

    // First turn configured explicitly, so it runs outside a git repo.
    // `send` is the shorthand when the defaults are fine.
    session
        .execute(
            ExecCommand::new("Remember the number 41. Reply with just: stored")
                .sandbox(SandboxMode::ReadOnly)
                .skip_git_repo_check(),
        )
        .await?;

    // Second turn resumes automatically: the session captured the thread id.
    session
        .send("Add one to the number you stored. Reply with digits only.")
        .await?;

    println!("thread:      {:?}", session.id());
    println!("turns:       {}", session.total_turns());
    println!(
        "last answer: {:?}",
        session.last_result().map(|r| &r.result)
    );

    // Usage accumulates across turns. Any turn whose `turn.completed` carried
    // no usage object is counted rather than silently treated as zero.
    println!("tokens:      {}", session.total_tokens());
    let missing = session.turns_missing_usage();
    if missing > 0 {
        println!("             ({missing} turn(s) reported no usage)");
    }

    Ok(())
}
