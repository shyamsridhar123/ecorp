# ADR 0001: Greenfield modular monolith

**Status:** Accepted  
**Date:** 2026-08-29

## Decision

Build Crony Corp as a new repository with one server binary and one runner binary. Do not fork
Munder Difflin or Buzz as the primary implementation base.

## Rationale

The source projects have incompatible centers: a desktop-owned local harness versus a broad
Nostr-based collaboration platform. A greenfield modular monolith preserves useful ideas without
inheriting either system's coupling or full surface area.

