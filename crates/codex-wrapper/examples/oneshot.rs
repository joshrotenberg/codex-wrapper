//! One prompt, one answer, raw output.
//!
//! ```sh
//! cargo run --example oneshot
//! ```

use codex_wrapper::{Codex, CodexCommand, ExecCommand, SandboxMode};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;

    let output = ExecCommand::new("Name the three primary colors. One sentence.")
        .sandbox(SandboxMode::ReadOnly)
        .skip_git_repo_check()
        .ephemeral()
        .execute(&codex)
        .await?;

    println!("{}", output.stdout);
    Ok(())
}
