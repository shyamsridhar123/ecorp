# Task-graph validation — August 29, 2026

## Scope

This record covers issue #13: replaceable planning strategies, explicit task contracts,
dependency-aware scheduling, deterministic capability matching, graph bounds, and retry limits.

## Deterministic graph scenario

Mission `fe38b0d4-67c4-413f-80f2-c16b416c9880` used
`parallel-specialists` and persisted three tasks:

- two depth-zero specialist roots assigned to different agents and adapters
- one depth-one synthesis task depending on both roots

The launch returned two initial run IDs. Runtime polling observed two active runs concurrently.
The synthesis `run.requested` event was committed only after both root `run.completed` events. All
three tasks completed on distinct worktrees and the mission completed.

## Retry-bound scenario

Mission `d7fd413b-dee1-4411-a14b-8a2033e7b8b0` used the `single` strategy with a deterministic
always-failing adapter task.

- task: `ac21071d-c47d-4ca9-b83f-c5b608573a1e`
- maximum attempts: two
- actual attempts: two
- terminal mission state: failed
- no third run was created

## Unit coverage

The planning suite rejects:

- unknown strategy IDs
- missing agents and adapter mismatches
- duplicate or unsafe task keys
- unknown dependencies and cycles
- incorrect or excessive depth
- retry limits above three
- incomplete contracts
- task budgets above the per-task ceiling
- summed task budgets above the mission ceiling

The same suite proves deterministic assignment for identical mission, agent, and preference input.
