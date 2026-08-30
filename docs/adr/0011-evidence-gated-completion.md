# ADR 0011: Evidence-gated completion

**Status:** Accepted
**Date:** 2026-08-29

## Context

An agent's statement that work is complete is not evidence. Verification commands also cannot run
on the control-plane server without violating the execution boundary.

## Decision

Each task stores a typed verifier policy. The runner buffers provider completion, runs automated
checks inside the assigned worktree, and emits immutable evidence linked to the task and run.

Supported automated checks are artifact integrity, files, commands, tests, JSON required-key
schemas, and screenshot signatures. Paths are worktree-relative and reject traversal and symlinks.
Commands execute directly with bounded time and output, never through an interpolated shell.

The server accepts `run.completed` only after complete passing evidence. Failed evidence moves the
task to `verification_failed`. Policies may instead require:

- human approval by an allowed role
- independent review by an allowed human who is neither the requester nor producer

Manual requests and decisions are durable and actor-attributed.

## Consequences

Provider adapters cannot bypass verification by emitting completion early. Approval is supported
for completed evidence packages; general suspension and exactly-once resumption of live side
effects remains a separate control-plane feature.
