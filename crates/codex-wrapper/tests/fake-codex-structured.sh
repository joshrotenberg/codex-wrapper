#!/usr/bin/env bash
# Deterministic stand-in for `codex exec --json --output-schema FILE`, shared in
# shape with genagent/codex_wrapper_ex test/fixtures/fake_codex_structured. It
# never execs a real CLI, reads no credentials and makes no network calls.
#
# FAKE_CODEX_SCENARIO selects the output. FAKE_CODEX_CALLS, when set, gets
# one line per invocation with the argv, followed by a `schema:` line with
# the contents of the --output-schema file as seen while the process ran.
#
# All scenarios are synthetic. Event names follow the thread event
# declarations of @openai/codex-sdk 0.164.0; values are not captured from a
# provider.

if [ -n "$FAKE_CODEX_CALLS" ]; then
  printf '%s\n' "$*" >> "$FAKE_CODEX_CALLS"
  schema=''
  previous=''
  for arg in "$@"; do
    if [ "$previous" = "--output-schema" ]; then
      schema="$arg"
    fi
    previous="$arg"
  done
  if [ -n "$schema" ] && [ -f "$schema" ]; then
    printf 'schema:%s\n' "$(cat "$schema")" >> "$FAKE_CODEX_CALLS"
  else
    printf 'schema:<missing>\n' >> "$FAKE_CODEX_CALLS"
  fi
fi

started='{"type":"thread.started","thread_id":"fake-thread"}
{"type":"turn.started"}'
completed='{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1}}'

message() {
  printf '{"type":"item.completed","item":{"id":"%s","type":"agent_message","text":%s}}\n' "$1" "$2"
}

case "$FAKE_CODEX_SCENARIO" in
  object)
    echo "$started"
    message item_0 '"{\"name\":\"Ada Lovelace\"}"'
    echo "$completed"
    ;;
  array)
    echo "$started"
    message item_0 '"[{\"name\":\"Ada\"},{\"name\":\"Grace\"}]"'
    echo "$completed"
    ;;
  multiple_messages)
    echo "$started"
    message item_0 '"{\"name\":\"draft\"}"'
    printf '{"type":"item.completed","item":{"id":"item_1","type":"reasoning","text":"thinking"}}\n'
    message item_2 '"{\"name\":\"final\"}"'
    echo "$completed"
    ;;
  transient_error)
    echo "$started"
    printf '{"type":"error","message":"synthetic reconnect notice"}\n'
    message item_0 '"{\"name\":\"Ada\"}"'
    echo "$completed"
    ;;
  missing)
    echo "$started"
    echo "$completed"
    ;;
  null)
    echo "$started"
    message item_0 '"null"'
    echo "$completed"
    ;;
  invalid_json)
    echo "$started"
    message item_0 '"{\"name\":"'
    echo "$completed"
    ;;
  scalar)
    echo "$started"
    message item_0 '"\"Ada Lovelace\""'
    echo "$completed"
    ;;
  turn_failed)
    echo "$started"
    message item_0 '"{\"name\":\"partial\"}"'
    printf '{"type":"turn.failed","error":{"message":"synthetic failure"}}\n'
    exit 1
    ;;
  turn_failed_exit_zero)
    # Synthetic: decoding must reject a failed turn even without a non-zero exit.
    echo "$started"
    message item_0 '"{\"name\":\"partial\"}"'
    printf '{"type":"turn.failed","error":{"message":"synthetic failure"}}\n'
    ;;
  error_only)
    echo "$started"
    printf '{"type":"error","message":"synthetic stream error"}\n'
    exit 1
    ;;
  no_terminal)
    echo "$started"
    message item_0 '"{\"name\":\"Ada\"}"'
    ;;
  success_then_exit)
    echo "$started"
    message item_0 '"{\"name\":\"Ada\"}"'
    echo "$completed"
    exit 1
    ;;
  crash)
    printf 'fake codex: synthetic crash\n' >&2
    exit 2
    ;;
  *)
    printf 'fake codex: unknown FAKE_CODEX_SCENARIO %s\n' "$FAKE_CODEX_SCENARIO" >&2
    exit 64
    ;;
esac
