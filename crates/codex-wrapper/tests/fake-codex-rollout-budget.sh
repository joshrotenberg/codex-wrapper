#!/usr/bin/env bash
# Captured codex-cli 0.145.0 JSONL shape for native rollout-budget exhaustion.
# Identifiers are synthetic; event order, terminal message, missing usage, and
# non-zero exit match the paid contract run made for codex-wrapper#117.
printf '%s\n' '{"type":"thread.started","thread_id":"00000000-0000-0000-0000-000000000117"}'
printf '%s\n' '{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Under-development features enabled: rollout_budget."}}'
printf '%s\n' '{"type":"turn.started"}'
printf '%s\n' '{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"ok"}}'
printf '%s\n' '{"type":"error","message":"shared rollout token budget exhausted"}'
printf '%s\n' '{"type":"turn.failed","error":{"message":"shared rollout token budget exhausted"}}'
exit 1
