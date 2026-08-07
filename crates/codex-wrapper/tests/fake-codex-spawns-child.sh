#!/usr/bin/env bash
# Fake codex that spawns a child of its own, the way the real CLI spawns
# subprocesses for tool use, then blocks.
#
# Writes both PIDs so a test can check that cancelling kills the whole tree
# and not just the process the wrapper spawned. That distinction is the whole
# of #78: kill_on_drop reaps the direct child only.
sleep 60 &
child=$!
{
  echo "parent=$$"
  echo "child=$child"
} > "$CODEX_WRAPPER_TEST_PIDFILE"
wait "$child"
