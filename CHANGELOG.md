# Changelog

All notable changes to this project will be documented in this file.

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


