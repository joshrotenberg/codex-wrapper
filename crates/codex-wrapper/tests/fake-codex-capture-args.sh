#!/usr/bin/env bash
# NUL separation preserves empty values and embedded newlines in each argv
# element. Reuse the existing real-shaped event fixture for JSON consumers.
printf '%s\0' "$@" > "$CODEX_WRAPPER_ARGS_CAPTURE"
exec /bin/bash "$(dirname "$0")/fake-codex-echo-stdin.sh"
