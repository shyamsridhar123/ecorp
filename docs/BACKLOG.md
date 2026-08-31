# Product backlog

This file is the local source for the initial GitHub milestones and issues.

## M0 — Foundation

| Priority | Issue | Definition of done |
|---|---|---|
| P0 | Establish product contract and ADR set | Product, architecture, security, threat model, evals, and ADRs are committed |
| P0 | Create durable Postgres state and event journal | Migrations, snapshots, idempotent events, and tests |
| P0 | Build deterministic fake-agent adapter | Runner launches a real process and records verified artifact evidence |
| P0 | Add one-command local environment | Start/stop scripts validate ports, services, and health |

## M1 — Multiplayer vertical slice

| Priority | Issue | Definition of done |
|---|---|---|
| P0 | Implement resumable browser event stream | Client resumes from sequence without losing or duplicating events |
| P0 | Harden control leases and handoff | Concurrent claim test, renewal, release, transfer, and emergency stop |
| P0 | Add rooms and threaded human/agent messages | Durable messages appear in both browser sessions |
| P0 | Add runner heartbeat and disconnect reconciliation | Lost runner enters grace state and reconnects by run/fencing token |
| P1 | Package a thin Tauri desktop client | Desktop connects to the same server and does not own process state |

## M2 — Real agent work

| Priority | Issue | Definition of done |
|---|---|---|
| P0 | Introduce the `AgentAdapter` contract | Fake and one real provider implement the same lifecycle |
| P0 | Add Codex adapter | Start, stream, steer, interrupt, resume, usage, and failure evidence |
| P0 | Add per-task git worktrees | Parallel runs cannot edit the same checkout |
| P0 | Implement task graph orchestration | Manager plan produces bounded dependency graph and assignments |
| P0 | Add independent verification policies | Tests and artifact validators gate completion |
| P1 | Add Claude Code and OpenCode adapters | Provider parity suite passes |

## M3 — Safety and trust

| Priority | Issue | Definition of done |
|---|---|---|
| P0 | Add OIDC/passkey authentication and RBAC | No demo identity is accepted in production mode |
| P0 | Add runner enrollment and mTLS rotation | Unknown runner cannot connect |
| P0 | Build scoped secret broker | Agent receives references or short-lived capabilities, not long-lived secrets |
| P0 | Implement durable approvals | Run suspends and resumes exactly once after authorized decision |
| P0 | Add budget policy and circuit breaker | Spend, recursion, repeated tools, and no-progress are bounded |
| P1 | ✅ Add artifact object storage and signed provenance | Host-local paths are removed from shared state |

## M4 — Alpha

| Priority | Issue | Definition of done |
|---|---|---|
| P0 | Build the 100-scenario eval suite | Results include reliability, cost, safety, and intervention metrics |
| P0 | Add chaos and restart testing | Required cases in `EVALS.md` pass |
| P0 | Validate Windows, macOS, and Linux runners | Signed evidence from each platform |
| P1 | Add MCP, ACP, and A2A boundaries | Conformance tests for each supported protocol |
| P1 | Complete public-name and licensing review | Brand, domain, package, art, and attribution decisions recorded |
| P1 | Dogfood with three real teams | Measured setup, completion, recovery, and rework results |


