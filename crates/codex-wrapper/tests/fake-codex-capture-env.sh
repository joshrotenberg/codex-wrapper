#!/usr/bin/env bash
# Fake codex that records the complete child environment before emitting
# verified JSONL event shapes. It uses absolute utility paths so a cleared
# environment does not need PATH in order for the fixture itself to run.
set -eu

: "${CODEX_WRAPPER_ENV_CAPTURE:?capture path is required}"
/usr/bin/env > "${CODEX_WRAPPER_ENV_CAPTURE}"

for arg in "$@"; do
    if [ "${arg}" = "-" ]; then
        /bin/cat >/dev/null
        break
    fi
done

echo '{"type":"thread.started","thread_id":"thread_env"}'
echo '{"type":"turn.started"}'
echo '{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"ok"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}}'
