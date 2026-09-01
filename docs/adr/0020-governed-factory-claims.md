# ADR 0020: Govern factory intake with durable fenced claims

**Status:** Accepted
**Date:** September 1, 2026

## Context

ECorp Build GitHub Project #3 is the operational source of truth for planned work, but a GitHub
issue is not an execution lease. Multiple controllers, retries, process restarts, and delayed API
responses can otherwise create duplicate missions or let stale automation continue.

The factory must compose ECorp's existing mission, budget, approval, worktree, and verification
boundaries. It must not create a privileged side channel that executes work or mutates GitHub based
only on transient controller memory.

## Decision

Before mission creation, a trusted controller must create a Corp-scoped `factory_work_item`:

- one unique record per GitHub Project item;
- a source revision and policy snapshot;
- an owner, expiring lease, opaque fencing token, and monotonic version;
- a Corp-global idempotency ledger for claim, renewal, and materialization operations;
- an optional durable link to exactly one ECorp mission.

Controllers serialize competing operations with transaction-scoped advisory locks and then enforce
the relational uniqueness constraints. Renewal and materialization require the current actor,
fencing token, version, and an unexpired lease.

Mission and task creation happens in the same Postgres transaction that advances the factory item
to `mission_created`. A successful retry returns the existing mission rather than planning a
second one.

Factory fencing tokens are returned only to the authorized claimant. They are not included in
Corp snapshots, events, mission prompts, or agent-visible artifacts.

Non-terminal factory states remain reclaimable after lease expiry without rewriting their source
revision, policy snapshot, or mission linkage. Fenced state transitions record `running`,
`blocked`, verification, and publication stages. Invalid regressions fail closed.

Materialization is constrained by the immutable policy snapshot recorded at claim time. The plan
may narrow but cannot widen repository, adapter, strategy, model, reasoning effort, tool, secret,
prohibition, write-scope, token, or cost authority. A transition to `verified` additionally checks
the authoritative mission and every task's persisted verification result.

The generic transition API cannot enter `publishing` or `published`; a later dedicated publication
operation must own those effects and their provenance.

GitHub Project status changes and pull-request publication are later effects. They may occur only
after the corresponding ECorp state is durable. Pull-request publication does not authorize merge
or deployment.

## Consequences

- Server restart and duplicate delivery cannot create a second mission for one claimed issue.
- A stale controller fails closed instead of renewing or materializing another controller's claim.
- Operators can inspect issue-to-mission provenance without receiving capability tokens.
- Expired claims can be reclaimed without deleting the original work-item history.
- Later GitHub integration must use this aggregate rather than treating labels or Project status as
  an execution lock.
