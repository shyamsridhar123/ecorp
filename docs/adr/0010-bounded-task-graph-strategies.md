# ADR 0010: Bounded task-graph strategies

**Status:** Accepted
**Date:** 2026-08-29

## Context

A single permanent "god agent" would make orchestration provider-specific, difficult to test, and
unsafe when task generation runs away. Messages also cannot substitute for explicit work state.

## Decision

Mission planning is a replaceable `ManagerStrategy` selected by ID. A strategy receives the
mission, available agent identities, adapter capabilities, and constraints, and returns a task
graph.

Every graph is validated before persistence:

- one to eight nodes
- maximum depth four
- no cycles or unknown dependencies
- one to three attempts per task
- bounded per-task and total token budgets
- deterministic assigned-agent and required-adapter matching
- complete objective, output, tools, boundaries, acceptance, write scope, and escalation contract

Scheduling is event-driven. Ready roots launch in parallel when their assigned agents and a
compatible runner are available. Committed completion events release dependents. Failed tasks
return to ready only while attempts remain.

## Consequences

Strategies can later be LLM-backed, hierarchical, or policy-driven without changing task
persistence or scheduler invariants. The first release uses deterministic strategies so graph and
failure behavior are reproducible.
