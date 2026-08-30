# Identity and authorization validation — August 30, 2026

## Scope

This evidence covers production human authentication, Corp RBAC, WebSocket authorization, and
runner workload identity.

## Automated path

`node tools/e2e_identity.mjs` performs the following against a real Postgres database and compiled
server:

1. creates a one-time runner enrollment token;
2. registers a WebSocket runner and receives a rotated workload credential;
3. proves replay of the enrollment token is rejected;
4. proves the workload credential rotates on reconnect;
5. revokes the runner and proves the rotated credential is rejected;
6. starts a production-mode server against the protocol-faithful local OIDC fixture;
7. proves a missing bearer token returns 401;
8. proves a mapped OIDC subject can read its Corp;
9. proves actor spoofing, an unmapped identity, and cross-Corp access return 403; and
10. proves the browser WebSocket is authorized before replay begins.

Machine-readable output is written to `output/e2e-identity.json` and uploaded by CI.

## Security boundary

The OIDC fixture is test-only and never selected by production configuration. Production startup
requires an explicit issuer and rejects plaintext HTTP unless the isolated-test override is set.
The fake fixture validates protocol and policy behavior; it does not replace an integration test
with the deployment's chosen identity provider.
