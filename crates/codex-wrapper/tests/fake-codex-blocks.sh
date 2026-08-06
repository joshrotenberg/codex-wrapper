#!/usr/bin/env bash
# Fake codex that records its own PID and then blocks until it is killed.
#
# Used by the process-lifetime tests to check that dropping the future kills
# the spawned process. `exec` replaces this shell with `sleep`, so the PID
# written here stays the PID tokio spawned, which is the one kill_on_drop
# reaps. Without `exec` the test would be watching the wrong process.
echo "$$" >"$CODEX_WRAPPER_TEST_PIDFILE"
exec sleep 60
