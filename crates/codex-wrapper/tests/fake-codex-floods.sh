#!/usr/bin/env bash
# Fake codex that prints far more than any sane capture ceiling.
#
# Deliberately emits one enormous line rather than many small ones: a bound
# that only noticed at line boundaries would pass a line-oriented fixture and
# still hold the whole stream in memory.
#
# CODEX_WRAPPER_TEST_FLOOD_STREAM picks which stream floods, so the ceiling
# can be shown to cover stderr as well as stdout.
stream=${CODEX_WRAPPER_TEST_FLOOD_STREAM:-stdout}
chunk=$(head -c 8192 /dev/zero | tr '\0' 'x')
i=0
while [ $i -lt 128 ]; do
  if [ "$stream" = stderr ]; then
    printf '%s' "$chunk" >&2
  else
    printf '%s' "$chunk"
  fi
  i=$((i + 1))
done
