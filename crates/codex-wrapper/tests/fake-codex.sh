#!/usr/bin/env bash
# Fake codex binary that emits JSONL events to stdout.
# Used by streaming and session unit tests.
#
# This mirrors the event vocabulary `codex exec --json` actually emits in
# codex-cli 0.145.0. An earlier version of this fixture invented a schema
# (`{"type":"completed","result":{"text":...,"cost":...}}`) that the CLI has
# never emitted, which made every test that consumed it self-consistent and
# wrong. See #73.
#
# Verified against the CLI: the event names, and that a completed turn reports
# token counts rather than any monetary cost.
# Assumed: the exact `item.completed` layout. See the ASSUMPTIONS block in
# src/types.rs.
echo '{"type":"thread.started","thread_id":"thread_test"}'
echo '{"type":"turn.started"}'
echo '{"type":"item.completed","item":{"id":"item_0","item_type":"agent_message","text":"hello"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":120,"cached_input_tokens":0,"output_tokens":45,"reasoning_output_tokens":0,"total_tokens":165}}'
