# ECorp user and developer journey

Updated: September 6, 2026

ECorp turns a repository-level outcome into isolated agent work, human decisions, and verified
artifacts. The office is a live projection of that workflow; it is not a separate game layer or the
system of record.

Start with the [interactive front office](https://ecorp-front-office.shyam-sridhar16.chatgpt.site)
for the product story and illustrative tour. That separate website does not run agents or connect
to a repository. This guide covers the actual ECorp application.

The browser is one arcade game shell with five focused cabinets: **Control floor**, **Factory**,
**Missions**, **Comms**, and **Audit**. Only one primary view is rendered at a time. A compact score
rail keeps live runs, pending decisions, active factory work, and verified results visible without
stacking every subsystem into one page. Hash links and desktop deep links select the appropriate
cabinet before focusing the requested room, mission, task, or run.

A fixed bottom control dock switches cabinets while the selected workspace owns the screen. The
Control floor is a coherent sprite-art world rather than a framed dashboard. Non-retired
agent identities remain selectable, including truthful off-shift identities. Retired mission
workers stay in historical missions and runs rather than occupying the current floor. Agent
details and controls open as a dismissible command HUD instead of consuming a permanent column.
Factory uses one issue queue and one full workbench dossier. Missions uses one cartridge list and
one active quest dossier. Mobile keeps the same game-world model with a bottom dock, a command
sheet, and no document-level horizontal overflow.

## The five-step user journey

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

In **Describe & setup**, select and confirm the exact source repository, symbolic ref, and immutable
commit advertised by a connected runner. The mission composer then lists only compatible adapters,
models, and reasoning levels for that source.

- **GitHub Copilot** discovers the models enabled for the signed-in Copilot account.
- **OpenAI Codex** uses its locally configured app-server runtime.
- **Claude Code** and **OpenCode** use normalized external-CLI runtimes on Windows. Their Unix
  adapters are currently disabled because equivalent descendant-process containment is not proven.
- **Test harness** is deterministic, quota-free, and contains no AI. Use it to test ECorp behavior,
  not to judge agent quality. It appears only after explicitly enabling **Developer fixtures**
  inside **Model, limits and output**; it is never the fallback for an ordinary application build.

For providers with model catalogs, expand **Model, limits and output** to choose a model and
supported reasoning effort. Leaving the model blank uses that provider's default.

ECorp can provision mission-owned workers when the accepted graph is saved; a proposal alone does
not create a crew. **Studio team · 3 Copilot agents** requires a compatible GitHub Copilot runner.
It creates visual, gameplay, and quality workers, with a later integration task assigned to the
gameplay worker. The development browser starts without a fixed demo cast; explicit legacy fixtures
remain available for testing.

### 3. Plan and run a mission

A mission is an outcome with a task contract, not a chat message. State the expected result and the
evidence that should prove it.

The light retro composer has two steps:

1. **Describe & setup:** enter the goal, confirm the repository/ref/commit, and choose the coding
   agent and team. Additional specification, model, budget, and output settings are optional
   disclosures rather than separate required screens.
2. **Review & build:** inspect the exact destination, current server-derived allocation and
   completion checks, then **Build**. Detailed requirements remain available on demand.

Verification accepts explicit objectives, expected output, acceptance criteria, allowed tools,
prohibited actions, repository or issue references, approved context sources, and
worktree-relative write scope. Enable **Custom verification** to add typed artifact, file, command,
test, JSON-schema, or screenshot checks and an optional human-approval or independent-review gate.
Enabling or editing checks does **not** change the separately selected launch-vs-save intent.
Expand each planned task to inspect the exact persisted completion plan before dispatch.
**Close setup** returns to the selected existing mission without creating work; the unsaved
draft stays on the current page. While setup is open, old mission evidence is not mixed into the
form. After creation, the new mission becomes the selected work item.

Use **Model, limits and output** to change the portable result before dispatch:

- **Source archive** packages changed tracked and non-ignored untracked files with hashes.
- **Git patch** produces deterministic binary patch bytes.
- **Typed artifact set** includes typed file entries and content hashes.
- **Verified commit and branch bundle** creates a post-verification commit only in the task
  worktree and includes portable source bytes.
- **Review-only report** records verification and change metadata without source content.

The optional commit toggle never publishes or merges. Pull-request publication and merge are
separate authorized effects.

By default, **Build** creates the task graph and requests dispatch. Explicitly enable **Save without
starting** to use **Save plan** instead. Its persisted **Awaiting dispatch** state survives
closing the browser and restarting the server; only an explicit authorized **Dispatch mission**
can release the first run. Normal dependency release and retries apply only after launch.

Before a ready mission's first run, an authorized operator can save a versioned `redispatch`
contract revision and then explicitly dispatch it. After a failed, cancelled, or lost provider
attempt preserves a resumable session and worktree, an authorized operator can save a bounded
`resume` revision and then explicitly resume that source run. The revision form shows the complete
typed contract and verifier JSON; revision history preserves the actor, reason, action, source run,
and version.

Strategy meanings:

- **Solo run:** focused builds, fixes, and reviews.
- **Two specialists and synthesis:** independent approaches followed by a bounded synthesis task.
- **Studio team · 3 Copilot agents:** three parallel handoff tasks, then integration by one of those
  workers after all three parents pass verification and any required review. Integration consumes
  signed, verified handoff artifacts, never another worker's live worktree.
- **Verification matrix:** deterministic verifier coverage.
- **Human approval / independent review:** completion pauses for an authorized decision.
- **Failure path:** intentionally exercises a verification failure.

The deterministic strategies appear under **Developer fixtures**. A fixture or planned graph is
not evidence of real provider execution. For the studio path and its exact evidence scope, read
the [mission-staffing report](evidence/2026-09-06-mission-staffing.md).

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
turn, or issue an emergency stop. The command HUD closes through its visible control, its scrim, or
the Escape key without ending or changing the agent process.

When a run becomes completed, failed, or cancelled, its adapter disconnects or stops the provider,
the runner removes the run from its active-process map, and the employee identity returns off shift.
ECorp preserves the identity and resumable session metadata without leaving an operating-system
process alive. Unpinned mission workers retire after terminal missions only when there is no active
run, control lease, queued message, approval, durable command, or teardown uncertainty. An
authorized resume can reactivate the preserved worker. Dedicated Pin/Unpin, Clear crew, and manual
Retire controls remain tracked in [#48](https://github.com/shyamsridhar123/ecorp/issues/48).

Risky commands create durable approval records. After verification passes, the mission card exposes
provider evidence, verification evidence, the signed source deliverable, and integration state as
four distinct records. Room members can download the actual deliverable without runner-host path
access. The room and immutable activity feed preserve the collaboration and replay trail.

### 5. Publish a verified factory result for review

An owner, admin, or manager can run `crony factory-publish <corp> <actor> <work-item>`. The trusted
CLI derives the exact ready commit/branch deliverable, target, source issue, title/body, authorization
record, and idempotent effect key from the exact Corp- and room-authorized publication context,
not a bounded browser snapshot. An owner/admin must first enroll a short-lived publisher workload
credential. Pass its identifier and credential-file path to the CLI; GitHub authentication remains
in the trusted publisher, not the producing agent or server. Follow the complete
[publisher setup and cleanup procedure](DARK_FACTORY_CONTRIBUTOR_GUIDE.md) rather than treating the
positional command alone as sufficient authority.

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

When `ECORP_FACTORY_WATCH=1`, startup also starts the configured trusted GitHub Project watcher.
Its heartbeat and pause/resume state are independent from individual missions. Without that
configuration, the UI reports that Factory is not configured.

The browser talks only to the server. The server persists domain state and sends fenced assignments
over the outbound runner WebSocket. The runner starts the provider adapter in an isolated worktree
and streams normalized lifecycle events back.

### Where to change each part

- Web journey and mission controls: `apps/web/src/App.tsx` and `apps/web/src/App.css`
- Office projection and worker inspection: `apps/web/src/OfficeFloor.tsx` and `apps/web/src/OfficeInspector.tsx`
- HTTP/WebSocket control plane: `crates/crony-server`
- Task planning and bounds: `crates/crony-server/src/planning.rs`
- Provider execution and worktree lifecycle: `crates/crony-runner`
- Shared domain state: `crates/crony-domain`
- Network contracts: `crates/crony-protocol`
- Persistence and event journal: `crates/crony-store`

### Verification loop

```powershell
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

For user-visible changes, also start the complete local stack and exercise the browser-to-server-to-
runner path. A static build or unit test does not prove the product journey works.
