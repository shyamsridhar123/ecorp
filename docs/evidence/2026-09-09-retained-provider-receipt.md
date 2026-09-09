# Retained native receipt collection — local validation

September 9, 2026. Issue #211 remains open. This document separates the
collection contract and observed local checks from application acceptance and
publication. It does not replace or relabel the retained native-read failures.

## Why this is a separate path

The earlier SDK history and timeline diagnostics both returned
`SessionNotFound`. They remain failures in
[the native-read report](2026-09-09-stopped-session-native-read.md); neither is
relabelled as successful retrieval.

The later read-only journal/receipt audit, recorded at 20:24:35 UTC on
September 9, found a different existing object: the original ECorp Copilot
adapter's `copilot-evidence.json`, already present in the preserved worktree.
The audit records 1,070 bytes with SHA-256
`c8e68b95081d84fad09163bbe6762acca60e14dc4d73e51c4c31d99f4826cc88`.
It correlates the original run's native `run.session_terminated` outcome
`completed` and subsequent native preservation. ECorp nevertheless cancelled
that original run at its budget boundary without accepting its artifact.
The later source-correction run stopped with a native `cancelled` outcome.

Consequently this receipt is **historical evidence from the original run**,
not proof that the later correction completed, not a fresh native history
export, and not a previously signed or accepted artifact. The persisted
artifact check has no freshness requirement. The existing code checks and
independent outcome review still apply to the current corrected source.

This new operation collects those existing bytes; it does not synthesize a
receipt, scrape private vendor history, attach to a provider, send a prompt,
or change the stopped source's status or accounting.

## One existing recovery action

The implementation uses the existing explicit checkpoint-verification
recovery, durable `VerifyRun` command, trusted runner, and artifact pipeline.
It adds no per-artifact human approval or separate provider execution loop.

1. The server derives a grant from the exact checkpoint/source history and
   current actor, room, policy, saved connection, and recovery generation.
2. A compatible runner receives that grant on the new provider-free verifier
   command. It must not also receive a provider session, model, secrets, or
   inherited provider artifact.
3. The collector reads the fixed native receipt from the exact sealed source,
   validates its original format and historical identity, and preserves its
   bytes. Arbitrary paths and missing/ambiguous native content are not receipts.
4. Reservation and final adoption recheck current collection authority.
   Signed artifact metadata belongs to the **new verifier run** and explicitly
   labels the content historical.
5. Only a durable `provider_evidence` storage acknowledgment releases receipt
   collection. Current verifier checks, portable export, and independent
   outcome review remain required. Publication is a separate effect.

The new runner capability is `retained-provider-receipt-v1`. Advertising the
capability or passing a synthetic fixture is not runtime acceptance.

## Identity and replay contract

| Field | Meaning |
| --- | --- |
| `collection_id` | Exact durable verifier command ID |
| `run_id` | New verifier/artifact owner |
| `source_run_id` | Exact source selected for this recovery |
| `checkpoint_run_id` | Measured budget-checkpoint origin |
| `source_checkpoint_event_id` | Selected source's latest native preservation event |
| `historical_run_id` | Original native receipt producer in that source lineage |
| `historical_termination_event_id` / `historical_checkpoint_event_id` | Exact native historical proof |
| Expected fingerprint and HEAD | Current sealed source, not a claim about old receipt digests |

The private durable command sibling `retained_provider_receipt_upload` binds
the first newly observed digest and byte count atomically. Identical replay is
allowed; a different observation is denied. Abandoning an empty staging
reservation cannot erase this binding. It is not part of the shared grant or
runner dispatch, and command comparisons may ignore only this private binding,
not executable authority.

Signed metadata explicitly sets prior server acceptance, historical digest
attestation, provider completion, and provider inference claims to false.
Native JSON duplicate fields are rejected from the original bytes rather than
silently normalized.

Historical checkpoint hashes use their exact immutable original contract and
policy; they must not be compared with a later authorized revision. A previously
suspended ancestor is recognized only through the exact persisted native
source-correction authority that admitted it. Neither rule exempts unrelated
stops, suspensions, loops, quarantine, or current collection authorization.

Definite final-adoption denial leaves staged metadata rejected. Database
failures remain retryable, as do transient object-store errors in the bounded
receipt-upload path. A denied upload cannot obtain a duplicate acknowledgment
for an earlier object.

## Local validation — September 9

The initial source handoff had only formatting and static review. The following
checks were subsequently observed locally; hosted Actions were not used:

| Check | Observed result |
| --- | --- |
| Affected-crate offline compilation | PASS after correcting an oversized test JSON fixture |
| Domain native-receipt validation | 5 passed |
| Server signed-object and command/ACK checks | 11 passed |
| Runner collection, current checks, ACK/cancellation and preservation | 12 passed on Windows, 226.06 seconds |
| Real-migration SQLx store family `issue211_collection_` | 15 passed, 0 failed, 162.04 seconds |
| Workspace formatting | PASS |
| Workspace/all-target Clippy, warnings denied | PASS after boxing the optional protocol grant |
| Web build and lint | PASS |
| Immutable migration-file check | PASS, 41 unchanged migrations |
| Full serial Rust workspace tests | FAILED in runner: 205 passed / 2 failed / 1 ignored; 2,146.90 seconds |
| Separate remaining server/store unit suites | Server 107 passed / 4 ignored; store 55 passed / 286 ignored |

There are 44 declared cross-platform cases, but **43 were executed in the
focused Windows lanes**. The additional receipt-symlink case is Unix-only and
was not executed on this host. The store cases use the already-owned disposable
SQLx fixture through a separate #211-only branch/head/filter guard; the original
#210 loader was not changed. No application database, service, provider session,
old ignored SQLx family, or native history diagnostic was invoked by those lanes.
The SQLx cases prove persisted authority, metadata, rollback, and gate ordering,
not real signed object bytes or application acceptance. The server in-memory
tests separately exercise signatures and object bytes.

Retained failures and corrections:

- Initial compile failed on a large nested `json!` test fixture. The fixture
  was split into smaller values; the crate recursion limit was not raised.
- First SQLx execution was **11 passed / 4 failed**, 119.26 seconds. The
  artifact-evidence fixture omitted native digest/length/media fields, and two
  historical-correction fixtures reported enough usage to suspend but not stop.
  They now use complete native evidence and actual accounted usage beyond the
  native STOP threshold; original limits and spend are not reset.
- That SQLx run also exposed a production null-grant routing gap. Explicit
  marker presence, including null, now enters fail-closed receipt validation.
  Ordinary no-grant commands omit the directive and retain ordinary verifier
  dispatch without enabling collection.
- Initial Clippy rejected the enlarged `ServerToRunner` enum. The optional
  grant is boxed without changing its JSON; no lint was suppressed.

Independent read-only review rechecked the historical suspension and immutable
policy fixes, the scoped fixture loader, and the null-grant correction. It is
not a substitute for the execution results above.

### Unresolved full-suite result

The serial workspace run stopped at two existing checkpoint tests:

- `issue190_cancelled_exit_checkpoints_before_terminal_event`: zero checkpoint
  proofs instead of one.
- `issue190_unpinned_legacy_source_retains_without_checkpoint_proof`: the
  existing ten-second readiness wait elapsed.

Both subsequently passed individually on the exact same compiled binary, in
14.73 and 13.86 seconds, with unchanged source, assertions and timeout values.
The direct reruns are not a full-workspace pass or a controlled baseline/current
environment comparison. No cause is inferred merely from retry success. These
observations are retained under #213; production checkpoint safety and deadlines
were not relaxed. Server/store unit suites that the failed workspace run did
not reach passed separately (107 and 55 respectively); their ignored SQLx cases
were not executed by that command. **This work is not merge-ready and the
full release gate is not green.**

Evidence is retained under the existing private
`checkpoint-source-correction-20260909` directory. Key receipt prefixes:

- `20260909T213139106-issue211-retained-compile` (failed).
- `20260909T213840213-issue211-retained-compile-fixed` (passed).
- `20260909T213945852-issue211-retained-domain-server`.
- `20260909T214137104-issue211-retained-runner`.
- `20260909T215029328-retained-issue211_collection_` (11/15).
- `20260909T221435371-retained-issue211_collection_` (15/15).
- `20260909T222542962-issue211-retained-clippy` (failed).
- `20260909T223010374-issue211-retained-clippy-fixed` (passed).
- `20260909T223127531-issue211-retained-workspace-serial` (failed).
- `20260909T231329393-same-binary-issue190_cancelled_exit_checkpoints_before_terminal_event`.
- `20260909T231346950-same-binary-issue190_unpinned_legacy_source_retains_without_checkpoint_proof`.
- `20260909T232125714-issue211-server-store-unit` (passed separately).

## Remaining application acceptance

The existing QA server, runner and web process identities were read-only checked
against their owned manifest; API health returned development mode with one
runner. A subsequent recovery-context read returned HTTP 500 because the owned
QA database login had expired. The existing same-role renewal helper extended
its expiry to September 9, 2026, 18:42:34 CDT without changing its password,
privileges, services, or application data. Context then returned the unchanged
work item v82, source `945246da-b3ac-48d5-ac28-c9039c06de6e`, fingerprint and HEAD.

**The new collector has not yet been exercised through that application
stack.** No new native history diagnostic, provider run, service restart, application
source edit, outcome approval, or publication has been performed in this lane.

Remaining acceptance must use the same retained application and source:
finish the local release checks; exercise real reservation,
signing, acknowledgment, current-source verification and independent review;
confirm unchanged original history/accounting; then separately publish the
review-only application PR. No merge, auto-merge, deployment, replacement
mission, source rewrite, or counter reset is authorized by this collection.
