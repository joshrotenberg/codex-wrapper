#!/usr/bin/env bash
# Fake codex binary that emits invalid JSON to test error handling.
echo '{"type":"thread.started"}'
echo '{this is not valid json}'
