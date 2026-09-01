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
persistent agent identity returns to `idle` with no `current_run_id`. It does not yet verify the
entire operating-system descendant tree; that stronger cleanup boundary is tracked in #51.

## Quality gates

```powershell
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
exemption. Authorized recovery after suspension remains open in #50, and cross-run fan-out for
aggregate limits remains open in #56. See
`docs/evidence/2026-08-30-approval-and-budget-validation.md`.

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
permission-bridging, mission-contract, and portable-deliverable gaps. It is internal systems
dogfood and does not satisfy the three-external-team requirement in GitHub issue #27. See
`docs/evidence/2026-09-01-enterprise-application-dogfood.md`.

The runner unit suite applies one provider-independent lifecycle conformance harness to the
`fake-process` adapter. It verifies spawn, stream, steer, artifact, stop, capability reporting, and
typed errors for unsupported resume and usage operations.

The Codex adapter suite uses a protocol-faithful fake app-server to verify availability reporting,
start, streaming, usage de-duplication, live `turn/steer`, graceful `turn/interrupt`, stop, durable
resume, completed evidence, cancelled evidence, and failed evidence without requiring credentials.
`tools/e2e_codex.mjs` also emits multiple usage updates during an active turn and proves the
resulting stop-stage breaker interrupts the Codex path before accepted artifact or completion.

The external-adapter conformance test and `tools/e2e_external_adapters.mjs` run one common sample
through Claude Code and OpenCode normalization, verifying equivalent session, usage, artifact, and
completion evidence. See `docs/evidence/2026-08-30-external-adapter-validation.md`.

`tools/e2e_copilot.mjs` uses a deterministic Copilot fixture to verify model discovery, disabled
policy states, model and reasoning validation, persisted selection, evidence, and same-session
resume without requiring an account. `tools/probe_copilot_live.mjs` is the separate authenticated
lane; it must use the official SDK, expose the live account catalog, run a selected real model, and
finish with verified worktree evidence. See
`docs/evidence/2026-08-30-github-copilot-adapter-validation.md`.

`tools/e2e_gateways.mjs` launches the MCP, ACP, and A2A binaries against a live server and verifies
version negotiation, scoped tools, session-to-mission mapping, agent discovery, task/message
methods, and SSE streaming. See `docs/evidence/2026-08-30-protocol-gateway-validation.md`.

`tools/e2e_factory_claims.mjs` races duplicate factory claims, renewals, and mission
materialization, restarts the server between claim and renewal, rejects guest, cross-Corp, stale
version, stale token, and duplicate-active operations, proves the fencing token is absent from
snapshots and events, proves non-operators cannot read source metadata through factory snapshots or
events, and launches the one linked mission through the real runner.

`tools/e2e_factory_controller.mjs` gives the CLI a deterministic GitHub API boundary and proves
Project eligibility, dependency parsing, a mutation-free dry run, durable claim-before-dispatch,
Todo-to-In-Progress synchronization only after mission linkage, one real child-process run, and
duplicate controller recovery without a second work item, mission, or run. It also injects a
GitHub Project status failure, verifies durable `blocked` state before launch, and proves retry
reuses the existing mission. Additional regressions prove verified items leave the intake queue
and terminal mission failure becomes terminal factory failure. It also proves lease renewal around
external effects, source revision and label revalidation before the Project mutation,
reopened-dependency revalidation before launch, persistence of the claimed repository/base
requirement, and rejection of a runner checked out to another repository before run creation.
`crony-store` unit coverage separately proves exact retention of policy-pinned model and reasoning
settings on every materialized task. See
`docs/evidence/2026-09-01-governed-dark-factory-foundation.md`.

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
required-key schemas, screenshot signatures, and path traversal. See
`docs/evidence/2026-08-29-evidence-verification-validation.md`.

`tools/e2e_artifacts.mjs` verifies server-mediated upload, content-addressed storage, normalized
media type, signed producer/run/task/verifier/retention provenance, path-free shared state,
authorized download, and non-member denial. The same flow was exercised against MinIO as the
S3-compatible backend. See `docs/evidence/2026-08-30-artifact-storage-validation.md`.
