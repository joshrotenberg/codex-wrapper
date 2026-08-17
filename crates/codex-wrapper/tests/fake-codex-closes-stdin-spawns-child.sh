#!/usr/bin/env bash
# Records a process tree, then closes stdin while continuing to run. A large
# wrapper prompt therefore fails to write while both processes are alive.
sleep 60 </dev/null &
child=$!
{
  echo "parent=$$"
  echo "child=$child"
} > "$CODEX_WRAPPER_TEST_PIDFILE"
exec 0<&-
wait "$child"
