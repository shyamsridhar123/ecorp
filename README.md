<div align="center">

<img src="./docs/assets/ecorp-relay-mascot.png" width="176" alt="Relay, the ECorp pixel-art operations mascot">

<h1>ECORP</h1>

<h3>Run an AI company on top of your codebase.</h3>

<p><strong>Give ECorp a Git repository. It gives your agents a mission control room.</strong></p>

<p>
Dispatch GitHub Copilot, OpenAI Codex, Claude Code, OpenCode, and deterministic workers into
isolated worktrees. Watch the work live. Steer active sessions. Approve risky actions. Accept only
evidence-backed results.
</p>

<p><strong>One repository. Multiple agents. Human authority. Receipts for every mission.</strong></p>

<p>
<img alt="Rust control plane" src="https://img.shields.io/badge/control_plane-Rust-111111?style=flat-square&logo=rust">
<img alt="React operations console" src="https://img.shields.io/badge/operations_console-React-111111?style=flat-square&logo=react">
<img alt="PostgreSQL durable state" src="https://img.shields.io/badge/durable_state-PostgreSQL-111111?style=flat-square&logo=postgresql">
<img alt="Apache 2.0 license" src="https://img.shields.io/badge/license-Apache--2.0-d71920?style=flat-square">
</p>

</div>

![ECorp operations control room](./docs/assets/ecorp-control-room.png)

<p align="center"><sub><strong>LIVE CONTROL FLOOR:</strong> concurrent Claude Code and OpenCode adapter sessions plus a deterministic worker, projected from actual runner state.</sub></p>

<p align="center"><strong>Meet Relay.</strong> Part dispatcher, part safety officer, always asking for evidence.</p>

## Not another agent chat. An operating system for agent work.

Most agent tools disappear behind a terminal or a conversation. ECorp turns repository work into a
durable operating process that humans can see, control, and audit.

| You get | What it means |
| --- | --- |
| **A live control floor** | Agent movement, status, review, and approval states are projections of real provider sessions—not decorative animation. |
| **Your choice of intelligence** | Select a connected runtime, an account-enabled GitHub Copilot model, and supported reasoning effort for each mission. |
| **Safe parallel execution** | Every write-capable run receives its own Git branch and linked worktree. Agents never edit the configured source checkout directly. |
| **Human authority at the point of risk** | Steer live work, interrupt a turn, transfer control, approve scoped actions, or issue an audited emergency stop. |
| **Proof before completion** | Artifacts, files, commands, tests, schemas, screenshots, human approval, and independent review can gate success. |
| **Durable operations** | Missions, rooms, messages, approvals, budgets, events, provider sessions, and signed artifact metadata survive the browser session. |

## From repository to verified result

```text
1. Point ECorp at a Git repository
                  ↓
2. Define the outcome, runtime, model, strategy, and safety budget
                  ↓
3. ECorp creates bounded tasks and isolated worktrees
                  ↓
4. Agents execute while humans watch, steer, and approve
                  ↓
5. Verification runs before the mission can complete
                  ↓
6. ECorp records the result, evidence, provenance, and audit trail
```

Choose one focused agent or run two independent specialists followed by a dependency-gated
synthesis task. The scheduler releases only ready work, matches it to a compatible connected
runner, and keeps retries, depth, fan-out, tokens, and cost inside explicit bounds.

## Bring the agents you already trust

| Runtime | ECorp integration |
| --- | --- |
| **GitHub Copilot** | Official SDK integration, live account model discovery, model and reasoning selection, streaming, steering, interruption, approvals, usage, evidence, and resumable sessions. |
| **OpenAI Codex** | Native app-server lifecycle with structured output, live steering, interrupt, stop, resume, usage, and repository-change evidence. |
| **Claude Code** | Normalized CLI execution with streaming, interruption, stop, resume, and common evidence. |
| **OpenCode** | Normalized CLI execution through the same governed runner and evidence contract. |
| **Deterministic harness** | Quota-free, no-AI lifecycle fixture for testing orchestration, verification, approvals, budgets, and failure paths. |

The provider is not the control plane. Runners advertise exactly what they support, and ECorp
dispatches only when the requested adapter, model, and reasoning capability are available.

## Control without surrendering the repository

```text
Web console · Tauri desktop · CLI · MCP / ACP / A2A
                         │
                    REST + WebSocket
                         ▼
              ┌─────────────────────┐
              │ ECorp control plane │
              │ missions · policy   │
              │ rooms · approvals   │
              │ events · artifacts  │
              └──────────┬──────────┘
                         │ fenced, outbound assignments
                         ▼
              ┌─────────────────────┐
              │ trusted runner node │
              │ provider supervisor │
              │ isolated worktrees  │
              └──────────┬──────────┘
                         ▼
                verification + signed evidence
```

The server never executes an agent shell command. Outbound-connected runner daemons own provider
processes, worktree isolation, artifact collection, and verification. Postgres remains the
authoritative state and event journal, so closing the UI does not terminate active work.

## Safety is part of the workflow

- **Deny-by-default access:** production OIDC identity, Corp-scoped RBAC, room membership, and
  one-time WebSocket tickets.
- **Revocable runner identity:** short-lived enrollment, rotating workload credentials, replay
  rejection, reconnect grace, and fenced assignments.
- **Scoped secrets:** encrypted at rest and released only for an authorized actor, task, run,
  runner, tool, resource, and expiry window.
- **Durable approvals:** ambiguous, external, networked, or policy-controlled actions pause until
  an authorized human decides.
- **Circuit breakers:** run, mission, requester, and Corp budgets can steer, constrain, suspend, or
  stop runaway work.
- **Artifact provenance:** content-addressed storage, SHA-256 integrity, media validation,
  retention metadata, HMAC-signed provenance, and authorized downloads.

Read the exact boundaries in [`docs/SECURITY.md`](docs/SECURITY.md) and
[`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md).

## Start locally

### Fastest path: Windows PowerShell

Prerequisites: Rust, Node.js, pnpm, Docker, and at least one optional authenticated provider CLI or
GitHub Copilot account. The deterministic harness works without provider credentials.

```powershell
git clone https://github.com/shyamsridhar123/ecorp.git
cd ecorp
./tools/start_local.ps1
```

Open **http://127.0.0.1:5187**.

The startup script launches Postgres, the Rust control plane, an enrolled outbound runner, and the
React operations console.

### Point ECorp at another repository

```powershell
$env:CRONY_SOURCE_REPOSITORY = 'C:\path\to\your\repository'
$env:CRONY_SOURCE_BASE_REF = 'HEAD'
./tools/start_local.ps1
```

The runner validates the repository and base ref at startup. Each mission then receives a dedicated
worktree beneath `CRONY_RUNNER_WORKSPACE`.

### Run the engineering checks

```powershell
pnpm install --frozen-lockfile
pnpm check
```

## What to try first

1. Choose **GitHub Copilot**, **Codex**, **Claude Code**, or **OpenCode**.
2. Select a model, reasoning effort, and mission budget when the provider exposes them.
3. Describe an outcome and the proof you expect.
4. Use **One agent** for a focused build or **Two specialists, then synthesis** for competing
   approaches.
5. Keep **Pause after planning** enabled when you want to inspect the task graph before dispatch.
6. Approve scoped actions directly inside the mission card.
7. Download the verified artifact when the mission completes.

Example mission:

> Build a browser-playable multiplayer game with power-ups and boss battles. Test the complete
> gameplay loop, attach browser evidence, and do not modify files outside the assigned worktree.

## Current state

ECorp is an active alpha with a working web console, Rust control plane, Postgres event journal,
outbound runner, provider adapters, thin Tauri desktop shell, protocol gateways, approval system,
budget circuit breakers, worktree isolation, and evidence-gated completion.

It is not yet a safe sandbox for fully untrusted child processes. Development mode intentionally
includes fixed demo identities, permissive local CORS, and a deterministic process that runs with
the local user's permissions. Production deployments require OIDC, deployment-managed keys,
explicit runner enrollment, and private S3-compatible artifact storage.

The public product is **ECorp**. Existing `crony-*` binaries, `CRONY_` environment variables, and
`X-Crony-*` headers remain supported for compatibility during the transition.

## Go deeper

- [User and developer journey](docs/USER_AND_DEVELOPER_JOURNEY.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Security](docs/SECURITY.md)
- [Threat model](docs/THREAT_MODEL.md)
- [Evaluation strategy](docs/EVALS.md)
- [Product and technical plan](docs/PRODUCT_AND_TECHNICAL_PLAN.md)
- [Backlog](docs/BACKLOG.md)

## License

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
