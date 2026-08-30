# Security

## Current status

The current implementation has production authentication and workload identity boundaries, but it
is not yet suitable for an untrusted network until scoped secrets and durable artifact storage land.

Known development-only shortcuts:

- fixed demo identities
- permissive CORS
- host-local artifact paths
- fake process runs with the local user's permissions
- no network sandbox

These are explicit backlog items, not production claims.

Production mode validates OIDC bearer tokens against the configured issuer's UserInfo endpoint and
maps `(issuer, subject)` to a Corp-local human actor. Claimed actor IDs cannot override that mapping.
Development mode still enforces room membership in persistence, snapshots, writes, WebSocket
replay, and live delivery. Eve is a deliberate non-member fixture used to prove that room-scoped
missions, tasks, runs, messages, and events are not returned.

Runner nodes require one-time enrollment followed by rotating, expiring workload credentials.
Only credential hashes are stored. Replayed, expired, unknown, and revoked credentials are denied.

## Required production boundaries

- Every persistent object is scoped to a Corp.
- Authorization runs before reads, writes, and subscriptions.
- Human identity uses OIDC and passkeys.
- Agent and runner identities are independent and revocable.
- The server does not execute untrusted shell commands.
- Runners connect outbound and receive scoped assignments.
- Long-lived secrets never enter prompts, logs, command arguments, or agent-readable files.
- Irreversible effects require authorization and idempotency.
- Artifacts are content-hashed.
- Agent control uses rotating fencing tokens; stale tokens are rejected.
- Lease tokens are returned only when the current controller explicitly claims or renews control;
  they are omitted from shared snapshots, transfers, and event payloads.
- Emergency stop is role-gated and audited.
- Runner connections use epochs, and run assignments use independent private fencing tokens.
- Assignment tokens are omitted from shared snapshots and event payloads.
- A stale runner cannot turn a `lost` run back into an active or cancelled run.

## Reporting

Until private vulnerability reporting is enabled on the GitHub repository, report security issues
directly to the repository owner rather than opening a public issue with exploit details.
