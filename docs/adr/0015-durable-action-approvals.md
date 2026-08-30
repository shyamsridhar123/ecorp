# ADR 0015: Durable action approvals and idempotent resume

## Status

Accepted — August 30, 2026

## Decision

Risky side effects use a first-class `action_approvals` record rather than a transient provider
prompt. The record binds the requested action to its Corp, room, mission, task, run, agent, risk,
rationale, required roles, and expiry.

An authorized human decides with a client-generated decision key. The database locks the pending
approval, stores the decision exactly once, and writes a durable runner command in the same
transaction. Repeating the same decision key returns the existing outcome and does not enqueue a
second effect. Conflicting later decisions fail closed.

Runner commands are dispatched from the durable table on the decision request and after runner
reconnection. Every command has an ID, and the runner suppresses duplicates for the lifetime of the
supervised process.

## Consequences

- Browser or server restarts do not lose suspended work.
- A second authorized device can decide the approval.
- A crash between command send and dispatch acknowledgement may redeliver the command, but the
  runner's command ID fence prevents a duplicate process effect.
- Provider-specific approval surfaces remain adapters; Crony's approval record is authoritative.
