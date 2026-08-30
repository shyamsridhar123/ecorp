# Threat model

## Assets

- source repositories and worktrees
- credentials and integration tokens
- human and agent identities
- private conversations
- tasks, decisions, and approvals
- artifact integrity
- compute and model budgets
- audit history

## Threats and planned controls

| Threat | Initial control | Production control |
|---|---|---|
| Cross-Corp access | Corp IDs on tables | authorization before query/subscription plus isolation tests |
| Duplicate effects | event idempotency key | effect ledger and fencing token |
| Runner impersonation | local-only deployment | enrollment and short-lived mTLS certificate |
| Malicious agent process | dedicated branch and worktree | container/user isolation and deny-by-default network |
| Secret exfiltration | fake adapter has no secrets | secret broker with short-lived capability |
| Path traversal | canonical worktree-root validation | mount boundary and sandbox policy |
| Prompt injection | deterministic adapter | trust labels, tool policy, and source-aware context |
| Memory poisoning | no persistent memory yet | provenance, scope, expiry, and admission policy |
| Runaway spend | deterministic local adapter | task/run/Corp budgets and breaker |
| Recursive delegation | bounded acyclic task graphs | causal-depth and task-generation limits |
| Conflicting human control | expiring agent lease | authenticated lease, transfer, and emergency-stop policy |
| Forged artifact | runner policy checks plus SHA-256 evidence | object-store digest, provenance, verifier attestation |
| Event replay | unique idempotency | signed runner events and replay window |
| Plugin supply chain | no plugins yet | signed manifests, allowlists, and isolated permissions |

## Review cadence

Update this model when adding:

- authentication
- a new agent adapter
- remote runners
- integrations or MCP servers
- artifact downloads
- autonomous deployments
- persistent memory
