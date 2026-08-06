#!/usr/bin/env bash
# Fake `codex exec review --json` output.
#
# Transcribed from a real codex-cli 0.145.0 run against a one-line uncommitted
# diff, with the review text shortened. Review emits the same event vocabulary
# as exec, preceded by the command_execution items the reviewer runs to read
# the diff.
#
# Two details are real and load-bearing, both verified in that run:
#   - the item discriminator is `type`, not `item_type`
#   - a review's `turn.completed` reports a usage object of all zeros
echo '{"type":"thread.started","thread_id":"019fd952-7ce9-7662-8a20-9c33c1718dca"}'
echo '{"type":"turn.started"}'
echo '{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"git diff"}}'
echo '{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"git diff","aggregated_output":"","exit_code":0,"status":"completed"}}'
echo '{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"- [P1] Keep add performing addition"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":0,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}}'
