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
# Verified against a real `codex exec --json --ephemeral` run: the event names,
# the `type` discriminator on the item, and a completed turn reporting token
# counts rather than any monetary cost. The usage object carries no
# `total_tokens`, so `TokenUsage::total` reaches 165 through its input plus
# output fallback, which is the path every real run takes.
#
# The ids and counts are synthetic. The shape is not.
echo '{"type":"thread.started","thread_id":"thread_test"}'
echo '{"type":"turn.started"}'
echo '{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"hello"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":120,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":45,"reasoning_output_tokens":0}}'
