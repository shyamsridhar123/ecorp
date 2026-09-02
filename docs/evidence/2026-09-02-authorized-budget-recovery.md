# Authorized mission budget recovery validation

**Date:** September 2, 2026  
**Core implementation:** `c3431f9526e3498c5fa14c0a1fb67ae19f3ff136`  
**Deterministic E2E checkpoint:** `892c3d24d3375575def97fd2c05f3b9521d55ae2`  
**Operator UI:** `c4ce25e`  
**Integrated publication base:** `bef6774851c79a1cd8a773883b9c5b090318b673`  
**Cross-scenario reset isolation:** `3a3702e`

## Scope

This evidence covers GitHub issue #50: durable owner/admin mission budget revision, explicit
original/current/consumed/remaining authority, optional bounded finish scope, pre-dispatch resume
rejection, same-session recovery, duplicate/restart-safe state, terminal overrun behavior, and the
desktop/mobile operator controls.

It does not authorize an override of `stop`, reset usage, widen a write boundary, replace a verifier
policy, merge a pull request, or deploy.

## Durable model and API

Migration `0027_mission_budget_revisions.sql` adds:

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
latest breaker state, and optional finish-scope bounds under transaction locks.

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
publish an artifact or accepted completion.

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

The full repository gate is recorded on the final evidence commit.
