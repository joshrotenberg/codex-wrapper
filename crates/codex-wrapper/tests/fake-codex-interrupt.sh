#!/bin/bash
# Process-lifetime fixture, not a Codex output-schema fixture. A cooperative
# CLI must flush both pipes before cleaning up its detached tool. Shutdown
# bytes deliberately are not JSON: cancellation must no longer parse them.
set -u
pidfile="${CODEX_WRAPPER_TEST_PIDFILE:?}"
mode="${CODEX_WRAPPER_TEST_SHUTDOWN_MODE:-cooperate}"

if [ "$mode" = "cooperate" ]; then
    # Job control gives the tool its own PGID, beyond the wrapper's killpg.
    set -m
    sleep 60 &
    tool=$!
    shutdown() {
        trap '' INT TERM
        touch "$pidfile.$1"
        # Each pipe exceeds its capacity. Closed or stalled pipes prevent
        # reaching the tool cleanup below and make the regression fail.
        head -c 524288 /dev/zero || exit 97
        head -c 524288 /dev/zero >&2 || exit 98
        kill -TERM "$tool"
        wait "$tool" 2>/dev/null || true
        touch "$pidfile.cleaned"
        exit 0
    }
    trap 'shutdown int' INT
    trap 'shutdown term' TERM
else
    # Same-group descendant so the hard fallback has authority to stop it.
    trap 'touch "$pidfile.int"' INT
    trap 'touch "$pidfile.term"' TERM
    (trap '' INT TERM; exec sleep 60) &
    tool=$!
fi
printf 'parent=%s\nchild=%s\n' "$$" "$tool" > "$pidfile"
touch "$pidfile.ready"
while kill -0 "$tool" 2>/dev/null; do
    wait "$tool" || true
done
