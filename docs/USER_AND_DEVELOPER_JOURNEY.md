# ECorp user and developer journey

Updated: September 3, 2026

ECorp turns a repository-level outcome into isolated agent work, human decisions, and verified
artifacts. The office is a live projection of that workflow; it is not a separate game layer or the
system of record.

The browser is organized as five arcade-cabinet views: **Control floor**, **Factory**,
**Missions**, **Comms**, and **Audit**. Only one primary view is rendered at a time. A persistent
operations HUD keeps live runs, pending decisions, active factory work, and verified results
visible without stacking every subsystem into one page. Hash links and desktop deep links select
the appropriate view before focusing the requested room, mission, task, or run.

On desktop, a compact cartridge rail switches cabinets while the selected workspace owns the
remaining screen. The Control floor keeps authoritative agent sprites in the playfield, including
truthful off-shift identities. Factory uses one issue queue and one full workbench dossier. Missions
uses one cartridge list and one active quest dossier. Mobile keeps the same model with a
horizontally scrollable cabinet rail and no document-level horizontal overflow.

## The four-step user journey

### 1. Connect a runner

A runner is the trusted process that owns provider sessions and Git worktrees. The web UI can be
closed without stopping it.

For local development:

```powershell
./tools/start_local.ps1
```

Open `http://127.0.0.1:5187`. The start guide should show the control plane, runner, source
repository, and available AI runtimes.

To target another repository:

```powershell
$env:CRONY_SOURCE_REPOSITORY = 'C:\path\to\your\repository'
$env:CRONY_SOURCE_BASE_REF = 'HEAD'
./tools/start_local.ps1
```

Every write-capable run receives a linked worktree below `CRONY_RUNNER_WORKSPACE`. ECorp does not
let an agent edit the configured source checkout directly.

### 2. Choose the crew

The mission composer lists only adapters reported as available by a connected runner.

- **GitHub Copilot** discovers the models enabled for the signed-in Copilot account.
- **OpenAI Codex**, **Claude Code**, and **OpenCode** use their locally configured runtimes.
- **Test harness** is deterministic, quota-free, and contains no AI. Use it to test ECorp behavior,
  not to judge agent quality.

For providers with model catalogs, choose a model and supported reasoning effort. Leaving the model
blank uses that provider's default.

### 3. Plan and run a mission

A mission is an outcome with a task contract, not a chat message. State the expected result and the
evidence that should prove it.

The arcade-style composer keeps one decision screen visible at a time:

1. **Mission:** short outcome plus the complete durable specification.
2. **Loadout:** runtime, model, budget, deliverable, and orchestration pattern.
3. **Win conditions:** tabbed outcome, guardrail, context, and verifier controls.

Win conditions accept explicit objectives, expected output, acceptance criteria, allowed tools,
prohibited actions, repository or issue references, approved context sources, and
worktree-relative write scope. Enable **Custom victory gates** to add typed artifact, file, command,
test, JSON-schema, or screenshot checks and an optional human-approval or independent-review gate.
Enabling it pauses after planning by default. Expand each planned task to inspect the exact
persisted completion plan before dispatch. After a mission is created, the composer collapses to a
single **New mission** control so the active mission log remains primary.

Choose the portable result before dispatch:

- **Source archive** packages changed tracked and non-ignored untracked files with hashes.
- **Git patch** produces deterministic binary patch bytes.
- **Typed artifact set** includes typed file entries and content hashes.
- **Verified commit and branch bundle** creates a post-verification commit only in the task
  worktree and includes portable source bytes.
- **Review-only report** records verification and change metadata without source content.

The optional commit toggle never publishes or merges. Pull-request publication and merge are
separate authorized effects.

By default, **Plan and run mission** creates the task graph and immediately dispatches ready tasks.
Enable **Pause after planning** when a human should inspect the generated graph before selecting
**Dispatch mission**.

Before a ready mission's first run, an authorized operator can save a versioned `redispatch`
contract revision and then explicitly dispatch it. After a failed, cancelled, or lost provider
attempt preserves a resumable session and worktree, an authorized operator can save a bounded
`resume` revision and then explicitly resume that source run. The revision form shows the complete
typed contract and verifier JSON; revision history preserves the actor, reason, action, source run,
and version.

Strategy meanings:

- **One agent delivers the outcome:** focused builds, fixes, and reviews.
- **Two specialists, then synthesis:** independent approaches followed by a bounded synthesis task.
- **Verification matrix:** deterministic verifier coverage.
- **Human approval / independent review:** completion pauses for an authorized decision.
- **Verification failure demo:** intentionally exercises the failure path.

### 4. Operate and review

The shared office maps authoritative agent state to visible behavior:

- idle identities move off shift and remain only in the available roster; no provider process is
  running for them;
- starting agents walk;
- working agents type;
- reviewing agents move to the review table;
- blocked agents move to the approval desk;
- offline identities show that their required runner or adapter is unavailable.

Select a sprite to inspect the agent. Claiming control grants the live steering lease. People without
the lease can still queue a note. Authorized operators can transfer or release control, interrupt a
turn, or issue an emergency stop.

When a run becomes completed, failed, or cancelled, its adapter disconnects or stops the provider,
the runner removes the run from its active-process map, and the employee identity returns off shift.
ECorp preserves the identity and resumable session metadata so future work can be scheduled without
leaving an operating-system process alive.

Risky commands create durable approval records. After verification passes, the mission card exposes
provider evidence, verification evidence, the signed source deliverable, and integration state as
four distinct records. Room members can download the actual deliverable without runner-host path
access. The room and immutable activity feed preserve the collaboration and replay trail.

### 5. Publish a verified factory result for review

An owner, admin, or manager can run `crony factory-publish <corp> <actor> <work-item>`. The trusted
CLI derives the exact ready commit/branch deliverable, target, source issue, title/body, authorization
record, and idempotent effect key from the durable snapshot. It uses the operator's existing GitHub
credential helper without exposing that credential to the agent or server.

Retries do not create another branch or pull request. The UI shows the target, base, branch, commit,
authorization, attempt history, failure detail, pull-request link, and Project transition. Project
status enters review only after the pull request exists. Publication never enables auto-merge and
does not merge or deploy.

## The developer journey

### Local process topology

`tools/start_local.ps1` starts:

1. PostgreSQL through Docker Compose.
2. `crony-server` on `127.0.0.1:8791`.
3. `crony-runner`, enrolled through an expiring token and then a rotating credential.
4. The Vite web client on `127.0.0.1:5187`.

The browser talks only to the server. The server persists domain state and sends fenced assignments
over the outbound runner WebSocket. The runner starts the provider adapter in an isolated worktree
and streams normalized lifecycle events back.

### Where to change each part

- Web journey and office projection: `apps/web/src/App.tsx` and `apps/web/src/App.css`
- HTTP/WebSocket control plane: `crates/crony-server`
- Task planning and bounds: `crates/crony-server/src/planning.rs`
- Provider execution and worktree lifecycle: `crates/crony-runner`
- Shared domain state: `crates/crony-domain`
- Network contracts: `crates/crony-protocol`
- Persistence and event journal: `crates/crony-store`

### Verification loop

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

For user-visible changes, also start the complete local stack and exercise the browser-to-server-to-
runner path. A static build or unit test does not prove the product journey works.
