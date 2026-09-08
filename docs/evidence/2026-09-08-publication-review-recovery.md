# Publication recovery review fixes — September 8, 2026

Scope: PR #197 / issue #148. These are concrete publication/recovery defects,
not a continuation of the unobserved #193 budget-race investigation.

## Corrections

- A valid publisher can record failure for its exact owned attempt after the
  actor's role or room membership changes, or the source artifact expires.
  Failure skips only effect-authority revalidation. Corp identity, independent
  publisher credentials, exact actor/token/version and live lease remain required.
  The HTTP route uses existing read permission only for failure reporting; every
  effect-advancing variant retains `Publish`.
- New checkpoint publications use provenance schema 3. Revalidation may
  transactionally reconstruct previously absent schema-1/2 proof from the native
  authority, after all other bindings pass. Non-null altered legacy proof,
  missing/altered schema-3 proof and unsupported versions reject.
- Source and checkpoint upgrades compose without downgrading the final schema.
  Idempotent renewal reloads the upgraded record before responding.

## Observed checks

All five new actual-migration SQLx regressions first failed against the unchanged
product code for their reported causes: role, room, expired artifact, missing
legacy checkpoint, and missing legacy proof on renewal replay.

The final **36/36** scoped SQLx family passed; its test bodies took **102.18
seconds**. Coverage includes the inherited exported-work retry cases and five
new publication cases. Failure reporting also rejects wrong actor, credential,
token and version, retains prior work and records one replayable failure. Legacy
upgrade cases check both source/checkpoint composition and returned persisted
provenance, without another publication attempt.

All required local gates passed on unchanged code inputs: 39 migration
checksums, formatting, workspace/all-target Clippy, **384 ordinary Rust tests**,
web build/lint and whitespace. **142 opt-in SQLx tests were ignored in that
ordinary run**; the selected 36 ran separately. Server unit coverage includes
all four checkpoint permission classifications.

Retained SQLx receipts under
`C:\Users\shyamsridhar\.codex\dogfood\issue148-checkpoint-publication-20260908`:
`20260908T205355560-issue148_checkpoint_publication_review_.result.json` (red) and
`20260908T205727487-issue148_checkpoint_.result.json` (green).
The complete gate is
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue197-publication-review-final-20260908T210413654`.

These metadata fixtures and server unit tests do not claim a new HTTP
role-revocation drill, signed-object-byte validation, real-provider inference,
real-GitHub application PR, hosted Actions run or production-auth acceptance.
Earlier native publisher/restart evidence remains at its recorded scope. No
budget policies, failed observations or application history were reset.
