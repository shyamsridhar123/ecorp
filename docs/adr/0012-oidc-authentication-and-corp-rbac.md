# ADR 0012: OIDC authentication and Corp-scoped RBAC

## Status

Accepted — August 30, 2026

## Decision

Production servers authenticate human requests with short-lived OpenID Connect bearer tokens.
The configured issuer is discovered at startup and tokens are validated through its UserInfo
endpoint. An `(issuer, subject)` mapping resolves the external identity to exactly one human actor.

Every Corp API resolves the authenticated principal inside the Corp named by the route before
performing a read or write. Claimed actor IDs remain in the current wire format for compatibility,
but production rejects a claim that differs from the authenticated actor. WebSocket replay is
authorized before any event is sent.

The supported human roles are owner, admin, manager, member, guest, and spectator. Authorization
is deny-by-default:

- every role may read visible state;
- guests and above may post room messages;
- members and above may operate missions and agents or decide approvals;
- managers and above may emergency-stop work;
- owners and admins may manage security-sensitive Corp resources.

Development mode keeps the fixed demo actors and credential-free local flow. Demo endpoints do not
exist in production mode.

## Consequences

- Account recovery, MFA, and passkeys are delegated to the configured identity provider.
- Production clients must obtain an OIDC access token and use TLS. They exchange the bearer token
  for a one-time, 30-second WebSocket ticket so the OIDC credential is never placed in a URL.
- Operators must provision an identity mapping before a user can enter a Corp.
- Authorization remains enforced in persistence as defense in depth.
