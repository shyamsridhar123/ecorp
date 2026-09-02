# ADR 0021: Portable source deliverables

**Status:** Accepted
**Date:** September 2, 2026

## Context

Provider evidence proves what an adapter reported, but it is not necessarily the application source
that a collaborator needs to review or integrate. Preserved runner worktrees are valuable recovery
state, yet remote operators cannot depend on runner-host filesystem access.

## Decision

Every task contract declares one typed deliverable form:

- verified commit and isolated task branch bundle;
- deterministic binary Git patch;
- content-addressed source archive;
- typed artifact set; or
- review-only report.

After automated verification passes, the runner builds the deliverable from a temporary Git index.
This captures tracked modifications, additions, deletions, and non-ignored untracked files without
mutating the configured source checkout. Provider evidence is excluded. Symbolic links, Git links,
path escapes, runner internals, ignored files, and secret-like paths are rejected.

The runner may create a commit only after verification and only on the isolated task branch. It
never pushes, opens a pull request, enables auto-merge, merges, or deploys as part of completion.

The exported bytes enter the existing bounded staging protocol. The server verifies their length,
digest, media type, role, file name, and deliverable metadata; signs the provenance; publishes the
content-addressed object; and persists a source-deliverable record. The record links:

- task and run;
- exact exported byte digest;
- exact normalized verification-report digest;
- base commit;
- optional post-verification head commit;
- isolated branch;
- retention; and
- integration state.

The server acknowledges durable source storage to the runner. Verification cannot transition to
passed until the persisted verification digest and deliverable digest match the ready object.
Workspace cleanup starts only after that acknowledgment.

## Consequences

- A remote room member can download the actual reviewable result without runner-host access.
- Provider evidence, verifier evidence, source deliverables, and integration state remain separate
  product concepts.
- Clean worktrees can still be reclaimed, but never before the source deliverable is durably ready.
- Pull-request publication and merge remain later, separately authorized and idempotent effects.
