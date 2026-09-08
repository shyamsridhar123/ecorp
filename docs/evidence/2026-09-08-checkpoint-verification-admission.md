# Provider-free stopped-checkpoint admission

- Issue: #148; full dark-factory outcome #63.
- Base: `8317851b1c5733dc30f41eb29d9ff1e1b4ffa7a8` (draft #194), on merged checkpoint layer #192.
- Date: September 8, 2026.
- Status: actual-store acceptance passed; full runner/browser/application recovery is not yet accepted.

## Implemented scope

The existing Factory recovery aggregate has an explicit `checkpoint_verification` mode.
It consumes the native stopped-source checkpoint and termination records rather than starting
another model or treating `workspace_disposition=preserved` alone as sufficient proof.

Admission binds Corp, mission, task, run, workspace lineage, agent, runner, repository/ref/base,
branch/HEAD, fingerprint, and verification/write-scope/deliverable policy digests. It requires a
measured budget incident and retains explicit stop, loop, quarantine, actor/room, claim/version,
current-assignment, and unrelated-Factory-block protection.

The mode uses the existing verifier executor and durable command path. The replacement has no
provider session, model, reasoning setting, provider secrets, token allocation, or model-cost
allocation. Original provider attempts, source records, and all spending remain unchanged. Native
independent review remains required when present in the original policy.

This is not a general budget bypass for verifier runs: only an exactly linked checkpoint recovery
can proceed past model-usage exhaustion. Verifier-only model usage is rejected. Missing provider
artifact evidence still fails its existing durable-storage check.

Migration 0039 extends the existing recovery-mode constraint and persists its checkpoint authority.
The prior 38 migration files/checksums are unchanged.

## Actual-store results

The scoped loader selected only `issue148_checkpoint_` against the already owned SQLx maintenance
fixture. SQLx created disposable databases and applied all real migrations. No manual application
database, service, model, or browser was operated.

- Initial execution: **4 passed, 2 failed**. Both failures were in test expectations: the review
  fixture omitted native `gate_type`; missing artifact evidence was correctly rejected earlier
  than the test expected. Original output remains retained.
- Corrected six-case execution: **6 passed, 0 failed**, 31.85 seconds.
- Expanded final execution: **9 passed, 0 failed**, 34.75 seconds, 151 filtered out.
- No older ignored SQLx family was selected.

Coverage:

1. Exhausted model budget and provider attempts → new provider-free verification → distinct
   authorized independent review, with the original source and counters unchanged.
2. Exact operation replay; a second verifier recovery remains on the original checkpoint and
   cannot use an older source once a newer verifier exists.
3. Missing native termination and changed checkpoint assignment/source/head/policy digests reject
   before durable recovery effects.
4. Ordinary verifier-only and provider-correction requests cannot use the new authority.
5. Source/policy revisions, wrong actor, and lost room membership reject.
6. Unrelated Factory blocks, quarantine, and loop exhaustion remain effective.
7. Missing required provider artifact evidence cannot pass its persisted check.
8. Runner-command insertion failure rolls back the entire authorization.
9. Another active agent assignment is preserved; a generic zero-limit verifier is still blocked.

Receipts and original stdout/stderr are retained under:
`C:\Users\shyamsridhar\.codex\dogfood\issue148-checkpoint-admission-20260908`.

## Remaining acceptance — do not infer completion

- A fresh owned server/runner/browser case must exercise the actual recovery command and exact
  physical checkpoint, without starting another provider.
- Default policies that require an original provider artifact need its real retained bytes and
  provenance; this change does not synthesize a replacement artifact to satisfy those checks.
- The usable-result, non-empty bundle, checkpoint-bound publication, and browser action still
  require their own full-path evidence. A file-check metadata fixture does not prove them.
- The original #193 race candidate remains a failed observation of its intended race, not a
  late-after-failure native acceptance pass.
- No real-vendor session persistence, global deadlock freedom, merge readiness, deployment,
  hosted Actions result, or completion of #148/#63 is claimed.

## Repository gate

The final local gate passed on this source: **370 Rust tests**, with 115 opt-in SQLx cases ignored
in that ordinary run. The nine new actual-store cases above ran separately. Migration validation
checked all 39 files; formatting, workspace/all-target Clippy with warnings denied, web build/lint,
and whitespace checks also passed.

The continuation independently rehashed all 13 code/schema/lockfile inputs against the retained
gate receipt; every hash matched. No new run is implied by that readback. The receipt and full
outputs are at:
`C:\Users\shyamsridhar\.codex\dogfood\remaining-work-20260908\issue148-checkpoint-admission-20260908T150825175`.

These gates do not replace the remaining native and application acceptance.
