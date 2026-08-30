# ADR 0019: MCP, ACP, and A2A are gateway boundaries

## Status

Accepted — August 30, 2026

## Decision

Crony's mission, task, approval, budget, and lease models remain internal. External protocols are
adapters over the authenticated REST and event surfaces:

- MCP exposes three Corp-scoped tools for snapshots, mission creation, and room messages.
- ACP maps local client sessions and prompts to Crony mission/run identities.
- A2A exposes an agent card, message-to-mission submission, task snapshots, and SSE updates.

Each gateway declares and validates a protocol version. Unknown versions and methods fail closed.
Private assignment tokens, lease tokens, database rows, and secret values are never projected into
external schemas.
