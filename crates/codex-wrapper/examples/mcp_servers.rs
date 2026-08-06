//! Manage MCP servers through the CLI: add, list, inspect, remove.
//!
//! ```sh
//! cargo run --example mcp_servers
//! ```
//!
//! This registers a server named `example-echo` and removes it again, so it
//! leaves your codex config as it found it.

use codex_wrapper::{
    Codex, CodexCommand, McpAddCommand, McpGetCommand, McpListCommand, McpRemoveCommand,
};

const NAME: &str = "example-echo";

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;

    // Typed listing, rather than parsing the human-readable table.
    let servers = McpListCommand::new().execute_json(&codex).await?;
    println!("before: {servers}");

    McpAddCommand::stdio(NAME, "bash")
        .arg("-c")
        .arg("cat")
        .env("EXAMPLE", "1")
        .execute(&codex)
        .await?;
    println!("\nadded {NAME}");

    let detail = McpGetCommand::new(NAME).execute(&codex).await?;
    println!("\n{}", detail.stdout);

    McpRemoveCommand::new(NAME).execute(&codex).await?;
    println!("removed {NAME}");

    Ok(())
}
