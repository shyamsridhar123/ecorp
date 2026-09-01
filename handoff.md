# ECorp / Crony Corp handoff

Updated: September 1, 2026

Repository: this checkout of `shyamsridhar123/ecorp`

Remote: [shyamsridhar123/ecorp](https://github.com/shyamsridhar123/ecorp)

## Executive summary

ECorp is an active alpha for running multiple coding-agent runtimes against a Git repository under
one durable control plane. It combines the strongest ideas explored in Munder Difflin and Buzz:
multi-agent orchestration, a shared multiplayer operations surface, isolated Git worktrees, live
human control, durable approvals, bounded budgets, and evidence-gated completion.

The public product name is **ECorp**. The repository folder, Rust crates, environment variables, and
some protocol headers still use `crony` for compatibility.

The **ECorp Build GitHub Project** and its linked issues are the operational source of truth for
priority, status, sequencing, parity decisions, and release gates. `docs/BACKLOG.md` is historical
seed material only.

This handoff is based on `main` commit
`67705f8d7a6da69171bea798b6c756423af50977` and includes the September 1 enterprise-dogfood
hardening and evidence prepared on `fix/enterprise-dogfood-hardening`. The local application stack
is stopped; the Docker engine is available.

## Current verified snapshot

| Item | State on September 1, 2026 |
| --- | --- |
| Working branch | `fix/enterprise-dogfood-hardening` |
| Base commit | `67705f8d7a6da69171bea798b6c756423af50977` |
| Base synchronization | `main` matched `origin/main` when the branch was created |
| Repository visibility | Private |
| Operational backlog | **ECorp Build** GitHub Project and linked issues |
| Open issues | #27 and #48 through #56 |
| In progress | #49, #51, and differentiation tracking issue #54 |
| Web console | Down; port `5187` is not listening |
| Control plane | Down; port `8791` is not listening |
| Mario game server | Down; port `8080` is not listening |
| Docker | Engine available; server version `29.6.2` |
| Enterprise dogfood | Recorded in `docs/evidence/2026-09-01-enterprise-application-dogfood.md` |
| GitHub Actions | The latest baseline run ended before repository jobs executed; inspect live Actions state |

## What is implemented

### Product and multiplayer control surface

- React operations console with a mission composer, live control floor, activity feed, rooms,
  approvals, artifacts, runner status, model selection, budgets, and agent inspection.
- Durable missions, tasks, runs, messages, events, approvals, leases, artifacts, provider sessions,
  and runner state stored in PostgreSQL.
- Resumable WebSocket event delivery and multi-human actor views.
- Live steering, queued messages, control-lease claim/release/transfer, interruption, and emergency
  stop.
- Mission planning modes for one focused agent, two specialists followed by synthesis, verifier
  matrices, manual review, and intentional failure-path testing.

### Agent execution

- Outbound runner daemon owns provider processes and worktrees. The server does not run agent shell
  commands.
- Each write-capable run receives a dedicated branch and linked Git worktree.
- Implemented adapters:
  - GitHub Copilot through `github-copilot-sdk` `1.0.11`
  - OpenAI Codex app-server lifecycle
  - Claude Code normalized CLI lifecycle
  - OpenCode normalized CLI lifecycle
  - deterministic `fake-process` systems harness
- Provider-independent lifecycle events normalize start, stream, steer, interrupt, stop, resume,
  usage, failure, artifacts, and evidence.

### Safety and verification

- Bounded task graphs, attempts, depth, fan-out, tokens, cost, repeated tools, and no-progress
  behavior.
- Durable risky-action approvals and exactly-once decision effects.
- OIDC production mode, Corp-scoped RBAC, runner enrollment, rotating credentials, replay defense,
  and scoped secret brokering.
- Content-addressed artifact storage, SHA-256 integrity, media validation, signed provenance, and
  authorized download.
- Verification policies can require artifacts, files, commands, tests, schemas, screenshots,
  independent review, or human approval before accepting completion.

## September 1 enterprise application dogfood

One operator ran three realistic application builds through the browser, control plane, PostgreSQL,
outbound runner, provider adapter, and isolated-worktree path. This is strong internal systems
evidence, but it does not satisfy #27's requirement for three external human teams.

- **VendorGuard / GitHub Copilot:** the generated application passed 13 of 13 tests and the browser
  vendor-submission, review, tenant-isolation, RBAC, and audit workflow. The mission crossed its
  500,000-token budget and could not recover because no authorized budget revision exists. Tracked
  by #50.
- **Incident Command / Codex:** the application passed 12 of 12 tests and the browser
  incident-lifecycle, SSE, RBAC, tenant-isolation, and audit workflow. The persisted run exposed a
  race that accepted completion after a stop-stage budget breaker. The local correction fences
  post-breaker progress, emits unique Codex API-call usage increments while the turn is active,
  blocks human completion bypasses, and prevents hard-stop retries; deterministic budget E2E
  passes. A fresh real-provider Codex run is still required before #49 closes.
- **Credit Exception / Claude Code:** the initial provider inherited user plugins, MCP servers,
  hooks, browser integration, and memory, contaminated the configured source checkout, and left an
  orphan descendant. Claude now launches in isolated safe mode on this branch, but provider
  permission bridging and descendant-process verification remain open in #51. The incomplete
  application failed 84 tests with two errors and was not accepted.

Cross-cutting product gaps are tracked by #48 for mission-scoped dynamic staffing, #52 for rich
mission contracts and verifier policies, #53 for portable merge-ready deliverables, and #56 for
aggregate hard-budget fan-out across concurrent runs.

## UX contract

The office is a projection of authoritative server and runner state, not a decorative game layer.

1. Connect a runner to a repository.
2. Choose an available adapter, model, reasoning effort, strategy, and budget.
3. Enter a concise mission title. Rich specifications and operator-authored verifier policies are
   tracked in #52.
4. Select **Plan and run mission**, or enable **Pause after planning** to inspect contracts before
   selecting **Dispatch mission**.
5. Watch active work, steer the lease holder, decide approvals, and inspect verification.
6. Download accepted, signed evidence artifacts. Portable source archives, patches, and commits are
   tracked in #53.

Idle identities may appear in the roster, but they do not represent live provider processes.
Starting, working, reviewing, blocked, completed, failed, and cancelled visuals should always map to
real persisted state.

## Harness terminology

### Deterministic systems harness

The `fake-process` harness launches a real local child process and exercises the real
server-to-runner path. It is quota-free and contains no AI. Use it to test orchestration, dispatch,
worktree isolation, approvals, budgets, verification, cleanup, and failure handling. Do not use it
to judge agent quality.

### GitHub Copilot fixture

The CI-only Copilot fixture exposes three synthetic model states so tests can validate model
discovery, disabled-policy handling, reasoning selection, persistence, evidence, and resume without
using provider quota.

### Real GitHub Copilot adapter

The real adapter discovers the model catalog enabled for the currently signed-in GitHub Copilot
account. It does **not** promise every model that exists globally. ECorp can offer only models and
reasoning levels returned by the account, policy, SDK, and connected runtime.

The authenticated probe recorded on August 30, 2026 returned 25 models, selected `gpt-5-mini`, wrote
`copilot-live-proof.txt` in an isolated worktree, and completed with verified evidence. That catalog
is historical and must be rediscovered on each new run.

## Baseline work already on `main`

| Commit | Result |
| --- | --- |
| `67705f8` | Restored immutable migration history and added migration-manifest enforcement |
| `c04b167` | Hardened dispatch, UI operations, lifecycle cleanup, identity, approvals, budgets, and evaluation paths |
| `4418e9c` | Changed README images to repository-relative paths |
| `2996a26` | Added the Relay pixel-art mascot and live-agent control-room hero image |
| `dd89843` | Rewrote the README and repository positioning around truthful product capability |
| `6b4f562` | Prevented Copilot missions from being stopped prematurely |
| `69ee484` | Fixed deterministic mission dispatch |

The repository description and topics are populated. The repository is still private, so the
README, images, and project cannot attract public traffic until the owner deliberately changes
visibility.

## README image status

The README now uses:

- `./docs/assets/ecorp-relay-mascot.png`
- `./docs/assets/ecorp-control-room.png`

Both PNG files and the mascot SVG are tracked on `main`. This avoids private-repository raw URL
redirect problems for authenticated GitHub viewers. Unauthorized viewers still cannot see assets
from a private repository.

## GitHub project and backlog

The **ECorp Build GitHub Project** and linked issues are authoritative. Its Project README records
the product position:

> **Munder makes agents delightful. Buzz makes agents collaborators. ECorp makes agent work
> governable.**

Current sequencing:

1. **In progress:** #49 hard-breaker completion fencing, #51 provider isolation and permissions,
   and differentiation tracking issue #54.
2. **Remaining P0:** #50 authorized budget revision, #56 aggregate budget fan-out, and #48
   mission-scoped dynamic agents.
3. **Outcome pipeline:** #52 rich contracts and verifier policies plus #53 portable merge-ready
   deliverables.
4. **External validation:** #27 three real teams.
5. **Sequenced P1:** #55 provenance-scoped institutional memory.

`docs/BACKLOG.md` is retained only as the historical seed for the initial milestones.

## GitHub Actions status

GitHub Actions is used because the repository needs repeatable:

- Rust formatting, linting, and workspace tests;
- web build and lint checks;
- live control-plane-to-runner integration tests;
- deterministic 100-scenario evaluation and chaos evidence;
- Windows, macOS, and Linux runner-contract tests; and
- Windows Tauri desktop builds.

The latest baseline workflow for `67705f8` ended before repository jobs executed. This is not
evidence that the code or tests failed. Inspect the live Actions annotations, resolve the external
runner/account blocker, rerun CI, and treat the rerun as the remote quality gate.

## Mario / Crony Kingdom use case

The previous GitHub Copilot mission did not produce an accepted deliverable:

- Mission `8c0b32f3-9992-41e6-8ee0-06f15b3acace` was cancelled after reaching 221,259 tokens against
  a 200,000-token run budget.
- Its run had no accepted artifact ID or artifact URI.
- The recorded game worktree and branch are now absent.
- A follow-up QA mission was cancelled by the repeated-tool circuit breaker and also produced no
  artifact.
- Port `8080` is currently down.

The JSON snapshots remain in:

- `output/mario-mission-live.json`
- `output/mario-mission-needs-approval.json`
- `output/mario-qa-mission-live.json`

Do not present the Mario game as completed or recoverable. Rerun it as a new mission after the stack
is healthy, use a narrower contract or larger budget, require browser evidence, and download or
commit the accepted artifact before resetting demo state.

## Start the complete local stack

Prerequisites: Docker Desktop, Rust, Node.js, and pnpm. Real-provider missions also require the
corresponding authenticated runtime.

```powershell
cd path\to\ecorp
./tools/start_local.ps1
```

The script stops processes recorded in the stale PID file, starts PostgreSQL, builds the Rust
server and runner, enrolls the runner, starts Vite, and waits for the full stack.

Expected endpoints:

- Web console: `http://127.0.0.1:5187`
- Server health: `http://127.0.0.1:8791/health`

To run against another repository:

```powershell
$env:CRONY_SOURCE_REPOSITORY = 'C:\path\to\repository'
$env:CRONY_SOURCE_BASE_REF = 'HEAD'
./tools/start_local.ps1
```

To stop the local processes:

```powershell
./tools/stop_local.ps1
```

## Clear demo missions for a manual walkthrough

After the server is running:

```powershell
Invoke-RestMethod `
  -Method Post `
  -Uri 'http://127.0.0.1:8791/api/demo/reset' `
  -ContentType 'application/json' `
  -Body '{}'
```

The reset deletes demo control leases, queued messages, room messages, runs, tasks, missions, and
events; returns demo agents to idle; and bootstraps a fresh demo Corp. It does not delete filesystem
worktrees or artifacts under `output/`.

## Validation commands

Run the repository quality gate:

```powershell
pnpm install --frozen-lockfile
pnpm check
```

For user-visible changes, also run the full stack and exercise the real browser-to-server-to-runner
path. Unit tests do not prove mission dispatch or the multiplayer UI.

Useful focused checks:

```powershell
node tools/check_migrations.mjs
node tools/e2e_copilot.mjs
node tools/e2e_worktrees.mjs
node tools/e2e_task_graph.mjs
node tools/e2e_verification.mjs
```

These E2E scripts expect the appropriate server, runner, database, and fixture configuration. Read
each script before running it as a standalone command.

## Known risks and gotchas

1. **Do not recursively delete `output/` or all registered worktrees.** There are hundreds of
   mission worktrees. The product deliberately preserves dirty, committed, or unverifiable work.
   Inspect each cleanup candidate and its resolved path first.
2. **Do not equate a persisted employee identity with a running agent process.** The UI must remain
   a projection of live state.
3. **Do not call the deterministic harness an AI agent.** It is a systems fixture.
4. **Do not claim Copilot supports every global model.** Discovery is account- and policy-specific.
5. **Do not claim CI is green.** The latest hosted workflow did not execute repository jobs.
6. **Do not claim the Mario use case succeeded.** It was cancelled and has no accepted artifact.
7. **Do not edit the configured source checkout from a mission.** All write-capable agent work must
   stay inside its assigned worktree.
8. **Do not clean up a worktree merely because a run ended.** Preserve it when it contains changes
   or cannot be verified safely.
9. **Do not close #49 on deterministic evidence alone.** The local hard-breaker correction still
   requires a fresh real Codex usage run.
10. **Do not present budget suspension as recoverable yet.** #50 must add an authorized revision or
    bounded re-scope path before resume can work after the mission ceiling is crossed.
11. **Do not treat Claude permissions as integrated ECorp approvals.** Safe-mode isolation is only
    a partial #51 fix; shell permissions and descendant cleanup remain unresolved.
12. **Do not equate provider metadata with a usable application result.** #52 and #53 must bind a
    durable mission contract, executed checks, and the exact exported source bytes or commit.
13. **Do not treat aggregate budgets as fully enforced across concurrent work.** #56 must fan out
    mission, requester, and Corp hard breakers to every active run in scope.

## Recommended next actions

1. Resolve the external GitHub Actions blocker and rerun CI for the dogfood-hardening branch.
2. Run a fresh real-provider Codex budget probe and close #49 only if stop remains monotonic.
3. Complete #51's isolated provider home, permission bridge, and descendant-process verification.
4. Implement #50's authorized budget revision and bounded finish-contract flow.
5. Implement #56 so aggregate hard budgets fence every concurrent run in scope.
6. Implement #48 so missions provision only the bounded roles they require.
7. Deliver #52 and #53 together so the exact operator contract gates the exact portable bytes.
8. Complete #27 with three external teams and measured setup, recovery, rework, and intervention.
9. Add #55's provenance-scoped institutional memory after the P0 integrity work.
10. Revisit broad Munder Difflin or Buzz surface parity only through the evidence gate in #54.

## Source map

- Product plan: `docs/PRODUCT_AND_TECHNICAL_PLAN.md`
- User and developer journey: `docs/USER_AND_DEVELOPER_JOURNEY.md`
- Architecture: `docs/ARCHITECTURE.md`
- Security boundaries: `docs/SECURITY.md`
- Threat model: `docs/THREAT_MODEL.md`
- Evaluation strategy: `docs/EVALS.md`
- Enterprise dogfood evidence: `docs/evidence/2026-09-01-enterprise-application-dogfood.md`
- ADRs: `docs/adr/`
- Evidence records: `docs/evidence/`
- Web UI: `apps/web/src/App.tsx`, `apps/web/src/App.css`
- Control plane: `crates/crony-server`
- Task planning: `crates/crony-server/src/planning.rs`
- Runner and adapters: `crates/crony-runner`
- Durable store: `crates/crony-store`
- Protocol gateways: `crates/crony-gateways`
- CLI: `crates/crony-cli`
- Desktop shell: `apps/desktop`
- Local startup: `tools/start_local.ps1`
- GitHub CI: `.github/workflows/ci.yml`
