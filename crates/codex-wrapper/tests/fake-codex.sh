#!/usr/bin/env bash
# Fake codex binary that emits JSONL events to stdout.
# Used by streaming unit tests.
echo '{"type":"thread.started","session_id":"sess_test","thread_id":"thread_test"}'
echo '{"type":"message.created","role":"assistant","content":[{"type":"text","text":"hello"}]}'
echo '{"type":"completed","result":{"text":"hello","cost":0.001}}'
