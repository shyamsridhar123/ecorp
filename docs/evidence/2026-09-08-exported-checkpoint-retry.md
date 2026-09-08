# Exported checkpoint retry correction — September 8, 2026

Scope: PR #195 / issue #148. This fixes retrying preserved work after a verifier
has already exported its authorized commit; it is not another budget-race drill.

## Defect and correction

Recovery preview and admission replaced a verifier's provenance-bound exported
HEAD with the original provider HEAD. A retry therefore selected the wrong
checkout guard. Both paths now use the original HEAD only for the original
provider source; verifier retries retain the validated exported/authorized HEAD.
The original checkpoint authority and failed-run history remain unchanged.

The provider-free allocation check now compares against the same retained-source
resolver. Its read-only use introduces no ancestor row lock into event accounting;
all existing locking callers retain their original `FOR UPDATE` behavior.
This prevents the corrected retry from being rejected merely because its exported
HEAD differs from the immutable origin. It does not remove the head guard.

## Observed checks

- The two new actual-migration SQLx cases first failed against unchanged product
  code: preview returned the original HEAD instead of the exported HEAD.
- An intermediate run passed the existing 23 cases; the two new cases reached
  retry but failed a test assertion using incorrect token-column names. That
  output remains retained rather than being relabeled as a product failure.
- The final scoped SQLx run passed **25/25**, including failed and cancelled
  post-export retries, immutable original/export history, rejection of the old
  HEAD, idempotent retry, exact provider-free allocation and native command
  dispatch authorization. The SQLx test body took **61.87 seconds**.
- Required local gates passed: all 39 migration checksums, formatting,
  workspace/all-target Clippy, **381 ordinary Rust tests**, web build/lint, and
  whitespace. **131 opt-in SQLx tests were ignored in the ordinary run**; the
  25 selected cases ran separately. Source inputs were unchanged during the gate.

Receipts are retained under
`C:\Users\shyamsridhar\.codex\dogfood\issue148-checkpoint-admission-20260908`:
`20260908T203433911-issue148_checkpoint_exported_retry_.result.json` (red),
`20260908T203539152-issue148_checkpoint_.result.json` (test-assertion correction),
and `20260908T204007458-issue148_checkpoint_.result.json` (green).
The complete local gate is
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue195-exported-head-review-final-20260908T204223287`.

These are actual-store metadata regressions, not new browser, transport-fault,
signed-object-byte, real-provider, real-GitHub or production-auth acceptance.
Earlier native application evidence remains at its recorded scope. The specific
unobserved queued-upload race in #193 remains an open follow-up, not a release
gate; no further budget experiments were run.
