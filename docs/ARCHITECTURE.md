# ECorp architecture

## Product invariant

The office is a projection of authoritative operational state. Closing a browser or desktop
window must not terminate an active agent run.

## Three planes

### Experience plane

- React web client
- Tauri 2 desktop shell
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
over the authenticated runner channel. The server validates the declared byte count, SHA-256, and
media type in memory. While holding the authoritative run lock, Postgres then rechecks assignment,
task, budget, and breaker state and persists the `staged` metadata reservation. Only the owner of
that reservation writes bytes to the top-level `staging/corps/<corp>/<event>` key, publishes the
Corp-namespaced content-addressed object, and atomically marks metadata `ready` with the immutable
artifact event. Production uses an S3-compatible private bucket; development uses the same
object-store interface on local disk.

Final publication and metadata finalization are idempotent. A retry can adopt the existing staged
row for the same run and digest and writes the same bytes to its authoritative key. Startup and
periodic recovery wait through a reservation grace window, finalize valid staged metadata, accept an
already-published final object when the staging copy was already cleaned, reject old metadata whose
bytes fail integrity checks, release old reservations whose staged and final objects are both
missing so the runner can retry, retry cleanup for `ready` or `rejected` rows, and remove staging
objects only after rechecking that no metadata reservation owns them. Transient object-store or
database failures are left recoverable rather than made terminal. Cleanup never deletes a
content-addressed final object, so accepted and rejected uploads may safely share a digest.

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

Claude Code uses its supported bidirectional stream-JSON boundary rather than terminal scraping or
an ECorp-specific child protocol. The runner starts Claude with stream-JSON input/output, manual
permission mode, and the stdio permission prompt tool. It completes a correlated `initialize`
control request/response before sending the mission as a typed user frame, then correlates each
`can_use_tool` control request by its unchanged provider request and tool-use IDs. A strictly
contained worktree-local read or write may receive an immediate one-shot response. Bash, network,
blocked-path, outside-worktree, and ambiguous requests emit one bounded durable action approval and
remain pending in the adapter. The durable decision produces the matching `control_response`;
allow responses retain the provider's original structured input in process memory, while durable
approval text contains bounded metadata and hashes rather than raw tool input. Rejection, expiry, stop,
interrupt, provider cancellation, or breaker termination denies or clears the pending request
before terminal session reporting. Unknown and duplicate decisions cannot select another request.

## Budgets and circuit breaking

Run, mission, requester, and Corp token/cost limits are evaluated after usage events. Explicit
tool-activity events feed no-progress and repeated-tool counters; human conversation is exempt.
Monotonic steer, constrain, suspend, and stop transitions create immutable incidents and durable
runner directives. Suspend is a terminal provider checkpoint: the session and worktree are
preserved for an explicit resume. A hard overrun stops immediately.

Mission limits retain immutable original token/cost values plus the current authorized ceiling.
An exhausted `suspend` can be recovered only through a versioned `mission_budget_revisions`
aggregate proposed and decided by an owner or admin. The row records current and proposed limits,
usage at proposal, rationale, exact idempotency keys, proposer, decider, and optional before/after
task-contract snapshots.

Only one revision can be pending. Proposal and approval both lock and revalidate the Corp, mission,
latest run, active-run set, current budget, and cumulative usage. Limits never decrease and the
approved ceiling must still exceed all consumed usage. An optional finish scope targets one
unfinished task, may reduce its remaining token/cost budget and write scope, and must retain the
existing verifier policy. The target must be the task from the latest suspended run. Approval
compares the task contract and verifier policy to the proposal snapshot before applying it, so an
intervening change cannot be overwritten.

Resume rejects exhausted mission authority before creating a run. An approved recovery creates the
new run in the same provider session and preserved worktree, with token/cost limits clamped to the
smallest of the revised task contract and remaining mission, requester rolling-24-hour, and Corp
rolling-24-hour authority. Usage updates, policy changes, and resume admission share advisory
budget-scope locks. Resume also serializes by provider-workspace lineage, requires the selected
source to be the latest lineage run, and rejects the entire lineage after any descendant reaches
`stop`.

Proposal, replay, decision, and decision replay require current membership in the mission room and
hold a key-share lock on that membership through the transaction. Browser proposal and decision
keys remain stable after a lost response, allowing the committed result to replay rather than
creating conflicting authority. See ADR 0023.

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

The deterministic process remains the offline systems fixture. The Codex adapter uses app-server
over stdio JSON-RPC rather than scraping terminal text.

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

## Mission specifications and contract revisions

Mission titles remain bounded labels. The durable `missions.description` field carries the complete
operator specification and is injected into every planned task alongside its role-specific
objective. Operator contract overlays preserve explicit expected output, acceptance criteria,
allowed tools, prohibited actions, references, and write scope. A supplied typed verifier policy
replaces the delivery task's generated policy before the plan is validated.

Missions and tasks begin at specification/contract version 1. A contract change transaction locks
the mission and task, rechecks actor and room authority, rejects active work, compares the expected
version, updates the current projections, inserts the immutable prior/replacement snapshot, and
emits `mission.contract_revised`.

`redispatch` revisions are pre-execution only. `resume` revisions require the latest terminal,
preserved, non-stop provider/worktree lineage and cannot change source, secret, provider, budget, or
deliverable authority. They also cannot widen tools or write scope or remove prohibitions. A
revision records the intended next action but never starts it; dispatch and resume remain separate
commands. Resume rebuilds the provider prompt from the current task contract, ensuring the revised
description, objective, acceptance criteria, guardrails, and references reach the preserved
provider session.

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

External CLI adapters place the provider root inside an OS-owned process scope before provider code
can launch tools. On Windows, the provider starts suspended, is assigned to a private Job Object
with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and is resumed only after assignment succeeds. If
ownership setup fails after process creation, the still-suspended root remains owned until cleanup
is positively verified. Interrupt, stop, breaker, protocol-failure, and drop paths converge on
`OwnedProcessTree`, which terminates the owned scope, waits for the provider root, and verifies the
scope is empty before the adapter returns. Unix external CLI adapters currently advertise
unsupported and refuse to spawn because a session/process group is not a non-escapable descendant
boundary. Availability probes use the same ownership boundary and allow only one outstanding
probe-or-cleanup guardian per adapter. Failure to establish or verify ownership is not permission
to emit a terminal claim.

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
GitHub owner and repository identities are normalized to lowercase before locking and uniqueness
checks, preventing case variants from creating duplicate work items.

Before a new claim or an unmaterialized reclaim, the controller sends the exact issue-derived
materialization payload and final policy snapshot to a mutation-free factory preflight endpoint.
Dry run and execution use this same boundary. The server repeats model and reasoning selection,
task-graph and budget validation, verifier-policy validation, deliverable and write-scope checks,
policy narrowing, mission title and description normalization, the 65,536-byte operation-snapshot
limit, and destination-room authorization. A rejected preflight leaves the GitHub Project item in
`Todo` and creates no factory item, mission, task, or run.

Mission and task creation is atomic with the `claimed -> mission_created` transition. Mission
placement selects the oldest room in the Corp that actually contains the requesting actor rather
than assuming that every operator belongs to the Corp's oldest room. A controller that loses its
response or restarts can replay the same operation and recover the existing mission; it cannot
create a duplicate graph.

Materialization repeats the full preflight because runner capabilities, room membership, or other
authority can change between the read-only check and the durable write. If a post-claim
pre-mission rejection still occurs, a dedicated store operation locks the exact work item, verifies
the current opaque claim token, records a claim-generation-scoped idempotency key, moves
`claimed -> blocked`, and releases the lease in the same transaction. It deliberately does not
require the attempted version or lease to remain current, so a concurrent renewal or controller
delay cannot strand the claim; a rotated token fences the stale compensation from a newer owner.
The token remains absent from logs, events, snapshots, and failure text. Recovery must still present
the exact original source revision and policy snapshot.

The same fenced controller advances explicit `running`, `blocked`, and `verified` states. External
status failure is persisted before the controller returns an error, and retry reclaims an expired
non-terminal lease without changing the original source revision or policy snapshot.
Execution failures and evidence-verification failures remain distinct factory states.
Materialization may only narrow the persisted repository, adapter, write-scope, token, and cost
policy. The server derives `verified` authority from a completed mission whose tasks all passed
verification; a controller cannot assert it directly.
When policy pins a model or reasoning effort, every materialized task must retain that exact value;
omission cannot fall back to a provider default.
Provider-backed factory tasks also receive a manual verification gate before accepted completion.
The deterministic `fake-process` harness remains gate-free so offline systems tests can terminate
without pretending to be production evidence.
The generic transition endpoint cannot assert `publishing` or `published`; those states are
reserved for the verifier-gated publication operation in #61.

Factory snapshots are limited to roles that can operate missions. Pre-materialization events omit
source issue metadata, and events become room-scoped as soon as a mission exists.
Controller discovery does not use that bounded snapshot as an index. After listing the current
GitHub Project candidates, the trusted controller sends only those Project item IDs to a
Corp-authorized lookup endpoint together with the normalized Project owner and Project number. The
lookup accepts at most 1,000 identifiers, bounds each identifier to 160 characters, deduplicates
the request, and returns an explicit match count. Its query includes the source kind and complete
Project identity prefix from the existing composite uniqueness index, so equal item IDs in
different Projects remain isolated without another index. Recovery re-fetches the selected Project
item through the same path immediately before claim so the persisted source revision, policy
snapshot, work item, and mission remain authoritative even when more recent historical work items
have displaced it from the shared snapshot. Claim and reclaim idempotency keys also include the
normalized lease duration because lease duration is part of the persisted operation request.

ECorp Build GitHub Project #3 and its linked issues remain the planning and status source of truth;
`docs/BACKLOG.md` is historical seed material only. External status changes must follow durable
ECorp transitions. Pull-request publication, merge, and deployment are separate effects with
separate authorization and idempotency boundaries. See ADR 0020.

Factory controller configuration and health are persisted separately from individual work-item
leases. A controller records its Project and repository scope, desired running or paused state,
connection epoch, heartbeat lease, monotonic reconciliation generation, active work item, last
result, and bounded failure detail. The browser derives `offline`, `watching`, `working`,
`blocked`, and `needs decision` from this authoritative record and exposes versioned pause, resume,
and reconciliation controls. Pausing intake never interrupts an existing mission.

Before changing GitHub Project state, the controller renews its lease to an external-effect window,
then re-fetches the Project item, issue revision, issue state, required label, and dependency state.
It renews again immediately before the mutation, and every GitHub CLI subprocess has a bounded
deadline below the minimum effect lease. After the mutation it renews, repeats the
source-eligibility check, and renews once more immediately before launch.
Any changed or newly blocked source durably moves the factory item to `blocked` without launching a
run. Before a new claim, the trusted controller verifies the configured GitHub remote and resolves
the human-readable source ref to a full immutable Git object ID. Factory policy, task contracts,
and run launch records retain the repository, symbolic ref, and resolved commit together.
Runners resolve that same tuple once at startup and advertise it as structured workspace
capability data. Scheduling compares the immutable commit rather than trusting a matching `HEAD`
label, and the runner repeats the check before creating or reusing a worktree. Pinned assignments
start from their exact authorized commit. Ordinary unpinned assignments preserve the prior behavior
of resolving the configured symbolic ref under the Git lock for each new worktree. Resume carries
the source run's persisted workspace base commit and must reuse a preserved branch descending from
that exact identity.

Migration 0022 derives legacy factory commits only when persisted workspace evidence identifies one
unambiguous commit. Legacy claims or materialized no-run missions without derivable evidence are
marked `source_commit_upgrade_required`. A fenced operator may explicitly pin a freshly resolved
commit before launch; the operation updates policy and any no-run task contracts atomically,
increments the factory version, and appends an audit event.

## Verifier executable-resolution boundary

Provider command execution and authoritative verification intentionally have different capabilities.
A provider adapter owns its documented vendor process or shell behavior. The verifier never assumes
that provider shell lookup is reproducible: it launches the persisted `program` and `args` as
separate values without constructing a shell command.

For a bare verifier program, the runner searches only absolute entries already present in its
process `PATH`; empty and relative entries are ignored, so neither the assigned worktree nor the
runner current directory is searched implicitly. Linux and macOS require an exact regular file with
at least one executable bit in one of those entries. On Windows, a name with an extension requires
an exact regular file, while an extensionless name is tested against the runner's validated
alphanumeric `PATHEXT` entries in their declared order. This resolves installed shims such as
`npm.cmd`, `pnpm.cmd`, and `npx.cmd` without mistaking an adjacent extensionless Unix shim for a
Windows executable.

An absolute program path is used only when explicitly declared. A relative program containing path
components is anchored to the assigned worktree; traversal, dot, rooted, and drive-relative
components are rejected, and canonical containment rejects symlink escapes. Every explicit target
is canonicalized and must be a regular file. The resolved canonical file is passed to Rust's
argv-based process API with each persisted argument separately.
The verifier does not read an alternate shell path or synthesize a command string; Rust's Windows
process implementation applies its platform batch-file argument escaping when the canonical target
is a `.cmd` or `.bat` shim.

Command evidence records the bounded requested program, `explicit_path`, `path`, or `path_pathext`
resolution mode, and a resolved identity containing at most 128 filename characters plus a SHA-256
digest of the canonical path. It never records the canonical path or the mutable `PATH`. An
unresolved identity is `null`, and missing, non-regular, ambiguous, or unspawnable programs fail the
check with a precise diagnostic. Resolution and execution share the declared timeout; stdin remains
closed, output remains bounded, and dropping a timed-out child still kills it.

## Portable source-deliverable boundary

Provider artifacts and application deliverables are separate object roles. After the provider
process terminates, the runner executes the persisted verifier policy in the assigned worktree. A
passing report is normalized and hashed. The runner then uses a temporary Git index to construct
the requested patch, archive, typed set, commit/branch bundle, or review report from tracked and
non-ignored untracked changes.

The resulting bytes use the existing reservation, staging, validation, finalization, and recovery
path. `source_deliverables` links the ready object to its task, run, verification digest, base
commit, optional post-verification commit, task branch, retention, and integration state. The
server returns a runner-only storage acknowledgment; only then can the runner emit passing
verification and evaluate safe worktree cleanup.

Pull-request publication, merge, and deployment are outside this boundary. A ready source
deliverable proves portable review material exists; it does not imply external integration. See
ADR 0021.

## Idempotent pull-request publication

Publication is a dedicated durable aggregate rather than a generic factory state transition. It
links one verified factory work item and one ready commit/branch deliverable to an immutable target
repository, base ref, branch, commit, pull-request title/body, source issue, explicit human
authorization snapshot, and effect key. Attempts have independent publisher leases and fencing
tokens; tokens never enter snapshots or events.

Publisher workloads have a separate Corp-scoped identity and credential from the authorizing human.
Owners or admins enroll bounded credentials whose plaintext is returned once and whose SHA-256 hash,
expiry, revocation state, and last-use time are stored. Start, renewal, failure, and every checkpoint
require both current human authority and the independently authenticated publisher identity. The
server derives the publisher ID from that workload credential and requires the request's publisher
ID to match exactly. The CLI reads the credential from a file and sends it only in the authenticated
publication request header.

Publisher planning does not use the bounded browser snapshot as an index. An exact Corp-authorized
publication-context read loads the requested work item, its durable publication, and every source
deliverable for the linked bounded mission. This keeps initial publication and restart recovery
available after newer history has displaced any of those objects from shared snapshot limits. Both
that context and the exact publication-status read join through the viewer's current mission-room
membership; an out-of-room actor receives no work-item source metadata or policy either.

The trusted publisher downloads the signed deliverable and imports its embedded Git bundle into a
temporary bare repository. It verifies the bundle digest, source branch provenance, exact commit,
authorized base ancestry, and current remote base before adopting or pushing the branch. Existing
matching branches and pull requests are recovered; conflicting remote identities fail closed and
branches are never force-pushed. The branch passes `git check-ref-format --branch` before durable
start, with a defensive server-side branch-shape check as a second boundary.
The runner bundles a short run-scoped ref pointing to the already validated workspace branch rather
than the worktree's possibly detached `HEAD`; the publisher accepts exactly one matching legacy
HEAD or run-scoped bundle head before import.

An adopted pull request must report the exact verified `headRefOid`, the target repository owner,
and `isCrossRepository = false`; a same-named branch from a fork is ignored and cannot advance
durable publication state. A symbolic `HEAD` base is resolved without `--refs`, then cross-checked
against its advertised explicit branch target. ECorp preserves `HEAD` as the authorized policy base
but passes and persists the resolved branch name, such as `main`, as the actual GitHub PR base.
Canonical GitHub pull-request URLs are matched by strict scheme/host/path/number while comparing
repository owner/name components case-insensitively.
Project status reads query the known Project item node ID directly and verify its Project and Status
field identity; a bounded Project item listing is never used to prove that the item disappeared.
The Status field and its options are queried directly by name from the known Project node, so
field-list pagination cannot hide it.
Factory policy accepts publication bases only as symbolic `HEAD`, a short branch name, or an
explicit `refs/heads/*` branch. Tags, remote-tracking refs, and invalid Git branch names fail before
the controller reads candidates or claims a work item, and the store repeats the policy check.
Omitting the option derives it from the selected source base ref, so `HEAD` remains the ordinary
default while an explicit source branch such as `release` also becomes the publication base unless
the operator overrides it.

The state sequence is `publishing -> branch_pushed -> pull_request_created -> published`. A
checkpoint can be replayed after duplicate delivery, process restart, or external success followed
by local failure. Project status is not changed until the pull-request identity is durable, and the
final transaction moves the factory item to `published` and the source deliverable to integration
state `published`. Auto-merge, merge, and deployment remain false and separately unauthorized. See
ADR 0022.

Every pre-effect lease renewal locks the publication and revalidates the current attempt actor's
persisted role plus current mission, verifier, deliverable, policy, run, requester, Corp-budget, and
hard-breaker authority before extending the lease. New starts, idempotent start replay, collision
recovery, and every renewal also require the acting publisher to remain a current member of the
mission room.
The selected deliverable run is always checked against current budget and breaker authority. A
`stop` stage anywhere in the mission remains terminal. A historical `suspend` is accepted only when
it is an explicit resumed ancestor of the selected verified run and its current no-progress and
repeated-tool counters remain below the current policy limits. Unrelated suspends, stop-level loop
metrics, and missing, duplicate, or cyclic resume lineage fail closed.
For the Project effect, the publisher reads the exact Project and Status identities, renews
authority, refreshes the exact item status, and re-fetches the durable PR to revalidate its open
state, base/head, content, repository/SHA, URL, draft, and auto-merge identity. It then performs a
second authority renewal immediately before `item-edit`, so slow remote reads cannot leave a stale
publisher mutating Project state. Before recording completion it refreshes Project status, repeats
the PR validation, and renews authority again.
Default start idempotency keys fingerprint the complete normalized invocation, so equal calls remain
stable while publisher-host, authorization-reason, or lease changes automatically receive a distinct
recovery key instead of conflicting with an earlier operation request. Custom pull-request titles
and body files use the server's trim, size, control-character, and newline rules before that
fingerprint or durable start is constructed. Target repository owner/name components are likewise
validated and lowercased before plan construction.
Recovery dry-runs and executions reuse the publication base persisted in an existing factory policy
unless the operator supplies the exact same override; a newly derived source default cannot replace
the durable publication target.

## Near-term architecture work

These categories are not a live priority list. Use ECorp Build Project #3 and linked issues for
ordering and status.

1. Add stronger OS/container isolation for untrusted child processes.
2. Add artifact retention sweeping and signing-key rotation.
3. Add multi-region control-plane and object-store recovery drills.
