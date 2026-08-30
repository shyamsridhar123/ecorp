# ADR 0014: Scoped secret broker

## Status

Accepted — August 30, 2026

## Decision

Secrets are encrypted at rest with ChaCha20-Poly1305 under a deployment master key. Tasks reference
secret IDs plus an environment name, tool, and resource; task contracts never contain plaintext
values.

At dispatch, the server authorizes each reference against:

- the authenticated mission requester;
- the exact task and run;
- the enrolled runner;
- an allowed tool;
- a resource prefix; and
- a maximum expiry.

The grant is durable and auditable, but its event contains metadata only. The decrypted value is
sent once over the authenticated runner channel and held only for process launch.

The initial Codex, Claude-compatible, OpenCode-compatible, and fake-process integrations inject
approved values through child-process environment variables. This is explicitly labeled
`environment_reduced_assurance` because the child runtime can inspect its environment. A future
high-assurance adapter may redeem capabilities through a local authenticated broker without
changing task contracts or server policy.

## Consequences

- Production startup requires `CRONY_SECRET_MASTER_KEY_HEX`.
- Rotating the deployment master key requires an explicit re-encryption procedure.
- Secret plaintext is excluded from prompts, arguments, logs, events, snapshots, and artifacts.
- Revoked secrets cannot produce new grants.
