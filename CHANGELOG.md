# Changelog

All notable changes to this project will be documented in this file.

## [0.4.1](https://github.com/joshrotenberg/codex-wrapper/compare/v0.4.0...v0.4.1) - 2026-08-17

### Fixed

- implement awaited cancellation settlement ([#126](https://github.com/joshrotenberg/codex-wrapper/pull/126))

## [0.4.0](https://github.com/joshrotenberg/codex-wrapper/compare/v0.3.1...v0.4.0) - 2026-08-17

### Added

- support resume prompts over stdin ([#124](https://github.com/joshrotenberg/codex-wrapper/pull/124))
- *(process)* support cleared child environments ([#121](https://github.com/joshrotenberg/codex-wrapper/pull/121))
- *(exec)* type the native rollout budget configuration ([#118](https://github.com/joshrotenberg/codex-wrapper/pull/118))

## [0.3.1](https://github.com/joshrotenberg/codex-wrapper/compare/v0.3.0...v0.3.1) - 2026-08-08

### Added

- support env-backed HTTP MCP headers ([#114](https://github.com/joshrotenberg/codex-wrapper/pull/114))

### Fixed

- deliver JSONL events before process exit ([#115](https://github.com/joshrotenberg/codex-wrapper/pull/115))

## [0.3.0](https://github.com/joshrotenberg/codex-wrapper/compare/v0.2.0...v0.3.0) - 2026-08-07

### Added

- catch up to codex-cli 0.147.0 ([#110](https://github.com/joshrotenberg/codex-wrapper/pull/110))
- make the run's own process group opt-out ([#109](https://github.com/joshrotenberg/codex-wrapper/pull/109))
- kill the whole process group on cancellation ([#106](https://github.com/joshrotenberg/codex-wrapper/pull/106))
- per-run MCP server config via -c overrides ([#105](https://github.com/joshrotenberg/codex-wrapper/pull/105))
- typed accessors for stream items, and record the absent deltas ([#104](https://github.com/joshrotenberg/codex-wrapper/pull/104))
- gate the safety-bypass flags behind an explicit opt-in ([#103](https://github.com/joshrotenberg/codex-wrapper/pull/103))
- *(history)* read-side access to on-disk codex session logs ([#102](https://github.com/joshrotenberg/codex-wrapper/pull/102))
- *(config)* read-side access to ~/.codex/config.toml ([#101](https://github.com/joshrotenberg/codex-wrapper/pull/101))
- *(auth)* detect which auth strategy the codex CLI will use ([#100](https://github.com/joshrotenberg/codex-wrapper/pull/100))
- classify command failures into typed error variants ([#99](https://github.com/joshrotenberg/codex-wrapper/pull/99))
- *(budget)* cumulative token budget tracking across turns ([#98](https://github.com/joshrotenberg/codex-wrapper/pull/98))
- tracing spans around command execution ([#97](https://github.com/joshrotenberg/codex-wrapper/pull/97))
- to_command_string() for previewing the argv a builder will spawn ([#93](https://github.com/joshrotenberg/codex-wrapper/pull/93))
- add ReviewCommand::execute_json, and correct the item schema against real output ([#79](https://github.com/joshrotenberg/codex-wrapper/pull/79))
- session cost accumulation and streaming turns ([#72](https://github.com/joshrotenberg/codex-wrapper/pull/72))
- report the installed CLI against a CI-backed tested-version range ([#71](https://github.com/joshrotenberg/codex-wrapper/pull/71))
- add missing ignore and output-schema flags to exec resume and exec review ([#68](https://github.com/joshrotenberg/codex-wrapper/pull/68))

### Fixed

- [**breaking**] deliver the prompt for ExecCommand::from_stdin ([#96](https://github.com/joshrotenberg/codex-wrapper/pull/96))
- kill spawned codex processes when the future is dropped ([#77](https://github.com/joshrotenberg/codex-wrapper/pull/77))
- parse the event schema the CLI actually emits ([#74](https://github.com/joshrotenberg/codex-wrapper/pull/74))
- stop emitting invalid approval, search, and full-auto arguments ([#64](https://github.com/joshrotenberg/codex-wrapper/pull/64))

### Other

- lint every target without default features ([#108](https://github.com/joshrotenberg/codex-wrapper/pull/108))
- add an examples/ directory ([#95](https://github.com/joshrotenberg/codex-wrapper/pull/95))
- add LICENSE-MIT and LICENSE-APACHE ([#91](https://github.com/joshrotenberg/codex-wrapper/pull/91))
- record why codex review is not wrapped, and guard the decision ([#69](https://github.com/joshrotenberg/codex-wrapper/pull/69))
- check emitted flags and config keys against the installed CLI ([#67](https://github.com/joshrotenberg/codex-wrapper/pull/67))

## [0.2.0] - 2026-07-24

### Bug Fixes

- Catch codex exec family up to codex-cli 0.145.0 
- Correct sandbox command for codex-cli 0.145.0, add real integration tests 

### Features

- Add typed QueryResult and make Error non_exhaustive for trait parity 
- Add archive/delete/unarchive session-lifecycle commands 
- Add doctor and update commands 
- Add plugin command group 
- Close exec-adjacent flag drift on fork, resume, mcp add, login 

### Miscellaneous

- Bump tokio from 1.51.1 to 1.52.3 in the tokio-ecosystem group 
- Bump actions/checkout from 6 to 7 
- Bump which from 8.0.2 to 8.0.5 
- Bump the serde-ecosystem group across 1 directory with 2 updates 
- Bump thiserror from 2.0.18 to 2.0.19 
- Release v0.2.0 

## [0.1.2] - 2026-04-13

### Bug Fixes

- Gate streaming tests behind cfg(unix) for Windows CI 

### Documentation

- *(command)* Add doc comments to RawCommand and VersionCommand 
- *(exec)* Add doc comments to ExecCommand and ExecResumeCommand builder methods 
- Consolidate README as primary project documentation 

### Features

- Add Session struct for multi-turn state management 
- Add execute_json_lines to ExecResumeCommand 
- Add streaming support via callback  

### Miscellaneous

- Update changelog 
- Bump tokio from 1.50.0 to 1.51.0 in the tokio-ecosystem group 
- Bump tokio from 1.51.0 to 1.51.1 in the tokio-ecosystem group 
- Release v0.1.2 

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


