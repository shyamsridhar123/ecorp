# ADR 0022: Publish verified factory deliverables through a fenced trusted publisher

**Status:** Accepted
**Date:** September 2, 2026

## Context

A verified factory mission can now produce a portable commit/branch deliverable, but a pull request
is an external effect with credentials, partial-success boundaries, and retry races. Treating a
runner worktree path or a GitHub Project status as authorization would expose repository credentials
to the producing agent and could create duplicate branches or pull requests.

## Decision

Pull-request publication is a dedicated Corp-scoped aggregate. The server accepts a publication
request only when:

- the factory work item is authoritatively `verified`;
- its mission and every task have completed with persisted passing verification;
- the selected source deliverable is a ready `commit_branch` artifact whose verification digest,
  byte digest, base commit, head commit, and portable Git bundle are exact;
- no run is blocked by current budget or circuit-breaker authority;
- the target repository, base ref, branch prefix, Project transition, and non-merging behavior fit
  the immutable factory policy; and
- the current human has the dedicated publication permission.

The publication row records the source issue, factory item, mission, selected task/run/artifact,
all task/run/evidence IDs, target repository/base/branch/commit, title/body, actor, explicit
authorization snapshot, effect key, initial idempotency key, attempts, failure, pull-request
identity, and Project status transition.

A trusted publisher CLI owns GitHub authentication. It receives an opaque, expiring publisher
token from the server; that token is omitted from snapshots and events. Repository credentials
remain in the publisher's GitHub credential helper or process environment and are never added to
mission contracts, agent environments, command arguments, artifacts, or durable ECorp state.

The effect sequence is monotonic:

1. persist `publishing` and start a fenced attempt;
2. import the signed portable Git bundle and prove its commit descends from the authorized base;
3. adopt or push the exact commit to the exact branch without force;
4. adopt or create one open pull request with auto-merge disabled;
5. persist the pull-request identity;
6. move the GitHub Project item from `In Progress` to the policy's review status; and
7. persist `published`, update deliverable integration state, and close the attempt.

Every local checkpoint is idempotent. On restart or lost response, a new attempt first probes the
remote branch, pull request, and Project status, adopts matching external success, and rejects any
conflicting identity. Expired attempts are retained as abandoned evidence. Failures never remove the
source deliverable or preserved worktree.

Pull-request adoption additionally requires the head to be in the target repository, not a fork,
and to resolve to the exact verified commit. The head repository owner, cross-repository flag, and
head object ID are persisted with the PR identity and checked again by the server. The remote PR
title and body must also exactly match the authorized publication content.

Every publisher-lease renewal revalidates the current attempt actor against its authorization-role
snapshot and reruns mission, verifier, deliverable, policy, run, requester, Corp-budget, and hard
breaker checks in the same transaction before extending authority. A valid start does not preserve
authority after a later demotion, budget exhaustion, policy change, or breaker transition.

When policy uses `HEAD` as the publication base, the publisher resolves the remote symbolic HEAD,
requires a branch target, and verifies that the advertised HEAD object equals the explicit target
ref object before any push. The authorized symbolic input remains `HEAD`, while the resolved branch
name is used for GitHub PR lookup/creation and persisted as the actual pull-request base.

Before creating durable publication state, the publisher performs a read-only remote preflight and
refuses a requested branch equal to the resolved base branch. The same guard runs again immediately
before push. Implicit authorization IDs are deterministically derived from actor and effect identity;
recovery reuses a persisted ID only for the same actor and derives a new actor-bound ID on handoff.
Body files are normalized to LF and trimmed with the same rules as the server before the idempotent
request is constructed.

If the authorized snapshot already contains a durable `published` publication, duplicate calls
return that result before consulting the mutable remote base. Completed publication recovery never
requires the original base branch to remain unchanged. Once any publication exists, retries default
to its persisted source deliverable rather than reselecting among other mission deliverables.

Publication does not call GitHub merge APIs, enable auto-merge, deploy, or authorize those effects.

## Consequences

- Duplicate and concurrent requests converge on one durable publication, branch, and pull request.
- A remote success followed by a local crash is recoverable without repeating an irreversible
  effect.
- Operators can inspect publication attempts and full provenance through the API and UI without
  receiving credentials or fencing tokens.
- A stale base, conflicting branch, closed pull request, enabled auto-merge, hard breaker, exhausted
  budget, wrong Corp, or insufficient role fails closed.
- Merge and deployment require future aggregates with distinct current authorization.
