# ADR 0013: Runner enrollment and rotating workload identity

## Status

Accepted — August 30, 2026

## Decision

Runner nodes are bound to one Corp and must present either:

1. a short-lived, one-time enrollment token created by a Corp owner or admin; or
2. the currently valid workload credential returned by the previous successful connection.

The server stores only SHA-256 token digests. A successful registration atomically consumes the
enrollment token or current credential, rotates it, records an audit event, and returns the next
credential. Reuse of an old token fails closed. Revocation marks the credential unusable and
disconnects the active runner.

Credentials are persisted in a runner-local file rather than command-line arguments. The initial
enrollment file is deleted after successful registration. Runner selection is Corp-scoped.

## Consequences

- A copied old token cannot create a second runner session after rotation.
- A runner that loses the newly issued credential must be explicitly re-enrolled.
- TLS is required for remote production connections; this opaque rotating credential is the
  equivalent workload identity for the initial release, without requiring a private CA.
- A future mTLS transport may replace bearer transport without changing enrollment semantics.
