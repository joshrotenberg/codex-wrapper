# History fixtures

Captured with `codex-cli 0.163.0` on 2026-10-20 from an isolated app-server
process and a fresh `CODEX_HOME`. No personal sessions were read. A loopback
Responses endpoint returned an intentional HTTP 400 for every request; no model
inference or account credentials were used.

For each `historyMode` (`legacy`, `paginated`), the capture initialized with
`experimentalApi: true`, started a persistent thread, and submitted two turns
containing `synthetic offline history fixture`. Both turns failed with the
fixture error and materialized a rollout under `sessions/YYYY/MM/DD/`.

These are reduced provider-produced records, not complete rollouts. They retain
session metadata relevant to discovery, user-message events, and completion
events. Other records and metadata fields (including base instructions and
runtime paths) were removed. IDs and the working directory were replaced with
synthetic values. Envelope structure, item casing, ordinals, and remaining
payload fields match the capture. In particular, disk `UserMessage` is not the
RPC history item's `userMessage`.

The same capture checked the provider view:

| Request | Materialized legacy | Materialized paginated |
|---|---|---|
| `thread/read`, `includeTurns: true` | succeeds | succeeds |
| `thread/turns/list`, `limit: 1`, then returned `nextCursor` | two pages | two pages |
| `thread/items/list`, `limit: 1`, then returned `nextCursor` | unsupported-method error | two pages |

Fresh empty threads were not yet materialized; querying them did not establish
the same availability. These fixtures qualify disk discovery and raw payload
preservation for these two modes. They do not prove complete conversation
reconstruction, all item kinds, archive/database discovery, or recovery from
pruning. The reader deliberately does not expose envelope ordinals or normalize
events into the provider's paginated turn/item view.
