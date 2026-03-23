# codex-wrapper

Rust tooling for the [Codex CLI](https://github.com/openai/codex).

[![CI](https://github.com/joshrotenberg/codex-wrapper/actions/workflows/ci.yml/badge.svg)](https://github.com/joshrotenberg/codex-wrapper/actions/workflows/ci.yml)

## Crates

| Crate | Description |
|-------|-------------|
| [`codex-wrapper`](crates/codex-wrapper/) | Type-safe Codex CLI wrapper with builder-pattern API |

## Quick Start

```bash
cargo add codex-wrapper
```

```rust
use codex_wrapper::{Codex, CodexCommand, ExecCommand, SandboxMode};

#[tokio::main]
async fn main() -> codex_wrapper::Result<()> {
    let codex = Codex::builder().build()?;
    let output = ExecCommand::new("summarize this repository")
        .sandbox(SandboxMode::WorkspaceWrite)
        .ephemeral()
        .execute(&codex)
        .await?;
    println!("{}", output.stdout);
    Ok(())
}
```

See the [crate README](crates/codex-wrapper/README.md) for full documentation.

## CI and Release

GitHub Actions workflows handle CI, dependency audits, changelog automation,
and `release-plz`-driven crates.io releases.

Expected repository secrets:

- `COMMITTER_TOKEN` -- for release PRs and changelog PRs
- `CARGO_REGISTRY_TOKEN` -- for publishing to crates.io

## License

MIT OR Apache-2.0
