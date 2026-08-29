# ADR 0002: Three-plane architecture

**Status:** Accepted  
**Date:** 2026-08-29

## Decision

Separate experience, collaboration/control, and execution.

- Clients display state.
- The server owns organizational state.
- Runners own agent processes.

## Consequence

Closing or updating a client cannot terminate active work.

