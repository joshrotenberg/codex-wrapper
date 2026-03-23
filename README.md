# codex-wrapper

Rust tooling for the Codex CLI.

This workspace currently focuses on the base `codex-wrapper` crate, modeled after
the `claude-wrapper` crate structure and builder style.

## CI and Release

The repository includes GitHub Actions for CI, quick PR checks, dependency
audits, changelog automation, and `release-plz`-driven crates.io releases.

Expected repository secrets:

- `COMMITTER_TOKEN` for release PRs and changelog PRs
- `CARGO_REGISTRY_TOKEN` for publishing to crates.io
