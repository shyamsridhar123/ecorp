# Evaluation and real-world testing

## Evidence rule

An implementation claim needs evidence at the same scope:

- code compiles
- targeted tests pass
- the real service starts
- the browser reaches the server
- the server reaches the runner
- the runner starts a child process
- the process creates an artifact
- Postgres records state and events
- the browser receives the final state

## First vertical-slice scenario

1. Start Postgres, server, runner, and web client.
2. Bootstrap the demo Corp.
3. Open Alice and Bob in separate browser tabs.
4. File a mission.
5. Dispatch it.
6. Confirm the runner launches a real child process.
7. Confirm live status events reach both tabs.
8. Confirm one operator can hold the agent control lease.
9. Confirm a second operator cannot replace an unexpired lease.
10. Send live direction to the active run.
11. Confirm the child process acknowledges it.
12. Confirm `result.md` exists.
13. Recompute and compare its SHA-256.
14. Confirm mission, task, and run reach `completed`.
15. Restart a browser and confirm state remains.

## Required chaos cases

- duplicate runner event
- server restart during an active run
- runner disconnect and reconnect
- expired lease
- two simultaneous lease claims
- duplicate mission launch
- child exits without terminal event
- child emits invalid JSON
- artifact missing after artifact event
- path containing spaces
- Postgres unavailable
- browser reconnect

## Terminal provider cleanup

`node tools/e2e_idle_cleanup.mjs` proves that a worker is visible only while its run is active, that
the runner emits `run.session_terminated` before verification and accepted completion, and that the
persistent agent identity returns to `idle` with no `current_run_id`.

`cargo test -p crony-runner process_tree -- --nocapture` creates a real parent/grandchild fixture
inside `OwnedProcessTree`. It covers a stubborn descendant, an already-exited descendant,
idempotent repeated termination, and an unrelated process that must remain alive. The external
adapter tests drive both interrupt and stop through the same ownership boundary. Windows exercises
the Job Object implementation; Unix unit coverage verifies that external CLI capabilities fail
closed rather than claiming process-group containment. A release candidate must additionally rerun
the real Claude parent/grandchild probe recorded in
`docs/evidence/2026-09-03-external-provider-process-tree.md`; unit success alone is insufficient.

## External provider permission bridge

The Claude Code adapter now uses Claude's supported stream-JSON `can_use_tool` request/response
protocol, manual permission mode, and required initialize handshake to translate tool requests into
durable ECorp approvals. Focused tests cover correlated allow/deny responses, bounded hashed
approval context, contained-path auto-allow behavior, rejected and expired decisions, and
provider-response write failure. See
`docs/evidence/2026-09-03-claude-permission-bridge.md`.

Together, the permission-bridge and process-tree evidence prove mediated tool authorization and
fail-closed Windows descendant cleanup. Issue #51 continues to track isolated provider homes,
inherited-environment allowlisting, stronger container isolation, and remaining real-provider
recovery drills.

## Quality gates

### Held mission / launch-admission regression

`tools/e2e_launch_admission.mjs` exercises saved ordinary and materialized factory plans,
unrelated completion/failure, explicit release, duplicate/concurrent requests, failed first
dispatch, fresh continuation run IDs, and normal graph dependencies/retries. Its `prepare` and
`release` phases let an external test supervisor restart the actual server and runner without
resetting their database or source.

`tools/e2e_launch_admission_browser.mjs` uses the actual mission composer, repository confirmation,
**Hold at briefing**, and **Dispatch mission** controls. It closes the browser before unrelated
work completes and checks the saved plan again after the supervisor's restart. It also checks
390-pixel layout and a successful, nonduplicating launch replay.

Use an explicitly owned isolated stack and independent fixture repository, never the manual
demo. The scripts reject shared/manual ports. The runner must use the deterministic process and
the existing Codex/Claude/OpenCode protocol fixtures, not authenticated real providers. Install
Playwright separately for the browser regression or set `CRONY_PLAYWRIGHT_MODULE` to its installed
package directory.

```powershell
$env:CRONY_ADMISSION_TEST = '1'
$env:CRONY_SERVER_HTTP = 'http://127.0.0.1:18961'
$env:CRONY_ADMISSION_WEB = 'http://127.0.0.1:15496'
$env:CRONY_ADMISSION_OUTPUT = 'C:\path\to\owned-qa-evidence'
$env:CRONY_ADMISSION_GRAPH_FIXTURES = '1'
node tools/e2e_launch_admission.mjs --phase prepare
node tools/e2e_launch_admission_browser.mjs --phase prepare
# Restart only the owned QA server and runner; preserve database, credentials and source.
node tools/e2e_launch_admission_browser.mjs --phase release
node tools/e2e_launch_admission.mjs --phase release
```

The supervisor must separately record actual old/new process identities and reconnection;
two invocations alone do not prove a restart. Retain checkpoint IDs after an interrupted phase
rather than resetting data or silently creating replacement missions. These are deterministic
control-plane/browser checks, not evidence of real Copilot inference or a three-agent game build.

### Mission staffing regression

`tools/e2e_mission_staffing.mjs` is an explicitly opted-in, isolated-stack regression.
It requires a fresh owned development database, an independent source repository, a
real runner using `fake-process`, and the explicit Copilot fixture for read-only
studio planning. It rejects default/manual ports and preserves every checkpoint.

It proves empty-crew bootstrap, exact mission-owned staffing, held plans, model/source
and guest rejection without orphan identities, mutation-free factory preflight,
materialization replay, and terminal worker retirement with historical attribution.
It executes only one deterministic worker; held studio plans are not a real Copilot
or concurrency proof.

```powershell
$env:CRONY_STAFFING_TEST = '1'
$env:CRONY_SERVER_HTTP = 'http://127.0.0.1:18962' # independently owned test stack only
$env:CRONY_STAFFING_OUTPUT = 'C:\path\to\new-evidence-directory'
node tools/e2e_mission_staffing.mjs
```

Real studio evidence must separately establish three actual Copilot sessions working
concurrently, four isolated task worktrees, the selected repository/ref/commit, every
verified handoff in the integration receipt, persisted application checks and authorized
review before publication. Development Alice/Bob identities are test principals, not
evidence of separate human GitHub authentication. Never label a fixture, a planned graph,
an approval script, or a provider completion claim as that full proof.

`tools/verify_arcade_browser.mjs` is an optional trusted runner verifier for a standalone
game contract, not a provider tool. It requires a Git worktree and an explicitly supplied
installed Playwright module, exercises keyboard-activated controls at desktop/390px,
blocks remote HTTP requests, and produces real screenshots and a structured report.
Pin its source digest in the persisted command policy when using it for a factory run.

### Factory polling and throttling

`tools/e2e_factory_polling.mjs` exercises complete discovery beyond 1,000 items,
zero-quota admission, durable retry timing, an actual controller restart and a
forced reconciliation that must not bypass the wait. It uses an owned
development server/runner and `tools/fake_github_cli.mjs`, not a real provider
or live GitHub mutations. Every process and POST intent is recorded; existing
checkpoints are preserved rather than reset.

The fake GitHub boundary supports explicit GraphQL quota, cursor pages,
primary/secondary/503 failures, Retry-After/reset headers and timestamped query
counters. Its configuration is documented at the top of the fixture script.
`tools/fake_github_quota.test.mjs` validates this boundary independently.

`tools/e2e_factory_polling_browser.mjs` reads the real Factory notice at desktop
and 390px. It allows only the normal idempotent development bootstrap handshake;
other mutation requests and response/page injection are forbidden. Zero is
required in the low-quota case, not in a secondary limit with healthy primary
quota. `tools/e2e_factory_polling_recovery.mjs` can verify the exact preserved
mission/item/run after an interrupted harness. `tools/e2e_factory_polling_notice.mjs`
checks a controlled 503 notice without creating work.
`tools/e2e_factory_source_drift.mjs` changes the controlled issue revision/body
after claim and requires a blocked item, held mission and zero added runs.

Use explicit `CRONY_POLLING_TEST=1`, owned `CRONY_SERVER_HTTP`/`CRONY_POLLING_WEB`,
an evidence output directory, independent source repository and the candidate CLI
binary. Never point these scripts at the canonical manual-test stack. Test-worker
delays must remain labeled deterministic fixture timing, not model concurrency
or real-provider performance.

### Workspace validation commands

```powershell
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

The repository also contains `tools/e2e_smoke.ps1`, which exercises the actual running stack.
`tools/e2e_demo_lifecycle.mjs` races bootstrap and reset requests to prove the demo lifecycle lock
prevents transactional deadlocks during browser/server reconnects.
`tools/e2e_replay.mjs` verifies ordered reconnect replay and proves that an up-to-date cursor
receives no duplicate events.
`tools/e2e_leases.mjs` races two controllers, verifies token rotation and stale-command rejection,
exercises explicit release and transfer, checks role-gated emergency stop, and confirms the runner
actually cancels the child process.
`tools/e2e_rooms.mjs` verifies persisted top-level messages and replies, author attribution,
structured mentions and entity links, two-member visibility, non-member write rejection, and
room-filtered WebSocket replay.
`tools/e2e_runner_reconnect.mjs` verifies persisted heartbeat timestamps, visible grace state,
short-disconnect process survival, assignment-token reconciliation, deterministic grace expiry,
lost-run state, and rejection of a later stale claim.
`tools/e2e_idempotency.mjs` makes the runner deliver the exact same `run.started` event twice and
proves that Postgres persists one event and applies one state transition.
`tools/e2e_codex.mjs` runs the complete server-to-runner Codex lifecycle against a deterministic
app-server fixture. It proves start, structured stream, live steer, resume into the same workspace,
interrupt, emergency stop, usage persistence, run ancestry, and artifact hash verification.
`tools/e2e_worktrees.mjs` launches fake-process and Codex tasks concurrently, proves their branches
and linked worktrees are distinct, verifies the configured checkout's HEAD and working state do not
change, confirms dirty work is preserved, and confirms a clean evidence-only run removes both its
worktree and branch.
`tools/e2e_task_graph.mjs` validates a three-node graph with two parallel specialist roots and a
dependency-gated synthesis task. The synthesis artifact must contain both verified specialist
outputs, not merely their task names. The same test proves an always-failing task stops exactly at
its retry limit.
`tools/e2e_verification.mjs` proves all six automated verifier types, a missing-file failure that
blocks completion, an owner approval gate, and an independent-review gate that rejects the
requester before accepting Bob's member-role decision.

`tools/e2e_mission_contracts.mjs` submits a long-form specification, repository/issue/approved
context references, a user-authored task contract, and all six verifier types through the public
mission API. It proves the generated task preserves that authority, failed tests block completion
despite provider evidence, and an independent reviewer must decide the manual gate. It also proves
versioned and exactly idempotent `redispatch` and `resume` revisions, explicit post-revision
dispatch/resume, current-room and role enforcement, stale-version rejection, authority-widening
denial, and preserved provider-session/worktree reuse. Desktop and 390-pixel Chromium validation
exercise the structured contract editor, exact completion-plan preview, revision form/history, and
responsive layout.

`tools/e2e_mission_repository_routing.mjs` connects two runners advertising different source
repositories and commits, creates an ordinary mission with one selected tuple, and proves only the
matching runner receives the run. The task and run retain the exact repository, ref, and commit;
the unselected runner creates no worktree; a stale commit is rejected before mission persistence;
and the ECorp checkout remains unchanged. Browser validation selects and confirms a separate local
dogfood repository, filters its available runtimes, launches through the UI, and verifies the
resulting worktree remains outside the ECorp repository.

`tools/e2e_identity.mjs` proves production OIDC enforcement, actor-spoof and cross-Corp rejection,
authorization before WebSocket replay, one-time runner enrollment, credential rotation, replay
rejection, active-run revocation, superseded-socket rejection, and rejection of a current socket
using the wrong assignment token. See `docs/evidence/2026-08-30-identity-validation.md`.

`tools/e2e_secrets.mjs` proves encrypted storage, scoped dispatch, environment delivery, denial for
an unauthorized requester, revocation, and absence of plaintext canaries from shared state and
logs. See `docs/evidence/2026-08-30-secret-broker-validation.md`.

`tools/e2e_approvals.mjs` restarts the server during a suspended risky action, approves from a
second actor, proves duplicate decisions do not duplicate effects, requires runner
acknowledgment, and verifies that an expired approval cancels coherently. `tools/e2e_budgets.mjs`
proves spend, repeated-tool, rolling requester/Corp budget evaluation, suspend and stop
transitions, non-retryable rejection of a misbehaving provider's late completion after a stop-stage
breaker, rejection of a pending action approval after the hard limit, and the healthy-conversation
exemption. `tools/e2e_budget_revision.mjs` proves owner/admin proposal and decision, unauthorized
member rejection, exact proposal/decision replay, rejected recovery, one-pending-revision
enforcement, bounded finish-scope replacement, pre-dispatch exhausted-budget rejection,
same-session/worktree resume under the remaining authorized ceiling, and a second overrun that
reaches terminal `stop` without artifact or accepted completion. Its review-hardening cases also
  prove actor and Corp rolling ceilings reject resume before run creation, a stopped descendant
  fences resume through an older suspended ancestor, room removal blocks proposal/decision replay, a
  changed task contract blocks stale approval, unsafe finish-scope paths fail closed, and a
  retryable pre-dispatch failure cannot displace the preserved source run from its resumable
  provider/workspace lineage. Chromium also proves cancelled+suspended missions retain recovery
  controls and that the same control remains available after a `dispatch_not_started` failure.
Cross-run fan-out for already active runs remains open in #56. See
`docs/evidence/2026-08-30-approval-and-budget-validation.md` and
`docs/evidence/2026-09-02-authorized-budget-recovery.md`.

`tests/scenarios/v1.jsonl` is a versioned 100-scenario corpus. `tools/run_evals.mjs` keeps
deterministic and real-provider lanes separate. Deterministic rows must reference fresh, hashed
integration evidence whose category contract passes. Reused category evidence is disclosed, and
latency, cost, rework, and intervention remain `null` unless they were actually measured; the eval
runner no longer derives outcomes or invented metrics from `scenario.expected`.

`tools/e2e_chaos_report.mjs` consolidates server-restart, runner-reconnect, duplicate-delivery, and
browser-replay evidence. The `runner-platforms` CI matrix runs the runner contract on Windows,
macOS, and Linux. See `docs/evidence/2026-08-30-alpha-eval-chaos-platform-validation.md`.

On August 31, 2026, a fresh isolated Windows matrix passed all 22 integration scripts. Separate
browser runs exercised plan inspection, real GitHub Copilot and Codex missions, live control,
Copilot action approvals, independent review across Alice and Bob, failed-verification recovery,
production OIDC login with a one-time WebSocket ticket, and a 390-pixel responsive layout with no
horizontal overflow.

On September 1, 2026, one operator ran three enterprise-application scenarios through the live
browser-to-server-to-runner path. VendorGuard passed 13 tests and its browser workflow; Incident
Command passed 12 tests and its browser workflow; Credit Exception remained incomplete with 84
failures and two errors. The pass exposed hard-breaker, budget-recovery, provider-isolation,
permission-bridging, mission-contract, and portable-deliverable gaps at that time. Subsequent
evidence below covers the landed breaker, budget-recovery, mission-contract, and portable-deliverable
work. Claude permission mediation and fail-closed process-tree teardown have also landed.
Provider-home isolation and the remaining real-world recovery drills remain tracked in #51.
This is internal systems dogfood and does not satisfy the three-external-team requirement in GitHub
issue #27. See
`docs/evidence/2026-09-01-enterprise-application-dogfood.md`.

The runner unit suite applies one provider-independent lifecycle conformance harness to the
`fake-process` adapter. It verifies spawn, stream, steer, artifact, stop, capability reporting, and
typed errors for unsupported resume and usage operations.

The Codex adapter suite uses a protocol-faithful fake app-server to verify availability reporting,
start, streaming, usage de-duplication, live `turn/steer`, graceful `turn/interrupt`, stop, durable
resume, completed evidence, cancelled evidence, and failed evidence without requiring credentials.
`tools/e2e_codex.mjs` also emits multiple usage updates during an active turn and proves the
resulting stop-stage breaker interrupts the Codex path before accepted artifact or completion.
An authenticated Codex `0.151.0-alpha.7.2` run on September 2, 2026 independently exceeded a
one-token ceiling, persisted usage before `stop` and runner acknowledgment, rejected its late
artifact, produced no completion or retry, removed the clean worktree, and rendered a coherent
failed state in desktop and mobile Chromium. See
`docs/evidence/2026-09-02-real-provider-budget-stop.md`.

The external-adapter conformance test and `tools/e2e_external_adapters.mjs` run one common sample
through Claude Code and OpenCode normalization, verifying equivalent session, usage, artifact, and
completion evidence. See `docs/evidence/2026-08-30-external-adapter-validation.md`.

The Claude external-adapter permission suite drives a protocol-faithful fake CLI over bidirectional
stream JSON. It verifies manual permission mode, the initialize response before the typed initial
user frame, hardened launch arguments, exact
`can_use_tool` correlation, bounded approval context, unchanged approved input, bounded rejection
and expiry denials, duplicate-decision rejection, and fail-closed cleanup. Contained worktree
read/write requests are the only local auto-allow case; blocked, ambiguous, outside-worktree, Bash,
and network requests suspend for a durable ECorp decision. It also proves raw tool input is absent
from durable approval text and control-response write failure fails the run. An installed Claude
Code `2.1.223` probe emitted `can_use_tool` for a Bash marker write after initialize under manual
mode; the correlated denial left the marker absent. See
`docs/evidence/2026-09-03-claude-permission-bridge.md`.

`tools/e2e_copilot.mjs` uses a deterministic Copilot fixture to verify model discovery, disabled
policy states, model and reasoning validation, persisted selection, evidence, and same-session
resume without requiring an account. `tools/probe_copilot_live.mjs` is the separate authenticated
lane; it must use the official SDK, expose the live account catalog, run a selected real model, and
finish with verified worktree evidence. See
`docs/evidence/2026-08-30-github-copilot-adapter-validation.md`.

That authenticated lane must also prove that Copilot's native managed permissions resolve ordinary
worktree-local built-in read, create, and edit operations without duplicating them as ECorp
approvals. Required build and test commands run through the persisted runner verifier unless an
operating-system shell boundary is independently proven. The lane must pre-create external and
credential canaries, assert their bytes and metadata remain unchanged, and inspect provider
telemetry rather than trusting configured sandbox settings. Any shell execution reporting
`sandboxApplied: false` fails the containment case. Destructive commands, interpreters,
network-capable commands, publication or infrastructure commands, sandbox bypass, credential
access, and external paths must fail closed or suspend through a durable ECorp decision.

The live probe now enrolls its own runner on a fresh isolated server and injects a synthetic
`GITHUB_TOKEN` into the actual runner launch. A process-boundary observer asserts that the runner
inherited it and that each SDK-launched Copilot process did not, before starting the real Copilot
binary. Missing seeds and unremoved canaries are test failures, not successful absence checks.
Each negative mission must reach an approval containing its intended boundary action; a model
refusal or an unrelated approval cannot count as exercised containment.

The same probe asks real Copilot to author Piper Kingdom under `scenarios/piper-kingdom/**`.
The runner executes the persisted syntax check, generated unit tests, and independently authored
gameplay assertions, then exports a source archive. The probe requires zero routine approvals,
the exact retained write scope, and an unchanged source checkout. It does not supply game code.
`tools/copilot_probe_process.test.mjs` independently verifies the observer's positive and negative
preconditions without an account. Native filesystem unit tests exercise dangling/ancestor links,
hard links, root protection, Windows aliases, scope-limited mutations, and Unix executable modes.

Run on a separate development server with no existing connected runner:

```powershell
$env:CRONY_SERVER_HTTP = 'http://127.0.0.1:<isolated-server-port>'
$env:ECORP_TEST_SOURCE_REPOSITORY = 'C:\path\to\clean-disposable-repository'
$env:CRONY_PROBE_COPILOT_BINARY = 'C:\path\to\real\copilot.exe'
$env:CRONY_PROBE_OUTPUT = 'C:\path\outside\ecorp\unique-probe-output'
$env:CRONY_PROBE_MODEL = '<enabled model id from the connected account>'
node tools/probe_copilot_live.mjs
```

Set `CRONY_PROBE_BOUNDARY_ONLY=1` to run the negative conformance lane separately. Its report is
explicitly labeled `boundaries_only` and cannot substitute for an application-build result. This is
useful when an application model refuses a test prompt before reaching the permission handler; that
refusal remains inconclusive for handler coverage rather than being counted as a passed probe.
The probe now settles each case and continues to later independent boundaries after such an
inconclusive terminal result. `copilot-boundary-progress.json` atomically records callback-only
progress; it is explicitly partial and does not replace the final sentinel/process/credential
checks. Final `e2e-copilot-live.json` includes all seven required cases and an explicit
`boundary_matrix.complete` flag. Any missing, mismatched, un-rejected, or inconclusive permission
case still makes the invocation exit nonzero. Unexpected transport, containment, and teardown
failures still stop the probe rather than proceeding with an unsafe active run.
`node --test tools/copilot_probe_boundaries.test.mjs tools/copilot_probe_process.test.mjs`
checks that partial/duplicate/cross-run evidence cannot silently become complete coverage.
Set `CRONY_PROBE_PRESERVE_DEMO=1` to retain earlier test history instead of resetting the specified
development server.

`CRONY_PROBE_NATIVE_READ_ONLY=1` selects the separate #153 native-read regression. It is mutually
exclusive with boundary-only and application-resume modes. Use a clean disposable Git source with
a small committed `native-read-seed.txt` containing one non-secret marker line. The diagnostic
requires actual successful `view` results for relative and absolute source paths and a newly
created readback file; provider self-report or final file existence cannot substitute. A
persisted verifier checks the exact readback hash, source bytes/mtime remain unchanged, no
routine approvals or shell executions are allowed, and the process observer must see the
product's no-auto-update flag. The first failed relevant view or unexpected approval stops only
that diagnostic run rather than burning its remaining budget. A Windows-native CRLF fixture
tests byte-exact round-trip behavior; the earlier LF-to-CRLF copy failure is retained, not
normalized into a pass.

`ECORP_COPILOT_FS_WIRE=1` optionally enables the test-only, transparent metadata observer for
Content-Length-framed `sessionFs.stat` / `sessionFs.exists` exchanges. It forwards original bytes
unchanged, bounds frame/pending-request memory, and records only response shape, boolean fields,
identifier type and date validity. Paths, file contents, credentials and unrelated RPC payloads
are not logged. The native-read and wire-observer Node tests exercise these negative cases and
exact transport forwarding. See
`docs/evidence/2026-09-06-copilot-native-read-runtime.md` for the before/after evidence and runtime
version controls.

For a preserved verification-failed run, `CRONY_PROBE_RECOVER_RUN_ID` plus its original
`CRONY_COPILOT_EVENT_ROOT` exercises the existing resume API. It retains the source, task, model,
provider session, worktree, and verifier policy; supplies bounded verifier feedback; and permits at
most two automatic repair requests per invocation within the original remaining budget. A
hard-stopped lineage is not resumable. Its evidence must be retained, not represented as recovered.

The probe supervises only its own enrolled runner and keeps logs, worktrees, and the game outside
the ECorp checkout. It does not publish, merge, deploy, or use hosted GitHub Actions.
See `docs/evidence/2026-09-06-copilot-native-filesystem.md` for the accepted application, browser
checks, retained failed lineage, and the explicitly incomplete live negative coverage.

`tools/e2e_gateways.mjs` launches the MCP, ACP, and A2A binaries against a live server and verifies
version negotiation, scoped tools, session-to-mission mapping, agent discovery, task/message
methods, and SSE streaming. See `docs/evidence/2026-08-30-protocol-gateway-validation.md`.

`tools/e2e_factory_claims.mjs` races duplicate factory claims, renewals, and mission
materialization, restarts the server between claim and renewal, rejects guest, cross-Corp, stale
version, stale token, and duplicate-active operations, proves the fencing token is absent from
snapshots and events, proves non-operators cannot read source metadata through factory snapshots or
events, rejects source or policy replacement during an expired pre-materialization reclaim, and
proves mixed-case GitHub identities collapse onto the same work item before launching the one
linked mission through the real runner. Rejected materializations use independent claims and prove
each invalid policy, tool, or secret expansion becomes a recoverable `blocked` item with no mission
and an immediately released lease rather than contaminating the valid concurrency case.

`tools/e2e_factory_controller.mjs` gives the CLI a deterministic GitHub API boundary and proves
Project eligibility, dependency parsing, a mutation-free dry run, durable claim-before-dispatch,
Todo-to-In-Progress synchronization only after mission linkage, one real child-process run, and
duplicate controller recovery without a second work item, mission, or run. It also injects a
GitHub Project status failure, verifies durable `blocked` state before launch, and proves retry
reuses the existing mission. The failure fixture emits multi-line stderr to prove error text cannot
prevent the durable transition, and a stalled Project mutation is killed before the lease window
can expire. Additional regressions prove verified items leave the intake queue
and terminal mission failure becomes terminal factory failure while verifier rejection remains
`verification_failed`. It also proves lease renewal around external effects, source revision and
label revalidation before the Project mutation,
reopened-dependency revalidation before launch, persistence of the claimed repository/base
requirement, and rejection of a runner checked out to another repository before run creation.
`crony-store` unit coverage separately proves exact retention of policy-pinned model and reasoning
settings on every materialized task and rejects provider-backed tasks without a manual gate. The
controller E2E proves an independent reviewer, not the requester, advances
`awaiting_approval -> verified`. The same E2E creates 501 newer historical factory items, proves
the recoverable target is absent from the legacy 500-item snapshot, then recovers the exact work
item, mission, persisted policy, and single run through the selected-Project-item lookup. It also
proves explicit lookup counts, identifier and request-count bounds, guest denial, isolation from an
equal item ID with a conflicting policy in another Project, and recovery after the lease duration
changes. A fresh post-replay lookup and snapshot count every matching work item, mission, task, and
run to prove there is exactly one of each. Mission discovery uses the deterministic GitHub issue
title and persisted source marker from the fresh snapshot rather than following the work item's
mission link, so an unlinked duplicate mission would also fail the regression. CLI unit coverage
verifies Markdown section boundaries for dependency and acceptance parsing.

`tools/e2e_factory_controller_service.mjs` verifies durable controller registration and exact
idempotent replay, owner-only configuration, manager-capable control, member denial, pause/resume,
monotonic reconciliation generations, bounded failure text, blocked and watching projections,
lease-expiry offline state, stale connection-epoch rejection, and reconnect recovery.
The live watcher probe starts `crony factory-watch` against a deterministic GitHub boundary,
observes `watching`, applies durable pause and resume, verifies resume requests a reconciliation
generation, terminates the watcher, and observes `offline` after the heartbeat lease expires.

`tools/e2e_factory_cockpit_reconnect.mjs` drives one factory work item through two independently
attributed browser-protocol clients. It posts contextual comments, takes and rotates the control
lease, delivers steering through durable runner commands, restarts the server, resumes each
browser from its prior sequence cursor, and records Bob's decision after reconnect. Exact replay
of comment, steer, claim, materialization, and decision operation keys creates no duplicate
message, provider effect, work item, mission, or run. The same drill proves two durable steer
acknowledgments, stale-token rejection, cancellation of a pending steer after its lease version
rotates, controller recovery to `watching`, and final `verified` state. A separate rendered
two-browser pass exposed and verified the **Reclaim control** recovery for a browser that lost its
private fencing token. See
`docs/evidence/2026-09-04-factory-cockpit-restart.md`.

`tools/e2e_factory_cockpit_publication.mjs` continues that operating lane through a verified
commit/branch deliverable and the trusted publisher. It crashes after the deterministic GitHub
boundary has created the pull request but before ECorp records the remote checkpoint, restarts the
server, and concurrently retries publication. The focused drill proves one work item, mission,
run, branch, pull request, and publication; Project status changes to `In Review` only after pull
request creation; Alice and Bob resume from independent event cursors; and auto-merge, merge, and
deployment remain disabled. See
`docs/evidence/2026-09-04-factory-cockpit-restart.md`.

`tools/e2e_factory_verification_recovery.mjs` proves the same source issue, factory item, mission,
task, branch, and workspace lineage can recover after verification rejection. Its verifier-only
case rejects evidence through an independent-review gate, replays the keyed decision, creates one
`verification_only` run, emits zero provider session/output/artifact events, preserves the exact
head commit, and reaches one verified factory item. A direct-command verifier writes a side-effect
file during the original run, then attempts to increment it and corrupt `result.md` during
verifier-only recovery. The later file check still reads a fresh snapshot, while the preserved
source retains its original side-effect value and valid `result.md`. The runner explicitly removes
each snapshot before accepted verification and then fingerprints the preserved source. The
`verifier_snapshot_is_physical_isolated_and_excludes_git_control` unit regression separately checks
snapshot isolation, `.git` exclusion, bounded explicit cleanup, and copying of untracked and ignored
source files. Unix coverage additionally rejects a relative symlink that escapes the worktree;
Windows runtime code rejects escaping symbolic links and reparse points.
Before recovery, the E2E removes only the source run's fingerprint to reproduce a legacy
preserved run. Signed artifact metadata is never edited to manufacture that fixture: changing
it must fail provenance verification. The owning runner checkpoints the legacy workspace without
provider execution. The separate Codex bridge case verifies an artifact held in signed object
storage, with no local source copy and no invented file-check evidence.
Its source-correction case starts with an automated verifier failure, changes the fake GitHub issue
revision, proves ordinary controller
replay is mutation-free and rejected, restarts only the test-owned server, stores a versioned
contract revision, then injects a revoked scoped secret before dispatch. It proves that failure
terminalizes the run and recovery, clears the active-recovery uniqueness fence, restores
`verification_failed`, and permits a newly authorized retry after the secret policy is repaired.
The retry resumes the exact Codex fixture session/worktree, increments the attempt monotonically,
and completes after independent approval. A verifier-only bridge keeps its own provider-session
field empty; a later source correction resolves only the existing ancestor session within the
same Corp, task, agent, and workspace. Repeated verifier-only generations retain the exact
authorized artifact rather than requiring its producer to be the immediate parent.

Run this destructive-fixture harness only with `CRONY_RECOVERY_TEST=1`, an explicitly owned
`CRONY_SERVER_HTTP` and `DATABASE_URL`, an independent `CRONY_RECOVERY_SOURCE` repository, and
`CRONY_RECOVERY_OUTPUT`. Shared/manual ports and the ECorp checkout are rejected. Cases preserve
earlier results and do not reset the demo between exercises. The report is checkpointed after
each case in the selected output directory. `--resume-after-bridge` continues only the exact
documented two-run, version-one source-correction checkpoint; it does not create a replacement
mission or reset the database.

The Windows restart drill additionally requires `CRONY_TEST_SERVER_BINARY` and a JSON
`CRONY_TEST_SERVER_PID_FILE` with `test_owned: true`, the exact `workspace`, `server_url`,
numeric `server` PID, and ISO `server_creation`. `tools/owned_test_stack.mjs` verifies the
executable, creation time, and listener ownership before stopping it, and records the replacement
identity. Database credentials are passed through the test supervisor's environment, never
process arguments. The same helper protects controller/publication reconnect drills.

The same harness interrupts a verifier-only recovery while a snapshot command is active. It proves
the cancelled run is preserved, the recovery becomes failed, the task and factory item return to
`verification_failed`, the active slot is released, and a second recovery can be authorized. It
also inserts 101 newer historical recoveries so the live recovery disappears from the bounded Corp
snapshot, then proves the exact work-item recovery context still returns and replays the original
recovery and run.

Integrity regressions hold a deliverable-upload acknowledgment, modify the preserved source,
and require rejection before either a manual-review wait or accepted completion. The retained
workspace becomes durably `quarantined`, not an unsealed legacy checkpoint. Cleanup replay cannot
downgrade that state, and checkpoint, resume, recovery, and publication admission reject it.
Mode-sensitive snapshot/fingerprint tests must also run on a real Unix filesystem; Windows
test success does not execute the Unix-only cases.

The September 4, 2026 preflight regression runs both dry-run and execution paths against invalid
3,000,000-token budgets, verifier timeouts, model and reasoning-policy mismatches, unsafe write
scope, unsupported description control characters, and an oversized materialization snapshot. Each
case proves the Project item remains `Todo` with zero claim, mission, task, run, or Project
mutation. A positive case proves execution reuses the same accepted preflight shape. The suite also
kills a real controller after its durable claim but before materialization, verifies the opaque
token is absent from shared state and controller output, then recovers the same work item with exactly one
mission and run. Direct post-claim rejection tests prove the server releases a blocked
pre-materialization lease, rejects source or policy replacement, and safely records a second
rejection after reclaim without idempotency collision. A separate live Postgres probe removes an
operator's room membership and proves preflight returns `403` before any durable object exists. See
`docs/evidence/2026-09-04-factory-preflight-and-recovery.md`.

See
`docs/evidence/2026-09-01-governed-dark-factory-foundation.md` and
`docs/evidence/2026-09-01-factory-pagination-safety.md`.

`tools/e2e_factory_commit_routing.mjs` connects two runners that advertise the same GitHub
repository and symbolic `HEAD` ref but resolve to different commits. It proves factory policy,
task contracts, and the persisted run retain the authorized full commit, the scheduler selects only
the matching runner, and the wrong runner receives no run or worktree files. Runner and server unit
coverage separately proves malformed object IDs, partial source tuples, mismatched starts, and
mismatched resumes fail closed. See
`docs/evidence/2026-09-01-factory-immutable-source-routing.md`.

`tools/e2e_factory_legacy_source_upgrade.mjs` rewrites one completed factory run into its pre-0022
shape, reapplies migration 0022, and proves policy, task, and run source commits are recovered from
the persisted workspace base commit. It also creates an unmaterialized legacy claim with no
derivable commit, proves migration marks an upgrade requirement, then runs the controller through
one fenced and audited source-pin operation before materialization and verified execution. The
same test proves new commit-less claims are rejected without creating a work item and that a
recovery invocation naming a different symbolic ref cannot mutate the migration-marked record. The
workspace unit regression advances `HEAD` while one manager remains alive and proves a later
unpinned worktree follows the new commit while pinned and resumed work remains on the original
identity.

An authenticated Windows probe on August 29, 2026 validated the same path against Codex CLI
`0.150.0-alpha.8`: one run accepted live steering and completed, a second run was interrupted and
resumed in the same provider thread and repository, and a third run was emergency-stopped before
its post-sleep side effect. See `docs/evidence/2026-08-29-codex-adapter-validation.md`.

Worktree unit tests run in source and managed paths containing spaces. They cover distinct parallel
worktrees, exact resume reuse, dirty and committed preservation, post-integration reclamation,
detached-state fail-safe behavior, path/ref validation, and occupied-target rejection. See
`docs/evidence/2026-08-29-worktree-isolation-validation.md`.

Planning unit tests prove strategy replacement, deterministic adapter matching, cycle rejection,
depth bounds, retry bounds, per-task budgets, and total mission budgets. See
`docs/evidence/2026-08-29-task-graph-validation.md`.

Runner verifier tests cover valid and missing files, artifact hashes, commands, tests, JSON
required-key schemas, screenshot signatures, path traversal, exact executable lookup on Linux and
macOS, and fail-closed explicit-path validation. Windows coverage deterministically exercises
`npm`, explicit `npm.cmd`, `pnpm`, `npx`, another `PATHEXT` executable, injection-shaped arguments,
missing tools, and the installed npm shim. `tools/e2e_windows_verifier_resolution.mjs` executes the
candidate verifier with the exact `npm --prefix scenarios/incident-command test` policy. See
`docs/evidence/2026-08-29-evidence-verification-validation.md` and
`docs/evidence/2026-09-03-windows-verifier-command-resolution.md`.

`tools/e2e_artifacts.mjs` verifies server-mediated upload, content-addressed storage, normalized
media type, signed producer/run/task/verifier/retention provenance, path-free shared state,
authorized download, and non-member denial. The same flow was exercised against MinIO as the
S3-compatible backend. See `docs/evidence/2026-08-30-artifact-storage-validation.md`.

`tools/e2e_artifact_staging.mjs` holds the run row, applies a hard breaker, and proves authoritative
rejection occurs before staging while an accepted artifact with the same digest remains
downloadable. It injects a Postgres reservation failure and proves no object bytes are written, then
injects a metadata-finalization failure after both staged and final bytes exist and proves restart
recovery completes the commit. The same recovery pass finalizes staged metadata, rejects an old row
whose bytes fail validation, releases an old reservation whose staged and final objects are both
missing, retries cleanup for a ready row, removes an unreserved staging object only after a fresh
reservation check, and restores accepted run-to-artifact links. See
`docs/evidence/2026-09-01-artifact-staging-recovery.md`.

`tools/e2e_portable_deliverables.mjs` verifies that a real runner exports tracked modifications and
untracked source while excluding provider evidence, uploads the exact bounded bytes through staged
content-addressed storage, links them to the exact verification digest, supports authorized remote
download, denies a non-member, optionally creates a commit only on the isolated task branch, and
receives durable storage acknowledgment before a clean worktree is reclaimed. Browser validation
checks that provider evidence, verification evidence, source deliverables, and integration state
are visibly distinct. See `docs/evidence/2026-09-02-portable-deliverables.md`.

`tools/e2e_factory_publication.mjs` uses a real server, runner, isolated worktrees, portable Git
bundle, bare Git remote, and deterministic fake GitHub API. It proves policy, role, Corp, budget,
and breaker rejection; manage-only publisher enrollment; independent publisher workload
authentication on start, renewal, failure, and every checkpoint; exact publisher-ID binding;
revoked, expired, invalid, missing, and cross-Corp credential rejection; workload-auth recovery
after server restart; branch-push recovery after a publisher crash and server restart; pull-request
adoption after external success plus local failure; Project-status recovery after another crash;
duplicate concurrent publication convergence; credential non-disclosure; exactly one branch and
pull request; same-named fork rejection; exact PR-head SHA binding; symbolic `HEAD` verification;
separate persistence of policy base `HEAD` and resolved GitHub PR base;
post-start role, breaker, run-budget, and Corp-budget revocation; and the required
pull-request-before-review ordering. It also proves the resolved base branch cannot be pushed as the
publication branch, implicit authorization IDs survive duplicate/restart invocation, and CRLF body
files canonicalize before idempotency comparison. The base-collision case leaves no durable
publication and accepts a corrected branch afterward, while a second manager's recovery attempt
receives a distinct actor-bound authorization identity. A completed-publication retry also returns
the persisted result after the remote base advances, without another PR or Project mutation.
The same harness injects an exact same-repository/SHA PR with an unauthorized title/body and proves
it is rejected before Project movement. Unit coverage pins retries to the persisted deliverable when
multiple mission deliverables are otherwise eligible. It also creates 501 newer work items,
deliverables, and publications, proves the requested objects fall outside each 500-row snapshot,
then publishes and replays the completed result through the exact publication context. Recovery
changes publisher ID, authorization reason, and lease duration; invalid `ecorp/foo//bar` and
`ecorp/foo.lock` branches fail before durable start. The Project regression places the authorized
item after 1,001 fillers and proves publication uses exact GraphQL node lookup without another
bounded item-list read. A separate same-Corp manager belongs only to another room: start and
recovery return `403`, exact publication status returns `404` without sensitive fields, and removing
the active publisher from the mission room makes each pre-branch, pre-PR, and pre-Project renewal
return `403` before its external effect. A whitespace-padded custom pull-request title is normalized
before start and idempotency, reaches post-plan validation, and recovers with the exact persisted
title. The cross-room context read also returns `404` without work-item source/policy/failure data,
and the controller rejects `refs/tags/v1` as a publication base before Project reads or durable
claim. A direct control-character base policy returns `400`; a mixed-case target repository
canonicalizes before plan validation; and a Project with 32 fields, with Status after the default
30-field page, completes through exact Status-field GraphQL lookup. A canonical PR URL retaining
mixed-case owner/repository components is accepted and persisted. The controller's omitted
publication base follows the selected source ref (`HEAD` or `release`); a scheduled PR close after
Project-stage renewal is detected after Project refresh and before any Project mutation. Runner unit
coverage detaches worktree HEAD and proves the validated branch still produces an importable bundle
with no temporary ref left behind. See
`docs/evidence/2026-09-02-idempotent-pull-request-publication.md`.

Store unit regressions cover recovery-aware publication provenance. A completed recovery selects
its reviewed source revision and recovery ID; recovery selection follows the chosen deliverable
run's resume lineage; a non-recovery publication retains the original claimed revision with a null
recovery ID; and legacy schema-version-1 provenance remains resumable while version 2 requires the
claimed/effective/recovery tuple. Earlier deterministic publication coverage passed, but the
early September 7, 2026 follow-up failed its concurrent-publication case with a database
deadlock. The expanded recovery drill also stalled because synthesis could not consume its
recovered parent's artifact. Issues #165/#166 repair those defects, and both complete local
suites now pass; see [the runtime-fix report](evidence/2026-09-07-recovery-publication-runtime-fixes.md).
The earlier failures remain preserved as evidence. These are local fixture results, not a
new real-GitHub recovery-to-publication effect claim. The separate pre-dispatch state-coherence
gap in #167 remains open.

Two opt-in SQLx regressions additionally exercise the actual `dependency_artifacts` store
method with recovered-parent metadata, multiple verifier-only generations, stale runs,
cycles and cross-boundary/mismatched evidence. Run them with `DATABASE_URL` scoped to an
explicitly owned QA PostgreSQL maintenance database:
`cargo test -p crony-store dependency_artifacts_ -- --ignored --test-threads=1`.
SQLx creates its isolated test databases; ordinary workspace test runs list these as
ignored unless explicitly selected. These metadata fixtures do not replace real signed
artifact verification in the full recovery drill.

`tools/e2e_publication_credential_lock.mjs` uses the exact credential-lock SQL selected by
`publication.rs` and native PostgreSQL sessions to reproduce the shared-lock upgrade
deadlock. It waits for an observed lock, not a guessed sleep, then verifies both transactions
complete with the update lock taken upfront. It requires explicit opt-in and an already
running owned loopback QA container, creates only a uniquely named synthetic schema,
terminates only its own tagged sessions, removes that schema, and checks that the real
publisher-credential fingerprint is unchanged. It starts no server, provider or container.
Complete publication acceptance still requires the server/runner/CLI/Git E2E.

The controller recovery preview also proves a source `release` item with persisted publication base
`main` continues to report `main` when a later dry run omits the override. At the Project boundary,
the publication harness pauses after the slow Project/PR reads but before the final authority
renewal, removes the human publisher from the mission room, receives `403`, and proves no Project
edit or status movement occurred. This demonstrates the second renewal fences the irreversible
effect rather than merely protecting the earlier reads.

On September 3, 2026, Incident Command review correction #96 exposed a recovered-suspend
publication regression. The original provider run crossed its token ceiling and reached
`suspend`; an approved budget revision then resumed the same provider session and worktree through
two descendants. The final run passed 16/16 checks and independent review, but publication rejected
the historical suspend ancestor. The corrected server accepts only suspend ancestors in the
selected deliverable run's explicit resume chain whose current loop counters remain below policy,
while preserving terminal rejection for `stop`, unrelated suspend, stop-level loop metrics, and
malformed lineage. The same live database then published PR #99 at exact
runner commit `ada87bd85dafdc62f354c4641c7e9340be5b1ece`; an identical publication replay returned the
same PR with one attempt and no additional remote effect. See
`docs/evidence/2026-09-03-recovered-suspend-publication.md`.
