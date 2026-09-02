# Authorized mission budget recovery validation

**Date:** September 2, 2026
**Core implementation:** `c3431f9526e3498c5fa14c0a1fb67ae19f3ff136`
**Deterministic E2E checkpoint:** `892c3d24d3375575def97fd2c05f3b9521d55ae2`
**Operator UI:** `c4ce25e`
**Integrated publication base:** `bef6774851c79a1cd8a773883b9c5b090318b673`
**Cross-scenario reset isolation:** `3a3702e`
**Initial full repository gate head:** `70b615e416add32ca64da9107983bd4a8caef707`
**Authority and retry hardening:** `2231c8d723fc7dd9dbedcc1c16cb9162c0b6dbaa`
**Rolling-budget and lineage hardening:** `91349215f07356dc6319f26aaefb48930363c720`
**Integrated publication room-scope head:** `c80c36a4e26d32d7fb236f19edaefc1102f1d1c7`
**Final integrated full-gate head:** `3b0ad5377265620a0cede2046156862d7f379197`
**Publication main merge:** `d7de6c89155ed059da05cf54bedc381d743df232`
**Final stacked integration head:** `10f39fe6591de05508c9979615cdfd98cfd4404c`

## Scope

This evidence covers GitHub issue #50: durable owner/admin mission budget revision, explicit
original/current/consumed/remaining authority, optional bounded finish scope, pre-dispatch resume
rejection, same-session recovery, duplicate/restart-safe state, terminal overrun behavior, and the
desktop/mobile operator controls.

It does not authorize an override of `stop`, reset usage, widen a write boundary, replace a verifier
policy, merge a pull request, or deploy.

## Durable model and API

Migration `0028_mission_budget_revisions.sql` adds:

- immutable `missions.original_budget_tokens` and
  `missions.original_budget_cost_microusd`;
- versioned Corp/mission-scoped revision proposal, decision, and request snapshots;
- one-pending-revision enforcement;
- proposer, decider, rationale, note, usage-at-proposal, and before/after contract provenance; and
- optional bounded finish-scope linkage to the existing task.

The protected API exposes:

```text
POST /api/corps/{corp_id}/missions/{mission_id}/budget-revisions
POST /api/corps/{corp_id}/missions/{mission_id}/budget-revisions/{revision_id}/decision
POST /api/corps/{corp_id}/runs/{run_id}/resume
```

Both revision endpoints require `Permission::Manage`, which is limited to owners and admins.
Proposal and approval revalidate mission status, active runs, current ceiling, cumulative usage,
latest breaker state, mission-room membership, and optional finish-scope bounds under transaction
locks. Approval also compares the current task contract and verifier policy to the proposal
snapshot.

## Deterministic recovery E2E

`tools/e2e_budget_revision.mjs` ran against the current Windows server, PostgreSQL, runner, real
worktree manager, and protocol-faithful fake Codex app server.

The successful path recorded:

```json
{
  "original_budget_tokens": 6000,
  "revised_budget_tokens": 20000,
  "consumed_tokens": 6012,
  "resume_budget_tokens": 4000,
  "approving_actor_id": "00000000-0000-4000-8000-000000000011",
  "final_status": "completed"
}
```

The run first consumed 6,000 tokens and suspended with its provider session and dirty worktree
preserved. Resume before approval returned a conflict and created no run. A member could neither
propose nor decide. Rejected and duplicate proposal/decision paths remained exactly idempotent.
Alice then approved a 20,000-token ceiling with a 4,000-token bounded finish contract. Resume reused
the same provider session and worktree, received a 4,000-token run limit, passed verification, and
completed the original mission.

The negative path recorded:

```json
{
  "revised_budget_tokens": 10000,
  "resume_budget_tokens": 4000,
  "resume_usage_tokens": 6000,
  "breaker_stage": "stop",
  "accepted_artifact": null,
  "completed_events": 0
}
```

The revised run exceeded its authorized resume limit, reached monotonic `stop`, and could not
publish an artifact or accepted completion. Retrying its older suspended ancestor was rejected
because the shared provider-workspace lineage contained the stopped descendant.

Additional review-hardening cases recorded:

```json
{
  "actor_resume_rejected_before_run": true,
  "corp_resume_rejected_before_run": true,
  "stale_contract_approval_rejected": true,
  "proposal_replay_after_room_removal_rejected": true,
  "decision_after_room_removal_rejected": true,
  "stopped_lineage_ancestor_resume_rejected": true
}
```

Requester and Corp rolling ceilings are recomputed before run creation and participate in the
clamped resume limit. Usage events, policy updates, and resume admission share advisory locks.
Finish-scope paths reject traversal, absolute/drive paths, backslashes, and unsupported globs.

## Cross-feature isolation

Running the budget-revision E2E immediately before
`tools/e2e_factory_publication.mjs` exposed that `/api/demo/reset` retained
`corp_budget_policies`. The publication harness therefore observed authority from the preceding
scenario. Commit `3a3702e` makes reset delete the demo Corp policy transactionally.

The exact sequential order then passed. Publication still proved its full crash/restart, role,
budget, breaker, exact-object lookup, 1,003-item Project, exact PR head/content, fork rejection, and
single-branch/single-PR boundaries with auto-merge, merge, and deployment false.

## Browser verification

The React app ran against the live server at the integrated branch. Chromium exercised:

1. a 6,000-token mission suspending with zero remaining token authority;
2. a disabled **Budget revision required** resume control;
3. the owner proposal form with a 20,000-token ceiling;
4. optional finish scope retaining the verifier policy and reducing the task to 4,000 tokens;
5. Bob's member view with approve/reject disabled;
6. Alice's approval producing explicit approver evidence and 14,000 remaining tokens;
7. an enabled resume action; and
8. completed same-session recovery with 6,012 cumulative tokens and 13,988 remaining.

A separate stop-stage fixture rendered **Stop-stage run cannot resume** and directed the operator to
create a new bounded mission from preserved evidence rather than overriding the breaker.

The browser retry lane disconnected realtime delivery, allowed the server to commit a proposal,
dropped the HTTP response, and retried from the still-open form. The stable proposal key returned
the one pending revision. The same response-loss sequence on approval returned the same version-2
approved revision and approver; no duplicate row or decision was created.

Desktop Chromium at 1,440 pixels and mobile Chromium at 390 pixels both had zero console/page
errors and no horizontal overflow.

Screenshots:

```text
output/playwright/budget-recovery-proposal.png
output/playwright/budget-recovery-completed-desktop.png
output/playwright/budget-recovery-mobile.png
```

## Validation commands

The integrated branch passed:

```text
cargo check --workspace
pnpm build:web
pnpm lint:web
git diff --check
tools/e2e_budget_revision.mjs
tools/e2e_factory_publication.mjs
```

On the integrated `c80c36a4e26d32d7fb236f19edaefc1102f1d1c7` head, the expanded budget
suite ran immediately before the expanded publication suite on the same rebuilt binaries. Budget
recovery passed all rolling-budget, room, stale-contract, path, and lineage cases. Publication then
passed start/recovery/status room isolation and membership revocation immediately before branch,
pull-request, and Project effects, while retaining every prior publication invariant.

After the final documentation integration, exact committed head
`3b0ad5377265620a0cede2046156862d7f379197` passed all 27 migration checks,
`cargo fmt --check`, warning-free workspace clippy, all 80 Rust tests, the production web build,
web lint, and `git diff --check`.

The exact committed head `70b615e416add32ca64da9107983bd4a8caef707` then passed the complete
repository-required sequence:

```text
node tools/check_migrations.mjs
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm build:web
pnpm lint:web
```

Migration validation reported 27 immutable migrations. The Rust workspace ran 80
non-documentation tests with zero failures. The web production build and lint completed without
errors.

## Final publication-main integration — September 2, 2026

PR #77 normally merged publication `main` at
`d7de6c89155ed059da05cf54bedc381d743df232`. Publication publisher credentials remain migration
0027, so the budget-revision schema moved without content changes to
`0028_mission_budget_revisions.sql`. Migration validation reported 28 ordered immutable checksums.

Exact merge head `10f39fe6591de05508c9979615cdfd98cfd4404c` passed formatting,
warning-free workspace clippy, all 95 Rust tests, the production web build, web lint, and
`git diff --check`.

The first runtime attempt intentionally remains disclosed: without the documented fake Codex
app-server configuration, the installed real Codex CLI reported 33,185 tokens and correctly reached
`stop` against the 6,000-token fixture. No product change was made for that harness error. The
isolated stack was recreated with `CRONY_CODEX_COMMAND=node` and
`CRONY_CODEX_COMMAND_ARGS=scripts/fake-codex-app-server.mjs`.

On that exact binary and fresh database, `tools/e2e_budget_revision.mjs` passed at
`2026-09-02T19:51:23.136Z`, immediately followed by
`tools/e2e_factory_publication.mjs` at `2026-09-02T19:56:51.850Z`. The budget report proved a
pre-dispatch failure marked `dispatch_not_started` did not strand the preserved source run; retry
reused the same provider session and workspace lineage and completed. The publication report then
retained every trusted-publisher, idempotency, authority, recovery, and non-disclosure invariant.

Chromium exercised the operator path at 1,440 and 390 pixels. A real `suspend`-stage run was used;
only its run/task/mission status triplet was changed to `cancelled` to reproduce the reviewed UI
state. The cancelled mission still exposed **Authorize recovery budget**. Alice proposed and
approved the revision in the browser. An injected repository mismatch then produced HTTP `409` and
a failed descendant with no workspace plus `workspace_detail=dispatch_not_started`; the
**Resume agent session** control remained available. After restoring the task contract, the browser
retry completed with the same provider session and workspace lineage.

Final browser state recorded 6,012 consumed tokens, mission status `completed`, no page errors,
zero console errors after the successful retry, and no horizontal overflow at either viewport.
The one console error during the negative lane was the expected HTTP `409`.

Screenshots:

```text
output/playwright/pr77-cancelled-suspend-recovery-desktop.png
output/playwright/pr77-cancelled-suspend-recovery-mobile.png
output/playwright/pr77-pre-dispatch-retry-completed-desktop.png
output/playwright/pr77-pre-dispatch-retry-completed-mobile.png
```
