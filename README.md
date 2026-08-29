# Crony Corp

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
- Browser WebSocket updates
- Resumable, room-filtered event replay
- Outbound-connected runner daemon
- Real child-process execution through a deterministic fake-agent adapter
- Mission creation and launch
- Agent control leasing with private rotating fencing tokens
- Explicit release and transfer, queued messages, and role-gated emergency stop
- Artifact creation and SHA-256 evidence
- Live office, operations panel, and activity replay

## Run locally

Prerequisites:

- Rust
- Node.js and pnpm
- Docker

```powershell
Copy-Item .env.example .env
docker compose -f deploy/compose/docker-compose.yml up -d
pnpm install
cargo build --workspace
```

Start the three processes in separate terminals:

```powershell
cargo run -p crony-server -- --bind 127.0.0.1:8791
cargo run -p crony-runner -- --server-ws ws://127.0.0.1:8791/ws/runner
pnpm --dir apps/web dev --host 127.0.0.1 --port 5187 --strictPort
```

Open `http://127.0.0.1:5187`.

## Documents

- [`docs/PRODUCT_AND_TECHNICAL_PLAN.md`](docs/PRODUCT_AND_TECHNICAL_PLAN.md)
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- [`docs/SECURITY.md`](docs/SECURITY.md)
- [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md)
- [`docs/EVALS.md`](docs/EVALS.md)
- [`docs/BACKLOG.md`](docs/BACKLOG.md)

## License

Apache-2.0. See `LICENSE` and `NOTICE`.
