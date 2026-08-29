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

The server publishes committed domain events to connected browser clients. A browser currently
refreshes its bounded snapshot after each event. The next protocol revision will resume from an
explicit event sequence rather than relying only on snapshot refresh.

## Control lease

Only one human actor controls an agent at a time. A lease:

- is scoped to an agent and Corp
- has a random token
- expires
- can be renewed by its owner
- can only be replaced by a different actor after expiry

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

1. Add event-sequence resume and bounded replay.
2. Add runner heartbeats, disconnect grace, and fencing tokens.
3. Move child-process behavior behind an `AgentAdapter` trait.
4. Add git worktree isolation.
5. Add durable approvals and policy evaluation.
6. Add authenticated users and runner enrollment.
7. Add artifact upload rather than host-local artifact paths.

