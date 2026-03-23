# Changelog

All notable changes to this project will be documented in this file.

## [unreleased]

## [0.1.1](https://github.com/joshrotenberg/codex-wrapper/compare/v0.1.0...v0.1.1) - 2026-03-23

### Added

- *(exec)* add Debug impl for CommandOutput that redacts long stdout/stderr ([#10](https://github.com/joshrotenberg/codex-wrapper/pull/10))
- add missing CLI commands, integration tests, and docs ([#2](https://github.com/joshrotenberg/codex-wrapper/pull/2))

### Fixed

- *(exec)* validate non-empty model name in ExecCommand::model() ([#11](https://github.com/joshrotenberg/codex-wrapper/pull/11))

### Other

- *(error)* add Display impl test coverage for each Error variant closes #4 ([#7](https://github.com/joshrotenberg/codex-wrapper/pull/7))
- note SandboxMode and ApprovalPolicy defaults in crate-level docs closes #3 ([#5](https://github.com/joshrotenberg/codex-wrapper/pull/5))

