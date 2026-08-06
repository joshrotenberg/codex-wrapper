//! Review the uncommitted changes in the current repo, as text and as a
//! typed result.
//!
//! ```sh
//! cargo run --example review
//! ```
//!
//! Needs a git repo with uncommitted changes. With a clean tree the CLI has
//! nothing to review and exits non-zero.

use codex_wrapper::{Codex, CodexCommand, ReviewCommand};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;

    let output = ReviewCommand::new()
        .uncommitted()
        .ephemeral()
        .execute(&codex)
        .await?;
    println!("{}", output.stdout);

    // The same review as a typed result. Review emits the same event
    // vocabulary as exec, so the comments land in `result`.
    let result = ReviewCommand::new()
        .uncommitted()
        .ephemeral()
        .execute_json(&codex)
        .await?;

    println!("--- typed ---");
    println!("thread:   {:?}", result.thread_id);
    println!("comments: {}", result.result);

    // Usage is reported for a review, but as zeros: unlike an exec turn, a
    // review's turn.completed carries no real counts.
    println!("tokens:   {:?}", result.usage.and_then(|u| u.total()));

    Ok(())
}
