# Incident Command review-correction evidence

Date: 2026-09-03

## Provenance

- Review-correction issue: GitHub `#101`
- Immutable source base: `5999a4c808867796671dbafe02ccd8af69a74445`
- Independently reviewed correction reference:
  `ada87bd85dafdc62f354c4641c7e9340be5b1ece`
- The complete `scenarios/incident-command/**` tree was restored from that
  reference before applying the corrections.
- No external network access or package installation was used.

## Corrections

- Timeline verification is bound to the containing incident and tenant IDs.
  Internally hash-consistent foreign timelines are invalid in incident views,
  audit responses, postmortem previews/downloads, and exported Markdown.
- Detail responses are fenced by both tenant scope and selection generation, so
  a delayed response for an earlier selection cannot replace the latest one.
- Postmortem previews are fenced by scope generation, selection generation, and
  incident ID. Selection and identity changes clear stale preview content, and
  delayed previews cannot render afterward.
- Prior postmortem-conflict draft preservation and tenant-list generation
  fencing remain covered.
- `npm test` explicitly runs only the dependency-free built-in server suite.
  The Playwright workflow remains available through `npm run test:browser` and
  requires `ECORP_PLAYWRIGHT_MODULE`.

## Verification

```text
npm --prefix scenarios/incident-command test
13 tests, 13 passed, 0 failed
```

```text
npm --prefix scenarios/incident-command run test:browser
Browser workflow passed
```

The regenerated `evidence/browser-result.json` records:

- postmortem conflict draft preservation and warning-only UI behavior;
- rejection of delayed detail responses after a newer incident selection;
- rejection of delayed postmortem previews after incident and tenant changes,
  with stale preview content cleared immediately;
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
PRs `#92` and `#99`, close `#75`, `#88`, `#90`, `#91`, `#96`, and `#101` after
manual merge, and leave auto-merge, merge, and deployment disabled.
