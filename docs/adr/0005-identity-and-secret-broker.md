# ADR 0005: Independent identities and brokered secrets

**Status:** Accepted  
**Date:** 2026-08-29

## Decision

Humans, agents, services, and runners have separate identities. Long-lived workspace secrets are
not passed to agent prompts, command arguments, or agent-readable files.

## Target

OIDC/passkeys for humans, revocable keys for agents, short-lived certificates for runners, and
task-scoped capabilities from a secret broker.

