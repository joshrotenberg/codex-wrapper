#!/usr/bin/env bash
# Fake codex that reports what it received on stdin.
#
# Used to check that a `codex exec -` prompt is actually delivered. The
# previous implementation spawned with stdin closed, so this script would have
# read nothing and the bug was invisible from argv alone.
#
# Emits the real event vocabulary so the JSONL paths can consume it, with the
# stdin content as the agent message.
prompt=$(cat)
escaped=${prompt//\\/\\\\}
escaped=${escaped//\"/\\\"}
escaped=${escaped//$'\n'/\\n}
echo '{"type":"thread.started","thread_id":"thread_stdin"}'
echo '{"type":"turn.started"}'
echo "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"agent_message\",\"text\":\"$escaped\"}}"
echo '{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}}'
