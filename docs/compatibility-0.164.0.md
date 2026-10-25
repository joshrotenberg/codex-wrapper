# Native CLI compatibility audit

Verified on macOS arm64, 2026-10-24. Target: 0.164.0; retained floor: 0.145.0.
Tracks #170. Native binaries came from exact npm package versions in isolated
task directories, with install scripts disabled. No global binary upgrade was used.

## Native contract results

- 0.145.0: test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.12s
- 0.164.0: test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.86s

The complete existing contract suite passed at both endpoints. Pinned PR CI and
the declared tested maximum now agree. Help/config/schema and local protocol
contracts do not prove model behavior or every upstream semantic.

## Local repository checks

- unit-all: test result: ok. 368 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.71s
- unit-minimal: test result: ok. 184 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.65s
- doc: test result: ok. 29 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

Required format, feature/build and Clippy checks also passed. Existing controlled
fixtures cover schema/results, sessions, interruption and process settlement.
No new authenticated native smoke result is claimed. Historical native-capture
versions elsewhere in the documentation remain unchanged.
