# Crony Corp architecture

## Product invariant

The office is a projection of authoritative operational state. Closing a browser or desktop
window must not terminate an active agent run.

## Three planes

### Experience plane

- React web client
- future Tauri desktop shell
- `crony` CLI

Clients display state and send authenticated commands. They do not supervise agent processes.

### Collaboration and control plane

The Rust/Axum server owns:

- Corps and membership
- rooms
- missions and tasks
- control leases
- run assignments
- the immutable event journal
- real-time WebSocket delivery
- policy and approvals

Postgres is the authoritative data store.

### Execution plane

Runner daemons:

- connect outbound to the server
- advertise capabilities
- accept leased run assignments
- create isolated workspaces
- spawn and supervise child processes
- stream structured lifecycle events
- upload artifact metadata and evidence

The server never executes an agent shell command.

## Current vertical slice

```text
React browser
    | REST + WebSocket
    v
crony-server ------ Postgres
    |
    | bidirectional WebSocket
    v
crony-runner
    |
    | child process
    v
scripts/fake-agent.mjs ---> output/runner/<run-id>/result.md
```

The deterministic fake-agent adapter is intentional. It proves the complete execution contract
before provider-specific behavior is introduced.

## State and events

Configuration and current state use relational tables. Every meaningful mutation also writes an
immutable event in the same transaction.

The event envelope contains:

- Corp scope
- actor attribution
- aggregate identity and version
- correlation and causation IDs
- idempotency key
- visibility
- structured payload
- server timestamp and sequence

Runner events have independent UUIDs. Replaying the same runner event is a no-op because
`(corp_id, idempotency_key)` is unique.

## Real-time delivery

The server publishes committed domain events to connected browser clients. Clients reconnect with
an actor identity and `after_seq` cursor. The server verifies Corp membership before upgrading the
connection, subscribes to live events before querying Postgres, replays every visible committed
event after that cursor in bounded pages, sends a replay watermark, and then switches to the live
stream while suppressing duplicate sequence numbers.

The current browser refreshes its bounded materialized snapshot after replay or a live event. Later
clients may apply typed events directly for lower latency.

## Rooms and threads

Room membership is a server-side visibility boundary:

- snapshots include only rooms, missions, tasks, runs, messages, and events visible to the viewer
- room writes require membership
- WebSocket replay and live fan-out apply the same room filter
- replies store both their immediate parent and stable thread root
- mentions are actor IDs validated against room membership
- links to missions, tasks, runs, and artifacts are validated against the same room

The demo includes Eve as a Corp guest without Product Lab membership so isolation can be exercised
end to end.

## Control lease

Only one human actor controls an agent at a time. A lease:

- is scoped to an agent and Corp
- has a random fencing token that is never included in shared snapshots or journal payloads
- expires
- can be renewed by its owner
- can only be replaced by a different actor after expiry
- can be explicitly released or transferred

Every immediate control command must carry the current fencing token. Renewing or transferring a
lease rotates that token, so delayed commands from an earlier controller fail closed. Operators
without the lease may still leave a durable queued message without a token.

Owners, admins, and managers may issue an emergency stop independently of ordinary control
ownership. The request is audited, delivered to the runner, kills the active child process, and
ends the mission, task, and run as cancelled.

Messages from the current controller can be delivered to the active process. Other messages are
durably queued.

## Target module boundaries

- `crony-domain`: pure domain types and state vocabulary
- `crony-protocol`: client, runner, MCP, ACP, and A2A wire types
- `crony-store`: transactional persistence
- `crony-server`: APIs, realtime gateway, and orchestration host
- `crony-runner`: process supervision and isolation
- `crony-cli`: scriptable operator interface

## Near-term architecture work

1. Add runner heartbeats and disconnect grace.
2. Move child-process behavior behind an `AgentAdapter` trait.
3. Add git worktree isolation.
4. Add durable approvals and policy evaluation.
5. Add authenticated users and runner enrollment.
6. Add artifact upload rather than host-local artifact paths.
