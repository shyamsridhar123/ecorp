# ADR 0009: Worktree isolation and preservation

**Status:** Accepted
**Date:** 2026-08-29

## Context

Parallel write-capable agents cannot share the configured checkout. Automatic cleanup also cannot
discard uncommitted or unintegrated work merely because a process ended.

## Decision

The runner provisions one deterministic Git branch and linked worktree for each initial task run.
Provider-session resumes reuse the root run's worktree and branch.

The workspace manager:

- validates the source repository and base ref before connecting
- resolves every managed path below one canonical worktree root
- serializes Git worktree mutations
- rejects occupied or mismatched paths without falling back to the source checkout
- preserves dirty trees, ignored files, commits ahead of the current base,
  detached/mismatched trees, and every state it cannot verify
- removes a worktree only when it is clean and integrated or tree-equivalent to the current base
- deletes the branch only through Git's safe `branch -d` check
- records active, preserved, or removed disposition on the run

## Consequences

Agent work survives crashes and cancellation by default. Preserved worktrees require a later
integration or explicit operator action. A future garbage collector may reclaim them only by
re-running the same fail-safe checks.
