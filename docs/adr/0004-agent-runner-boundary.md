# ADR 0004: Agent runner boundary

**Status:** Accepted  
**Date:** 2026-08-29

## Decision

A separate outbound-connected runner starts PTYs and child processes. The server issues scoped run
assignments but never executes shell commands.

## Consequence

Runner identity, leases, fencing tokens, reconnect, and process cleanup are security-critical.

