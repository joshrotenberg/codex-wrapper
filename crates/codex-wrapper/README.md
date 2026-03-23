# codex-wrapper

Type-safe Rust wrapper around the Codex CLI with a two-layer builder API.

```rust
use codex_wrapper::{Codex, CodexCommand, ExecCommand, SandboxMode};

# async fn example() -> codex_wrapper::Result<()> {
let codex = Codex::builder().build()?;
let output = ExecCommand::new("summarize this repository")
    .sandbox(SandboxMode::WorkspaceWrite)
    .skip_git_repo_check()
    .execute(&codex)
    .await?;

println!("{}", output.stdout);
# Ok(())
# }
```
