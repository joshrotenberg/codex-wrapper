# Changelog

All notable changes to this project will be documented in this file.

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


