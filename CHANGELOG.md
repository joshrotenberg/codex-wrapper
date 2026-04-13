# Changelog

All notable changes to this project will be documented in this file.

## [0.1.2](https://github.com/joshrotenberg/codex-wrapper/compare/v0.1.1...v0.1.2) - 2026-04-13

### Added

- add streaming support via callback (closes #20) ([#24](https://github.com/joshrotenberg/codex-wrapper/pull/24))
- add execute_json_lines to ExecResumeCommand ([#19](https://github.com/joshrotenberg/codex-wrapper/pull/19))
- add Session struct for multi-turn state management ([#25](https://github.com/joshrotenberg/codex-wrapper/pull/25))

### Fixed

- gate streaming tests behind cfg(unix) for Windows CI ([#27](https://github.com/joshrotenberg/codex-wrapper/pull/27))

### Other

- *(exec)* add doc comments to ExecCommand and ExecResumeCommand builder methods ([#17](https://github.com/joshrotenberg/codex-wrapper/pull/17))
- *(command)* add doc comments to RawCommand and VersionCommand ([#15](https://github.com/joshrotenberg/codex-wrapper/pull/15))

## [0.1.1] - 2026-03-23

### Bug Fixes

- *(exec)* Validate non-empty model name in ExecCommand::model() 

### Documentation

- Note SandboxMode and ApprovalPolicy defaults in crate-level docs closes #3 

### Features

- Add missing CLI commands, integration tests, and docs 
- *(exec)* Add Debug impl for CommandOutput that redacts long stdout/stderr 

### Miscellaneous

- Bump peter-evans/create-pull-request from 7 to 8 
- Release v0.1.1 

### Testing

- *(error)* Add Display impl test coverage for each Error variant closes #4 


