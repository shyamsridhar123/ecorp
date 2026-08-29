# ADR 0003: Transactional state plus immutable events

**Status:** Accepted  
**Date:** 2026-08-29

## Decision

Use relational state for current authoritative data and write immutable domain events in the same
transaction. Use idempotency keys for externally delivered effects.

## Rejected

- Git files as the shared message bus
- pure event sourcing for every product object
- ephemeral in-memory state as the system of record

