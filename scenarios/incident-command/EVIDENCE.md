# Incident Command final correction evidence

Date: 2026-09-03

## Provenance

- Final correction issue: GitHub `#104`
- Immutable source base: `5999a4c808867796671dbafe02ccd8af69a74445`
- Latest verified Incident Command reference:
  `622ae894a787296725a8e762cfe05b2a7f79e394`
- The complete `scenarios/incident-command/**` tree was restored from that
  reference before applying the corrections.
- No external network access or package installation was used.

## Corrections

- Every incident-list request has a monotonically increasing generation, so
  obsolete filter and scheduled-refresh responses cannot replace the newest
  queue within the current tenant scope.
- Incident creation captures tenant scope. A response arriving after an
  identity change cannot reset the create form, select or fetch the created
  incident, or report success in the new scope.
- Mutations reset forms and show success only when the still-current operation
  returns `true`; completions after incident selection or tenant changes are
  silent.
- Postmortem downloads capture incident ID, scope generation, and selection
  generation. Stale responses cannot download or report success, and filename
  fallback and feedback derive only from the captured incident.
- Timeline verification treats every null, array, or non-object entry,
  including the tail, as `ENTRY_NOT_OBJECT`; malformed tails return a null
  `headHash`. Detail, audit, preview, download, and Markdown generation return
  stable `INVALID` evidence instead of dereferencing malformed entries.
- The persisted incident version anchors the expected audit-entry count.
  Removing the latest entry or the complete timeline now yields
  `ENTRY_COUNT_MISMATCH`; an empty timeline also yields `TIMELINE_EMPTY`.
- Startup validates every persisted incident, timeline entry, postmortem,
  idempotency record, cached response, timestamp, and tenant/key binding before
  reporting persistence as ready. Malformed records and truncated audit chains
  fail startup instead of causing later 500 responses.
- The optional Playwright harness is `e2e/browser.mjs`, outside Node's `test/`
  discovery directory. Bare discovery runs only the 16 dependency-free
  built-in server tests, while
  `npm run test:browser` explicitly executes the desktop/mobile workflow.
- All prior fixes from the verified reference remain present, including
  postmortem conflict handling, tenant lists, authoritative audit binding,
  latest-detail selection, and stale-preview rejection.

## Verification

```text
npm --prefix scenarios/incident-command test
16 tests, 16 passed, 0 failed
```

```text
cd scenarios/incident-command
node --test
16 tests, 16 passed, 0 failed
```

Both commands pass without `ECORP_PLAYWRIGHT_MODULE`; `npm test` reports only
the 16 built-in server tests.

```text
$env:ECORP_PLAYWRIGHT_MODULE = '<bundled Playwright module>'
npm --prefix scenarios/incident-command run test:browser
Browser workflow passed
```

The regenerated browser result reports every workflow, SSE, tenant,
concurrency, stale-response, audit, persistence, desktop, and mobile flag as
`true`, with no unexpected console or page errors and no horizontal overflow
at `1280x900` or `390x844`.

```text
pnpm check
30 immutable migrations
Rust formatting passed
Clippy passed with warnings denied
99 Rust tests passed
Production web build passed
Web lint passed
```

Evidence SHA-256:

- `browser.png`:
  `8ad4e147a2200c50e98a1d6fe0f750cb23dab8fddd1606981505e40ad6becca2`
- `browser-mobile.png`:
  `92a38945beb5c5eb500b409708fb2075e58e1bef0a0e85ef9641ace0f7282e19`
- `browser-result.json`:
  `1cda056a7d623531bedb080ae2c3b63f8fc34b3d1a7dd98fa8da5b743da62ce3`

Port `9312` was free after both server and browser verification.

## Governance boundary

All changes are confined to `scenarios/incident-command/**`. ECorp owns final
verification, independent review, commit creation, export, and publication.
The single final pull request must supersede PRs `#92`, `#99`, and `#102`;
close `#75`, `#88`, `#90`, `#91`, `#96`, `#101`, `#103`, and `#104` after
manual merge; and leave auto-merge, merge, and deployment disabled.
