#!/usr/bin/env bash
# Fake codex that spawns a child of its own, the way the real CLI spawns
# subprocesses for tool use, then blocks.
#
# Writes both PIDs so a test can check that cancelling kills the whole tree
# and not just the process the wrapper spawned. That distinction is the whole
# of #78: kill_on_drop reaps the direct child only.
if [ "${CODEX_WRAPPER_TEST_IGNORE_TERM:-}" = 1 ]; then
  trap 'touch "$CODEX_WRAPPER_TEST_PIDFILE.term"; exit 0' TERM
  sh -c 'trap "" TERM; touch "$CODEX_WRAPPER_TEST_PIDFILE.ready"; exec sleep 60' &
else
  sleep 60 &
fi
child=$!
if [ "${CODEX_WRAPPER_TEST_IGNORE_TERM:-}" = 1 ]; then
  # Do not announce readiness until the descendant has installed its trap.
  while [ ! -f "$CODEX_WRAPPER_TEST_PIDFILE.ready" ]; do sleep 0.01; done
fi
{
  echo "parent=$$"
  echo "child=$child"
} > "$CODEX_WRAPPER_TEST_PIDFILE"
wait "$child"
