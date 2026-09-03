# Incident Command review-correction evidence

Date: 2026-09-03

## Provenance

- Review-correction issue: GitHub `#96`
- Immutable source base: `5999a4c808867796671dbafe02ccd8af69a74445`
- Authorized published reference: `4dd6be6b528f178609cb5a33fd655c4f031692b3`
- The complete `scenarios/incident-command/**` tree was restored from that
  reference before applying the corrections.
- No external network access or package installation was used.

## Corrections

- A rejected postmortem update now reloads the latest incident while preserving
  dirty textarea values, rejects through the error path, and cannot emit the
  success toast.
- Identity changes synchronously clear incidents and detail state. List and
  detail responses carry a scope generation and are discarded after a tenant or
  identity switch.
- `npm test` explicitly runs only the dependency-free built-in server suite.
  The Playwright workflow remains available through `npm run test:browser` and
  requires `ECORP_PLAYWRIGHT_MODULE`.

## Verification

```text
npm --prefix scenarios/incident-command test
12 tests, 12 passed, 0 failed
```

```text
npm --prefix scenarios/incident-command run test:browser
Browser workflow passed
```

The regenerated `evidence/browser-result.json` records:

- postmortem conflict draft preservation and warning-only UI behavior;
- immediate tenant-scope clearing and rejection of a delayed old-tenant list;
- RBAC, guarded transitions, idempotency, optimistic concurrency, SLOs,
  tenant-filtered SSE, audit integrity, deterministic postmortems, and reload
  persistence;
- desktop `1280x900` and mobile `390x844` with no horizontal overflow, remaining
  console errors, or page errors.

The intentional rejected PATCH produces one Chromium 412 resource diagnostic;
the harness matches and consumes exactly that expected diagnostic while keeping
every other console error fatal.

## Governance boundary

All changes are confined to `scenarios/incident-command/**`. ECorp owns commit
creation, independent-review attestation, export, and publication. The
replacement pull request must be the single new provenance-bound PR, supersede
PR `#92`, close `#75`, `#88`, `#90`, `#91`, and `#96` after manual merge, and
leave auto-merge, merge, and deployment disabled.
