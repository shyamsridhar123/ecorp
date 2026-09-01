# ECorp architecture

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
- upload bounded artifact bytes, digest, and declared media type to the server

The server never executes an agent shell command.

## Artifact storage and provenance

The runner reads the adapter artifact from its isolated worktree and sends a bounded base64 upload
over the authenticated runner channel. The server revalidates the declared byte count, SHA-256, and
media type before writing a Corp-namespaced content-addressed object. Production uses an
S3-compatible private bucket; development can use the same object-store interface on local disk.

The server records artifact ID, Corp, task, run, producing agent, producing runner, verifier,
digest, normalized media type, byte count, retention deadline, and an HMAC-SHA256 provenance
signature. The shared `Run` projection exposes an API URI and signed metadata, not a runner path or
bucket URL. Downloads re-check the signature, retention, object bytes, digest, and media type, then
enforce Corp and room membership before returning an attachment with `nosniff`.

## Human identity and authorization

Production mode authenticates humans with an OIDC bearer token. The server discovers the issuer's
UserInfo endpoint, resolves `(issuer, subject)` to a Corp-local human actor, and applies a
deny-by-default role matrix before every Corp read or mutation. An actor ID supplied by an older
client is treated only as a consistency claim and must match the authenticated actor.

Browser clients exchange their bearer token for a one-time, 30-second WebSocket ticket. The ticket
is consumed and Corp-authorized before event replay begins, so the OIDC token is never placed in a
URL. Development mode retains the fixed Alice, Bob, and Eve actors, but demo routes and claimed
development identities are not registered in production mode.

The web client presents a production connection form for Corp ID, actor ID, and a short-lived
token. The token stays in session storage, REST requests use the bearer header, artifact downloads
use an authenticated fetch, and the operating-as selector is locked to the authenticated actor.
The CLI accepts the same token through `CRONY_ACCESS_TOKEN`.

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
logs, but a compromised child process can still inspect its own environment. The runner rejects a
grant that is already expired and stops the provider at the earliest grant expiry.

## Durable action approvals

Agents can request a typed action approval while remaining supervised by the runner. The server
stores risk, action, rationale, required roles, expiry, and process lineage. Decisions use a
client-generated idempotency key and transactionally enqueue a durable runner command. Pending
commands are retried after server or runner reconnect. They remain pending until the runner
acknowledges application, while runner command IDs suppress duplicate process effects. Expired
approvals cancel the run, task, and mission coherently and enqueue a durable rejection.

## Budgets and circuit breaking

Run, mission, requester, and Corp token/cost limits are evaluated after usage events. Explicit
tool-activity events feed no-progress and repeated-tool counters; human conversation is exempt.
Monotonic steer, constrain, suspend, and stop transitions create immutable incidents and durable
runner directives. Suspend is a terminal provider checkpoint: the session and worktree are
preserved for an explicit resume. A hard overrun stops immediately.

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

Runner events have independent UUIDs and carry both the authenticated connection epoch and the
private assignment token. The server rejects superseded sockets and wrong-assignment events before
state mutation. Replaying the same valid runner event is a no-op because `(corp_id,
idempotency_key)` is unique.

## Real-time delivery

The server publishes committed domain events to connected browser clients. Clients reconnect with
an actor identity and `after_seq` cursor. The server verifies Corp membership before upgrading the
connection, subscribes to live events before querying Postgres, replays every visible committed
event after that cursor in bounded pages, sends a replay watermark, and then switches to the live
stream while suppressing duplicate sequence numbers.

If the in-memory broadcast subscriber lags, the server refills the missing sequence range from
Postgres before continuing live delivery.

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

The demo includes Eve as a Corp guest without Automation Division membership so isolation can be exercised
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

Messages from the current controller can be delivered to adapters that support steering. Other
messages are durably queued, reserved into the next task prompt, and marked delivered only after
the run starts.

## Target module boundaries

- `crony-domain`: pure domain types and state vocabulary
- `crony-protocol`: client, runner, MCP, ACP, and A2A wire types
- `crony-store`: transactional persistence
- `crony-server`: APIs, realtime gateway, and orchestration host
- `crony-runner`: process supervision and isolation
- `crony-cli`: scriptable operator interface
- `crony-gateways`: MCP, ACP, and A2A translation without leaking internal schemas

## External protocol gateways

MCP, ACP, and A2A remain adapters over authenticated ECorp APIs. They negotiate explicit versions
and expose bounded context, mission, message, task, and streaming operations. Assignment fencing,
control leases, approval tables, budgets, and secret records remain private ECorp concepts.

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
tasks become `blocked`, their missions fail, and reusable agent identities return to `idle`. A
later stale runner claim cannot overwrite that terminal lost state.

The runner keeps active process controls and its complete outbound event queue outside any
individual WebSocket connection. Buffered run events are rebound to the new authenticated epoch
without changing their assignment token, so a transport reconnect does not kill the child process
or discard events.

Server startup recovery is explicitly single-owner. Secondary replicas and production-auth probes
disable it so they cannot place another server's healthy runners into grace.

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
- injects verified dependency artifacts into synthesis prompts
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

`execute` and `resume` are terminal lifecycle boundaries: an adapter may return only after its
child process or SDK client has exited, disconnected, or been force-stopped. The runner then emits
`run.session_terminated` before verification, records that no live provider process remains, and
removes the run from its active-process map after workspace finalization. Provider session IDs may
remain persisted for an explicit future resume, but they do not imply a resident process.

Provider runtimes implement one `AgentAdapter` contract:

- execute a new run
- stream status, output, artifacts, usage, and terminal events
- receive steer, interrupt, and stop controls
- optionally resume a provider session
- optionally collect usage after a session

Every feature is reported as supported or unsupported with a reason. The deterministic
`fake-process` and real provider implementations use the same contract. The fake process is an
offline lifecycle simulator, not an inference provider; it remains registered beside the real
adapters so orchestration can be tested without credentials or quota.

The Codex adapter:

- starts one app-server process per active turn
- uses `thread/start`, `turn/start`, `turn/steer`, and `turn/interrupt`
- resumes durable provider state with `thread/resume`
- converts structured item and turn notifications into ECorp events
- records token usage without double-counting cumulative notifications
- denies unexpected interactive provider requests
- applies a workspace-write, network-disabled sandbox policy
- disables user-configured MCP servers, apps, and hooks for supervised runs
- fingerprints tracked and untracked changed files in the evidence artifact

Claude Code and OpenCode use a shared normalized external-CLI adapter. Provider-specific launch
flags are isolated at the boundary, while JSONL output, sessions, usage, cancellation, and
provider-neutral evidence map into the same lifecycle. Batch-mode steering limitations are
reported explicitly rather than hidden.

The GitHub Copilot adapter uses the official Rust SDK. A runner discovers the signed-in account's
model catalog at registration and advertises policy state, model limits, vision support, reasoning
levels, and billing multiplier metadata. The selected model and reasoning effort are persisted in
the task contract and run, survive resume, and participate in runner matching. Copilot may write
inside the assigned worktree and read its per-worktree isolated SDK state automatically. Network,
sandbox bypass, external paths, and shell commands that cannot be proven scoped suspend through
ECorp's durable approval flow.

Deterministic app-server fixtures and authenticated real-provider probes cover start, structured
streaming, steering, interruption, emergency stop, resume, usage, artifacts, and failure behavior.

## Governed dark-factory intake

GitHub Project issues enter the execution plane through a durable factory work item rather than
directly launching a provider. Each item records the exact Project item, repository issue, source
revision, policy snapshot, current state, claim owner, lease expiry, monotonic version, and linked
mission.

Claim, renewal, and mission materialization are Corp-scoped and idempotent. Competing controllers
are serialized in Postgres, while fencing tokens and expected versions reject stale automation.
The claim token is a capability: it is returned only in the direct authorized response and is
omitted from shared snapshots and immutable events.
Every reclaim preserves and revalidates the original source and policy snapshots; only ownership,
fencing, lease, and recoverable lifecycle fields can change.

Mission and task creation is atomic with the `claimed -> mission_created` transition. A controller
that loses its response or restarts can replay the same operation and recover the existing mission;
it cannot create a duplicate graph.

The same fenced controller advances explicit `running`, `blocked`, and `verified` states. External
status failure is persisted before the controller returns an error, and retry reclaims an expired
non-terminal lease without changing the original source revision or policy snapshot.
Materialization may only narrow the persisted repository, adapter, write-scope, token, and cost
policy. The server derives `verified` authority from a completed mission whose tasks all passed
verification; a controller cannot assert it directly.
When policy pins a model or reasoning effort, every materialized task must retain that exact value;
omission cannot fall back to a provider default.
The generic transition endpoint cannot assert `publishing` or `published`; those states are
reserved for the verifier-gated publication operation in #61.

Factory snapshots are limited to roles that can operate missions. Pre-materialization events omit
source issue metadata, and events become room-scoped as soon as a mission exists.

GitHub remains the planning and status source of truth, but external status changes must follow
durable ECorp transitions. Pull-request publication, merge, and deployment are separate effects
with separate authorization and idempotency boundaries. See ADR 0020.

Before changing GitHub Project state, the controller renews its lease to an external-effect window,
then re-fetches the Project item, issue revision, issue state, required label, and dependency state.
After the mutation it renews and repeats the source-eligibility check immediately before launch.
Any changed or newly blocked source durably moves the factory item to `blocked` without launching a
run. Factory tasks persist the claimed GitHub repository and source base ref. Runners advertise a
normalized `remote` and `base`, and scheduling rejects a runner whose configured checkout does not
match before creating a run.

## Near-term architecture work

1. Connect eligible GitHub Project items to the durable factory claim API.
2. Add verifier-gated, idempotent pull-request publication without implicit merge.
3. Add stronger OS/container isolation for untrusted child processes.
4. Add artifact retention sweeping and signing-key rotation.
5. Add multi-region control-plane and object-store recovery drills.
