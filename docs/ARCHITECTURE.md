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

## Human identity and authorization

Production mode authenticates humans with an OIDC bearer token. The server discovers the issuer's
UserInfo endpoint, resolves `(issuer, subject)` to a Corp-local human actor, and applies a
deny-by-default role matrix before every Corp read or mutation. An actor ID supplied by an older
client is treated only as a consistency claim and must match the authenticated actor.

Browser clients exchange their bearer token for a one-time, 30-second WebSocket ticket. The ticket
is consumed and Corp-authorized before event replay begins, so the OIDC token is never placed in a
URL. Development mode retains the fixed Alice, Bob, and Eve actors, but demo routes and claimed
development identities are not registered in production mode.

## Runner identity

Runners are bound to one Corp. An owner or admin creates a short-lived, one-time enrollment token.
The first successful registration consumes it and returns a rotating workload credential. Each
later connection atomically exchanges the current credential for the next one, so replayed
credentials fail. Revocation disconnects the live node and blocks reconnection.

## Scoped secrets

Secret plaintext is encrypted at rest and never embedded in a task prompt, event, artifact, command
argument, or shared snapshot. Task contracts carry only typed secret references. At dispatch, the
server checks the mission requester, task, run, enrolled runner, tool, resource prefix, and expiry,
then records a metadata-only grant.

Current process adapters receive the short-lived value through their environment and advertise
`environment_reduced_assurance`. The boundary is explicit: this mode protects shared state and
logs, but a compromised child process can still inspect its own environment.

## Durable action approvals

Agents can request a typed action approval while remaining supervised by the runner. The server
stores risk, action, rationale, required roles, expiry, and process lineage. Decisions use a
client-generated idempotency key and transactionally enqueue a durable runner command. Pending
commands are retried after server or runner reconnect, while runner command IDs suppress duplicate
process effects.

## Budgets and circuit breaking

Run, mission, requester, and Corp token/cost limits are evaluated after usage events. Explicit
tool-activity events feed no-progress and repeated-tool counters; human conversation is exempt.
Monotonic steer, constrain, suspend, and stop transitions create immutable incidents and durable
runner directives.

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
    +-- deterministic process ---> output/runner/<run-id>/result.md
    |
    +-- Codex app-server JSON-RPC ---> provider thread + repository changes
```

The deterministic process remains the offline systems fixture. The first real provider adapter
uses Codex app-server over stdio JSON-RPC rather than scraping terminal text.

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
ownership. The request is audited and delivered to the runner. Adapters first receive a graceful
turn interruption and are force-terminated after a bounded timeout. The mission, task, and run end
as cancelled.

Messages from the current controller can be delivered to the active process. Other messages are
durably queued.

## Target module boundaries

- `crony-domain`: pure domain types and state vocabulary
- `crony-protocol`: client, runner, MCP, ACP, and A2A wire types
- `crony-store`: transactional persistence
- `crony-server`: APIs, realtime gateway, and orchestration host
- `crony-runner`: process supervision and isolation
- `crony-cli`: scriptable operator interface
- `crony-gateways`: MCP, ACP, and A2A translation without leaking internal schemas

## External protocol gateways

MCP, ACP, and A2A remain adapters over authenticated Crony APIs. They negotiate explicit versions
and expose bounded context, mission, message, task, and streaming operations. Assignment fencing,
control leases, approval tables, budgets, and secret records remain private Crony concepts.

## Runner liveness and reconciliation

Every runner connection has a fresh connection epoch. Postgres stores:

- hostname, operating system, and capabilities
- connected, grace, or offline status
- connection epoch
- last heartbeat
- disconnect and grace-expiry timestamps

Every run assignment has a private assignment fencing token that is sent only to its runner.
Active runners include run IDs and assignment tokens when reconnecting. Matching claims emit a
`run.reconciled` event and continue. Unknown or stale claims receive a stop command.

A disconnect enters a bounded grace period rather than immediately failing work. Reconnecting with
a newer epoch cancels the old grace timer. When grace expires, active runs become `lost`, their
tasks become `blocked`, and their agents become `offline`. A later stale runner claim cannot
overwrite that terminal lost state.

The runner keeps active process controls and a bounded outbound event queue outside any individual
WebSocket connection, so a transport reconnect does not kill the child process or discard events.

## Worktree lifecycle

Every initial run receives a linked Git worktree under the configured runner workspace and a
deterministic `crony/task-.../run-...` branch. A resumed provider session re-enters that exact
worktree and branch through the root run ID.

Provisioning is fail-closed:

- the source repository and base ref are validated at runner startup
- every resolved worktree path must remain below the configured worktree root
- an occupied, detached, mismatched, or non-worktree path is rejected
- provisioning never falls back to the source checkout
- Git metadata mutations are serialized inside one runner

After a process ends, cleanup checks the actual Git state. Dirty worktrees, ignored files, branches
with commits not integrated into the current base, and any state that cannot be verified are preserved.
Automatic removal occurs only when the tree is clean and its branch is reachable from or
tree-equivalent to the base. The runner emits `run.workspace_preserved` or
`run.workspace_removed`, and Postgres stores the final disposition.

## Planning and scheduling

Mission decomposition is a replaceable server-side strategy, not a privileged singleton agent.
The initial registry includes:

- `single`: one bounded delivery task
- `parallel-specialists`: two independent specialist roots followed by one synthesis task

Every planned task persists a self-contained contract: objective, expected output, acceptance
tests, allowed tools, prohibited actions, references, write scope, token budget, deadline, and
escalation path. Validation rejects unknown agents, adapter mismatches, missing contract fields,
cycles, excessive depth, node fan-out, retry counts, and budgets.

The scheduler:

- releases only tasks whose dependencies are completed
- requires the assigned agent to be idle
- chooses a connected runner advertising the required adapter
- orders tasks and runner IDs deterministically
- dispatches independent roots in parallel
- increments attempts transactionally
- retries failed tasks only while attempts remain
- launches downstream tasks after committed completion events
- marks the mission complete only after every task completes

## Evidence-gated completion

Every task stores a typed verification policy. Automated checks execute on the runner, never on the
server, and currently support:

- recorded artifact existence, byte floor, and SHA-256 integrity
- worktree-relative files
- direct command execution without a shell
- test commands
- JSON object required-key schemas
- PNG or JPEG screenshot evidence

The runner buffers an adapter's completion signal, emits one evidence record per check, and sends
`run.completed` only after every automated check passes. The server rejects completion events that
arrive before complete passing evidence or while a manual gate is required.

Failed verification sets the task to `verification_failed` and the mission to failed. A successful
automated policy can instead enter `waiting_for_approval`. Human-approval gates enforce configured
roles. Independent-review gates additionally reject the mission requester and producing agent.
Decisions are durable, actor-attributed, and can release downstream scheduler work.

## Agent adapters

Provider runtimes implement one `AgentAdapter` contract:

- execute a new run
- stream status, output, artifacts, usage, and terminal events
- receive steer, interrupt, and stop controls
- optionally resume a provider session
- optionally collect usage after a session

Every feature is reported as supported or unsupported with a reason. The deterministic
`fake-process` and real `codex` implementations use the same contract.

The Codex adapter:

- starts one app-server process per active turn
- uses `thread/start`, `turn/start`, `turn/steer`, and `turn/interrupt`
- resumes durable provider state with `thread/resume`
- converts structured item and turn notifications into Crony events
- records token usage without double-counting cumulative notifications
- denies unexpected interactive provider requests
- applies a workspace-write, network-disabled sandbox policy
- disables user-configured MCP servers, apps, and hooks for supervised runs
- fingerprints tracked and untracked changed files in the evidence artifact

Claude Code and OpenCode use a shared normalized external-CLI adapter. Provider-specific launch
flags are isolated at the boundary, while JSONL output, sessions, usage, cancellation, and
provider-neutral evidence map into the same lifecycle. Batch-mode steering limitations are
reported explicitly rather than hidden.

Deterministic app-server fixtures and authenticated real-provider probes cover start, structured
streaming, steering, interruption, emergency stop, resume, usage, artifacts, and failure behavior.

## Near-term architecture work

1. Add authenticated users and runner enrollment.
2. Add the scoped secret broker.
3. Generalize durable approval suspension for risky side effects.
4. Add budgets and circuit-breaker policy.
5. Add artifact upload rather than host-local artifact paths.
