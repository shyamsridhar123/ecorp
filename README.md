# Crony Corp

> **Naming status:** Crony Corp is the internal repository codename. The preliminary public-name
> recommendation is **Guildframe**, pending legal clearance. See
> `docs/BRAND_NAME_AND_LICENSING_REVIEW.md`.

**A multiplayer command center where humans and autonomous agents run a persistent company together.**

Crony Corp combines a real-time shared office, durable missions and approvals, isolated agent
execution, and evidence-backed completion. The office floor is a projection of actual system
events: agents move because work is happening, not because an animation timer fired.

> Status: early vertical-slice implementation.

## Architecture

Crony Corp has three strict planes:

1. **Experience:** web and desktop clients.
2. **Collaboration/control:** rooms, missions, tasks, leases, approvals, events, and audit.
3. **Execution:** runner daemons that own PTYs, worktrees, sandboxes, and agent processes.

The UI can close without terminating active work.

## Current vertical slice

- Shared demo Corp with three human actors and two agent identities
- Durable room membership, human-agent messages, replies, mentions, and work-item links
- Durable Postgres state and event journal
- Production OIDC authentication with Corp-scoped, deny-by-default RBAC
- One-time runner enrollment, rotating workload credentials, replay rejection, and revocation
- Encrypted, actor/task/tool/resource-scoped secret delivery with metadata-only audit events
- Durable risky-action suspension with role-gated, idempotent decisions
- Run, mission, requester, and Corp budgets with steer-constrain-suspend-stop incidents
- Versioned 100-scenario evaluation corpus with deterministic regression metrics
- Consolidated chaos evidence and Windows/macOS/Linux runner CI
- Thin Tauri 2 desktop shell with `crony://` deep links and Windows build smoke coverage
- MCP, ACP, and A2A gateways with explicit version negotiation and schema isolation
- Browser WebSocket updates
- Resumable, room-filtered event replay
- Outbound-connected runner daemon
- Persisted runner heartbeats, disconnect grace, and active-run reconciliation
- Pluggable `AgentAdapter` lifecycle contract with explicit feature capabilities
- Real child-process execution through a deterministic fake-agent adapter
- Native OpenAI Codex app-server adapter with structured lifecycle streaming
- Normalized Claude Code and OpenCode CLI adapters with common evidence
- Durable Codex provider sessions with start, live steer, interrupt, stop, and resume
- Per-run token usage plus SHA-256 evidence for tracked and untracked file changes
- Dedicated Git branch and linked worktree for every write-capable task lineage
- Fail-safe worktree cleanup that preserves dirty, committed, or uncertain work
- Replaceable manager strategies that produce bounded, dependency-aware task graphs
- Deterministic capability matching, parallel root dispatch, dependency release, and bounded retries
- Runner-side evidence policies for artifacts, files, commands, tests, JSON schemas, and screenshots
- Human approval and independent-review gates with role and requester separation
- Mission creation and launch
- Agent control leasing with private rotating fencing tokens
- Explicit release and transfer, queued messages, and role-gated emergency stop
- Server-mediated content-addressed artifact storage with digest/media verification, HMAC-signed
  provenance, retention metadata, and Corp-authorized downloads
- Live office, operations panel, and activity replay

## Run locally

Prerequisites:

- Rust
- Node.js and pnpm
- Docker
- An authenticated `codex` CLI for real Codex missions (optional; the fake adapter works offline)

```powershell
Copy-Item .env.example .env
docker compose -f deploy/compose/docker-compose.yml up -d
pnpm install
cargo build --workspace
```

The supported development path performs demo bootstrap and runner enrollment automatically:

```powershell
./tools/start_local.ps1
```

Open `http://127.0.0.1:5187`.

The runner uses the current Git repository and `HEAD` as its source by default. Set
`CRONY_SOURCE_REPOSITORY` and `CRONY_SOURCE_BASE_REF` to choose another local checkout and base.
Agents receive linked worktrees below `CRONY_RUNNER_WORKSPACE`; they never execute in the source
checkout itself.

In production, set `CRONY_MODE=production`, configure an HTTPS `CRONY_OIDC_ISSUER`, provision
`human_identities`, and explicitly enroll each runner. Demo endpoints and claimed demo identities
are unavailable in production. `CRONY_SECRET_MASTER_KEY_HEX` must contain a deployment-managed
32-byte key encoded as 64 hexadecimal characters. Production also requires
`CRONY_ARTIFACT_SIGNING_KEY_HEX` and a private S3-compatible bucket. Shared snapshots contain only
the authorized artifact API URI and signed metadata, never runner-local paths or direct bucket URLs.

## Documents

- [`docs/PRODUCT_AND_TECHNICAL_PLAN.md`](docs/PRODUCT_AND_TECHNICAL_PLAN.md)
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- [`docs/SECURITY.md`](docs/SECURITY.md)
- [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md)
- [`docs/EVALS.md`](docs/EVALS.md)
- [`docs/BACKLOG.md`](docs/BACKLOG.md)
- [`docs/BRAND_NAME_AND_LICENSING_REVIEW.md`](docs/BRAND_NAME_AND_LICENSING_REVIEW.md)
- [`docs/evidence/2026-08-29-codex-adapter-validation.md`](docs/evidence/2026-08-29-codex-adapter-validation.md)
- [`docs/evidence/2026-08-29-worktree-isolation-validation.md`](docs/evidence/2026-08-29-worktree-isolation-validation.md)
- [`docs/evidence/2026-08-29-task-graph-validation.md`](docs/evidence/2026-08-29-task-graph-validation.md)
- [`docs/evidence/2026-08-29-evidence-verification-validation.md`](docs/evidence/2026-08-29-evidence-verification-validation.md)
- [`docs/evidence/2026-08-30-identity-validation.md`](docs/evidence/2026-08-30-identity-validation.md)
- [`docs/evidence/2026-08-30-secret-broker-validation.md`](docs/evidence/2026-08-30-secret-broker-validation.md)
- [`docs/evidence/2026-08-30-approval-and-budget-validation.md`](docs/evidence/2026-08-30-approval-and-budget-validation.md)
- [`docs/evidence/2026-08-30-alpha-eval-chaos-platform-validation.md`](docs/evidence/2026-08-30-alpha-eval-chaos-platform-validation.md)
- [`docs/evidence/2026-08-30-artifact-storage-validation.md`](docs/evidence/2026-08-30-artifact-storage-validation.md)
- [`docs/evidence/2026-08-30-external-adapter-validation.md`](docs/evidence/2026-08-30-external-adapter-validation.md)
- [`docs/evidence/2026-08-30-protocol-gateway-validation.md`](docs/evidence/2026-08-30-protocol-gateway-validation.md)
- [`docs/evidence/2026-08-30-tauri-desktop-validation.md`](docs/evidence/2026-08-30-tauri-desktop-validation.md)

## License

Apache-2.0. See `LICENSE` and `NOTICE`.
