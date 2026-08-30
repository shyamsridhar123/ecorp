# Security

## Current status

The current implementation has production authentication, workload identity, and scoped secret
boundaries, but it is not yet suitable for an untrusted network until durable artifact storage and
stronger process isolation land.

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

Secrets are encrypted with ChaCha20-Poly1305 and authenticated associated data. The broker checks
actor, task, run, runner, tool, resource, and expiry scope before dispatch. Events and snapshots
contain grant metadata only. Environment injection is labeled reduced assurance.

Risky action approvals are Corp-scoped, role-gated, expiring, and idempotent. Approval decisions
transactionally enqueue durable runner commands, and command IDs fence duplicate delivery.

The GitHub Copilot permission handler automatically approves writes inside the assigned worktree,
read-only operations it can prove are scoped to that worktree, and reads from the SDK state
directory isolated to that worktree. It canonicalizes existing ancestors to reject symlink escapes.
External paths, network URLs, sandbox bypass, managed-policy approvals, and ambiguous shell
commands suspend durably. Shell approval cards include the bounded command text rather than only a
generic action label.

Budget policies constrain run, mission, requester, and Corp usage. Repeated tools and explicit
no-progress events feed an auditable circuit breaker; ordinary human conversation does not.

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
