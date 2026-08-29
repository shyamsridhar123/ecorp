# Security

## Current status

The current implementation is a local development vertical slice. It demonstrates architecture
and end-to-end behavior; it is not yet suitable for an untrusted network.

Known development-only shortcuts:

- no human authentication
- fixed demo identities
- no runner enrollment secret or certificate
- permissive CORS
- host-local artifact paths
- fake process runs with the local user's permissions
- no network sandbox

These are explicit backlog items, not production claims.

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

## Reporting

Until private vulnerability reporting is enabled on the GitHub repository, report security issues
directly to the repository owner rather than opening a public issue with exploit details.

