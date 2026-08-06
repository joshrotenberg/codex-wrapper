# AGENTS.md

Guidance for AI assistants working on this repo.

## What this is

`codex-wrapper` is a type-safe Rust wrapper around the Codex CLI. Each subcommand is a builder
that produces typed output, executed via tokio. It is the sister crate to
[`claude-wrapper`](https://github.com/joshrotenberg/claude-wrapper), and the two are being
aligned so a downstream abstraction can target either through one trait. When you change a shape
that exists on both sides, check what the other crate calls it before inventing a name.

## Layout

This is a **workspace**, not a single crate. The library lives at `crates/codex-wrapper`, so:

- `cargo` commands run from the repo root but resolve the package as `-p codex-wrapper`.
- `env!("CARGO_MANIFEST_DIR")` in a test points at `crates/codex-wrapper`, not the repo root.
- Files that must ship in the published `.crate` belong under `crates/codex-wrapper`. Root-level
  files are not packaged, which is why `LICENSE-MIT` and `LICENSE-APACHE` exist in both places.

```
crates/codex-wrapper/
  src/
    lib.rs           Codex client + builder
    command/         one builder per subcommand, all implementing CodexCommand
    exec.rs          process execution: spawn, timeout, retry, argv assembly
    streaming.rs     JSONL streaming via piped stdout        (json feature)
    session.rs       multi-turn sessions over exec resume    (json feature)
    types.rs         JSONL event types, enums, version parsing
    version.rs       tested CLI version range
  tests/
    contract.rs      drift guard, checks emitted flags against the real CLI
    integration.rs   end-to-end, needs a real authenticated codex
    fake-codex*.sh   fixtures for unit tests
```

## Build and test

Before any push:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib --all-features
cargo test --lib --no-default-features
cargo test --doc --all-features
```

MSRV is 1.90.0 and the edition is 2024. Let-chains are available; `cargo check --all-features`
under 1.90 is a CI job.

### Where tests actually run

| Suite | Runs on every push |
|---|---|
| unit tests in `src/**`, including the fake-codex ones | yes, via `--lib` |
| doc tests | yes |
| `tests/contract.rs` | only in the `contract` job, with a real CLI and `--ignored` |
| `tests/integration.rs` | no, every test is `#[ignore]` and needs auth |

A test that must run in CI belongs in `src/`. Putting it in `tests/` without `#[ignore]` is
possible but that directory is for suites needing a real binary.

Unit tests that need a process drive a bash fixture through the client:

```rust
Codex::builder().binary("/bin/bash").arg(script_path).build()
```

## The two drift guards

The wrapper silently drifted from codex-cli 0.116 to 0.145 while every test passed. Both
mechanisms below exist because of that, and both are easy to leave stale.

**`tests/contract.rs`** builds a maximal command from each builder and checks every emitted flag
against `codex <sub> --help`, probing config keys with `--strict-config`. Adding a flag or a
config key to a builder means adding it to the contract check **in the same change**.

**`TESTED_CLI_VERSION_MIN` / `MAX`** in `src/version.rs` declare the tested range. A unit test
asserts the CI matrix in `.github/workflows/ci.yml` contains both bounds, so bumping one without
the other fails. `contract-latest.yml` additionally runs the contract check against `@latest`
weekly, non-blocking, so upstream removals surface before a user reports them.

## Never invent a schema

`types.rs` opens with a block splitting what is **verified** about the JSONL event schema from
what is **assumed**. It is there because a previous parser matched an event shape the CLI has
never emitted, against a fixture that invented the same shape, so every test passed while real
runs produced empty results (#73).

The rules that follow from it:

- Fixtures under `tests/fake-codex*.sh` are transcribed from real output. Synthetic ids and
  counts are fine. Synthetic **shapes** are not.
- To confirm a shape, capture a run and read it:
  ```bash
  codex exec --json --ephemeral --skip-git-repo-check "reply with: ok" > turn.jsonl
  ```
- When a parser tolerates several layouts because the real one is unconfirmed, say so at the
  point of tolerance and keep a test per layout. Silent tolerance reads as knowledge.
- Move a claim from Assumed to Verified only with a captured run behind it, and correct the
  block in the same change.

## Feature flags

`default = ["json"]`. The `json` feature gates `session`, `streaming`, and the event types in
`types.rs` (`JsonLineEvent`, `QueryResult`, `TokenUsage`).

Test code needs the same gating as the code it tests. A test naming a json-only type compiles
under `--all-features` and breaks the `--no-default-features` build, which is why both test
commands are in the list above and in CI (#80).

## Code conventions

- Conventional-commit prefixes on commits and PR titles.
- No em dashes anywhere, in code comments, docs, commit messages, or PR bodies.
- No AI attribution trailers or "generated with" footers.
- Comments explain why, not what. A comment restating the line above it is noise; one recording
  a CLI behavior, a rejected alternative, or the reason a workaround exists is not.
- Public items carry rustdoc. `cargo doc` runs with `-D warnings` in CI, so a broken intra-doc
  link fails the build. Link across modules with full paths (`crate::types::QueryResult`) when
  the type is not in scope.
- `README.md` and `crates/codex-wrapper/README.md` are byte-identical copies. Edit the crate one
  and copy it to the root.

## Git workflow

Feature branch, draft PR opened before the work with the plan as its body, then commits. Mark
ready once CI is green. Never commit to `main`. Squash merge.

PR bodies state what changed and why, including deviations from the issue and anything verified
against the real CLI. If a change is larger than the issue asked for, say so and say why.

## Release process

`release-plz` handles version bumps and publishing. `cliff.toml` plus
`.github/workflows/changelog.yml` generate the changelog from conventional commits.

## What is deliberately not wrapped

- **Top-level `codex review`.** Same command as `codex exec review` but accepting a strict subset
  of its flags, missing `--json` among ten others, so a builder on it could not offer typed
  output. The rationale is on `ReviewCommand` and a contract check guards it.
- **`app-server`, `remote-control`, `app`, `debug`, `exec-server`.** Interactive or experimental,
  no clear programmatic use.
- **`cloud`.** Held while it is experimental upstream (#47).
- **Duplex or conversation mode.** The codex CLI is exec-oneshot plus resume with no
  streaming-stdin equivalent, so this stays a `claude-wrapper` capability rather than being
  forced into the shared trait.

Use `RawCommand` for anything unwrapped rather than adding a builder for it.

## What to avoid

- Adding a builder flag without a contract check for it.
- Writing a fixture from what the CLI plausibly emits rather than from what it did emit.
- Asserting a monetary cost anywhere. The CLI reports token counts and no cost; converting needs
  a price table it does not provide, and a hardcoded one would go stale silently.
- Editing only one of the two README copies.
- Assuming a root-level file ships in the published crate.
