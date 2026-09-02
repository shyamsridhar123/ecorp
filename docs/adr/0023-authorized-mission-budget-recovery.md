# ADR 0023: Recover budget-suspended missions through explicit revision authority

**Status:** Accepted
**Date:** September 2, 2026

## Context

A provider can reach a useful checkpoint exactly as a mission-level budget is exhausted. Preserving
the provider session and worktree makes `suspend` safer than immediate destruction, but ordinary
resume cannot succeed while the same exhausted ceiling remains authoritative. Resetting usage,
silently granting an overage, or creating an unrelated mission would break auditability and
continuity.

## Decision

Every mission stores immutable original token/cost limits and a current authorized ceiling.
Consumed usage is always the sum of persisted runs and is never reset by recovery.

Budget recovery is a dedicated, durable `mission_budget_revisions` aggregate:

- only Corp owners and admins may propose or decide;
- proposal and decision have separate exact idempotency keys;
- every row records current and proposed limits, usage at proposal, rationale, proposer, status,
  version, decider, decision note, and timestamps;
- only one revision may be pending for a mission;
- at least one current limit must increase while neither may decrease;
- proposed limits must remain within policy bounds and exceed already consumed usage; and
- proposal and approval both reject active runs, completed missions, stale budgets, and any latest
  run that is not a resumable `suspend`.

A proposal may include a bounded finish-scope replacement for one unfinished task. It may change
the objective, expected output, acceptance checks, write paths, and remaining task budget, but it
cannot widen the previous write scope, increase the task budget, or replace the verifier policy.
The previous and replacement contracts and verifier policies are retained with the revision.

Approval atomically updates the mission ceiling and optional task contract after rechecking current
usage and authority under the same transaction lock. Rejection preserves the previous limits.

Resume remains a separate operator effect. Before a new run exists, the server rejects a source run
at `stop` and rejects any mission with no remaining token or cost authority. An accepted resume
reuses the exact provider session and preserved worktree and clamps its run limits to the smaller of
the task contract and remaining mission authority.

The operations UI displays consumed, original, current, and remaining token/cost values plus the
approving actor. Exhausted resume is disabled with actionable guidance. Owner/admin proposal and
decision controls expose the optional bounded finish scope, while members receive a read-only audit
view.

## Consequences

- `suspend` is a recoverable checkpoint without weakening `stop`.
- Additional spend has durable human authority and rationale.
- Usage and original limits remain visible through every revision.
- Recovery preserves provider and repository continuity.
- A narrower finish contract can complete the original mission without a disconnected replacement.
- Duplicate requests and decisions replay without duplicate authority changes.
