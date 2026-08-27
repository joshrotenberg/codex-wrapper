# Changelog

All notable changes to this project will be documented in this file.

## [0.4.3] - 2026-08-27

### Features

- *(exec)* Bound captured child output by bytes  

### Miscellaneous

- Release v0.4.3 

## [0.4.2] - 2026-08-27

### Features

- *(process)* Implement durable worker containment 
- *(json)* Classify upstream API request rejections 

### Miscellaneous

- Start durable worker containment 
- Release v0.4.2 

## [0.4.1] - 2026-08-17

### Bug Fixes

- Implement awaited cancellation settlement 

### Miscellaneous

- Start awaited cancellation settlement 
- Release v0.4.1 

## [0.4.0] - 2026-08-17

### Features

- *(exec)* Type the native rollout budget configuration 
- *(process)* Support cleared child environments 
- Support resume prompts over stdin 

### Miscellaneous

- Bump thiserror from 2.0.19 to 2.0.20 
- Bump libc from 0.2.183 to 0.2.189 
- Release v0.4.0 

## [0.3.1] - 2026-08-09

### Bug Fixes

- Deliver JSONL events before process exit 

### Features

- Support env-backed HTTP MCP headers 

### Miscellaneous

- Release v0.3.1 

## [0.3.0] - 2026-08-07

### Bug Fixes

- Stop emitting invalid approval, search, and full-auto arguments 
- Parse the event schema the CLI actually emits 
- Kill spawned codex processes when the future is dropped 
- [**breaking**] Deliver the prompt for ExecCommand::from_stdin 

### Documentation

- Record why codex review is not wrapped, and guard the decision 
- Add AGENTS.md and CLAUDE.md 
- Add an examples/ directory 

### Features

- Add missing ignore and output-schema flags to exec resume and exec review 
- Report the installed CLI against a CI-backed tested-version range 
- Session cost accumulation and streaming turns 
- Add ReviewCommand::execute_json, and correct the item schema against real output 
- To_command_string() for previewing the argv a builder will spawn 
- Tracing spans around command execution 
- *(budget)* Cumulative token budget tracking across turns 
- Classify command failures into typed error variants 
- *(auth)* Detect which auth strategy the codex CLI will use 
- *(config)* Read-side access to ~/.codex/config.toml 
- *(history)* Read-side access to on-disk codex session logs 
- Gate the safety-bypass flags behind an explicit opt-in 
- Typed accessors for stream items, and record the absent deltas 
- Per-run MCP server config via -c overrides 
- Kill the whole process group on cancellation 
- Make the run's own process group opt-out 
- Catch up to codex-cli 0.147.0 

### Miscellaneous

- Bump tokio from 1.52.3 to 1.53.1 in the tokio-ecosystem group 
- Bump actions/setup-node from 6 to 7 
- Add LICENSE-MIT and LICENSE-APACHE 
- Compile and run the tests/ directory in the test job 
- Lint every target without default features 
- Update changelog 
- Release v0.3.0 

### Testing

- Check emitted flags and config keys against the installed CLI 

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


