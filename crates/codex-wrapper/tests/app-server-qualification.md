# Attached-server qualification

Native probes used `codex-cli 0.163.0` on macOS on 2026-10-20, an isolated
`CODEX_HOME`, a temporary workspace, and an explicitly selected Unix socket.
They used two protocol clients, not a private TUI process. No real model or
account credentials were used. A loopback Responses fixture either held a
request open, returned an intentional failure, or emitted one deterministic
tool call whose approval was cancelled without running the command.

The [provider transport contract](https://learn.chatgpt.com/docs/app-server)
specifies WebSocket HTTP Upgrade over Unix sockets, then one JSON-RPC message
per text frame. A plain JSONL client through `app-server proxy --sock` failed
the native handshake: that proxy forwards bytes without translating framing.
Adding WebSocket framing worked, but abrupt server loss could leave the byte
proxy's stdout open. Direct Unix WebSocket connections detected EOF promptly.
This is why the wrapper attaches directly rather than relying on that proxy.

## Observed native behavior

| Scenario | Observation |
|---|---|
| Two independently initialized connections | Both used the same server |
| `thread/resume` for a materialized running thread | Joined the active thread |
| `turn/steer` from the observer, with the active `expectedTurnId` | Accepted |
| Observer disconnect during a held turn | Server stayed alive |
| New connection, initialize, resume, then interrupt | Accepted; no automatic replay was needed |
| Original client disconnect after interruption | Server stayed alive; another client could read the thread |
| Server terminated while `command/exec` was pending | Direct Unix connection reached EOF and the waiter failed |
| Observer joins while a command approval is pending | Received the pending `item/commandExecution/requestApproval` |
| Observer answers that approval with `cancel` | Original client received `turn/completed` with status `interrupted` |
| Observer disconnects while approval is pending, then a fresh connection resumes | Pending approval was delivered to the new connection; its `cancel` answer interrupted the turn |

An empty thread was not yet materialized and an immediate observer resume
returned `no rollout found`. Rejoin was verified after turn start and while an
approval was pending. A host must handle that distinction rather than assuming
every newly returned thread ID can immediately be resumed by another client.

## Boundaries

These observations qualify this version and these schedules. They do not
establish which of simultaneous competing answers wins, transfer ownership of
the shared server, adopt an unrelated private TUI server, or guarantee recovery
of arbitrary running commands. Reconnection means a fresh transport handshake
and explicit thread/subscription/approval reconciliation. Mutation outcomes
around disconnection can be uncertain; the wrapper does not replay requests.
Transport closure does not itself promise native turn interruption.

The Rust unit suite separately tests wrapper ownership, pending-request
settlement, queue and frame bounds, and connection teardown with local peers.
Synthetic peer schedules qualify wrapper behavior, not additional native race
semantics. App-server and remote transports remain experimental.
