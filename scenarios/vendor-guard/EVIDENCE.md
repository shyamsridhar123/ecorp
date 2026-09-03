# VendorGuard verification evidence

## Provenance

- Recovery issue: GitHub `#85`, Refresh and publish VendorGuard from current main
- Prior feature issue: GitHub `#74`, VendorGuard third-party risk onboarding
- Refreshed immutable base commit: `b4c0981b7f4b79b49e78484f0aaec07ef910627d`
- Approved prior verified local reference: `a4deca71cb132c3a370536f329d6ce38239a6577`
- Reconstruction: only `scenarios/vendor-guard/**` was restored from the approved
  local Git object; no prior branch history was imported
- Producing environment: isolated ECorp mission worktree
- Dependency activity: no network access and no package installation
- Hosted CI: unavailable by contract; local ECorp verifier output is authoritative

## Producing-worktree results

The exact governed commands and final observed results are:

```text
node --test scenarios/vendor-guard/test/server.test.mjs
8 tests, 8 passed, 0 failed, duration 1281.709 ms

node scenarios/vendor-guard/test/browser.test.mjs
Browser workflow passed; evidence written to scenarios/vendor-guard/evidence/browser-result.json

git diff --check -- scenarios/vendor-guard
exit 0; no output
```

The browser result is machine-readable at `evidence/browser-result.json`; the
corresponding desktop and mobile full-page captures are `evidence/browser.png`
and `evidence/browser-mobile.png`. Exact screenshot byte sizes are intentionally
not asserted because rendering output can vary between verifier executions. The
final JSON records `status: passed`, healthy desktop and mobile service checks,
all workflow assertions as `true`, empty unexpected console/page error arrays,
desktop `viewport` at 1280-by-900 with `scrollWidth: 1280`, and top-level
`mobileViewport` at 390-by-844 with `scrollWidth: 390`; both report
`horizontalOverflow: false`.

## Coverage

The Node suite uses a distinct temporary data directory for every test and covers
deterministic scoring, bounded validation, structured errors, tenant isolation,
RBAC, maker-checker separation, durable review reasoning, idempotent replay and
conflicting reuse, concurrent optimistic writes, terminal transitions, restart
durability, audit verification, and persisted-event tamper detection.

The browser harness traverses the real browser-to-HTTP-to-persistence path. It
checks tenant switching, requester creation and submission, a same-actor
maker-checker denial, independent reviewer approval, stale version conflict,
idempotent replay and conflict, visible risk factors and thresholds, admin audit
verification, server health, console and page errors, and horizontal overflow.
It performs responsive checks at desktop 1280-by-900 and mobile 390-by-844 and
writes the measured desktop and mobile viewport results into the JSON evidence.

## Rework and recovery

During the prior `#74` producing run, the first browser verifier execution
completed the workflow but failed its final console assertion because Chromium
emits generic resource-error console messages for the intentionally exercised
`409`, `409`, and `403` API responses. The harness was revised to classify those
three messages only while their exact negative response paths are under assertion.
It continues to fail for any unrelated console error, page error, unexpected
count, failed health check, incomplete workflow, or overflow. The prior run's
second execution passed and replaced its failed artifacts. A subsequent reviewer
correction revision removed a premature independent-review claim and required the
desktop/mobile responsive verifier pass. This refreshed run passed on its first
browser verifier execution. No external network recovery, package installation,
budget suspension, or budget revision occurred during producing verification.

## Honest limitations and governance

This producing run implements and verifies the portable source deliverable only.
The durable independent-review decision is external to this producing run and
remains pending until a separate reviewer acts. Publication, Project status
movement, and independent reviewer acceptance belong to the ECorp controller's
exactly-once publication and approval stages; this evidence does not claim that
those external stages occurred. No pull request was merged, no auto-merge was
enabled, and nothing was deployed.

Setup/completion timestamps, controller retries, intervention, approvals, and
token/cost accounting are authoritative only in controller records. They are not
invented here. The scenario records the configured model (`gpt-5.6-sol`) and
reasoning effort (`high`), but does not claim unavailable consumption metrics.

The local header-based identity boundary, single-process JSON storage, and
unanchored local audit head are intentionally documented demonstration
limitations. See `README.md` for production controls that remain out of scope.
