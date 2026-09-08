# Missing-start-report cleanup: negative coverage

- **Issue:** #186; follow-up to the merged #183 behavior.
- **Source base:** `41fc0d6c6f847ebe0dcaa1cef864c56aa90e211e`.
- **Observed:** September 8, 2026.
- **Product behavior:** unchanged.

## Coverage added

Twenty-one new `issue186_` SQLx cases use the existing native ACK/loss fixture
with actual migrations. Binding mutations occur **before** cleanup, while all
four assignment fields are NULL and no `run.started` event exists.

The matrix covers completed/missing/replaced receipts, missing replacement or
resume identity, wrong recovery mode/source run, foreign Corp/mission/task/Factory
item, changed agent/runner/workspace root, changed or missing source tuple, and
changed authorized fingerprint.

Assertions verify that the authenticated cleanup cannot backfill unauthorized
assignment metadata or admit recovery. They retain the original parent record
and fingerprint, non-cleanup run fields, task/mission/budget/attempt state,
receipt/command state and earlier journal rows. A fresh recovery key is denied
at the exact checkpoint boundary, before unrelated minimal-fixture policy checks;
run/recovery/command/revision counts remain unchanged. Cleanup replay is
idempotent and a changed child fingerprint is rejected.

Two quarantine cases cover both a missing fingerprint and an already
native-confirmed fingerprint. Fresh later preserved/fingerprint reports cannot
clear quarantine, replace the established seal or admit recovery.

The original thirteen #183 tests and assertions remain unchanged. Only the
test file was modified; no product predicate or authorization was relaxed.

## Observed checks

| Check | Result |
| --- | --- |
| New actual-migration SQLx family | **21 passed, 0 failed**, 68.48 seconds |
| Ordinary store unit suite | **43 passed**, 96 opt-in cases ignored |
| Focused format/no-run compile/store Clippy | Passed |
| Scoped whitespace/prefix-preservation audit | Passed |

The initial new helper incorrectly read raw event rows through `event_type`
instead of their native `type` column. All 21 cases failed before the intended
cleanup path. Only that test lookup was corrected; full failed output is retained
and is not described as a product defect.

The new database cases ran only through the explicit credential-safe #186 loader.
Older #169 and #183 database families stayed ignored. An unnecessary environment
preflight was denied before execution; it was not repeated, and subsequent
format checks did not require that operation.

## Boundaries

This is Windows-host store-metadata coverage—not physical workspace cleanup,
provider execution, signed-object verification, browser acceptance or a
concurrency stress test. No product defect was exposed by the completed matrix.
No service, provider, browser or container was started/stopped, no application
database was selected, and no credentials were changed.

The worker's exact commands, named cases and original logs are retained under
`C:\Users\shyamsridhar\.codex\worktrees\ecorp-issue174-local-start\target\issue186-validation-20260908`.
The test file SHA-256 is
`526712E234837D608B753670939ADECC425E5A76A595EB340EED9442205919E3`.
Full repository gates are recorded separately before landing.
